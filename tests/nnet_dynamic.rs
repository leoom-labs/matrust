//! Independent expectations: tool/refs/nnet/dynamic.py (NumPy, never MATLAB).
use matrust::{Array, Engine};
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}\n{err}"));
}
fn values(e: &Engine, name: &str) -> Vec<f64> {
    let Array::Numeric(m) = e.array(name).unwrap() else {
        panic!("{name}")
    };
    m.data().to_vec()
}
fn near(e: &Engine, name: &str, want: &[f64], tol: f64) {
    let got = values(e, name);
    assert_eq!(got.len(), want.len(), "{name}");
    for (a, b) in got.iter().zip(want) {
        assert!(
            (a - b).abs() <= tol * b.abs().max(1e-12),
            "{name}: {a} != {b}"
        );
    }
}
#[test]
fn topology_and_distances() {
    let mut e = Engine::default();
    run(
        &mut e,
        "p=gridtop([3 4]); assert(isequal(p,[0 1 2 0 1 2 0 1 2 0 1 2;0 0 0 1 1 1 2 2 2 3 3 3])); h=hextop([3 4]); a=linkdist(p); b=mandist(p); assert(isequal(a,b)); c=boxdist(p); assert(a(1,12)==5 && c(1,12)==3); z=linkdist(h); assert(z(1,12)==4); rng(77); r=randtop([3 4]); rng(77); assert(isequal(r,randtop([3 4])));",
    );
    near(
        &e,
        "h",
        &[
            0.,
            0.,
            1.,
            0.,
            2.,
            0.,
            0.5,
            3f64.sqrt() / 2.,
            1.5,
            3f64.sqrt() / 2.,
            2.5,
            3f64.sqrt() / 2.,
            0.,
            3f64.sqrt(),
            1.,
            3f64.sqrt(),
            2.,
            3f64.sqrt(),
            0.5,
            3. * 3f64.sqrt() / 2.,
            1.5,
            3. * 3f64.sqrt() / 2.,
            2.5,
            3. * 3f64.sqrt() / 2.,
        ],
        1e-14,
    );
}

