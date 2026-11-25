use matrust::Engine;
use serde_json::Value;

#[test]
fn volume_data_interp3_nearest_ties() {
    let mut e = Engine::default();
    e.execute("V=reshape(1:27,3,3,3); Q=interp3(V,1.5,1.5,1.5,'nearest'); R=interp3(V,1.5,1.5,1.5,'*Nearest');")
        .unwrap();
    assert_eq!(e.variables["Q"].data(), &[14.]);
    assert_eq!(e.variables["R"].data(), &[14.]);
}

#[test]
fn volume_data_interp3_cubic_matches_interpn() {
    let mut e = Engine::default();
    e.execute("A=magic(5).^2; P=interp3(repmat(A,1,1,5),2.3,3.6,3,'cubic'); Q=interpn(A,3.6,2.3,'cubic');")
        .unwrap();
    let p = e.variables["P"].data()[0];
    let q = e.variables["Q"].data()[0];
    assert!((p - q).abs() < 1e-12, "interp3={p}, interpn={q}");
}

#[test]
fn volume_data_scipy_smoothing_and_interpolation() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/volume/data.json")).unwrap();
    let input = fixture["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let mut e = Engine::default();
    e.execute(&format!("V=reshape([{input}],7,6,5); B=smooth3(V,'BOX'); G=smooth3(V,'Gaussian'); L=interp3(V,[3.3 5 2],[2.2 6 0],[4.4 4 2],'linear',-7); N=interp3(V,[3.3 5 2],[2.2 6 0],[4.4 4 2],'nearest',-7); S=interp3(V,[3.3 5 2],[2.2 6 0],[4.4 4 2],'spline',-7);" )).unwrap();
    e.execute("h=1e-40; T=interp3(h*(1:6),h*(1:7),h*(1:5),V,h*[3.3 5 2],h*[2.2 6 0],h*[4.4 4 2],'spline',-7);")
        .unwrap();
    e.execute(
        "[XX,YY,ZZ]=meshgrid(1:3); VV=XX.^2+YY.^2+ZZ.^2; PP=interp3(VV,1.5,1.5,1.5,'spline');",
    )
    .unwrap();
    assert!((e.variables["PP"].data()[0] - fixture["quadratic"].as_f64().unwrap()).abs() < 1e-12);
    for (name, key) in [
        ("B", "box"),
        ("G", "gaussian"),
        ("L", "linear"),
        ("N", "nearest"),
        ("S", "spline"),
        ("T", "spline"),
    ] {
        let expected = fixture[key].as_array().unwrap();
        for (a, b) in e.variables[name].data().iter().zip(expected) {
            assert!((a - b.as_f64().unwrap()).abs() < 1e-12, "{name}: {a} {b}");
        }
    }
}
#[test]
fn volume_data_grid_reduction_and_bounds() {
    let mut e = Engine::default();
    e.execute("V=reshape(1:120,4,6,5); [X,Y,Z]=meshgrid(1:6,1:4,1:5); [A,B,C,R]=reducevolume(X,Y,Z,V,[2 2 2]); [U,W,T,S]=subvolume(X,Y,Z,V,[2 4 NaN NaN 2 3]); K=reducevolume(V,[2 3 1]); D=volumebounds(V); [P,Q,R0]=ndgrid(1:2,1:3,1:4); J=interp3(X,Y,Z,V,2.5,1.5,2.5); I=interp3(V,ones(2,2,2),ones(2,2,2),ones(2,2,2));").unwrap();
    let octave: Value = serde_json::from_str(include_str!("fixtures/volume/octave.json")).unwrap();
    for (name, key) in [("A", "x"), ("B", "y"), ("C", "z"), ("R", "reduced")] {
        let expected = octave[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(e.variables[name].data(), expected);
    }
    assert_eq!(e.array("R").unwrap().dims().as_ref(), &[2, 3, 3]);
    assert_eq!(&e.variables["R"].data()[..6], &[1., 3., 9., 11., 17., 19.]);
    assert_eq!(e.array("S").unwrap().dims().as_ref(), &[4, 3, 2]);
    let scipy: Value = serde_json::from_str(include_str!("fixtures/volume/data.json")).unwrap();
    for (name, expected) in [("S", &scipy["subvolume"]), ("K", &octave["asymmetric"])] {
        let expected = expected
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(e.variables[name].data(), expected);
    }
    assert_eq!(e.variables["D"].data(), &[1., 6., 1., 4., 1., 5., 1., 120.]);
    assert_eq!(e.variables["J"].data(), &[43.5]);
    assert_eq!(e.array("I").unwrap().dims().as_ref(), &[2, 2, 2]);
    assert!(e.variables["I"].data().iter().all(|&x| x == 1.));
    assert_eq!(e.array("P").unwrap().dims().as_ref(), &[2, 3, 4]);
}

#[test]
fn volume_data_reducevolume_preserves_class() {
    let mut e = Engine::default();
    e.execute(
        "V=reshape(uint8(1:27),3,3,3); R=reducevolume(V,2); \
         u=bitshift(uint64(1),53)+uint64(1); W=repmat(u,3,3,3); T=reducevolume(W,2);",
    )
    .unwrap();
    assert_eq!(e.array("R").unwrap().class(), "uint8");
    assert_eq!(e.array("T").unwrap().class(), "uint64");
    e.execute("assert(isequal(R,V(1:2:end,1:2:end,1:2:end))); assert(isequal(T,repmat(u,2,2,2)));")
        .unwrap();
}

#[test]
fn volume_data_subvolume_preserves_class() {
    let mut e = Engine::default();
    e.execute(
        "L=reshape(mod(1:27,2)==1,3,3,3); S=subvolume(L,[2 3 1 2 1 2]); \
         [X,Y,Z]=meshgrid(1:3); X=single(X); V=reshape(int16(-13:13),3,3,3); \
         [A,B,C,T]=subvolume(X,Y,Z,V,[2 3 1 2 1 2]);",
    )
    .unwrap();
    assert_eq!(e.array("S").unwrap().class(), "logical");
    assert_eq!(e.array("A").unwrap().class(), "single");
    assert_eq!(e.array("T").unwrap().class(), "int16");
    e.execute("assert(isequal(S,L(1:2,2:3,1:2))); assert(isequal(T,V(1:2,2:3,1:2)));")
        .unwrap();
}

#[test]
fn volume_data_smooth3_preserves_single() {
    let mut e = Engine::default();
    e.execute("S=smooth3(single(ones(3,3,3)));").unwrap();
    let s = e.array("S").unwrap();
    assert_eq!(s.class(), "single");
    assert_eq!(s.dims().as_ref(), &[3, 3, 3]);
}

#[test]
fn volume_data_interp3_preserves_single() {
    let mut e = Engine::default();
    e.execute("V=single(ones(3,3,3)); Q=ones(2,2,2); S=interp3(V,Q,Q,Q);")
        .unwrap();
    let s = e.array("S").unwrap();
    assert_eq!(s.class(), "single");
    assert_eq!(s.dims().as_ref(), &[2, 2, 2]);
}

#[test]
fn volume_data_flow_and_errors() {
    let mut e = Engine::default();
    e.execute("[X,Y,Z,V]=flow(4); D=size(V); F=flow; [XF,YF,ZF,VF]=flow; T=smooth3(uint8(ones(3,3,3))); L=smooth3(true(3,3,3));")
        .unwrap();
    assert_eq!(e.variables["D"].data(), &[4., 8., 4.]);
    assert_eq!(e.array("F").unwrap().dims().as_ref(), &[25, 50, 25]);
    assert_eq!(e.variables["F"].data(), e.variables["VF"].data());
    let (min, max) = e.variables["F"]
        .data()
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        });
    assert!((min - (-12.06)).abs() < 0.005, "min = {min}");
    assert!((max - 2.48).abs() < 0.005, "max = {max}");
    assert!((min..=max).contains(&-3.));
    assert_eq!(e.array("T").unwrap().dims().as_ref(), &[3, 3, 3]);
    assert_eq!(e.array("L").unwrap().dims().as_ref(), &[3, 3, 3]);
    let fixture: Value = serde_json::from_str(include_str!("fixtures/volume/data.json")).unwrap();
    for (actual, expected) in e.variables["V"]
        .data()
        .iter()
        .zip(fixture["flow"].as_array().unwrap())
    {
        assert!((actual - expected.as_f64().unwrap()).abs() < 1e-12);
    }
    for (actual, expected) in e.variables["F"]
        .data()
        .iter()
        .step_by(1250)
        .zip(fixture["flow_default"].as_array().unwrap())
    {
        assert!((actual - expected.as_f64().unwrap()).abs() < 1e-12);
    }
    for expr in [
        "flow(0)",
        "smooth3(ones(3,3,3),'box',2)",
        "reducevolume(ones(3,3,3),0)",
        "interp3(ones(3,3,3),1,1,1,'bad')",
    ] {
        assert!(e.execute(expr).is_err(), "{expr}");
    }
}

#[test]
#[ignore = "manual release benchmark"]
fn volume_data_benchmark() {
    use std::time::Instant;
    let mut e = Engine::default();
    let start = Instant::now();
    e.execute("[X,Y,Z,V]=flow(40); S=smooth3(V,'gaussian'); R=reducevolume(S,2); U=subvolume(S,[5 25 5 25 5 25]); B=volumebounds(S); Q=interp3(S,[3.1 4.5 6.3],[2.1 5.5 7.2],[2.2 3.3 4.4]); C=interp3(S,[3.1 4.5 6.3],[2.1 5.5 7.2],[2.2 3.3 4.4],'cubic');").unwrap();
    eprintln!(
        "volume benchmark {:?}; flow grid 40x80x40, 3 linear and 3 cubic query points",
        start.elapsed()
    );
}
