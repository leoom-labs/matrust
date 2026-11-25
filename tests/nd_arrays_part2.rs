//! N-D arrays part 2: operators, reductions with options, shape functions, complex/char
//! writes, A(idx) shapes, cross-class isequal, chained reads, MAT checks and empty display.
//! Expected values: tests/fixtures/nd/part2.json from tool/refs/nd/part2.py (NumPy, MATLAB
//! ordering applied explicitly). Tolerances are stated per check; "exact" means bitwise
//! equal f64 values (integer-valued data or a single correctly rounded operation).
use matrust::{Array, Engine};
use serde_json::Value as J;

fn fixture() -> J {
    serde_json::from_str(include_str!("fixtures/nd/part2.json")).unwrap()
}

fn num(v: &J) -> f64 {
    if v.as_str() == Some("NaN") {
        f64::NAN
    } else {
        v.as_f64().unwrap()
    }
}

/// MATLAB drops trailing singleton dimensions beyond the second.
fn matlab_shape(shape: &[usize]) -> Vec<usize> {
    let mut s = shape.to_vec();
    while s.len() > 2 && s.last() == Some(&1) {
        s.pop();
    }
    while s.len() < 2 {
        s.push(1);
    }
    s
}

fn reals(a: &Array) -> Vec<f64> {
    match a {
        Array::Numeric(m) => m.data().to_vec(),
        Array::Logical(m) => m.data().iter().map(|&x| f64::from(x)).collect(),
        Array::Typed(t) => (0..t.len()).map(|i| t.real(i)).collect(),
        Array::Character(c) => c.units().iter().map(|&x| f64::from(x)).collect(),
        other => panic!("expected a real array, got {}", other.class()),
    }
}

fn same(a: f64, b: f64, tol: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a == b || (a - b).abs() <= tol * b.abs().max(1.)
}

