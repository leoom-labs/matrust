use matrust::{Engine, Matrix};

fn run(code: &str) -> Engine {
    let mut engine = Engine::default();
    engine
        .execute(code)
        .unwrap_or_else(|e| panic!("{code}: {e}"));
    engine
}
fn get(engine: &Engine, name: &str) -> Matrix {
    engine.variables[name].clone()
}
fn error(code: &str) -> String {
    Engine::default().execute(code).unwrap_err()
}
fn close(a: &Matrix, shape: (usize, usize), expected: &[f64], tol: f64) {
    assert_eq!((a.rows(), a.cols()), shape, "shape");
    assert_eq!(a.len(), expected.len());
    for (i, (&x, &y)) in a.data().iter().zip(expected).enumerate() {
        if y.is_nan() {
            assert!(x.is_nan(), "element {i}: {x} vs NaN");
        } else if y.is_infinite() {
            assert_eq!(x, y, "element {i}");
        } else {
            assert!((x - y).abs() <= tol, "element {i}: {x} vs {y}");
        }
    }
}
fn exact(a: &Matrix, shape: (usize, usize), expected: &[f64]) {
    close(a, shape, expected, 0.);
}
/// Largest element magnitude of a real or complex variable.
fn max_abs(engine: &Engine, name: &str) -> f64 {
    match engine.complex_variables.get(name) {
        Some(z) => z.data().iter().map(|v| v.norm()).fold(0., f64::max),
        None => get(engine, name)
            .data()
            .iter()
            .map(|v| v.abs())
            .fold(0., f64::max),
    }
}

#[test]
fn mod_and_rem_sign_table() {
    let e = run(
        "x=[5 -5 5 -5 5 -5 0 7 -7 7 5.5];y=[3 3 -3 -3 0 0 3 Inf Inf -Inf 2];m=mod(x,y);r=rem(x,y);",
    );
    let nan = f64::NAN;
    exact(
        &get(&e, "m"),
        (1, 11),
        &[2., 1., -1., -2., 5., -5., 0., nan, nan, nan, 1.5],
    );
    exact(
        &get(&e, "r"),
        (1, 11),
        &[2., -2., 2., -2., nan, nan, 0., nan, nan, nan, 1.5],
    );
    let e = run("a=mod(-7,3);b=rem(-7,3);c=mod(5,0);d=rem(5,0);f=mod(Inf,3);g=rem(Inf,3);");
    assert_eq!(get(&e, "a").data(), &[2.]);
    assert_eq!(get(&e, "b").data(), &[-1.]);
    assert_eq!(get(&e, "c").data(), &[5.]);
    assert!(get(&e, "d").data()[0].is_nan());
    assert!(get(&e, "f").data()[0].is_nan());
    assert!(get(&e, "g").data()[0].is_nan());
    let e = run("a=mod([1 2;3 4],[2 2;2 2]);b=mod(10,[3 4 5]);");
    exact(&get(&e, "a"), (2, 2), &[1., 1., 0., 0.]);
    exact(&get(&e, "b"), (1, 3), &[1., 2., 0.]);
    assert!(error("mod([1 2 3],[1 2])").contains("dimensions"));
}

#[test]
fn mod_exact_multiple_zero_sign_reference() {
    // Reference m4/m5/m6/m15: reciprocals distinguish the zero signs.
    let e = run(
        "m=1./[mod(-3,3) mod(-6,3) mod(6,-3) mod(3,-3) mod(-3,-3) mod(-0,3) mod(0,-3)];r=1./[rem(-6,3) rem(6,-3) rem(-3,-3) rem(-0,3)];s=1./[mod(-0.3,0.1) mod(0.3,-0.1) rem(-0.3,0.1)];",
    );
    let (pos, neg) = (f64::INFINITY, f64::NEG_INFINITY);
    exact(&get(&e, "m"), (1, 7), &[pos, pos, neg, neg, neg, neg, pos]);
    exact(&get(&e, "r"), (1, 4), &[neg, pos, neg, neg]);
    // Round-off compensation remains +0 even for a negative divisor.
    exact(&get(&e, "s"), (1, 3), &[pos, pos, pos]);
}

