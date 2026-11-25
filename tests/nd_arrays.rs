use matrust::Engine;

#[test]
fn constructors_queries_and_reshape_keep_full_dimensions() {
    let mut e = Engine::default();
    e.execute("A=zeros(2,3,4); B=ones([2 3 4]); C=true(2,3,4); D=false([2 3 4]); E=rand(2,3,4); F=randn([2 3 4]); G=randi(7,2,3,4); H=int8(ones(2,3,4)); T=zeros(2,3,4,'int8'); N=nan(2,3,4); I=inf([2 3 4]); R=reshape(1:24,2,3,4); Q=reshape(R,[],3,2);").unwrap();
    for name in [
        "A", "B", "C", "D", "E", "F", "G", "H", "T", "N", "I", "R", "Q",
    ] {
        let expected = if name == "Q" {
            &[4, 3, 2][..]
        } else {
            &[2, 3, 4][..]
        };
        assert_eq!(e.array(name).unwrap().dims().as_ref(), expected, "{name}");
    }
    assert_eq!(e.variables["A"].data(), &[0.; 24]);
    assert_eq!(e.variables["B"].data(), &[1.; 24]);
    assert_eq!(e.logical_variables["C"].data(), &[1; 24]);
    assert_eq!(e.logical_variables["D"].data(), &[0; 24]);
    assert_eq!(e.array("H").unwrap().class(), "int8");
    assert_eq!(e.array("T").unwrap().class(), "int8");
    e.execute("W=double(H);").unwrap();
    assert_eq!(e.array("W").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.array("W").unwrap().class(), "double");
    assert!(e.variables["N"].data().iter().all(|x| x.is_nan()));
    assert!(e.variables["I"].data().iter().all(|x| x.is_infinite()));
    assert_eq!(
        e.variables["R"].data(),
        &(1..=24).map(f64::from).collect::<Vec<_>>()
    );
    assert_eq!(
        e.variables["Q"].data(),
        &(1..=24).map(f64::from).collect::<Vec<_>>()
    );
    e.execute("S=size(A); N=ndims(A); M=numel(A); L=length(A); K=size(A,3); [r,c]=size(A);")
        .unwrap();
    assert_eq!(e.variables["S"].data(), &[2., 3., 4.]);
    assert_eq!(e.variables["N"].data(), &[3.]);
    assert_eq!(e.variables["M"].data(), &[24.]);
    assert_eq!(e.variables["L"].data(), &[4.]);
    assert_eq!(e.variables["K"].data(), &[4.]);
    assert_eq!(e.variables["r"].data(), &[2.]);
    assert_eq!(e.variables["c"].data(), &[12.]);
    assert!(e.array("A").unwrap().is_nd());
}

#[test]
fn n_subscript_linear_logical_and_end_reads_follow_column_major_order() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:24,2,3,4); x=A(2,3,4); y=A(2,12); z=A(2,3,4,1); q=A(end,1,end); v=A(3:6); m=A(logical(reshape([1 0 0 1],2,2)));").unwrap();
    assert_eq!(e.variables["x"].data(), &[24.]);
    assert_eq!(e.variables["y"].data(), &[24.]);
    assert_eq!(e.variables["z"].data(), &[24.]);
    assert_eq!(e.variables["q"].data(), &[20.]);
    assert_eq!(e.variables["v"].data(), &[3., 4., 5., 6.]);
    assert_eq!(e.variables["m"].data(), &[1., 4.]);
}