// Hard-coded NumPy output from tool/refs/nnet/dynamic.py.
const GRID: &[f64] = &[
    0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0, 0.0, 2.0, 1.0, 2.0, 2.0, 2.0, 0.0,
    3.0, 1.0, 3.0, 2.0, 3.0,
];
const GRID_LINKDIST: &[f64] = &[
    0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 3.0, 4.0, 5.0, 1.0, 0.0, 1.0, 2.0, 1.0, 2.0, 3.0,
    2.0, 3.0, 4.0, 3.0, 4.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 4.0, 3.0, 2.0, 5.0, 4.0, 3.0, 1.0, 2.0,
    3.0, 0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 2.0, 1.0, 2.0, 1.0, 0.0, 1.0, 2.0, 1.0, 2.0,
    3.0, 2.0, 3.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 4.0, 3.0, 2.0, 2.0, 3.0, 4.0, 1.0,
    2.0, 3.0, 0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 1.0, 2.0, 1.0,
    2.0, 4.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 3.0, 4.0, 5.0, 2.0, 3.0, 4.0,
    1.0, 2.0, 3.0, 0.0, 1.0, 2.0, 4.0, 3.0, 4.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 1.0, 5.0,
    4.0, 3.0, 4.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0,
];
const GRID_BOXDIST: &[f64] = &[
    0.0, 1.0, 2.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 2.0,
    2.0, 2.0, 3.0, 3.0, 3.0, 2.0, 1.0, 0.0, 2.0, 1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 1.0, 1.0,
    2.0, 0.0, 1.0, 2.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0,
    2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 2.0, 1.0, 0.0, 2.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 1.0,
    1.0, 2.0, 0.0, 1.0, 2.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0,
    1.0, 2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 2.0, 1.0, 0.0, 2.0, 1.0, 1.0, 3.0, 3.0, 3.0, 2.0, 2.0, 2.0,
    1.0, 1.0, 2.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 3.0,
    3.0, 3.0, 2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 2.0, 1.0, 0.0,
];
const GRID_MANDIST: &[f64] = &[
    0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 3.0, 4.0, 5.0, 1.0, 0.0, 1.0, 2.0, 1.0, 2.0, 3.0,
    2.0, 3.0, 4.0, 3.0, 4.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 4.0, 3.0, 2.0, 5.0, 4.0, 3.0, 1.0, 2.0,
    3.0, 0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 2.0, 1.0, 2.0, 1.0, 0.0, 1.0, 2.0, 1.0, 2.0,
    3.0, 2.0, 3.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 4.0, 3.0, 2.0, 2.0, 3.0, 4.0, 1.0,
    2.0, 3.0, 0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 1.0, 2.0, 1.0,
    2.0, 4.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 3.0, 4.0, 5.0, 2.0, 3.0, 4.0,
    1.0, 2.0, 3.0, 0.0, 1.0, 2.0, 4.0, 3.0, 4.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 1.0, 5.0,
    4.0, 3.0, 4.0, 3.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0,
];
// NumPy golden values remain literal, including sqrt(2).
#[allow(clippy::approx_constant)]
const GRID_DIST: &[f64] = &[
    0.0,
    1.0,
    2.0,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    2.0,
    2.23606797749979,
    2.8284271247461903,
    3.0,
    3.1622776601683795,
    3.605551275463989,
    1.0,
    0.0,
    1.0,
    1.4142135623730951,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    2.0,
    2.23606797749979,
    3.1622776601683795,
    3.0,
    3.1622776601683795,
    2.0,
    1.0,
    0.0,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    2.8284271247461903,
    2.23606797749979,
    2.0,
    3.605551275463989,
    3.1622776601683795,
    3.0,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    0.0,
    1.0,
    2.0,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    2.0,
    2.23606797749979,
    2.8284271247461903,
    1.4142135623730951,
    1.0,
    1.4142135623730951,
    1.0,
    0.0,
    1.0,
    1.4142135623730951,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    2.0,
    2.23606797749979,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    2.0,
    1.0,
    0.0,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    2.8284271247461903,
    2.23606797749979,
    2.0,
    2.0,
    2.23606797749979,
    2.8284271247461903,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    0.0,
    1.0,
    2.0,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    2.23606797749979,
    2.0,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    1.4142135623730951,
    1.0,
    0.0,
    1.0,
    1.4142135623730951,
    1.0,
    1.4142135623730951,
    2.8284271247461903,
    2.23606797749979,
    2.0,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    2.0,
    1.0,
    0.0,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    3.0,
    3.1622776601683795,
    3.605551275463989,
    2.0,
    2.23606797749979,
    2.8284271247461903,
    1.0,
    1.4142135623730951,
    2.23606797749979,
    0.0,
    1.0,
    2.0,
    3.1622776601683795,
    3.0,
    3.1622776601683795,
    2.23606797749979,
    2.0,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    1.4142135623730951,
    1.0,
    0.0,
    1.0,
    3.605551275463989,
    3.1622776601683795,
    3.0,
    2.8284271247461903,
    2.23606797749979,
    2.0,
    2.23606797749979,
    1.4142135623730951,
    1.0,
    2.0,
    1.0,
    0.0,
];
const HEX: &[f64] = &[
    0.0,
    0.0,
    1.0,
    0.0,
    2.0,
    0.0,
    0.5,
    0.8660254037844386,
    1.5,
    0.8660254037844386,
    2.5,
    0.8660254037844386,
    0.0,
    1.7320508075688772,
    1.0,
    1.7320508075688772,
    2.0,
    1.7320508075688772,
    0.5,
    2.598076211353316,
    1.5,
    2.598076211353316,
    2.5,
    2.598076211353316,
];
const HEX_LINKDIST: &[f64] = &[
    0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 2.0, 3.0, 3.0, 3.0, 4.0, 1.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0,
    2.0, 2.0, 3.0, 3.0, 3.0, 2.0, 1.0, 0.0, 2.0, 1.0, 1.0, 3.0, 2.0, 2.0, 3.0, 3.0, 3.0, 1.0, 1.0,
    2.0, 0.0, 1.0, 2.0, 1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 2.0, 1.0, 1.0, 1.0, 0.0, 1.0, 2.0, 1.0, 1.0,
    2.0, 2.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 3.0, 2.0, 2.0, 2.0, 2.0, 3.0, 1.0,
    2.0, 3.0, 0.0, 1.0, 2.0, 1.0, 2.0, 3.0, 2.0, 2.0, 2.0, 1.0, 1.0, 2.0, 1.0, 0.0, 1.0, 1.0, 1.0,
    2.0, 3.0, 2.0, 2.0, 2.0, 1.0, 1.0, 2.0, 1.0, 0.0, 2.0, 1.0, 1.0, 3.0, 3.0, 3.0, 2.0, 2.0, 3.0,
    1.0, 1.0, 2.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 0.0, 1.0, 4.0,
    3.0, 3.0, 3.0, 2.0, 2.0, 3.0, 2.0, 1.0, 2.0, 1.0, 0.0,
];
const HEX_BOXDIST: &[f64] = &[
    0.0,
    1.0,
    2.0,
    0.8660254037844386,
    1.5,
    2.5,
    1.7320508075688772,
    1.7320508075688772,
    2.0,
    2.598076211353316,
    2.598076211353316,
    2.598076211353316,
    1.0,
    0.0,
    1.0,
    0.8660254037844386,
    0.8660254037844386,
    1.5,
    1.7320508075688772,
    1.7320508075688772,
    1.7320508075688772,
    2.598076211353316,
    2.598076211353316,
    2.598076211353316,
    2.0,
    1.0,
    0.0,
    1.5,
    0.8660254037844386,
    0.8660254037844386,
    2.0,
    1.7320508075688772,
    1.7320508075688772,
    2.598076211353316,
    2.598076211353316,
    2.598076211353316,
    0.8660254037844386,
    0.8660254037844386,
    1.5,
    0.0,
    1.0,
    2.0,
    0.8660254037844386,
    0.8660254037844386,
    1.5,
    1.7320508075688774,
    1.7320508075688774,
    2.0,
    1.5,
    0.8660254037844386,
    0.8660254037844386,
    1.0,
    0.0,
    1.0,
    1.5,
    0.8660254037844386,
    0.8660254037844386,
    1.7320508075688774,
    1.7320508075688774,
    1.7320508075688774,
    2.5,
    1.5,
    0.8660254037844386,
    2.0,
    1.0,
    0.0,
    2.5,
    1.5,
    0.8660254037844386,
    2.0,
    1.7320508075688774,
    1.7320508075688774,
    1.7320508075688772,
    1.7320508075688772,
    2.0,
    0.8660254037844386,
    1.5,
    2.5,
    0.0,
    1.0,
    2.0,
    0.8660254037844388,
    1.5,
    2.5,
    1.7320508075688772,
    1.7320508075688772,
    1.7320508075688772,
    0.8660254037844386,
    0.8660254037844386,
    1.5,
    1.0,
    0.0,
    1.0,
    0.8660254037844388,
    0.8660254037844388,
    1.5,
    2.0,
    1.7320508075688772,
    1.7320508075688772,
    1.5,
    0.8660254037844386,
    0.8660254037844386,
    2.0,
    1.0,
    0.0,
    1.5,
    0.8660254037844388,
    0.8660254037844388,
    2.598076211353316,
    2.598076211353316,
    2.598076211353316,
    1.7320508075688774,
    1.7320508075688774,
    2.0,
    0.8660254037844388,
    0.8660254037844388,
    1.5,
    0.0,
    1.0,
    2.0,
    2.598076211353316,
    2.598076211353316,
    2.598076211353316,
    1.7320508075688774,
    1.7320508075688774,
    1.7320508075688774,
    1.5,
    0.8660254037844388,
    0.8660254037844388,
    1.0,
    0.0,
    1.0,
    2.598076211353316,
    2.598076211353316,
    2.598076211353316,
    2.0,
    1.7320508075688774,
    1.7320508075688774,
    2.5,
    1.5,
    0.8660254037844388,
    2.0,
    1.0,
    0.0,
];
const HEX_MANDIST: &[f64] = &[
    0.0,
    1.0,
    2.0,
    1.3660254037844386,
    2.3660254037844384,
    3.3660254037844384,
    1.7320508075688772,
    2.732050807568877,
    3.732050807568877,
    3.098076211353316,
    4.098076211353316,
    5.098076211353316,
    1.0,
    0.0,
    1.0,
    1.3660254037844386,
    1.3660254037844386,
    2.3660254037844384,
    2.732050807568877,
    1.7320508075688772,
    2.732050807568877,
    3.098076211353316,
    3.098076211353316,
    4.098076211353316,
    2.0,
    1.0,
    0.0,
    2.3660254037844384,
    1.3660254037844386,
    1.3660254037844386,
    3.732050807568877,
    2.732050807568877,
    1.7320508075688772,
    4.098076211353316,
    3.098076211353316,
    3.098076211353316,
    1.3660254037844386,
    1.3660254037844386,
    2.3660254037844384,
    0.0,
    1.0,
    2.0,
    1.3660254037844386,
    1.3660254037844386,
    2.3660254037844384,
    1.7320508075688774,
    2.7320508075688776,
    3.7320508075688776,
    2.3660254037844384,
    1.3660254037844386,
    1.3660254037844386,
    1.0,
    0.0,
    1.0,
    2.3660254037844384,
    1.3660254037844386,
    1.3660254037844386,
    2.7320508075688776,
    1.7320508075688774,
    2.7320508075688776,
    3.3660254037844384,
    2.3660254037844384,
    1.3660254037844386,
    2.0,
    1.0,
    0.0,
    3.3660254037844384,
    2.3660254037844384,
    1.3660254037844386,
    3.7320508075688776,
    2.7320508075688776,
    1.7320508075688774,
    1.7320508075688772,
    2.732050807568877,
    3.732050807568877,
    1.3660254037844386,
    2.3660254037844384,
    3.3660254037844384,
    0.0,
    1.0,
    2.0,
    1.3660254037844388,
    2.366025403784439,
    3.366025403784439,
    2.732050807568877,
    1.7320508075688772,
    2.732050807568877,
    1.3660254037844386,
    1.3660254037844386,
    2.3660254037844384,
    1.0,
    0.0,
    1.0,
    1.3660254037844388,
    1.3660254037844388,
    2.366025403784439,
    3.732050807568877,
    2.732050807568877,
    1.7320508075688772,
    2.3660254037844384,
    1.3660254037844386,
    1.3660254037844386,
    2.0,
    1.0,
    0.0,
    2.366025403784439,
    1.3660254037844388,
    1.3660254037844388,
    3.098076211353316,
    3.098076211353316,
    4.098076211353316,
    1.7320508075688774,
    2.7320508075688776,
    3.7320508075688776,
    1.3660254037844388,
    1.3660254037844388,
    2.366025403784439,
    0.0,
    1.0,
    2.0,
    4.098076211353316,
    3.098076211353316,
    3.098076211353316,
    2.7320508075688776,
    1.7320508075688774,
    2.7320508075688776,
    2.366025403784439,
    1.3660254037844388,
    1.3660254037844388,
    1.0,
    0.0,
    1.0,
    5.098076211353316,
    4.098076211353316,
    3.098076211353316,
    3.7320508075688776,
    2.7320508075688776,
    1.7320508075688774,
    3.366025403784439,
    2.366025403784439,
    1.3660254037844388,
    2.0,
    1.0,
    0.0,
];
const HEX_DIST: &[f64] = &[
    0.0,
    1.0,
    2.0,
    0.9999999999999999,
    1.7320508075688772,
    2.6457513110645907,
    1.7320508075688772,
    1.9999999999999998,
    2.6457513110645907,
    2.6457513110645907,
    3.0,
    3.605551275463989,
    1.0,
    0.0,
    1.0,
    0.9999999999999999,
    0.9999999999999999,
    1.7320508075688772,
    1.9999999999999998,
    1.7320508075688772,
    1.9999999999999998,
    2.6457513110645907,
    2.6457513110645907,
    3.0,
    2.0,
    1.0,
    0.0,
    1.7320508075688772,
    0.9999999999999999,
    0.9999999999999999,
    2.6457513110645907,
    1.9999999999999998,
    1.7320508075688772,
    3.0,
    2.6457513110645907,
    2.6457513110645907,
    0.9999999999999999,
    0.9999999999999999,
    1.7320508075688772,
    0.0,
    1.0,
    2.0,
    0.9999999999999999,
    0.9999999999999999,
    1.7320508075688772,
    1.7320508075688774,
    2.0,
    2.6457513110645907,
    1.7320508075688772,
    0.9999999999999999,
    0.9999999999999999,
    1.0,
    0.0,
    1.0,
    1.7320508075688772,
    0.9999999999999999,
    0.9999999999999999,
    2.0,
    1.7320508075688774,
    2.0,
    2.6457513110645907,
    1.7320508075688772,
    0.9999999999999999,
    2.0,
    1.0,
    0.0,
    2.6457513110645907,
    1.7320508075688772,
    0.9999999999999999,
    2.6457513110645907,
    2.0,
    1.7320508075688774,
    1.7320508075688772,
    1.9999999999999998,
    2.6457513110645907,
    0.9999999999999999,
    1.7320508075688772,
    2.6457513110645907,
    0.0,
    1.0,
    2.0,
    1.0000000000000002,
    1.7320508075688774,
    2.6457513110645907,
    1.9999999999999998,
    1.7320508075688772,
    1.9999999999999998,
    0.9999999999999999,
    0.9999999999999999,
    1.7320508075688772,
    1.0,
    0.0,
    1.0,
    1.0000000000000002,
    1.0000000000000002,
    1.7320508075688774,
    2.6457513110645907,
    1.9999999999999998,
    1.7320508075688772,
    1.7320508075688772,
    0.9999999999999999,
    0.9999999999999999,
    2.0,
    1.0,
    0.0,
    1.7320508075688774,
    1.0000000000000002,
    1.0000000000000002,
    2.6457513110645907,
    2.6457513110645907,
    3.0,
    1.7320508075688774,
    2.0,
    2.6457513110645907,
    1.0000000000000002,
    1.0000000000000002,
    1.7320508075688774,
    0.0,
    1.0,
    2.0,
    3.0,
    2.6457513110645907,
    2.6457513110645907,
    2.0,
    1.7320508075688774,
    2.0,
    1.7320508075688774,
    1.0000000000000002,
    1.0000000000000002,
    1.0,
    0.0,
    1.0,
    3.605551275463989,
    3.0,
    2.6457513110645907,
    2.6457513110645907,
    2.0,
    1.7320508075688774,
    2.6457513110645907,
    1.7320508075688774,
    1.0000000000000002,
    2.0,
    1.0,
    0.0,
];
const SOM_1: &[f64] = &[
    -0.25999999999999995,
    0.10000000000000003,
    0.10000000000000003,
    0.10000000000000003,
    0.10000000000000003,
    0.7222222222222222,
    -0.02999999999999998,
    0.15833333333333335,
    0.15833333333333335,
    0.15833333333333335,
    0.15833333333333335,
    0.5444444444444445,
];
const HITS_1: &[f64] = &[4.0, 3.0, 0.0, 0.0, 0.0, 5.0];
const SOM_10: &[f64] = &[
    -1.1199999999999999,
    -0.6375,
    0.7222222222222222,
    -1.1199999999999999,
    0.7222222222222222,
    0.9714285714285714,
    -0.5399999999999999,
    -0.16249999999999998,
    0.5444444444444445,
    -0.5399999999999999,
    0.5444444444444445,
    0.6571428571428573,
];
const HITS_10: &[f64] = &[3.0, 2.0, 3.0, 0.0, 0.0, 4.0];
const SOM_200: &[f64] = &[
    -1.1199999999999999,
    -0.6375,
    0.7222222222222222,
    -1.1199999999999999,
    0.7222222222222222,
    0.9714285714285714,
    -0.5399999999999999,
    -0.16249999999999998,
    0.5444444444444445,
    -0.5399999999999999,
    0.5444444444444445,
    0.6571428571428573,
];
const HITS_200: &[f64] = &[3.0, 2.0, 3.0, 0.0, 0.0, 4.0];
const OPEN: &[f64] = &[
    -0.020940159961802686,
    -0.03273628042304408,
    -0.04182509545951814,
    -0.04779975630005204,
    -0.05053520883144057,
    -0.050000140949429145,
    -0.04615157983541851,
    -0.038951047630168184,
    -0.028505216507026226,
    -0.015287612427743395,
    -0.00031675994602333585,
    0.014881684705670188,
    0.02858821126735477,
    0.03930308671727991,
    0.04599936471806894,
    0.04813176123427329,
    0.04553011925773033,
    0.03834724331494559,
    0.027129555372898944,
    0.012934694735278841,
];
const CLOSED: &[f64] = &[
    -0.020940159961802686,
    -0.027503559904159647,
    -0.03623719814364382,
    -0.04288410211242222,
    -0.04643332911497197,
    -0.0468478413896601,
    -0.04412381426657111,
    -0.03828736945140372,
    -0.029486521429958973,
    -0.018151068263936858,
    -0.005130956466689443,
    0.008309809588937386,
    0.020701032236524947,
    0.0306805677119479,
    0.03723481050312638,
    0.039771638625510114,
    0.03808184654316622,
    0.032303486762905675,
    0.02294835368903418,
    0.010948408859765758,
];
const LM_WEIGHTS: &[f64] = &[
    -0.11589792238978325,
    -0.09728504865537585,
    0.07021699455243446,
    -1.0889603833688335,
    0.3770084724529186,
    -1.049074529494769,
    -0.8248273775227911,
    -0.22993005153598606,
    -0.6349771556921497,
    -0.2317277306092052,
    -0.030094243056652814,
    -0.8495158573235894,
    -0.032512165311268826,
];
const LM_PERF: &[f64] = &[
    0.19237043886376107,
    0.16802515670984688,
    0.12459354978294589,
    0.08754465325809409,
    0.07777993814315308,
    0.005216733302751715,
];

