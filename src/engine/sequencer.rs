use crate::engine::kit::DrumKit;
use crate::engine::sampler::VoicePool;

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
    pub lanes: Vec<Lane>,
    pub swing: f32,
    playing: bool,
    current_step: usize,
    tick_accumulator: f64,
    last_pos_beats: f64,
}

impl Sequencer {
    pub fn new() -> Self {
        Self {
            lanes: (0..16).map(Lane::new).collect(),
            swing: 0.0,
            playing: false,
            current_step: 0,
            tick_accumulator: 0.0,
            last_pos_beats: 0.0,
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

    pub fn current_step(&self) -> usize {
        self.current_step
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Fire all enabled, non-muted lanes for the current step.
    fn fire_step(
        &self,
        sample_offset: usize,
        voices: &mut VoicePool,
        kit: &DrumKit,
    ) -> usize {
        let step_idx = self.current_step;
        let mut count = 0;

        for lane in &self.lanes {
            if lane.muted {
                continue;
            }

            let step = &lane.steps[step_idx];
            if !step.enabled {
                continue;
            }

            // Probability gate
            if step.probability < 1.0 {
                let roll: f32 = rand::random();
                if roll >= step.probability {
                    continue;
                }
            }

            voices.trigger(lane.pad_index, step.velocity, kit, sample_offset);
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
}