#[test]
fn nd_growth_deletion_and_repmat_preserve_page_layout() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:12,2,3,2); A(:,:,3)=0; B=A(:,:,3); A(:,:,2)=[]; C=repmat(reshape(1:8,2,2,2),1,2,2); G=reshape(1:4,2,2); G(:,:,3)=ones(2,2); T=int8(ones(2,2,2)); T(:,:,2)=3; Ch=char(reshape(65:72,2,2,2)); Ch(:,:,3)='x';")
        .unwrap();
    assert_eq!(e.array("A").unwrap().dims().as_ref(), &[2, 3, 2]);
    assert_eq!(
        e.variables["A"].data(),
        &([1., 2., 3., 4., 5., 6., 0., 0., 0., 0., 0., 0.])
    );
    assert_eq!(e.variables["B"].data(), &[0.; 6]);
    assert_eq!(e.array("C").unwrap().dims().as_ref(), &[2, 4, 4]);
    assert_eq!(
        e.variables["C"].data()[0..8],
        [1., 2., 3., 4., 1., 2., 3., 4.]
    );
    assert_eq!(e.array("G").unwrap().dims().as_ref(), &[2, 2, 3]);
    assert_eq!(
        e.variables["G"].data(),
        &[1., 2., 3., 4., 0., 0., 0., 0., 1., 1., 1., 1.]
    );
    assert_eq!(e.array("T").unwrap().class(), "int8");
    let matrust::Array::Typed(t) = e.array("T").unwrap() else {
        panic!("typed assignment class lost")
    };
    assert_eq!(
        (0..t.len()).map(|i| t.real(i)).collect::<Vec<_>>(),
        [1., 1., 1., 1., 3., 3., 3., 3.]
    );
    assert_eq!(e.array("Ch").unwrap().dims().as_ref(), &[2, 2, 3]);
}

#[test]
fn complex_nd_writes_growth_and_deletion_preserve_complex_values() {
    let mut e = Engine::default();
    e.execute(
        "C=reshape(complex(1:8,ones(1,8)),2,2,2); R=repmat(C,1,1,2); K=cat(4,C,C); C(:,:,2)=3+4i; C(:,:,3)=2i; P=C(:,:,3); C(:,:,1)=[];",
    )
    .unwrap();
    assert_eq!(e.array("C").unwrap().dims().as_ref(), &[2, 2, 2]);
    let matrust::Array::Complex(c) = e.array("C").unwrap() else {
        panic!("complex class lost during N-D assignment");
    };
    assert_eq!(
        c.data(),
        &[
            matrust::Complex64::new(3., 4.),
            matrust::Complex64::new(3., 4.),
            matrust::Complex64::new(3., 4.),
            matrust::Complex64::new(3., 4.),
            matrust::Complex64::new(0., 2.),
            matrust::Complex64::new(0., 2.),
            matrust::Complex64::new(0., 2.),
            matrust::Complex64::new(0., 2.),
        ]
    );
    let matrust::Array::Complex(p) = e.array("P").unwrap() else {
        panic!("complex indexing lost class");
    };
    assert_eq!(p.data(), &[matrust::Complex64::new(0., 2.); 4]);
    assert_eq!(e.array("R").unwrap().dims().as_ref(), &[2, 2, 4]);
    assert_eq!(e.array("K").unwrap().dims().as_ref(), &[2, 2, 2, 2]);
    assert!(matches!(e.array("R"), Some(matrust::Array::Complex(_))));
    assert!(matches!(e.array("K"), Some(matrust::Array::Complex(_))));
}

