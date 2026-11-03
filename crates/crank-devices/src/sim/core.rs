//! Fixed-step rider + trainer model ported from the trainer, cadence, speed, HR and
//! RR sections of `shared/src/sim.ts`. Derived metrics, the metric bus and coach
//! hooks are not here (known limit; #203).
use super::rng::Rng;
use crank_domain::{
    wbal::wbal_step,
    workout::{Athlete, hr_steady},
};

/// Round to the nearest `u16`, saturating at both ends and mapping NaN to 0.
/// Binary search keeps float-to-integer conversion cast-free, as in `control.rs`.
#[must_use]
pub(crate) fn saturate_u16(x: f64) -> u16 {
    let target = x.round();
    let (mut lo, mut hi) = (0u16, u16::MAX);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f64::from(mid) < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Simulation step in seconds: trainer metrics at 4 Hz, HR every 4th tick.
pub const DT: f64 = 0.25;
const HR_FAST_W: f64 = 0.7;
const DELAY_TICKS: usize = 16;

/// Simulated rider physiology, the sim's own documented configuration.
///
/// `cp_w` and `w_prime_j` default to the reference `physioFor`: CP equals FTP and
/// W′ is 20 kJ. Steady HR at zero is `rest_hr + 12` and at FTP `round(0.9·max_hr)`
/// (via `crank_domain::workout::hr_steady`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimRider {
    /// Functional threshold power, watts.
    pub ftp_w: f64,
    /// Maximum heart rate, bpm.
    pub max_hr: f64,
    /// Resting heart rate, bpm.
    pub rest_hr: f64,
    /// Critical power, watts.
    pub cp_w: f64,
    /// Anaerobic work capacity, joules.
    pub w_prime_j: f64,
    /// Effort the rider holds when the trainer is not in ERG, watts.
    pub free_power_w: f64,
}
impl SimRider {
    /// Rider with reference CP/W′ and a free effort of 55 % FTP.
    #[must_use]
    pub fn new(ftp_w: f64, max_hr: f64, rest_hr: f64) -> Self {
        Self {
            ftp_w,
            max_hr,
            rest_hr,
            cp_w: ftp_w,
            w_prime_j: 20_000.0,
            free_power_w: 0.55 * ftp_w,
        }
    }
    fn athlete(&self) -> Athlete {
        Athlete {
            ftp: self.ftp_w,
            weight_kg: 0.0,
            max_hr: self.max_hr,
            rest_hr: self.rest_hr,
        }
    }
}
impl Default for SimRider {
    /// The reference default athlete: FTP 280 W, max HR 190, rest HR 50.
    fn default() -> Self {
        Self::new(280.0, 190.0, 50.0)
    }
}

/// Per-tick inputs from the trainer layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    /// Power the trainer asks of the rider, watts (ERG target or rider effort).
    pub target_w: f64,
    /// Road grade in percent (SIM only; zero otherwise).
    pub grade_pct: f64,
    /// Preferred cadence, rpm; `None` derives it from intensity.
    pub cadence_pref: Option<f64>,
    /// Force the rider's cadence target (stall, spiral).
    pub cadence_force: Option<f64>,
}

/// One simulated second of heart-rate data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HrReading {
    /// Beats per minute; `None` while the strap drops out.
    pub bpm: Option<u8>,
}

/// Result of one 4 Hz tick. `None` measurements mean the trainer link blipped.
#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    /// Time since ride start, seconds.
    pub t_s: f64,
    /// Measured power, watts.
    pub power_w: Option<f64>,
    /// Measured cadence in rpm, rounded to 0.1.
    pub cadence_rpm: Option<f64>,
    /// Speed, km/h.
    pub speed_kmh: Option<f64>,
    /// W′ balance in percent of capacity.
    pub wbal_pct: f64,
    /// Odometer, kilometres.
    pub distance_km: f64,
    /// Heart rate, on every fourth tick.
    pub hr: Option<HrReading>,
    /// Beats that occurred this tick: `(time_s, rr_ms)`; empty during dropouts.
    pub beats: Vec<(f64, u16)>,
}

/// Deterministic rider + trainer physics.
#[derive(Clone, Debug)]
pub struct SimCore {
    rider: SimRider,
    athlete: Athlete,
    mass_kg: f64,
    rng: Rng,
    tick: u64,
    blip: (f64, f64),
    day_offset: f64,
    erg_cmd: f64,
    ar: f64,
    spike_left: i32,
    spike_mag: f64,
    cad_ar: f64,
    cadence: f64,
    v_ms: f64,
    dist_km: f64,
    delay: [f64; DELAY_TICKS],
    delay_idx: usize,
    p_ema: f64,
    hr_f: f64,
    hr_s: f64,
    drift: f64,
    wbal: f64,
    w_expended: f64,
    hr_drop_left: i32,
    intensity_ema: f64,
    next_beat_in: f64,
    rr_ar: f64,
    rr_prev_u: f64,
}

