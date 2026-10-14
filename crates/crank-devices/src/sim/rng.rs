//! Seeded generator ported from `shared/src/rng.ts`: mulberry32 uniforms and a
//! Box–Muller normal with a cached spare. The draw order is part of the parity
//! contract with `simulator.json`; do not reorder or add draws.
//!
//! `libm` supplies `log`, `sin`, `cos` and `sqrt` so streams do not depend on the
//! platform C library.

/// Deterministic per-seed generator.
#[derive(Clone, Debug)]
pub struct Rng {
    s: u32,
    spare: Option<f64>,
}

impl Rng {
    /// Seed `0` maps to the reference constant `0x9e3779b9`, as in the TS port.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self {
            s: if seed == 0 { 0x9e37_79b9 } else { seed },
            spare: None,
        }
    }
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        self.s = self.s.wrapping_add(0x6d2b_79f5);
        let mut t = self.s;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 0x3d));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
    /// Uniform in `[lo, hi)`.
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    /// Uniform integer in `lo..=hi`.
    pub fn int(&mut self, lo: i32, hi: i32) -> i32 {
        lo + i32::from(super::core::saturate_u16(
            (self.unit() * f64::from(hi - lo + 1)).floor(),
        ))
    }
    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    /// Normal with the given mean and standard deviation.
    pub fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        if let Some(z) = self.spare.take() {
            return mean + sd * z;
        }
        let mut u = 0.0;
        while u <= 1e-12 {
            u = self.unit();
        }
        let v = self.unit();
        let r = libm::sqrt(-2.0 * libm::log(u));
        let a = 2.0 * std::f64::consts::PI * v;
        self.spare = Some(r * libm::sin(a));
        mean + sd * r * libm::cos(a)
    }
}