#[test]
fn permute_squeeze_implicit_expansion_and_reductions_use_all_axes() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:24,2,3,4); P=permute(A,[3 1 2]); I=ipermute(P,[3 1 2]); B=cat(4,A,A); H=horzcat(A,A); V=vertcat(A,A); C2=cat(3,reshape(1:4,2,2),reshape(5:8,2,2)); REP2=repmat(reshape(1:4,2,2),1,1,3);")
        .unwrap();
    e.execute("S=squeeze(reshape(1:8,1,2,1,4));").unwrap();
    e.execute(
        "X=reshape(1:24,2,3,4).*ones(1,1,4); Y=ones(2,1,4)+ones(1,3); Z=ones(2,1)+ones(1,3,4);",
    )
    .unwrap();
    e.execute(
        "R1=sum(A,1); R2=sum(A,2); R3=sum(A,3); RD=sum(A); PR=prod(A,1); MN=mean(A,2); MX=max(A,[],1); MI=min(A,[],1); ANY=any(A,1); ALL=all(A,1); [MV,MVI]=max(A,[],1); D=permute(reshape(1:24,2,3,4),[3 1 2]);",
    )
    .unwrap();
    assert_eq!(e.array("P").unwrap().dims().as_ref(), &[4, 2, 3]);
    assert_eq!(e.variables["I"].data(), e.variables["A"].data());
    assert_eq!(e.array("B").unwrap().dims().as_ref(), &[2, 3, 4, 2]);
    assert_eq!(e.array("H").unwrap().dims().as_ref(), &[2, 6, 4]);
    assert_eq!(e.array("V").unwrap().dims().as_ref(), &[4, 3, 4]);
    assert_eq!(e.array("C2").unwrap().dims().as_ref(), &[2, 2, 2]);
    assert_eq!(e.array("REP2").unwrap().dims().as_ref(), &[2, 2, 3]);
    assert_eq!(e.array("S").unwrap().dims().as_ref(), &[2, 4]);
    assert_eq!(e.array("X").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.variables["X"].data(), e.variables["A"].data());
    assert_eq!(e.array("Y").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.variables["Y"].data(), &[2.; 24]);
    assert_eq!(e.array("Z").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.variables["Z"].data(), &[2.; 24]);
    assert_eq!(e.array("R1").unwrap().dims().as_ref(), &[1, 3, 4]);
    assert_eq!(
        e.variables["R1"].data(),
        &[3., 7., 11., 15., 19., 23., 27., 31., 35., 39., 43., 47.]
    );
    assert_eq!(e.array("R2").unwrap().dims().as_ref(), &[2, 1, 4]);
    assert_eq!(e.array("R3").unwrap().dims().as_ref(), &[2, 3]);
    assert_eq!(e.variables["R3"].data(), &[40., 44., 48., 52., 56., 60.]);
    assert_eq!(e.variables["RD"].data(), e.variables["R1"].data());
    assert_eq!(
        e.variables["PR"].data(),
        &[
            2., 12., 30., 56., 90., 132., 182., 240., 306., 380., 462., 552.
        ]
    );
    assert_eq!(
        e.variables["MN"].data(),
        &[3., 4., 9., 10., 15., 16., 21., 22.]
    );
    assert_eq!(
        e.variables["MVI"].data(),
        &[2., 2., 2., 2., 2., 2., 2., 2., 2., 2., 2., 2.]
    );
    assert!(e.logical_variables["ANY"].data().iter().all(|&x| x == 1));
    assert!(e.logical_variables["ALL"].data().iter().all(|&x| x == 1));
    assert_eq!(e.variables["P"].data(), e.variables["D"].data());
}

#[test]
fn nd_display_emits_page_labels_and_disp_text() {
    let mut e = Engine::default();
    let echo = e.execute("A=reshape(1:8,2,2,2)").unwrap();
    assert!(echo.output.join("\n").contains("A(:,:,1) ="));
    assert!(echo.output.join("\n").contains("A(:,:,2) ="));
    let display = e.execute("disp(A);").unwrap();
    assert!(display.output.join("\n").contains("(:,:,1) ="));
}

#[cfg(feature = "mat-v73")]
#[test]
fn mat_v5_and_v73_roundtrip_nd_numeric_logical_typed_and_char() {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::atomic::AtomicBool,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("A=reshape(1:24,2,3,4); L=logical(A); T=int16(A); C=char(reshape(65:88,2,3,4));")
        .unwrap();
    let values = BTreeMap::from([
        ("A".into(), e.array("A").unwrap().clone()),
        ("L".into(), e.array("L").unwrap().clone()),
        ("T".into(), e.array("T").unwrap().clone()),
        ("C".into(), e.array("C").unwrap().clone()),
    ]);
    for v73 in [false, true] {
        let path = dir.path().join(if v73 { "nd73.mat" } else { "nd5.mat" });
        if v73 {
            matrust::files::save_array_workspace_v73(&path, &values, &AtomicBool::new(false))
                .unwrap();
        } else {
            matrust::files::save_array_workspace(&path, &values, &AtomicBool::new(false)).unwrap();
        }
        let catalog = matrust::files::inspect_workspace(&path, &AtomicBool::new(false)).unwrap();
        let names = BTreeSet::from(["A".into(), "L".into(), "T".into(), "C".into()]);
        let loaded =
            matrust::files::load_selected_arrays(&catalog, &names, &AtomicBool::new(false))
                .unwrap();
        for name in ["A", "L", "T", "C"] {
            assert_eq!(loaded[name].dims().as_ref(), &[2, 3, 4], "{name} v73={v73}");
            assert_eq!(
                loaded[name].class(),
                values[name].class(),
                "{name} v73={v73}"
            );
        }
    }
}

