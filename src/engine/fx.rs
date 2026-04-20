//! Master FX bus DSP: DJ filter, tempo-synced feedback delay, Schroeder reverb.
//!
//! All state is pre-allocated in `prepare()`. `process_sample()` does zero heap
//! work — safe to call under `assert_process_allocs`. Three processors share
//! one struct so the master bus gets a single `FxBus::process_sample` call per
//! sample and the per-sample knob values can be passed in one struct.
//!
//! Signal chain (applied in order inside `process_sample`):
//!     input → dj_filter → delay (wet mix) → reverb (wet mix) → output
//!
//! The DJ filter comes first so its sweep feeds the delay and reverb tails
//! musically (filter-kill a break and the wet tail goes with it). Delay
//! feeds reverb so echoes diffuse naturally.

use std::f32::consts::PI;

/// Targets driven by `StepSmoother`s upstream — one value per knob, sampled
/// once per sample in the master bus loop and passed in here. No smoothing
/// happens inside this module; it trusts that the caller has already ramped.
#[derive(Clone, Copy)]
pub struct FxTargets {
    /// 0.0 = dry, 1.0 = full wet reverb.
    pub reverb_mix: f32,
    /// 0.0 = dry, 1.0 = full wet delay.
    pub delay_mix: f32,
    /// Delay time in samples. Caller computes from tempo + sync division.
    pub delay_samples: f32,
    /// Bipolar DJ filter: -1.0 = lowpass kill, 0.0 = bypass, +1.0 = highpass kill.
    pub dj_filter: f32,
}

impl Default for FxTargets {
    fn default() -> Self {
        Self {
            reverb_mix: 0.0,
            delay_mix: 0.0,
            delay_samples: 0.0,
            dj_filter: 0.0,
        }
    }
}

// ── DJ filter (state-variable, bipolar) ──────────────────────────────────
//
// Single 2-pole SVF per channel. The bipolar knob morphs between
// "lowpass-kill" (−1) and "highpass-kill" (+1) through a hard-bypass at 0.
// The cutoff sweeps from one end of the band to the other as the knob
// moves, matching the feel of a DJ mixer's isolator-style filter.

const DJ_MIN_HZ: f32 = 40.0;
const DJ_MAX_HZ: f32 = 18_000.0;

struct SvfChannel {
    ic1eq: f32,
    ic2eq: f32,
}

impl SvfChannel {
    const fn new() -> Self {
        Self { ic1eq: 0.0, ic2eq: 0.0 }
    }

    fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Run one sample through the SVF, returning (lowpass, highpass).
    /// Coefficients (g, k, a*) are recomputed per-sample in the caller so
    /// the knob can sweep freely.
    #[inline]
    fn tick(&mut self, x: f32, g: f32, k: f32) -> (f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let v3 = x - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + g * v1;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        let lp = v2;
        let hp = x - k * v1 - v2;
        (lp, hp)
    }
}

pub struct DjFilter {
    left: SvfChannel,
    right: SvfChannel,
    sample_rate: f32,
}

impl DjFilter {
    pub const fn new() -> Self {
        Self {
            left: SvfChannel::new(),
            right: SvfChannel::new(),
            sample_rate: 44_100.0,
        }
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.left.reset();
        self.right.reset();
    }

    #[inline]
    pub fn process_sample(&mut self, l: f32, r: f32, knob: f32) -> (f32, f32) {
        // Always tick the filter so its state stays fresh — even when the
        // knob is near zero. A stale state + sudden activation is the main
        // click source on pattern switches that change the filter level.
        //
        // Near zero we crossfade wet→dry with a smooth window so the polarity
        // flip at knob=0 (HP ↔ LP) gets multiplied by ~0 and never produces a
        // discontinuity in the output. At |knob| < 0.005 the output is pure
        // dry; at |knob| > 0.04 it's pure wet; in between an equal-power
        // taper. At those extreme cutoffs the wet signal is already very
        // close to dry anyway, so the crossfade is acoustically transparent.
        let k = 0.7;

        let log_min = DJ_MIN_HZ.ln();
        let log_max = DJ_MAX_HZ.ln();
        let amount = knob.abs().min(1.0);

        let cutoff_hz = if knob < 0.0 {
            (log_max + (log_min - log_max) * amount).exp()
        } else {
            (log_min + (log_max - log_min) * amount).exp()
        };

        let g = (PI * cutoff_hz / self.sample_rate).tan();

        let (lp_l, hp_l) = self.left.tick(l, g, k);
        let (lp_r, hp_r) = self.right.tick(r, g, k);

        let (wet_l, wet_r) = if knob < 0.0 {
            (lp_l, lp_r)
        } else {
            (hp_l, hp_r)
        };

        // Smooth wet/dry mix as a function of |knob|.
        let abs_k = knob.abs();
        let mix_lin = ((abs_k - 0.005) / 0.035).clamp(0.0, 1.0);
        // Equal-power taper avoids a 6 dB notch at the crossover.
        let wet_gain = (mix_lin * std::f32::consts::FRAC_PI_2).sin();
        let dry_gain = (mix_lin * std::f32::consts::FRAC_PI_2).cos();

        (wet_l * wet_gain + l * dry_gain, wet_r * wet_gain + r * dry_gain)
    }
}