#[test]
fn mod_rem_infinite_divisor_reference() {
    // Reference m11/m12: both signs of infinity produce NaN.
    let e =
        run("x=[-7 7 0 -0 NaN Inf -Inf];a=mod(x,Inf);b=mod(x,-Inf);c=rem(x,Inf);d=rem(x,-Inf);");
    for name in ["a", "b", "c", "d"] {
        exact(&get(&e, name), (1, 7), &[f64::NAN; 7]);
    }
}

#[test]
fn mod_and_rem_compensate_round_off() {
    // 0.3/0.1 = 2.9999999999999996: within round-off of 3, so exactly 0.
    let e = run(
        "a=mod(0.3,0.1);b=rem(0.3,0.1);c=mod(-0.3,0.1);d=rem(0.3,-0.1);f=mod(1,0.1);g=mod(1+eps,1);h=mod(0.35,0.1);k=rem(-7.5,2);",
    );
    for name in ["a", "b", "c", "d", "f"] {
        assert_eq!(get(&e, name).data(), &[0.], "{name}");
    }
    // Integer divisors keep the exact remainder.
    assert_eq!(get(&e, "g").data(), &[f64::EPSILON]);
    close(&get(&e, "h"), (1, 1), &[0.05], 1e-15);
    assert_eq!(get(&e, "k").data(), &[-1.5]);
    // Reference m1/m2/m3: retain the exact remainder, not x-floor(x/y)*y.
    let e = run("a=mod(1e16,3);b=rem(1e16,3);c=mod(1,0.3);");
    exact(&get(&e, "a"), (1, 1), &[1.]);
    exact(&get(&e, "b"), (1, 1), &[1.]);
    exact(&get(&e, "c"), (1, 1), &[0.10000000000000003]);
}

#[test]
fn fix_sign_hypot_atan2() {
    let e = run(
        "a=fix([-2.7 2.7 -0.5 3]);b=sign([-4 0 9 NaN]);c=hypot(3,4);d=atan2(1,1);f=atan2([0 1],[-1 0]);",
    );
    exact(&get(&e, "a"), (1, 4), &[-2., 2., 0., 3.]);
    exact(&get(&e, "b"), (1, 4), &[-1., 0., 1., f64::NAN]);
    close(&get(&e, "c"), (1, 1), &[5.], 0.);
    close(&get(&e, "d"), (1, 1), &[std::f64::consts::FRAC_PI_4], 1e-16);
    close(
        &get(&e, "f"),
        (1, 2),
        &[std::f64::consts::PI, std::f64::consts::FRAC_PI_2],
        1e-16,
    );
    let e = run("z=sign(3+4i);w=fix(2.7-3.2i);");
    let z = &e.complex_variables["z"];
    assert!((z.data()[0].re - 0.6).abs() < 1e-15 && (z.data()[0].im - 0.8).abs() < 1e-15);
    let w = &e.complex_variables["w"];
    assert_eq!((w.data()[0].re, w.data()[0].im), (2., -3.));
}

#[test]
fn prod_dims_and_empty() {
    let e = run(
        "a=prod([1 2 3 4]);b=prod([1 2;3 4]);c=prod([1 2;3 4],2);d=prod([]);f=prod(zeros(0,3));g=prod(zeros(3,0));h=prod([1 2;3 4],3);k=prod([true true false]);",
    );
    exact(&get(&e, "a"), (1, 1), &[24.]);
    exact(&get(&e, "b"), (1, 2), &[3., 8.]);
    exact(&get(&e, "c"), (2, 1), &[2., 12.]);
    exact(&get(&e, "d"), (1, 1), &[1.]);
    exact(&get(&e, "f"), (1, 3), &[1., 1., 1.]);
    assert_eq!((get(&e, "g").rows(), get(&e, "g").cols()), (1, 0));
    exact(&get(&e, "h"), (2, 2), &[1., 3., 2., 4.]);
    exact(&get(&e, "k"), (1, 1), &[0.]);
    // The reference narrows a complex product whose imaginary part is zero to real (cplx2 corpus).
    let e = run("p=prod([1+i 1-i]);");
    exact(&get(&e, "p"), (1, 1), &[2.]);
}

