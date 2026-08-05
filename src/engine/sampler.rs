use std::sync::Arc;

use nih_plug::util::permit_alloc;

use crate::engine::kit::DrumKit;

/// Number of simultaneous voices.
const MAX_VOICES: usize = 32;

/// Fade-out duration in seconds when re-triggering the same pad.
const RETRIGGER_FADE_SECS: f32 = 0.05; // 50ms

/// A single playback voice.
struct Voice {
    /// Which pad this voice is playing, or None if inactive.
    pad_index: Option<usize>,
    /// Sample data reference (shared with DrumPad).
    sample: Option<Arc<Vec<f32>>>,
    /// Current playback position in source samples (fractional for pitch shifting).
    position: f64,
    /// Playback rate multiplier: 2^(pitch_semitones/12). 1.0 = original pitch.
    rate: f64,
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
    /// Maximum source samples to play (from decay param). usize::MAX = full sample.
    max_samples: usize,
    /// Output samples rendered so far (for decay cutoff tracking).
    samples_rendered: usize,
    /// Effective pan value, resolved at trigger time (pad pan or p-lock override).
    pan: f32,
    /// Resolved reverb send (0..1). Step plock overrides pad default at trigger time.
    fx_rvb_send: f32,
    /// Resolved delay send (0..1). Step plock overrides pad default at trigger time.
    fx_dly_send: f32,
    /// Resolved filter routing. Step plock overrides pad default at trigger time.
    fx_to_filter: bool,
}

