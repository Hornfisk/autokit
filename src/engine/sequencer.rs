use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::{Serialize, Deserialize};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::engine::kit::{DrumKit, NUM_PADS};
use crate::engine::sampler::VoicePool;
use crate::util::history::{StepSnapshot, LaneSnapshot, PatternSnapshot, SequencerSnapshot};

/// Conditional trig types — Elektron-style step conditions.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ConditionTrig {
    Always,        // Default — fires every loop
    Every(u8),     // 1:N — fires every Nth loop (N = 2, 4, 8)
    NotEvery(u8),  // !1:N — fires on all loops EXCEPT every Nth
    Fill,          // Fires only when FILL mode is active
    NotFill,       // Fires only when FILL mode is NOT active
}

impl Default for ConditionTrig {
    fn default() -> Self {
        Self::Always
    }
}

impl ConditionTrig {
    /// All conditions in cycle order for the GUI selector.
    pub const CYCLE: &'static [ConditionTrig] = &[
        Self::Always,
        Self::Every(2), Self::Every(4), Self::Every(8),
        Self::NotEvery(2), Self::NotEvery(4), Self::NotEvery(8),
        Self::Fill, Self::NotFill,
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
}

/// A single step in the sequencer.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Step {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
    pub pan: Option<f32>,        // None = inherit pad default, Some = p-lock
    pub pitch: Option<f32>,      // None = inherit pad default, Some = p-lock (semitones)
    pub condition: ConditionTrig,
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
        }
    }
}

/// One lane = one pad's 16-step sequence.
#[derive(Clone, Serialize, Deserialize)]
pub struct Lane {
    pub pad_index: usize,
    pub steps: [Step; 16],
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
}

impl Lane {
    pub fn new(pad_index: usize) -> Self {
        Self {
            pad_index,
            steps: [Step::default(); 16],
            muted: false,
            solo: false,
        }
    }
}

pub const NUM_STEPS: usize = 16;
pub const NUM_PATTERNS: usize = 16;

/// One pattern: 8 lanes + swing setting.
#[derive(Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub lanes: Vec<Lane>,
    pub swing: f32,
}

impl Pattern {
    pub fn new() -> Self {
        Self {
            lanes: (0..NUM_PADS).map(Lane::new).collect(),
            swing: 0.0,
        }
    }

    /// Returns true if any step in any lane is enabled.
    pub fn has_data(&self) -> bool {
        self.lanes.iter().any(|lane| lane.steps.iter().any(|s| s.enabled))
    }
}

/// Bank of 16 patterns with active/queued selection.
#[derive(Serialize, Deserialize)]
pub struct PatternBank {
    pub patterns: Vec<Pattern>,
    pub active: usize,
    pub queued: Option<usize>,
}

impl PatternBank {
    pub fn new() -> Self {
        Self {
            patterns: (0..NUM_PATTERNS).map(|_| Pattern::new()).collect(),
            active: 0,
            queued: None,
        }
    }

    pub fn active_pattern(&self) -> &Pattern {
        &self.patterns[self.active]
    }

    pub fn active_pattern_mut(&mut self) -> &mut Pattern {
        &mut self.patterns[self.active]
    }

    pub fn snapshot(&self) -> crate::util::history::SequencerSnapshot {
        crate::util::history::SequencerSnapshot {
            patterns: self.patterns.iter().map(|pat| {
                crate::util::history::PatternSnapshot {
                    lanes: core::array::from_fn(|i| {
                        crate::util::history::LaneSnapshot {
                            steps: core::array::from_fn(|j| crate::util::history::StepSnapshot {
                                enabled: pat.lanes[i].steps[j].enabled,
                                velocity: pat.lanes[i].steps[j].velocity,
                                probability: pat.lanes[i].steps[j].probability,
                                pan: pat.lanes[i].steps[j].pan,
                                pitch: pat.lanes[i].steps[j].pitch,
                                condition: pat.lanes[i].steps[j].condition,
                            }),
                            muted: pat.lanes[i].muted,
                            solo: pat.lanes[i].solo,
                        }
                    }),
                    swing: pat.swing,
                }
            }).collect(),
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
                }
                lane.muted = snap_lane.muted;
                lane.solo = snap_lane.solo;
            }
            pat.swing = snap_pat.swing;
        }
        self.active = snapshot.active_pattern;
    }
}

