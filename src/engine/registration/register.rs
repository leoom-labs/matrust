//! imregtform and imregister (pieces 7c3a, 7c3b): intensity-based 2-D
//! registration, written from the public MathWorks imregtform/imregister
//! reference pages and the public ITK documentation (Apache-2.0, algorithm
//! reference only) of the regular step gradient descent and one-plus-one
//! evolutionary optimizers and the Mattes mutual information metric. No
//! MathWorks code or examples.
//!
//! The estimated map U goes from fixed world coordinates to moving world
//! coordinates (ITK's direction); the result is its inverse T (moving to
//! fixed), so imwarp(moving, T, 'OutputView', Rfixed) aligns moving with
//! fixed. U(x) = A (x - c) + c + t about the centre c of Rfixed's world
//! box, so rotation, scale and translation do not fight each other:
//!
//! - translation: A = I; rigid: A = R(theta); similarity: A = s R(theta);
//!   affine: A = [a b; d e]. Parameters [theta], [s theta], [a b d e], then
//!   [tx ty], in world units and radians.
//! - metric (MeanSquares): mean of (M(U x) - F(x))^2 over the fixed pixels
//!   whose U x lands inside the moving image (bilinear M; as in ITK, the
//!   half pixel beyond the outer pixel centres counts and repeats the edge
//!   value, so a small move from a grid-aligned start keeps every sample);
//!   the gradient uses central-difference images of M, sampled bilinearly.
//! - metric (MattesMutualInformation, Mattes et al. as ITK describes it):
//!   minus the mutual information of the joint histogram of fixed and moving
//!   values over the overlapping samples. NumberOfHistogramBins bins per
//!   image with two padding bins at each end, so the level's smallest..
//!   largest value spans bins 2..bins-2; a fixed value adds 1 to its bin
//!   (zero-order kernel), a moving value M(U x) spreads over four bins with
//!   the cubic B-spline (Parzen window). The analytic gradient differentiates
//!   the B-spline (Thevenaz and Unser); the fixed marginal does not move.
//!   Samples: every finite fixed pixel (UseAllPixels) or
//!   NumberOfSpatialSamples distinct ones drawn per level with a fixed seed.
//! - scales: parameter j is normalised by c_j = RMS over the fixed pixels of
//!   the shift (fixed pixels of the level) a unit change of it causes
//!   (ITK's "physical shift" scales). RegularStepGradientDescent steps
//!   in these normalised units, so one step length suits rotation, scale,
//!   shear and translation; step lengths are pixels of the current level.
//! - pyramid: each level halves both images with the [1 3 3 1]/8 filter over
//!   pixel pairs (same world box; the last pixel of an odd size repeats the
//!   edge); levels run coarse to fine and the world-unit parameters carry
//!   over unchanged, so translations in pixels double from level to level.
//! - OnePlusOneEvolutionary (ITK's rules, Styner et al.): per iteration a
//!   child = parent + A z with z standard normal (fixed seed, so runs
//!   repeat); a lower cost makes the child the parent and stretches A along
//!   z by GrowthFactor, otherwise A shrinks along z by GrowthFactor^(-1/4).
//!   A starts as InitialRadius times the identity in the normalised units
//!   above (pixels of the level) at every level; the level stops when the
//!   Frobenius norm of A is at most Epsilon or after MaximumIterations.
//!
//! The app (7c4) calls [`estimate`] with a progress callback.
use super::super::image_basic::{self, Img, Kind as ImageKind, registration_config as rc};
use super::super::{Engine, Execution, Value};
use super::{Geo, Imref2d, Interp, Kind, Sampling, Tform, geo_of, imref_of, new_object, tform_of};
use crate::Result;
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

/// Fixed samples per block: one cancellation poll and one parallel work item.
const BLOCK: usize = 1 << 14;

/// Fixed samples from which a metric evaluation runs its blocks on the
/// worker pool. 2026-11-17, i7-12700KF under load (gate profile), affine,
/// 30 iterations: 512^2 257 ms on 1 worker, 178 ms on 8; 2048^2 4.96 s vs
/// 2.25 s; forcing the pool at 64^2 cost nothing measurable.
const PAR_SAMPLES: usize = 1 << 16;

/// Pixels a dimension keeps at the coarsest pyramid level, at least.
const MIN_LEVEL_PIXELS: usize = 4;

/// Most histogram bins per image of the Mattes metric (its joint histogram
/// has bins^2 entries).
const MAX_BINS: usize = 4096;

/// Probabilities at or below this are empty histogram entries (ITK's).
const TINY: f64 = 1e-16;

/// Seed of the spatial sample draws (plus the level number) and of the
/// evolutionary search.
const SEED: u64 = 0x6d61_7472_7573_7463;

/// SplitMix64 with Box-Muller normal variates: small, fixed-seed, so a
/// registration repeats exactly.
struct Random {
    state: u64,
    spare: Option<f64>,
}

impl Random {
    fn new(seed: u64) -> Self {
        Random {
            state: seed,
            spare: None,
        }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in [0, n).
    fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }
    /// Uniform in [0, 1).
    fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Standard normal (Box-Muller; both values of a pair are used).
    fn normal(&mut self) -> f64 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        let r = (-2. * libm::log(1. - self.uniform())).sqrt();
        let a = std::f64::consts::TAU * self.uniform();
        self.spare = Some(r * libm::sin(a));
        r * libm::cos(a)
    }
}

/// `wanted` distinct finite pixels of `data` (sorted indices), drawn with
/// a fixed seed for pyramid level `stage`; None when that is all of them.
fn pick(data: &[f64], wanted: usize, stage: usize) -> Option<Vec<usize>> {
    let finite = data.iter().filter(|v| v.is_finite()).count();
    if wanted >= finite {
        return None;
    }
    let mut random = Random::new(SEED.wrapping_add(stage as u64));
    // Floyd's algorithm: `wanted` distinct positions among the finite ones.
    let mut chosen = HashSet::with_capacity(wanted);
    for j in finite - wanted..finite {
        let t = random.below(j + 1);
        if !chosen.insert(t) {
            chosen.insert(j);
        }
    }
    let mut positions: Vec<usize> = chosen.into_iter().collect();
    positions.sort_unstable();
    if finite == data.len() {
        return Some(positions);
    }
    // Position k among the finite pixels -> pixel index.
    let finite_indices = data
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .map(|(i, _)| i);
    let mut next = positions.into_iter().peekable();
    let mut out = Vec::with_capacity(wanted);
    for (k, i) in finite_indices.enumerate() {
        match next.peek() {
            None => break,
            Some(&want) if want == k => {
                out.push(i);
                next.next();
            }
            _ => {}
        }
    }
    Some(out)
}

/// The transformation model of imregtform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum TransformType {
    Translation,
    Rigid,
    Similarity,
    Affine,
}

