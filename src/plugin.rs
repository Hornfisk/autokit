use nih_plug::prelude::*;
use nih_plug::util::permit_alloc;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering};

use crate::engine::kit::NUM_PADS;

use crate::analysis::library::{SampleLibrary, ScanProgress, ScanResult};
use crate::engine::echo_detect::EchoDetector;
use crate::engine::sampler::VoicePool;
use crate::engine::sequencer::Sequencer;
use crate::logging;
use crate::ui::state::{ScanStatus, SharedState};
use crate::util::config;
use crate::util::default_kit;
use crate::util::history::HistorySnapshot;
use crate::util::preset;

/// Number of waveform display points per pad.
const WAVEFORM_POINTS: usize = 200;

#[derive(Params)]
pub struct AutokitParams {
    #[persist = "editor-state-v2"]
    pub editor_state: Arc<EguiState>,

    /// Serialized kit + pattern state for DAW save/load persistence.
    #[persist = "plugin-state"]
    pub plugin_state: Arc<parking_lot::Mutex<String>>,

    #[id = "master_vol"]
    pub master_volume: FloatParam,

    #[id = "comp_threshold"]
    pub comp_threshold: FloatParam,

    #[id = "comp_drive"]
    pub comp_drive: FloatParam,

    #[id = "limiter_on"]
    pub limiter_on: BoolParam,

    // ── Master FX ─────────────────────────────────────────────────────
    // These use nih-plug's built-in smoothing for DAW-side automation
    // writes. The engine-layer `StepSmoother` in `FxBus` on top of that
    // handles musical step-boundary transitions for pattern changes and
    // Volca-style automation playback.
    #[id = "reverb_mix"]
    pub reverb_mix: FloatParam,

    #[id = "delay_mix"]
    pub delay_mix: FloatParam,

    /// 0..1 normalized, mapped to {1/32, 1/16, 1/8, 1/4} note by the audio
    /// thread each buffer based on the current tempo.
    #[id = "delay_time"]
    pub delay_time: FloatParam,

    /// Bipolar DJ filter: -1 = full lowpass kill, 0 = bypass, +1 = full
    /// highpass kill.
    #[id = "dj_filter"]
    pub dj_filter: FloatParam,
}

impl Default for AutokitParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(1060, 540),
            plugin_state: Arc::new(parking_lot::Mutex::new(String::new())),
            master_volume: FloatParam::new(
                "Master Volume",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(6.0),
                    factor: FloatRange::gain_skew_factor(-60.0, 6.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            comp_threshold: FloatParam::new(
                "Comp Threshold",
                -12.0,
                FloatRange::Linear { min: -40.0, max: 0.0 },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" dB")
            .with_value_to_string(Arc::new(|v| format!("{v:.1}")))
            .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches(" dB").trim().parse().ok())),
            comp_drive: FloatParam::new(
                "Drive",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit("%")
            .with_value_to_string(Arc::new(|v| format!("{:.0}", v * 100.0)))
            .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches('%').trim().parse::<f32>().ok().map(|v| v / 100.0))),
            limiter_on: BoolParam::new("Limiter", true),
            reverb_mix: FloatParam::new(
                "Reverb Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(60.0))
            .with_unit("%")
            .with_value_to_string(Arc::new(|v| format!("{:.0}", v * 100.0)))
            .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches('%').trim().parse::<f32>().ok().map(|v| v / 100.0))),
            delay_mix: FloatParam::new(
                "Delay Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(60.0))
            .with_unit("%")
            .with_value_to_string(Arc::new(|v| format!("{:.0}", v * 100.0)))
            .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches('%').trim().parse::<f32>().ok().map(|v| v / 100.0))),
            delay_time: FloatParam::new(
                "Delay Time",
                0.5, // 1/8 note
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::None)
            .with_value_to_string(Arc::new(|v| {
                let label = match (v * 4.0).round() as i32 {
                    0 => "1/32",
                    1 => "1/16",
                    2 => "1/8",
                    _ => "1/4",
                };
                label.to_string()
            })),
            dj_filter: FloatParam::new(
                "DJ Filter",
                0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_value_to_string(Arc::new(|v| {
                if v.abs() < 0.01 {
                    "OFF".to_string()
                } else if v < 0.0 {
                    format!("LP {:.0}%", -v * 100.0)
                } else {
                    format!("HP {:.0}%", v * 100.0)
                }
            })),
        }
    }
}

/// Lightweight preview voice — separate from VoicePool.
struct PreviewVoice {
    data: Option<Arc<Vec<f32>>>,
    position: usize,
}

impl PreviewVoice {
    fn new() -> Self {
        Self { data: None, position: 0 }
    }

    /// Start a new preview. Replaces any currently playing preview.
    fn start(&mut self, sample: Arc<Vec<f32>>) {
        permit_alloc(|| {
            self.data = Some(sample);
        });
        self.position = 0;
    }

    /// Render into stereo buffers (center pan, unity gain). Adds to existing content.
    fn process(&mut self, output_left: &mut [f32], output_right: &mut [f32]) {
        let data = match &self.data {
            Some(d) => d,
            None => return,
        };

        for (l, r) in output_left.iter_mut().zip(output_right.iter_mut()) {
            if self.position >= data.len() {
                permit_alloc(|| {
                    self.data = None;
                });
                return;
            }
            let s = data[self.position] * std::f32::consts::FRAC_1_SQRT_2;
            *l += s;
            *r += s;
            self.position += 1;
        }
    }
}