#[test]
fn topology_numpy_full_matrices() {
    let mut e = Engine::default();
    run(
        &mut e,
        "g=gridtop([3 4]); h=hextop([3 4]); gl=linkdist(g); gb=boxdist(g); gm=mandist(g); gd=dist(g); hl=linkdist(h); hb=boxdist(h); hm=mandist(h); hd=dist(h);",
    );
    for (name, want) in [
        ("g", GRID),
        ("h", HEX),
        ("gl", GRID_LINKDIST),
        ("gb", GRID_BOXDIST),
        ("gm", GRID_MANDIST),
        ("gd", GRID_DIST),
        ("hl", HEX_LINKDIST),
        ("hb", HEX_BOXDIST),
        ("hm", HEX_MANDIST),
        ("hd", HEX_DIST),
    ] {
        near(&e, name, want, 1e-14);
    }
    run(&mut e, "assert(isequal(linkdist([0 5]),[0 2;2 0]));");
}
const SOM_DATA: &str = "x=[-2 -1.8 -1.5 -.5 -.2 0 .2 .7 1 1.5 1.8 2;-1 -.8 -1.2 .7 1 1.2 -.4 -.8 .3 .7 1 1.2]; n=configure(selforgmap([3 2],5,2),x); n=setwb(n,[-2 -1 0 .5 1 2 -1 0 1 -1 .5 1.5]');";
#[test]
fn som_numpy_batch_epochs_hits_and_value_semantics() {
    let mut e = Engine::default();
    run(&mut e, SOM_DATA);
    for (epochs, want, hits) in [
        (1, SOM_1, HITS_1),
        (10, SOM_10, HITS_10),
        (200, SOM_200, HITS_200),
    ] {
        run(
            &mut e,
            &format!(
                "m=n; m.trainParam.epochs={epochs}; [m,tr]=train(m,x); w=getwb(m); ids=vec2ind(m(x)); hits=sum(m(x),2); assert(tr.num_epochs=={epochs}); assert(isequal(getwb(n),[-2 -1 0 .5 1 2 -1 0 1 -1 .5 1.5]')); assert(isequal(size(m(x)),[6 12]));"
            ),
        );
        near(&e, "w", want, 1e-10);
        near(&e, "hits", hits, 0.);
    }
    run(
        &mut e,
        "assert(isequal(n.layers{1}.positions,hextop([3 2]))); assert(strcmp(n.inputWeights{1,1}.learnFcn,'learnsomb')); z=init(n); assert(isconfigured(z));",
    );
}
const DYNAMIC_DATA: &str = "x=con2seq(sin((0:21)*.3)); t=con2seq(cos((0:21)*.23)*.5+.2*sin((0:21)*.3)); n=narxnet(1:2,1:2,2); n.inputs{1}.processFcns={}; n.outputs{2}.processFcns={}; [xs,xi,ai,ts,ew,shift]=preparets(n,x,{},t); n=configure(n,xs,ts); n=setwb(n,(1:n.numWeightElements)'*.03-.3);";
#[test]
fn preparets_shapes_helpers_and_delay_removal() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=con2seq(1:10); t=con2seq(11:20); [xs,xi,ai,ts,ew,s]=preparets(narxnet(1:2,1:2),x,{},t); assert(s==2 && isequal(size(xs),[2 8]) && isequal(size(xi),[2 2]) && isequal(size(ai),[2 0])); assert(isequal(xi,{1,2;11,12}) && isequal(ts,t(3:end))); [xs,xi,ai,ts,ew,s]=preparets(narnet(1:3),{},{},t); assert(s==3 && isequal(size(xs),[1 7]) && isequal(xi,{11,12,13})); [xs,xi,ai,ts,ew,s]=preparets(timedelaynet(0:2),x,t); assert(s==2 && isequal(xs,x(3:end)) && isequal(size(xi),[1 2])); n=removedelay(narxnet(1:2,1:2),1); assert(isequal(n.inputWeights{1,1}.delays,0:1) && isequal(n.inputWeights{1,2}.delays,2:3)); [a,b]=tonndata([1 2 3;4 5 6],true,false); z=fromnndata(a,b,true,false); assert(isequal(z,[1 2 3;4 5 6])); [a,b]=tonndata([1 4;2 5;3 6],false,false); z=fromnndata(a,b,false,false); assert(isequal(z,[1 4;2 5;3 6])); assert(isequal(cell2mat(seq2con(con2seq([1 2;3 4]))),[1 2;3 4]));",
    );
}
#[test]
fn dynamic_numpy_open_closed_and_continuation_states() {
    let mut e = Engine::default();
    run(&mut e, DYNAMIC_DATA);
    run(
        &mut e,
        "[y,xf,af]=n(xs,xi,ai); y=cell2mat(seq2con(y)); [a,xf1,af1]=n(xs(:,1:7),xi,ai); [b,xf2,af2]=n(xs(:,8:end),xf1,af1); joined=cell2mat(seq2con([a,b])); assert(isequal(y,joined) && isequal(xf,xf2) && isequal(af,af2)); c=closeloop(n); [xc,xic,aic]=preparets(c,x,{},t); [yc,xfc,afc]=c(xc,xic,aic); yc=cell2mat(seq2con(yc)); [a,xf1,af1]=c(xc(:,1:7),xic,aic); [b,xf2,af2]=c(xc(:,8:end),xf1,af1); assert(isequal(yc,cell2mat(seq2con([a,b]))) && isequal(xfc,xf2) && isequal(afc,af2)); [c,xic,aic]=closeloop(n,xf,af); [nn,xio,aio]=openloop(c,xic,aic); assert(isequal(xio,xf)); assert(isequal(getwb(nn),getwb(n))); ",
    );
    near(&e, "y", OPEN, 1e-13);
    near(&e, "yc", CLOSED, 1e-13);
    run(
        &mut e,
        "c=narxnet(1:2,1:2,2,'closed'); c.inputs{1}.processFcns={}; c.outputs{2}.processFcns={}; c=configure(c,x,t); c=setwb(c,(1:c.numWeightElements)'*.03-.3); [xc,xic,aic]=preparets(c,x,{},t); yc=cell2mat(seq2con(c(xc,xic,aic))); assert(isequal(n(xs,[],[]),n(xs)));",
    );
    near(&e, "yc", CLOSED, 1e-13);
}
#[test]
fn dynamic_lm_numpy_five_epochs() {
    let mut e = Engine::default();
    run(&mut e, DYNAMIC_DATA);
    run(
        &mut e,
        "n.divideFcn='divideind'; n.divideParam.trainInd=1:12; n.divideParam.valInd=[]; n.divideParam.testInd=[]; n.trainParam.epochs=5; n.trainParam.min_grad=0; n.trainParam.goal=0; [n,tr]=train(n,xs,ts,xi,ai); w=getwb(n); perf=tr.perf; assert(tr.num_epochs==5 && isequal(tr.trainInd,1:12));",
    );
    near(&e, "w", LM_WEIGHTS, 1e-8);
    near(&e, "perf", LM_PERF, 1e-8);
}
#[test]
fn narnet_sine_closed_loop_converges() {
    let mut e = Engine::default();
    run(
        &mut e,
        "rng(14); t=con2seq(sin((0:119)*.1)); n=narnet(1:2,6); n.divideFcn='dividetrain'; n.trainParam.epochs=120; n.trainParam.min_grad=1e-10; [xs,xi,ai,ts]=preparets(n,{},{},t); n=train(n,xs,ts,xi,ai); [y,xf,af]=n(xs,xi,ai); [c,xic,aic]=closeloop(n,xf,af); yp=c(cell(0,20),xic,aic); err=cell2mat(seq2con(yp))-sin((120:139)*.1); rmse=sqrt(mean(err.^2)); assert(rmse<.05);",
    );
}
#[test]
fn som_plots_are_editable_graphics() {
    let mut e = Engine::default();
    run(&mut e, SOM_DATA);
    run(
        &mut e,
        "f=plotsomhits(n,x); a=findobj(f,'Type','axes'); children=get(a,'Children'); patches=0; total=0; for k=1:numel(children); kind=get(children(k),'Type'); if strcmp(kind,'patch'); patches=patches+1; assert(strcmp(get(children(k),'EdgeColor'),'none') && strcmp(get(children(k),'DisplayName'),'Neuron')); elseif strcmp(kind,'text'); total=total+str2double(get(children(k),'String')); end; end; assert(patches==1 && total==12);",
    );
    let (_, axes) = e.figure.as_ref().unwrap().all_axes().next().unwrap();
    let patch = axes
        .series
        .iter()
        .find(|s| s.kind == matrust::plot::PlotKind::Patch)
        .unwrap();
    assert_eq!(patch.points.len(), 72);
    for i in 0..6 {
        let points = patch
            .points
            .range(i * 12..(i + 1) * 12)
            .unwrap()
            .collect::<Vec<_>>();
        for r in 0..2 {
            let (lo, hi) = points
                .iter()
                .map(|p| p[r])
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                });
            assert!(((lo + hi) / 2. - HEX[i * 2 + r]).abs() < 1e-14);
        }
    }
    run(
        &mut e,
        "f=plotsompos(n,x); a=findobj(f,'Type','axes'); p=get(a,'Children'); data=0; neurons=0; for k=1:numel(p); label=get(p(k),'DisplayName'); if strcmp(label,'Data'); assert(strcmp(get(p(k),'Type'),'scatter') && isequal(get(p(k),'XData'),x(1,:))); data=data+1; elseif strcmp(label,'Neuron'); assert(numel(get(p(k),'XData'))==6); neurons=neurons+1; end; end; assert(data==1 && neurons==1); f=plotsomtop(n); assert(numel(findobj(f,'Type','line','DisplayName','NeuronOutline'))==1); f=plotsomnc(n); assert(numel(findobj(f,'Type','line','DisplayName','Neighbor'))==1); f=plotsomnd(n); assert(numel(findobj(f,'Type','line','DisplayName','Neighbor'))==1); f=plotsomplanes(n); assert(numel(findobj(f,'Type','axes'))==2);",
    );
}