impl TransformType {
    const NAMES: [&'static str; 4] = ["translation", "rigid", "similarity", "affine"];
    const ALL: [Self; 4] = [
        Self::Translation,
        Self::Rigid,
        Self::Similarity,
        Self::Affine,
    ];
    /// A name or a unique prefix of one, any case.
    pub(in crate::engine) fn parse(text: &str) -> Option<Self> {
        let lower = text.to_ascii_lowercase();
        if lower.is_empty() {
            return None;
        }
        let found: Vec<_> = Self::NAMES
            .iter()
            .zip(Self::ALL)
            .filter(|(n, _)| n.starts_with(&lower))
            .collect();
        match found.as_slice() {
            [(_, t)] => Some(*t),
            _ => None,
        }
    }
    pub(in crate::engine) fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub(in crate::engine) fn kind(self) -> Kind {
        match self {
            Self::Translation => Kind::Transl,
            Self::Rigid => Kind::Rigid,
            Self::Similarity => Kind::Sim,
            Self::Affine => Kind::Affine,
        }
    }
}

/// U(x) = A (x - c) + c + t, fixed world to moving world.
#[derive(Clone, Copy, Debug)]
struct Model {
    ty: TransformType,
    c: [f64; 2],
}

/// Most parameters a model has.
const MAX_PARAMS: usize = 6;

impl Model {
    /// A and the offset b of U(x) = A x + b.
    fn map(&self, p: &[f64]) -> ([[f64; 2]; 2], [f64; 2]) {
        let a = match self.ty {
            TransformType::Translation => [[1., 0.], [0., 1.]],
            TransformType::Rigid => rotation(p[0]),
            TransformType::Similarity => {
                let r = rotation(p[1]);
                [
                    [p[0] * r[0][0], p[0] * r[0][1]],
                    [p[0] * r[1][0], p[0] * r[1][1]],
                ]
            }
            TransformType::Affine => [[p[0], p[1]], [p[2], p[3]]],
        };
        let n = p.len();
        if self.ty == TransformType::Translation {
            return (a, [p[0], p[1]]);
        }
        let [cx, cy] = self.c;
        let b = [
            cx + p[n - 2] - (a[0][0] * cx + a[0][1] * cy),
            cy + p[n - 1] - (a[1][0] * cx + a[1][1] * cy),
        ];
        (a, b)
    }
    /// dU/dp_j at the fixed world point (x, y), into `d[j]` = [dx, dy].
    fn jacobian(&self, p: &[f64], x: f64, y: f64, d: &mut [[f64; 2]; MAX_PARAMS]) {
        let (u, v) = (x - self.c[0], y - self.c[1]);
        let n = p.len();
        match self.ty {
            TransformType::Translation => {}
            TransformType::Rigid => {
                let (s, c) = (libm::sin(p[0]), libm::cos(p[0]));
                d[0] = [-s * u - c * v, c * u - s * v];
            }
            TransformType::Similarity => {
                let (s, c) = (libm::sin(p[1]), libm::cos(p[1]));
                let k = p[0];
                d[0] = [c * u - s * v, s * u + c * v];
                d[1] = [k * (-s * u - c * v), k * (c * u - s * v)];
            }
            TransformType::Affine => {
                d[0] = [u, 0.];
                d[1] = [v, 0.];
                d[2] = [0., u];
                d[3] = [0., v];
            }
        }
        d[n - 2] = [1., 0.];
        d[n - 1] = [0., 1.];
    }
    /// The parameters of the fixed-to-moving map `u` (a transformation of
    /// this model's class).
    fn params(&self, u: &Tform) -> Vec<f64> {
        let a = u.matrix();
        if self.ty == TransformType::Translation {
            return vec![a[0][2], a[1][2]];
        }
        let (theta_scale, m): (Vec<f64>, [[f64; 2]; 2]) = match u {
            Tform::Transl(_) => (Vec::new(), [[1., 0.], [0., 1.]]),
            Tform::Rigid { angle, .. } => {
                let theta = angle.to_radians();
                (vec![theta], rotation(theta))
            }
            Tform::Sim { scale, angle, .. } => {
                let theta = angle.to_radians();
                let r = rotation(theta);
                (
                    vec![*scale, theta],
                    [
                        [scale * r[0][0], scale * r[0][1]],
                        [scale * r[1][0], scale * r[1][1]],
                    ],
                )
            }
            _ => {
                let m = [[a[0][0], a[0][1]], [a[1][0], a[1][1]]];
                (vec![m[0][0], m[0][1], m[1][0], m[1][1]], m)
            }
        };
        let [cx, cy] = self.c;
        let mut p = theta_scale;
        p.push(m[0][0] * cx + m[0][1] * cy + a[0][2] - cx);
        p.push(m[1][0] * cx + m[1][1] * cy + a[1][2] - cy);
        p
    }
    /// The parameters of the identity map.
    fn identity(&self) -> Vec<f64> {
        match self.ty {
            TransformType::Translation => vec![0.; 2],
            TransformType::Rigid => vec![0.; 3],
            TransformType::Similarity => vec![1., 0., 0., 0.],
            TransformType::Affine => vec![1., 0., 0., 1., 0., 0.],
        }
    }
    /// The moving-to-fixed transformation (the inverse of U), of the class
    /// of the transformation type.
    fn tform(&self, p: &[f64]) -> Result<Tform> {
        let (a, b) = self.map(p);
        let u = match self.ty {
            TransformType::Translation => return Tform::translation([-b[0], -b[1]]),
            TransformType::Rigid => Tform::rigid(p[0].to_degrees(), b)?,
            TransformType::Similarity => {
                if p[0].is_nan() || p[0] <= 0. {
                    return Err(format!("the estimated scale ({}) is not positive", p[0]));
                }
                Tform::similarity(p[0], p[1].to_degrees(), b)?
            }
            TransformType::Affine => Tform::affine([
                [a[0][0], a[0][1], b[0]],
                [a[1][0], a[1][1], b[1]],
                [0., 0., 1.],
            ])?,
        };
        u.inverse()
            .map_err(|_| "the estimated transformation is singular".into())
    }
}

/// `[cos -sin; sin cos]` of an angle in radians.
fn rotation(theta: f64) -> [[f64; 2]; 2] {
    let (s, c) = (libm::sin(theta), libm::cos(theta));
    [[c, -s], [s, c]]
}

/// Polls `cancel` once per BLOCK elements of a loop over columns of `rows`.
fn poll_column(cancel: &AtomicBool, column: usize, rows: usize) -> Result<()> {
    if column.is_multiple_of((BLOCK / rows.max(1)).max(1)) {
        image_basic::cancelled(cancel)?;
    }
    Ok(())
}

/// One image of a pyramid level: m-by-n column-major values (borrowed at
/// full resolution) and their referencing object.
#[derive(Clone, Debug)]
struct Plane<'a> {
    h: usize,
    w: usize,
    data: Cow<'a, [f64]>,
    r: Imref2d,
}

impl Plane<'_> {
    /// Half size along each axis (ceil): the [1 3 3 1]/8 filter over the
    /// pixel pairs (2k-1, 2k), edges repeated. The world box keeps its first
    /// corner; the pixel extents double.
    fn reduce(&self, cancel: &AtomicBool) -> Result<Plane<'static>> {
        let (h, w) = (self.h, self.w);
        let (h2, w2) = (h.div_ceil(2), w.div_ceil(2));
        let taps = |k: usize, n: usize| -> [usize; 4] {
            let first = 2 * k as i64 - 1;
            std::array::from_fn(|i| (first + i as i64).clamp(0, n as i64 - 1) as usize)
        };
        const WEIGHTS: [f64; 4] = [0.125, 0.375, 0.375, 0.125];
        // Along columns (rows halve) first, then along rows.
        let mut half = vec![0.; h2 * w];
        for c in 0..w {
            poll_column(cancel, c, h)?;
            let column = &self.data[c * h..(c + 1) * h];
            for k in 0..h2 {
                half[c * h2 + k] = taps(k, h)
                    .iter()
                    .zip(WEIGHTS)
                    .map(|(&i, wt)| wt * column[i])
                    .sum();
            }
        }
        let mut data = vec![0.; h2 * w2];
        for k in 0..w2 {
            poll_column(cancel, k, h2)?;
            let cols = taps(k, w);
            for r in 0..h2 {
                data[k * h2 + r] = cols
                    .iter()
                    .zip(WEIGHTS)
                    .map(|(&c, wt)| wt * half[c * h2 + r])
                    .sum();
            }
        }
        let (ex, ey) = (self.r.pixel_extent_x(), self.r.pixel_extent_y());
        let (x0, y0) = (self.r.x[0], self.r.y[0]);
        let r = Imref2d::with_limits(
            [h2 as f64, w2 as f64],
            &[x0, x0 + 2. * ex * w2 as f64],
            &[y0, y0 + 2. * ey * h2 as f64],
        )?;
        Ok(Plane {
            h: h2,
            w: w2,
            data: Cow::Owned(data),
            r,
        })
    }
}

