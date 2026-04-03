use nih_plug::prelude::*;
use nih_plug::util::permit_alloc;
use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Receiver;

use crate::analysis::library::SampleLibrary;
use crate::engine::kit::DrumKit;
use crate::engine::sampler::VoicePool;
use crate::engine::sequencer::Sequencer;
use crate::logging;
use crate::util::history::{History, HistorySnapshot};

/// Hard-coded sample library root — folder picker comes in GUI phase.
const SAMPLE_LIBRARY_ROOT: &str = "/home/natalia/Music/Samples";

#[derive(Params)]
pub struct AutokitParams {
    #[id = "master_vol"]
    pub master_volume: FloatParam,
}

impl Default for AutokitParams {
    fn default() -> Self {
        Self {
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
    kit: DrumKit,
    voices: Option<VoicePool>,
    /// Receive messages from background thread (checked in process()).
    bg_rx: Option<Receiver<BgMessage>>,
    /// Library reference kept for kit regeneration.
    library: Option<SampleLibrary>,
    sequencer: Sequencer,
    /// Undo/redo history for kit + sequencer changes.
    history: History,
    /// Debug: counts process() calls to log periodic status.
    #[cfg(debug_assertions)]
    process_count: u64,
}

impl Default for Autokit {
    fn default() -> Self {
        Self {
            params: Arc::new(AutokitParams::default()),
            sample_rate: 44100.0,
            kit: DrumKit::new(),
            voices: None,
            bg_rx: None,
            library: None,
            sequencer: Sequencer::new(),
            history: History::new(),
            #[cfg(debug_assertions)]
            process_count: 0,
        }
    }
}

/// Populate the kit from the library using the default layout.
fn populate_kit_from_library(kit: &mut DrumKit, library: &SampleLibrary) {
    let layout = library.generate_kit();
    let mut assigned = 0u32;

    for (pad_idx, category) in layout {
        if pad_idx >= kit.pads.len() {
            break;
        }

        // Skip locked pads
        if kit.pads[pad_idx].locked {
            continue;
        }

        if let Some(sample) = library.random_from(category) {
            kit.pads[pad_idx].sample = Some(Arc::clone(&sample.data));
            kit.pads[pad_idx].sample_path = Some(sample.entry.path.to_string_lossy().to_string());
            kit.pads[pad_idx].name = sample.entry.filename.clone();
            kit.pads[pad_idx].category = sample.entry.category;
            assigned += 1;
        }
    }

    tracing::info!(assigned, total_pads = kit.pads.len(), "kit populated from library");
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
                            // Push snapshot before first population for undo support
                            let snapshot = HistorySnapshot {
                                pads: self.kit.snapshot(),
                                sequencer: self.sequencer.snapshot(),
                            };
                            self.history.push(snapshot);
                            populate_kit_from_library(&mut self.kit, &library);
                            self.library = Some(library);
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
                let has_lib = self.library.is_some();
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

        // Drain MIDI events and trigger voices
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    if let Some(pad_idx) = self.kit.pad_for_note(note) {
                        voices.trigger(pad_idx, velocity, &self.kit, 0);
                    }
                }
                NoteEvent::NoteOff { .. } => {}
                _ => {}
            }
        }

        // Run sequencer — triggers voices at step boundaries
        let transport = context.transport();
        self.sequencer.process_buffer(
            buffer.samples(),
            transport.playing,
            transport.tempo,
            transport.pos_beats(),
            self.sample_rate,
            voices,
            &self.kit,
        );

        let num_samples = buffer.samples();
        let channels = buffer.as_slice();

        if channels.len() < 2 {
            return ProcessStatus::KeepAlive;
        }

        let (left_channels, right_channels) = channels.split_at_mut(1);
        let output_left = &mut left_channels[0][..num_samples];
        let output_right = &mut right_channels[0][..num_samples];

        output_left.fill(0.0);
        output_right.fill(0.0);

        voices.process(output_left, output_right, &self.kit);

        let master_gain = self.params.master_volume.smoothed.next();
        for s in output_left.iter_mut() {
            *s *= master_gain;
        }
        for s in output_right.iter_mut() {
            *s *= master_gain;
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