#[test]
fn unsupported_builtins_report_nd_size_and_two_dimensional_extra_one_subscripts_work() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:8,2,2,2);").unwrap();
    let error = e.execute("B=inv(A);").unwrap_err().to_string();
    assert!(
        error.contains("inv does not support N-D arrays yet (size 2x2x2)"),
        "{error}"
    );
    e.execute("C=reshape(1:6,2,3); D=C(1,1,1);").unwrap();
    assert_eq!(e.variables["D"].data(), &[1.]);
}

#[test]
fn round3_two_dimensional_calls_and_argument_effects_are_preserved() {
    let mut e = Engine::default();
    e.execute("I=reshape(int16(1:4),2,2); S=reshape(single(1:6),3,2); C=reshape({1,2;3,4},1,4); K=cat(3,{1,2}); J=cat(3,{1},[]); T=true(); F=false();").unwrap();
    assert_eq!(e.array("I").unwrap().class(), "int16");
    assert_eq!(e.array("S").unwrap().class(), "single");
    assert_eq!(e.array("C").unwrap().cols(), 4);
    assert_eq!(e.logical_variables["T"].data(), &[1]);
    assert_eq!(e.logical_variables["F"].data(), &[0]);
    let out = e.execute("[m,i]=max([3 fprintf('EVAL\\n')]);").unwrap();
    assert_eq!(out.output.join("\n").matches("EVAL").count(), 1);
    e.execute("R=reshape(int16(1:8),2,2,2); Q=reshape(single(1:8),2,2,2);")
        .unwrap();
    assert_eq!(e.array("R").unwrap().dims().as_ref(), &[2, 2, 2]);
    assert_eq!(e.array("Q").unwrap().dims().as_ref(), &[2, 2, 2]);
}

#[test]
fn round3_empty_expansion_linear_writes_and_folded_deletion() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/nd/round3.json")).unwrap();
    let mut e = Engine::default();
    e.execute("E=zeros(2,0,3); P=E+1; Q=E==1; R=E & true; Z=reshape(complex(zeros(2,0),zeros(2,0)),2,0,3)+1; A=reshape(1:24,2,3,4); A(5)=70; A(A>20)=0; B=A; B(5)=[]; C=A; C(:,2)=[]; D=reshape(complex(reshape(A,2,12),reshape(A,2,12)),2,3,4); D(:,2)=[];").unwrap();
    for name in ["P", "Q", "R", "Z"] {
        assert_eq!(e.array(name).unwrap().dims().as_ref(), &[2, 0, 3]);
    }
    assert_eq!(e.array("A").unwrap().dims().as_ref(), &[2, 3, 4]);
    assert_eq!(e.variables["A"].data()[4], 0.); // mask also selected the edited value 70
    assert_eq!(e.array("B").unwrap().dims().as_ref(), &[1, 23]);
    assert_eq!(e.array("C").unwrap().dims().as_ref(), &[2, 11]);
    assert_eq!(e.array("D").unwrap().dims().as_ref(), &[2, 11]);
    let expected: Vec<f64> = reference["folded_delete"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(e.variables["C"].data(), expected);
    assert!(
        e.execute("A(1,13)=1;")
            .unwrap_err()
            .contains("ambiguous dimension")
    );
    assert_eq!(e.array("A").unwrap().dims().as_ref(), &[2, 3, 4]);
    e.execute("H=[A A]; V=[A;A];").unwrap();
    assert_eq!(e.array("H").unwrap().dims().as_ref(), &[2, 6, 4]);
    assert_eq!(e.array("V").unwrap().dims().as_ref(), &[4, 3, 4]);
    let t = matrust::TypedArray::from_ints(matrust::NumericClass::Int8, 2, 4, [1; 8].into_iter())
        .unwrap()
        .with_dims(&[2, 2, 2])
        .unwrap();
    assert_eq!(t.select(1, 1, &[0]).unwrap().dims().as_ref(), &[1, 1]);
    // Empty axes may accompany large legal extents; unused strides must not overflow.
    e.execute("Huge=zeros(2^24,0,2^24,2^24); Empty=Huge+1; Mask=Huge==1;")
        .unwrap();
    assert_eq!(
        e.array("Empty").unwrap().dims().as_ref(),
        &[1 << 24, 0, 1 << 24, 1 << 24]
    );
    for source in [
        "X=ones(1024,1,2)+ones(1,24577);",
        "X=ones(1024,1,2)==ones(1,24577);",
    ] {
        assert!(e.execute(source).unwrap_err().contains("element limit"));
    }
}