/// The plane with its central-difference derivatives per world unit
/// (one-sided at the edges): [value, d/dx, d/dy] per pixel, interleaved so a
/// bilinear sample reads four neighbouring entries.
fn with_gradients(p: &Plane, cancel: &AtomicBool) -> Result<Vec<[f64; 3]>> {
    let (h, w) = (p.h, p.w);
    let (ex, ey) = (p.r.pixel_extent_x(), p.r.pixel_extent_y());
    let at = |r: usize, c: usize| p.data[c * h + r];
    let mut out = vec![[0.; 3]; h * w];
    for c in 0..w {
        poll_column(cancel, c, h)?;
        let (c0, c1) = (c.saturating_sub(1), (c + 1).min(w - 1));
        for r in 0..h {
            let (r0, r1) = (r.saturating_sub(1), (r + 1).min(h - 1));
            let o = &mut out[c * h + r];
            o[0] = at(r, c);
            if c1 > c0 {
                o[1] = (at(r, c1) - at(r, c0)) / ((c1 - c0) as f64 * ex);
            }
            if r1 > r0 {
                o[2] = (at(r1, c) - at(r0, c)) / ((r1 - r0) as f64 * ey);
            }
        }
    }
    Ok(out)
}

/// Bin layout of one image for the Mattes metric: two padding bins at each
/// end, so the values min..max have Parzen window terms 2..bins-2.
#[derive(Clone, Copy, Debug)]
struct Window {
    min: f64,
    size: f64,
}

impl Window {
    /// Over the finite `values`: bin size (max - min)/(bins - 4); 1 when all
    /// values are equal.
    fn new(values: &[f64], bins: usize) -> Self {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &v in values.iter().filter(|v| v.is_finite()) {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        if lo > hi {
            (lo, hi) = (0., 0.);
        }
        let size = (hi - lo) / (bins - 4) as f64;
        Window {
            min: lo,
            size: if size > 0. && size.is_finite() {
                size
            } else {
                1.
            },
        }
    }
    #[inline]
    fn term(&self, v: f64) -> f64 {
        2. + (v - self.min) / self.size
    }
}

/// The joint histogram layout of the Mattes metric at one level.
#[derive(Clone, Copy, Debug)]
struct Bins {
    count: usize,
    fixed: Window,
    moving: Window,
}

impl Bins {
    /// The bin of a window term: its floor, kept in [2, count - 3].
    #[inline]
    fn index(&self, term: f64) -> usize {
        (term.max(2.) as usize).min(self.count - 3)
    }
}

/// Cubic B-spline.
#[inline]
fn bspline3(x: f64) -> f64 {
    let a = x.abs();
    if a < 1. {
        (4. - 6. * a * a + 3. * a * a * a) / 6.
    } else if a < 2. {
        let b = 2. - a;
        b * b * b / 6.
    } else {
        0.
    }
}

/// Derivative of the cubic B-spline.
#[inline]
fn bspline3_derivative(x: f64) -> f64 {
    let a = x.abs();
    if a < 1. {
        x * (1.5 * a - 2.)
    } else if a < 2. {
        let b = 2. - a;
        -0.5 * b * b * x.signum()
    } else {
        0.
    }
}

/// One pyramid level: the fixed image (its finite pixels, or the picked
/// ones, are the samples) and the moving image with its world-unit
/// gradients.
struct Level<'a> {
    fixed: Plane<'a>,
    /// Moving image size [rows, columns] and referencing.
    msize: [usize; 2],
    mr: Imref2d,
    moving: Vec<[f64; 3]>,
    /// Sorted indices of the sampled fixed pixels (None: all of them).
    picks: Option<Vec<usize>>,
    /// Histogram layout (Mattes metric only).
    bins: Option<Bins>,
}

impl<'a> Level<'a> {
    /// Level `stage` (1 = coarsest), set up for `cost`.
    fn new(
        fixed: Plane<'a>,
        moving: &Plane,
        cost: &Cost,
        stage: usize,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let (mut picks, mut bins) = (None, None);
        if let Cost::MattesMutualInformation {
            bins: count,
            samples,
        } = *cost
        {
            bins = Some(Bins {
                count,
                fixed: Window::new(&fixed.data, count),
                moving: Window::new(&moving.data, count),
            });
            picks = samples.and_then(|wanted| pick(&fixed.data, wanted, stage));
        }
        Ok(Level {
            fixed,
            msize: [moving.h, moving.w],
            mr: moving.r.clone(),
            moving: with_gradients(moving, cancel)?,
            picks,
            bins,
        })
    }
    /// Sample slots: the picked pixels, or every pixel.
    fn slots(&self) -> usize {
        self.picks.as_ref().map_or(self.fixed.data.len(), Vec::len)
    }
    /// Calls `f(x, y, value)` for the finite fixed samples in the slots
    /// `start..end`, at their world centres.
    #[inline]
    fn samples(&self, start: usize, end: usize, mut f: impl FnMut(f64, f64, f64)) {
        let Some(picks) = &self.picks else {
            return self.pixels(start, end, f);
        };
        let p = &self.fixed;
        let (ex, ey) = (p.r.pixel_extent_x(), p.r.pixel_extent_y());
        for &i in &picks[start..end] {
            let v = p.data[i];
            if v.is_finite() {
                let (r, c) = (i % p.h, i / p.h);
                f(
                    p.r.x[0] + (c as f64 + 0.5) * ex,
                    p.r.y[0] + (r as f64 + 0.5) * ey,
                    v,
                );
            }
        }
    }
    /// Calls `f(x, y, value)` for the finite fixed pixels `start..end`
    /// (column-major) at their world centres.
    #[inline]
    fn pixels(&self, start: usize, end: usize, mut f: impl FnMut(f64, f64, f64)) {
        let p = &self.fixed;
        let (ex, ey) = (p.r.pixel_extent_x(), p.r.pixel_extent_y());
        let (mut r, mut c) = (start % p.h, start / p.h);
        let mut x = p.r.x[0] + (c as f64 + 0.5) * ex;
        for &v in &p.data[start..end] {
            if v.is_finite() {
                f(x, p.r.y[0] + (r as f64 + 0.5) * ey, v);
            }
            r += 1;
            if r == p.h {
                r = 0;
                c += 1;
                x = p.r.x[0] + (c as f64 + 0.5) * ex;
            }
        }
    }
    /// RMS shift (fixed pixels of this level) of the samples per unit change
    /// of each parameter at `p`.
    fn scales(&self, model: &Model, p: &[f64], cancel: &AtomicBool) -> Result<Vec<f64>> {
        let (ex, ey) = (self.fixed.r.pixel_extent_x(), self.fixed.r.pixel_extent_y());
        let mut total = vec![0.; p.len()];
        let mut count = 0usize;
        let mut d = [[0.; 2]; MAX_PARAMS];
        let n = self.fixed.data.len();
        for start in (0..n).step_by(BLOCK) {
            image_basic::cancelled(cancel)?;
            self.pixels(start, (start + BLOCK).min(n), |x, y, _| {
                model.jacobian(p, x, y, &mut d);
                for (t, dj) in total.iter_mut().zip(&d) {
                    let (sx, sy) = (dj[0] / ex, dj[1] / ey);
                    *t += sx * sx + sy * sy;
                }
                count += 1;
            });
        }
        let count = count.max(1) as f64;
        Ok(total
            .into_iter()
            .map(|t| (t / count).sqrt().max(1e-12))
            .collect())
    }
    /// U as a map from fixed world coordinates to moving intrinsic
    /// coordinates: `[[a00, a01, b0], [a10, a11, b1]]`.
    fn to_moving_intrinsic(&self, a: [[f64; 2]; 2], b: [f64; 2]) -> [[f64; 3]; 2] {
        let (ex, ey) = (self.mr.pixel_extent_x(), self.mr.pixel_extent_y());
        [
            [a[0][0] / ex, a[0][1] / ex, 0.5 + (b[0] - self.mr.x[0]) / ex],
            [a[1][0] / ey, a[1][1] / ey, 0.5 + (b[1] - self.mr.y[0]) / ey],
        ]
    }
    /// Bilinear value and world gradient of the moving image at intrinsic
    /// (u, v); None outside the image (the pixel-centre hull plus half a
    /// pixel, where the edge values repeat) or where they are not finite.
    #[inline]
    fn moving_at(&self, u: f64, v: f64) -> Option<[f64; 3]> {
        let [mh, mw] = self.msize;
        let (w, h) = (mw as f64, mh as f64);
        if !(0.5..=w + 0.5).contains(&u) || !(0.5..=h + 0.5).contains(&v) {
            return None;
        }
        let (u, v) = (u.clamp(1., w), v.clamp(1., h));
        // Upper-left tap, 0-based; the last pixel interpolates from before it.
        let c0 = (u.floor().min(w - 1.) - 1.) as usize;
        let r0 = (v.floor().min(h - 1.) - 1.) as usize;
        let (fx, fy) = (u - (c0 + 1) as f64, v - (r0 + 1) as f64);
        let i = c0 * mh + r0;
        let m = &self.moving;
        let (a, b, c, d) = (m[i], m[i + 1], m[i + mh], m[i + mh + 1]);
        let out: [f64; 3] = std::array::from_fn(|k| {
            (1. - fx) * ((1. - fy) * a[k] + fy * b[k]) + fx * ((1. - fy) * c[k] + fy * d[k])
        });
        out.iter().all(|v| v.is_finite()).then_some(out)
    }
}

