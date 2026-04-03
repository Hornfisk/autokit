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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