#[test]
fn dynamic_and_som_reject_invalid_syntax_and_limits() {
    let mut e = Engine::default();
    for code in [
        "selforgmap([10000 10000])",
        "con2seq(zeros(1,65537))",
        "selforgmap([2 2],3,0)",
        "narxnet([2 1],1:2)",
        "closeloop(narnet(0:2))",
        "removedelay(timedelaynet(0:2),1)",
        "preparets(narnet(1:3),{},{},{1,2})",
        "narxnet(1:5000,1:2)",
    ] {
        assert!(e.execute(code).is_err(), "{code}");
    }
    run(&mut e, DYNAMIC_DATA);
    run(
        &mut e,
        "c=closeloop(n); [xc,xic,aic,tc]=preparets(c,x,{},t);",
    );
    assert!(
        e.execute("train(c,xc,tc,xic,aic)")
            .unwrap_err()
            .contains("closed-loop")
    );
}

#[test]
fn som_callback_stop_cancellation_and_workspace() {
    use matrust::{
        Matrix,
        engine::nnet::som_train::{self, SomOptions},
    };
    use std::{ops::ControlFlow, sync::atomic::AtomicBool, time::Duration};
    let weights = Matrix::from_column_vec(2, 1, vec![0., 2.]).unwrap();
    let x = Matrix::from_column_vec(1, 3, vec![0., 1., 3.]).unwrap();
    let distances = Matrix::from_column_vec(2, 2, vec![0., 1., 1., 0.]).unwrap();
    let cancel = AtomicBool::new(false);
    let options = || SomOptions {
        epochs: 10,
        cover_steps: 5,
        initial_neighbor: 2.,
        time_limit: Duration::MAX,
        memory_bytes: 1_000_000,
        cancel: &cancel,
    };
    let mut seen = vec![];
    let (_, record) = som_train::train(&weights, &x, &distances, options(), &mut |r| {
        seen.push(r.epoch);
        if r.epoch == 2 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .unwrap();
    assert_eq!(seen, vec![0, 1, 2]);
    assert_eq!(record.stop, "User stop.");
    let mut small = options();
    small.memory_bytes = 1;
    assert!(
        som_train::train(&weights, &x, &distances, small, &mut |_| {
            ControlFlow::Continue(())
        })
        .is_err()
    );
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        som_train::train(&weights, &x, &distances, options(), &mut |_| {
            ControlFlow::Continue(())
        })
        .is_err()
    );
    assert_eq!(weights.data(), &[0., 2.]);
}

#[test]
fn release_benchmark_som_and_dynamic() {
    use matrust::{
        Matrix,
        engine::nnet::som_train::{self, SomOptions},
    };
    use std::{
        ops::ControlFlow,
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };
    let x = Matrix::from_column_vec(
        32,
        2048,
        (0..32 * 2048).map(|i| (i as f64 * 0.13).sin()).collect(),
    )
    .unwrap();
    let w = Matrix::from_column_vec(
        64,
        32,
        (0..64 * 32).map(|i| (i as f64 * 0.31).cos()).collect(),
    )
    .unwrap();
    let d = Matrix::from_column_vec(
        64,
        64,
        (0..64 * 64)
            .map(|i| {
                ((i % 64 % 8) as f64 - (i / 64 % 8) as f64).abs()
                    + ((i % 64 / 8) as f64 - (i / 64 / 8) as f64).abs()
            })
            .collect(),
    )
    .unwrap();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
 let cancel=AtomicBool::new(false);let started=Instant::now();
 let result=som_train::train(&w,&x,&d,SomOptions{epochs:10,cover_steps:5,initial_neighbor:3.,time_limit:Duration::MAX,memory_bytes:64*1024*1024,cancel:&cancel},&mut |_|ControlFlow::Continue(())).unwrap();
 assert_eq!(result.1.epochs.len(),11);
 println!("SOM 64 neurons x 32 features x 2048 samples x 10 epochs, workers={workers}: {:?}",started.elapsed());
 let mut e=Engine::default();run(&mut e,DYNAMIC_DATA);let started=Instant::now();
 for _ in 0..100 {run(&mut e,"[y,xf,af]=n(xs,xi,ai); c=closeloop(n); [xc,xic,aic]=preparets(c,x,{},t); [y,xf,af]=c(xc,xic,aic);");}
 println!("NARX 2 hidden x 4 taps x 20 steps, 100 open+closed simulations, workers={workers}: {:?}",started.elapsed());
 });
    }
}
const TDNN: &[f64] = &[
    -0.04080553972907533,
    -0.04306069907297773,
    -0.044690525380455134,
    -0.04564490222270973,
    -0.04591658626501237,
    -0.045506756144642675,
    -0.04441419669901944,
    -0.042651458664860774,
    -0.040282756689379084,
    -0.03746540004360251,
    -0.03446314918721645,
    -0.03160750575222019,
    -0.0292211708984227,
    -0.027551146416980293,
    -0.02674562275749098,
    -0.026866326295094202,
    -0.027904460815769437,
    -0.029777883545335154,
    -0.0323131120523944,
    -0.03523939890690936,
];
const NAR: &[f64] = &[
    -0.04232178213491899,
    -0.042351654241590014,
    -0.042079535791071584,
    -0.041499175744899044,
    -0.04061802966627609,
    -0.03946180519050259,
    -0.03807932819519997,
    -0.03654516945751369,
    -0.03495739251357401,
    -0.033429308711436116,
    -0.03207681001732065,
    -0.031005005232077117,
    -0.03029786182680143,
    -0.030012379387403337,
    -0.03017622212881179,
    -0.030786556659966723,
    -0.03180867250179803,
    -0.03317507525513093,
    -0.03478764678156845,
];