/// A cost to minimise over the transformation parameters.
#[derive(Clone, Copy, Debug)]
pub(in crate::engine) enum Cost {
    MeanSquares,
    /// Minus the Mattes mutual information: histogram bins per image and
    /// spatial samples per level (None: every pixel).
    MattesMutualInformation {
        bins: usize,
        samples: Option<usize>,
    },
}

/// One cost evaluation.
struct Eval {
    value: f64,
    /// d value / d p (empty when not asked for).
    gradient: Vec<f64>,
}

/// Partial sums over one block of samples.
#[derive(Clone, Copy)]
struct Partial {
    sum: f64,
    grad: [f64; MAX_PARAMS],
    count: usize,
}

impl Cost {
    fn new(m: &rc::Metric) -> Result<Self> {
        match m {
            rc::Metric::MeanSquares => Ok(Cost::MeanSquares),
            rc::Metric::MattesMutualInformation(m) => {
                let bins = m.number_of_histogram_bins;
                if !(5. ..=MAX_BINS as f64).contains(&bins) || bins.fract() != 0. {
                    return Err(format!(
                        "NumberOfHistogramBins of the MattesMutualInformation metric must be an integer from 5 to {MAX_BINS} (the joint histogram has NumberOfHistogramBins^2 entries), not {bins}"
                    ));
                }
                Ok(Cost::MattesMutualInformation {
                    bins: bins as usize,
                    samples: (!m.use_all_pixels)
                        .then(|| m.number_of_spatial_samples.max(1.) as usize),
                })
            }
        }
    }
    fn evaluate(
        &self,
        cancel: &AtomicBool,
        level: &Level,
        model: &Model,
        p: &[f64],
        gradient: bool,
    ) -> Result<Option<Eval>> {
        match self {
            Cost::MeanSquares => mean_squares(cancel, level, model, p, gradient),
            Cost::MattesMutualInformation { .. } => mattes(cancel, level, model, p, gradient),
        }
    }
}

/// `block(k)` for every block of `slots` sample slots, in block order; on
/// the worker pool from PAR_SAMPLES slots. Callers combine the results in
/// that order, so 1 and N workers give the same sums.
fn run_blocks<T: Send>(
    slots: usize,
    block: impl Fn(usize) -> Result<T> + Sync + Send,
) -> Result<Vec<T>> {
    let blocks = slots.div_ceil(BLOCK);
    if slots >= PAR_SAMPLES && crate::parallel::current_worker_count() > 1 {
        (0..blocks).into_par_iter().map(block).collect()
    } else {
        (0..blocks).map(block).collect()
    }
}

/// Mean of squared differences over the overlapping samples and its
/// gradient; None when no sample overlaps the moving image.
fn mean_squares(
    cancel: &AtomicBool,
    level: &Level,
    model: &Model,
    p: &[f64],
    gradient: bool,
) -> Result<Option<Eval>> {
    let (a, b) = model.map(p);
    let t = level.to_moving_intrinsic(a, b);
    let n = p.len();
    let samples = level.slots();
    let block = |k: usize| -> Result<Partial> {
        image_basic::cancelled(cancel)?;
        let mut part = Partial {
            sum: 0.,
            grad: [0.; MAX_PARAMS],
            count: 0,
        };
        let mut d = [[0.; 2]; MAX_PARAMS];
        let start = k * BLOCK;
        level.samples(start, (start + BLOCK).min(samples), |x, y, f| {
            let u = t[0][0] * x + t[0][1] * y + t[0][2];
            let v = t[1][0] * x + t[1][1] * y + t[1][2];
            let Some([m, gx, gy]) = level.moving_at(u, v) else {
                return;
            };
            let r = m - f;
            part.sum += r * r;
            part.count += 1;
            if gradient {
                model.jacobian(p, x, y, &mut d);
                for (g, dj) in part.grad[..n].iter_mut().zip(&d) {
                    *g += r * (gx * dj[0] + gy * dj[1]);
                }
            }
        });
        Ok(part)
    };
    let parts = run_blocks(samples, block)?;
    // Blocks combine in order: the same sums on 1 or N workers.
    let mut total = Partial {
        sum: 0.,
        grad: [0.; MAX_PARAMS],
        count: 0,
    };
    for part in &parts {
        total.sum += part.sum;
        total.count += part.count;
        for (t, g) in total.grad.iter_mut().zip(part.grad) {
            *t += g;
        }
    }
    if total.count == 0 {
        return Ok(None);
    }
    let count = total.count as f64;
    Ok(Some(Eval {
        value: total.sum / count,
        gradient: if gradient {
            total.grad[..n].iter().map(|g| 2. * g / count).collect()
        } else {
            Vec::new()
        },
    }))
}

