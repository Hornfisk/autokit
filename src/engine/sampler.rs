use std::sync::Arc;

use nih_plug::util::permit_alloc;

use crate::engine::kit::DrumKit;

/// Number of simultaneous voices.
const MAX_VOICES: usize = 32;

/// Fade-out duration in seconds when re-triggering the same pad.
const RETRIGGER_FADE_SECS: f32 = 0.05; // 50ms

/// A single playback voice.
struct Voice {
    /// Which pad (0..15) this voice is playing, or None if inactive.
    pad_index: Option<usize>,
    /// Sample data reference (shared with DrumPad).
    sample: Option<Arc<Vec<f32>>>,
    /// Current playback position in samples.
    position: usize,
    /// Velocity gain (0.0–1.0).
    velocity: f32,
    /// Monotonic counter set at trigger time — used for oldest-voice stealing.
    age: u64,
    /// If > 0, voice is fading out. Counts down in samples.
    fade_remaining: usize,
    /// Total fade length in samples (set once at fade start).
    fade_length: usize,
    /// Samples to skip at buffer start (for sample-accurate sequencer triggers).
    start_offset: usize,
}

impl Voice {
    fn new() -> Self {
        Self {
            pad_index: None,
            sample: None,
            position: 0,
            velocity: 0.0,
            age: 0,
            fade_remaining: 0,
            fade_length: 0,
            start_offset: 0,
        }
    }

    fn is_active(&self) -> bool {
        self.pad_index.is_some()
    }

    fn deactivate(&mut self) {
        self.pad_index = None;
        // Drop the Arc outside the hot loop — permit_alloc wraps this
        // since Arc::drop can deallocate
        permit_alloc(|| {
            self.sample = None;
        });
    }

    fn start_fade_out(&mut self, fade_samples: usize) {
        if self.fade_remaining == 0 && self.is_active() {
            self.fade_remaining = fade_samples;
            self.fade_length = fade_samples;
        }
    }

    /// Render one sample. Returns (left, right) or None if voice is done.
    fn next_sample(&mut self, pan: f32) -> Option<(f32, f32)> {
        if !self.is_active() {
            return None;
        }

        let data = match &self.sample {
            Some(s) => s,
            None => {
                self.pad_index = None;
                return None;
            }
        };

        if self.position >= data.len() {
            self.pad_index = None;
            return None;
        }

        let mut s = data[self.position] * self.velocity;
        self.position += 1;

        // Apply fade-out envelope if active
        if self.fade_remaining > 0 {
            let fade_gain = self.fade_remaining as f32 / self.fade_length as f32;
            s *= fade_gain;
            self.fade_remaining -= 1;
            if self.fade_remaining == 0 {
                self.pad_index = None;
                return Some((0.0, 0.0));
            }
        }

        // Constant-power pan: pan in [-1, 1], center = 0
        let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI; // 0..PI/2
        let left = s * angle.cos();
        let right = s * angle.sin();
        Some((left, right))
    }
}

/// Pool of voices that mixes triggered drum samples into stereo output.
pub struct VoicePool {
    voices: Vec<Voice>,
    /// Monotonic counter for age-based voice stealing.
    trigger_counter: u64,
    /// Fade-out length in samples (computed from sample rate).
    fade_samples: usize,
}

impl VoicePool {
    pub fn new(sample_rate: f32) -> Self {
        let fade_samples = (RETRIGGER_FADE_SECS * sample_rate) as usize;
        tracing::info!(MAX_VOICES, fade_samples, "VoicePool created");
        Self {
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            trigger_counter: 0,
            fade_samples,
        }
    }

    /// Trigger a pad. Fades out any existing voices on the same pad,
    /// then allocates a new voice.
    /// Called from audio thread — all allocating ops wrapped in permit_alloc.
    pub fn trigger(&mut self, pad_index: usize, velocity: f32, kit: &DrumKit, start_offset: usize) {
        let pad = &kit.pads[pad_index];

        let sample = match &pad.sample {
            Some(s) => s,
            None => return,
        };

        // Fade out any active voices on this same pad (re-trigger)
        for voice in self.voices.iter_mut() {
            if voice.pad_index == Some(pad_index) && voice.fade_remaining == 0 {
                voice.start_fade_out(self.fade_samples);
            }
        }

        // Find a free voice, or steal the oldest
        let slot = self.find_free_or_steal();

        self.trigger_counter += 1;
        let voice = &mut self.voices[slot];
        voice.pad_index = Some(pad_index);
        // Arc::clone does atomic refcount increment — wrap in permit_alloc
        // because assert_no_alloc intercepts atomic ops on some platforms
        permit_alloc(|| {
            voice.sample = Some(Arc::clone(sample));
        });
        voice.position = 0;
        voice.velocity = velocity * pad.volume;
        voice.age = self.trigger_counter;
        voice.fade_remaining = 0;
        voice.fade_length = 0;
        voice.start_offset = start_offset;
    }

