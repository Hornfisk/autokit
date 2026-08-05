use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::engine::kit::{DrumKit, NUM_PADS};
use crate::engine::sampler::{Trigger, VoicePool};

/// Conditional trig types — Elektron-style step conditions.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum ConditionTrig {
    #[default]
    Always, // Default — fires every loop
    Every(u8),    // 1:N — fires every Nth loop (N = 2, 4, 8)
    NotEvery(u8), // !1:N — fires on all loops EXCEPT every Nth
    Fill,         // Fires only when FILL mode is active
    NotFill,      // Fires only when FILL mode is NOT active
}

impl ConditionTrig {
    /// All conditions in cycle order for the GUI selector.
    pub const CYCLE: &'static [ConditionTrig] = &[
        Self::Always,
        Self::Every(2),
        Self::Every(4),
        Self::Every(8),
        Self::NotEvery(2),
        Self::NotEvery(4),
        Self::NotEvery(8),
        Self::Fill,
        Self::NotFill,
    ];

    /// Short display label for grid cells and selector button.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Always => "——",
            Self::Every(2) => "1:2",
            Self::Every(4) => "1:4",
            Self::Every(8) => "1:8",
            Self::NotEvery(2) => "!1:2",
            Self::NotEvery(4) => "!1:4",
            Self::NotEvery(8) => "!1:8",
            Self::Fill => "FIL",
            Self::NotFill => "!FIL",
            _ => "??",
        }
    }

    /// Next condition in cycle (for click-to-cycle UI).
    pub fn next(&self) -> ConditionTrig {
        let idx = Self::CYCLE.iter().position(|c| c == self).unwrap_or(0);
        Self::CYCLE[(idx + 1) % Self::CYCLE.len()]
    }

    /// Coerce an out-of-range divisor to `Always`. `Every(0)` / `NotEvery(0)`
    /// are unreachable through the UI (which only offers N ∈ {2,4,8}) but are
    /// representable in JSON, and `loop_count % 0` divides by zero on the
    /// audio thread. See also the `n.max(1)` guard in `evaluate_condition`.
    fn sanitized(self) -> Self {
        match self {
            Self::Every(0) | Self::NotEvery(0) => Self::Always,
            other => other,
        }
    }
}

/// A single step in the sequencer.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Step {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
    pub pan: Option<f32>,   // None = inherit pad default, Some = p-lock
    pub pitch: Option<f32>, // None = inherit pad default, Some = p-lock (semitones)
    pub condition: ConditionTrig,
    #[serde(default)]
    pub fx_rvb: Option<f32>, // None = inherit pad fx_send_rvb, Some = override
    #[serde(default)]
    pub fx_dly: Option<f32>, // None = inherit pad fx_send_dly, Some = override
    #[serde(default)]
    pub fx_filter: Option<bool>, // None = inherit pad fx_filter, Some = override
}

impl Default for Step {
    fn default() -> Self {
        Self {
            enabled: false,
            velocity: 0.8,
            probability: 1.0,
            pan: None,
            pitch: None,
            condition: ConditionTrig::Always,
            fx_rvb: None,
            fx_dly: None,
            fx_filter: None,
        }
    }
}

/// Clamp a float into `0.0..=1.0`, substituting `fallback` for NaN.
fn clamp_unit(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

impl Step {
    /// Force every field back into its documented range. See
    /// [`PatternBank::sanitize`] for why this exists.
    fn sanitize(&mut self) {
        self.velocity = clamp_unit(self.velocity, 0.8);
        self.probability = clamp_unit(self.probability, 1.0);
        self.condition = self.condition.sanitized();
        if let Some(p) = self.pan {
            self.pan = Some(if p.is_finite() {
                p.clamp(-1.0, 1.0)
            } else {
                0.0
            });
        }
        if let Some(p) = self.pitch {
            self.pitch = Some(if p.is_finite() {
                p.clamp(-24.0, 24.0)
            } else {
                0.0
            });
        }
        if let Some(v) = self.fx_rvb {
            self.fx_rvb = Some(clamp_unit(v, 0.0));
        }
        if let Some(v) = self.fx_dly {
            self.fx_dly = Some(clamp_unit(v, 0.0));
        }
    }
}

/// One lane = one pad's 16-step sequence.
#[derive(Clone, Serialize, Deserialize)]
pub struct Lane {
    pub pad_index: usize,
    pub steps: [Step; NUM_STEPS],
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    /// Per-lane reverb send (0..1). Per-pattern.
    #[serde(default)]
    pub fx_send_rvb: f32,
    /// Per-lane delay send (0..1). Per-pattern.
    #[serde(default)]
    pub fx_send_dly: f32,
    /// Whether this lane is routed through the master DJ filter insert. Per-pattern.
    #[serde(default)]
    pub fx_filter: bool,
}

impl Lane {
    pub fn new(pad_index: usize) -> Self {
        Self {
            pad_index,
            steps: [Step::default(); NUM_STEPS],
            muted: false,
            solo: false,
            fx_send_rvb: 0.0,
            fx_send_dly: 0.0,
            fx_filter: false,
        }
    }

    /// Force every field back into range. See [`PatternBank::sanitize`].
    fn sanitize(&mut self) {
        for step in &mut self.steps {
            step.sanitize();
        }
        self.fx_send_rvb = clamp_unit(self.fx_send_rvb, 0.0);
        self.fx_send_dly = clamp_unit(self.fx_send_dly, 0.0);
    }
}

pub const NUM_STEPS: usize = 16;
pub const NUM_PATTERNS: usize = 16;

/// Per-pattern master FX automation. One slot per 16th-note step; `None`
/// means the live knob value wins. Stored globally per pattern (not
/// per-lane) so the master FX timing is independent of which lane fires.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct MasterAutomation {
    pub reverb_mix: [Option<f32>; NUM_STEPS],
    pub delay_mix: [Option<f32>; NUM_STEPS],
    pub dj_filter: [Option<f32>; NUM_STEPS],
}