/// Minus the Mattes mutual information of the joint histogram of the
/// overlapping samples, and its gradient; None when no sample overlaps the
/// moving image.
fn mattes(
    cancel: &AtomicBool,
    level: &Level,
    model: &Model,
    p: &[f64],
    gradient: bool,
) -> Result<Option<Eval>> {
    let bins = level.bins.expect("the Mattes metric sets up its bins");
    let nb = bins.count;
    let (a, b) = model.map(p);
    let t = level.to_moving_intrinsic(a, b);
    let slots = level.slots();
    // The fixed bin and the moving window term of each overlapping sample,
    // in sample order.
    let parts = run_blocks(slots, |k| -> Result<Vec<(u32, f64)>> {
        image_basic::cancelled(cancel)?;
        let (start, end) = (k * BLOCK, ((k + 1) * BLOCK).min(slots));
        let mut part = Vec::with_capacity(end - start);
        level.samples(start, end, |x, y, f| {
            let u = t[0][0] * x + t[0][1] * y + t[0][2];
            let v = t[1][0] * x + t[1][1] * y + t[1][2];
            if let Some([m, _, _]) = level.moving_at(u, v) {
                part.push((bins.index(bins.fixed.term(f)) as u32, bins.moving.term(m)));
            }
        });
        Ok(part)
    })?;
    // Joint histogram, row = fixed bin: zero-order fixed kernel, cubic
    // B-spline moving kernel (its four weights sum to 1).
    let mut joint = vec![0.; nb * nb];
    let mut count = 0usize;
    for &(fixed_bin, term) in parts.iter().flatten() {
        let k0 = bins.index(term);
        let row = &mut joint[fixed_bin as usize * nb..][..nb];
        for (k, h) in row.iter_mut().enumerate().take(k0 + 3).skip(k0 - 1) {
            *h += bspline3(k as f64 - term);
        }
        count += 1;
    }
    drop(parts);
    if count == 0 {
        return Ok(None);
    }
    let n = count as f64;
    let mut fixed_pdf = vec![0.; nb];
    let mut moving_pdf = vec![0.; nb];
    for (row, pf) in joint.chunks_exact_mut(nb).zip(&mut fixed_pdf) {
        for (h, pm) in row.iter_mut().zip(&mut moving_pdf) {
            *h /= n;
            *pf += *h;
            *pm += *h;
        }
    }
    // The mutual information and, in place of the joint probabilities, the
    // log ratios log(p(i, k) / pM(k)) the gradient weighs.
    let mut mi = 0.;
    for (row, &pf) in joint.chunks_exact_mut(nb).zip(&fixed_pdf) {
        for (h, &pm) in row.iter_mut().zip(&moving_pdf) {
            let pj = std::mem::take(h);
            if pj > TINY && pm > TINY && pf > TINY {
                let ratio = libm::log(pj / pm);
                *h = ratio;
                mi += pj * (ratio - libm::log(pf));
            }
        }
    }
    if !gradient {
        return Ok(Some(Eval {
            value: -mi,
            gradient: Vec::new(),
        }));
    }
    // d(-MI)/dp_j = 1/(n binsize) sum over samples of
    // sum_k B3'(k - term) log(p(i, k)/pM(k)) * (grad M . dU/dp_j).
    let table = &joint;
    let np = p.len();
    let parts = run_blocks(slots, |k| -> Result<[f64; MAX_PARAMS]> {
        image_basic::cancelled(cancel)?;
        let mut g = [0.; MAX_PARAMS];
        let mut d = [[0.; 2]; MAX_PARAMS];
        let start = k * BLOCK;
        level.samples(start, (start + BLOCK).min(slots), |x, y, f| {
            let u = t[0][0] * x + t[0][1] * y + t[0][2];
            let v = t[1][0] * x + t[1][1] * y + t[1][2];
            let Some([m, gx, gy]) = level.moving_at(u, v) else {
                return;
            };
            let term = bins.moving.term(m);
            let k0 = bins.index(term);
            let row = &table[bins.index(bins.fixed.term(f)) * nb..][..nb];
            let w: f64 = (k0 - 1..=k0 + 2)
                .map(|k| bspline3_derivative(k as f64 - term) * row[k])
                .sum();
            if w == 0. {
                return;
            }
            model.jacobian(p, x, y, &mut d);
            for (gj, dj) in g[..np].iter_mut().zip(&d) {
                *gj += w * (gx * dj[0] + gy * dj[1]);
            }
        });
        Ok(g)
    })?;
    let mut total = [0.; MAX_PARAMS];
    for g in &parts {
        for (t, v) in total.iter_mut().zip(g) {
            *t += v;
        }
    }
    let scale = 1. / (n * bins.moving.size);
    Ok(Some(Eval {
        value: -mi,
        gradient: total[..np].iter().map(|g| g * scale).collect(),
    }))
}

/// Why an optimization at one level stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum Stop {
    /// The normalised gradient fell below GradientMagnitudeTolerance.
    GradientMagnitude,
    /// The relaxed step fell below MinimumStepLength.
    MinimumStepLength,
    /// MaximumIterations iterations ran.
    MaximumIterations,
    /// The evolutionary search radius fell to Epsilon.
    Epsilon,
}

impl Stop {
    fn text(self) -> &'static str {
        match self {
            Stop::GradientMagnitude => "gradient magnitude below GradientMagnitudeTolerance",
            Stop::MinimumStepLength => "step length below MinimumStepLength",
            Stop::MaximumIterations => "MaximumIterations reached",
            Stop::Epsilon => "search radius at most Epsilon",
        }
    }
}

/// One progress report: after every iteration (`stop` None) and once at the
/// end of each level (`stop` Some).
#[allow(dead_code)] // `tform` is for the Registration Estimator (7c4).
#[derive(Clone, Debug)]
pub(in crate::engine) struct Progress {
    /// 1 = coarsest; `levels` = full resolution.
    pub level: usize,
    pub levels: usize,
    /// Fixed image size at this level.
    pub size: [usize; 2],
    pub iteration: usize,
    /// Cost at the start of the iteration (gradient descent) or of the
    /// parent after it (evolutionary).
    pub value: f64,
    /// Normalised gradient magnitude at the start of the iteration; NaN for
    /// the evolutionary search.
    pub gradient: f64,
    /// Step length taken (level pixels); the last one at a level end. For
    /// the evolutionary search: the search radius (Frobenius norm of A,
    /// level pixels) the iteration used.
    pub step: f64,
    /// The estimate after the iteration (moving to fixed), when valid.
    pub tform: Option<Tform>,
    pub stop: Option<Stop>,
}

/// The search over the parameters.
#[derive(Clone, Copy, Debug)]
pub(in crate::engine) enum Search {
    RegularStepGradientDescent(rc::RegularStepGradientDescent),
    OnePlusOneEvolutionary(rc::OnePlusOneEvolutionary),
}

impl Search {
    fn new(o: &rc::Optimizer) -> Result<Self> {
        match o {
            rc::Optimizer::RegularStepGradientDescent(o) => {
                Ok(Search::RegularStepGradientDescent(*o))
            }
            rc::Optimizer::OnePlusOneEvolutionary(o) => Ok(Search::OnePlusOneEvolutionary(*o)),
        }
    }
}

/// What imregtform estimates.
#[derive(Clone, Debug)]
pub(in crate::engine) struct Settings {
    pub ty: TransformType,
    pub optimizer: rc::Optimizer,
    pub metric: rc::Metric,
    /// Starting moving-to-fixed transformation (None: identity).
    pub initial: Option<Tform>,
    /// Pyramid levels, at least 1.
    pub levels: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ty: TransformType::Rigid,
            optimizer: rc::Optimizer::RegularStepGradientDescent(Default::default()),
            metric: rc::Metric::MeanSquares,
            initial: None,
            levels: 3,
        }
    }
}