#[test]
fn dot_cross_kron() {
    let e = run(
        "a=dot([1 2 3],[4 5 6]);b=dot([1 2;3 4],[5 6;7 8]);c=dot([1 2;3 4],[5 6;7 8],2);d=dot([1;2;3],[1 2 3]);",
    );
    exact(&get(&e, "a"), (1, 1), &[32.]);
    exact(&get(&e, "b"), (1, 2), &[26., 44.]);
    exact(&get(&e, "c"), (2, 1), &[17., 53.]);
    exact(&get(&e, "d"), (1, 1), &[14.]);
    // conj(a).*b summed: real 6, narrowed to real as in the reference (cplx2 corpus).
    let e = run("z=dot([1+i 2],[1+i 2]);");
    exact(&get(&e, "z"), (1, 1), &[6.]);
    let e = run(
        "a=cross([1 0 0],[0 1 0]);b=cross([1;2;3],[4;5;6]);c=cross([1 0 0;0 1 0],[0 1 0;0 0 1],2);d=cross([1 0;0 1;0 0],[0 0;1 0;0 1]);",
    );
    exact(&get(&e, "a"), (1, 3), &[0., 0., 1.]);
    exact(&get(&e, "b"), (3, 1), &[-3., 6., -3.]);
    exact(&get(&e, "c"), (2, 3), &[0., 1., 0., 0., 1., 0.]);
    exact(&get(&e, "d"), (3, 2), &[0., 0., 1., 1., 0., 0.]);
    assert!(error("cross([1 2],[3 4])").contains("length 3"));
    let e = run("k=kron([1 2;3 4],[1 1]);j=kron([1 2],[1;1]);");
    exact(&get(&e, "k"), (2, 4), &[1., 3., 1., 3., 2., 4., 2., 4.]);
    exact(&get(&e, "j"), (2, 2), &[1., 1., 2., 2.]);
    // Reference arity messages.
    assert!(error("kron(1)").ends_with("Not enough input arguments."));
    assert!(error("kron(1,2,3)").ends_with("Too many input arguments."));
}

#[test]
fn cross_vector_orientations_reference() {
    // Reference cr1: mixed row/column inputs return [0 0 1], size 1x3.
    let e = run(
        "rr=cross([1 0 0],[0 1 0]);rc=cross([1 0 0],[0;1;0]);cr=cross([1;0;0],[0 1 0]);cc=cross([1;0;0],[0;1;0]);",
    );
    for name in ["rr", "rc", "cr"] {
        exact(&get(&e, name), (1, 3), &[0., 0., 1.]);
    }
    exact(&get(&e, "cc"), (3, 1), &[0., 0., 1.]);
    let e = run("z=cross([1i;0;0],[0 1 0]);");
    let z = &e.complex_variables["z"];
    assert_eq!((z.rows(), z.cols()), (1, 3));
    assert_eq!((z.data()[2].re, z.data()[2].im), (0., 1.));
    // The special vector rule applies only when dim is omitted.
    assert!(error("cross([1 0 0],[0;1;0],1)").contains("same size"));
    assert!(error("cross([1;0;0],[0 1 0],2)").contains("same size"));
}

