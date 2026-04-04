use nih_plug::prelude::*;
use nih_plug::util::permit_alloc;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crossbeam_channel::Receiver;
use crate::engine::kit::NUM_PADS;

use crate::analysis::library::SampleLibrary;
use crate::engine::sampler::VoicePool;
use crate::engine::sequencer::Sequencer;
use crate::logging;
use crate::ui::state::{ScanStatus, SharedState};
use crate::util::history::HistorySnapshot;

/// Hard-coded sample library root — folder picker comes in GUI phase.
const SAMPLE_LIBRARY_ROOT: &str = "/home/natalia/Music/Samples";

/// Number of waveform display points per pad.
const WAVEFORM_POINTS: usize = 200;

#[derive(Params)]
pub struct AutokitParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "master_vol"]
    pub master_volume: FloatParam,
}

impl Default for AutokitParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(900, 365),
            master_volume: FloatParam::new(
                "Master Volume",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(6.0),
                    factor: FloatRange::gain_skew_factor(-60.0, 6.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

/// Messages from background thread to audio thread.
enum BgMessage {
    /// Library scan complete — assign samples to kit.
    LibraryReady(SampleLibrary),
}

pub struct Autokit {
    params: Arc<AutokitParams>,
    sample_rate: f32,
    /// State shared with the GUI thread.
    pub shared: Arc<Mutex<SharedState>>,
    voices: Option<VoicePool>,
    /// Receive messages from background thread (checked in process()).
    bg_rx: Option<Receiver<BgMessage>>,
    sequencer: Sequencer,
    /// Lockfree trigger counters — incremented each time a pad fires.
    /// Shared with the GUI thread which reads them each frame for activity animation.
    pub trigger_flags: Arc<[AtomicU8; NUM_PADS]>,
    /// GUI-to-audio trigger requests — GUI sets to 1, audio thread reads and clears.
    pub gui_triggers: Arc<[AtomicU8; NUM_PADS]>,
    /// Debug: counts process() calls to log periodic status.
    #[cfg(debug_assertions)]
    process_count: u64,
}

impl Default for Autokit {
    fn default() -> Self {
        Self {
            params: Arc::new(AutokitParams::default()),
            sample_rate: 44100.0,
            shared: Arc::new(Mutex::new(SharedState::new())),
            voices: None,
            bg_rx: None,
            sequencer: Sequencer::new(),
            trigger_flags: Arc::new(core::array::from_fn(|_| AtomicU8::new(0))),
            gui_triggers: Arc::new(core::array::from_fn(|_| AtomicU8::new(0))),
            #[cfg(debug_assertions)]
            process_count: 0,
        }
    }
}

/// Populate the kit from the library using the default layout, then update waveforms.
/// Ensures each pad receives a unique sample (best-effort when category has fewer samples than pads).
fn populate_kit_from_library(shared: &mut SharedState) {
    let layout = shared.library.as_ref().expect("library must be set before populate").generate_kit();
    let mut assigned = 0u32;

    // Seed exclusion set with paths from locked pads so we don't duplicate those either
    let mut used: HashSet<String> = shared
        .kit
        .pads
        .iter()
        .filter(|p| p.locked)
        .filter_map(|p| p.sample_path.clone())
        .collect();

    for (pad_idx, category) in layout {
        if pad_idx >= shared.kit.pads.len() {
            break;
        }

        // Skip locked pads
        if shared.kit.pads[pad_idx].locked {
            continue;
        }

        if let Some(sample) = shared.library.as_ref().unwrap().random_from_excluding(category, &used) {
            let path = sample.entry.path.to_string_lossy().to_string();
            used.insert(path.clone());
            shared.kit.pads[pad_idx].sample = Some(Arc::clone(&sample.data));
            shared.kit.pads[pad_idx].sample_path = Some(path);
            shared.kit.pads[pad_idx].name = sample.entry.filename.clone();
            shared.kit.pads[pad_idx].category = sample.entry.category;
            assigned += 1;
        }
    }

    tracing::info!(assigned, total_pads = shared.kit.pads.len(), "kit populated from library");

    shared.update_all_waveforms(WAVEFORM_POINTS);
}

impl Plugin for Autokit {
    const NAME: &'static str = "Autokit";
    const VENDOR: &'static str = "REXIST";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        tracing::info!("editor() called — creating egui editor");

        let shared = Arc::clone(&self.shared);
        let params = Arc::clone(&self.params);

        // Create a snapshot function that captures sequencer state.
        // The sequencer lives on the audio thread; we snapshot it via a closure.
        let seq = self.sequencer.snapshot();
        let seq_snapshot: Arc<dyn Fn() -> crate::util::history::SequencerSnapshot + Send + Sync> =
            Arc::new(move || seq.clone());

        let result = crate::ui::editor::create(
            self.params.editor_state.clone(),
            shared,
            params,
            seq_snapshot,
            Arc::clone(&self.trigger_flags),
            Arc::clone(&self.gui_triggers),
        );
        tracing::info!("editor() result: {}", if result.is_some() { "Some" } else { "None" });
        result
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        logging::init();
        tracing::info!(
            "Autokit v{} initialized — sample rate: {}",
            Self::VERSION,
            self.sample_rate
        );

        self.voices = Some(VoicePool::new(self.sample_rate));

        // Spawn background thread to scan sample library
        let (tx, rx) = crossbeam_channel::bounded::<BgMessage>(1);
        self.bg_rx = Some(rx);

        let sample_rate = self.sample_rate;
        let root = PathBuf::from(SAMPLE_LIBRARY_ROOT);

        std::thread::Builder::new()
            .name("autokit-scanner".to_string())
            .spawn(move || {
                tracing::info!("background scan starting");
                let library = SampleLibrary::build(&root, sample_rate);
                if tx.send(BgMessage::LibraryReady(library)).is_err() {
                    tracing::warn!("plugin dropped before scan completed");
                }
            })
            .expect("failed to spawn scanner thread");

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Check for background thread messages (non-blocking)
        if let Some(rx) = &self.bg_rx {
            if let Ok(msg) = rx.try_recv() {
                permit_alloc(|| {
                    match msg {
                        BgMessage::LibraryReady(library) => {
                            tracing::info!(
                                total = library.total,
                                "library received — populating kit"
                            );
                            let mut shared = self.shared.lock();
                            // Push snapshot before first population for undo support
                            let snapshot = HistorySnapshot {
                                pads: shared.kit.snapshot(),
                                sequencer: self.sequencer.snapshot(),
                            };
                            shared.history.push(snapshot);
                            shared.library = Some(library);
                            populate_kit_from_library(&mut shared);
                            shared.scan_status = ScanStatus::Ready {
                                total: shared.library.as_ref().map(|l| l.total).unwrap_or(0),
                            };
                        }
                    }
                });
            }
        }

        // Periodic debug heartbeat (~every 5s)
        #[cfg(debug_assertions)]
        {
            self.process_count += 1;
            if self.process_count % 1000 == 1 {
                let active = self.voices.as_ref().map(|v| v.active_count()).unwrap_or(0);
                let has_lib = self
                    .shared
                    .try_lock()
                    .map(|s| s.library.is_some())
                    .unwrap_or(false);
                let seq_step = self.sequencer.current_step();
                let seq_playing = self.sequencer.is_playing();
                permit_alloc(|| {
                    tracing::debug!(
                        call = self.process_count,
                        active_voices = active,
                        library_loaded = has_lib,
                        seq_step,
                        seq_playing,
                        "process() heartbeat"
                    );
                });
            }
        }

        let voices = match &mut self.voices {
            Some(v) => v,
            None => return ProcessStatus::KeepAlive,
        };

        let num_samples = buffer.samples();

        // Try to lock shared state — if the GUI holds it, skip MIDI/sequencer
        // this buffer (a few ms) rather than blocking the audio thread.
        // permit_alloc: parking_lot may allocate during contended unlock_slow(),
        // which would otherwise trigger assert_no_alloc panic.
        let got_lock = permit_alloc(|| self.shared.try_lock());
        if let Some(shared) = got_lock {
            // Drain MIDI events and trigger voices
            while let Some(event) = context.next_event() {
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        if let Some(pad_idx) = shared.kit.pad_for_note(note) {
                            voices.trigger(pad_idx, velocity, &shared.kit, 0);
                            self.trigger_flags[pad_idx].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    NoteEvent::NoteOff { .. } => {}
                    _ => {}
                }
            }

            // Check GUI trigger requests (keyboard/click-to-play)
            for i in 0..NUM_PADS {
                if self.gui_triggers[i].swap(0, Ordering::Relaxed) != 0 {
                    voices.trigger(i, 0.8, &shared.kit, 0);
                    self.trigger_flags[i].fetch_add(1, Ordering::Relaxed);
                }
            }

            // Run sequencer — triggers voices at step boundaries
            let transport = context.transport();
            self.sequencer.process_buffer(
                num_samples,
                transport.playing,
                transport.tempo,
                transport.pos_beats(),
                self.sample_rate,
                voices,
                &shared.kit,
                &self.trigger_flags,
            );

            let channels = buffer.as_slice();
            if channels.len() >= 2 {
                let (left_channels, right_channels) = channels.split_at_mut(1);
                let output_left = &mut left_channels[0][..num_samples];
                let output_right = &mut right_channels[0][..num_samples];
                output_left.fill(0.0);
                output_right.fill(0.0);
                voices.process(output_left, output_right, &shared.kit);
            }
            // permit_alloc for drop: parking_lot unlock_slow() may allocate.
            permit_alloc(|| drop(shared));
        } else {
            // GUI holds the lock — output silence this buffer.
            let channels = buffer.as_slice();
            if channels.len() >= 2 {
                let (left_channels, right_channels) = channels.split_at_mut(1);
                left_channels[0][..num_samples].fill(0.0);
                right_channels[0][..num_samples].fill(0.0);
            }
        }

        // Apply master volume after lock is released
        let master_gain = self.params.master_volume.smoothed.next();
        let channels = buffer.as_slice();
        if channels.len() >= 2 {
            for s in channels[0][..num_samples].iter_mut() {
                *s *= master_gain;
            }
            for s in channels[1][..num_samples].iter_mut() {
                *s *= master_gain;
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Autokit {
    const CLAP_ID: &'static str = "com.rexist.autokit";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("AI-powered drum machine with sample map visualization");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::DrumMachine,
        ClapFeature::Sampler,
    ];
}

impl Vst3Plugin for Autokit {
    const VST3_CLASS_ID: [u8; 16] = *b"AutokitDrumM001\0";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Drum,
        Vst3SubCategory::Sampler,
    ];
}
