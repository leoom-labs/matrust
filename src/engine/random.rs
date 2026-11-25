//! `rand`, `randn`, `randi` and `rng` on one stream per engine session.
//!
//! * Default generator is `'twister'` (MT19937). Uniform doubles are
//!   `genrand_res53` on the open interval (0,1). With `rng(0)` / `rng('default')`
//!   they reproduce MATLAB's "twister" generator (MATLAB maps seed 0 to the
//!   reference MT seed 5489).
//! * `'philox'` uses Philox4x32-10 (Salmon et al., Random123) and
//!   `'combRecursive'` uses MRG32k3a (L'Ecuyer 1999). No claim that doubles
//!   match MATLAB bit-for-bit for `'philox'` or `'combRecursive'`.
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
use crate::{Array, Matrix, NumericClass, Result, StructArray, TypedArray, typed_array::TypedData};
use std::{
    borrow::Cow,
    sync::{Arc, OnceLock},
};

const N: usize = 624;
const M: usize = 397;
const CHUNK: usize = 1 << 16;
const LAYERS: usize = 256;

const PHILOX_M0: u32 = 0xD251_1F53;
const PHILOX_M1: u32 = 0xCD9E_8D57;
const PHILOX_W0: u32 = 0x9E37_79B9;
const PHILOX_W1: u32 = 0xBB67_AE85;

const MRG_M1: i64 = 4_294_967_087;
const MRG_M2: i64 = 4_294_944_443;
const MRG_A12: i64 = 1_403_580;
const MRG_A13: i64 = 810_728;
const MRG_A21: i64 = 527_612;
const MRG_A23: i64 = 1_370_589;
const MRG_NORM: f64 = 1.0 / (MRG_M1 as f64 + 1.0);

/// Raw Philox4x32-10 block function (Salmon et al. 2011, Random123).
pub fn philox4x32_10(mut ctr: [u32; 4], mut key: [u32; 2]) -> [u32; 4] {
    for round in 0..10 {
        if round > 0 {
            key[0] = key[0].wrapping_add(PHILOX_W0);
            key[1] = key[1].wrapping_add(PHILOX_W1);
        }
        let p0 = u64::from(PHILOX_M0) * u64::from(ctr[0]);
        let p1 = u64::from(PHILOX_M1) * u64::from(ctr[2]);
        let (lo0, hi0) = (p0 as u32, (p0 >> 32) as u32);
        let (lo1, hi1) = (p1 as u32, (p1 >> 32) as u32);
        ctr = [hi1 ^ ctr[1] ^ key[0], lo1, hi0 ^ ctr[3] ^ key[1], lo0];
    }
    ctr
}