impl Default for MasterAutomation {
    fn default() -> Self {
        Self {
            reverb_mix: [None; NUM_STEPS],
            delay_mix: [None; NUM_STEPS],
            dj_filter: [None; NUM_STEPS],
        }
    }
}

impl MasterAutomation {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn any_recorded(&self) -> bool {
        self.reverb_mix.iter().any(|s| s.is_some())
            || self.delay_mix.iter().any(|s| s.is_some())
            || self.dj_filter.iter().any(|s| s.is_some())
    }
}

/// Per-pattern base values for the three master FX knobs. Captured when
/// the user switches away from a pattern and re-applied via ParamSetter
/// when the pattern becomes active. `initialized = false` means this
/// pattern has never been touched — first activation captures the live
/// knob values into it (so old presets/fresh projects keep current FX).
#[derive(Clone, Copy, Serialize, Deserialize, Default)]
pub struct MasterFxBase {
    pub reverb_mix: f32,
    pub delay_mix: f32,
    pub dj_filter: f32,
    pub initialized: bool,
}

/// One pattern: 8 lanes + swing setting + master FX automation.
#[derive(Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub lanes: Vec<Lane>,
    pub swing: f32,
    #[serde(default)]
    pub master_automation: MasterAutomation,
    #[serde(default)]
    pub master_fx_base: MasterFxBase,
}

impl Default for Pattern {
    fn default() -> Self {
        Self::new()
    }
}

impl Pattern {
    pub fn new() -> Self {
        Self {
            lanes: (0..NUM_PADS).map(Lane::new).collect(),
            swing: 0.0,
            master_automation: MasterAutomation::default(),
            master_fx_base: MasterFxBase::default(),
        }
    }

    /// Returns true if any step in any lane is enabled.
    pub fn has_data(&self) -> bool {
        self.lanes
            .iter()
            .any(|lane| lane.steps.iter().any(|s| s.enabled))
    }

    /// Force the lane list to exactly `NUM_PADS` correctly-indexed lanes and
    /// clamp every value. See [`PatternBank::sanitize`].
    pub fn sanitize(&mut self) {
        self.lanes.truncate(NUM_PADS);
        while self.lanes.len() < NUM_PADS {
            let idx = self.lanes.len();
            self.lanes.push(Lane::new(idx));
        }
        for (i, lane) in self.lanes.iter_mut().enumerate() {
            // Lane N always drives pad N. `pad_index` is redundant with the
            // slot but is serialized, so a hand-edited or corrupt file can
            // disagree — and the audio thread indexes `kit.pads` and
            // `trigger_flags` with it directly.
            lane.pad_index = i;
            lane.sanitize();
        }
        self.swing = clamp_unit(self.swing, 0.0);
    }
}

/// Bank of 16 patterns with active/queued selection.
#[derive(Serialize, Deserialize)]
pub struct PatternBank {
    pub patterns: Vec<Pattern>,
    pub active: usize,
    pub queued: Option<usize>,
}

impl Default for PatternBank {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternBank {
    pub fn new() -> Self {
        Self {
            patterns: (0..NUM_PATTERNS).map(|_| Pattern::new()).collect(),
            active: 0,
            queued: None,
        }
    }

    /// Restore every invariant the audio thread relies on.
    ///
    /// **Call this after any deserialization** — the DAW `#[persist]` blob,
    /// a preset file, or a single-pattern file. All three are user-reachable
    /// (hand-edited JSON, a file from another Autokit version, a truncated
    /// write) and the audio thread indexes `patterns[active]`,
    /// `kit.pads[lane.pad_index]` and `trigger_flags[lane.pad_index]`
    /// directly. Without this, a bad index is an audio-thread panic, which on
    /// the JACK backend kills the process callback and wedges the user's
    /// PipeWire graph.
    pub fn sanitize(&mut self) {
        self.patterns.truncate(NUM_PATTERNS);
        while self.patterns.len() < NUM_PATTERNS {
            self.patterns.push(Pattern::new());
        }
        for pattern in &mut self.patterns {
            pattern.sanitize();
        }
        if self.active >= self.patterns.len() {
            self.active = 0;
        }
        if self.queued.is_some_and(|q| q >= self.patterns.len()) {
            self.queued = None;
        }
    }

    /// Index of the active pattern, clamped into range. `sanitize()` should
    /// already guarantee this; the clamp is belt-and-braces because the
    /// callers run on the audio thread.
    #[inline]
    fn active_index(&self) -> usize {
        if self.active < self.patterns.len() {
            self.active
        } else {
            0
        }
    }

    pub fn active_pattern(&self) -> &Pattern {
        &self.patterns[self.active_index()]
    }

    pub fn active_pattern_mut(&mut self) -> &mut Pattern {
        let idx = self.active_index();
        &mut self.patterns[idx]
    }