/// Atomic state shared lock-free between audio thread and GUI.
///
/// Audio thread writes: current_step, playing, active_pattern, ext_mode, host_playing, tempo, is_daw.
/// GUI thread writes: fill_active, internal_play, standalone_tempo.
pub struct SequencerSync {
    /// Current step position (0-15).
    pub current_step: AtomicUsize,
    /// Whether sequencer is playing (host or internal).
    pub playing: AtomicBool,
    /// Active pattern index (0-15).
    pub active_pattern: AtomicUsize,
    /// Fill mode toggle.
    pub fill_active: AtomicBool,
    /// Internal play toggle — runs sequencer without host transport.
    pub internal_play: AtomicBool,
    /// Whether echo suppression is active — drives "EXT" indicator.
    pub ext_mode: AtomicBool,
    /// Whether host transport is actively driving playback.
    pub host_playing: AtomicBool,
    /// Current tempo (BPM * 10) — written by audio thread.
    pub tempo: AtomicU32,
    /// Standalone tempo override (BPM * 10) — written by GUI.
    pub standalone_tempo: AtomicU32,
    /// Whether running inside a real DAW (host has ever stopped).
    pub is_daw: AtomicBool,
    /// Set by audio thread when state should be persisted; cleared by GUI after serialization.
    pub persist_dirty: AtomicBool,
    /// Master FX automation record arm. GUI toggles; audio thread reads.
    pub rec_armed: AtomicBool,
    /// Touched-since-last-capture counters for the three master FX params.
    /// GUI increments on knob change; audio thread swaps to 0 at each step
    /// boundary and writes the current knob value for any nonzero counter.
    pub fx_touch_rvb: AtomicU32,
    pub fx_touch_dly: AtomicU32,
    pub fx_touch_flt: AtomicU32,
    /// GUI-initiated automation clear for the active pattern. Audio thread
    /// reads and clears.
    pub clr_automation: AtomicBool,
    /// Audio thread sets when it flips the active pattern at a bar boundary.
    /// GUI polls each frame and applies the new pattern's `master_fx_base`
    /// via ParamSetter, then clears the flag.
    pub pattern_fx_apply_pending: AtomicBool,
}

impl SequencerSync {
    pub fn new() -> Self {
        Self {
            current_step: AtomicUsize::new(0),
            playing: AtomicBool::new(false),
            active_pattern: AtomicUsize::new(0),
            fill_active: AtomicBool::new(false),
            internal_play: AtomicBool::new(false),
            ext_mode: AtomicBool::new(false),
            host_playing: AtomicBool::new(false),
            tempo: AtomicU32::new(1200),             // 120.0 BPM
            standalone_tempo: AtomicU32::new(1200),   // 120.0 BPM
            is_daw: AtomicBool::new(false),
            persist_dirty: AtomicBool::new(false),
            rec_armed: AtomicBool::new(false),
            fx_touch_rvb: AtomicU32::new(0),
            fx_touch_dly: AtomicU32::new(0),
            fx_touch_flt: AtomicU32::new(0),
            clr_automation: AtomicBool::new(false),
            pattern_fx_apply_pending: AtomicBool::new(false),
        }
    }
}

pub struct Autokit {
    params: Arc<AutokitParams>,
    sample_rate: f32,
    /// State shared with the GUI thread.
    pub shared: Arc<Mutex<SharedState>>,
    voices: Option<VoicePool>,
    sequencer: Sequencer,
    /// Lockfree trigger counters — incremented each time a pad fires.
    /// Shared with the GUI thread which reads them each frame for activity animation.
    pub trigger_flags: Arc<[AtomicU8; NUM_PADS]>,
    /// GUI-to-audio trigger requests — GUI sets to 1, audio thread reads and clears.
    pub gui_triggers: Arc<[AtomicU8; NUM_PADS]>,
    /// Lightweight preview voice for sample map auditioning.
    preview_voice: PreviewVoice,
    /// Lock-free sequencer state shared with GUI thread.
    pub seq_sync: Arc<SequencerSync>,
    /// Detects MIDI echo from host and suppresses doubled playback.
    echo_detector: EchoDetector,
    /// Internal beat accumulator for free-running mode.
    /// Tracks beats directly (not samples) so tempo changes don't cause position jumps.
    seq_internal_beats: f64,
    /// Counter for periodic state persistence (~1s intervals).
    persist_counter: u64,
    /// Whether initial state restoration is complete — blocks persist until then.
    state_restored: bool,
    /// Whether the host transport has ever reported `playing = false`.
    /// Used to distinguish real DAW transport (which stops/starts) from
    /// standalone backends that always report `playing = true`.
    host_ever_stopped: bool,
    /// Whether internal play was active on the previous buffer — for edge detection.
    seq_internal_play_prev: bool,
    /// Master bus DSP chain (compressor + saturator + limiter).
    master_bus: crate::engine::master_bus::MasterBus,
    /// Master FX bus (DJ filter + delay + reverb). Applied before the
    /// mastering chain so reverb tails get glued by the comp.
    fx_bus: crate::engine::fx::FxBus,
    /// Pre-allocated per-lane FX routing buffers. Voices render into one of
    /// four parallel buses based on per-pad send levels and filter routing:
    /// `dry_bypass_*` (direct, unfiltered), `dry_filter_*` (direct, runs
    /// through the master DJ filter), `send_rvb_*` (reverb send input),
    /// `send_dly_*` (delay send input). The per-sample loop sums the four
    /// buses (with FX applied) before the master bus.
    dry_bypass_l: Vec<f32>,
    dry_bypass_r: Vec<f32>,
    dry_filter_l: Vec<f32>,
    dry_filter_r: Vec<f32>,
    send_rvb_l: Vec<f32>,
    send_rvb_r: Vec<f32>,
    send_dly_l: Vec<f32>,
    send_dly_r: Vec<f32>,
    /// Master FX automation playback smoothers — ease to each step's
    /// recorded target over ~1/8 of a step so pattern sweeps don't zipper.
    aut_rvb_smoother: crate::engine::step_smoother::StepSmoother,
    aut_dly_smoother: crate::engine::step_smoother::StepSmoother,
    aut_flt_smoother: crate::engine::step_smoother::StepSmoother,
    /// Whether the active pattern has automation at the current step for
    /// each master FX param. When true, playback overrides the live knob.
    aut_rvb_active: bool,
    aut_dly_active: bool,
    aut_flt_active: bool,
    /// Last sequencer step we processed for automation record/playback.
    /// `None` means never processed yet (fresh transport).
    last_automation_step: Option<usize>,
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
            sequencer: Sequencer::new(),
            trigger_flags: Arc::new(core::array::from_fn(|_| AtomicU8::new(0))),
            gui_triggers: Arc::new(core::array::from_fn(|_| AtomicU8::new(0))),
            preview_voice: PreviewVoice::new(),
            seq_sync: Arc::new(SequencerSync::new()),
            echo_detector: EchoDetector::new(44100.0),
            seq_internal_beats: 0.0,
            persist_counter: 0,
            state_restored: false,
            host_ever_stopped: false,
            seq_internal_play_prev: false,
            master_bus: crate::engine::master_bus::MasterBus::new(),
            fx_bus: crate::engine::fx::FxBus::new(),
            dry_bypass_l: Vec::new(),
            dry_bypass_r: Vec::new(),
            dry_filter_l: Vec::new(),
            dry_filter_r: Vec::new(),
            send_rvb_l: Vec::new(),
            send_rvb_r: Vec::new(),
            send_dly_l: Vec::new(),
            send_dly_r: Vec::new(),
            aut_rvb_smoother: crate::engine::step_smoother::StepSmoother::new(0.0),
            aut_dly_smoother: crate::engine::step_smoother::StepSmoother::new(0.0),
            aut_flt_smoother: crate::engine::step_smoother::StepSmoother::new(0.0),
            aut_rvb_active: false,
            aut_dly_active: false,
            aut_flt_active: false,
            last_automation_step: None,
            #[cfg(debug_assertions)]
            process_count: 0,
        }
    }
}