/// An image to register: m-by-n column-major values placed by `r`.
#[derive(Clone, Copy, Debug)]
pub(in crate::engine) struct Image<'a> {
    pub data: &'a [f64],
    pub size: [usize; 2],
    pub r: &'a Imref2d,
}

impl<'a> Image<'a> {
    fn plane(&self) -> Plane<'a> {
        Plane {
            h: self.size[0],
            w: self.size[1],
            data: Cow::Borrowed(self.data),
            r: self.r.clone(),
        }
    }
}

/// Per-iteration progress callback shared by the optimizers.
type ReportFn<'a> = dyn FnMut(usize, f64, f64, f64, &[f64], Option<Stop>) -> Result<()> + 'a;

/// Regular step gradient descent at one level (ITK's rules): the step starts
/// at MaximumStepLength; it is multiplied by RelaxationFactor whenever the
/// normalised gradient turns by more than 90 degrees from the previous one;
/// the search stops when the gradient magnitude is below
/// GradientMagnitudeTolerance, when the step falls below MinimumStepLength or
/// after MaximumIterations steps. Each step moves the normalised parameters
/// by the step length against the gradient.
#[allow(clippy::too_many_arguments)]
fn regular_step(
    cancel: &AtomicBool,
    o: &rc::RegularStepGradientDescent,
    cost: &Cost,
    level: &Level,
    model: &Model,
    p: &mut [f64],
    report: &mut ReportFn<'_>,
    overlap: &dyn Fn(usize) -> String,
) -> Result<()> {
    let scales = level.scales(model, p, cancel)?;
    let mut step = o.maximum_step_length;
    let mut previous = vec![0.; p.len()];
    let mut iteration = 0;
    let mut last = (f64::NAN, f64::NAN);
    loop {
        if iteration as f64 >= o.maximum_iterations {
            return report(
                iteration,
                last.0,
                last.1,
                step,
                p,
                Some(Stop::MaximumIterations),
            );
        }
        iteration += 1;
        image_basic::cancelled(cancel)?;
        let eval = cost
            .evaluate(cancel, level, model, p, true)?
            .ok_or_else(|| overlap(iteration))?;
        let g: Vec<f64> = eval
            .gradient
            .iter()
            .zip(&scales)
            .map(|(g, c)| g / c)
            .collect();
        let magnitude = g.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !(eval.value.is_finite() && magnitude.is_finite()) {
            return Err(format!(
                "the metric is not finite at iteration {iteration} (value {}, gradient {magnitude})",
                eval.value
            ));
        }
        last = (eval.value, magnitude);
        if magnitude < o.gradient_magnitude_tolerance {
            return report(
                iteration,
                eval.value,
                magnitude,
                step,
                p,
                Some(Stop::GradientMagnitude),
            );
        }
        let turn: f64 = g.iter().zip(&previous).map(|(a, b)| a * b).sum();
        if turn < 0. {
            step *= o.relaxation_factor;
        }
        if step < o.minimum_step_length {
            return report(
                iteration,
                eval.value,
                magnitude,
                step,
                p,
                Some(Stop::MinimumStepLength),
            );
        }
        for ((pj, gj), c) in p.iter_mut().zip(&g).zip(&scales) {
            *pj -= step * gj / magnitude / c;
        }
        previous = g;
        report(iteration, eval.value, magnitude, step, p, None)?;
    }
}

/// One-plus-one evolutionary search at one level (ITK's rules): A starts
/// as InitialRadius times the identity in normalised units (level pixels);
/// each iteration tries child = parent + A z with z standard normal and
/// keeps it when its cost is lower (a child without overlap or with a
/// non-finite cost is worse); then the search stops if the Frobenius norm
/// of A is at most Epsilon, else A += (adjust - 1)/|z|^2 (A z) z' with
/// adjust GrowthFactor after a success and GrowthFactor^(-1/4) after a
/// failure. At most MaximumIterations children per level.
#[allow(clippy::too_many_arguments)]
fn one_plus_one(
    cancel: &AtomicBool,
    o: &rc::OnePlusOneEvolutionary,
    cost: &Cost,
    level: &Level,
    model: &Model,
    p: &mut [f64],
    random: &mut Random,
    report: &mut ReportFn<'_>,
    overlap: &dyn Fn(usize) -> String,
) -> Result<()> {
    let scales = level.scales(model, p, cancel)?;
    let n = p.len();
    // A, column-major, in normalised units.
    let mut a = vec![0.; n * n];
    for i in 0..n {
        a[i * n + i] = o.initial_radius;
    }
    let shrink = o.growth_factor.powf(-0.25);
    let mut parent = cost
        .evaluate(cancel, level, model, p, false)?
        .ok_or_else(|| overlap(0))?
        .value;
    if !parent.is_finite() {
        return Err(format!(
            "the metric is not finite at the start of the level (value {parent})"
        ));
    }
    let (mut z, mut delta, mut child) = (vec![0.; n], vec![0.; n], vec![0.; n]);
    let mut radius = o.initial_radius * (n as f64).sqrt();
    let mut iteration = 0;
    while (iteration as f64) < o.maximum_iterations {
        iteration += 1;
        image_basic::cancelled(cancel)?;
        z.fill_with(|| random.normal());
        for (r, dr) in delta.iter_mut().enumerate() {
            *dr = z.iter().enumerate().map(|(c, zc)| a[c * n + r] * zc).sum();
        }
        for (((cj, pj), dj), sj) in child.iter_mut().zip(&*p).zip(&delta).zip(&scales) {
            *cj = pj + dj / sj;
        }
        let better = match cost.evaluate(cancel, level, model, &child, false)? {
            Some(e) if e.value < parent => {
                parent = e.value;
                p.copy_from_slice(&child);
                true
            }
            _ => false,
        };
        radius = a.iter().map(|v| v * v).sum::<f64>().sqrt();
        if radius <= o.epsilon {
            return report(iteration, parent, f64::NAN, radius, p, Some(Stop::Epsilon));
        }
        let adjust = if better { o.growth_factor } else { shrink };
        let zz: f64 = z.iter().map(|v| v * v).sum();
        if zz > 0. {
            let alpha = (adjust - 1.) / zz;
            for (c, zc) in z.iter().enumerate() {
                for (r, dr) in delta.iter().enumerate() {
                    a[c * n + r] += alpha * dr * zc;
                }
            }
        }
        report(iteration, parent, f64::NAN, radius, p, None)?;
    }
    report(
        iteration,
        parent,
        f64::NAN,
        radius,
        p,
        Some(Stop::MaximumIterations),
    )
}

/// Smallest image dimension `levels` pyramid levels need.
fn min_size(levels: usize) -> Option<usize> {
    let shift = u32::try_from(levels.checked_sub(1)?).ok()?;
    MIN_LEVEL_PIXELS
        .checked_shl(shift)
        .filter(|v| v >> shift == MIN_LEVEL_PIXELS)
}