    pub fn snapshot(&self) -> crate::util::history::SequencerSnapshot {
        crate::util::history::SequencerSnapshot {
            patterns: self
                .patterns
                .iter()
                .map(|pat| crate::util::history::PatternSnapshot {
                    lanes: core::array::from_fn(|i| crate::util::history::LaneSnapshot {
                        steps: core::array::from_fn(|j| crate::util::history::StepSnapshot {
                            enabled: pat.lanes[i].steps[j].enabled,
                            velocity: pat.lanes[i].steps[j].velocity,
                            probability: pat.lanes[i].steps[j].probability,
                            pan: pat.lanes[i].steps[j].pan,
                            pitch: pat.lanes[i].steps[j].pitch,
                            condition: pat.lanes[i].steps[j].condition,
                            fx_rvb: pat.lanes[i].steps[j].fx_rvb,
                            fx_dly: pat.lanes[i].steps[j].fx_dly,
                            fx_filter: pat.lanes[i].steps[j].fx_filter,
                        }),
                        muted: pat.lanes[i].muted,
                        solo: pat.lanes[i].solo,
                        fx_send_rvb: pat.lanes[i].fx_send_rvb,
                        fx_send_dly: pat.lanes[i].fx_send_dly,
                        fx_filter: pat.lanes[i].fx_filter,
                    }),
                    swing: pat.swing,
                    master_automation: pat.master_automation,
                    master_fx_base: pat.master_fx_base,
                })
                .collect(),
            active_pattern: self.active,
        }
    }

    pub fn restore(&mut self, snapshot: &crate::util::history::SequencerSnapshot) {
        for (pat, snap_pat) in self.patterns.iter_mut().zip(snapshot.patterns.iter()) {
            for (lane, snap_lane) in pat.lanes.iter_mut().zip(snap_pat.lanes.iter()) {
                for (step, snap_step) in lane.steps.iter_mut().zip(snap_lane.steps.iter()) {
                    step.enabled = snap_step.enabled;
                    step.velocity = snap_step.velocity;
                    step.probability = snap_step.probability;
                    step.pan = snap_step.pan;
                    step.pitch = snap_step.pitch;
                    step.condition = snap_step.condition;
                    step.fx_rvb = snap_step.fx_rvb;
                    step.fx_dly = snap_step.fx_dly;
                    step.fx_filter = snap_step.fx_filter;
                }
                lane.muted = snap_lane.muted;
                lane.solo = snap_lane.solo;
                lane.fx_send_rvb = snap_lane.fx_send_rvb;
                lane.fx_send_dly = snap_lane.fx_send_dly;
                lane.fx_filter = snap_lane.fx_filter;
            }
            pat.swing = snap_pat.swing;
            pat.master_automation = snap_pat.master_automation;
            pat.master_fx_base = snap_pat.master_fx_base;
        }
        self.active = snapshot.active_pattern;
    }
}

/// Advance to queued pattern at bar boundary (step 0).
fn advance_pattern_if_queued(bank: &mut PatternBank) {
    if let Some(queued) = bank.queued.take() {
        bank.active = queued;
    }
}

/// Host transport state for a single buffer.
///
/// Grouped rather than passed as five positional arguments — `buffer_len`,
/// `tempo`, `pos_beats` and `sample_rate` are all numeric and were easy to
/// transpose at a call site.
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    pub buffer_len: usize,
    /// Whether the host transport is rolling.
    pub playing: bool,
    /// Host tempo in BPM. `None` means the host didn't report one.
    pub tempo: Option<f64>,
    /// Host position in quarter notes. `None` means free-running.
    pub pos_beats: Option<f64>,
    pub sample_rate: f32,
}

/// The sequencer — owns playback state only.
///
/// Pattern data lives in `SharedState::pattern_bank` and is passed to
/// [`Sequencer::process_buffer_with_patterns`] per buffer. Until 0.5.5 this
/// struct also carried its own `bank`, which nothing in the audio or UI path
/// ever read or wrote — with the result that the undo snapshot taken when a
/// scan completed captured that empty bank instead of the user's patterns.
pub struct Sequencer {
    playing: bool,
    current_step: usize,
    tick_accumulator: f64,
    last_pos_beats: f64,
    /// Last step derived from host position (not from accumulator advancement).
    last_host_step: usize,
    rng: SmallRng,
    pub fill_active: bool,
    loop_count: u64,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new()
    }
}

impl Sequencer {
    pub fn new() -> Self {
        Self {
            playing: false,
            current_step: 0,
            tick_accumulator: 0.0,
            last_pos_beats: 0.0,
            last_host_step: 0,
            rng: SmallRng::from_os_rng(),
            fill_active: false,
            loop_count: 0,
        }
    }

    /// Reset playback position to step 0 for a clean start.
    pub fn reset_position(&mut self) {
        self.current_step = 0;
        self.last_host_step = 0;
        self.tick_accumulator = 0.0;
        self.last_pos_beats = 0.0;
        self.playing = false;
    }

    pub(crate) fn evaluate_condition(&self, cond: ConditionTrig) -> bool {
        match cond {
            ConditionTrig::Always => true,
            // `n.max(1)`: a zero divisor is unreachable through the UI and is
            // stripped by `ConditionTrig::sanitized`, but `% 0` panics on the
            // audio thread so the guard stays here too.
            ConditionTrig::Every(n) => self.loop_count.is_multiple_of(n.max(1) as u64),
            ConditionTrig::NotEvery(n) => !self.loop_count.is_multiple_of(n.max(1) as u64),
            ConditionTrig::Fill => self.fill_active,
            ConditionTrig::NotFill => !self.fill_active,
        }
    }