#[test]
fn tdnn_nar_numpy_forward() {
    let mut e = Engine::default();
    run(&mut e, DYNAMIC_DATA);
    run(
        &mut e,
        "n=timedelaynet(0:2,2); n.inputs{1}.processFcns={}; n.outputs{2}.processFcns={}; [xs,xi,ai,ts]=preparets(n,x,t); n=configure(n,xs,ts); n=setwb(n,(1:n.numWeightElements)'*.03-.3); y=cell2mat(seq2con(n(xs,xi,ai)));",
    );
    near(&e, "y", TDNN, 1e-13);
    run(
        &mut e,
        "n=narnet(1:3,2); n.inputs{1}.processFcns={}; n.outputs{2}.processFcns={}; [xs,xi,ai,ts]=preparets(n,{},{},t); n=configure(n,xs,ts); n=setwb(n,(1:n.numWeightElements)'*.03-.3); y=cell2mat(seq2con(n(xs,xi,ai)));",
    );
    near(&e, "y", NAR, 1e-13);
}
#[test]
fn time_division_keeps_concurrent_samples_together() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x={[1 2],[2 3],[3 4],[4 5],[5 6],[6 7]}; n=timedelaynet(0:1,2); n.trainParam.epochs=0; n.divideFcn='divideind'; n.divideParam.trainInd=[1 3 5]; n.divideParam.valInd=2; n.divideParam.testInd=4; [xs,xi,ai,ts]=preparets(n,x,x); [n,tr]=train(n,xs,ts,xi,ai); assert(isequal(tr.trainInd,[1 3 5]) && tr.valInd==2 && tr.testInd==4); [y,xf,af]=n(xs,xi,ai); assert(isequal(size(y{1}),[1 2])); assert(isequal(xf{1},[6 7]));",
    );
}
const SOM_INIT: &[f64] = &[
    -0.9881770701571752,
    0.035765086303260696,
    1.0597072427636967,
    -1.0597072427636964,
    -0.035765086303260696,
    0.9881770701571752,
    -1.2413446876557446,
    -0.6578513657270826,
    -0.0743580437984207,
    0.07435804379842076,
    0.6578513657270826,
    1.2413446876557446,
];