impl Voice {
    fn new() -> Self {
        Self {
            pad_index: None,
            sample: None,
            position: 0.0,
            rate: 1.0,
            velocity: 0.0,
            age: 0,
            fade_remaining: 0,
            fade_length: 0,
            start_offset: 0,
            max_samples: usize::MAX,
            samples_rendered: 0,
            pan: 0.0,
            fx_rvb_send: 0.0,
            fx_dly_send: 0.0,
            fx_to_filter: false,
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

        // Need at least one sample to read (pos_floor must be a valid index)
        let pos_floor = self.position as usize;
        if pos_floor >= data.len() {
            self.pad_index = None;
            return None;
        }

        // Decay cutoff: start fade-out when approaching max_samples
        self.samples_rendered += 1;
        if self.max_samples != usize::MAX
            && self.samples_rendered >= self.max_samples
            && self.fade_remaining == 0
        {
            // Trigger a short fade-out to avoid clicks
            let fade_len = self.fade_length.min(self.max_samples / 10).max(1);
            self.fade_remaining = fade_len;
            self.fade_length = fade_len;
        }

        // Linear interpolation between adjacent samples
        let frac = (self.position - pos_floor as f64) as f32;
        let s0 = data[pos_floor];
        let s1 = if pos_floor + 1 < data.len() {
            data[pos_floor + 1]
        } else {
            0.0
        };
        let mut s = (s0 + (s1 - s0) * frac) * self.velocity;
        self.position += self.rate;

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

/// Everything needed to start one voice.
///
/// Replaces a 12-argument positional `trigger()` whose call sites read
/// `None, None, None, None, None, 0.0, 0.0, false` — five same-typed
/// `Option<f32>` p-locks in a row is an argument-order bug waiting to happen.
///
/// The `Option` fields are step p-locks: `None` means "inherit", and the
/// `lane_*` fields supply what to inherit for the three FX routings. Pan and
/// pitch inherit from the pad instead.
#[derive(Clone, Copy)]
pub struct Trigger {
    pub pad_index: usize,
    pub velocity: f32,
    /// Samples to wait before this voice starts, for sample-accurate
    /// sequencer placement within a buffer.
    pub start_offset: usize,
    pub pan: Option<f32>,
    pub pitch: Option<f32>,
    pub fx_rvb: Option<f32>,
    pub fx_dly: Option<f32>,
    pub fx_filter: Option<bool>,
    pub lane_rvb: f32,
    pub lane_dly: f32,
    pub lane_filter: bool,
}

impl Trigger {
    /// A trigger with no p-locks and no lane sends — everything inherits from
    /// the pad. This is what a MIDI note or a pad click produces.
    pub fn new(pad_index: usize, velocity: f32) -> Self {
        Self {
            pad_index,
            velocity,
            start_offset: 0,
            pan: None,
            pitch: None,
            fx_rvb: None,
            fx_dly: None,
            fx_filter: None,
            lane_rvb: 0.0,
            lane_dly: 0.0,
            lane_filter: false,
        }
    }

    /// Attach the active lane's FX routing defaults.
    pub fn with_lane_fx(mut self, rvb: f32, dly: f32, filter: bool) -> Self {
        self.lane_rvb = rvb;
        self.lane_dly = dly;
        self.lane_filter = filter;
        self
    }
}

/// The four parallel buses voices render into, borrowed for one buffer.
///
/// - `dry_bypass_*` — direct, unfiltered. Voices with `fx_to_filter == false`.
/// - `dry_filter_*` — direct, routed through the master DJ filter insert.
///   Voices with `fx_to_filter == true`.
/// - `send_rvb_*` — reverb send: each voice's output scaled by its resolved
///   reverb send level and summed.
/// - `send_dly_*` — delay send, same idea.
///
/// Grouping these replaces an eight-`&mut [f32]` positional argument list
/// where every parameter had the same type.
pub struct RenderBuses<'a> {
    pub dry_bypass_l: &'a mut [f32],
    pub dry_bypass_r: &'a mut [f32],
    pub dry_filter_l: &'a mut [f32],
    pub dry_filter_r: &'a mut [f32],
    pub send_rvb_l: &'a mut [f32],
    pub send_rvb_r: &'a mut [f32],
    pub send_dly_l: &'a mut [f32],
    pub send_dly_r: &'a mut [f32],
}

impl RenderBuses<'_> {
    /// Zero every bus. Called once per buffer before rendering.
    pub fn clear(&mut self) {
        self.dry_bypass_l.fill(0.0);
        self.dry_bypass_r.fill(0.0);
        self.dry_filter_l.fill(0.0);
        self.dry_filter_r.fill(0.0);
        self.send_rvb_l.fill(0.0);
        self.send_rvb_r.fill(0.0);
        self.send_dly_l.fill(0.0);
        self.send_dly_r.fill(0.0);
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
    pub fn trigger(&mut self, t: &Trigger, kit: &DrumKit) {
        let pad_index = t.pad_index;
        // Callers are expected to bounds-check, but this indexes `kit.pads` on
        // the audio thread, so it verifies rather than trusts.
        let Some(pad) = kit.pads.get(pad_index) else {
            return;
        };

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
        let pitch = t.pitch.unwrap_or(pad.pitch);
        let rate = 2.0_f64.powf(pitch as f64 / 12.0);
        let voice = &mut self.voices[slot];
        voice.pad_index = Some(pad_index);
        permit_alloc(|| {
            voice.sample = Some(Arc::clone(sample));
        });

        // Start/end trim: compute playback region in source samples
        let sample_len = sample.len();
        let start_frac = pad.start.clamp(0.0, 1.0) as f64;
        let end_frac = pad.end.clamp(0.0, 1.0).max(pad.start + 0.001) as f64;
        let start_sample = (start_frac * sample_len as f64) as usize;
        let region_samples = ((end_frac - start_frac) * sample_len as f64).max(1.0);

        voice.position = start_sample as f64;
        voice.rate = rate;
        voice.velocity = t.velocity * pad.volume;
        voice.age = self.trigger_counter;
        voice.fade_remaining = 0;
        voice.fade_length = self.fade_samples;
        voice.start_offset = t.start_offset;
        voice.samples_rendered = 0;
        voice.pan = t.pan.unwrap_or(pad.pan);
        voice.fx_rvb_send = t.fx_rvb.unwrap_or(t.lane_rvb).clamp(0.0, 1.0);
        voice.fx_dly_send = t.fx_dly.unwrap_or(t.lane_dly).clamp(0.0, 1.0);
        voice.fx_to_filter = t.fx_filter.unwrap_or(t.lane_filter);
        let full_region = pad.start <= 0.001 && pad.end >= 0.999;
        voice.max_samples = if pad.decay >= 1.0 && full_region {
            // Full sample, full decay: natural end-of-data handles stop
            usize::MAX
        } else if pad.decay >= 1.0 {
            // Trimmed region, full decay: stop at end point
            (region_samples / rate).max(1.0) as usize
        } else {
            // Decay shortens the active region
            ((region_samples * pad.decay as f64) / rate).max(1.0) as usize
        };
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

    /// Mix voices into the four parallel buses of [`RenderBuses`].
    ///
    /// Every voice carries its own resolved sends (pad default, optionally
    /// overridden by a step p-lock), so per-hit FX routing works automatically.
    ///
    /// Zero-alloc: the caller pre-allocates all eight buffers.
    pub fn process_sends(&mut self, buses: &mut RenderBuses<'_>) {
        let num_samples = buses.dry_bypass_l.len();
        for voice in self.voices.iter_mut() {
            if !voice.is_active() {
                continue;
            }
            let pan = voice.pan;
            let rvb_g = voice.fx_rvb_send;
            let dly_g = voice.fx_dly_send;
            let to_filter = voice.fx_to_filter;

            for i in 0..num_samples {
                if i < voice.start_offset {
                    continue;
                }
                match voice.next_sample(pan) {
                    Some((vl, vr)) => {
                        if to_filter {
                            buses.dry_filter_l[i] += vl;
                            buses.dry_filter_r[i] += vr;
                        } else {
                            buses.dry_bypass_l[i] += vl;
                            buses.dry_bypass_r[i] += vr;
                        }
                        if rvb_g > 0.0 {
                            buses.send_rvb_l[i] += vl * rvb_g;
                            buses.send_rvb_r[i] += vr * rvb_g;
                        }
                        if dly_g > 0.0 {
                            buses.send_dly_l[i] += vl * dly_g;
                            buses.send_dly_r[i] += vr * dly_g;
                        }
                    }
                    None => break,
                }
            }

            voice.start_offset = 0;
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

    /// Constant-power centre pan: cos(PI/4) ≈ 0.7071.
    const CENTRE_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

    /// Four render buses matching `VoicePool::process_sends`.
    struct Buses {
        dry_bypass_l: Vec<f32>,
        dry_bypass_r: Vec<f32>,
        dry_filter_l: Vec<f32>,
        dry_filter_r: Vec<f32>,
        send_rvb_l: Vec<f32>,
        send_rvb_r: Vec<f32>,
        send_dly_l: Vec<f32>,
        send_dly_r: Vec<f32>,
    }

    impl Buses {
        fn new(n: usize) -> Self {
            Self {
                dry_bypass_l: vec![0.0; n],
                dry_bypass_r: vec![0.0; n],
                dry_filter_l: vec![0.0; n],
                dry_filter_r: vec![0.0; n],
                send_rvb_l: vec![0.0; n],
                send_rvb_r: vec![0.0; n],
                send_dly_l: vec![0.0; n],
                send_dly_r: vec![0.0; n],
            }
        }

        /// Render one buffer through the shipping path.
        fn render(&mut self, pool: &mut VoicePool) {
            pool.process_sends(&mut RenderBuses {
                dry_bypass_l: &mut self.dry_bypass_l,
                dry_bypass_r: &mut self.dry_bypass_r,
                dry_filter_l: &mut self.dry_filter_l,
                dry_filter_r: &mut self.dry_filter_r,
                send_rvb_l: &mut self.send_rvb_l,
                send_rvb_r: &mut self.send_rvb_r,
                send_dly_l: &mut self.send_dly_l,
                send_dly_r: &mut self.send_dly_r,
            });
        }
    }

    /// A kit whose pad 0 holds `len` samples of 1.0 — easy to verify.
    fn test_kit_len(len: usize) -> DrumKit {
        let mut kit = DrumKit::new();
        kit.pads[0].sample = Some(Arc::new(vec![1.0; len]));
        kit.pads[0].volume = 1.0;
        kit.pads[0].pan = 0.0;
        kit.pads[0].category = SampleCategory::Kick;
        kit
    }

    fn test_kit() -> DrumKit {
        test_kit_len(8)
    }

    // ── Basic playback ───────────────────────────────────────────────────

    #[test]
    fn trigger_with_zero_offset_plays_from_start() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0), &kit);

        let mut b = Buses::new(8);
        b.render(&mut pool);

        assert!(
            (b.dry_bypass_l[0] - CENTRE_GAIN).abs() < 0.001,
            "first sample should be non-zero"
        );
        assert!(
            (b.dry_bypass_l[7] - CENTRE_GAIN).abs() < 0.001,
            "last sample should be non-zero"
        );
    }

    #[test]
    fn trigger_with_offset_delays_playback() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        let t = Trigger {
            start_offset: 4,
            ..Trigger::new(0, 1.0)
        };
        pool.trigger(&t, &kit);

        let mut b = Buses::new(12);
        b.render(&mut pool);

        for i in 0..4 {
            assert_eq!(
                b.dry_bypass_l[i], 0.0,
                "sample {i} should be silent (before offset)"
            );
        }
        assert!(
            (b.dry_bypass_l[4] - CENTRE_GAIN).abs() < 0.001,
            "sample 4 should have audio"
        );
    }

    #[test]
    fn start_offset_resets_after_first_buffer() {
        let kit = test_kit_len(100);
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(
            &Trigger {
                start_offset: 4,
                ..Trigger::new(0, 1.0)
            },
            &kit,
        );

        let mut first = Buses::new(8);
        first.render(&mut pool);
        assert_eq!(
            first.dry_bypass_l[0], 0.0,
            "first buffer: sample 0 should be silent"
        );

        let mut second = Buses::new(8);
        second.render(&mut pool);
        assert!(
            (second.dry_bypass_l[0] - CENTRE_GAIN).abs() < 0.001,
            "second buffer: sample 0 should have audio (offset reset)"
        );
    }

    #[test]
    fn velocity_scales_output_amplitude() {
        let kit = test_kit();

        let render_at = |velocity: f32| {
            let mut pool = VoicePool::new(44100.0);
            pool.trigger(&Trigger::new(0, velocity), &kit);
            let mut b = Buses::new(4);
            b.render(&mut pool);
            b.dry_bypass_l[0]
        };

        assert!(
            (render_at(1.0) - CENTRE_GAIN).abs() < 0.001,
            "full velocity"
        );
        assert!(
            (render_at(0.5) - CENTRE_GAIN * 0.5).abs() < 0.001,
            "half velocity"
        );
        assert_eq!(render_at(0.0), 0.0, "zero velocity should be silent");
    }

    #[test]
    fn start_point_offsets_playback_into_sample() {
        let mut kit = DrumKit::new();
        kit.pads[0].sample = Some(Arc::new(vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25]));
        kit.pads[0].volume = 1.0;
        kit.pads[0].pan = 0.0;
        kit.pads[0].start = 0.5; // halfway = source index 4

        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0), &kit);

        let mut b = Buses::new(4);
        b.render(&mut pool);

        assert!(
            (b.dry_bypass_l[0] - CENTRE_GAIN).abs() < 0.01,
            "start=0.5 should play from mid-sample, got {}",
            b.dry_bypass_l[0]
        );
    }

    #[test]
    fn end_point_stops_playback_early() {
        let mut kit = test_kit_len(100);
        kit.pads[0].end = 0.1; // 10 source samples

        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0), &kit);

        let mut b = Buses::new(20);
        b.render(&mut pool);

        assert!(
            (b.dry_bypass_l[0] - CENTRE_GAIN).abs() < 0.01,
            "should have audio at start"
        );
        assert!(
            b.dry_bypass_l[15].abs() < 0.01,
            "should be silent past end point, got {}",
            b.dry_bypass_l[15]
        );
    }

    #[test]
    fn trigger_on_an_empty_pad_is_a_noop() {
        let kit = DrumKit::new(); // no samples loaded
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0), &kit);
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn trigger_with_out_of_range_pad_index_does_not_panic() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(999, 1.0), &kit);
        assert_eq!(pool.active_count(), 0);
    }

    // ── FX bus routing (the four-bus path — previously untested) ──────────

    #[test]
    fn voices_default_to_the_unfiltered_dry_bus() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0), &kit);

        let mut b = Buses::new(8);
        b.render(&mut pool);

        assert!(
            b.dry_bypass_l[0].abs() > 0.001,
            "audio should land on the bypass bus"
        );
        assert_eq!(b.dry_filter_l[0], 0.0, "filter bus should be untouched");
        assert_eq!(b.send_rvb_l[0], 0.0, "reverb send should be silent");
        assert_eq!(b.send_dly_l[0], 0.0, "delay send should be silent");
    }

    #[test]
    fn lane_filter_flag_routes_to_the_filter_bus_instead() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0).with_lane_fx(0.0, 0.0, true), &kit);

        let mut b = Buses::new(8);
        b.render(&mut pool);

        assert!(
            b.dry_filter_l[0].abs() > 0.001,
            "audio should land on the filter bus"
        );
        assert_eq!(b.dry_bypass_l[0], 0.0, "bypass bus should be untouched");
    }

    #[test]
    fn lane_sends_scale_the_reverb_and_delay_buses() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0).with_lane_fx(0.5, 0.25, false), &kit);

        let mut b = Buses::new(8);
        b.render(&mut pool);

        let dry = b.dry_bypass_l[0];
        assert!(
            (b.send_rvb_l[0] - dry * 0.5).abs() < 0.001,
            "reverb send should be 50% of dry"
        );
        assert!(
            (b.send_dly_l[0] - dry * 0.25).abs() < 0.001,
            "delay send should be 25% of dry"
        );
        assert!(
            (dry - CENTRE_GAIN).abs() < 0.001,
            "sends are taps, so the dry signal is unchanged"
        );
    }

    #[test]
    fn step_plocks_override_the_lane_fx_defaults() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        // Lane says "no reverb, no filter"; the step p-lock says otherwise.
        let t = Trigger {
            fx_rvb: Some(1.0),
            fx_filter: Some(true),
            ..Trigger::new(0, 1.0).with_lane_fx(0.0, 0.0, false)
        };
        pool.trigger(&t, &kit);

        let mut b = Buses::new(8);
        b.render(&mut pool);

        assert!(
            b.dry_filter_l[0].abs() > 0.001,
            "p-lock should route to the filter bus"
        );
        assert!(
            (b.send_rvb_l[0] - b.dry_filter_l[0]).abs() < 0.001,
            "p-locked reverb send of 1.0 should equal the dry level"
        );
    }

    #[test]
    fn out_of_range_fx_sends_are_clamped() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        let t = Trigger {
            fx_rvb: Some(9.0),
            fx_dly: Some(-3.0),
            ..Trigger::new(0, 1.0)
        };
        pool.trigger(&t, &kit);

        let mut b = Buses::new(8);
        b.render(&mut pool);

        let dry = b.dry_bypass_l[0];
        assert!(
            (b.send_rvb_l[0] - dry).abs() < 0.001,
            "send above 1.0 should clamp to 1.0"
        );
        assert_eq!(b.send_dly_l[0], 0.0, "negative send should clamp to 0.0");
    }

    #[test]
    fn pan_plock_overrides_the_pad_pan() {
        let kit = test_kit();
        let mut pool = VoicePool::new(44100.0);
        // Hard left.
        pool.trigger(
            &Trigger {
                pan: Some(-1.0),
                ..Trigger::new(0, 1.0)
            },
            &kit,
        );

        let mut b = Buses::new(8);
        b.render(&mut pool);

        assert!(
            (b.dry_bypass_l[0] - 1.0).abs() < 0.001,
            "hard left should put full level on L"
        );
        assert!(
            b.dry_bypass_r[0].abs() < 0.001,
            "hard left should leave R silent"
        );
    }

    // ── Voice pool management ────────────────────────────────────────────

    #[test]
    fn retrigger_fades_the_previous_voice_instead_of_cutting_it() {
        let kit = test_kit_len(44100);
        let mut pool = VoicePool::new(44100.0);
        pool.trigger(&Trigger::new(0, 1.0), &kit);
        pool.trigger(&Trigger::new(0, 1.0), &kit);
        assert_eq!(
            pool.active_count(),
            2,
            "the fading voice stays active alongside the new one"
        );
    }

    #[test]
    fn pool_never_exceeds_max_voices() {
        let mut kit = DrumKit::new();
        for pad in &mut kit.pads {
            pad.sample = Some(Arc::new(vec![1.0; 44100]));
            pad.volume = 1.0;
        }
        let mut pool = VoicePool::new(44100.0);
        for i in 0..(MAX_VOICES * 3) {
            pool.trigger(&Trigger::new(i % NUM_PADS_FOR_TEST, 1.0), &kit);
        }
        assert!(
            pool.active_count() <= MAX_VOICES,
            "voice count must stay bounded"
        );
    }

    const NUM_PADS_FOR_TEST: usize = 8;
}