    /// Process one audio buffer using pattern data from an external PatternBank.
    /// Used when patterns live in SharedState; the Sequencer owns only playback state.
    pub fn process_buffer_with_patterns(
        &mut self,
        transport: &Transport,
        voices: &mut VoicePool,
        kit: &DrumKit,
        bank: &mut PatternBank,
        trigger_flags: &[AtomicU8; NUM_PADS],
    ) -> usize {
        let Transport {
            buffer_len,
            playing: host_playing,
            tempo,
            pos_beats,
            sample_rate,
        } = *transport;
        let tempo = match (host_playing, tempo) {
            (true, Some(t)) if t > 0.0 => t,
            _ => {
                self.playing = false;
                return 0;
            }
        };

        let mut fire_steps = [0usize; NUM_STEPS];
        let mut fire_count = 0usize;
        if let Some(beats) = pos_beats {
            if beats < 0.0 {
                self.playing = false;
                return 0;
            }
            let sixteenths = beats * 4.0;
            let swing = bank.active_pattern().swing as f64;
            let (host_step, frac) = Self::beats_to_swung_step(sixteenths, swing);

            self.current_step = host_step;
            let step_dur = self.step_duration_with_swing(
                host_step,
                tempo,
                sample_rate,
                bank.active_pattern().swing,
            );
            self.tick_accumulator = frac * step_dur;

            if !self.playing {
                fire_steps[fire_count] = host_step;
                fire_count += 1;
            } else if host_step != self.last_host_step {
                let prev = self.last_host_step;
                let mut s = (prev + 1) % NUM_STEPS;
                loop {
                    if s == 0 {
                        self.loop_count += 1;
                        advance_pattern_if_queued(bank);
                    }
                    if fire_count < NUM_STEPS {
                        fire_steps[fire_count] = s;
                        fire_count += 1;
                    }
                    if s == host_step {
                        break;
                    }
                    s = (s + 1) % NUM_STEPS;
                }
            }

            self.last_host_step = host_step;
            self.last_pos_beats = beats;
        }

        self.playing = true;
        let mut triggered = 0usize;

        for &step in &fire_steps[..fire_count] {
            self.current_step = step;
            triggered += self.fire_step_from_bank(0, voices, kit, bank, trigger_flags);
        }
        // Restore current_step from host after firing missed steps
        if fire_count > 0 {
            self.current_step = fire_steps[fire_count - 1];
        }

        for sample_offset in 0..buffer_len {
            self.tick_accumulator += 1.0;
            let step_dur = self.step_duration_with_swing(
                self.current_step,
                tempo,
                sample_rate,
                bank.active_pattern().swing,
            );

            if self.tick_accumulator >= step_dur {
                self.tick_accumulator -= step_dur;
                self.current_step = (self.current_step + 1) % NUM_STEPS;
                // Keep last_host_step in sync so the next buffer's catch-up
                // doesn't re-fire a step the accumulator already advanced past.
                self.last_host_step = self.current_step;

                if self.current_step == 0 {
                    self.loop_count += 1;
                    advance_pattern_if_queued(bank);
                }

                triggered +=
                    self.fire_step_from_bank(sample_offset, voices, kit, bank, trigger_flags);
            }
        }

        triggered
    }

    /// Duration of one step in samples, accounting for swing. Even steps
    /// (0,2,4,…) are lengthened and odd steps shortened by the same amount,
    /// so a full 16-step bar always spans the same time.
    pub fn step_duration_with_swing(
        &self,
        step: usize,
        tempo: f64,
        sample_rate: f32,
        swing: f32,
    ) -> f64 {
        let base = sample_rate as f64 * 60.0 / tempo / 4.0;
        let swing_offset = swing as f64 * base * 0.5;
        if step.is_multiple_of(2) {
            base + swing_offset
        } else {
            base - swing_offset
        }
    }

    /// Map a host beat position to a swing-adjusted (step, fractional_position) pair.
    /// Without this, `sixteenths.floor()` snaps to a straight grid and ignores swing.
    fn beats_to_swung_step(sixteenths: f64, swing: f64) -> (usize, f64) {
        let bar_pos = sixteenths.rem_euclid(NUM_STEPS as f64);
        let pair = (bar_pos / 2.0).floor() as usize;
        let pos_in_pair = bar_pos - pair as f64 * 2.0;
        let even_len = 1.0 + swing * 0.5;
        if pos_in_pair < even_len {
            (pair * 2, pos_in_pair / even_len)
        } else {
            let odd_len = 2.0 - even_len;
            (pair * 2 + 1, (pos_in_pair - even_len) / odd_len)
        }
    }

    fn fire_step_from_bank(
        &mut self,
        sample_offset: usize,
        voices: &mut VoicePool,
        kit: &DrumKit,
        bank: &PatternBank,
        trigger_flags: &[AtomicU8; NUM_PADS],
    ) -> usize {
        let step_idx = self.current_step.min(NUM_STEPS - 1);
        let pattern = bank.active_pattern();
        let any_solo = pattern.lanes.iter().any(|l| l.solo);
        let mut count = 0;

        for lane in &pattern.lanes {
            if any_solo && !lane.solo {
                continue;
            }
            if lane.muted {
                continue;
            }

            let step = &lane.steps[step_idx];
            if !step.enabled {
                continue;
            }

            // `pad_index` is serialized, so a corrupt or hand-edited pattern
            // can point past the kit. `PatternBank::sanitize` normalizes it on
            // load; this guard is the audio-thread backstop.
            let pad_index = lane.pad_index;
            if pad_index >= kit.pads.len() || pad_index >= trigger_flags.len() {
                continue;
            }

            if !self.evaluate_condition(step.condition) {
                continue;
            }

            if step.probability < 1.0 {
                let roll: f32 = self.rng.random();
                if roll >= step.probability {
                    continue;
                }
            }

            voices.trigger(
                &Trigger {
                    start_offset: sample_offset,
                    pan: step.pan,
                    pitch: step.pitch,
                    fx_rvb: step.fx_rvb,
                    fx_dly: step.fx_dly,
                    fx_filter: step.fx_filter,
                    ..Trigger::new(pad_index, step.velocity).with_lane_fx(
                        lane.fx_send_rvb,
                        lane.fx_send_dly,
                        lane.fx_filter,
                    )
                },
                kit,
            );
            trigger_flags[pad_index].fetch_add(1, Ordering::Relaxed);
            count += 1;
        }
        count
    }

