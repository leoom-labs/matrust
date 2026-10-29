//! `rand`, `randn`, `randi` and `rng` on one MT19937 stream per engine session.
//!
//! * Uniform doubles are `genrand_res53` on the open interval (0,1). With
//!   `rng(0)` / `rng('default')` they reproduce MATLAB's "twister" generator
//!   (MATLAB maps seed 0 to the reference MT seed 5489).
//! * `randi(imax)` is `ceil(imax*rand)`, the same stream consumption as MATLAB.
//! * `randn` is a 256-layer Marsaglia-Tsang ziggurat. Its fast path takes two
//!   32-bit words per attempt (offset-binary magnitude, top byte of the second
//!   word as layer); this matches the reference to about nine digits on the
//!   first values after `rng(0)`, but is NOT claimed bit-identical to MATLAB,
//!   and the wedge/tail paths are a standard implementation.
//!
//! Generation is serial and chunked: the stream is inherently sequential and a
//! measured 1e7 draws take well under a second, so no worker threads are used
//! and results never depend on the worker count.
use super::{Engine, Value};
use crate::{Matrix, Result};
use std::sync::OnceLock;

const N: usize = 624;
const M: usize = 397;
const CHUNK: usize = 1 << 16;
const LAYERS: usize = 256;

#[derive(Clone, Debug)]
pub(super) struct Generator {
    state: Box<[u32; N]>,
    index: usize,
}

impl Default for Generator {
    fn default() -> Self {
        let mut g = Self {
            state: Box::new([0; N]),
            index: N,
        };
        g.seed(0);
        g
    }
}

impl Generator {
    pub(super) fn seed(&mut self, seed: u32) {
        let mut s = if seed == 0 { 5489 } else { seed };
        for i in 0..N {
            self.state[i] = s;
            s = 1812433253u32
                .wrapping_mul(s ^ (s >> 30))
                .wrapping_add(i as u32 + 1);
        }
        self.index = N;
    }

    fn twist(&mut self) {
        let mix = |u: u32, v: u32| {
            let y = (u & 0x8000_0000) | (v & 0x7fff_ffff);
            (y >> 1) ^ if y & 1 == 1 { 0x9908_b0df } else { 0 }
        };
        for k in 0..N {
            let next = self.state[(k + 1) % N];
            self.state[k] = self.state[(k + M) % N] ^ mix(self.state[k], next);
        }
        self.index = 0;
    }

