use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::engine::kit::DrumKit;
use crate::engine::sampler::VoicePool;
use crate::util::history::{StepSnapshot, LaneSnapshot, SequencerSnapshot};

/// A single step in the sequencer.
#[derive(Clone, Copy)]
pub struct Step {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            enabled: false,
            velocity: 0.8,
            probability: 1.0,
        }
    }
}

/// One lane = one pad's 16-step sequence.
pub struct Lane {
    pub pad_index: usize,
    pub steps: [Step; 16],
    pub muted: bool,
}

impl Lane {
    pub fn new(pad_index: usize) -> Self {
        Self {
            pad_index,
            steps: [Step::default(); 16],
            muted: false,
        }
    }
}

/// The 16-lane, 16-step sequencer.
pub struct Sequencer {
    pub lanes: [Lane; 16],
    pub swing: f32,
    playing: bool,
    current_step: usize,
    tick_accumulator: f64,
    last_pos_beats: f64,
    rng: SmallRng,
}

impl Sequencer {
    pub fn new() -> Self {
        let lanes: [Lane; 16] = core::array::from_fn(|i| Lane::new(i));
        Self {
            lanes,
            swing: 0.0,
            playing: false,
            current_step: 0,
            tick_accumulator: 0.0,
            last_pos_beats: 0.0,
            rng: SmallRng::from_os_rng(),
        }
    }

    /// Compute duration of a step in samples, accounting for swing.
    /// Even steps (0,2,4,...) are lengthened, odd steps (1,3,5,...) are shortened.
    pub fn step_duration_samples(&self, step: usize, tempo: f64, sample_rate: f32) -> f64 {
        let base = sample_rate as f64 * 60.0 / tempo / 4.0;
        let swing_offset = self.swing as f64 * base * 0.5;
        if step % 2 == 0 {
            base + swing_offset
        } else {
            base - swing_offset
        }
    }