impl SimCore {
    /// Start a ride. `mass_kg` is rider plus bicycle mass.
    #[must_use]
    pub fn new(rider: SimRider, mass_kg: f64, seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let blip_start = rng.uniform(600.0, 1100.0);
        let blip_end = blip_start + rng.uniform(3.0, 6.0);
        let day_offset = rng.uniform(-1.0, 4.0);
        let r = 8.0; // pre-ride HR slightly above rest
        Self {
            rider,
            athlete: rider.athlete(),
            mass_kg,
            rng,
            tick: 0,
            blip: (blip_start, blip_end),
            day_offset,
            erg_cmd: 0.0,
            ar: 0.0,
            spike_left: 0,
            spike_mag: 0.0,
            cad_ar: 0.0,
            cadence: 0.0,
            v_ms: 0.0,
            dist_km: 0.0,
            delay: [0.0; DELAY_TICKS],
            delay_idx: 0,
            p_ema: 0.0,
            hr_f: HR_FAST_W * r,
            hr_s: (1.0 - HR_FAST_W) * r,
            drift: 0.0,
            wbal: rider.w_prime_j,
            w_expended: 0.0,
            hr_drop_left: 0,
            intensity_ema: 0.0,
            next_beat_in: 0.8,
            rr_ar: 0.0,
            rr_prev_u: 0.0,
        }
    }
    /// Replace the rider + bicycle mass (from the trainer's rider configuration).
    pub fn set_mass_kg(&mut self, mass_kg: f64) {
        self.mass_kg = mass_kg;
    }
    /// Current cadence in rpm before measurement rounding.
    #[must_use]
    pub fn cadence(&self) -> f64 {
        self.cadence
    }
    /// Advance one `DT` tick.
    pub fn step(&mut self, drive: Drive) -> Tick {
        let rider = self.rider;
        self.tick += 1;
        let t = f64::from(u32::try_from(self.tick).unwrap_or(u32::MAX)) * DT;
        let trainer_ok = !(t >= self.blip.0 && t < self.blip.1);

        // trainer: first-order lag plus AR(1) noise and rare spikes
        self.erg_cmd += (drive.target_w - self.erg_cmd) * (1.0 - libm::exp(-DT / 1.2));
        self.ar = 0.9 * self.ar + self.rng.normal(0.0, 0.011);
        if self.spike_left > 0 {
            self.spike_left -= 1;
        } else if self.rng.chance(0.0025) {
            self.spike_left = self.rng.int(1, 6);
            self.spike_mag = if self.rng.chance(0.75) {
                self.rng.uniform(0.15, 0.4)
            } else {
                -self.rng.uniform(0.2, 0.4)
            };
        }
        let spike = if self.spike_left > 0 {
            self.spike_mag
        } else {
            0.0
        };
        let actual =
            (self.erg_cmd * (1.0 + self.ar + spike + self.rng.normal(0.0, 0.012))).max(0.0);
        let measured = trainer_ok.then_some(actual.round());

        // energy
        self.wbal =
            wbal_step(self.wbal, actual, DT, rider.cp_w, rider.w_prime_j).unwrap_or(self.wbal);
        if actual > rider.cp_w {
            self.w_expended += (actual - rider.cp_w) * DT;
        }
        let w_frac = (self.wbal / rider.w_prime_j).clamp(0.0, 1.0);

        // cadence
        let intensity = actual / rider.ftp_w;
        let mut pref = drive
            .cadence_pref
            .unwrap_or_else(|| 87.0 + 8.0 * ((intensity - 0.6) / 0.6).clamp(0.0, 1.0));
        pref -= 2.5 * (t / 3600.0) + 5.0 * (1.0 - w_frac);
        self.cad_ar = 0.97 * self.cad_ar + self.rng.normal(0.0, 0.35);
        let cad_target = drive.cadence_force.unwrap_or(if actual < 5.0 {
            0.0
        } else {
            pref + self.cad_ar
        });
        self.cadence += (cad_target - self.cadence) * (1.0 - libm::exp(-DT / 1.5));
        let cad_measured = trainer_ok.then_some((self.cadence * 10.0).round() / 10.0);

        let v = self.solve_speed(actual, drive.grade_pct);
        self.v_ms += (v - self.v_ms) * (1.0 - libm::exp(-DT / 3.0));
        self.dist_km += self.v_ms * DT / 1000.0;

        // heart: delayed power, two-component kinetics (fast 70 % + slow 30 %)
        let delayed = self.delay[self.delay_idx];
        self.delay[self.delay_idx] = actual;
        self.delay_idx = (self.delay_idx + 1) % DELAY_TICKS;
        self.p_ema += (delayed - self.p_ema) * (1.0 - libm::exp(-DT / 4.0));
        self.intensity_ema +=
            (self.p_ema / rider.cp_w - self.intensity_ema) * (1.0 - libm::exp(-DT / 60.0));
        self.drift += (DT / 60.0) * 0.12 * ((self.p_ema / rider.ftp_w - 0.4) / 0.6).clamp(0.0, 1.5);
        let fatigue =
            1.0 + 0.2 * (self.w_expended / rider.w_prime_j).min(2.5) + 0.3 * (1.0 - w_frac);
        let steady = hr_steady(self.p_ema, rider.ftp_w, &self.athlete);
        let ss = (rider.max_hr - 1.0)
            .min(steady + self.drift + self.day_offset + 4.0 * (1.0 - w_frac))
            - rider.rest_hr;
        let relax = |x: f64, target: f64, up: f64, down: f64| {
            x + (target - x) * (1.0 - libm::exp(-DT / if target > x { up } else { down }))
        };
        self.hr_f = relax(self.hr_f, HR_FAST_W * ss, 20.0, 32.0 * fatigue);
        self.hr_s = relax(self.hr_s, (1.0 - HR_FAST_W) * ss, 60.0, 100.0 * fatigue);
        let hr_true = rider.rest_hr + self.hr_f + self.hr_s;

        // RR intervals; beats keep drawing from the generator during dropouts
        let mut beats = Vec::new();
        self.next_beat_in -= DT;
        while self.next_beat_in <= 0.0 {
            let rr = self.next_rr(hr_true);
            if self.hr_drop_left <= 0 {
                beats.push(((t + self.next_beat_in).max(0.0), saturate_u16(rr)));
            }
            self.next_beat_in += rr / 1000.0;
        }

        // 1 Hz strap reading with occasional dropouts
        let hr = self.tick.is_multiple_of(4).then(|| {
            if self.hr_drop_left > 0 {
                self.hr_drop_left -= 1;
            } else if t > 60.0 && self.rng.chance(1.0 / 420.0) {
                self.hr_drop_left = self.rng.int(2, 6);
            }
            let bpm = if self.hr_drop_left <= 0 {
                let noisy = hr_true + self.rng.normal(0.0, 0.6);
                Some(u8::try_from(saturate_u16(noisy).min(255)).unwrap_or(u8::MAX))
            } else {
                None
            };
            HrReading { bpm }
        });

        Tick {
            t_s: t,
            power_w: measured,
            cadence_rpm: cad_measured,
            speed_kmh: trainer_ok.then_some(self.v_ms * 3.6),
            wbal_pct: self.wbal / rider.w_prime_j * 100.0,
            distance_km: self.dist_km,
            hr,
            beats,
        }
    }
    /// Newton solve of aero + rolling + grade resistance against 97 % drivetrain power.
    fn solve_speed(&self, actual: f64, grade_pct: f64) -> f64 {
        let pw = actual * 0.97;
        let g = grade_pct / 100.0;
        let mut v = self.v_ms.max(1.0);
        for _ in 0..3 {
            let f = 0.5 * 1.2 * 0.32 * v * v * v + (0.004 + g) * self.mass_kg * 9.81 * v - pw;
            let df = 1.5 * 1.2 * 0.32 * v * v + (0.004 + g) * self.mass_kg * 9.81;
            v = (v - f / df).max(0.0);
        }
        v
    }
    fn next_rr(&mut self, hr: f64) -> f64 {
        let mean_rr = 60_000.0 / hr;
        let sigma = (50.0 * libm::exp(-(hr - self.rider.rest_hr) / 35.0)).max(3.5);
        let i = self.intensity_ema;
        let lerp = |a: f64, b: f64, f: f64| a + (b - a) * f;
        let alpha = if i <= 0.5 {
            1.15
        } else if i <= 0.75 {
            lerp(1.15, 0.75, (i - 0.5) / 0.25)
        } else if i <= 1.0 {
            lerp(0.75, 0.4, (i - 0.75) / 0.25)
        } else if i <= 1.15 {
            lerp(0.4, 0.25, (i - 1.0) / 0.15)
        } else {
            0.25
        };
        let phi: f64 = 0.85;
        self.rr_ar = phi * self.rr_ar + libm::sqrt(1.0 - phi * phi) * self.rng.normal(0.0, 1.0);
        let white = self.rng.normal(0.0, 1.0);
        let u = self.rng.normal(0.0, 1.0);
        let anti = (u - self.rr_prev_u) / std::f64::consts::SQRT_2;
        self.rr_prev_u = u;
        let z = if alpha >= 0.5 {
            let wa = ((alpha - 0.5) / 0.75).clamp(0.0, 1.0);
            libm::sqrt(wa) * self.rr_ar + libm::sqrt(1.0 - wa) * white
        } else {
            let wb = ((0.5 - alpha) / 0.4).clamp(0.0, 1.0);
            libm::sqrt(1.0 - wb) * white + libm::sqrt(wb) * anti
        };
        mean_rr + sigma * z
    }
}