#[test]
fn circshift_trace_logspace() {
    let e = run(
        "a=circshift([1 2 3 4 5],2,2);b=circshift([1 2 3 4 5],-1,2);c=circshift([1 2;3 4;5 6],1);d=circshift([1 2;3 4;5 6],1,2);f=circshift([1 2;3 4;5 6],[1 1]);g=circshift([true false false],1,2);h=circshift(1:5,0);",
    );
    exact(&get(&e, "a"), (1, 5), &[4., 5., 1., 2., 3.]);
    exact(&get(&e, "b"), (1, 5), &[2., 3., 4., 5., 1.]);
    exact(&get(&e, "c"), (3, 2), &[5., 1., 3., 6., 2., 4.]);
    exact(&get(&e, "d"), (3, 2), &[2., 4., 6., 1., 3., 5.]);
    exact(&get(&e, "f"), (3, 2), &[6., 2., 4., 5., 1., 3.]);
    assert_eq!(e.logical_variables["g"].data(), &[0, 1, 0]);
    exact(&get(&e, "h"), (1, 5), &[1., 2., 3., 4., 5.]);
    assert!(error("circshift([1 2 3],1.5)").contains("integers"));
    // Reference behavior: a scalar shift without dim shifts dimension 1, even for rows.
    let e = run(
        "a=circshift([1 2 3],1);b=circshift([1 2 3],1,2);c=circshift([1;2;3],1);d=circshift([1 2 3],[0 1]);",
    );
    exact(&get(&e, "a"), (1, 3), &[1., 2., 3.]);
    exact(&get(&e, "b"), (1, 3), &[3., 1., 2.]);
    exact(&get(&e, "c"), (3, 1), &[3., 1., 2.]);
    exact(&get(&e, "d"), (1, 3), &[3., 1., 2.]);
    // Huge shifts wrap exactly (1e20 = 0 mod 5, 2 mod 7) instead of saturating.
    let e = run(
        "a=circshift(1:5,1e20,2);b=circshift(1:5,-1e20,2);c=circshift(1:7,1e20,2);d=circshift((1:7)',[1e20 0]);",
    );
    exact(&get(&e, "a"), (1, 5), &[1., 2., 3., 4., 5.]);
    exact(&get(&e, "b"), (1, 5), &[1., 2., 3., 4., 5.]);
    exact(&get(&e, "c"), (1, 7), &[6., 7., 1., 2., 3., 4., 5.]);
    exact(&get(&e, "d"), (7, 1), &[6., 7., 1., 2., 3., 4., 5.]);
    let e = run("t=trace([1 2;3 4]);l=logspace(0,2,3);m=logspace(1,2,1);n=size(logspace(0,1));");
    exact(&get(&e, "t"), (1, 1), &[5.]);
    close(&get(&e, "l"), (1, 3), &[1., 10., 100.], 1e-13);
    exact(&get(&e, "m"), (1, 1), &[100.]);
    exact(&get(&e, "n"), (1, 2), &[1., 50.]);
    assert!(error("trace([1 2 3])").contains("square"));
}

#[test]
fn meshgrid_and_ndgrid_shapes() {
    let e = run(
        "[X,Y]=meshgrid(1:3,[10 20]);[A,B]=meshgrid(1:2);[P,Q]=ndgrid(1:3,[10 20]);n=ndgrid(1:3);",
    );
    exact(&get(&e, "X"), (2, 3), &[1., 1., 2., 2., 3., 3.]);
    exact(&get(&e, "Y"), (2, 3), &[10., 20., 10., 20., 10., 20.]);
    exact(&get(&e, "A"), (2, 2), &[1., 1., 2., 2.]);
    exact(&get(&e, "B"), (2, 2), &[1., 2., 1., 2.]);
    exact(&get(&e, "P"), (3, 2), &[1., 2., 3., 1., 2., 3.]);
    exact(&get(&e, "Q"), (3, 2), &[10., 10., 10., 20., 20., 20.]);
    exact(&get(&e, "n"), (3, 1), &[1., 2., 3.]);
    assert!(error("[X,Y,Z]=meshgrid(1:2,1:2,1:2)").contains("not supported yet"));
}

#[test]
fn eig_symmetric_and_general() {
    let e = run("A=[2 -1 0;-1 2 -1;0 -1 2];l=eig(A);[V,D]=eig(A);r=A*V-V*D;o=V'*V;");
    let s2 = 2f64.sqrt();
    close(&get(&e, "l"), (3, 1), &[2. - s2, 2., 2. + s2], 1e-13);
    assert!(get(&e, "r").data().iter().all(|v| v.abs() < 1e-13));
    close(
        &get(&e, "o"),
        (3, 3),
        &[1., 0., 0., 0., 1., 0., 0., 0., 1.],
        1e-13,
    );
    close(
        &get(&e, "D"),
        (3, 3),
        &[2. - s2, 0., 0., 0., 2., 0., 0., 0., 2. + s2],
        1e-13,
    );
    let e = run("a=eig([2 0;0 1]);b=eig([4 1;2 3]);c=eig([0 1;-1 0]);");
    exact(&get(&e, "a"), (2, 1), &[1., 2.]);
    let mut b = get(&e, "b").data().to_vec();
    b.sort_by(f64::total_cmp);
    close(
        &Matrix::from_column_slice(2, 1, &b).unwrap(),
        (2, 1),
        &[2., 5.],
        1e-13,
    );
    let c = &e.complex_variables["c"];
    assert_eq!((c.data()[0].re, c.data()[0].im), (0., 1.));
    assert_eq!((c.data()[1].re, c.data()[1].im), (0., -1.));
    let e = run("A=[1 2;3 4];[V,D]=eig(A);r=A*V-V*D;");
    assert!(max_abs(&e, "r") < 1e-14);
    assert!(error("eig([1 NaN;2 3])").contains("NaN"));
}