    /// Process one audio buffer. Scans for step boundaries, triggers voices.
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
    ) -> usize {
        let tempo = match (host_playing, tempo) {
            (true, Some(t)) if t > 0.0 => t,
            _ => {
                self.playing = false;
                return 0;
            }
        };

        // Sync to host position
        let mut fire_immediately = false;
        if let Some(beats) = pos_beats {
            // Guard against negative beat positions (pre-roll / count-in)
            if beats < 0.0 {
                self.playing = false;
                return 0;
            }
            let sixteenths = beats * 4.0;
            let host_step = ((sixteenths.floor() as usize) % 16) as usize;
            let frac = sixteenths.fract();

            // Detect jump: if host position doesn't match our expectation, resync
            let expected_beats = self.last_pos_beats
                + (self.tick_accumulator / sample_rate as f64) * (tempo / 60.0);
            let drift = (beats - expected_beats).abs();

            if !self.playing || drift > 0.01 {
                // Resync: snap to host position
                self.current_step = host_step;
                let step_dur = self.step_duration_samples(host_step, tempo, sample_rate);
                self.tick_accumulator = frac * step_dur;
                // If we landed exactly on a step boundary, fire it now
                if frac < 0.001 {
                    fire_immediately = true;
                }
            }

            self.last_pos_beats = beats;
        }

        self.playing = true;
        let mut triggered = 0usize;

        // Fire current step immediately if we just synced to a step boundary
        if fire_immediately {
            triggered += self.fire_step(0, voices, kit);
        }

        for sample_offset in 0..buffer_len {
            self.tick_accumulator += 1.0;
            let step_dur = self.step_duration_samples(self.current_step, tempo, sample_rate);

            if self.tick_accumulator >= step_dur {
                self.tick_accumulator -= step_dur;
                self.current_step = (self.current_step + 1) % 16;
                triggered += self.fire_step(sample_offset, voices, kit);
            }
        }

        // Update last_pos_beats for next buffer's drift detection
        self.last_pos_beats += (buffer_len as f64 / sample_rate as f64) * (tempo / 60.0);

        triggered
    }

    /// Capture the undoable sequencer state (steps, lanes, swing).
    /// Excludes playback state (playing, current_step, tick_accumulator, rng).
    pub fn snapshot(&self) -> SequencerSnapshot {
        let lanes: [LaneSnapshot; 16] = core::array::from_fn(|i| {
            let steps: [StepSnapshot; 16] = core::array::from_fn(|j| StepSnapshot {
                enabled: self.lanes[i].steps[j].enabled,
                velocity: self.lanes[i].steps[j].velocity,
                probability: self.lanes[i].steps[j].probability,
            });
            LaneSnapshot {
                steps,
                muted: self.lanes[i].muted,
            }
        });
        SequencerSnapshot {
            lanes,
            swing: self.swing,
        }
    }

    /// Restore sequencer state from a snapshot. Preserves playback state.
    pub fn restore(&mut self, snapshot: &SequencerSnapshot) {
        for (lane, snap_lane) in self.lanes.iter_mut().zip(snapshot.lanes.iter()) {
            for (step, snap_step) in lane.steps.iter_mut().zip(snap_lane.steps.iter()) {
                step.enabled = snap_step.enabled;
                step.velocity = snap_step.velocity;
                step.probability = snap_step.probability;
            }
            lane.muted = snap_lane.muted;
        }
        self.swing = snapshot.swing;
    }

    pub fn current_step(&self) -> usize {
        self.current_step
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Fire all enabled, non-muted lanes for the current step.
    fn fire_step(
        &mut self,
        sample_offset: usize,
        voices: &mut VoicePool,
        kit: &DrumKit,
    ) -> usize {
        let step_idx = self.current_step;
        let mut count = 0;

        for i in 0..self.lanes.len() {
            if self.lanes[i].muted {
                continue;
            }

            let step = &self.lanes[i].steps[step_idx];
            if !step.enabled {
                continue;
            }

            // Probability gate (uses stored RNG — no thread-local or allocation)
            if step.probability < 1.0 {
                let roll: f32 = self.rng.random();
                if roll >= step.probability {
                    continue;
                }
            }

            let velocity = step.velocity;
            let pad_index = self.lanes[i].pad_index;
            voices.trigger(pad_index, velocity, kit, sample_offset);
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
    fn new_sequencer_has_16_lanes_with_16_steps_each() {
        let seq = Sequencer::new();
        assert_eq!(seq.lanes.len(), 16);
        for (i, lane) in seq.lanes.iter().enumerate() {
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
        assert!((seq.swing - 0.0).abs() < 0.001);
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
        seq.swing = 0.5;
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
        seq.lanes[0].steps[0].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

        // Simulate: host at beat 0.0, playing, 120 BPM
        // Step 0 should fire immediately (at sample offset 0)
        let triggers = seq.process_buffer(
            512,       // buffer_len
            true,      // host playing
            Some(120.0), // tempo
            Some(0.0),   // pos_beats (beat 0 = step 0)
            44100.0,
            &mut voices,
            &kit,
        );

        assert!(triggers > 0, "should have triggered at least one voice");
        assert!(voices.active_count() > 0, "voice pool should have active voices");
    }

    #[test]
    fn muted_lane_does_not_trigger() {
        let mut seq = Sequencer::new();
        seq.lanes[0].steps[0].enabled = true;
        seq.lanes[0].muted = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

        let triggers = seq.process_buffer(
            512, true, Some(120.0), Some(0.0), 44100.0,
            &mut voices, &kit,
        );

        assert_eq!(triggers, 0, "muted lane should not trigger");
        assert_eq!(voices.active_count(), 0);
    }

    #[test]
    fn probability_zero_never_triggers() {
        let mut seq = Sequencer::new();
        seq.lanes[0].steps[0].enabled = true;
        seq.lanes[0].steps[0].probability = 0.0;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

        // Run it several times — should never trigger
        for beat in 0..10 {
            let triggers = seq.process_buffer(
                512, true, Some(120.0), Some(beat as f64 * 4.0), 44100.0,
                &mut voices, &kit,
            );
            if triggers > 0 {
                panic!("probability 0.0 should never trigger (beat {beat})");
            }
        }
    }

    #[test]
    fn no_trigger_when_host_stopped() {
        let mut seq = Sequencer::new();
        seq.lanes[0].steps[0].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

        let triggers = seq.process_buffer(
            512, false, Some(120.0), Some(0.0), 44100.0,
            &mut voices, &kit,
        );

        assert_eq!(triggers, 0, "should not trigger when host is stopped");
    }

    #[test]
    fn full_pattern_cycles_through_16_steps() {
        let mut seq = Sequencer::new();
        // Enable step 0 on lane 0, step 4 on lane 1, step 8 on lane 2
        seq.lanes[0].steps[0].enabled = true;
        seq.lanes[1].steps[4].enabled = true;
        seq.lanes[2].steps[8].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

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
                &mut voices, &kit,
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
        seq.swing = 0.7;

        let total: f64 = (0..16)
            .map(|s| seq.step_duration_samples(s, 120.0, 44100.0))
            .sum();

        // Without swing, total = 16 * 5512.5 = 88200.0
        assert!((total - 88200.0).abs() < 0.1, "swing should preserve total pattern length");
    }

    #[test]
    fn host_rewind_resyncs_sequencer() {
        let mut seq = Sequencer::new();
        seq.lanes[0].steps[0].enabled = true;
        seq.lanes[0].steps[8].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

        // Play forward to beat 2.0 (step 8)
        let triggers1 = seq.process_buffer(
            512, true, Some(120.0), Some(2.0), 44100.0,
            &mut voices, &kit,
        );
        assert!(triggers1 > 0, "should fire step 8 at beat 2.0");

        // Host rewinds to beat 0.0 — sequencer should resync and fire step 0
        let triggers2 = seq.process_buffer(
            512, true, Some(120.0), Some(0.0), 44100.0,
            &mut voices, &kit,
        );
        assert!(triggers2 > 0, "should fire step 0 after rewind to beat 0.0");
    }

    #[test]
    fn snapshot_captures_sequencer_state() {
        let mut seq = Sequencer::new();
        seq.lanes[0].steps[0].enabled = true;
        seq.lanes[0].steps[0].velocity = 0.6;
        seq.lanes[3].muted = true;
        seq.swing = 0.3;

        let snap = seq.snapshot();
        assert!(snap.lanes[0].steps[0].enabled);
        assert!((snap.lanes[0].steps[0].velocity - 0.6).abs() < 0.001);
        assert!(snap.lanes[3].muted);
        assert!((snap.swing - 0.3).abs() < 0.001);
    }

    #[test]
    fn restore_applies_sequencer_snapshot() {
        let mut seq = Sequencer::new();
        seq.lanes[0].steps[0].enabled = true;
        seq.swing = 0.5;

        // Capture, then modify
        let snap = seq.snapshot();
        seq.lanes[0].steps[0].enabled = false;
        seq.swing = 0.0;

        // Restore
        seq.restore(&snap);
        assert!(seq.lanes[0].steps[0].enabled);
        assert!((seq.swing - 0.5).abs() < 0.001);
    }

    #[test]
    fn negative_pos_beats_does_not_trigger() {
        let mut seq = Sequencer::new();
        seq.lanes[0].steps[0].enabled = true;

        let kit = test_kit();
        let mut voices = VoicePool::new(44100.0);

        let triggers = seq.process_buffer(
            512, true, Some(120.0), Some(-1.0), 44100.0,
            &mut voices, &kit,
        );
        assert_eq!(triggers, 0, "negative pos_beats should not trigger");
    }
}