    pub(super) fn next_u32(&mut self) -> u32 {
        if self.index >= N {
            self.twist();
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// Uniform on the open interval (0,1) with 53 random bits.
    pub(super) fn next_f64(&mut self) -> f64 {
        loop {
            let a = (self.next_u32() >> 5) as f64;
            let b = (self.next_u32() >> 6) as f64;
            let u = (a * 67_108_864. + b) / 9_007_199_254_740_992.;
            if u > 0. {
                return u;
            }
        }
    }

    pub(super) fn next_normal(&mut self) -> f64 {
        let t = ziggurat();
        loop {
            let word = self.next_u32();
            let layer = LAYERS - 1 - (self.next_u32() >> 24) as usize;
            let j = (word ^ 0x8000_0000) as i32;
            let z = f64::from(j) * t.scale[layer];
            if j.unsigned_abs() < t.accept[layer] {
                return z;
            }
            if layer == 0 {
                let r = t.x[1];
                loop {
                    let x = -self.next_f64().ln() / r;
                    let y = -self.next_f64().ln();
                    if y + y >= x * x {
                        return if j < 0 { -(r + x) } else { r + x };
                    }
                }
            }
            let (inner, outer) = (density(t.x[layer]), density(t.x[layer + 1]));
            if inner + self.next_f64() * (outer - inner) < density(z) {
                return z;
            }
        }
    }
}

fn density(x: f64) -> f64 {
    (-0.5 * x * x).exp()
}

struct Ziggurat {
    /// Layer right edges, x[1] = r down to x[LAYERS] = 0; x[0] is the base width.
    x: [f64; LAYERS + 1],
    scale: [f64; LAYERS],
    accept: [u32; LAYERS],
}

fn ziggurat() -> &'static Ziggurat {
    static TABLE: OnceLock<Ziggurat> = OnceLock::new();
    TABLE.get_or_init(|| {
        let r = 3.654_152_885_361_009_f64;
        let v = r * density(r) + (std::f64::consts::PI / 2.).sqrt() * libm::erfc(r / 2f64.sqrt());
        let mut x = [0.; LAYERS + 1];
        x[1] = r;
        x[0] = v / density(r);
        for i in 2..LAYERS {
            x[i] = (-2. * (v / x[i - 1] + density(x[i - 1])).ln()).sqrt();
        }
        let mut scale = [0.; LAYERS];
        let mut accept = [0u32; LAYERS];
        for i in 0..LAYERS {
            scale[i] = x[i] / 2_147_483_648.;
            accept[i] = ((x[i + 1] / x[i]) * 2_147_483_648.) as u32;
        }
        Ziggurat { x, scale, accept }
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Uniform,
    Normal,
    Integer { low: f64, span: f64 },
}

fn text_of(value: &Value) -> Option<&str> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}

impl Engine {
    pub(super) fn random_call(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        if name == "rng" {
            return self.rng_call(args).map(|()| Value::Void);
        }
        let (kind, rest) = if name == "randi" {
            let first = args
                .first()
                .ok_or("randi expects an upper limit as its first argument")?;
            (self.integer_range(first)?, &args[1..])
        } else if args.first().and_then(text_of).is_some() {
            return Err(format!(
                "{name}: generator-state forms such as '{}' are not supported; use rng",
                args.first().and_then(text_of).unwrap_or_default()
            ));
        } else if name == "randn" {
            (Kind::Normal, args)
        } else {
            (Kind::Uniform, args)
        };
        let mut sizes = Vec::with_capacity(rest.len());
        for arg in rest {
            match text_of(arg) {
                Some("double") => {}
                Some(class) => {
                    return Err(format!("{name}: class '{class}' is not supported yet"));
                }
                None => sizes.push(arg.clone()),
            }
        }
        let (rows, cols) = if sizes.is_empty() {
            (1, 1)
        } else {
            super::shape_arguments::shape(&sizes, None)?
        };
        self.preflight_elements(Matrix::checked_len(rows, cols)?)?;
        let mut data = Vec::with_capacity(rows * cols);
        let total = rows * cols;
        while data.len() < total {
            self.check_cancel()?;
            let end = (data.len() + CHUNK).min(total);
            for _ in data.len()..end {
                let value = match kind {
                    Kind::Uniform => self.random.next_f64(),
                    Kind::Normal => self.random.next_normal(),
                    Kind::Integer { low, span } => {
                        low - 1. + (span * self.random.next_f64()).ceil()
                    }
                };
                data.push(value);
            }
        }
        self.check_cancel()?;
        Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
    }

    fn integer_range(&self, value: &Value) -> Result<Kind> {
        let limits = self.real_arg("randi", value)?;
        let integer = |v: f64| v.is_finite() && v.fract() == 0. && v.abs() < 9_007_199_254_740_992.;
        let (low, high) = match limits.data() {
            [high] => (1., *high),
            [low, high] => (*low, *high),
            _ => return Err("randi: the limit must be a scalar or a [lo hi] pair".into()),
        };
        if !integer(low) || !integer(high) {
            return Err("randi: limits must be integers".into());
        }
        if limits.len() == 1 && high < 1. {
            return Err(
                "randi: the upper limit must be an integer greater than or equal to 1".into(),
            );
        }
        if high < low {
            return Err("randi: the second limit must not be smaller than the first".into());
        }
        Ok(Kind::Integer {
            low,
            span: high - low + 1.,
        })
    }

    fn rng_call(&mut self, args: &[Value]) -> Result<()> {
        if args.is_empty() || args.len() > 2 {
            return Err("rng: use rng(seed), rng('default') or rng('shuffle'); querying or restoring the generator state is not supported yet".into());
        }
        if let Some(generator) = args.get(1)
            && text_of(generator).map(str::to_lowercase).as_deref() != Some("twister")
        {
            return Err("rng: only the 'twister' generator is supported yet".into());
        }
        let seed = match text_of(&args[0]) {
            Some(word) => match word.to_lowercase().as_str() {
                "default" => 0,
                "shuffle" => crate::clock::system_now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| (d.as_nanos() as u64 ^ (d.as_nanos() >> 32) as u64) as u32)
                    .unwrap_or(1),
                other => return Err(format!("rng: '{other}' is not supported yet")),
            },
            None => {
                let m = self.real_arg("rng", &args[0])?;
                let seed = m
                    .as_scalar()
                    .map_err(|_| "rng: the seed must be a scalar".to_string())?;
                if !seed.is_finite() || seed.fract() != 0. || !(0. ..4_294_967_296.).contains(&seed)
                {
                    return Err("rng: the seed must be an integer from 0 to 2^32-1".into());
                }
                seed as u32
            }
        };
        self.random.seed(seed);
        Ok(())
    }
}