#[test]
fn eig_complex_hermitian() {
    let e = run("A=[2 1i;-1i 2];l=eig(A);[V,D]=eig(A);r=A*V-V*D;o=V'*V-eye(2);");
    close(&get(&e, "l"), (2, 1), &[1., 3.], 1e-14);
    close(&get(&e, "D"), (2, 2), &[1., 0., 0., 3.], 1e-14);
    let v = &e.complex_variables["V"];
    assert_eq!((v.rows(), v.cols()), (2, 2));
    for c in 0..2 {
        let norm: f64 = (0..2).map(|r| v.data()[r + 2 * c].norm_sqr()).sum();
        assert!((norm - 1.).abs() < 1e-14, "column {c} norm {norm}");
    }
    assert!(max_abs(&e, "r") < 1e-14);
    assert!(max_abs(&e, "o") < 1e-14);
    let e = run(
        "A=[4 1-1i 0;1+1i 3 2i;0 -2i 1];l=eig(A);[V,D]=eig(A);r=A*V-V*D;o=V'*V-eye(3);t=sum(l);",
    );
    let l = get(&e, "l");
    assert!(l.data().windows(2).all(|w| w[0] <= w[1]));
    close(&get(&e, "t"), (1, 1), &[8.], 1e-13);
    assert!(max_abs(&e, "r") < 1e-13);
    assert!(max_abs(&e, "o") < 1e-13);
    // Complex non-Hermitian matrices (see tests/fn_linalg3.rs for exact output).
    let e = run("A=[1 1i;1i 1];[V,D]=eig(A);r=A*V-V*D;B=[1 2i;3 4];[W,E]=eig(B);q=B*W-W*E;");
    assert!(max_abs(&e, "r") < 1e-14);
    assert!(max_abs(&e, "q") < 1e-13);
}

#[test]
fn svd_values_and_reconstruction() {
    let e = run("s=svd([3 0;0 4]);t=svd([1 2;3 4;5 6]);");
    close(&get(&e, "s"), (2, 1), &[4., 3.], 1e-14);
    close(
        &get(&e, "t"),
        (2, 1),
        &[9.525518091565107, 0.5143005806586446],
        1e-13,
    );
    let e = run(
        "A=[1 2;3 4;5 6];[U,S,V]=svd(A);r=U*S*V'-A;[u,s,v]=svd(A,'econ');q=u*s*v'-A;[w,x,y]=svd(A,0);B=A';[P,Q,R]=svd(B);c=P*Q*R'-B;[a,b,d]=svd(B,0);",
    );
    assert_eq!((get(&e, "U").rows(), get(&e, "U").cols()), (3, 3));
    assert_eq!((get(&e, "S").rows(), get(&e, "S").cols()), (3, 2));
    assert!(get(&e, "r").data().iter().all(|v| v.abs() < 1e-13));
    assert_eq!((get(&e, "u").rows(), get(&e, "u").cols()), (3, 2));
    assert_eq!((get(&e, "s").rows(), get(&e, "s").cols()), (2, 2));
    assert!(get(&e, "q").data().iter().all(|v| v.abs() < 1e-13));
    assert_eq!((get(&e, "w").rows(), get(&e, "w").cols()), (3, 2));
    assert_eq!((get(&e, "P").rows(), get(&e, "P").cols()), (2, 2));
    assert_eq!((get(&e, "R").rows(), get(&e, "R").cols()), (3, 3));
    assert!(get(&e, "c").data().iter().all(|v| v.abs() < 1e-13));
    assert_eq!((get(&e, "d").rows(), get(&e, "d").cols()), (3, 3));
    let e = run("[U,S,V]=svd([1 2;3 4;5 6]);o=U'*U;");
    close(
        &get(&e, "o"),
        (3, 3),
        &[1., 0., 0., 0., 1., 0., 0., 0., 1.],
        1e-13,
    );
    assert!(error("[U,S]=svd([1 2;3 4])").contains("outputs"));
}

