//! Sample division. Indices in the Rust API are zero based.
use crate::Result;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug)]
pub enum Division {
    All,
    Random([f64; 3]),
    Block([f64; 3]),
    Interleaved([f64; 3]),
    Indices([Vec<usize>; 3]),
}
impl Division {
    pub fn name(&self) -> &'static str {
        match self {
            Self::All => "dividetrain",
            Self::Random(_) => "dividerand",
            Self::Block(_) => "divideblock",
            Self::Interleaved(_) => "divideint",
            Self::Indices(_) => "divideind",
        }
    }
    pub fn split(
        &self,
        q: usize,
        uniform: &mut dyn FnMut() -> f64,
        cancel: &AtomicBool,
    ) -> Result<[Vec<usize>; 3]> {
        crate::dsp::check_cancel(cancel)?;
        if let Self::Indices(groups) = self {
            let mut seen = vec![false; q];
            let mut out: [Vec<usize>; 3] = Default::default();
            for (dst, src) in out.iter_mut().zip(groups) {
                for &i in src {
                    if i >= q {
                        continue;
                    } // Public divideind contract clips > Q.
                    if seen[i] {
                        return Err("divideind: indices must be distinct and disjoint".into());
                    }
                    seen[i] = true;
                    dst.push(i);
                }
            }
            return Ok(out);
        }
        let ratios = match self {
            Self::All => return Ok([(0..q).collect(), vec![], vec![]]),
            Self::Random(r) | Self::Block(r) | Self::Interleaved(r) => *r,
            _ => unreachable!(),
        };
        let total: f64 = ratios.iter().sum();
        if !total.is_finite() || total <= 0. || ratios.iter().any(|r| !r.is_finite() || *r < 0.) {
            return Err("division: ratios must be finite, nonnegative, with positive sum".into());
        }
        // Round training and validation counts; the remaining samples are test.
        let a = ((q as f64 * (ratios[0] / total)).round() as usize).min(q);
        let b = ((q as f64 * (ratios[1] / total)).round() as usize).min(q - a);
        let counts = [a, b, q - a - b];
        let mut out: [Vec<usize>; 3] = Default::default();
        if matches!(self, Self::Interleaved(_)) {
            // Weighted fair interleaving, deterministic training/validation/test ties.
            for i in 0..q {
                if i.is_multiple_of(4096) {
                    crate::dsp::check_cancel(cancel)?;
                }
                let k = (0..3)
                    .filter(|&k| out[k].len() < counts[k])
                    .min_by(|&j, &k| {
                        let x = (out[j].len() as f64 + 0.5) / counts[j] as f64;
                        let y = (out[k].len() as f64 + 0.5) / counts[k] as f64;
                        x.total_cmp(&y).then(j.cmp(&k))
                    })
                    .unwrap();
                out[k].push(i);
            }
        } else {
            let mut indices: Vec<_> = (0..q).collect();
            if matches!(self, Self::Random(_)) {
                for i in (1..q).rev() {
                    if i.is_multiple_of(4096) {
                        crate::dsp::check_cancel(cancel)?;
                    }
                    let u = uniform();
                    if !(0. ..1.).contains(&u) {
                        return Err("division: RNG must return values in [0,1)".into());
                    }
                    indices.swap(i, (u * (i + 1) as f64) as usize);
                }
            }
            out = [
                indices[..a].to_vec(),
                indices[a..a + b].to_vec(),
                indices[a + b..].to_vec(),
            ];
            for group in &mut out {
                group.sort_unstable();
            }
        }
        Ok(out)
    }
}
