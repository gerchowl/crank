//! The plan line on the tick grid (ADR-0002 §7.10.2, "The line on the tick grid").
//!
//! Tick `k` starts at `k·Δt`; a block's line is sampled at the tick's start, and the line is a
//! list of canonical [`Piece`]s built from tick 0 every time, so it never depends on the tick
//! that asks (round 10).
use super::{DT, RAW_JOIN, SLOPE_TOL, VALUE_TOL, tick_f};

/// One block of the plan as the easer sees it: `ticks` long, its line from `from_w` to `to_w`
/// (equal for a steady block), before the multiplier and the clamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    /// Length in ticks (block durations are on the 0.25 s grid).
    pub ticks: u32,
    /// The value on its first tick, W.
    pub from_w: f64,
    /// The value it would reach at its end, W (the next block's first tick carries it if joined).
    pub to_w: f64,
}

impl Block {
    /// The block's slope, W/s.
    #[must_use]
    pub fn slope(&self) -> f64 {
        (self.to_w - self.from_w) / (f64::from(self.ticks) * DT)
    }
}

/// A piece of the line: value `r0` on tick `k0`, slope `s` W/s, `n` ticks (multiplier and clamp
/// applied). `joined`: the piece starts where the line already is (a kink, or where the clamp at
/// 0 W or `M` starts or ends), so its start is a vertex, not a discontinuity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    /// First tick.
    pub k0: i64,
    /// Value on `k0`, W.
    pub r0: f64,
    /// Slope, W/s.
    pub s: f64,
    /// Length in ticks.
    pub n: i64,
    /// Its start is a vertex (the line continuous there), not a discontinuity.
    pub joined: bool,
}

impl Piece {
    /// The piece's line at tick `k` (extended either way).
    #[must_use]
    pub fn at(&self, k: i64) -> f64 {
        self.r0 + self.s * tick_f(k - self.k0) * DT
    }
    /// The tick after its last.
    #[must_use]
    pub fn end(&self) -> i64 {
        self.k0 + self.n
    }
}

/// Two slopes are one within [`SLOPE_TOL`] relative to `max(1, |s1|, |s2|)`.
pub(crate) fn collinear(s1: f64, s2: f64) -> bool {
    (s1 - s2).abs() <= SLOPE_TOL * 1.0_f64.max(s1.abs()).max(s2.abs())
}

/// Where a value lies against the clamp: below or at 0 W, inside, at or above `M`.
fn region(v: f64, m: f64) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= m {
        2
    } else {
        1
    }
}