// ── Feedback delay ───────────────────────────────────────────────────────
//
// Stereo circular buffer sized for the slowest musical delay we'd ever want
// (1/4 note at 40 BPM ≈ 1.5 s). Fixed ~35 % feedback, one-pole LP in the
// feedback path to darken repeats so they don't pile up harshness.

const DELAY_MAX_SECS: f32 = 2.0;
const DELAY_FEEDBACK: f32 = 0.38;
const DELAY_DAMP: f32 = 0.25; // 1-pole LP coeff (0 = no damping, 1 = full kill)

pub struct Delay {
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write_pos: usize,
    buf_len: usize,
    damp_state_l: f32,
    damp_state_r: f32,
}

impl Delay {
    pub fn new() -> Self {
        Self {
            buf_l: Vec::new(),
            buf_r: Vec::new(),
            write_pos: 0,
            buf_len: 0,
            damp_state_l: 0.0,
            damp_state_r: 0.0,
        }
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        let len = (sample_rate * DELAY_MAX_SECS).ceil() as usize;
        self.buf_l.resize(len, 0.0);
        self.buf_r.resize(len, 0.0);
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.buf_len = len;
        self.write_pos = 0;
        self.damp_state_l = 0.0;
        self.damp_state_r = 0.0;
    }

    /// Process one stereo sample, returning the wet output (pre-mix).
    /// Caller handles dry/wet blending via `FxTargets::delay_mix`.
    #[inline]
    pub fn process_sample(&mut self, l: f32, r: f32, delay_samples: f32) -> (f32, f32) {
        if self.buf_len == 0 {
            return (0.0, 0.0);
        }

        // Clamp requested delay to something we actually have buffer for.
        let d = delay_samples.clamp(1.0, (self.buf_len - 2) as f32);

        // Linear-interpolated read head.
        let read_f = self.write_pos as f32 - d;
        let read_f = if read_f < 0.0 {
            read_f + self.buf_len as f32
        } else {
            read_f
        };
        let i0 = read_f.floor() as usize % self.buf_len;
        let i1 = (i0 + 1) % self.buf_len;
        let frac = read_f - read_f.floor();

        let wet_l = self.buf_l[i0] * (1.0 - frac) + self.buf_l[i1] * frac;
        let wet_r = self.buf_r[i0] * (1.0 - frac) + self.buf_r[i1] * frac;

        // Darken feedback path with a 1-pole LP.
        self.damp_state_l = self.damp_state_l + DELAY_DAMP * (wet_l - self.damp_state_l);
        self.damp_state_r = self.damp_state_r + DELAY_DAMP * (wet_r - self.damp_state_r);

        let fb_l = self.damp_state_l * DELAY_FEEDBACK;
        let fb_r = self.damp_state_r * DELAY_FEEDBACK;

        self.buf_l[self.write_pos] = l + fb_l;
        self.buf_r[self.write_pos] = r + fb_r;
        self.write_pos += 1;
        if self.write_pos >= self.buf_len {
            self.write_pos = 0;
        }

        (wet_l, wet_r)
    }
}

// ── Schroeder reverb ─────────────────────────────────────────────────────
//
// 4 parallel comb filters → 2 series allpasses per channel. Classic
// Schroeder topology — not the most lush reverb in the world, but cheap,
// stable, zero-alloc, and sounds "good enough" for a master bus effect.
//
// Delay line lengths in samples at 44.1 kHz (from the Freeverb/Schroeder
// literature) — scaled to the actual sample rate in `prepare()`.

const COMB_TUNINGS_L: [usize; 4] = [1116, 1188, 1277, 1356];
const COMB_TUNINGS_R: [usize; 4] = [1139, 1211, 1300, 1379];
const ALLPASS_TUNINGS_L: [usize; 2] = [556, 441];
const ALLPASS_TUNINGS_R: [usize; 2] = [579, 464];

const COMB_FEEDBACK: f32 = 0.84;
const COMB_DAMP: f32 = 0.2;
const ALLPASS_FEEDBACK: f32 = 0.5;

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    lp_state: f32,
}

impl Comb {
    fn new() -> Self {
        Self { buf: Vec::new(), idx: 0, lp_state: 0.0 }
    }