#[test]
fn round3_guard_covers_every_builtin_entry_and_option_position() {
    let mut e = Engine::default();
    e.execute("A=ones(2,2,2);").unwrap();
    for (source, name) in [
        ("x=inv(A);", "inv"),
        ("[q,r]=qr(A);", "qr"),
        ("[a,b]=arrayfun(@abs,A);", "arrayfun"),
        ("[u,s,v]=svd(A);", "svd"),
        // imwrite takes M-by-N-by-3 images; audiowrite keeps the statement-form guard.
        ("audiowrite('guard.wav',A,8000);", "audiowrite"),
        ("writematrix(A,'guard.csv');", "writematrix"),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(
            error.contains(&format!(
                "{name} does not support N-D arrays yet (size 2x2x2)"
            )),
            "{source}: {error}"
        );
    }
    let error = e.execute("x=sum(1,A);").unwrap_err();
    assert!(
        error.contains("Dimension argument must be a positive integer"),
        "{error}"
    );
    e.execute("X=max(5,A); Y=min(A,0); E=isequal(zeros(2,4),zeros(2,2,2)); I=int8(true(2,2,2)); S=single(true(2,2,2)); T=int16(I); [M,K]=max(A,[],3);").unwrap();
    assert_eq!(e.array("X").unwrap().dims().as_ref(), &[2, 2, 2]);
    assert_eq!(e.variables["X"].data(), &[5.; 8]);
    assert_eq!(e.variables["Y"].data(), &[0.; 8]);
    assert_eq!(e.logical_variables["E"].data(), &[0]);
    for name in ["I", "S", "T"] {
        assert_eq!(e.array(name).unwrap().dims().as_ref(), &[2, 2, 2]);
    }
    assert_eq!(e.variables["M"].data(), &[1.; 4]);
    assert_eq!(e.variables["K"].data(), &[1.; 4]);
    assert!(e.execute("[m,i]=max(A,5);").is_err());
}