/// Populate the kit from the library using the default layout, then update waveforms.
/// Ensures each pad receives a unique sample (best-effort when category has fewer samples than pads).
///
/// Three-pass flow:
/// 1. If the library is empty, return without touching pads — preserves the
///    bundled-default-kit safety net for fresh installs (see commit 619a6af).
/// 2. Clear non-locked pads so bundled defaults from `initialize()` can't leak
///    through when the user's library lacks samples in a given category.
/// 3. Fill each pad with a category-matched sample; if that category is empty
///    in the library, fall back to any unused sample so pads still play.
pub fn populate_kit_from_library(shared: &mut SharedState) {
    let library = shared.library.as_ref().expect("library must be set before populate");
    if library.total == 0 {
        tracing::info!("library empty — leaving bundled default kit in place");
        return;
    }
    let layout = library.generate_kit();

    // Seed exclusion set with paths from locked pads so we don't duplicate those either
    let mut used: HashSet<String> = shared
        .kit
        .pads
        .iter()
        .filter(|p| p.locked)
        .filter_map(|p| p.sample_path.clone())
        .collect();

    // Clear non-locked pads so defaults can't leak into categories the user's
    // library doesn't cover.
    for pad in shared.kit.pads.iter_mut() {
        if !pad.locked {
            pad.sample = None;
            pad.sample_path = None;
            pad.name.clear();
        }
    }

    let mut assigned = 0u32;

    // Pass 1: category-matched fill.
    for (pad_idx, category) in layout {
        if pad_idx >= shared.kit.pads.len() {
            break;
        }
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

    // Pass 2: fallback fill for pads still empty (category absent in library).
    for pad_idx in 0..shared.kit.pads.len() {
        if shared.kit.pads[pad_idx].locked || shared.kit.pads[pad_idx].sample.is_some() {
            continue;
        }
        if let Some(sample) = shared.library.as_ref().unwrap().random_any_excluding(&used) {
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

impl Autokit {
    /// Handle a completed library scan — install pre-built state, update status.
    ///
    /// CRITICAL: This runs on the audio thread. It must not perform any disk
    /// I/O. The scanner thread (see `initialize` and `process`) is responsible
    /// for loading sample audio off-thread; here we only swap pre-built data
    /// into shared state under a brief lock.
    ///
    /// Background: previously this method called `preset::apply_to_kit`, which
    /// opens and decodes every persisted sample file synchronously. Loading a
    /// host project whose samples were missing (different machine, removed
    /// folder, stale automount) would block the audio thread on filesystem
    /// I/O long enough to freeze the host. The fix is to do all I/O upstream,
    /// in `ScanResult::restored`, so this routine is allocation- and I/O-free.
    fn receive_scan_result(&mut self, result: ScanResult) {
        let ScanResult { library, restored } = result;
        tracing::info!(
            total = library.total,
            restored_present = restored.is_some(),
            "scan result received — installing"
        );

        let mut shared = self.shared.lock();

        // Push snapshot before first population for undo support
        let snapshot = HistorySnapshot {
            pads: shared.kit.snapshot(),
            sequencer: self.sequencer.snapshot(),
        };
        shared.history.push(snapshot);
        shared.library = Some(library);

        if let Some(restored) = restored {
            tracing::info!(
                missing = restored.missing_paths.len(),
                "installing pre-loaded restored kit+patterns"
            );
            // Move pre-built kit and pattern bank into shared state. Sample
            // audio is already loaded; we only need to recompute waveform
            // summaries from the in-memory data (no I/O).
            shared.kit = restored.kit;
            shared.pattern_bank = restored.patterns;
            shared.update_all_waveforms(WAVEFORM_POINTS);
            // TODO: surface `restored.missing_paths` in the GUI so the user
            // knows which samples need to be relocated.
        } else {
            // No persisted state — fall back to a fresh kit picked from the
            // newly built library. If the library is empty, this is a no-op
            // and the bundled default kit applied during `initialize()` is
            // left intact so the pads stay playable.
            populate_kit_from_library(&mut shared);
        }

        shared.scan_status = ScanStatus::Ready {
            total: shared.library.as_ref().map(|l| l.total).unwrap_or(0),
        };
        self.state_restored = true;
    }
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

        let result = crate::ui::editor::create(
            self.params.editor_state.clone(),
            shared,
            params,
            Arc::clone(&self.trigger_flags),
            Arc::clone(&self.gui_triggers),
            Arc::clone(&self.seq_sync),
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
        self.master_bus.prepare(self.sample_rate);
        self.fx_bus.prepare(self.sample_rate);
        let max_buf = buffer_config.max_buffer_size as usize;
        self.dry_bypass_l.resize(max_buf, 0.0);
        self.dry_bypass_r.resize(max_buf, 0.0);
        self.dry_filter_l.resize(max_buf, 0.0);
        self.dry_filter_r.resize(max_buf, 0.0);
        self.send_rvb_l.resize(max_buf, 0.0);
        self.send_rvb_r.resize(max_buf, 0.0);
        self.send_dly_l.resize(max_buf, 0.0);
        self.send_dly_r.resize(max_buf, 0.0);

        logging::init();
        tracing::info!(
            "Autokit v{} initialized — sample rate: {}",
            Self::VERSION,
            self.sample_rate
        );

        self.voices = Some(VoicePool::new(self.sample_rate));
        self.echo_detector = EchoDetector::new(self.sample_rate);

        // Fresh-install fallback: if we have neither DAW-persisted state nor
        // a saved standalone session, seed the pads with the bundled default
        // kit so the plugin is immediately playable. This runs BEFORE the
        // scan; `receive_library` / persist restore will overwrite these
        // pads once real library samples arrive.
        {
            let persist_empty = self.params.plugin_state.lock().is_empty();
            let standalone_missing = preset::load_standalone_state().is_none();
            if persist_empty && standalone_missing {
                let mut shared = self.shared.lock();
                default_kit::apply_to_kit(&mut shared, self.sample_rate);
            }
        }

        // Load config and decide whether to scan or show setup dialog
        let cfg = config::Config::load();
        let sample_rate = self.sample_rate;

        if let Some(ref cfg) = cfg {
            let root = PathBuf::from(&cfg.sample_library_root);
            if root.is_dir() {
                // Config exists and path is valid — scan immediately
                let (tx, rx) = crossbeam_channel::bounded::<ScanResult>(1);

                let progress = Arc::new(ScanProgress {
                    processed: AtomicU32::new(0),
                    total: AtomicU32::new(0),
                });
                {
                    let mut shared = self.shared.lock();
                    shared.scan_progress = Some(Arc::clone(&progress));
                    shared.bg_rx = Some(rx);
                }

                // Clone the persisted-state Arc into the worker so it can do
                // preset deserialization + sample loading off the audio thread.
                // This is the critical fix for the host-freeze bug when the
                // saved project references samples missing on this machine.
                let plugin_state = Arc::clone(&self.params.plugin_state);

                if let Err(e) = std::thread::Builder::new()
                    .name("autokit-scanner".to_string())
                    .spawn(move || {
                        tracing::info!("background scan starting");
                        let library = SampleLibrary::build_with_progress(&root, sample_rate, Some(&progress));
                        // After the library is built, do the heavy state
                        // restoration here — never on the audio thread.
                        let persisted = plugin_state.lock().clone();
                        let restored = preset::restore_persisted_off_thread(&persisted, sample_rate);
                        if tx.send(ScanResult { library, restored }).is_err() {
                            tracing::warn!("plugin dropped before scan completed");
                        }
                    })
                {
                    tracing::error!("failed to spawn scanner thread: {e}");
                    let mut shared = self.shared.lock();
                    shared.scan_status = ScanStatus::Ready { total: 0 };
                }
            } else {
                tracing::warn!(path = %cfg.sample_library_root, "configured sample path not found");
                let mut shared = self.shared.lock();
                shared.scan_status = ScanStatus::NeedsSetup {
                    suggested_path: config::discover_sample_root(),
                };
            }
        } else {
            // No config — check if default path exists for auto-discovery
            let discovered = config::discover_sample_root();
            let mut shared = self.shared.lock();
            shared.scan_status = ScanStatus::NeedsSetup {
                suggested_path: discovered,
            };
            tracing::info!("no config found — showing setup dialog");
        }

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Check for background thread messages (non-blocking)
        {
            let result = self.shared.try_lock().and_then(|s| {
                s.bg_rx.as_ref().and_then(|rx| rx.try_recv().ok())
            });
            if let Some(result) = result {
                permit_alloc(|| {
                    self.receive_scan_result(result);
                });
            }
        }

        // Check if GUI requested a (re)scan
        {
            let scan_path = self.shared.try_lock().and_then(|mut s| s.pending_scan_path.take());
            if let Some(root) = scan_path {
                permit_alloc(|| {
                    let (tx, rx) = crossbeam_channel::bounded::<ScanResult>(1);
                    let sample_rate = self.sample_rate;

                    let progress = Arc::new(ScanProgress {
                        processed: AtomicU32::new(0),
                        total: AtomicU32::new(0),
                    });
                    if let Some(mut s) = self.shared.try_lock() {
                        s.scan_progress = Some(Arc::clone(&progress));
                        s.bg_rx = Some(rx);
                    }

                    if let Err(e) = std::thread::Builder::new()
                        .name("autokit-scanner".to_string())
                        .spawn(move || {
                            tracing::info!(path = %root.display(), "background scan starting (from GUI)");
                            let library = SampleLibrary::build_with_progress(&root, sample_rate, Some(&progress));
                            // GUI-triggered rescan: kit/patterns are already
                            // live in memory; do not re-restore from persisted
                            // JSON (which would clobber unsaved edits).
                            if tx.send(ScanResult { library, restored: None }).is_err() {
                                tracing::warn!("plugin dropped before scan completed");
                            }
                        })
                    {
                        tracing::error!("failed to spawn scanner thread: {e}");
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
        if let Some(mut shared) = got_lock {
            // Advance echo detector clock
            self.echo_detector.tick(num_samples);

            // Check for preview sample request
            if let Some(preview_data) = shared.preview_sample.take() {
                self.preview_voice.start(preview_data);
            }

            // Sync sequencer fill state from GUI
            self.sequencer.fill_active = self.seq_sync.fill_active.load(Ordering::Relaxed);

            // Sequencer play state:
            // 1. Internal PLAY button / Space bar: free-run using internal sample counter
            // 2. Host transport playing: follow host position (works in DAWs)
            // 3. Neither: sequencer stopped
            // Sequencer play state — two modes:
            // 1. Internal PLAY (Space / button): free-run using internal sample counter.
            // 2. Host transport: follow DAW position when the host is playing.
            //
            // Standalone backends (CPAL, JACK via PipeWire) report playing=true even
            // when nothing is driving the transport.  To avoid auto-playing the
            // sequencer we track whether the host has ever reported playing=false.
            // Real DAW hosts toggle transport on/off; standalone backends never stop.
            let transport = context.transport();
            if !transport.playing {
                self.host_ever_stopped = true;
            }
            self.seq_sync.is_daw.store(self.host_ever_stopped, Ordering::Relaxed);
            let internal_play = self.seq_sync.internal_play.load(Ordering::Relaxed);
            let host_driving = self.host_ever_stopped && transport.playing && !internal_play;
            self.seq_sync.host_playing.store(host_driving, Ordering::Relaxed);

            // Edge detection: reset internal counter when internal play is toggled on
            if internal_play && !self.seq_internal_play_prev {
                self.seq_internal_beats = 0.0;
                self.sequencer.reset_position();
            }
            self.seq_internal_play_prev = internal_play;

            let (playing, tempo, pos_beats) = if internal_play {
                // Free-running — use standalone tempo if no host, otherwise host tempo
                let t = if !self.host_ever_stopped {
                    self.seq_sync.standalone_tempo.load(Ordering::Relaxed) as f64 / 10.0
                } else {
                    transport.tempo.unwrap_or(120.0)
                };
                // Accumulate beats incrementally so tempo changes don't jump position
                let beats = self.seq_internal_beats;
                self.seq_internal_beats += num_samples as f64 / self.sample_rate as f64 * (t / 60.0);
                (true, Some(t), Some(beats))
            } else if host_driving {
                self.seq_internal_beats = 0.0;
                (true, transport.tempo, transport.pos_beats())
            } else {
                self.seq_internal_beats = 0.0;
                (false, transport.tempo, None)
            };

            // Store current tempo for GUI display
            let display_tempo = tempo.unwrap_or(120.0);
            self.seq_sync.tempo.store((display_tempo * 10.0) as u32, Ordering::Relaxed);

            permit_alloc(|| {
                tracing::debug!(
                    internal_play,
                    host_playing = transport.playing,
                    host_ever_stopped = self.host_ever_stopped,
                    host_driving,
                    ?tempo,
                    ?pos_beats,
                    internal_beats = self.seq_internal_beats,
                    "transport decision"
                );
            });

            // --- Sequencer fires FIRST so echo detector can record outgoing notes ---
            // Capture trigger counts before sequencer runs
            let pre_triggers: [u8; NUM_PADS] = core::array::from_fn(|i| {
                self.trigger_flags[i].load(Ordering::Relaxed)
            });

            // Run sequencer with pattern data from SharedState
            // Split borrow: kit (immutable) and pattern_bank (mutable) are separate fields
            let shared_ref = &mut *shared;
            let triggered = self.sequencer.process_buffer_with_patterns(
                num_samples,
                playing,
                tempo,
                pos_beats,
                self.sample_rate,
                voices,
                &shared_ref.kit,
                &mut shared_ref.pattern_bank,
                &self.trigger_flags,
            );
            if triggered > 0 {
                let step = self.sequencer.current_step();
                permit_alloc(|| {
                    tracing::debug!(triggered, step, "sequencer fired");
                });
            }

            // Send MIDI output and record in echo detector BEFORE processing incoming MIDI
            for i in 0..NUM_PADS {
                let post = self.trigger_flags[i].load(Ordering::Relaxed);
                if post != pre_triggers[i] {
                    let note = shared.kit.note_for_pad(i);
                    let velocity = {
                        let pattern = shared.pattern_bank.active_pattern();
                        if i < pattern.lanes.len() {
                            let step_idx = self.sequencer.current_step();
                            pattern.lanes[i].steps[step_idx].velocity
                        } else {
                            0.8
                        }
                    };
                    self.echo_detector.record(note);
                    context.send_event(NoteEvent::NoteOn {
                        timing: 0,
                        voice_id: None,
                        channel: 9,
                        note,
                        velocity,
                    });
                    context.send_event(NoteEvent::NoteOff {
                        timing: 1,
                        voice_id: None,
                        channel: 9,
                        note,
                        velocity: 0.0,
                    });
                }
            }

            // --- Now process incoming MIDI — echoed notes will be suppressed ---
            while let Some(event) = context.next_event() {
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        if let Some(pad_idx) = shared.kit.pad_for_note(note) {
                            if !self.echo_detector.check(note) {
                                let (lr, ld, lf) = shared.pattern_bank.active_pattern()
                                    .lanes.get(pad_idx)
                                    .map(|l| (l.fx_send_rvb, l.fx_send_dly, l.fx_filter))
                                    .unwrap_or((0.0, 0.0, false));
                                voices.trigger(pad_idx, velocity, &shared.kit, 0, None, None, None, None, None, lr, ld, lf);
                                self.trigger_flags[pad_idx].fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    NoteEvent::NoteOff { .. } => {}
                    _ => {}
                }
            }

            // Check GUI trigger requests (keyboard/click-to-play)
            for i in 0..NUM_PADS {
                if self.gui_triggers[i].swap(0, Ordering::Relaxed) != 0 {
                    let (lr, ld, lf) = shared.pattern_bank.active_pattern()
                        .lanes.get(i)
                        .map(|l| (l.fx_send_rvb, l.fx_send_dly, l.fx_filter))
                        .unwrap_or((0.0, 0.0, false));
                    voices.trigger(i, 0.8, &shared.kit, 0, None, None, None, None, None, lr, ld, lf);
                    self.trigger_flags[i].fetch_add(1, Ordering::Relaxed);
                }
            }

            // Sync echo suppression state for GUI
            self.seq_sync.ext_mode.store(self.echo_detector.is_suppressing(), Ordering::Relaxed);

            // ── Master FX automation: record + playback override targets ──
            //
            // Runs while the `shared` lock is held so we can mutate the active
            // pattern's `master_automation` in place. The per-sample master
            // loop below runs OUTSIDE the lock and only reads the cached
            // StepSmoother state, so no lock contention there.
            //
            // Step-boundary detection is block-granular: we walk from
            // `last_automation_step+1` through the sequencer's current step.
            // At the typical buffer sizes (< 1 step at 120 BPM), only one
            // step is crossed per buffer, so the ~one-buffer latency is
            // below the audible threshold and is hidden by the smoothers'
            // 1/8-of-a-step ramp.
            if shared.pattern_bank.patterns.get(shared.pattern_bank.active).is_some() {
                // Handle CLR request from GUI — wipe active pattern's automation.
                if self.seq_sync.clr_automation.swap(false, Ordering::Relaxed) {
                    shared.pattern_bank.active_pattern_mut().master_automation.clear();
                    self.aut_rvb_active = false;
                    self.aut_dly_active = false;
                    self.aut_flt_active = false;
                }

                let rec_armed = self.seq_sync.rec_armed.load(Ordering::Relaxed);
                let seq_playing = self.sequencer.is_playing();
                let current_step = self.sequencer.current_step();
                let step_changed = self.last_automation_step != Some(current_step);

                if seq_playing && step_changed {
                    let tempo_bpm_now = self.seq_sync.tempo.load(Ordering::Relaxed) as f32 / 10.0;
                    let ramp = crate::engine::step_smoother::ramp_samples_for_tempo(
                        self.sample_rate, tempo_bpm_now,
                    );

                    // Build the list of crossed step indices (inclusive of current).
                    let mut crossed: [usize; 16] = [0; 16];
                    let mut n = 0usize;
                    match self.last_automation_step {
                        None => {
                            crossed[0] = current_step;
                            n = 1;
                        }
                        Some(prev) => {
                            let mut s = (prev + 1) % 16;
                            loop {
                                crossed[n] = s;
                                n += 1;
                                if s == current_step || n == 16 { break; }
                                s = (s + 1) % 16;
                            }
                        }
                    }

                    // Record: on REC armed, swap touch counters once and
                    // write the live knob value into every crossed step for
                    // each param that was touched since the last step.
                    if rec_armed {
                        let touched_rvb = self.seq_sync.fx_touch_rvb.swap(0, Ordering::Relaxed) != 0;
                        let touched_dly = self.seq_sync.fx_touch_dly.swap(0, Ordering::Relaxed) != 0;
                        let touched_flt = self.seq_sync.fx_touch_flt.swap(0, Ordering::Relaxed) != 0;
                        if touched_rvb || touched_dly || touched_flt {
                            let rvb_v = self.params.reverb_mix.unmodulated_plain_value();
                            let dly_v = self.params.delay_mix.unmodulated_plain_value();
                            let flt_v = self.params.dj_filter.unmodulated_plain_value();
                            let pat = shared.pattern_bank.active_pattern_mut();
                            for &s in &crossed[..n] {
                                if touched_rvb { pat.master_automation.reverb_mix[s] = Some(rvb_v); }
                                if touched_dly { pat.master_automation.delay_mix[s] = Some(dly_v); }
                                if touched_flt { pat.master_automation.dj_filter[s] = Some(flt_v); }
                            }
                        }
                    }

                    // Playback override: arm smoothers to the current step's
                    // recorded values. REC armed disables override so the user
                    // hears exactly what they're playing in.
                    let pat = shared.pattern_bank.active_pattern();
                    let auto_enabled = !rec_armed;

                    if auto_enabled {
                        if let Some(v) = pat.master_automation.reverb_mix[current_step] {
                            if !self.aut_rvb_active {
                                self.aut_rvb_smoother.reset(self.params.reverb_mix.unmodulated_plain_value());
                            }
                            self.aut_rvb_smoother.set_target_now(v, ramp);
                            self.aut_rvb_active = true;
                        } else {
                            self.aut_rvb_active = false;
                        }
                        if let Some(v) = pat.master_automation.delay_mix[current_step] {
                            if !self.aut_dly_active {
                                self.aut_dly_smoother.reset(self.params.delay_mix.unmodulated_plain_value());
                            }
                            self.aut_dly_smoother.set_target_now(v, ramp);
                            self.aut_dly_active = true;
                        } else {
                            self.aut_dly_active = false;
                        }
                        if let Some(v) = pat.master_automation.dj_filter[current_step] {
                            if !self.aut_flt_active {
                                self.aut_flt_smoother.reset(self.params.dj_filter.unmodulated_plain_value());
                            }
                            self.aut_flt_smoother.set_target_now(v, ramp);
                            self.aut_flt_active = true;
                        } else {
                            self.aut_flt_active = false;
                        }
                    } else {
                        self.aut_rvb_active = false;
                        self.aut_dly_active = false;
                        self.aut_flt_active = false;
                    }

                    self.last_automation_step = Some(current_step);
                } else if !seq_playing {
                    // Transport stopped — clear overrides and reset step tracker.
                    self.aut_rvb_active = false;
                    self.aut_dly_active = false;
                    self.aut_flt_active = false;
                    self.last_automation_step = None;
                    // Also drain touch counters so the next REC pass starts clean.
                    self.seq_sync.fx_touch_rvb.store(0, Ordering::Relaxed);
                    self.seq_sync.fx_touch_dly.store(0, Ordering::Relaxed);
                    self.seq_sync.fx_touch_flt.store(0, Ordering::Relaxed);
                }
            }

            // Periodic state persistence for DAW save/load (~every 1s)
            // Audio thread sets a dirty flag; GUI thread does the actual serialization.
            if self.state_restored {
                self.persist_counter += num_samples as u64;
            }
            if self.state_restored && self.persist_counter >= self.sample_rate as u64 {
                self.persist_counter = 0;
                self.seq_sync.persist_dirty.store(true, Ordering::Relaxed);
            }

            // Write playback state for GUI
            self.seq_sync.current_step.store(self.sequencer.current_step(), Ordering::Relaxed);
            self.seq_sync.playing.store(self.sequencer.is_playing(), Ordering::Relaxed);
            let new_active = shared.pattern_bank.active;
            let prev_active = self.seq_sync.active_pattern.swap(new_active, Ordering::Relaxed);
            if prev_active != new_active {
                self.seq_sync.pattern_fx_apply_pending.store(true, Ordering::Relaxed);
            }

            // permit_alloc for drop: parking_lot unlock_slow() may allocate.
            permit_alloc(|| drop(shared));
        }

        // Render voices OUTSIDE the lock — pan is cached at trigger time,
        // so voices never need the kit reference. No more silence on lock contention.
        let channels = buffer.as_slice();
        if channels.len() >= 2 {
            let (left_channels, right_channels) = channels.split_at_mut(1);
            let output_left = &mut left_channels[0][..num_samples];
            let output_right = &mut right_channels[0][..num_samples];
            output_left.fill(0.0);
            output_right.fill(0.0);

            // Zero the four routing buses, then render voices into them.
            // Voices carry resolved FX sends in their own fields (pad default
            // or step plock override, captured at trigger time), so no
            // per-buffer kit snapshot is needed.
            let dry_bypass_l = &mut self.dry_bypass_l[..num_samples];
            let dry_bypass_r = &mut self.dry_bypass_r[..num_samples];
            let dry_filter_l = &mut self.dry_filter_l[..num_samples];
            let dry_filter_r = &mut self.dry_filter_r[..num_samples];
            let send_rvb_l = &mut self.send_rvb_l[..num_samples];
            let send_rvb_r = &mut self.send_rvb_r[..num_samples];
            let send_dly_l = &mut self.send_dly_l[..num_samples];
            let send_dly_r = &mut self.send_dly_r[..num_samples];
            dry_bypass_l.fill(0.0);
            dry_bypass_r.fill(0.0);
            dry_filter_l.fill(0.0);
            dry_filter_r.fill(0.0);
            send_rvb_l.fill(0.0);
            send_rvb_r.fill(0.0);
            send_dly_l.fill(0.0);
            send_dly_r.fill(0.0);

            voices.process_sends(
                dry_bypass_l, dry_bypass_r,
                dry_filter_l, dry_filter_r,
                send_rvb_l, send_rvb_r,
                send_dly_l, send_dly_r,
            );
            // Preview voice is always dry-bypass — auditioning samples
            // shouldn't pick up whatever FX the user has loaded.
            self.preview_voice.process(dry_bypass_l, dry_bypass_r);

            // Per-sample master loop: sum the four buses through FX + mastering.
            // nih-plug's built-in smoothers ramp params within each buffer,
            // so DAW automation and GUI knob moves sound continuous.
            let lim_on = self.params.limiter_on.value();
            let tempo_bpm = self.seq_sync.tempo.load(Ordering::Relaxed) as f32 / 10.0;
            let sr = self.sample_rate;
            // delay_time knob is quantized to {1/32, 1/16, 1/8, 1/4} note —
            // evaluated once per buffer so the mapping doesn't jitter between
            // divisions while the smoother ramps.
            let delay_time_raw = self.params.delay_time.unmodulated_plain_value();
            let delay_division = match (delay_time_raw * 3.0).round() as i32 {
                0 => 0.125, // 1/32
                1 => 0.25,  // 1/16
                2 => 0.5,   // 1/8
                _ => 1.0,   // 1/4
            };
            let delay_samples = (60.0 / tempo_bpm.max(1.0)) * sr * delay_division;
            for i in 0..num_samples {
                let threshold_db = self.params.comp_threshold.smoothed.next();
                let drive = self.params.comp_drive.smoothed.next();
                let master_gain = self.params.master_volume.smoothed.next();
                // Always advance the param smoothers so they stay in sync
                // for the moment automation turns off (avoids a jump).
                let rvb_return_live = self.params.reverb_mix.smoothed.next();
                let dly_return_live = self.params.delay_mix.smoothed.next();
                let filter_knob_live = self.params.dj_filter.smoothed.next();
                let rvb_return = if self.aut_rvb_active {
                    self.aut_rvb_smoother.next()
                } else { rvb_return_live };
                let dly_return = if self.aut_dly_active {
                    self.aut_dly_smoother.next()
                } else { dly_return_live };
                let filter_knob = if self.aut_flt_active {
                    self.aut_flt_smoother.next()
                } else { filter_knob_live };

                // Reverb + delay returns (pure-wet processors).
                let (rl_wet, rr_wet) = self.fx_bus.reverb.process_sample(
                    send_rvb_l[i], send_rvb_r[i],
                );
                let (dl_wet, dr_wet) = self.fx_bus.delay.process_sample(
                    send_dly_l[i], send_dly_r[i], delay_samples,
                );

                // FX returns feed into the filter bus so the master DJ filter
                // sweeps the wet tails alongside any lanes that opted in
                // (F toggle). At knob=0 the filter is a unity bypass, so
                // routing returns through it is free when unused.
                let filt_in_l = dry_filter_l[i] + rl_wet * rvb_return + dl_wet * dly_return;
                let filt_in_r = dry_filter_r[i] + rr_wet * rvb_return + dr_wet * dly_return;
                let (fl_out, fr_out) = self.fx_bus.dj_filter.process_sample(
                    filt_in_l, filt_in_r, filter_knob,
                );

                // Final mix: direct-bypass path (lanes with F off) + filter output.
                let mix_l = dry_bypass_l[i] + fl_out;
                let mix_r = dry_bypass_r[i] + fr_out;

                let (l, r) = self.master_bus.process_sample(
                    mix_l, mix_r, threshold_db, drive, lim_on,
                );
                output_left[i] = l * master_gain;
                output_right[i] = r * master_gain;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::features::AudioFeatures;
    use crate::analysis::library::AnalyzedSample;
    use crate::analysis::scanner::SampleEntry;
    use crate::engine::kit::{DrumKit, SampleCategory};
    use crate::ui::state::SharedState;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Regression test: bundled-default sample names must not leak into pad
    /// slots after the user scans a folder of uncategorized samples.
    ///
    /// Scenario:
    /// 1. Initialize SharedState with empty kit (no defaults yet)
    /// 2. Pre-populate pads with bundled default samples (simulating plugin init)
    /// 3. Build a SampleLibrary with 10 uncategorized (Other) samples
    /// 4. Call populate_kit_from_library
    /// 5. Assert: every non-locked pad has a sample_path from the user's library,
    ///    NOT from the bundled defaults
    #[test]
    fn populate_kit_fills_all_pads_from_uncategorized_library() {
        // Step 1: Create SharedState with fresh kit
        let mut shared = SharedState::new();

        // Step 2: Pre-populate with bundled defaults (simulate apply_to_kit behavior)
        // Note: bundled defaults have Some(sample) but None for sample_path
        let default_names = [
            "bd_909.wav",
            "sd_1.wav",
            "hh_1.wav",
            "hh_2.wav",
            "808.wav",
            "tom.wav",
            "hitom.wav",
            "bd_psy.wav",
        ];
        let default_categories = [
            SampleCategory::Kick,
            SampleCategory::Snare,
            SampleCategory::Hihat,
            SampleCategory::Hihat,
            SampleCategory::Bass,
            SampleCategory::Tom,
            SampleCategory::Tom,
            SampleCategory::Kick,
        ];

        for (pad_idx, (name, category)) in default_names
            .iter()
            .zip(default_categories.iter())
            .enumerate()
        {
            let pad = &mut shared.kit.pads[pad_idx];
            pad.name = name.to_string();
            pad.category = *category;
            pad.sample = Some(Arc::new(vec![0.1f32; 4410])); // dummy audio data
            pad.sample_path = None; // bundled samples have NO filesystem path
        }

        // Step 3: Build a library with 10 uncategorized samples (all Other category)
        let mut by_category: HashMap<SampleCategory, Vec<AnalyzedSample>> = HashMap::new();
        let test_samples = [
            "user_sample_01.wav",
            "user_sample_02.wav",
            "user_sample_03.wav",
            "user_sample_04.wav",
            "user_sample_05.wav",
            "user_sample_06.wav",
            "user_sample_07.wav",
            "user_sample_08.wav",
            "user_sample_09.wav",
            "user_sample_10.wav",
        ];

        for sample_name in &test_samples {
            let entry = SampleEntry {
                path: PathBuf::from(format!("/user/uncategorized/{}", sample_name)),
                filename: sample_name.to_string(),
                category: SampleCategory::Other, // Uncategorized — the key condition
                folder_hint: None,
                duration_ms: 500,
                is_percussive: true,
            };

            let sample = AnalyzedSample {
                entry,
                features: AudioFeatures {
                    attack_time: 0.005,
                    decay_time: 0.2,
                    spectral_centroid: 2000.0,
                    spectral_flatness: 0.6,
                    sub_energy_ratio: 0.2,
                    high_freq_ratio: 0.15,
                    peak: 0.8,
                    duration: 0.5,
                    is_percussive: true,
                },
                data: Arc::new(vec![0.5f32; 22050]), // dummy audio
            };

            by_category
                .entry(SampleCategory::Other)
                .or_default()
                .push(sample);
        }

        let library = SampleLibrary {
            total: 10,
            by_category,
            sample_rate: 44100.0,
        };

        shared.library = Some(library);

        // Step 4: Call populate_kit_from_library
        populate_kit_from_library(&mut shared);

        // Step 5: Verify no bundled defaults leaked through
        for (pad_idx, pad) in shared.kit.pads.iter().enumerate() {
            if pad.locked {
                continue; // Skip locked pads as per test requirements
            }

            // Assert: pad must have a sample
            assert!(
                pad.sample.is_some(),
                "pad {} should have sample data assigned",
                pad_idx
            );

            // Assert: pad must have a sample_path (crucial — bundled had None)
            assert!(
                pad.sample_path.is_some(),
                "pad {} should have a sample_path (none means bundled default leaked)",
                pad_idx
            );

            // Assert: sample_path must come from the user's library, not bundled
            let path = pad.sample_path.as_ref().unwrap();
            assert!(
                path.contains("user/uncategorized/"),
                "pad {} sample_path '{}' should come from user library, not bundled defaults",
                pad_idx,
                path
            );
        }

        // Step 6: Verify all assigned paths are unique (no duplicates)
        let mut paths: Vec<String> = shared
            .kit
            .pads
            .iter()
            .filter(|p| !p.locked)
            .filter_map(|p| p.sample_path.clone())
            .collect();

        let unique_count = {
            let mut unique_set = HashSet::new();
            for path in &paths {
                unique_set.insert(path.clone());
            }
            unique_set.len()
        };

        assert_eq!(
            unique_count, paths.len(),
            "all assigned sample paths should be unique (no duplicates)"
        );
        assert!(
            paths.len() > 0,
            "at least some pads should be assigned from the library"
        );
    }
}