/// The sequencer — owns playback state; pattern data lives in PatternBank.
pub struct Sequencer {
    pub bank: PatternBank,
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

impl Sequencer {
    pub fn new() -> Self {
        Self {
            bank: PatternBank::new(),
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

    /// Access lanes of the active pattern (convenience for existing code).
    pub fn lanes(&self) -> &[Lane] {
        &self.bank.active_pattern().lanes
    }

    pub fn lanes_mut(&mut self) -> &mut Vec<Lane> {
        &mut self.bank.active_pattern_mut().lanes
    }

    pub fn swing(&self) -> f32 {
        self.bank.active_pattern().swing
    }

    pub fn set_swing(&mut self, value: f32) {
        self.bank.active_pattern_mut().swing = value;
    }

    /// Compute duration of a step in samples, accounting for swing.
    /// Even steps (0,2,4,...) are lengthened, odd steps (1,3,5,...) are shortened.
    pub fn step_duration_samples(&self, step: usize, tempo: f64, sample_rate: f32) -> f64 {
        let base = sample_rate as f64 * 60.0 / tempo / 4.0;
        let swing = self.bank.active_pattern().swing;
        let swing_offset = swing as f64 * base * 0.5;
        if step % 2 == 0 {
            base + swing_offset
        } else {
            base - swing_offset
        }
    }

    pub(crate) fn evaluate_condition(&self, cond: ConditionTrig) -> bool {
        match cond {
            ConditionTrig::Always => true,
            ConditionTrig::Every(n) => self.loop_count % n as u64 == 0,
            ConditionTrig::NotEvery(n) => self.loop_count % n as u64 != 0,
            ConditionTrig::Fill => self.fill_active,
            ConditionTrig::NotFill => !self.fill_active,
        }
    }

    /// Process one audio buffer using self.bank. Scans for step boundaries, triggers voices.
    /// Returns the number of voices triggered (useful for testing/debug).
    pub fn process_buffer(
        &mut self,
        buffer_len: usize,
        host_playing: bool,
        tempo: Option<f64>,
        pos_beats: Option<f64>,
        sample_rate: f32,
        voices: &mut VoicePool,
        kit: &DrumKit,
        trigger_flags: &[AtomicU8; NUM_PADS],
    ) -> usize {
        let tempo = match (host_playing, tempo) {
            (true, Some(t)) if t > 0.0 => t,
            _ => {
                self.playing = false;
                return 0;
            }
        };

        // Sync to host position — always derive step from host beats (no drift accumulation)
        let mut fire_steps: Vec<usize> = Vec::new();
        if let Some(beats) = pos_beats {
            if beats < 0.0 {
                self.playing = false;
                return 0;
            }
            let sixteenths = beats * 4.0;
            let host_step = ((sixteenths.floor() as usize) % 16) as usize;
            let frac = sixteenths.fract();

            self.current_step = host_step;
            let step_dur = self.step_duration_samples(host_step, tempo, sample_rate);
            self.tick_accumulator = frac * step_dur;

            if !self.playing {
                fire_steps.push(host_step);
            } else if host_step != self.last_host_step {
                let prev = self.last_host_step;
                let mut s = (prev + 1) % 16;
                loop {
                    if s == 0 {
                        self.loop_count += 1;
                        if let Some(queued) = self.bank.queued.take() {
                            self.bank.active = queued;
                        }
                    }
                    fire_steps.push(s);
                    if s == host_step { break; }
                    s = (s + 1) % 16;
                }
            }

            self.last_host_step = host_step;
            self.last_pos_beats = beats;
        }

        self.playing = true;
        let mut triggered = 0usize;

        for &step in &fire_steps {
            self.current_step = step;
            triggered += self.fire_step(0, voices, kit, trigger_flags);
        }
        if let Some(&last) = fire_steps.last() {
            self.current_step = last;
        }

        for sample_offset in 0..buffer_len {
            self.tick_accumulator += 1.0;
            let step_dur = self.step_duration_samples(self.current_step, tempo, sample_rate);

            if self.tick_accumulator >= step_dur {
                self.tick_accumulator -= step_dur;
                self.current_step = (self.current_step + 1) % 16;
                self.last_host_step = self.current_step;

                if self.current_step == 0 {
                    self.loop_count += 1;
                    if let Some(queued) = self.bank.queued.take() {
                        self.bank.active = queued;
                    }
                }

                triggered += self.fire_step(sample_offset, voices, kit, trigger_flags);
            }
        }

        triggered
    }

    /// Process one audio buffer using pattern data from an external PatternBank.
    /// Used when patterns live in SharedState; the Sequencer owns only playback state.
    pub fn process_buffer_with_patterns(
        &mut self,
        buffer_len: usize,
        host_playing: bool,
        tempo: Option<f64>,
        pos_beats: Option<f64>,
        sample_rate: f32,
        voices: &mut VoicePool,
        kit: &DrumKit,
        bank: &mut PatternBank,
        trigger_flags: &[AtomicU8; NUM_PADS],
    ) -> usize {
        let tempo = match (host_playing, tempo) {
            (true, Some(t)) if t > 0.0 => t,
            _ => {
                self.playing = false;
                return 0;
            }
        };

        let mut fire_steps: Vec<usize> = Vec::new();
        if let Some(beats) = pos_beats {
            if beats < 0.0 {
                self.playing = false;
                return 0;
            }
            let sixteenths = beats * 4.0;
            let host_step = ((sixteenths.floor() as usize) % 16) as usize;
            let frac = sixteenths.fract();

            self.current_step = host_step;
            let step_dur = self.step_duration_with_swing(host_step, tempo, sample_rate, bank.active_pattern().swing);
            self.tick_accumulator = frac * step_dur;

            if !self.playing {
                // Fresh start — fire the step we land on
                fire_steps.push(host_step);
            } else if host_step != self.last_host_step {
                // Host step changed — fire any steps we may have missed
                // (e.g. due to GUI lock contention skipping a buffer)
                let prev = self.last_host_step;
                let mut s = (prev + 1) % 16;
                loop {
                    if s == 0 {
                        self.loop_count += 1;
                        if let Some(queued) = bank.queued.take() {
                            bank.active = queued;
                        }
                    }
                    fire_steps.push(s);
                    if s == host_step { break; }
                    s = (s + 1) % 16;
                }
            }

            self.last_host_step = host_step;
            self.last_pos_beats = beats;
        }

        self.playing = true;
        let mut triggered = 0usize;

        for &step in &fire_steps {
            self.current_step = step;
            triggered += self.fire_step_from_bank(0, voices, kit, bank, trigger_flags);
        }
        // Restore current_step from host after firing missed steps
        if let Some(&last) = fire_steps.last() {
            self.current_step = last;
        }

        for sample_offset in 0..buffer_len {
            self.tick_accumulator += 1.0;
            let step_dur = self.step_duration_with_swing(self.current_step, tempo, sample_rate, bank.active_pattern().swing);

            if self.tick_accumulator >= step_dur {
                self.tick_accumulator -= step_dur;
                self.current_step = (self.current_step + 1) % 16;
                // Keep last_host_step in sync so the next buffer's catch-up
                // doesn't re-fire a step the accumulator already advanced past.
                self.last_host_step = self.current_step;

                if self.current_step == 0 {
                    self.loop_count += 1;
                    if let Some(queued) = bank.queued.take() {
                        bank.active = queued;
                    }
                }

                triggered += self.fire_step_from_bank(sample_offset, voices, kit, bank, trigger_flags);
            }
        }

        triggered
    }

    fn step_duration_with_swing(&self, step: usize, tempo: f64, sample_rate: f32, swing: f32) -> f64 {
        let base = sample_rate as f64 * 60.0 / tempo / 4.0;
        let swing_offset = swing as f64 * base * 0.5;
        if step % 2 == 0 {
            base + swing_offset
        } else {
            base - swing_offset
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
        let step_idx = self.current_step;
        let pattern = bank.active_pattern();
        let any_solo = pattern.lanes.iter().any(|l| l.solo);
        let mut count = 0;

        for i in 0..pattern.lanes.len() {
            let lane = &pattern.lanes[i];
            if any_solo && !lane.solo { continue; }
            if lane.muted { continue; }

            let step = &lane.steps[step_idx];
            if !step.enabled { continue; }

            if !self.evaluate_condition(step.condition) { continue; }

            if step.probability < 1.0 {
                let roll: f32 = self.rng.random();
                if roll >= step.probability { continue; }
            }

            let velocity = step.velocity;
            let pad_index = lane.pad_index;
            voices.trigger(pad_index, velocity, kit, sample_offset, step.pan, step.pitch);
            trigger_flags[pad_index].fetch_add(1, Ordering::Relaxed);
            count += 1;
        }
        count
    }

    /// Capture the undoable sequencer state (all patterns).
    pub fn snapshot(&self) -> SequencerSnapshot {
        let patterns: Vec<PatternSnapshot> = self.bank.patterns.iter().map(|pat| {
            let lanes: [LaneSnapshot; NUM_PADS] = core::array::from_fn(|i| {
                let steps: [StepSnapshot; 16] = core::array::from_fn(|j| StepSnapshot {
                    enabled: pat.lanes[i].steps[j].enabled,
                    velocity: pat.lanes[i].steps[j].velocity,
                    probability: pat.lanes[i].steps[j].probability,
                    pan: pat.lanes[i].steps[j].pan,
                    pitch: pat.lanes[i].steps[j].pitch,
                    condition: pat.lanes[i].steps[j].condition,
                });
                LaneSnapshot {
                    steps,
                    muted: pat.lanes[i].muted,
                    solo: pat.lanes[i].solo,
                }
            });
            PatternSnapshot {
                lanes,
                swing: pat.swing,
            }
        }).collect();

        SequencerSnapshot {
            patterns,
            active_pattern: self.bank.active,
        }
    }

    /// Restore sequencer state from a snapshot. Preserves playback state.
    pub fn restore(&mut self, snapshot: &SequencerSnapshot) {
        for (pat, snap_pat) in self.bank.patterns.iter_mut().zip(snapshot.patterns.iter()) {
            for (lane, snap_lane) in pat.lanes.iter_mut().zip(snap_pat.lanes.iter()) {
                for (step, snap_step) in lane.steps.iter_mut().zip(snap_lane.steps.iter()) {
                    step.enabled = snap_step.enabled;
                    step.velocity = snap_step.velocity;
                    step.probability = snap_step.probability;
                    step.pan = snap_step.pan;
                    step.pitch = snap_step.pitch;
                    step.condition = snap_step.condition;
                }
                lane.muted = snap_lane.muted;
                lane.solo = snap_lane.solo;
            }
            pat.swing = snap_pat.swing;
        }
        self.bank.active = snapshot.active_pattern;
    }

    pub fn current_step(&self) -> usize {
        self.current_step
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn active_pattern_index(&self) -> usize {
        self.bank.active
    }

    /// Fire all enabled, non-muted lanes of the active pattern for the current step.
    fn fire_step(
        &mut self,
        sample_offset: usize,
        voices: &mut VoicePool,
        kit: &DrumKit,
        trigger_flags: &[AtomicU8; NUM_PADS],
    ) -> usize {
        let step_idx = self.current_step;
        let any_solo = self.bank.active_pattern().lanes.iter().any(|l| l.solo);
        let mut count = 0;

        for i in 0..self.bank.active_pattern().lanes.len() {
            let lane = &self.bank.active_pattern().lanes[i];
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

            if !self.evaluate_condition(step.condition) {
                continue;
            }

            if step.probability < 1.0 {
                let roll: f32 = self.rng.random();
                if roll >= step.probability {
                    continue;
                }
            }

            let velocity = step.velocity;
            let pad_index = lane.pad_index;
            voices.trigger(pad_index, velocity, kit, sample_offset, step.pan, step.pitch);
            if pad_index < trigger_flags.len() {
                trigger_flags[pad_index].fetch_add(1, Ordering::Relaxed);
            }
            count += 1;
        }

        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kit::DrumKit;
    use crate::engine::sampler::VoicePool;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU8;

    fn dummy_flags() -> [AtomicU8; NUM_PADS] {
        core::array::from_fn(|_| AtomicU8::new(0))
    }

    /// Helper: create a kit with all 16 pads loaded (1.0 samples).
    fn test_kit() -> DrumKit {
        let mut kit = DrumKit::new();
        for pad in &mut kit.pads {
            pad.sample = Some(Arc::new(vec![1.0; 1024]));
            pad.volume = 1.0;
        }
        kit
    }

    #[test]
    fn new_sequencer_has_correct_lane_count_with_16_steps_each() {
        let seq = Sequencer::new();
        assert_eq!(seq.lanes().len(), NUM_PADS);
        for (i, lane) in seq.lanes().iter().enumerate() {
            assert_eq!(lane.pad_index, i);
            assert_eq!(lane.steps.len(), 16);
            assert!(!lane.muted);
            for step in &lane.steps {
                assert!(!step.enabled);
                assert!((step.velocity - 0.8).abs() < 0.001);
                assert!((step.probability - 1.0).abs() < 0.001);
            }
        }
    }

    #[test]
    fn default_swing_is_zero() {
        let seq = Sequencer::new();
        assert!((seq.swing() - 0.0).abs() < 0.001);
    }

    #[test]
    fn step_duration_at_120bpm_44100hz() {
        let seq = Sequencer::new();
        // At 120 BPM: one quarter note = 0.5s = 22050 samples
        // One sixteenth = 22050 / 4 = 5512.5
        let dur = seq.step_duration_samples(0, 120.0, 44100.0);
        assert!((dur - 5512.5).abs() < 0.1);
    }

    #[test]
    fn swing_lengthens_even_steps_shortens_odd() {
        let mut seq = Sequencer::new();
        seq.set_swing(0.5);
        let base = 5512.5; // 120 BPM, 44100 Hz
        let swing_offset = 0.5 * base * 0.5; // 1378.125

        let even_dur = seq.step_duration_samples(0, 120.0, 44100.0);
        let odd_dur = seq.step_duration_samples(1, 120.0, 44100.0);

        assert!((even_dur - (base + swing_offset)).abs() < 0.1);
        assert!((odd_dur - (base - swing_offset)).abs() < 0.1);
    }

    #[test]
    fn process_triggers_enabled_steps_at_correct_positions() {
        let mut seq = Sequencer::new();
        // Enable step 0 on lane 0 (kick)
        seq.lanes_mut()[0].steps[0].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

        // Simulate: host at beat 0.0, playing, 120 BPM
        // Step 0 should fire immediately (at sample offset 0)
        let flags = dummy_flags();
        let triggers = seq.process_buffer(
            512,       // buffer_len
            true,      // host playing
            Some(120.0), // tempo
            Some(0.0),   // pos_beats (beat 0 = step 0)
            44100.0,
            &mut voices,
            &kit,
            &flags,
        );

        assert!(triggers > 0, "should have triggered at least one voice");
        assert!(voices.active_count() > 0, "voice pool should have active voices");
    }

    #[test]
    fn muted_lane_does_not_trigger() {
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;
        seq.lanes_mut()[0].muted = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        let triggers = seq.process_buffer(
            512, true, Some(120.0), Some(0.0), 44100.0,
            &mut voices, &kit, &flags,
        );

        assert_eq!(triggers, 0, "muted lane should not trigger");
        assert_eq!(voices.active_count(), 0);
    }

    #[test]
    fn probability_zero_never_triggers() {
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;
        seq.lanes_mut()[0].steps[0].probability = 0.0;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        // Run it several times — should never trigger
        for beat in 0..10 {
            let triggers = seq.process_buffer(
                512, true, Some(120.0), Some(beat as f64 * 4.0), 44100.0,
                &mut voices, &kit, &flags,
            );
            if triggers > 0 {
                panic!("probability 0.0 should never trigger (beat {beat})");
            }
        }
    }

    #[test]
    fn no_trigger_when_host_stopped() {
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        let triggers = seq.process_buffer(
            512, false, Some(120.0), Some(0.0), 44100.0,
            &mut voices, &kit, &flags,
        );

        assert_eq!(triggers, 0, "should not trigger when host is stopped");
    }

    #[test]
    fn full_pattern_cycles_through_16_steps() {
        let mut seq = Sequencer::new();
        // Enable step 0 on lane 0, step 4 on lane 1, step 8 on lane 2
        seq.lanes_mut()[0].steps[0].enabled = true;
        seq.lanes_mut()[1].steps[4].enabled = true;
        seq.lanes_mut()[2].steps[8].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        // At 120 BPM, one full pattern (16 sixteenths = 4 beats) = 2 seconds = 88200 samples
        // Process in 512-sample blocks
        let mut total_triggers = 0;
        let samples_per_pattern: usize = 88200;
        let block_size: usize = 512;
        let blocks = samples_per_pattern / block_size;

        for block in 0..blocks {
            let beat_pos = (block * block_size) as f64 / 44100.0 * (120.0 / 60.0);
            let triggers = seq.process_buffer(
                block_size, true, Some(120.0), Some(beat_pos), 44100.0,
                &mut voices, &kit, &flags,
            );
            total_triggers += triggers;
        }

        // Should have triggered exactly 3 times (one per enabled step)
        assert_eq!(total_triggers, 3, "expected 3 triggers across one full pattern");
    }

    #[test]
    fn swing_does_not_change_total_pattern_length() {
        // With swing, even steps get longer and odd steps get shorter,
        // but the total cycle should remain the same.
        let mut seq = Sequencer::new();
        seq.set_swing(0.7);

        let total: f64 = (0..16)
            .map(|s| seq.step_duration_samples(s, 120.0, 44100.0))
            .sum();

        // Without swing, total = 16 * 5512.5 = 88200.0
        assert!((total - 88200.0).abs() < 0.1, "swing should preserve total pattern length");
    }

    #[test]
    fn host_rewind_resyncs_sequencer() {
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;
        seq.lanes_mut()[0].steps[8].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        // Play forward to beat 2.0 (step 8)
        let triggers1 = seq.process_buffer(
            512, true, Some(120.0), Some(2.0), 44100.0,
            &mut voices, &kit, &flags,
        );
        assert!(triggers1 > 0, "should fire step 8 at beat 2.0");

        // Host rewinds to beat 0.0 — sequencer should resync and fire step 0
        let triggers2 = seq.process_buffer(
            512, true, Some(120.0), Some(0.0), 44100.0,
            &mut voices, &kit, &flags,
        );
        assert!(triggers2 > 0, "should fire step 0 after rewind to beat 0.0");
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
    fn condition_trig_default_is_always() {
        assert_eq!(ConditionTrig::default(), ConditionTrig::Always);
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
    fn step_with_plocks() {
        let step = Step {
            enabled: true,
            velocity: 0.6,
            probability: 1.0,
            pan: Some(-0.5),
            pitch: Some(7.0),
            condition: ConditionTrig::Fill,
        };
        assert_eq!(step.pan, Some(-0.5));
        assert_eq!(step.pitch, Some(7.0));
        assert_eq!(step.condition, ConditionTrig::Fill);
    }

    #[test]
    fn snapshot_captures_sequencer_state() {
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;
        seq.lanes_mut()[0].steps[0].velocity = 0.6;
        seq.lanes_mut()[3].muted = true;
        seq.set_swing(0.3);

        let snap = seq.snapshot();
        assert!(snap.patterns[0].lanes[0].steps[0].enabled);
        assert!((snap.patterns[0].lanes[0].steps[0].velocity - 0.6).abs() < 0.001);
        assert!(snap.patterns[0].lanes[3].muted);
        assert!((snap.patterns[0].swing - 0.3).abs() < 0.001);
    }

    #[test]
    fn restore_applies_sequencer_snapshot() {
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;
        seq.set_swing(0.5);

        // Capture, then modify
        let snap = seq.snapshot();
        seq.lanes_mut()[0].steps[0].enabled = false;
        seq.set_swing(0.0);

        // Restore
        seq.restore(&snap);
        assert!(seq.lanes()[0].steps[0].enabled);
        assert!((seq.swing() - 0.5).abs() < 0.001);
    }

    #[test]
    fn negative_pos_beats_does_not_trigger() {
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        let triggers = seq.process_buffer(
            512, true, Some(120.0), Some(-1.0), 44100.0,
            &mut voices, &kit, &flags,
        );
        assert_eq!(triggers, 0, "negative pos_beats should not trigger");
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

    #[test]
    fn condition_always_fires() {
        let seq = Sequencer::new();
        assert!(seq.evaluate_condition(ConditionTrig::Always));
    }

    #[test]
    fn pattern_queued_switches_at_bar_boundary() {
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[0].steps[0].enabled = true;
        bank.patterns[1].lanes[1].steps[0].enabled = true;
        bank.queued = Some(1);

        assert_eq!(bank.active, 0);
        assert!(bank.active_pattern().lanes[0].steps[0].enabled);

        // Simulate bar boundary switch
        if let Some(queued) = bank.queued.take() {
            bank.active = queued;
        }
        assert_eq!(bank.active, 1);
        assert!(bank.active_pattern().lanes[1].steps[0].enabled);
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
        assert_eq!(restored.patterns[0].lanes[0].steps[0].condition, ConditionTrig::Fill);
        assert_eq!(restored.patterns[0].lanes[0].steps[3].pan, Some(-0.5));
        assert!((restored.patterns[0].swing - 0.4).abs() < 0.001);
    }

    #[test]
    fn internal_play_fires_all_four_on_the_floor_kicks() {
        // Simulates internal play: linearly ramping beats, no DAW transport.
        // Verifies that a 4/4 kick (steps 0,4,8,12) fires exactly 4 times per pattern.
        let mut seq = Sequencer::new();
        seq.lanes_mut()[0].steps[0].enabled = true;
        seq.lanes_mut()[0].steps[4].enabled = true;
        seq.lanes_mut()[0].steps[8].enabled = true;
        seq.lanes_mut()[0].steps[12].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        let tempo = 120.0_f64;
        let sr = 44100.0_f32;
        let block_size = 512_usize;
        // One full pattern at 120 BPM = 4 beats = 2s = 88200 samples
        let samples_per_pattern = 88200_usize;
        let blocks = samples_per_pattern / block_size;

        let mut total_triggers = 0;
        let mut internal_samples: u64 = 0;

        for _ in 0..blocks {
            let beats = internal_samples as f64 / sr as f64 * (tempo / 60.0);
            internal_samples += block_size as u64;
            let triggers = seq.process_buffer(
                block_size, true, Some(tempo), Some(beats), sr,
                &mut voices, &kit, &flags,
            );
            total_triggers += triggers;
        }

        assert_eq!(total_triggers, 4, "4/4 kick should fire exactly 4 times per pattern");
    }

    #[test]
    fn internal_play_with_patterns_fires_all_steps() {
        // Same test but using process_buffer_with_patterns (the actual code path for GUI).
        let mut seq = Sequencer::new();
        let mut bank = PatternBank::new();
        bank.patterns[0].lanes[0].steps[0].enabled = true;
        bank.patterns[0].lanes[0].steps[4].enabled = true;
        bank.patterns[0].lanes[0].steps[8].enabled = true;
        bank.patterns[0].lanes[0].steps[12].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);
        let flags = dummy_flags();

        let tempo = 120.0_f64;
        let sr = 44100.0_f32;
        let block_size = 512_usize;
        let samples_per_pattern = 88200_usize;
        let blocks = samples_per_pattern / block_size;

        let mut total_triggers = 0;
        let mut internal_samples: u64 = 0;

        for _ in 0..blocks {
            let beats = internal_samples as f64 / sr as f64 * (tempo / 60.0);
            internal_samples += block_size as u64;
            let triggers = seq.process_buffer_with_patterns(
                block_size, true, Some(tempo), Some(beats), sr,
                &mut voices, &kit, &mut bank, &flags,
            );
            total_triggers += triggers;
        }

        assert_eq!(total_triggers, 4, "4/4 kick should fire exactly 4 times per pattern");
    }
}