#[test]
fn som_pca_initialization_numpy() {
    let mut e = Engine::default();
    run(&mut e, SOM_DATA);
    run(
        &mut e,
        "m=configure(selforgmap([3 2],5,2),x); w=getwb(m); z=init(m); assert(isequal(getwb(m),getwb(z)));",
    );
    near(&e, "w", SOM_INIT, 1e-12);
}

#[test]
fn all_dynamic_help_examples_execute() {
    for name in "selforgmap gridtop hextop randtop dist linkdist boxdist mandist narxnet narnet timedelaynet preparets closeloop openloop removedelay tonndata fromnndata con2seq seq2con plotsomtop plotsomnc plotsomnd plotsomplanes plotsomhits plotsompos".split_whitespace() {
 let page=matrust::help::page(name).unwrap();
 for example in page.lines().filter_map(|s|s.strip_prefix("Example:")) {run(&mut Engine::default(),example.trim());}
 }
}

#[test]
fn default_som_plots_fit_renderer_limits() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=[0 1 0 1;0 0 1 1]; n=configure(selforgmap(),x); f=plotsomtop(n); a=findobj(f,'Type','axes'); assert(numel(get(a,'Children'))==3); f=plotsomplanes(n); assert(numel(findobj(f,'Type','axes'))==2); f=plotsomhits(n,x); a=findobj(f,'Type','axes'); assert(numel(get(a,'Children'))==67); assert(isequal(n.plotFcns,{'plotsomtop','plotsomnc','plotsomnd','plotsomplanes','plotsomhits','plotsompos'}));",
    );
}

