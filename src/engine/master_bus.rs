use nih_plug::util;

const RATIO: f32 = 4.0;
const LIMITER_CEILING_DB: f32 = -0.3;

const RMS_WINDOW_SECS: f32 = 0.005;
const COMP_ATTACK_MS: f32 = 10.0;
const COMP_RELEASE_MS: f32 = 100.0;
const LIM_ATTACK_MS: f32 = 0.1;
const LIM_RELEASE_MS: f32 = 50.0;

const MAX_RMS_BUF: usize = 8192;

/// Master bus chain: RMS compressor → tanh saturator → brickwall limiter.
/// All state pre-allocated — zero heap allocations on the audio thread.
pub struct MasterBus {
    rms_buf: Box<[f32; MAX_RMS_BUF]>,
    rms_write: usize,
    rms_sum: f32,
    rms_buf_len: usize,
    env_db: f32,

    lim_env: f32,
    limiter_ceiling_lin: f32,

    comp_attack_coeff: f32,
    comp_release_coeff: f32,
    lim_attack_coeff: f32,
    lim_release_coeff: f32,
}

impl MasterBus {
    pub fn new() -> Self {
        Self {
            rms_buf: Box::new([0.0f32; MAX_RMS_BUF]),
            rms_write: 0,
            rms_sum: 0.0,
            rms_buf_len: 1,
            env_db: -60.0,
            lim_env: 0.0,
            limiter_ceiling_lin: util::db_to_gain(LIMITER_CEILING_DB),
            comp_attack_coeff: 0.0,
            comp_release_coeff: 0.0,
            lim_attack_coeff: 0.0,
            lim_release_coeff: 0.0,
        }
    }

    /// Recompute coefficients for a new sample rate. Call from `initialize()`.
    pub fn prepare(&mut self, sample_rate: f32) {
        fn coeff(ms: f32, sr: f32) -> f32 {
            (-1.0 / (ms * 0.001 * sr)).exp()
        }

        self.comp_attack_coeff = coeff(COMP_ATTACK_MS, sample_rate);
        self.comp_release_coeff = coeff(COMP_RELEASE_MS, sample_rate);
        self.lim_attack_coeff = coeff(LIM_ATTACK_MS, sample_rate);
        self.lim_release_coeff = coeff(LIM_RELEASE_MS, sample_rate);

        self.rms_buf_len = ((RMS_WINDOW_SECS * sample_rate).ceil() as usize).clamp(1, MAX_RMS_BUF);
        self.rms_write = 0;
        self.rms_sum = 0.0;
        self.rms_buf.fill(0.0);
        self.env_db = -60.0;
        self.lim_env = 0.0;
    }

    /// Process a single stereo sample through the master bus chain.
    #[inline]
    pub fn process_sample(
        &mut self,
        l: f32,
        r: f32,
        threshold_db: f32,
        drive: f32,
        limiter_on: bool,
    ) -> (f32, f32) {
        // ── Stage 1: RMS compressor ──

        // Mono sum for level detection
        let mid = (l + r) * 0.5;
        let sq = mid * mid;

        // Rolling RMS window
        let old = self.rms_buf[self.rms_write];
        self.rms_sum = (self.rms_sum - old + sq).max(0.0);
        self.rms_buf[self.rms_write] = sq;
        self.rms_write += 1;
        if self.rms_write >= self.rms_buf_len {
            self.rms_write = 0;
        }

        let rms_lin = (self.rms_sum / self.rms_buf_len as f32).sqrt();
        let rms_db = util::gain_to_db(rms_lin.max(1e-9));

        // One-pole envelope follower in dB domain
        let coeff = if rms_db > self.env_db {
            self.comp_attack_coeff
        } else {
            self.comp_release_coeff
        };
        self.env_db = coeff * self.env_db + (1.0 - coeff) * rms_db;

        // Gain computation
        let overshoot_db = self.env_db - threshold_db;
        let gain_reduction_db = if overshoot_db > 0.0 {
            -overshoot_db * (1.0 - 1.0 / RATIO)
        } else {
            0.0
        };

        // Auto makeup: compensate by half the max theoretical GR
        let makeup_db = -threshold_db * (1.0 - 1.0 / RATIO) * 0.5;
        let comp_gain = util::db_to_gain(gain_reduction_db + makeup_db);

        let mut l = l * comp_gain;
        let mut r = r * comp_gain;

        // ── Stage 2: Tanh saturator ──

        if drive > 0.001 {
            let pre_gain = 1.0 + drive * 5.0;
            let post_gain = 1.0 / (1.0 + drive * 0.7);
            l = (l * pre_gain).tanh() * post_gain;
            r = (r * pre_gain).tanh() * post_gain;
        }

        // ── Stage 3: Brickwall limiter ──

        if limiter_on {
            let peak = l.abs().max(r.abs());
            self.lim_env = if peak > self.lim_env {
                self.lim_attack_coeff * self.lim_env + (1.0 - self.lim_attack_coeff) * peak
            } else {
                self.lim_release_coeff * self.lim_env + (1.0 - self.lim_release_coeff) * peak
            };
            if self.lim_env > self.limiter_ceiling_lin {
                let lim_gain = self.limiter_ceiling_lin / self.lim_env;
                l *= lim_gain;
                r *= lim_gain;
            }
        }

        (l, r)
    }
}
