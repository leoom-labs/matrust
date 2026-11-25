//! Colormap tables. The default map is viridis (van der Walt and Smith, 2015,
//! CC0, github.com/BIDS/colormap); `parula` is accepted as a name for it because
//! MATLAB's own parula table is not published under terms we may reproduce.
//! The other maps are the classic closed-form ramps (gray, hot, cool, jet, hsv,
//! bone, copper), written from their usual piecewise-linear definitions.
use crate::Result;

#[path = "colormap_viridis.rs"]
mod viridis;

pub const DEFAULT_ROWS: usize = 256;
pub const NAMES: [&str; 10] = [
    "viridis", "parula", "gray", "hot", "cool", "jet", "hsv", "bone", "copper", "default",
];

pub fn default_map() -> Vec<[f64; 3]> {
    viridis::VIRIDIS.to_vec()
}

fn clamp(v: f64) -> f64 {
    v.clamp(0., 1.)
}
fn ramp(i: usize, n: usize) -> f64 {
    if n <= 1 {
        0.
    } else {
        i as f64 / (n - 1) as f64
    }
}

/// `n` rows of the named colormap, or None for an unknown name.
pub fn named(name: &str, n: usize) -> Result<Option<Vec<[f64; 3]>>> {
    if n > super::MAX_COLORMAP_ROWS {
        return Err("Colormap length exceeds 4096 rows".into());
    }
    let name = name.to_ascii_lowercase();
    let rows: Vec<[f64; 3]> = match name.as_str() {
        "viridis" | "parula" | "default" => (0..n).map(|i| viridis_at(ramp(i, n))).collect(),
        "gray" | "grey" => (0..n).map(|i| [ramp(i, n); 3]).collect(),
        "hot" => hot(n),
        "cool" => (0..n)
            .map(|i| {
                let t = ramp(i, n);
                [t, 1. - t, 1.]
            })
            .collect(),
        "jet" => (0..n)
            .map(|i| {
                // Piecewise-linear blue-cyan-yellow-red ramp with flat 1/8 ends.
                let t = ramp(i, n);
                [
                    clamp(1.5 - (4. * t - 3.).abs()),
                    clamp(1.5 - (4. * t - 2.).abs()),
                    clamp(1.5 - (4. * t - 1.).abs()),
                ]
            })
            .collect(),
        "hsv" => (0..n)
            .map(|i| hue(if n == 0 { 0. } else { i as f64 / n as f64 }))
            .collect(),
        "bone" => {
            let hot = hot(n);
            (0..n)
                .map(|i| {
                    let g = ramp(i, n);
                    // Gray tinted with a reversed-channel hot ramp: blue highlights.
                    let h = hot[i];
                    [
                        (7. * g + h[2]) / 8.,
                        (7. * g + h[1]) / 8.,
                        (7. * g + h[0]) / 8.,
                    ]
                })
                .collect()
        }
        "copper" => (0..n)
            .map(|i| {
                let t = ramp(i, n);
                [clamp(1.25 * t), clamp(0.7812 * t), clamp(0.4975 * t)]
            })
            .collect(),
        _ => return Ok(None),
    };
    Ok(Some(rows))
}

/// Black, red, yellow, white: red rises over the first 3/8, green over the
/// next 3/8 and blue over the final quarter.
fn hot(n: usize) -> Vec<[f64; 3]> {
    let first = ((3 * n) / 8).max(1) as f64;
    let rest = (n as f64 - 2. * first).max(1.);
    (1..=n)
        .map(|i| {
            let i = i as f64;
            [
                clamp(i / first),
                clamp((i - first) / first),
                clamp((i - 2. * first) / rest),
            ]
        })
        .collect()
}

/// Fully saturated, full-value colour at hue `h` in [0, 1).
fn hue(h: f64) -> [f64; 3] {
    let h6 = (h.rem_euclid(1.)) * 6.;
    let sector = h6.floor() as u8;
    let f = h6 - h6.floor();
    match sector {
        0 => [1., f, 0.],
        1 => [1. - f, 1., 0.],
        2 => [0., 1., f],
        3 => [0., 1. - f, 1.],
        4 => [f, 0., 1.],
        _ => [1., 0., 1. - f],
    }
}

fn viridis_at(t: f64) -> [f64; 3] {
    sample(&viridis::VIRIDIS, t)
}

/// Linear interpolation into a colormap at t in [0, 1].
pub fn sample(map: &[[f64; 3]], t: f64) -> [f64; 3] {
    if map.is_empty() {
        return [0.; 3];
    }
    if map.len() == 1 || !t.is_finite() {
        return map[0];
    }
    let x = clamp(t) * (map.len() - 1) as f64;
    let i = (x.floor() as usize).min(map.len() - 2);
    let f = x - i as f64;
    let (a, b) = (map[i], map[i + 1]);
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// The colormap row of value `v` under CDataMapping 'scaled' (MATLAB's
/// documented rule, Octave graphics.cc convert_cdata): row
/// floor(m (v - cmin) / (cmax - cmin)), clamped to the map; NaN takes row 0.
/// As 8-bit RGB.
pub fn scaled_rgb(map: &[[f64; 3]], clim: [f64; 2], v: f64) -> [u8; 3] {
    let m = map.len().max(1);
    let t = (v - clim[0]) / (clim[1] - clim[0]);
    let i = if t.is_nan() {
        0
    } else {
        ((t * m as f64).floor().max(0.) as usize).min(m - 1)
    };
    map.get(i)
        .copied()
        .unwrap_or([0.; 3])
        .map(|c| (c * 255.).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_maps_have_documented_end_points() {
        let gray = named("gray", 5).unwrap().unwrap();
        assert_eq!(gray[0], [0.; 3]);
        assert_eq!(gray[4], [1.; 3]);
        let hot = named("hot", 8).unwrap().unwrap();
        assert_eq!(hot[7], [1.; 3]);
        assert!(hot[0][0] > 0. && hot[0][1] == 0.);
        let jet = named("jet", 9).unwrap().unwrap();
        assert_eq!(jet[0], [0., 0., 0.5]);
        assert_eq!(jet[4], [0.5, 1., 0.5]);
        assert_eq!(jet[8], [0.5, 0., 0.]);
        let hsv = named("hsv", 6).unwrap().unwrap();
        assert_eq!(hsv[0], [1., 0., 0.]);
        assert_eq!(hsv[2], [0., 1., 0.]);
        let viridis = named("viridis", 256).unwrap().unwrap();
        assert_eq!(viridis[0], [0.267004, 0.004874, 0.329415]);
        assert_eq!(viridis[255], [0.993248, 0.906157, 0.143936]);
        assert!(named("nonesuch", 4).unwrap().is_none());
    }
}