/// The moving-to-fixed transformation that aligns `moving` with `fixed`,
/// of the class `settings.ty` gives. `progress` hears every iteration and
/// every level end (an error from it stops the estimate); `cancel` is
/// polled every iteration and every 16384 samples.
pub(in crate::engine) fn estimate(
    cancel: &AtomicBool,
    moving: Image,
    fixed: Image,
    settings: &Settings,
    progress: &mut dyn FnMut(&Progress) -> Result<()>,
) -> Result<Tform> {
    let cost = Cost::new(&settings.metric)?;
    let search = Search::new(&settings.optimizer)?;
    let levels = settings.levels.max(1);
    let smallest = |s: [usize; 2]| s[0].min(s[1]);
    let need = min_size(levels);
    if need.is_none_or(|n| smallest(moving.size) < n || smallest(fixed.size) < n) {
        let need = need.map_or("more".into(), |n| n.to_string());
        return Err(format!(
            "the images are too small for PyramidLevels = {levels}: each dimension of the moving and fixed images needs at least {need} pixels ({MIN_LEVEL_PIXELS} at the coarsest level, {MIN_LEVEL_PIXELS}*2^(PyramidLevels-1)); moving is {}-by-{}, fixed is {}-by-{}. Use fewer pyramid levels",
            moving.size[0], moving.size[1], fixed.size[0], fixed.size[1]
        ));
    }
    let kind = settings.ty.kind();
    let model = Model {
        ty: settings.ty,
        c: [
            (fixed.r.x[0] + fixed.r.x[1]) / 2.,
            (fixed.r.y[0] + fixed.r.y[1]) / 2.,
        ],
    };
    let mut p = match &settings.initial {
        None => model.identity(),
        Some(t) => {
            let t = Tform::from_premultiply(kind, t.matrix(), kind.class()).map_err(|m| {
                format!(
                    "InitialTransformation must be a {} transformation for transformType '{}' ({m})",
                    kind.class(),
                    settings.ty.name()
                )
            })?;
            let u = t
                .inverse()
                .map_err(|_| "InitialTransformation must be invertible".to_string())?;
            model.params(&u)
        }
    };
    // Pyramids, full resolution (borrowed) first.
    let mut fixed_levels = vec![fixed.plane()];
    let mut moving_levels = vec![moving.plane()];
    for _ in 1..levels {
        let f = fixed_levels.last().unwrap().reduce(cancel)?;
        let m = moving_levels.last().unwrap().reduce(cancel)?;
        fixed_levels.push(f);
        moving_levels.push(m);
    }
    let mut random = Random::new(SEED);
    for stage in 1..=levels {
        let level = Level::new(
            fixed_levels.pop().expect("one plane per level"),
            &moving_levels.pop().expect("one plane per level"),
            &cost,
            stage,
            cancel,
        )?;
        let size = [level.fixed.h, level.fixed.w];
        let mut report = |iteration: usize,
                          value: f64,
                          gradient: f64,
                          step: f64,
                          p: &[f64],
                          stop: Option<Stop>|
         -> Result<()> {
            progress(&Progress {
                level: stage,
                levels,
                size,
                iteration,
                value,
                gradient,
                step,
                tform: model.tform(p).ok(),
                stop,
            })
        };
        let overlap = |iteration: usize| {
            format!(
                "no fixed pixel maps inside the moving image at pyramid level {stage} of {levels}, iteration {iteration}; the images do not overlap at the current transformation (give a closer 'InitialTransformation', or a smaller MaximumStepLength or InitialRadius)"
            )
        };
        match &search {
            Search::RegularStepGradientDescent(o) => regular_step(
                cancel,
                o,
                &cost,
                &level,
                &model,
                &mut p,
                &mut report,
                &overlap,
            )?,
            Search::OnePlusOneEvolutionary(o) => one_plus_one(
                cancel,
                o,
                &cost,
                &level,
                &model,
                &mut p,
                &mut random,
                &mut report,
                &overlap,
            )?,
        }
    }
    model.tform(&p)
}

// ------------------------------------------------------------------ builtins

const OPTIONS: [&str; 3] = [
    "DisplayOptimization",
    "InitialTransformation",
    "PyramidLevels",
];

fn usage(who: &str) -> String {
    format!(
        "{who}: expected {who}(moving, fixed, transformType, optimizer, metric) or {who}(moving, Rmoving, fixed, Rfixed, transformType, optimizer, metric), then name-value pairs 'DisplayOptimization', 'InitialTransformation', 'PyramidLevels'"
    )
}

/// A parameter name: exact or a unique prefix, any case.
fn option(who: &str, name: &str) -> Result<&'static str> {
    if let Some(found) = OPTIONS.iter().find(|o| o.eq_ignore_ascii_case(name)) {
        return Ok(found);
    }
    let lower = name.to_ascii_lowercase();
    let matches: Vec<_> = OPTIONS
        .iter()
        .filter(|o| !lower.is_empty() && o.to_ascii_lowercase().starts_with(&lower))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        _ => Err(format!(
            "{who}: unknown parameter '{name}' (expected 'DisplayOptimization', 'InitialTransformation' or 'PyramidLevels')"
        )),
    }
}

/// Class name of any argument, for messages.
fn class_name(e: &Engine, v: &Value) -> String {
    if let Some(g) = geo_of(e, v) {
        return g.class().into();
    }
    if e.obj_live()
        && let Some(c) = super::super::objects::reg_config_value(e, v)
    {
        return c.class().into();
    }
    match v {
        Value::Text(_) => "char".into(),
        Value::QuotedString(_) => "string".into(),
        _ => v
            .clone()
            .array(&e.cancel)
            .map(|a| a.class().to_owned())
            .unwrap_or_else(|_| "unknown".into()),
    }
}

/// A 2-D grayscale image of a class the reference page lists.
fn grayscale(e: &Engine, who: &str, what: &str, v: &Value) -> Result<Img> {
    if geo_of(e, v).is_some()
        || (e.obj_live() && super::super::objects::reg_config_value(e, v).is_some())
    {
        return Err(format!(
            "{who}: {what} must be an image, not a {} object. {}",
            class_name(e, v),
            usage(who)
        ));
    }
    let img = image_basic::image(e, who, v)?;
    let supported = match img.kind {
        ImageKind::Double => true,
        ImageKind::Logical => false,
        ImageKind::Typed(c) => !matches!(c.name(), "int64" | "uint64"),
    };
    if !supported {
        return Err(format!(
            "{who}: {what} must be single, double, int8, int16, int32, uint8, uint16 or uint32, not {}",
            class_name(e, v)
        ));
    }
    if img.c != 1 {
        return Err(format!(
            "{who}: {what} must be a 2-D grayscale image; 3-D volumes and RGB images are not supported yet"
        ));
    }
    if img.h == 0 || img.w == 0 {
        return Err(format!("{who}: {what} must not be empty"));
    }
    Ok(img)
}

fn referencing(
    e: &Engine,
    who: &str,
    what: &str,
    v: &Value,
    img: &Img,
    name: &str,
) -> Result<Imref2d> {
    let r = imref_of(e, v).ok_or_else(|| {
        format!(
            "{who}: {what} must be an imref2d object, not a {}. {}",
            class_name(e, v),
            usage(who)
        )
    })?;
    if r.size != [img.h as f64, img.w as f64] {
        return Err(format!(
            "{who}: {what}.ImageSize [{} {}] must match the size of {name} ({}-by-{})",
            r.size[0], r.size[1], img.h, img.w
        ));
    }
    Ok(r)
}

/// The parsed inputs of imregtform/imregister.
struct Inputs {
    moving: Img,
    rm: Imref2d,
    fixed: Img,
    rf: Imref2d,
    settings: Settings,
    display: bool,
}