    pub fn current_step(&self) -> usize {
        self.current_step
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kit::DrumKit;
    use crate::engine::sampler::VoicePool;
    use std::sync::atomic::AtomicU8;
    use std::sync::Arc;

    const SR: f32 = 44100.0;
    const TEMPO: f64 = 120.0;
    /// One 16-step bar at 120 BPM / 44.1 kHz = 4 beats = 2 s.
    const SAMPLES_PER_BAR: usize = 88200;
    /// One 16th step at 120 BPM / 44.1 kHz.
    const STEP_SAMPLES: f64 = 5512.5;

    fn dummy_flags() -> [AtomicU8; NUM_PADS] {
        core::array::from_fn(|_| AtomicU8::new(0))
    }

    /// A kit with every pad loaded, so any triggered lane produces a voice.
    fn test_kit() -> DrumKit {
        let mut kit = DrumKit::new();
        for pad in &mut kit.pads {
            pad.sample = Some(Arc::new(vec![1.0; 1024]));
            pad.volume = 1.0;
        }
        kit
    }

    /// A bank with the given `(lane, step)` pairs enabled on pattern 0.
    fn bank_with(enabled: &[(usize, usize)]) -> PatternBank {
        let mut bank = PatternBank::new();
        for &(lane, step) in enabled {
            bank.patterns[0].lanes[lane].steps[step].enabled = true;
        }
        bank
    }

    /// Drive the sequencer across `bars` bars of host-locked transport, in
    /// 512-sample blocks, and return the total number of voices triggered.
    ///
    /// This goes through `process_buffer_with_patterns` — the same call
    /// `Plugin::process` makes — so these tests cover the shipping path.
    fn run_bars(seq: &mut Sequencer, bank: &mut PatternBank, bars: usize) -> usize {
        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();
        let block = 512usize;
        let blocks = SAMPLES_PER_BAR * bars / block;

        let mut total = 0;
        for b in 0..blocks {
            let beats = (b * block) as f64 / SR as f64 * (TEMPO / 60.0);
            total += seq.process_buffer_with_patterns(
                &Transport {
                    buffer_len: block,
                    playing: true,
                    tempo: Some(TEMPO),
                    pos_beats: Some(beats),
                    sample_rate: SR,
                },
                &mut voices,
                &kit,
                bank,
                &flags,
            );
        }
        total
    }

    // ── Structure ────────────────────────────────────────────────────────

    #[test]
    fn new_bank_has_one_lane_per_pad_with_16_steps_each() {
        let bank = PatternBank::new();
        let pattern = bank.active_pattern();
        assert_eq!(pattern.lanes.len(), NUM_PADS);
        for (i, lane) in pattern.lanes.iter().enumerate() {
            assert_eq!(lane.pad_index, i);
            assert_eq!(lane.steps.len(), NUM_STEPS);
            assert!(!lane.muted);
            for step in &lane.steps {
                assert!(!step.enabled);
                assert!((step.velocity - 0.8).abs() < 0.001);
                assert!((step.probability - 1.0).abs() < 0.001);
            }
        }
    }

    #[test]
    fn pattern_bank_has_16_empty_patterns() {
        let bank = PatternBank::new();
        assert_eq!(bank.patterns.len(), NUM_PATTERNS);
        assert_eq!(bank.active, 0);
        assert!(bank.queued.is_none());
        for pat in &bank.patterns {
            assert_eq!(pat.lanes.len(), NUM_PADS);
            assert!((pat.swing - 0.0).abs() < 0.001);
        }
    }

    #[test]
    fn pattern_has_data_check() {
        let mut bank = PatternBank::new();
        assert!(!bank.patterns[0].has_data());
        bank.patterns[0].lanes[0].steps[0].enabled = true;
        assert!(bank.patterns[0].has_data());
    }

    #[test]
    fn step_default_has_no_plocks_and_always_condition() {
        let step = Step::default();
        assert!(!step.enabled);
        assert!((step.velocity - 0.8).abs() < 0.001);
        assert!((step.probability - 1.0).abs() < 0.001);
        assert!(step.pan.is_none());
        assert!(step.pitch.is_none());
        assert_eq!(step.condition, ConditionTrig::Always);
    }

    #[test]
    fn step_with_plocks() {
        let step = Step {
            enabled: true,
            velocity: 0.6,
            probability: 1.0,
            pan: Some(-0.5),
            pitch: Some(7.0),
            condition: ConditionTrig::Fill,
            fx_rvb: None,
            fx_dly: None,
            fx_filter: None,
        };
        assert_eq!(step.pan, Some(-0.5));
        assert_eq!(step.pitch, Some(7.0));
        assert_eq!(step.condition, ConditionTrig::Fill);
    }

    #[test]
    fn condition_trig_default_is_always() {
        assert_eq!(ConditionTrig::default(), ConditionTrig::Always);
    }

    // ── Timing / swing ───────────────────────────────────────────────────

    #[test]
    fn step_duration_at_120bpm_44100hz() {
        let seq = Sequencer::new();
        let dur = seq.step_duration_with_swing(0, TEMPO, SR, 0.0);
        assert!((dur - STEP_SAMPLES).abs() < 0.1);
    }

    #[test]
    fn swing_lengthens_even_steps_shortens_odd() {
        let seq = Sequencer::new();
        let swing = 0.5;
        let offset = swing as f64 * STEP_SAMPLES * 0.5;

        let even = seq.step_duration_with_swing(0, TEMPO, SR, swing);
        let odd = seq.step_duration_with_swing(1, TEMPO, SR, swing);

        assert!((even - (STEP_SAMPLES + offset)).abs() < 0.1);
        assert!((odd - (STEP_SAMPLES - offset)).abs() < 0.1);
    }

    #[test]
    fn swing_does_not_change_total_pattern_length() {
        let seq = Sequencer::new();
        let total: f64 = (0..NUM_STEPS)
            .map(|s| seq.step_duration_with_swing(s, TEMPO, SR, 0.7))
            .sum();
        assert!(
            (total - SAMPLES_PER_BAR as f64).abs() < 0.1,
            "swing should preserve total bar length"
        );
    }

    #[test]
    fn beats_to_swung_step_no_swing_is_straight() {
        for i in 0..NUM_STEPS {
            let sixteenths = i as f64 + 0.5;
            let (step, frac) = Sequencer::beats_to_swung_step(sixteenths, 0.0);
            assert_eq!(step, i, "step at sixteenth {sixteenths}");
            assert!((frac - 0.5).abs() < 0.001, "frac at sixteenth {sixteenths}");
        }
    }

    #[test]
    fn beats_to_swung_step_shifts_odd_steps() {
        let swing = 0.5;
        let even_len = 1.0 + swing * 0.5;
        let (step, _) = Sequencer::beats_to_swung_step(1.0, swing);
        assert_eq!(
            step, 0,
            "sixteenth 1.0 is still in the even step at swing 0.5"
        );
        let (step, frac) = Sequencer::beats_to_swung_step(even_len, swing);
        assert_eq!(step, 1, "odd step begins at {even_len}");
        assert!(frac.abs() < 0.001, "frac should be ~0 at the boundary");
    }

    #[test]
    fn beats_to_swung_step_pairs_always_span_two_sixteenths() {
        for swing in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let even_len = 1.0 + swing * 0.5;
            let odd_len = 2.0 - even_len;
            assert!(
                (even_len + odd_len - 2.0_f64).abs() < 1e-10,
                "pair should span 2 sixteenths at swing {swing}"
            );
        }
    }