/// The clamped plan line from tick `k` on, as canonical pieces (ADR-0002 §7.10.2 steps 1–7):
/// 1. each block's line (multiplier `mult` applied), joined if it starts within [`RAW_JOIN`] of
///    where the previous block ended;
/// 2. joined lines whose slopes agree within [`SLOPE_TOL`] merged into one, its slope recomputed
///    from the merged endpoints (a split ramp is the single block, bitwise at multiplier 1);
/// 3. a merged slope within [`SLOPE_TOL`] of 0 is exactly 0;
/// 4. each line split where the clamp to `[0, m]` starts or ends; a clamped piece is flat;
/// 5. after the clamp a block's first piece is joined only if the clamped line continues there
///    (within [`VALUE_TOL`]); inside one line, where the clamp starts or ends, it is joined;
/// 6. adjacent pieces merged after the clamp only if joined, contiguous, at the same value and
///    collinear ([`merge_collinear`]);
/// 7. only then the pieces that end at or before tick `k` dropped.
#[must_use]
pub fn segments(blocks: &[Block], k: i64, mult: f64, m: f64) -> Vec<Piece> {
    // 1. the raw lines, each with its end value
    let mut raw: Vec<(Piece, f64)> = Vec::with_capacity(blocks.len());
    let (mut k0, mut prev_end) = (0_i64, None::<f64>);
    for b in blocks {
        if b.ticks > 0 {
            let joined = prev_end.is_some_and(|e| (b.from_w * mult - e).abs() < RAW_JOIN);
            let piece = Piece {
                k0,
                r0: b.from_w * mult,
                s: b.slope() * mult,
                n: i64::from(b.ticks),
                joined,
            };
            raw.push((piece, b.to_w * mult));
            prev_end = Some(b.to_w * mult);
        }
        k0 += i64::from(b.ticks);
    }
    // 2. collinear joined lines merged, the slope from the merged endpoints
    let mut lines: Vec<Piece> = Vec::with_capacity(raw.len());
    for (seg, end) in raw {
        match lines.last_mut() {
            Some(q) if seg.joined && collinear(seg.s, q.s) => {
                let n = q.n + seg.n;
                *q = Piece {
                    s: (end - q.r0) / (tick_f(n) * DT),
                    n,
                    ..*q
                };
            }
            _ => lines.push(seg),
        }
    }
    // 3. float noise in a flat block's endpoints is not a slope (round 7)
    for q in &mut lines {
        if collinear(q.s, 0.0) {
            q.s = 0.0;
        }
    }
    // 4–5. split at the clamp; joined after the clamp only where the clamped line continues
    let mut out: Vec<Piece> = Vec::new();
    let mut last: Option<Piece> = None;
    for line in &lines {
        let mut kk = line.k0;
        while kk < line.end() {
            let v = line.at(kk);
            let state = region(v, m);
            let mut end = kk + 1;
            while end < line.end() && region(line.at(end), m) == state {
                end += 1;
            }
            let (r0, s) = match state {
                1 => (v, line.s),
                0 => (0.0, 0.0),
                _ => (m, 0.0),
            };
            let joined = if kk == line.k0 {
                line.joined && last.is_some_and(|p| (p.at(kk) - r0).abs() <= VALUE_TOL)
            } else {
                true
            };
            let piece = Piece {
                k0: kk,
                r0,
                s,
                n: end - kk,
                joined,
            };
            out.push(piece);
            last = Some(piece);
            kk = end;
        }
    }
    // 6–7. merge first, then drop: the line is the same whichever tick asks
    let mut merged = merge_collinear(&out);
    merged.retain(|p| p.end() > k);
    merged
}

/// After the clamp: a piece is merged into the previous one only if it starts on the tick the
/// previous one ends, at its value (within [`VALUE_TOL`]) and with its slope (within
/// [`SLOPE_TOL`]): two blocks both held at `M`, or both at 0 W. A flat piece at `M` never swallows
/// a lower flat block (round 6).
#[must_use]
pub fn merge_collinear(pieces: &[Piece]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::with_capacity(pieces.len());
    for p in pieces {
        match out.last_mut() {
            Some(q)
                if p.k0 == q.end()
                    && collinear(p.s, q.s)
                    && (q.at(p.k0) - p.r0).abs() <= VALUE_TOL =>
            {
                q.n += p.n;
            }
            _ => out.push(*p),
        }
    }
    out
}

/// The plan line at tick `k` as known now, clamped to `[0, m]`: past the plan's end, the last
/// piece's end value; before the first piece, the first piece extended back. With no piece at all
/// (never asked: the easer reads the line only while a plan is in force) it is `m`.
#[must_use]
pub fn line_at(segs: &[Piece], k: i64, m: f64) -> f64 {
    if let Some(sg) = segs.iter().find(|sg| sg.k0 <= k && k < sg.end()) {
        return super::clamp(sg.at(k), m);
    }
    match (segs.first(), segs.last()) {
        (Some(_), Some(last)) if k >= last.end() => super::clamp(last.at(last.end()), m),
        (Some(first), _) => super::clamp(first.at(k), m),
        _ => m,
    }
}

/// The target before the clamp is a number on tick `k`: the multiplier, and every block not yet
/// ended at `k`. A NaN or ±∞ there is a fault, not a target (ADR-0002 §7.10.2, round 14).
#[must_use]
pub fn plan_finite(blocks: &[Block], k: i64, mult: f64) -> bool {
    let mut kk = 0_i64;
    let mut finite = mult.is_finite();
    for b in blocks {
        if kk + i64::from(b.ticks) > k {
            finite = finite && b.from_w.is_finite() && b.to_w.is_finite();
        }
        kk += i64::from(b.ticks);
    }
    finite
}