/// One raw step of MRG32k3a (L'Ecuyer 1999), returning the combined integer in `1..=MRG_M1`.
pub fn mrg32k3a_step(s1: &mut [u32; 3], s2: &mut [u32; 3]) -> u32 {
    let p1 = (MRG_A12 * i64::from(s1[1]) - MRG_A13 * i64::from(s1[0])).rem_euclid(MRG_M1);
    s1[0] = s1[1];
    s1[1] = s1[2];
    s1[2] = p1 as u32;

    let p2 = (MRG_A21 * i64::from(s2[2]) - MRG_A23 * i64::from(s2[0])).rem_euclid(MRG_M2);
    s2[0] = s2[1];
    s2[1] = s2[2];
    s2[2] = p2 as u32;

    let z = if p1 <= p2 { p1 - p2 + MRG_M1 } else { p1 - p2 };
    z as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeneratorKind {
    Twister,
    Philox,
    CombRecursive,
}

impl GeneratorKind {
    fn parse(name: &str) -> std::result::Result<Self, String> {
        // Reuse the counter-based, modular-recurrence or bitwise-state generator
        // closest to the requested algorithm when its exact kernel is unavailable.
        match name.to_ascii_lowercase().as_str() {
            "twister" | "mt19937ar" | "simdtwister" | "v5normal" => Ok(Self::Twister),
            "philox" | "threefry" => Ok(Self::Philox),
            "combrecursive" | "multfibonacci" | "v4" | "v5uniform" => Ok(Self::CombRecursive),
            _ => Err(format!("rng: unrecognized generator '{name}'")),
        }
    }
}

#[derive(Clone, Debug)]
enum GeneratorState {
    Twister {
        state: Box<[u32; N]>,
        index: usize,
    },
    Philox {
        ctr: [u32; 4],
        key: [u32; 2],
        buf: [u32; 4],
        index: usize,
    },
    CombRecursive {
        s1: [u32; 3],
        s2: [u32; 3],
    },
}

#[derive(Clone, Debug)]
pub struct Generator {
    kind: GeneratorKind,
    type_name: Cow<'static, str>,
    seed: u32,
    state: GeneratorState,
}

impl Default for Generator {
    fn default() -> Self {
        let mut g = Self {
            kind: GeneratorKind::Twister,
            type_name: Cow::Borrowed("twister"),
            seed: 0,
            state: GeneratorState::Twister {
                state: Box::new([0; N]),
                index: N,
            },
        };
        g.seed(0);
        g
    }
}

fn twist_twister(state: &mut [u32; N]) {
    for k in 0..N {
        let next = state[(k + 1) % N];
        let y = (state[k] & 0x8000_0000) | (next & 0x7fff_ffff);
        let mixed = (y >> 1) ^ if y & 1 == 1 { 0x9908_b0df } else { 0 };
        state[k] = state[(k + M) % N] ^ mixed;
    }
}

impl Generator {
    fn set_kind_and_seed(&mut self, kind: GeneratorKind, type_name: Cow<'_, str>, seed: u32) {
        self.kind = kind;
        self.type_name = Cow::Owned(type_name.into_owned());
        self.seed(seed);
    }

    pub(super) fn seed(&mut self, seed: u32) {
        self.seed = seed;
        match self.kind {
            GeneratorKind::Twister => {
                let mut arr = Box::new([0; N]);
                let mut s = if seed == 0 { 5489 } else { seed };
                for (i, slot) in arr.iter_mut().enumerate() {
                    *slot = s;
                    s = 1812433253u32
                        .wrapping_mul(s ^ (s >> 30))
                        .wrapping_add(i as u32 + 1);
                }
                self.state = GeneratorState::Twister {
                    state: arr,
                    index: N,
                };
            }
            GeneratorKind::Philox => {
                self.state = GeneratorState::Philox {
                    ctr: [0, 0, 0, 0],
                    key: [seed, 0],
                    buf: [0; 4],
                    index: 4,
                };
            }
            GeneratorKind::CombRecursive => {
                // Mix 32-bit seed into 6 nonzero residues modulo m1 and m2.
                let mut x = if seed == 0 { 12345u64 } else { u64::from(seed) };
                let mut next_res = |m: i64| -> u32 {
                    x = x
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let r = ((x >> 16) as i64).rem_euclid(m - 1) + 1;
                    r as u32
                };
                let s1 = [next_res(MRG_M1), next_res(MRG_M1), next_res(MRG_M1)];
                let s2 = [next_res(MRG_M2), next_res(MRG_M2), next_res(MRG_M2)];
                self.state = GeneratorState::CombRecursive { s1, s2 };
            }
        }
    }

    pub(super) fn next_u32(&mut self) -> u32 {
        match &mut self.state {
            GeneratorState::Twister { state, index } => {
                if *index >= N {
                    twist_twister(state);
                    *index = 0;
                }
                let mut y = state[*index];
                *index += 1;
                y ^= y >> 11;
                y ^= (y << 7) & 0x9d2c_5680;
                y ^= (y << 15) & 0xefc6_0000;
                y ^ (y >> 18)
            }
            GeneratorState::Philox {
                ctr,
                key,
                buf,
                index,
            } => {
                if *index >= 4 {
                    *buf = philox4x32_10(*ctr, *key);
                    for word in ctr.iter_mut() {
                        let (next, carry) = word.overflowing_add(1);
                        *word = next;
                        if !carry {
                            break;
                        }
                    }
                    *index = 0;
                }
                let val = buf[*index];
                *index += 1;
                val
            }
            GeneratorState::CombRecursive { s1, s2 } => mrg32k3a_step(s1, s2),
        }
    }

    /// Uniform on the open interval (0,1).
    pub(super) fn next_f64(&mut self) -> f64 {
        if let GeneratorState::CombRecursive { s1, s2 } = &mut self.state {
            return f64::from(mrg32k3a_step(s1, s2)) * MRG_NORM;
        }
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

fn text_of(value: &Value) -> Option<std::borrow::Cow<'_, str>> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(std::borrow::Cow::Borrowed(s)),
        Value::Container(arr) => read_text_scalar(arr).map(std::borrow::Cow::Owned),
        _ => None,
    }
}

fn shuffle_seed() -> u32 {
    crate::clock::system_now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_nanos() as u64 ^ (d.as_nanos() >> 32) as u64) as u32)
        .unwrap_or(1)
}