fn inputs(e: &Engine, who: &str, args: &[Value]) -> Result<Inputs> {
    let referenced = args.get(1).is_some_and(|v| imref_of(e, v).is_some());
    let used = if referenced { 7 } else { 5 };
    if args.len() < used {
        return Err(format!("{who}: not enough input arguments. {}", usage(who)));
    }
    let moving = grayscale(e, who, "moving", &args[0])?;
    let (fixed, rm, rf) = if referenced {
        let fixed = grayscale(e, who, "fixed", &args[2])?;
        let rm = referencing(e, who, "Rmoving", &args[1], &moving, "moving")?;
        let rf = referencing(e, who, "Rfixed", &args[3], &fixed, "fixed")?;
        (fixed, rm, rf)
    } else {
        let fixed = grayscale(e, who, "fixed", &args[1])?;
        let rm = Imref2d::new([moving.h as f64, moving.w as f64]);
        let rf = Imref2d::new([fixed.h as f64, fixed.w as f64]);
        (fixed, rm, rf)
    };
    let (ty, opt, met) = (&args[used - 3], &args[used - 2], &args[used - 1]);
    let ty_text = image_basic::text(e, ty).ok_or_else(|| {
        format!(
            "{who}: transformType must be 'translation', 'rigid', 'similarity' or 'affine', not a {} input. {}",
            class_name(e, ty),
            usage(who)
        )
    })?;
    let ty = TransformType::parse(&ty_text).ok_or_else(|| {
        format!(
            "{who}: transformType must be 'translation', 'rigid', 'similarity' or 'affine', not '{ty_text}'"
        )
    })?;
    let optimizer = rc::optimizer(e, opt).ok_or_else(|| {
        let hint = if rc::metric(e, opt).is_some() {
            " (the optimizer comes before the metric)"
        } else {
            ""
        };
        format!(
            "{who}: OPTIMIZER must be a registration.optimizer object (RegularStepGradientDescent or OnePlusOneEvolutionary, made by imregconfig), not a {}{hint}",
            class_name(e, opt)
        )
    })?;
    let metric = rc::metric(e, met).ok_or_else(|| {
        format!(
            "{who}: METRIC must be a registration.metric object (MeanSquares or MattesMutualInformation, made by imregconfig), not a {}",
            class_name(e, met)
        )
    })?;
    let mut settings = Settings {
        ty,
        optimizer,
        metric,
        initial: None,
        levels: 3,
    };
    let mut display = false;
    let mut rest = &args[used..];
    while let Some(first) = rest.first() {
        let key = image_basic::text(e, first).ok_or_else(|| {
            format!(
                "{who}: expected a parameter name, not a {} input. {}",
                class_name(e, first),
                usage(who)
            )
        })?;
        let name = option(who, &key)?;
        let value = rest
            .get(1)
            .ok_or_else(|| format!("{who}: the parameter '{name}' needs a value"))?;
        match name {
            "DisplayOptimization" => {
                let v = image_basic::numbers(value)
                    .filter(|v| v.len() == 1 && !v[0].is_nan())
                    .ok_or_else(|| {
                        format!(
                            "{who}: DisplayOptimization must be a logical scalar (true or false)"
                        )
                    })?;
                display = v[0] != 0.;
            }
            "InitialTransformation" => {
                settings.initial = Some(tform_of(e, value).ok_or_else(|| {
                    format!(
                        "{who}: InitialTransformation must be a 2-D geometric transformation object (transltform2d, rigidtform2d, simtform2d, affinetform2d or affine2d), not a {}",
                        class_name(e, value)
                    )
                })?);
            }
            _ => {
                let v = image_basic::numbers(value)
                    .filter(|v| {
                        v.len() == 1 && v[0] >= 1. && v[0].fract() == 0. && v[0].is_finite()
                    })
                    .ok_or_else(|| format!("{who}: PyramidLevels must be a positive integer"))?;
                settings.levels = v[0].min(1e6) as usize;
            }
        }
        rest = &rest[2..];
    }
    Ok(Inputs {
        moving,
        rm,
        fixed,
        rf,
        settings,
        display,
    })
}

/// One DisplayOptimization line (our format; the reference page gives none).
fn display_line(p: &Progress) -> String {
    match p.stop {
        None if p.gradient.is_nan() => format!(
            "  iteration {:4}  metric {:.6e}  radius {:.4e}",
            p.iteration, p.value, p.step
        ),
        None => format!(
            "  iteration {:4}  metric {:.6e}  gradient {:.4e}  step {:.4e}",
            p.iteration, p.value, p.gradient, p.step
        ),
        Some(stop) => format!(
            "  stopped after {} iterations: {}",
            p.iteration,
            stop.text()
        ),
    }
}

/// `imregtform(...)` and `imregister(...)`.
pub(in crate::engine) fn builtin(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let register = name == "imregister";
    if nargout > if register { 2 } else { 1 } {
        return Err("Too many output arguments.".into());
    }
    let inputs = inputs(e, name, args)?;
    // Working memory: the moving image with its two gradients and both
    // pyramids (a third more).
    // Mattes: the per-sample bins of one evaluation and the joint histogram.
    let (mpx, fpx) = (inputs.moving.data.len(), inputs.fixed.data.len());
    let mattes = match inputs.settings.metric {
        rc::Metric::MattesMutualInformation(m) => {
            let bins = m.number_of_histogram_bins.clamp(0., MAX_BINS as f64) as usize;
            fpx.saturating_mul(2).saturating_add(bins * bins)
        }
        rc::Metric::MeanSquares => 0,
    };
    e.preflight_elements(
        mpx.saturating_mul(4)
            .saturating_add(fpx / 2)
            .saturating_add(mattes),
    )?;
    let cancel = e.cancel.clone();
    let mut shown = 0;
    let mut progress = |p: &Progress| -> Result<()> {
        if !inputs.display {
            return Ok(());
        }
        if p.level != shown {
            shown = p.level;
            e.emit(
                out,
                format!(
                    "Pyramid level {} of {} ({}-by-{} fixed image):",
                    p.level, p.levels, p.size[0], p.size[1]
                ),
            )?;
        }
        e.emit(out, display_line(p))
    };
    let tform = estimate(
        &cancel,
        Image {
            data: &inputs.moving.data,
            size: [inputs.moving.h, inputs.moving.w],
            r: &inputs.rm,
        },
        Image {
            data: &inputs.fixed.data,
            size: [inputs.fixed.h, inputs.fixed.w],
            r: &inputs.rf,
        },
        &inputs.settings,
        &mut progress,
    )
    .map_err(|m| {
        if m == "Calculation cancelled" || m.starts_with(name) {
            m
        } else {
            format!("{name}: {m}")
        }
    })?;
    if !register {
        return Ok(vec![new_object(e, Geo::Tform(tform))?]);
    }
    let rf = inputs.rf;
    let (rows, cols) = (rf.size[0] as usize, rf.size[1] as usize);
    e.preflight_elements(rows * cols)?;
    let data = super::warp(
        &cancel,
        &inputs.moving.data,
        [inputs.moving.h, inputs.moving.w],
        &inputs.rm,
        &tform,
        &rf,
        Sampling {
            interp: Interp::Linear,
            fill: 0.,
            smooth: false,
        },
    )
    .map_err(|m| m.replacen("imwarp", name, 1))?;
    let image = Img {
        h: rows,
        w: cols,
        c: 1,
        kind: inputs.moving.kind,
        data,
    };
    let mut outputs = vec![image_basic::to_value(e, image)?];
    if nargout > 1 {
        outputs.push(new_object(e, Geo::Imref(rf))?);
    }
    Ok(outputs)
}