#[test]
fn round3_native_typed_nd_paths_preserve_wide_integers_and_classes() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/nd/round3.json")).unwrap();
    for class in ["int64", "uint64"] {
        let mut e = Engine::default();
        e.execute(&format!("A=zeros(2,2,2,'{class}'); a=intmax('{class}')-5; A(1)=a; A(8)=a-1; C=cat(4,A,A); D=A; D(:,2)=[]; G=A; G(:,:,3)=a; R=repmat(A,1,1,2); P=A+0; Q=-A; [M,I]=max(A,[],3); X=A==a; Y=isequal(A,P); B=max(A,a); J=cat(3,a,2);")).unwrap();
        let a: i128 = reference["wide"][class].as_str().unwrap().parse().unwrap();
        let data = |name: &str| {
            let matrust::Array::Typed(t) = e.array(name).unwrap() else {
                panic!("{name} lost {class}")
            };
            assert_eq!(t.class().name(), class);
            (0..t.len()).map(|i| t.int(i)).collect::<Vec<_>>()
        };
        assert_eq!(data("A"), [a, 0, 0, 0, 0, 0, 0, a - 1]);
        assert_eq!(data("P"), data("A"));
        assert_eq!(data("D"), [a, 0, 0, 0, 0, a - 1]);
        assert_eq!(data("M"), [a, 0, 0, a - 1]);
        assert_eq!(data("B"), [a; 8]);
        assert_eq!(data("J"), [a, 2]);
        assert_eq!(
            data("Q"),
            if class == "int64" {
                vec![-a, 0, 0, 0, 0, 0, 0, 1 - a]
            } else {
                vec![0; 8]
            }
        );
        assert_eq!(&data("G")[..8], data("A"));
        assert_eq!(&data("G")[8..], &[a; 4]);
        let repeated = [data("A"), data("A")].concat();
        assert_eq!(data("C"), repeated);
        assert_eq!(data("R"), repeated);
        assert_eq!(e.logical_variables["X"].data(), &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(e.logical_variables["Y"].data(), &[1]);
        assert_eq!(e.variables["I"].data(), &[1., 1., 1., 2.]);
    }
    for class in [
        "int8", "uint8", "int16", "uint16", "int32", "uint32", "single",
    ] {
        let mut e = Engine::default();
        e.execute(&format!(
            "A={class}(ones(2,2,2)); B=A+0.6; C=-A; D=abs(C); E=cat(4,A,A); [M,I]=max(A,[],3);"
        ))
        .unwrap();
        for name in ["A", "B", "C", "D", "E", "M"] {
            assert_eq!(e.array(name).unwrap().class(), class);
        }
        let matrust::Array::Typed(b) = e.array("B").unwrap() else {
            unreachable!()
        };
        let expected = if class == "single" {
            f64::from(1.6_f32)
        } else {
            2.
        };
        assert!((0..b.len()).all(|i| b.real(i) == expected));
    }
    let mut e = Engine::default();
    e.execute("C=cat(4,int64(zeros(0,2,2)),ones(2,2,2));")
        .unwrap();
    assert_eq!(e.array("C").unwrap().class(), "int64");
}

#[test]
fn round3_nan_extrema_complex_domains_and_empty_length() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/nd/round3.json")).unwrap();
    let mut e = Engine::default();
    e.execute("A=nan(2,2,2); [M,I]=max(A); [N,J]=min(A,[],3); B=reshape([NaN 2 3 NaN NaN NaN -Inf -Inf],2,2,2); [V,K]=max(B); S=sqrt(-ones(2,2,2)); L=log(-ones(2,2,2)); Z=length(zeros(0,3,4)); T=sqrt(single(ones(2,2,2))); ES=sum(zeros(1,0,3)); EM=mean(zeros(2,0,3),2);").unwrap();
    assert!(e.variables["M"].data().iter().all(|x| x.is_nan()));
    assert!(e.variables["N"].data().iter().all(|x| x.is_nan()));
    assert_eq!(e.variables["I"].data(), &[1.; 4]);
    assert_eq!(e.variables["J"].data(), &[1.; 4]);
    let maximum = e.variables["V"].data();
    assert_eq!(&maximum[..2], &[2., 3.]);
    assert!(maximum[2].is_nan());
    assert_eq!(maximum[3], f64::NEG_INFINITY);
    assert_eq!(e.variables["K"].data(), &[2., 1., 1., 1.]);
    for (name, key) in [("S", "sqrt_negative"), ("L", "log_negative")] {
        let matrust::Array::Complex(a) = e.array(name).unwrap() else {
            panic!("{name} requires complex output")
        };
        assert_eq!(a.dims().as_ref(), &[2, 2, 2]);
        let (re, im) = (
            reference[key][0].as_f64().unwrap(),
            reference[key][1].as_f64().unwrap(),
        );
        assert!(
            a.data()
                .iter()
                .all(|z| (z.re - re).abs() < 1e-15 && (z.im - im).abs() < 1e-15)
        );
    }
    assert_eq!(e.variables["Z"].data(), &[0.]);
    assert_eq!(e.array("T").unwrap().class(), "single");
    assert_eq!(e.array("ES").unwrap().dims().as_ref(), &[1, 1, 3]);
    assert_eq!(e.variables["ES"].data(), &[0.; 3]);
    assert_eq!(e.array("EM").unwrap().dims().as_ref(), &[2, 1, 3]);
    assert!(reference["empty_mean_all_nan"].as_bool().unwrap());
    assert!(e.variables["EM"].data().iter().all(|x| x.is_nan()));
}