    fn find_free_or_steal(&mut self) -> usize {
        // Prefer an inactive voice
        if let Some(i) = self.voices.iter().position(|v| !v.is_active()) {
            return i;
        }

        // Steal oldest (lowest age counter)
        let (oldest_idx, _) = self
            .voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| v.age)
            .unwrap(); // safe: MAX_VOICES > 0

        // Deactivate stolen voice (drops its Arc, which may deallocate)
        self.voices[oldest_idx].deactivate();
        oldest_idx
    }

    /// Mix all active voices into the output buffer.
    /// `kit` provides per-pad pan values.
    pub fn process(&mut self, output_left: &mut [f32], output_right: &mut [f32], kit: &DrumKit) {
        for voice in self.voices.iter_mut() {
            if !voice.is_active() {
                continue;
            }

            let pan = voice
                .pad_index
                .map(|i| kit.pads[i].pan)
                .unwrap_or(0.0);

            for (i, (l, r)) in output_left.iter_mut().zip(output_right.iter_mut()).enumerate() {
                if i < voice.start_offset {
                    continue;
                }
                match voice.next_sample(pan) {
                    Some((vl, vr)) => {
                        *l += vl;
                        *r += vr;
                    }
                    None => break,
                }
            }

            // Reset offset so next buffer plays from sample 0
            voice.start_offset = 0;

            // Clean up finished voices
            if !voice.is_active() {
                voice.deactivate();
            }
        }
    }

    /// Number of currently active voices (for debug display).
    pub fn active_count(&self) -> usize {
        self.voices.iter().filter(|v| v.is_active()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kit::{DrumKit, SampleCategory};

    /// Build a minimal kit with one pad loaded with a known sample.
    fn test_kit() -> DrumKit {
        let mut kit = DrumKit::new();
        // 8 samples of 1.0 — easy to verify in output
        kit.pads[0].sample = Some(Arc::new(vec![1.0; 8]));
        kit.pads[0].volume = 1.0;
        kit.pads[0].pan = 0.0;
        kit.pads[0].category = SampleCategory::Kick;
        kit
    }

    #[test]
    fn trigger_with_zero_offset_plays_from_start() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(0, 1.0, &kit, 0);

        let mut left = vec![0.0f32; 8];
        let mut right = vec![0.0f32; 8];
        pool.process(&mut left, &mut right, &kit);

        // Pan center: constant-power pan gives cos(PI/4) ≈ 0.7071
        let expected = (0.25 * std::f32::consts::PI).cos();
        assert!((left[0] - expected).abs() < 0.001, "first sample should be non-zero");
        assert!((left[7] - expected).abs() < 0.001, "last sample should be non-zero");
    }

    #[test]
    fn trigger_with_offset_delays_playback() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(0, 1.0, &kit, 4); // start at sample 4

        let mut left = vec![0.0f32; 12];
        let mut right = vec![0.0f32; 12];
        pool.process(&mut left, &mut right, &kit);

        // Samples 0..4 should be silent
        for i in 0..4 {
            assert_eq!(left[i], 0.0, "sample {i} should be silent (before offset)");
        }

        // Samples 4..12 should have audio (8 samples of the pad)
        let expected = (0.25 * std::f32::consts::PI).cos();
        assert!((left[4] - expected).abs() < 0.001, "sample 4 should have audio");
    }

    #[test]
    fn start_offset_resets_after_first_buffer() {
        let mut pool = VoicePool::new(44100.0);
        let mut kit = DrumKit::new();
        kit.pads[0].sample = Some(Arc::new(vec![1.0; 100]));
        kit.pads[0].volume = 1.0;
        kit.pads[0].pan = 0.0;
        kit.pads[0].category = SampleCategory::Kick;

        pool.trigger(0, 1.0, &kit, 4);

        // First buffer: 8 samples, offset should apply (0..4 silent)
        let mut left1 = vec![0.0f32; 8];
        let mut right1 = vec![0.0f32; 8];
        pool.process(&mut left1, &mut right1, &kit);
        assert_eq!(left1[0], 0.0, "first buffer: sample 0 should be silent");

        // Second buffer: offset should be reset, audio starts at sample 0
        let mut left2 = vec![0.0f32; 8];
        let mut right2 = vec![0.0f32; 8];
        pool.process(&mut left2, &mut right2, &kit);
        let expected = (0.25 * std::f32::consts::PI).cos();
        assert!((left2[0] - expected).abs() < 0.001, "second buffer: sample 0 should have audio (offset reset)");
    }
}