#[test]
fn conversion_batch_forms_follow_public_docs() {
    let mut e = Engine::default();
    run(
        &mut e,
        "b={[1 3 4 5;1 1 7 4];[7 3 4 4;6 9 4 1]}; s=con2seq(b,2); assert(isequal(size(s),[2 2]) && isequal(s{1,1},[1 3;1 1])); assert(isequal(seq2con(s),b)); assert(isequal(seq2con({1,2,3}),{[1 2 3]})); x=[0 1 0 1;0 0 1 1]; n=configure(selforgmap([2 2]),x); f=plotsompos(n); assert(numel(findobj(f,'Type','axes'))==1);",
    );
}

#[test]
fn concurrent_sample_mismatches_are_errors_before_expansion() {
    // Contract expectations from the step review; no numerical reference needed.
    let mut e = Engine::default();
    for function in ["train", "configure"] {
        for (x, t) in [
            ("{[1 2],[3 4];1,2}", "[1 2 3 4]"),
            ("{[1 2],3;[4 5],[6 7]}", "[1 2 3 4]"),
            ("{[1 2],[3 4];[5 6],[7 8]}", "{1,[2 3 4]}"),
            ("{[1 2],[3 4];[5 6],[7 8]}", "{1,2,3,4}"),
        ] {
            let code = format!("n=narxnet(0,1,1); {function}(n,{x},{t});");
            let err = e.execute(&code).unwrap_err();
            assert!(
                err.contains(&format!(
                    "{function}: inconsistent number of samples (columns)"
                )),
                "{code}: {err}"
            );
        }
    }
    for code in [
        "preparets(narxnet(0,1,1),{[1 2],[3 4]}, {}, {1,2})",
        "preparets(narxnet(0,1,1),{[1 2],3}, {}, {[1 2],[3 4]})",
        "preparets(narxnet(0,1,1),{[1 2],[3 4]}, {}, {[1 2],3})",
        "preparets(narxnet(0,1,1,'closed'),{[1 2],[3 4]}, {}, {1,2})",
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(
            err.contains("preparets: inconsistent number of samples (columns)"),
            "{code}: {err}"
        );
    }
    run(
        &mut e,
        "n=configure(narxnet(0,1,1),{[1 2],[3 4];[5 6],[7 8]},[1 2 3 4]);",
    );
    for (code, function) in [
        (
            "train(n,{[1 2],[3 4];[5 6],[7 8]},[1 2 3 4],{[1 2];1})",
            "train",
        ),
        ("sim(n,{[1 2],[3 4];1,2})", "sim"),
        ("sim(n,{[1 2],3;[5 6],[7 8]})", "sim"),
        ("n({[1 2],[3 4];[5 6],[7 8]},{[1 2];1})", "sim"),
    ] {
        let err = e.execute(code).unwrap_err();
        assert!(
            err.contains(&format!(
                "{function}: inconsistent number of samples (columns)"
            )),
            "{code}: {err}"
        );
    }
    let err = e
        .execute("configure(narxnet(0,1,1,'closed'),{[1 2],[3 4]},{1,2})")
        .unwrap_err();
    assert!(
        err.contains("configure: inconsistent number of samples (columns)"),
        "{err}"
    );
    run(&mut e, "c=closeloop(n);");
    for ai in ["{0;[1 2]}", "{[0 0];1}"] {
        let code = format!("c({{[1 2],[3 4]}},[],{ai})");
        let err = e.execute(&code).unwrap_err();
        assert!(
            err.contains("sim: inconsistent number of samples (columns)"),
            "{code}: {err}"
        );
    }
}