    fn prepare(&mut self, size: usize) {
        self.buf.resize(size, 0.0);
        self.buf.fill(0.0);
        self.idx = 0;
        self.lp_state = 0.0;
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let out = self.buf[self.idx];
        // Damped feedback — the comb's "high-frequency absorption" that
        // turns a pure comb into a reverb-ish decay.
        self.lp_state = out * (1.0 - COMB_DAMP) + self.lp_state * COMB_DAMP;
        self.buf[self.idx] = x + self.lp_state * COMB_FEEDBACK;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new() -> Self {
        Self { buf: Vec::new(), idx: 0 }
    }

    fn prepare(&mut self, size: usize) {
        self.buf.resize(size, 0.0);
        self.buf.fill(0.0);
        self.idx = 0;
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let bufout = self.buf[self.idx];
        let out = -x + bufout;
        self.buf[self.idx] = x + bufout * ALLPASS_FEEDBACK;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

pub struct Reverb {
    combs_l: [Comb; 4],
    combs_r: [Comb; 4],
    allpasses_l: [Allpass; 2],
    allpasses_r: [Allpass; 2],
}

impl Reverb {
    pub fn new() -> Self {
        Self {
            combs_l: [Comb::new(), Comb::new(), Comb::new(), Comb::new()],
            combs_r: [Comb::new(), Comb::new(), Comb::new(), Comb::new()],
            allpasses_l: [Allpass::new(), Allpass::new()],
            allpasses_r: [Allpass::new(), Allpass::new()],
        }
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        // Scale the 44.1 kHz reference tunings to the actual rate.
        let scale = sample_rate / 44_100.0;
        for (i, comb) in self.combs_l.iter_mut().enumerate() {
            comb.prepare((COMB_TUNINGS_L[i] as f32 * scale) as usize);
        }
        for (i, comb) in self.combs_r.iter_mut().enumerate() {
            comb.prepare((COMB_TUNINGS_R[i] as f32 * scale) as usize);
        }
        for (i, ap) in self.allpasses_l.iter_mut().enumerate() {
            ap.prepare((ALLPASS_TUNINGS_L[i] as f32 * scale) as usize);
        }
        for (i, ap) in self.allpasses_r.iter_mut().enumerate() {
            ap.prepare((ALLPASS_TUNINGS_R[i] as f32 * scale) as usize);
        }
    }

    /// Returns the wet signal (pre-mix). Caller handles dry/wet.
    #[inline]
    pub fn process_sample(&mut self, l: f32, r: f32) -> (f32, f32) {
        // Slight input gain reduction — the parallel combs sum loud.
        let input_l = l * 0.015;
        let input_r = r * 0.015;

        let mut out_l = 0.0;
        let mut out_r = 0.0;
        for comb in &mut self.combs_l {
            out_l += comb.tick(input_l);
        }
        for comb in &mut self.combs_r {
            out_r += comb.tick(input_r);
        }
        for ap in &mut self.allpasses_l {
            out_l = ap.tick(out_l);
        }
        for ap in &mut self.allpasses_r {
            out_r = ap.tick(out_r);
        }
        (out_l, out_r)
    }
}

// ── FX bus (wires the three together) ────────────────────────────────────

pub struct FxBus {
    pub dj_filter: DjFilter,
    pub delay: Delay,
    pub reverb: Reverb,
}

impl FxBus {
    pub fn new() -> Self {
        Self {
            dj_filter: DjFilter::new(),
            delay: Delay::new(),
            reverb: Reverb::new(),
        }
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        self.dj_filter.prepare(sample_rate);
        self.delay.prepare(sample_rate);
        self.reverb.prepare(sample_rate);
    }

    /// Process one stereo sample through DJ filter → delay → reverb, with
    /// dry/wet mixing applied per-stage. `targets` is sampled once per
    /// sample by the caller (already StepSmoother-ramped).
    #[inline]
    pub fn process_sample(&mut self, l: f32, r: f32, targets: &FxTargets) -> (f32, f32) {
        // Stage 1: DJ filter (replaces dry signal — it's a color/kill effect,
        // not a send).
        let (mut l, mut r) = self.dj_filter.process_sample(l, r, targets.dj_filter);

        // Stage 2: Delay send — wet is a mix, dry passes through.
        let (dwet_l, dwet_r) = self.delay.process_sample(l, r, targets.delay_samples);
        let dmix = targets.delay_mix.clamp(0.0, 1.0);
        l += dwet_l * dmix;
        r += dwet_r * dmix;

        // Stage 3: Reverb send — same pattern.
        let (rwet_l, rwet_r) = self.reverb.process_sample(l, r);
        let rmix = targets.reverb_mix.clamp(0.0, 1.0);
        l += rwet_l * rmix;
        r += rwet_r * rmix;

        (l, r)
    }
}