#[test]
fn courses_multi_output_dispatch_keeps_nd_guard() {
    for source in [
        "[p,f]=periodogram(ones(1,1,2));",
        "[K,S,P]=lqr(-1,ones(1,1,2),1,eye(2));",
        "periodogram(ones(1,1,2));",
        "findpeaks(ones(1,1,3));",
        "step(tf(1,[1 1]),ones(1,1,2));",
        "rlocus(tf(1,[1 1]),ones(1,1,2));",
        "[y,t]=step(tf(1,[1 1]),ones(1,1,2));",
        "[num,den]=tfdata(tf(1,[1 1]),ones(1,1,2));",
        "[p,loc]=findpeaks(ones(1,1,3));",
        "[b,a]=butter(2,reshape([0.1 0.2],1,1,2));",
        "[x,f]=fminunc(@(x) sum(x(:).^2),ones(1,1,2));",
    ] {
        let error = Engine::default().execute(source).unwrap_err();
        assert!(
            error.contains("does not support N-D arrays yet"),
            "{source}: {error}"
        );
    }
}

#[test]
fn courses_dispatch_keeps_nd_guard_before_two_dimensional_kernels() {
    for source in [
        "y=periodogram(ones(2,2,2));",
        "y=hilbert(ones(2,2,2));",
        "y=ctrb(ones(2,2,2),ones(2,1));",
        "f=@ctrb; y=f(ones(2,2,2),ones(2,1));",
    ] {
        let error = Engine::default().execute(source).unwrap_err();
        assert!(
            error.contains("does not support N-D arrays yet"),
            "{source}: {error}"
        );
    }
}