#[test]
fn feedback_and_derived_metadata_setters_reject_changes() {
    let mut e = Engine::default();
    run(
        &mut e,
        "n=narxnet(1:2,1:2,1); n.outputs{2}.feedbackMode='open'; n.outputs{2}.feedbackDelays=1:2;",
    );
    for (assignment, message) in [
        (
            "feedbackMode='closed'",
            "use closeloop(net) or openloop(net)",
        ),
        ("feedbackMode='none'", "use closeloop(net) or openloop(net)"),
        ("feedbackMode='bogus'", "invalid feedbackMode"),
        ("feedbackMode=7", "invalid feedbackMode"),
        ("feedbackMode=[111 112 101 110]", "invalid feedbackMode"),
        ("feedbackDelays=[-1 NaN]", "create the network with narxnet"),
        ("feedbackDelays=[1 3]", "create the network with narxnet"),
    ] {
        let code = format!("n.outputs{{2}}.{assignment};");
        let err = e.execute(&code).unwrap_err();
        assert!(err.contains(message), "{code}: {err}");
        run(
            &mut e,
            "assert(strcmp(n.outputs{2}.feedbackMode,'open')); assert(isequal(n.outputs{2}.feedbackDelays,1:2));",
        );
    }
    run(
        &mut e,
        "c=closeloop(n); c.outputs{2}.feedbackMode='closed'; c.outputs{2}.feedbackDelays=1:2; d=timedelaynet(1:2,1); d.outputs{2}.feedbackMode='none'; d.outputs{2}.feedbackDelays=d.outputs{2}.feedbackDelays; s=selforgmap([2 2]);",
    );
    for (field, value) in [
        ("dimensions", "[3 3]"),
        ("positions", "zeros(2,4)"),
        ("distances", "zeros(4,4)"),
        ("topologyFcn", "'gridtop'"),
        ("distanceFcn", "'dist'"),
    ] {
        run(
            &mut e,
            &format!("s.layers{{1}}.{field}=s.layers{{1}}.{field};"),
        );
        let code = format!("s.layers{{1}}.{field}={value};");
        let err = e.execute(&code).unwrap_err();
        assert!(
            err.contains(field) && err.contains("read-only"),
            "{code}: {err}"
        );
    }
}
