/// Zero-alloc linear ramp to a target value over a fixed number of samples.
///
/// Used by the FX bus to ease to a new target (e.g. reverb mix) over 1/8 of
/// a sequencer step on step boundaries — makes pattern changes feel
/// hand-turned instead of instant. Not a general-purpose smoother; the
/// ramp length is set explicitly on each `set_target_now` call.
pub struct StepSmoother {
    current: f32,
    target: f32,
    inc_per_sample: f32,
    samples_remaining: u32,
}

impl StepSmoother {
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            inc_per_sample: 0.0,
            samples_remaining: 0,
        }
    }

    /// Snap both current and target — no ramp. Call on sample-rate change or reset.
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.inc_per_sample = 0.0;
        self.samples_remaining = 0;
    }

    /// Start a new ramp to `target` over `ramp_samples` samples.
    /// `ramp_samples == 0` snaps immediately.
    #[inline]
    pub fn set_target_now(&mut self, target: f32, ramp_samples: u32) {
        self.target = target;
        if ramp_samples == 0 || (target - self.current).abs() < 1e-9 {
            self.current = target;
            self.inc_per_sample = 0.0;
            self.samples_remaining = 0;
        } else {
            self.inc_per_sample = (target - self.current) / ramp_samples as f32;
            self.samples_remaining = ramp_samples;
        }
    }

    /// Advance one sample. Returns the current value.
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.samples_remaining > 0 {
            self.current += self.inc_per_sample;
            self.samples_remaining -= 1;
            if self.samples_remaining == 0 {
                self.current = self.target;
            }
        }
        self.current
    }

    #[inline]
    pub fn current(&self) -> f32 {
        self.current
    }
}

/// Compute ramp length = 1/8 of a 16th-note step at the given tempo.
///
/// At 120 BPM / 48 kHz: step = 48000 * 60 / 120 / 4 = 6000 samples,
/// ramp = 750 samples ≈ 15.6 ms. Fast enough to feel responsive, slow
/// enough to kill zipper noise on big jumps.
#[inline]
pub fn ramp_samples_for_tempo(sample_rate: f32, tempo_bpm: f32) -> u32 {
    let step = sample_rate * 60.0 / tempo_bpm.max(1.0) / 4.0;
    (step / 8.0).max(1.0) as u32
}