#[test]
fn svd_output_sizes_are_checked_up_front() {
    // Full U would be 4097x4097: rejected by the element limit before any work.
    let start = std::time::Instant::now();
    assert!(error("[U,S,V]=svd(ones(4097,1))").contains("element limit"));
    assert!(start.elapsed().as_secs_f64() < 5.);
    // Economy outputs are 6000x1, 1x1, 1x1 and fit easily.
    let e =
        run("[U,S,V]=svd(ones(6000,1),'econ');[P,Q,R]=svd(ones(6000,1),0);r=U*S*V'-ones(6000,1);");
    assert_eq!((get(&e, "U").rows(), get(&e, "U").cols()), (6000, 1));
    close(&get(&e, "S"), (1, 1), &[6000f64.sqrt()], 1e-10);
    assert_eq!((get(&e, "V").rows(), get(&e, "V").cols()), (1, 1));
    assert_eq!((get(&e, "P").rows(), get(&e, "P").cols()), (6000, 1));
    // nalgebra's thin U of a 6000x1 vector is off by ~2e-12; the backward
    // error bound is about m*eps*norm(A) = 1e-10.
    assert!(max_abs(&e, "r") < 1e-10);
    // Full form of a tall vector: the completed U stays orthogonal.
    let e = run("A=ones(300,1);[U,S,V]=svd(A);o=U'*U-eye(300);r=U*S*V'-A;");
    assert_eq!((get(&e, "U").rows(), get(&e, "U").cols()), (300, 300));
    assert_eq!((get(&e, "S").rows(), get(&e, "S").cols()), (300, 1));
    assert!(max_abs(&e, "o") < 1e-13);
    assert!(max_abs(&e, "r") < 1e-13);
}

#[test]
fn rank_of_rank_deficient_matrices() {
    let e = run(
        "a=rank([1 2 3;4 5 6;7 8 9]);b=rank(eye(4));c=rank(zeros(3));d=rank([1 2;2 4.0000001],1e-3);f=rank([]);g=rank([1 2 3]);",
    );
    exact(&get(&e, "a"), (1, 1), &[2.]);
    exact(&get(&e, "b"), (1, 1), &[4.]);
    exact(&get(&e, "c"), (1, 1), &[0.]);
    exact(&get(&e, "d"), (1, 1), &[1.]);
    exact(&get(&e, "f"), (1, 1), &[0.]);
    exact(&get(&e, "g"), (1, 1), &[1.]);
}

#[test]
fn polyfit_exact_cubic_and_noisy_residual() {
    let e = run("x=-2:0.5:3;y=2*x.^3-x.^2+4*x-7;p=polyfit(x,y,3);q=polyfit([1 2 3],[2 4 6],1);");
    close(&get(&e, "p"), (1, 4), &[2., -1., 4., -7.], 1e-9);
    close(&get(&e, "q"), (1, 2), &[2., 0.], 1e-12);
    // Deterministic pseudo-noise: the least-squares residual is orthogonal to the basis.
    let e =
        run("x=(0:19)';y=3*x+1+sin(7*x);p=polyfit(x,y,1);r=y-(p(1)*x+p(2));a=sum(r);b=sum(r.*x);");
    assert!(get(&e, "a").data()[0].abs() < 1e-10);
    assert!(get(&e, "b").data()[0].abs() < 1e-9);
    // Years as x: badly conditioned but solvable; check the fitted values.
    let e = run(
        "x=1990:2000;t=x-1995;y=2*t.^3-t.^2+4*t-7;p=polyfit(x,y,3);r=polyval(p,x)-y;m=max(abs(r));",
    );
    assert_eq!(get(&e, "p").data().len(), 4);
    assert!(get(&e, "m").data()[0] < 1e-4 * 250.);
    // Repeated x with fewer distinct x than terms: 1x(n+1) result, no error.
    let e = run("p=polyfit([1 1 1],[1 2 3],1);r=polyval(p,1);");
    assert_eq!((get(&e, "p").rows(), get(&e, "p").cols()), (1, 2));
    assert!((get(&e, "r").data()[0] - 2.).abs() < 1e-12);
    assert!(error("[p,S]=polyfit([1 2 3],[1 2 3],1)").contains("not supported yet"));
}