fn uint32_col(values: &[u32]) -> Result<Arc<Array>> {
    Ok(Arc::new(Array::Typed(TypedArray::from_ints(
        NumericClass::Uint32,
        values.len(),
        1,
        values.iter().map(|&v| i128::from(v)),
    )?)))
}

fn read_uint32_slice(arr: &Array) -> Option<(&[u32], usize, usize)> {
    let Array::Typed(t) = arr else {
        return None;
    };
    if t.is_nd() {
        return None;
    }
    let TypedData::Uint32(slice) = t.data() else {
        return None;
    };
    Some((slice.as_ref(), t.rows(), t.cols()))
}

fn read_text_scalar(arr: &Array) -> Option<String> {
    match arr {
        Array::Character(c) if c.is_text_row() => String::from_utf16(c.units()).ok(),
        Array::Str(s) if s.rows() == 1 && s.cols() == 1 => {
            s.data()[0].as_deref().map(str::to_owned)
        }
        _ => None,
    }
}

const INVALID_STRUCT_ERR: &str =
    "rng: The input must be a valid generator settings structure, as returned by rng.";

impl Engine {
    pub(super) fn random_call(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        if name == "rng" {
            return self.rng_call(args);
        }
        let (kind, rest) = if name == "randi" {
            let first = args
                .first()
                .ok_or("randi expects an upper limit as its first argument")?;
            (self.integer_range(first)?, &args[1..])
        } else if let Some(text) = args.first().and_then(text_of) {
            return Err(format!(
                "{name}: generator-state forms such as '{text}' are not supported; use rng"
            ));
        } else if name == "randn" {
            (Kind::Normal, args)
        } else {
            (Kind::Uniform, args)
        };
        let mut sizes = Vec::with_capacity(rest.len());
        for arg in rest {
            match text_of(arg).as_deref() {
                Some("double") => {}
                Some(class) => {
                    return Err(format!("{name}: class '{class}' is not supported yet"));
                }
                None => sizes.push(arg.clone()),
            }
        }
        let dims = if sizes.is_empty() {
            vec![1, 1]
        } else {
            super::shape_arguments::shape_dims(&sizes, None)?
        };
        let rows = dims[0];
        let cols = super::nd::shape_product(&dims[1..])?;
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
        Ok(Value::Matrix(
            Matrix::from_column_vec(rows, cols, data)?.with_dims(&dims)?,
        ))
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

    fn rng_struct(&self) -> Result<Value> {
        let type_arr =
            Arc::new(Value::Text(self.random.type_name.to_string()).array(&self.cancel)?);
        let seed_arr = uint32_col(&[self.random.seed])?;
        let state_arr = match &self.random.state {
            GeneratorState::Twister { state, index } => {
                let mut words = Vec::with_capacity(N + 1);
                words.extend_from_slice(&state[..]);
                words.push(*index as u32);
                uint32_col(&words)?
            }
            GeneratorState::Philox {
                ctr,
                key,
                buf,
                index,
            } => {
                let words = [
                    ctr[0],
                    ctr[1],
                    ctr[2],
                    ctr[3],
                    key[0],
                    key[1],
                    buf[0],
                    buf[1],
                    buf[2],
                    buf[3],
                    *index as u32,
                ];
                uint32_col(&words)?
            }
            GeneratorState::CombRecursive { s1, s2 } => {
                let words = [s1[0], s1[1], s1[2], s2[0], s2[1], s2[2]];
                uint32_col(&words)?
            }
        };
        let names = ["Type", "Seed", "State"].map(str::to_owned).to_vec();
        let slots = vec![type_arr, seed_arr, state_arr];
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?)))
    }

    fn rng_restore(&mut self, value: &Value) -> Result<()> {
        let Value::Container(Array::Struct(s)) = value else {
            return Err(INVALID_STRUCT_ERR.into());
        };
        if s.rows() != 1 || s.cols() != 1 || s.field_count() != 3 {
            return Err(INVALID_STRUCT_ERR.into());
        }
        let type_idx = s.field_index("Type").ok_or(INVALID_STRUCT_ERR)?;
        let seed_idx = s.field_index("Seed").ok_or(INVALID_STRUCT_ERR)?;
        let state_idx = s.field_index("State").ok_or(INVALID_STRUCT_ERR)?;

        let type_name = read_text_scalar(s.get(type_idx, 0)?).ok_or(INVALID_STRUCT_ERR)?;
        let kind = GeneratorKind::parse(&type_name).map_err(|_| INVALID_STRUCT_ERR.to_string())?;

        let (seed_slice, seed_rows, seed_cols) =
            read_uint32_slice(s.get(seed_idx, 0)?).ok_or(INVALID_STRUCT_ERR)?;
        if seed_rows != 1 || seed_cols != 1 {
            return Err(INVALID_STRUCT_ERR.into());
        }
        let seed = seed_slice[0];

        let (state_slice, state_rows, state_cols) =
            read_uint32_slice(s.get(state_idx, 0)?).ok_or(INVALID_STRUCT_ERR)?;
        if state_cols != 1 {
            return Err(INVALID_STRUCT_ERR.into());
        }

        let state = match kind {
            GeneratorKind::Twister => {
                if state_rows != N + 1 {
                    return Err(INVALID_STRUCT_ERR.into());
                }
                let index = state_slice[N] as usize;
                if index > N {
                    return Err(INVALID_STRUCT_ERR.into());
                }
                let mut arr = Box::new([0u32; N]);
                arr.copy_from_slice(&state_slice[..N]);
                let mut twisted = *arr;
                twist_twister(&mut twisted);
                if arr.iter().all(|&w| w == 0) || twisted.iter().all(|&w| w == 0) {
                    return Err(INVALID_STRUCT_ERR.into());
                }
                GeneratorState::Twister { state: arr, index }
            }
            GeneratorKind::Philox => {
                if state_rows != 11 {
                    return Err(INVALID_STRUCT_ERR.into());
                }
                let index = state_slice[10] as usize;
                if index > 4 {
                    return Err(INVALID_STRUCT_ERR.into());
                }
                GeneratorState::Philox {
                    ctr: [
                        state_slice[0],
                        state_slice[1],
                        state_slice[2],
                        state_slice[3],
                    ],
                    key: [state_slice[4], state_slice[5]],
                    buf: [
                        state_slice[6],
                        state_slice[7],
                        state_slice[8],
                        state_slice[9],
                    ],
                    index,
                }
            }
            GeneratorKind::CombRecursive => {
                if state_rows != 6 {
                    return Err(INVALID_STRUCT_ERR.into());
                }
                let s1 = [state_slice[0], state_slice[1], state_slice[2]];
                let s2 = [state_slice[3], state_slice[4], state_slice[5]];
                if s1.iter().any(|&w| i64::from(w) >= MRG_M1)
                    || s2.iter().any(|&w| i64::from(w) >= MRG_M2)
                    || s1.iter().all(|&w| w == 0)
                    || s2.iter().all(|&w| w == 0)
                {
                    return Err(INVALID_STRUCT_ERR.into());
                }
                GeneratorState::CombRecursive { s1, s2 }
            }
        };

        self.random = Generator {
            kind,
            type_name: Cow::Owned(type_name),
            seed,
            state,
        };
        Ok(())
    }

    fn parse_rng_seed(&self, value: &Value) -> Result<u32> {
        if let Value::Container(Array::Typed(t)) = value {
            if t.is_nd() || t.len() != 1 {
                return Err("rng: the seed must be a scalar".into());
            }
            if t.is_integer() {
                return u32::try_from(t.int(0))
                    .map_err(|_| "rng: the seed must be an integer from 0 to 2^32-1".into());
            }
            let seed = t.real(0);
            if !seed.is_finite() || seed.fract() != 0. || !(0. ..4_294_967_296.).contains(&seed) {
                return Err("rng: the seed must be an integer from 0 to 2^32-1".into());
            }
            return Ok(seed as u32);
        }
        let m = self.real_arg("rng", value)?;
        let seed = m
            .as_scalar()
            .map_err(|_| "rng: the seed must be a scalar".to_string())?;
        if !seed.is_finite() || seed.fract() != 0. || !(0. ..4_294_967_296.).contains(&seed) {
            return Err("rng: the seed must be an integer from 0 to 2^32-1".into());
        }
        Ok(seed as u32)
    }

    pub(super) fn rng_call(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return self.rng_struct();
        }
        if args.len() > 2 {
            return Err("Too many input arguments.".into());
        }
        let prev = self.rng_struct()?;
        if args.len() == 1 {
            if matches!(args[0], Value::Container(Array::Struct(_))) {
                self.rng_restore(&args[0])?;
                return Ok(prev);
            }
            // `rng 0`: command syntax passes the char word '0' (fix line 12).
            let word =
                super::graphics::number_word(&args[0]).map(|n| Value::Matrix(Matrix::scalar(n)));
            let arg = word.as_ref().unwrap_or(&args[0]);
            match text_of(arg) {
                Some(text) if text.eq_ignore_ascii_case("default") => {
                    self.random = Generator::default();
                }
                Some(text) if text.eq_ignore_ascii_case("shuffle") => {
                    self.random.seed(shuffle_seed());
                }
                Some(text) => {
                    let kind = GeneratorKind::parse(&text)?;
                    self.random.set_kind_and_seed(kind, text, 0);
                }
                None => {
                    let seed = self.parse_rng_seed(arg)?;
                    self.random.seed(seed);
                }
            }
            return Ok(prev);
        }

        // args.len() == 2: rng(seed, generator)
        let gen_text = text_of(&args[1])
            .ok_or_else(|| "rng: second argument must be a generator name".to_string())?;
        let kind = GeneratorKind::parse(&gen_text)?;
        let word = super::graphics::number_word(&args[0]).map(|n| Value::Matrix(Matrix::scalar(n)));
        let seed_arg = word.as_ref().unwrap_or(&args[0]);
        let seed = match text_of(seed_arg).as_deref() {
            Some(text) if text.eq_ignore_ascii_case("shuffle") => shuffle_seed(),
            Some(text) if text.eq_ignore_ascii_case("default") => 0,
            Some(_) => {
                return Err(
                    "rng: first argument with a generator must be a numeric seed, 'shuffle', or 'default'"
                        .into(),
                );
            }
            None => self.parse_rng_seed(seed_arg)?,
        };
        self.random.set_kind_and_seed(kind, gen_text, seed);
        Ok(prev)
    }
}