/// Octave 10.3: `A(:,:,1)=[1 2 3 4]` errors (subscript 2-by-2, right side 1-by-4).
/// A right side is accepted when its non-singleton sizes match in order, including
/// `reshape(1:4,1,2,2)` and `reshape(1:4,2,1,2)`, both stored as `[1 2 3 4]`.
/// A scalar expands. One linear subscript checks the element count only.
/// MathWorks: non-scalar dimensions must match (MATLAB Answers 745702).
#[test]
fn review_d1112_nd_subsasgn_matches_non_singleton_dimensions() {
    let mut e = Engine::default();
    let mismatch = e
        .execute("A=zeros(2,2,2); A(:,:,1)=[1 2 3 4];")
        .unwrap_err();
    assert!(
        mismatch.contains("Subscripted assignment dimensions must agree"),
        "{mismatch}"
    );
    assert_eq!(e.variables["A"].data(), &[0.; 8]);
    for expr in [
        "A(:,:,1)=[1;2;3;4]",
        "A(:,:,1)=reshape(1:4,4,1,1)",
        "A(:,:,1)=reshape(1:4,1,1,4)",
        "A(:,:,1)=reshape(1:4,1,4,1)",
        "G=zeros(2,2); G(:,:,3)=[1 2 3 4]",
        "B=zeros(2,0,2); B(:,:,1)=zeros(0,2)",
        "C=zeros(2,2,2)+0i; C(:,:,1)=[1 2 3 4]",
        "T=zeros(2,2,2,'int8'); T(:,:,1)=int8([1 2 3 4])",
        "L=false(2,2,2); L(:,:,1)=logical([1 0 1 0])",
        "S=zeros(2,2,2,'single'); S(:,:,1)=single([1 2 3 4])",
    ] {
        let err = e.execute(&format!("{expr};")).unwrap_err();
        assert!(
            err.contains("Subscripted assignment dimensions must agree"),
            "{expr}: {err}"
        );
    }
    let count = e.execute("A(:,:,1)=[1 2 3];").unwrap_err();
    assert!(count.contains("different number of elements"), "{count}");

    e.execute("A=zeros(2,2,2); A(:,:,1)=reshape(1:4,1,2,2);")
        .unwrap();
    assert_eq!(e.variables["A"].data(), &[1., 2., 3., 4., 0., 0., 0., 0.]);
    e.execute("A(:,:,1)=reshape(1:4,2,1,2);").unwrap();
    assert_eq!(e.variables["A"].data(), &[1., 2., 3., 4., 0., 0., 0., 0.]);
    e.execute("A(:,:,1)=reshape(1:4,2,2,1);").unwrap();
    assert_eq!(e.variables["A"].data(), &[1., 2., 3., 4., 0., 0., 0., 0.]);
    e.execute("A(:,:,1)=reshape(1:4,1,1,2,2);").unwrap();
    assert_eq!(e.variables["A"].data(), &[1., 2., 3., 4., 0., 0., 0., 0.]);
    e.execute("A(:,:,1)=reshape(1:4,2,2)';").unwrap();
    assert_eq!(e.variables["A"].data(), &[1., 3., 2., 4., 0., 0., 0., 0.]);
    e.execute("A(:,:,1)=7;").unwrap();
    assert_eq!(e.variables["A"].data(), &[7., 7., 7., 7., 0., 0., 0., 0.]);
    e.execute("A=zeros(2,2,2); A(1,1,:)=[9 8]; A(2,2,:)=[7;6];")
        .unwrap();
    assert_eq!(e.variables["A"].data(), &[9., 0., 0., 7., 8., 0., 0., 6.]);

    e.execute("A=zeros(2,2,2); A(:)=1:8;").unwrap();
    assert_eq!(
        e.variables["A"].data(),
        &(1..=8).map(f64::from).collect::<Vec<_>>()
    );
    let folded = e.execute("A(:,:)=1:8;").unwrap_err();
    assert!(
        folded.contains("Subscripted assignment dimensions must agree"),
        "{folded}"
    );
    e.execute("A(:,:)=reshape(1:8,2,4);").unwrap();
    assert_eq!(
        e.variables["A"].data(),
        &(1..=8).map(f64::from).collect::<Vec<_>>()
    );

    e.execute("B=zeros(2,0,2); B(:,:,1)=zeros(2,0); B(:,:,1)=0;")
        .unwrap();
    assert_eq!(e.array("B").unwrap().dims().as_ref(), &[2, 0, 2]);

    e.execute(
        "C=zeros(2,2,2)+0i; C(:,:,1)=reshape(1:4,1,2,2); P=real(C(:,:,1)); Q=imag(C(:,:,1));",
    )
    .unwrap();
    assert_eq!(e.variables["P"].data(), &[1., 2., 3., 4.]);
    assert_eq!(e.variables["Q"].data(), &[0.; 4]);

    e.execute("T=zeros(2,2,2,'int8'); T(:,:,1)=int8(reshape(1:4,1,2,2)); T(:,:,2)=int8(5);")
        .unwrap();
    let matrust::Array::Typed(t) = e.array("T").unwrap() else {
        panic!("int8 page");
    };
    assert_eq!(
        (0..t.len()).map(|i| t.real(i)).collect::<Vec<_>>(),
        [1., 2., 3., 4., 5., 5., 5., 5.]
    );

    e.execute("L=false(2,2,2); L(:,:,1)=true(1,2,2);").unwrap();
    assert_eq!(e.logical_variables["L"].data(), &[1, 1, 1, 1, 0, 0, 0, 0]);

    e.execute("S=zeros(2,2,2,'single'); S(:,:,1)=single(reshape(1:4,2,1,2));")
        .unwrap();
    assert_eq!(e.array("S").unwrap().class(), "single");
    let matrust::Array::Typed(s) = e.array("S").unwrap() else {
        panic!("single page");
    };
    assert_eq!(
        (0..4).map(|i| s.real(i)).collect::<Vec<_>>(),
        [1., 2., 3., 4.]
    );

    e.execute("G=zeros(2,2); G(:,:,3)=ones(2,2);").unwrap();
    assert_eq!(e.array("G").unwrap().dims().as_ref(), &[2, 2, 3]);
    assert_eq!(
        e.variables["G"].data(),
        &[0., 0., 0., 0., 0., 0., 0., 0., 1., 1., 1., 1.]
    );
}