#[test]
fn polyfit_underdetermined_basic_solution_reference() {
    // Reference pf6: a basic solution, with dependent coefficients zero.
    let e = run("p=polyfit([1 2],[1 2],3);r=polyval(p,[1 2]);");
    close(
        &get(&e, "p"),
        (1, 4),
        &[0.14285714285714285, 0., 0., 0.857_142_857_142_857_4],
        1e-14,
    );
    assert_eq!(&get(&e, "p").data()[1..3], &[0., 0.]);
    close(&get(&e, "r"), (1, 2), &[1., 2.], 1e-14);
    // One point exercises a pivot beyond the short R diagonal.
    let e = run("p=polyfit(0,5,3);q=polyfit([0 0],[1 3],3);");
    exact(&get(&e, "p"), (1, 4), &[0., 0., 0., 5.]);
    close(&get(&e, "q"), (1, 4), &[0., 0., 0., 2.], 1e-14);
}

#[test]
fn rng_zero_reproduces_matlab_twister() {
    let e = run("rng(0);r=rand(1,3);");
    exact(
        &get(&e, "r"),
        (1, 3),
        &[0.8147236863931789, 0.9057919370756192, 0.12698681629350606],
    );
    let e = run("rng('default');a=rand;");
    assert_eq!(get(&e, "a").data()[0], 0.8147236863931789);
    // A fresh session starts from the default stream, like MATLAB.
    assert_eq!(get(&run("a=rand;"), "a").data()[0], 0.8147236863931789);
    let e = run("rng(0);r=randi(10,1,5);");
    exact(&get(&e, "r"), (1, 5), &[9., 10., 2., 10., 7.]);
}

#[test]
fn rng_seeds_are_reproducible() {
    let e = run("rng(1);a=rand(2,3);rng(1);b=rand([2 3]);c=randn(3);rng(1);d=rand;");
    assert_eq!(get(&e, "a").data(), get(&e, "b").data());
    assert_eq!(get(&e, "d").data()[0], get(&e, "a").data()[0]);
    // numpy RandomState(1).random_sample() shares the init_genrand(1) stream.
    assert_eq!(get(&e, "d").data()[0], 0.417022004702574);
    assert_eq!((get(&e, "c").rows(), get(&e, "c").cols()), (3, 3));
    let e = run("rng('shuffle');a=rand(1,2);");
    assert!(get(&e, "a").data().iter().all(|v| *v > 0. && *v < 1.));
    assert!(error("rng(-1)").contains("seed"));
    assert_eq!(
        run("r=rand(2,'single');").array("r").unwrap().class(),
        "single"
    );
    assert!(error("rand(2,'int8')").contains("does not support class 'int8'"));
}

#[test]
fn randn_tracks_matlab_and_is_standard_normal() {
    let e = run("rng(0);r=randn(1,6);");
    close(
        &get(&e, "r"),
        (1, 6),
        &[
            0.5376671395461,
            1.8338850145950865,
            -2.258_846_861_003_648,
            0.8622,
            0.3188,
            -1.3077,
        ],
        // Nine-digit agreement on the fast path; see the module docs.
        1e-3,
    );
    let e = run(
        "rng(5);x=randn(1,200000);m=sum(x)/200000;v=sum((x-m).^2)/199999;k=sum(abs(x)>3)/200000;",
    );
    assert!(get(&e, "m").data()[0].abs() < 0.01);
    assert!((get(&e, "v").data()[0] - 1.).abs() < 0.02);
    assert!((get(&e, "k").data()[0] - 0.0027).abs() < 0.0007);
}