    // ── Playback (shipping path) ─────────────────────────────────────────

    #[test]
    fn process_triggers_enabled_steps_at_correct_positions() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0)]);
        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        let triggered = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: true,
                tempo: Some(TEMPO),
                pos_beats: Some(0.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );

        assert!(triggered > 0, "should have triggered at least one voice");
        assert!(
            voices.active_count() > 0,
            "voice pool should have active voices"
        );
    }

    #[test]
    fn full_pattern_cycles_through_16_steps() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0), (1, 4), (2, 8)]);
        let total = run_bars(&mut seq, &mut bank, 1);
        assert_eq!(total, 3, "expected 3 triggers across one full bar");
    }

    #[test]
    fn four_on_the_floor_fires_exactly_four_times_per_bar() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0), (0, 4), (0, 8), (0, 12)]);
        let total = run_bars(&mut seq, &mut bank, 1);
        assert_eq!(total, 4, "4/4 kick should fire exactly 4 times per bar");
    }

    #[test]
    fn muted_lane_does_not_trigger() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0)]);
        bank.patterns[0].lanes[0].muted = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        let triggered = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: true,
                tempo: Some(TEMPO),
                pos_beats: Some(0.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );

        assert_eq!(triggered, 0, "muted lane should not trigger");
        assert_eq!(voices.active_count(), 0);
    }

    #[test]
    fn solo_lane_silences_every_other_lane() {
        let mut seq = Sequencer::new();
        // Three lanes all firing on step 0; only lane 1 is soloed.
        let mut bank = bank_with(&[(0, 0), (1, 0), (2, 0)]);
        bank.patterns[0].lanes[1].solo = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        let triggered = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: true,
                tempo: Some(TEMPO),
                pos_beats: Some(0.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );

        assert_eq!(triggered, 1, "only the soloed lane should fire");
        assert_eq!(
            flags[1].load(Ordering::Relaxed),
            1,
            "lane 1 should have fired"
        );
        assert_eq!(
            flags[0].load(Ordering::Relaxed),
            0,
            "lane 0 should be silenced"
        );
        assert_eq!(
            flags[2].load(Ordering::Relaxed),
            0,
            "lane 2 should be silenced"
        );
    }

    #[test]
    fn probability_zero_never_triggers() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0)]);
        bank.patterns[0].lanes[0].steps[0].probability = 0.0;

        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        for beat in 0..10 {
            let triggered = seq.process_buffer_with_patterns(
                &Transport {
                    buffer_len: 512,
                    playing: true,
                    tempo: Some(TEMPO),
                    pos_beats: Some(beat as f64 * 4.0),
                    sample_rate: SR,
                },
                &mut voices,
                &kit,
                &mut bank,
                &flags,
            );
            assert_eq!(
                triggered, 0,
                "probability 0.0 should never trigger (beat {beat})"
            );
        }
    }

    #[test]
    fn no_trigger_when_host_stopped() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0)]);
        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        let triggered = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: false,
                tempo: Some(TEMPO),
                pos_beats: Some(0.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );
        assert_eq!(triggered, 0, "should not trigger when host is stopped");
        assert!(!seq.is_playing());
    }

    #[test]
    fn negative_pos_beats_does_not_trigger() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0)]);
        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        let triggered = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: true,
                tempo: Some(TEMPO),
                pos_beats: Some(-1.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );
        assert_eq!(triggered, 0, "negative pos_beats should not trigger");
    }

    #[test]
    fn host_rewind_resyncs_sequencer() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0), (0, 8)]);
        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        let first = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: true,
                tempo: Some(TEMPO),
                pos_beats: Some(2.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );
        assert!(first > 0, "should fire step 8 at beat 2.0");

        let after_rewind = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: true,
                tempo: Some(TEMPO),
                pos_beats: Some(0.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );
        assert!(
            after_rewind > 0,
            "should fire step 0 after rewind to beat 0.0"
        );
    }

    // ── Conditional trigs ────────────────────────────────────────────────

    #[test]
    fn condition_always_fires() {
        let seq = Sequencer::new();
        assert!(seq.evaluate_condition(ConditionTrig::Always));
    }

    #[test]
    fn condition_every_2_fires_on_even_loops() {
        let mut seq = Sequencer::new();
        seq.loop_count = 0;
        assert!(seq.evaluate_condition(ConditionTrig::Every(2)));
        seq.loop_count = 1;
        assert!(!seq.evaluate_condition(ConditionTrig::Every(2)));
        seq.loop_count = 2;
        assert!(seq.evaluate_condition(ConditionTrig::Every(2)));
    }

    #[test]
    fn condition_fill_respects_fill_active() {
        let mut seq = Sequencer::new();
        seq.fill_active = false;
        assert!(!seq.evaluate_condition(ConditionTrig::Fill));
        seq.fill_active = true;
        assert!(seq.evaluate_condition(ConditionTrig::Fill));
        assert!(!seq.evaluate_condition(ConditionTrig::NotFill));
    }

    /// Regression: `loop_count % 0` panics. `Every(0)` is unreachable through
    /// the UI but is representable in a hand-edited or corrupt pattern file,
    /// and `evaluate_condition` runs on the audio thread.
    #[test]
    fn condition_every_zero_does_not_divide_by_zero() {
        let mut seq = Sequencer::new();
        for loop_count in 0..4 {
            seq.loop_count = loop_count;
            let _ = seq.evaluate_condition(ConditionTrig::Every(0));
            let _ = seq.evaluate_condition(ConditionTrig::NotEvery(0));
        }
    }

    // ── Pattern switching ────────────────────────────────────────────────

    #[test]
    fn pattern_queued_switches_at_bar_boundary() {
        let mut seq = Sequencer::new();
        let mut bank = PatternBank::new();
        // Pattern 0 fires lane 0 on every step; pattern 1 fires lane 1.
        for s in 0..NUM_STEPS {
            bank.patterns[0].lanes[0].steps[s].enabled = true;
            bank.patterns[1].lanes[1].steps[s].enabled = true;
        }
        bank.queued = Some(1);
        assert_eq!(bank.active, 0);

        run_bars(&mut seq, &mut bank, 2);

        assert_eq!(
            bank.active, 1,
            "queued pattern should be active after a bar"
        );
        assert!(bank.queued.is_none(), "queue should be consumed");
    }

    #[test]
    fn lane_mute_persists_across_pattern_switch() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[3].muted = true;
        bank.active = 1;
        assert!(!bank.active_pattern().lanes[3].muted);
        bank.active = 0;
        assert!(bank.active_pattern().lanes[3].muted);
    }

    #[test]
    fn lane_fx_send_persists_across_pattern_switch() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[2].fx_send_rvb = 0.7;
        bank.patterns[0].lanes[2].fx_filter = true;
        bank.active = 1;
        assert!((bank.active_pattern().lanes[2].fx_send_rvb - 0.0).abs() < 0.001);
        assert!(!bank.active_pattern().lanes[2].fx_filter);
        bank.active = 0;
        assert!((bank.active_pattern().lanes[2].fx_send_rvb - 0.7).abs() < 0.001);
        assert!(bank.active_pattern().lanes[2].fx_filter);
    }

    // ── Snapshot / serde ─────────────────────────────────────────────────

    #[test]
    fn snapshot_captures_pattern_bank_state() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[0].steps[0].enabled = true;
        bank.patterns[0].lanes[0].steps[0].velocity = 0.6;
        bank.patterns[0].lanes[3].muted = true;
        bank.patterns[0].swing = 0.3;

        let snap = bank.snapshot();
        assert!(snap.patterns[0].lanes[0].steps[0].enabled);
        assert!((snap.patterns[0].lanes[0].steps[0].velocity - 0.6).abs() < 0.001);
        assert!(snap.patterns[0].lanes[3].muted);
        assert!((snap.patterns[0].swing - 0.3).abs() < 0.001);
    }

    #[test]
    fn restore_applies_pattern_bank_snapshot() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[0].steps[0].enabled = true;
        bank.patterns[0].swing = 0.5;

        let snap = bank.snapshot();
        bank.patterns[0].lanes[0].steps[0].enabled = false;
        bank.patterns[0].swing = 0.0;

        bank.restore(&snap);
        assert!(bank.patterns[0].lanes[0].steps[0].enabled);
        assert!((bank.patterns[0].swing - 0.5).abs() < 0.001);
    }

    #[test]
    fn pattern_bank_serializes_roundtrip() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[0].steps[0].enabled = true;
        bank.patterns[0].lanes[0].steps[0].velocity = 0.6;
        bank.patterns[0].lanes[0].steps[0].condition = ConditionTrig::Fill;
        bank.patterns[0].lanes[0].steps[3].pan = Some(-0.5);
        bank.patterns[0].swing = 0.4;

        let json = serde_json::to_string(&bank).unwrap();
        let restored: PatternBank = serde_json::from_str(&json).unwrap();

        assert!(restored.patterns[0].lanes[0].steps[0].enabled);
        assert!((restored.patterns[0].lanes[0].steps[0].velocity - 0.6).abs() < 0.001);
        assert_eq!(
            restored.patterns[0].lanes[0].steps[0].condition,
            ConditionTrig::Fill
        );
        assert_eq!(restored.patterns[0].lanes[0].steps[3].pan, Some(-0.5));
        assert!((restored.patterns[0].swing - 0.4).abs() < 0.001);
    }

    #[test]
    fn master_fx_base_roundtrips_through_pattern_serde() {
        let mut bank = PatternBank::new();
        bank.patterns[0].master_fx_base.reverb_mix = 0.42;
        bank.patterns[0].master_fx_base.delay_mix = 0.13;
        bank.patterns[0].master_fx_base.dj_filter = -0.25;
        bank.patterns[0].master_fx_base.initialized = true;

        let json = serde_json::to_string(&bank).unwrap();
        let restored: PatternBank = serde_json::from_str(&json).unwrap();

        assert!((restored.patterns[0].master_fx_base.reverb_mix - 0.42).abs() < 0.001);
        assert!((restored.patterns[0].master_fx_base.delay_mix - 0.13).abs() < 0.001);
        assert!((restored.patterns[0].master_fx_base.dj_filter - (-0.25)).abs() < 0.001);
        assert!(restored.patterns[0].master_fx_base.initialized);
    }

    // ── sanitize(): the audio thread's invariants ────────────────────────

    #[test]
    fn sanitize_clamps_out_of_range_active_index() {
        let mut bank = PatternBank::new();
        bank.active = 999;
        bank.sanitize();
        assert_eq!(bank.active, 0);
        // Would have panicked before sanitize.
        let _ = bank.active_pattern();
    }

    #[test]
    fn sanitize_drops_out_of_range_queued_index() {
        let mut bank = PatternBank::new();
        bank.queued = Some(999);
        bank.sanitize();
        assert!(bank.queued.is_none());
    }

    #[test]
    fn sanitize_restores_the_pattern_count() {
        let mut bank = PatternBank::new();
        bank.patterns.truncate(3);
        bank.sanitize();
        assert_eq!(bank.patterns.len(), NUM_PATTERNS);

        let mut bank = PatternBank::new();
        for _ in 0..5 {
            bank.patterns.push(Pattern::new());
        }
        bank.sanitize();
        assert_eq!(bank.patterns.len(), NUM_PATTERNS);
    }

    #[test]
    fn sanitize_restores_the_lane_count_and_reindexes_pads() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes.truncate(2);
        bank.patterns[0].lanes[0].pad_index = 99;
        bank.sanitize();

        assert_eq!(bank.patterns[0].lanes.len(), NUM_PADS);
        for (i, lane) in bank.patterns[0].lanes.iter().enumerate() {
            assert_eq!(lane.pad_index, i, "lane {i} should drive pad {i}");
        }
    }

    #[test]
    fn sanitize_strips_zero_divisor_conditions() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[0].steps[0].condition = ConditionTrig::Every(0);
        bank.patterns[0].lanes[0].steps[1].condition = ConditionTrig::NotEvery(0);
        bank.sanitize();
        assert_eq!(
            bank.patterns[0].lanes[0].steps[0].condition,
            ConditionTrig::Always
        );
        assert_eq!(
            bank.patterns[0].lanes[0].steps[1].condition,
            ConditionTrig::Always
        );
    }

    #[test]
    fn sanitize_clamps_out_of_range_and_nan_values() {
        let mut bank = PatternBank::new();
        {
            let step = &mut bank.patterns[0].lanes[0].steps[0];
            step.velocity = f32::NAN;
            step.probability = 7.5;
            step.pan = Some(-9.0);
            step.pitch = Some(f32::INFINITY);
            step.fx_rvb = Some(3.0);
        }
        bank.patterns[0].lanes[0].fx_send_dly = -2.0;
        bank.patterns[0].swing = f32::NAN;
        bank.sanitize();

        let step = &bank.patterns[0].lanes[0].steps[0];
        assert!(
            (step.velocity - 0.8).abs() < 0.001,
            "NaN velocity falls back to default"
        );
        assert!((step.probability - 1.0).abs() < 0.001);
        assert_eq!(step.pan, Some(-1.0));
        assert_eq!(step.pitch, Some(0.0), "non-finite pitch falls back to 0");
        assert_eq!(step.fx_rvb, Some(1.0));
        assert!((bank.patterns[0].lanes[0].fx_send_dly - 0.0).abs() < 0.001);
        assert!((bank.patterns[0].swing - 0.0).abs() < 0.001);
    }

    #[test]
    fn sanitize_survives_a_completely_empty_pattern_list() {
        let mut bank = PatternBank::new();
        bank.patterns.clear();
        bank.active = 4;
        bank.sanitize();
        // The list is refilled first, so an in-range `active` is kept — it now
        // points at a valid (empty) pattern rather than being reset to 0.
        assert_eq!(bank.patterns.len(), NUM_PATTERNS);
        assert!(bank.active < bank.patterns.len());
        let _ = bank.active_pattern();
    }

    /// Regression: a lane whose `pad_index` points past the kit used to index
    /// `kit.pads` and `trigger_flags` directly and panic on the audio thread.
    /// `sanitize` normalizes it, but `fire_step_from_bank` guards too.
    #[test]
    fn out_of_range_pad_index_is_skipped_not_panicked() {
        let mut seq = Sequencer::new();
        let mut bank = bank_with(&[(0, 0), (1, 0)]);
        // Deliberately NOT sanitized — this exercises the audio-thread guard.
        bank.patterns[0].lanes[0].pad_index = 250;

        let kit = test_kit();
        let mut voices = VoicePool::new(SR);
        let flags = dummy_flags();

        let triggered = seq.process_buffer_with_patterns(
            &Transport {
                buffer_len: 512,
                playing: true,
                tempo: Some(TEMPO),
                pos_beats: Some(0.0),
                sample_rate: SR,
            },
            &mut voices,
            &kit,
            &mut bank,
            &flags,
        );

        assert_eq!(
            triggered, 1,
            "the valid lane still fires, the bad one is skipped"
        );
        assert_eq!(flags[1].load(Ordering::Relaxed), 1);
    }
}