/// Compare variable `name` with fixture entry `key` (shape and values, relative `tol`).
fn check(e: &Engine, name: &str, reference: &J, key: &str, tol: f64) {
    let r = &reference[key];
    let shape: Vec<usize> = r["shape"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    let a = e.array(name).unwrap_or_else(|| panic!("{name} missing"));
    assert_eq!(a.dims().as_ref(), matlab_shape(&shape), "{name} vs {key}");
    if let Some(re) = r.get("re") {
        let Array::Complex(z) = a else {
            panic!("{name} must be complex")
        };
        let im = r["im"].as_array().unwrap();
        for (i, (x, (p, q))) in z
            .data()
            .iter()
            .zip(re.as_array().unwrap().iter().zip(im))
            .enumerate()
        {
            assert!(
                same(x.re, num(p), tol) && same(x.im, num(q), tol),
                "{name}[{i}] = {x} vs {key}"
            );
        }
        return;
    }
    let expected: Vec<f64> = r["flat"].as_array().unwrap().iter().map(num).collect();
    let actual = reals(&a);
    assert_eq!(actual.len(), expected.len(), "{name} vs {key}");
    for (i, (x, y)) in actual.iter().zip(&expected).enumerate() {
        assert!(same(*x, *y, tol), "{name}[{i}] = {x}, {key} = {y}");
    }
}

fn error(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .err()
        .unwrap_or_else(|| panic!("{source} must fail"))
}

const SETUP: &str = "A=reshape(1:24,2,3,4); B=reshape(mod((1:24)*7,11)-5,2,3,4); B([3 10 17])=NaN; C=char(reshape(65:76,2,3,2));";

#[test]
fn part2_operators_scalar_forms_char_operands_and_documented_errors() {
    let r = fixture();
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("P=2*A; Q=A*2; R=A/4; S=4\\A; D=C+1; T=A(:,:,2).'; U=A(:,:,2)'; N=-C; I=int8(A)*2; G=single(A)/4; Z=complex(A,A)*2; L=(A>3)*1;").unwrap();
    // Exact: integer-valued data and single correctly rounded divisions.
    check(&e, "P", &r, "times2", 0.);
    check(&e, "Q", &r, "times2", 0.);
    check(&e, "R", &r, "div4", 0.);
    check(&e, "S", &r, "ldiv4", 0.);
    check(&e, "D", &r, "char_plus1", 0.);
    check(&e, "T", &r, "page2_transpose", 0.);
    check(&e, "U", &r, "page2_transpose", 0.);
    assert_eq!(e.array("D").unwrap().class(), "double");
    assert_eq!(e.array("N").unwrap().class(), "double");
    assert_eq!(e.array("I").unwrap().class(), "int8");
    assert_eq!(e.array("G").unwrap().class(), "single");
    assert_eq!(e.array("Z").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.array("L").unwrap().dims().as_ref(), &[2, 3, 4]);
    for (source, message) in [
        (
            "x=A*ones(4,2);",
            "Use TIMES (.*) for elementwise multiplication.",
        ),
        (
            "x=ones(2)*A;",
            "Use TIMES (.*) for elementwise multiplication.",
        ),
        (
            "x=A/ones(2);",
            "Use RDIVIDE (./) for elementwise right division.",
        ),
        ("x=2/A;", "Use RDIVIDE (./) for elementwise right division."),
        (
            "x=ones(2)\\A;",
            "Use LDIVIDE (.\\) for elementwise left division.",
        ),
        (
            "x=A\\2;",
            "Use LDIVIDE (.\\) for elementwise left division.",
        ),
        ("x=A^2;", "use POWER (.^) for elementwise power."),
        ("x=2^A;", "use POWER (.^) for elementwise power."),
        (
            "x=A';",
            "Transpose on ND array is not defined. Use PERMUTE instead.",
        ),
        (
            "x=A.';",
            "Transpose on ND array is not defined. Use PERMUTE instead.",
        ),
        (
            "x=C';",
            "Transpose on ND array is not defined. Use PERMUTE instead.",
        ),
        (
            "x=A && true;",
            "Operands to the || and && operators must be convertible to logical scalar values.",
        ),
        (
            "x=false || A;",
            "Operands to the || and && operators must be convertible to logical scalar values.",
        ),
    ] {
        let text = error(&mut e, source);
        assert!(text.contains(message), "{source}: {text}");
    }
    // The right operand is not evaluated once the left decides the result.
    e.execute("x=true || A; y=false && A;").unwrap();
    assert_eq!(e.logical_variables["x"].data(), &[1]);
    assert_eq!(e.logical_variables["y"].data(), &[0]);
}

#[test]
fn part2_reductions_with_dims_vecdim_all_and_nan_flags() {
    let r = fixture();
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    check(&e, "B", &r, "B", 0.);
    e.execute("s1=sum(B,1); s2=sum(B,2); s3=sum(B,3); o1=sum(B,1,'omitnan'); o2=sum(B,2,'omitnan'); o3=sum(B,3,'omitnan'); v13=sum(A,[1 3]); sa=sum(A,'all'); so=sum(B,'all','omitnan'); m3=mean(B,3,'omitnan'); m12=mean(A,[1 2]); p3=prod(A,3); s5=sum(A,5);").unwrap();
    for d in 1..=3 {
        // Exact: integer-valued sums.
        check(&e, &format!("s{d}"), &r, &format!("sum_dim{d}"), 0.);
        check(&e, &format!("o{d}"), &r, &format!("sum_omit_dim{d}"), 0.);
    }
    check(&e, "v13", &r, "sum_vec13", 0.);
    assert_eq!(e.variables["sa"].data(), &[r["sum_all"].as_f64().unwrap()]);
    assert_eq!(
        e.variables["so"].data(),
        &[r["sum_all_omit"].as_f64().unwrap()]
    );
    // Means: one division after an exact sum; relative 1e-15.
    check(&e, "m3", &r, "mean_omit_dim3", 1e-15);
    check(&e, "m12", &r, "mean_vec12", 1e-15);
    // prod of 1..24 by page: exact below 2^53 (largest is 6*12*18*24 = 31104).
    check(&e, "p3", &r, "prod_dim3", 0.);
    assert_eq!(e.array("s5").unwrap().dims().as_ref(), &[2, 3, 4]);
    for f in ["max", "min"] {
        e.execute(&format!(
            "[V,I]={f}(B,[],3); [W,J]={f}(B,[],3,'includenan'); [X,K]={f}(B,[],[1 3]);"
        ))
        .unwrap();
        check(&e, "V", &r, &format!("{f}_dim3"), 0.);
        check(&e, "I", &r, &format!("{f}_dim3_index"), 0.);
        check(&e, "W", &r, &format!("{f}_dim3_includenan"), 0.);
        check(&e, "J", &r, &format!("{f}_dim3_includenan_index"), 0.);
        check(&e, "X", &r, &format!("{f}_vec13"), 0.);
        check(&e, "K", &r, &format!("{f}_vec13_index"), 0.);
    }
    e.execute("[M,I]=max(B,[],'all'); [M2,I2]=max(B,[],[1 2 3],'linear');")
        .unwrap();
    assert_eq!(e.variables["M"].data(), &[num(&r["max_all"][0])]);
    assert_eq!(e.variables["I"].data(), &[num(&r["max_all"][1])]);
    assert_eq!(e.variables["I2"].data(), &[num(&r["max_all"][1])]);
    // any ignores NaN; all counts NaN as true.
    e.execute("N=reshape([NaN 0 0 0 NaN 1 0 0],2,2,2); y=any(N,3); z=all(N,3); ya=any(N,'all'); za=all(N,[1 2 3]);").unwrap();
    check(&e, "y", &r, "any_nan_dim3", 0.);
    check(&e, "z", &r, "all_nan_dim3", 0.);
    assert_eq!(e.logical_variables["ya"].data(), &[1]);
    assert_eq!(e.logical_variables["za"].data(), &[0]);
    e.execute("c3=cumsum(A,3); cr=cumsum(A,3,'reverse'); cp=cumprod(A,2); co=cumsum(B,1,'omitnan'); cd=cumsum(A);").unwrap();
    check(&e, "c3", &r, "cumsum_dim3", 0.);
    check(&e, "cr", &r, "cumsum_dim3_reverse", 0.);
    check(&e, "cp", &r, "cumprod_dim2", 0.);
    check(&e, "co", &r, "cumsum_omit_dim1", 0.);
    assert_eq!(e.array("cd").unwrap().dims().as_ref(), &[2, 3, 4]);
    // Empty dimensions: sum is 0, prod 1, mean NaN, max keeps the empty extent.
    e.execute("E=zeros(0,3,2); es=sum(E); ep=prod(E,1); em=mean(E,'all'); [ex,ix]=max(E,[],1); ea=any(E,3);").unwrap();
    assert_eq!(e.array("es").unwrap().dims().as_ref(), &[1, 3, 2]);
    assert_eq!(e.variables["es"].data(), &[0.; 6]);
    assert_eq!(e.variables["ep"].data(), &[1.; 6]);
    assert!(e.variables["em"].data()[0].is_nan());
    assert_eq!(e.array("ex").unwrap().dims().as_ref(), &[0, 3, 2]);
    assert_eq!(e.array("ea").unwrap().dims().as_ref(), &[0, 3]);
    for (source, message) in [
        ("x=sum(A,[1 1]);", "repeated"),
        ("x=sum(A,0);", "positive integer"),
        ("x=any(A,'omitnan','includenan');", "Invalid option"),
        ("x=cumsum(A,'all');", "Invalid option"),
        ("x=max(A,ones(3,3));", "Array dimensions must agree"),
        ("x=max(A,[],'bogus');", "Invalid option"),
    ] {
        let text = error(&mut e, source);
        assert!(text.contains(message), "{source}: {text}");
    }
}

#[test]
fn part2_typed_complex_and_char_reductions() {
    let r = fixture();
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("I8=int8(reshape([100 100 -100 50 20 30 1 2],2,2,2)); s8=sum(I8,3); d8=sum(I8,3,'double'); m8=mean(I8,3); c8=cumsum(I8,3); [x8,k8]=max(I8,[],3);").unwrap();
    check(&e, "s8", &r, "int8_sum_dim3", 0.);
    assert_eq!(e.array("s8").unwrap().class(), "double");
    assert_eq!(e.array("d8").unwrap().class(), "double");
    assert_eq!(e.variables["d8"].data(), &[120., 130., -99., 52.]);
    assert_eq!(e.array("m8").unwrap().class(), "double");
    assert_eq!(e.variables["m8"].data(), &[60., 65., -49.5, 26.]);
    assert_eq!(e.array("c8").unwrap().class(), "int8");
    assert_eq!(e.array("x8").unwrap().class(), "int8");
    // uint64 near 2^64: native integer sums are exact (no double rounding).
    e.execute("U=zeros(1,2,2,'uint64'); u=intmax('uint64')-9; U(1,1,1)=u; U(1,1,2)=3; U(1,2,1)=u; U(1,2,2)=7; su=sum(U,3,'native'); [mu,iu]=max(U,[],3); eq=isequal(su,su);").unwrap();
    let Array::Typed(su) = e.array("su").unwrap() else {
        panic!("uint64 sum")
    };
    assert_eq!(su.class().name(), "uint64");
    let expected: Vec<i128> = r["uint64_sum_dim3"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().parse().unwrap())
        .collect();
    assert_eq!(
        (0..su.len()).map(|i| su.int(i)).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(e.variables["iu"].data(), &[1., 1.]);
    // single: f64 accumulation of f32 values, rounded once to f32 (exact vs NumPy's same rule).
    e.execute("S=single(reshape(1:8,2,2,2))/3; ms=mean(S,3); ss=sum(S,'all');")
        .unwrap();
    check(&e, "ms", &r, "single_mean_dim3", 0.);
    assert_eq!(e.array("ms").unwrap().class(), "single");
    assert_eq!(e.array("ss").unwrap().class(), "single");
    // Complex: sums/products to 1e-15 relative; max by magnitude then angle.
    e.execute("Z=reshape([1+1i -2 3i 1-1i 2 -3 0.5i 1i],2,2,2); zs=sum(Z,3); zp=prod(Z,3); [zm,zi]=max(Z,[],3); za=any(Z,3); zc=cumsum(Z,3); zz=mean(Z,'all');").unwrap();
    check(&e, "zs", &r, "complex_sum_dim3", 1e-15);
    check(&e, "zp", &r, "complex_prod_dim3", 1e-15);
    check(&e, "zm", &r, "complex_max_dim3", 0.);
    check(&e, "zi", &r, "complex_max_dim3_index", 0.);
    assert_eq!(e.array("zc").unwrap().dims().as_ref(), &[2, 2, 2]);
    assert_eq!(e.logical_variables["za"].data(), &[1; 4]);
    // Char: reductions give double.
    e.execute("cs=sum(C,3); [cm,ci]=max(C,[],3); ca=all(C,'all');")
        .unwrap();
    check(&e, "cs", &r, "char_sum_dim3", 0.);
    assert_eq!(e.array("cm").unwrap().class(), "double");
    assert_eq!(e.variables["ci"].data(), &[2.; 6]);
}

#[test]
fn part2_size_forms_permute_shiftdim_squeeze_and_grids() {
    let r = fixture();
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("a=size(A,[1 3]); b=size(A,2,3); c=size(A,5); [r1,c1,p1]=size(A); [r2,c2]=size(A); [x1,x2]=size(A,[1 3]); [y1,y2,y3,y4]=size(A); z=size(zeros(0,3,2),[3 1]);").unwrap();
    assert_eq!(e.variables["a"].data(), &[2., 4.]);
    assert_eq!(e.variables["b"].data(), &[3., 4.]);
    assert_eq!(e.variables["c"].data(), &[1.]);
    let scalars = |e: &Engine, names: &[&str]| -> Vec<f64> {
        names.iter().map(|n| e.variables[*n].data()[0]).collect()
    };
    assert_eq!(scalars(&e, &["r1", "c1", "p1"]), [2., 3., 4.]);
    assert_eq!(scalars(&e, &["r2", "c2"]), [2., 12.]);
    assert_eq!(scalars(&e, &["x1", "x2"]), [2., 4.]);
    assert_eq!(scalars(&e, &["y1", "y2", "y3", "y4"]), [2., 3., 4., 1.]);
    assert_eq!(e.variables["z"].data(), &[2., 0.]);
    assert!(error(&mut e, "[q,w]=size(A,1);").contains("Number of output arguments"));
    e.execute("P=permute(reshape(1:6,2,3),[3 1 2]); Q=ipermute(P,[3 1 2]); T=permute(int16(reshape(1:6,2,3)),[3 1 2]); W=permute(A,[3 1 2]);").unwrap();
    check(&e, "P", &r, "permute_2d_312", 0.);
    check(&e, "Q", &r, "ipermute_roundtrip", 0.);
    assert_eq!(e.array("T").unwrap().class(), "int16");
    assert_eq!(e.array("W").unwrap().dims().as_ref(), &[4, 2, 3]);
    e.execute("s1=shiftdim(A,1); s2=shiftdim(A,2); s3=shiftdim(A,3); [s4,n4]=shiftdim(ones(1,1,3)); s5=shiftdim(1:3,-2); s6=shiftdim(C,1);").unwrap();
    check(&e, "s1", &r, "shiftdim_A_1", 0.);
    check(&e, "s2", &r, "shiftdim_A_2", 0.);
    assert_eq!(e.array("s3").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.array("s4").unwrap().dims().as_ref(), &[3, 1]);
    assert_eq!(e.variables["n4"].data(), &[2.]);
    let neg: Vec<usize> = r["shiftdim_row_neg2"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(e.array("s5").unwrap().dims().as_ref(), neg);
    assert_eq!(e.array("s6").unwrap().class(), "char");
    assert_eq!(e.array("s6").unwrap().dims().as_ref(), &[3, 2, 2]);
    e.execute("q1=squeeze(ones(1,1,3)); q2=squeeze(ones(2,1,3)); q3=squeeze(ones(1,2,1,3));")
        .unwrap();
    assert_eq!(e.array("q1").unwrap().dims().as_ref(), &[3, 1]);
    assert_eq!(e.array("q2").unwrap().dims().as_ref(), &[2, 3]);
    assert_eq!(e.array("q3").unwrap().dims().as_ref(), &[2, 3]);
    e.execute("[X,Y,Z]=meshgrid([1 2 3],[10 20],[100 200 300 400]); [G1,G2,G3]=ndgrid([1 2 3],[10 20],[100 200 300 400]); [H1,H2,H3]=meshgrid(1:2); K=ndgrid(1:2,1:3,1:4,1:5); [L1,L2,L3]=ndgrid(1:2);").unwrap();
    for (name, key) in [
        ("X", "meshgrid_X"),
        ("Y", "meshgrid_Y"),
        ("Z", "meshgrid_Z"),
        ("G1", "ndgrid_1"),
        ("G2", "ndgrid_2"),
        ("G3", "ndgrid_3"),
    ] {
        check(&e, name, &r, key, 0.);
    }
    assert_eq!(e.array("H3").unwrap().dims().as_ref(), &[2, 2, 2]);
    assert_eq!(e.array("K").unwrap().dims().as_ref(), &[2, 3, 4, 5]);
    assert_eq!(e.array("L3").unwrap().dims().as_ref(), &[2, 2, 2]);
    assert!(error(&mut e, "[a,b,c,d]=meshgrid(1:2);").contains("more outputs"));
}

#[test]
fn part2_find_and_sort_along_any_dimension() {
    let r = fixture();
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("F=reshape([0 3 0 0 5 0 7 0 0 0 0 9],2,3,2); k=find(F); [fr,fc]=find(F); [gr,gc,gv]=find(F); l=find(F,2,'last'); f1=find(F,1); fl=find(F>4);").unwrap();
    let list = |key: &str| -> Vec<f64> { r[key].as_array().unwrap().iter().map(num).collect() };
    assert_eq!(e.variables["k"].data(), list("find_linear"));
    assert_eq!(e.array("k").unwrap().dims().as_ref(), &[4, 1]);
    assert_eq!(e.variables["fr"].data(), list("find_rows"));
    assert_eq!(e.variables["fc"].data(), list("find_cols"));
    assert_eq!(e.variables["gv"].data(), &[3., 5., 7., 9.]);
    assert_eq!(e.variables["l"].data(), list("find_last2"));
    assert_eq!(e.variables["f1"].data(), &[2.]);
    assert_eq!(e.variables["fl"].data(), &[5., 7., 12.]);
    for axis in [2, 3] {
        for direction in ["ascend", "descend"] {
            e.execute(&format!("[V,I]=sort(B,{axis},'{direction}');"))
                .unwrap();
            let key = format!("sort_dim{axis}_{direction}");
            check(&e, "V", &r, &key, 0.);
            check(&e, "I", &r, &format!("{key}_index"), 0.);
        }
    }
    e.execute("D=sort(B); T=sort(int8(A),3,'descend'); Ch=sort(C,3,'descend'); Zs=sort(reshape([3i -1 2 1i],1,2,2),3);").unwrap();
    assert_eq!(e.array("D").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.array("T").unwrap().class(), "int8");
    assert_eq!(&reals(&e.array("T").unwrap())[..2], &[19., 20.]);
    assert_eq!(e.array("Ch").unwrap().class(), "char");
    let Array::Complex(z) = e.array("Zs").unwrap() else {
        panic!("complex sort")
    };
    // Lanes [3i 2] and [-1 1i]: |2| < |3i|; |-1| = |1i|, so phase pi/2 < pi orders them.
    assert_eq!(
        z.data().iter().map(|c| (c.re, c.im)).collect::<Vec<_>>(),
        [(2., 0.), (0., 1.), (0., 3.), (-1., 0.)]
    );
}

#[test]
fn part2_complex_char_writes_index_shapes_isequal_and_chained_reads() {
    let r = fixture();
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("Z=complex(A,-A); Z(:,:,5)=1i; Z(:,2,:)=[]; Y=Z; Y(1,1,1)=7; Ch=C; Ch(:,:,3)=['abc';'def']; Ch(:,2,:)=[]; Ch(2,2,3)='z'; M=A; M(:,:,2)=C(:,:,1);").unwrap();
    assert_eq!(e.array("Z").unwrap().dims().as_ref(), &[2, 2, 5]);
    let Array::Complex(z) = e.array("Z").unwrap() else {
        panic!("complex")
    };
    assert_eq!(z.data()[0], matrust::Complex64::new(1., -1.));
    assert_eq!(z.data()[16], matrust::Complex64::new(0., 1.));
    assert_eq!(e.array("Ch").unwrap().class(), "char");
    assert_eq!(e.array("Ch").unwrap().dims().as_ref(), &[2, 2, 3]);
    assert_eq!(
        &reals(&e.array("Ch").unwrap())[8..],
        &[97., 100., 99., f64::from(b'z')]
    );
    assert_eq!(
        &e.variables["M"].data()[6..12],
        &[65., 66., 67., 68., 69., 70.]
    );
    // A(idx) keeps the N-D shape of idx, for N-D and 2-D sources.
    e.execute("idx=reshape([1 3 5 7 9 11 13 15],2,2,2); X=A(idx); W=reshape(1:24,4,6); V=W(idx); L=A(logical(idx));").unwrap();
    check(&e, "X", &r, "index_shape", 0.);
    check(&e, "V", &r, "index_shape", 0.);
    assert_eq!(e.array("L").unwrap().dims().as_ref(), &[8, 1]);
    // isequal ignores class; NaN never equals NaN.
    e.execute("q1=isequal(A,int16(A)); q2=isequal(A,single(A)); q3=isequal(true(2,2,2),ones(2,2,2)); q4=isequal(double(C),C); q5=isequal(A,A+1); q6=isequal(complex(A,zeros(2,3,4)),A); q7=isequal(nan(2,2,2),nan(2,2,2)); q8=isequal(A,A,double(A));").unwrap();
    for (name, value) in [
        ("q1", 1),
        ("q2", 1),
        ("q3", 1),
        ("q4", 1),
        ("q5", 0),
        ("q6", 1),
        ("q7", 0),
        ("q8", 1),
    ] {
        assert_eq!(e.logical_variables[name].data(), &[value], "{name}");
    }
    // Chained field and content reads.
    e.execute("s.f=A; c={A,C}; a1=s.f(:,:,2); a2=c{1}(:,:,2); a3=s.f(2,3,4); a4=c{2}(1,:,2); a5=c{1}(end);").unwrap();
    assert_eq!(e.variables["a1"].data(), &[7., 8., 9., 10., 11., 12.]);
    assert_eq!(e.variables["a2"].data(), &[7., 8., 9., 10., 11., 12.]);
    assert_eq!(e.variables["a3"].data(), &[24.]);
    assert_eq!(reals(&e.array("a4").unwrap()), &[71., 73., 75.]);
    assert_eq!(e.variables["a5"].data(), &[24.]);
}

/// Complex data of `name` as (re, im) pairs.
fn complex_parts(e: &Engine, name: &str) -> Vec<(f64, f64)> {
    let Array::Complex(z) = e.array(name).unwrap() else {
        panic!("{name} must be complex")
    };
    z.data().iter().map(|z| (z.re, z.im)).collect()
}

fn same_parts(actual: &[(f64, f64)], expected: &[(f64, f64)], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}");
    for (i, (x, y)) in actual.iter().zip(expected).enumerate() {
        assert!(
            same(x.0, y.0, 0.) && same(x.1, y.1, 0.),
            "{name}[{i}] = {x:?}, expected {y:?}"
        );
    }
}

#[test]
fn part2_complex_with_a_single_part_errors_like_2d() {
    // Fix round 1, finding 1: 2-D complex rejects any single or integer part
    // (typed_functions.rs); N-D parts give the same message instead of a double result.
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    let two_d = error(&mut e, "z=complex(single(ones(2)),ones(2));");
    assert!(
        two_d.contains("complex single values are not supported"),
        "{two_d}"
    );
    for source in [
        "z=complex(single(A),A);",
        "z=complex(A,single(A));",
        "z=complex(single(A),1);",
        "z=complex(A,single(2));",
        "z=complex(single(A));",
        "z=complex(int8(A),A);",
    ] {
        assert_eq!(error(&mut e, source), two_d, "{source}");
    }
    assert!(e.array("z").is_none());
}

#[test]
fn part2_pairwise_max_min_with_complex_nd_operands() {
    // Fix round 1, finding 2: max(A,B) / min(A,B) with a complex N-D operand use the
    // 2-D rule: abs, then angle in (-pi, pi]; equal keys keep A; NaN loses unless
    // 'includenan', which returns the NaN element (A's first). Expected values are
    // derived by hand from that rule (unit and integer magnitudes): exact.
    let mut e = Engine::default();
    e.execute("Z=reshape([1+1i -2 3i 1-1i 2 -3 0.5i 1i],2,2,2); R=[-1i 2]; mx=max(Z,R); mn=min(Z,R); rx=max(R,Z);").unwrap();
    for name in ["mx", "mn", "rx"] {
        assert_eq!(e.array(name).unwrap().dims().as_ref(), &[2, 2, 2], "{name}");
    }
    let mx = [
        (1., 1.),
        (-2., 0.),
        (0., 3.),
        (2., 0.),
        (2., 0.),
        (-3., 0.),
        (2., 0.),
        (2., 0.),
    ];
    same_parts(&complex_parts(&e, "mx"), &mx, "mx");
    same_parts(&complex_parts(&e, "rx"), &mx, "rx");
    let mn = [
        (0., -1.),
        (0., -1.),
        (2., 0.),
        (1., -1.),
        (0., -1.),
        (0., -1.),
        (0., 0.5),
        (0., 1.),
    ];
    same_parts(&complex_parts(&e, "mn"), &mn, "mn");
    // Equal magnitude: the larger angle wins max (angle(-1) = pi > angle(1i) = pi/2 > 0).
    e.execute("T=reshape([-1 1i 1 -1i],1,1,4); tx=max(T,1i); tn=min(T,1i);")
        .unwrap();
    same_parts(
        &complex_parts(&e, "tx"),
        &[(-1., 0.), (0., 1.), (0., 1.), (0., 1.)],
        "tx",
    );
    same_parts(
        &complex_parts(&e, "tn"),
        &[(0., 1.), (0., 1.), (1., 0.), (0., -1.)],
        "tn",
    );
    // A result with zero imaginary parts is real, as in 2-D; the shape stays N-D.
    e.execute("nr=max(reshape([1 -2 3 -4],1,1,4),0.5i);")
        .unwrap();
    assert_eq!(e.array("nr").unwrap().class(), "double");
    assert_eq!(e.array("nr").unwrap().dims().as_ref(), &[1, 1, 4]);
    assert_eq!(e.variables["nr"].data(), &[1., -2., 3., -4.]);
    // NaN flags: complex(NaN,1) and NaN lose by default; 'includenan' returns them.
    e.execute("W=reshape([complex(NaN,1) 2 1i NaN],2,1,2); wo=max(W,1); wi=max(W,1,'includenan'); wb=max(1,W,'includenan'); wm=min(W,1,'omitnan');").unwrap();
    same_parts(
        &complex_parts(&e, "wo"),
        &[(1., 0.), (2., 0.), (0., 1.), (1., 0.)],
        "wo",
    );
    let nan = f64::NAN;
    let with_nan = [(nan, 1.), (2., 0.), (0., 1.), (nan, 0.)];
    same_parts(&complex_parts(&e, "wi"), &with_nan, "wi");
    same_parts(&complex_parts(&e, "wb"), &with_nan, "wb");
    // wm: 1 vs 1i has equal abs and the smaller angle, so min picks 1 everywhere: real.
    assert_eq!(e.array("wm").unwrap().dims().as_ref(), &[2, 1, 2]);
    assert_eq!(e.variables["wm"].data(), &[1.; 4]);
}

#[test]
fn part2_cell_and_struct_nd_give_clear_errors() {
    let mut e = Engine::default();
    for source in [
        "c=cell(2,2,2);",
        "c={1,2}; c(1,1,2)={3};",
        "s=struct('a',1); s(1,1,2).a=2;",
        "s=repmat(struct('a',1),[1 1 2]);",
    ] {
        let text = error(&mut e, source);
        assert!(
            text.contains("more than two dimensions") || text.contains("N-D"),
            "{source}: {text}"
        );
    }
}

// Fix round 3 regressions. Expected values hand-derived (exact): every source holds k (char:
// 64+k) at linear position k, and a non-vector index gives the index's shape whatever the
// source's orientation or class (MathWorks "Array Indexing"; NumPy fancy indexing agrees).

/// Dims, class and real values of variable `name`.
fn shaped(e: &Engine, name: &str) -> (Vec<usize>, String, Vec<f64>) {
    let a = e.array(name).unwrap_or_else(|| panic!("{name} missing"));
    (a.dims().into_owned(), a.class().to_owned(), reals(&a))
}

#[test]
fn part2_column_vector_source_with_nd_index_keeps_the_index_shape() {
    let mut e = Engine::default();
    e.execute("W=(1:24)'; R=W(reshape(1:4,1,2,2)); s.f=(1:24)'; F=s.f(reshape(1:4,1,1,4)); L=(W>0); G=L(reshape(1:4,1,2,2)); V=W([1 2 3]);").unwrap();
    let four = vec![1., 2., 3., 4.];
    assert_eq!(
        shaped(&e, "R"),
        (vec![1, 2, 2], "double".into(), four.clone())
    );
    assert_eq!(shaped(&e, "F"), (vec![1, 1, 4], "double".into(), four));
    assert_eq!(
        shaped(&e, "G"),
        (vec![1, 2, 2], "logical".into(), vec![1.; 4])
    );
    // A vector index still follows the source's orientation.
    assert_eq!(e.array("V").unwrap().dims().as_ref(), &[3, 1]);
}

#[test]
fn part2_char_and_typed_sources_keep_an_nd_index_shape() {
    let mut e = Engine::default();
    e.execute("c=char(64+(1:24)); r1=c(reshape(1:4,1,2,2)); cc=c'; r2=cc(reshape(1:4,1,2,2)); s.t=c; r3=s.t(reshape(1:4,2,1,2)); x=int8(1:24); r4=x(reshape(1:4,2,1,2)); xc=x'; r5=xc(reshape(1:4,1,2,2)); u=intmax('uint64')-uint64(0:23); r6=u(reshape(1:4,1,2,2)); g=single(1:24); r7=g(reshape(1:4,2,2,1,1)); v1=c([1 2 3]); v2=xc([1 2 3]);").unwrap();
    let four = vec![1., 2., 3., 4.];
    let letters = vec![65., 66., 67., 68.];
    for (name, dims, class, values) in [
        ("r1", vec![1, 2, 2], "char", letters.clone()),
        ("r2", vec![1, 2, 2], "char", letters.clone()),
        ("r3", vec![2, 1, 2], "char", letters),
        ("r4", vec![2, 1, 2], "int8", four.clone()),
        ("r5", vec![1, 2, 2], "int8", four.clone()),
        ("r7", vec![2, 2], "single", four),
    ] {
        assert_eq!(shaped(&e, name), (dims, class.into(), values), "{name}");
    }
    // uint64 near 2^64: values are compared exactly as integers, not through f64.
    let Array::Typed(t) = e.array("r6").unwrap() else {
        panic!("r6 must be uint64")
    };
    assert_eq!(
        (t.class().name(), t.dims().as_ref()),
        ("uint64", &[1, 2, 2][..])
    );
    let top = i128::from(u64::MAX);
    assert_eq!(
        (0..4).map(|i| t.int(i)).collect::<Vec<_>>(),
        &[top, top - 1, top - 2, top - 3]
    );
    // Vector indices keep the source-orientation rule.
    assert_eq!(e.array("v1").unwrap().dims().as_ref(), &[1, 3]);
    assert_eq!(e.array("v2").unwrap().dims().as_ref(), &[3, 1]);
}

#[test]
fn part2_cell_drops_trailing_singleton_dimensions() {
    let mut e = Engine::default();
    e.execute("c1=cell(2,3,1); c2=cell(2,3,1,1); c3=cell([2 3 1]); c4=cell([0 3 1 1]);")
        .unwrap();
    for (name, dims) in [
        ("c1", [2, 3]),
        ("c2", [2, 3]),
        ("c3", [2, 3]),
        ("c4", [0, 3]),
    ] {
        let a = e.array(name).unwrap();
        assert_eq!(
            (a.class(), a.dims().as_ref()),
            ("cell", &dims[..]),
            "{name}"
        );
    }
    let text = error(&mut e, "c5=cell(2,3,2);");
    assert!(text.contains("more than two dimensions"), "{text}");
}

#[test]
fn part2_empty_nd_echo_uses_the_documented_empty_array_line() {
    let mut e = Engine::default();
    let out = e.execute("A=zeros(0,3,2)").unwrap().output.join("\n");
    assert!(out.contains("A =\n"), "{out:?}");
    assert!(
        out.contains("  0\u{00d7}3\u{00d7}2 empty double array"),
        "{out:?}"
    );
    assert!(!out.contains("(:,:,1)"), "{out:?}");
    let out = e.execute("B=true(2,0,3)").unwrap().output.join("\n");
    assert!(
        out.contains("  2\u{00d7}0\u{00d7}3 empty logical array"),
        "{out:?}"
    );
    let out = e.execute("disp(zeros(0,3,2))").unwrap().output.join("\n");
    assert!(out.is_empty(), "{out:?}");
}

/// Minimal Level 5 MAT file with one uncompressed miMATRIX element.
fn mat_v5(class: u8, flags: u8, dims: &[i32], name: &str, data_type: u32, data: &[u8]) -> Vec<u8> {
    let mut header = vec![b' '; 116];
    header[..10].copy_from_slice(b"MATLAB 5.0");
    header.extend_from_slice(&[0; 8]);
    header.extend_from_slice(&0x0100u16.to_le_bytes());
    header.extend_from_slice(b"IM");
    let pad = |v: &mut Vec<u8>| {
        while !v.len().is_multiple_of(8) {
            v.push(0);
        }
    };
    let mut body = Vec::new();
    body.extend_from_slice(&6u32.to_le_bytes());
    body.extend_from_slice(&8u32.to_le_bytes());
    body.extend_from_slice(&u32::from_le_bytes([class, flags, 0, 0]).to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&5u32.to_le_bytes());
    body.extend_from_slice(&(4 * dims.len() as u32).to_le_bytes());
    for d in dims {
        body.extend_from_slice(&d.to_le_bytes());
    }
    pad(&mut body);
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(&(name.len() as u32).to_le_bytes());
    body.extend_from_slice(name.as_bytes());
    pad(&mut body);
    body.extend_from_slice(&data_type.to_le_bytes());
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(data);
    pad(&mut body);
    let mut file = header;
    file.extend_from_slice(&14u32.to_le_bytes());
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&body);
    file
}

#[test]
fn part2_mat_rejects_overflowing_dimensions_and_logical_char() {
    use std::{collections::BTreeSet, sync::atomic::AtomicBool};
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    // Sanity: the same writer produces a loadable 1x3 char.
    let units: Vec<u8> = "abc".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let good = dir.path().join("good.mat");
    std::fs::write(&good, mat_v5(4, 0, &[1, 3], "c", 4, &units)).unwrap();
    let catalog = matrust::files::inspect_workspace(&good, &cancel).unwrap();
    let names = BTreeSet::from(["c".to_owned()]);
    let loaded = matrust::files::load_selected_arrays(&catalog, &names, &cancel).unwrap();
    assert_eq!(loaded["c"].class(), "char");
    for (file, class, flags, dims, data_type, message) in [
        (
            "logical_char.mat",
            4u8,
            0x02u8,
            &[1, 3][..],
            4u32,
            "logical flag",
        ),
        (
            "overflow.mat",
            6,
            0,
            &[65536, 65536, 65536, 65536, 65536][..],
            9,
            "dimensions overflow",
        ),
    ] {
        let path = dir.path().join(file);
        std::fs::write(&path, mat_v5(class, flags, dims, "x", data_type, &units)).unwrap();
        let text = match matrust::files::inspect_workspace(&path, &cancel) {
            Err(error) => error,
            Ok(catalog) => {
                let names = BTreeSet::from(["x".to_owned()]);
                match matrust::files::load_selected_arrays(&catalog, &names, &cancel) {
                    Err(error) => error,
                    Ok(_) => panic!("{file} must be rejected"),
                }
            }
        };
        assert!(text.contains(message), "{file}: {text}");
    }
}

/// Octave 10.3 and the 2-D double product: `uint64(2^63)` cubed is `2^189`.
/// Default and `'double'` both convert each factor before multiplying.
#[test]
fn review_s1113_nd_uint64_prod_is_2_pow_189() {
    let mut e = Engine::default();
    e.execute(
        "A=uint64(ones(1,1,3)*2^63); \
         p=prod(A,3); pd=prod(A,3,'double'); \
         p2=prod(reshape(A,1,3),2);",
    )
    .unwrap();
    let expected = 7.846377169233351e56;
    assert_eq!(expected, 2f64.powi(189));
    for name in ["p", "pd", "p2"] {
        assert_eq!(e.array(name).unwrap().class(), "double", "{name}");
        assert_eq!(e.variables[name].data(), &[expected], "{name}");
    }
    assert_eq!(e.variables["p"].data(), e.variables["p2"].data());
}

/// Octave 10.3: `sum` of `uint64(2^53+1)` and `uint64(1)` is `2^53` in double.
/// Adding the integers first would keep `2^53+2`, which double can represent.
#[test]
fn review_s1113_nd_uint64_sum_rounds_each_element() {
    let mut e = Engine::default();
    e.execute(
        "u=bitshift(uint64(1),53)+uint64(1); \
         s=sum(reshape([u,uint64(1)],1,1,2),3); \
         sd=sum(reshape([u,uint64(1)],1,1,2),3,'double'); \
         s2=sum([u uint64(1)],2);",
    )
    .unwrap();
    let expected = 9007199254740992.0;
    for name in ["s", "sd", "s2"] {
        assert_eq!(e.array(name).unwrap().class(), "double", "{name}");
        assert_eq!(e.variables[name].data(), &[expected], "{name}");
    }
    assert_eq!(e.variables["s"].data(), e.variables["s2"].data());
}

/// The same two inputs agree between the N-D reduction and the 2-D one.
#[test]
fn review_s1113_nd_uint64_sum_prod_match_2d() {
    let mut e = Engine::default();
    e.execute(
        "A=uint64(ones(1,1,3)*2^63); \
         pe=isequal(prod(A,3), prod(reshape(A,1,3),2)); \
         u=bitshift(uint64(1),53)+uint64(1); \
         se=isequal(sum(reshape([u,uint64(1)],1,1,2),3), sum([u uint64(1)],2));",
    )
    .unwrap();
    assert_eq!(e.logical_variables["pe"].data(), &[1]);
    assert_eq!(e.logical_variables["se"].data(), &[1]);
}