#[test]
fn randi_ranges_and_sizes() {
    let e = run("rng(3);a=randi([5 8],1,1000);b=randi(3,2);c=size(rand(0));");
    assert!(
        get(&e, "a")
            .data()
            .iter()
            .all(|v| (5. ..=8.).contains(v) && v.fract() == 0.)
    );
    assert_eq!((get(&e, "b").rows(), get(&e, "b").cols()), (2, 2));
    exact(&get(&e, "c"), (1, 2), &[0., 0.]);
    assert!(error("randi(0)").contains("upper limit"));
    assert!(error("rand(1e5)").contains("limit"));
}

#[test]
fn tic_toc_are_monotone() {
    let mut engine = Engine::default();
    let out = engine.execute("tic;t1=toc;t2=toc;").unwrap();
    assert!(out.output.is_empty());
    let (t1, t2) = (get(&engine, "t1").data()[0], get(&engine, "t2").data()[0]);
    assert!(t1 >= 0. && t2 >= t1);
    let out = engine.execute("tic;toc").unwrap();
    assert!(!out.output.is_empty());
    let line = &out.output[0];
    assert!(
        line.starts_with("Elapsed time is ") && line.ends_with(" seconds."),
        "{line}"
    );
    let digits = line
        .trim_start_matches("Elapsed time is ")
        .trim_end_matches(" seconds.");
    assert_eq!(digits.split('.').nth(1).unwrap().len(), 6);
    engine.execute("id=tic;a=toc(id);b=toc(id);").unwrap();
    assert!(get(&engine, "id").data()[0] > 0.);
    assert!(get(&engine, "b").data()[0] >= get(&engine, "a").data()[0]);
    assert!(error("toc").contains("tic"));
    // Reference toc2: tic with an output does not start the implicit timer.
    assert!(error("t=tic;toc").contains("tic"));
}

/// Focused release timings: `cargo test --release --test math_functions
/// round3_path_timings -- --ignored --nocapture` (RAYON_NUM_THREADS=1 or 20).
#[test]
#[ignore]
fn round3_path_timings() {
    println!("Rayon workers: {}", rayon::current_num_threads());
    for (setup, code, repetitions) in [
        ("x=-500000:499999;", "r=mod(x,3);", 5),
        ("x=-500000:499999;", "r=mod(x,Inf);", 5),
        ("x=-500000:499999;", "r=rem(x,-Inf);", 5),
        ("a=[1 0 0];b=[0;1;0];", "r=cross(a,b);", 10000),
        ("x=[1 2];y=[1 2];", "r=polyfit(x,y,3);", 10000),
    ] {
        let mut engine = run(setup);
        engine.execute(code).unwrap();
        let mut samples = Vec::new();
        for _ in 0..5 {
            let start = std::time::Instant::now();
            for _ in 0..repetitions {
                engine.execute(code).unwrap();
                std::hint::black_box(&engine.variables["r"]);
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / repetitions as f64);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{code} {repetitions} repetitions/sample, 5 samples: median {:.3} us/call ({:.3}..{:.3})",
            samples[2], samples[0], samples[4]
        );
    }
}

/// Release timings for the report: `cargo test --release --test math_functions
/// timing_report -- --ignored --nocapture` (set RAYON_NUM_THREADS=1 for one worker).
#[test]
#[ignore]
fn timing_report() {
    let mut engine = Engine::default();
    for code in [
        "x=rand(1,10000000);",
        "x=randn(1,10000000);",
        "A=rand(500);S=A+A';",
        "s=svd(A);",
        "[U,T,V]=svd(A);",
        "l=eig(S);",
        "[V,D]=eig(S);",
        "g=eig(A);",
        "H=S+1i*(A-A');[W,E]=eig(H);",
    ] {
        let start = std::time::Instant::now();
        engine.execute(code).unwrap();
        println!("{code:<24} {:>9.1} ms", start.elapsed().as_secs_f64() * 1e3);
    }
}
