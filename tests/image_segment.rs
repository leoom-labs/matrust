//! Independent references: tool/refs/image/image_segment.py (SciPy/NumPy/
//! scikit-image), plus explicitly stated public formulas and hand fixtures.
use matrust::Engine;
fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e
}
fn values(e: &mut Engine, expression: &str) -> Vec<f64> {
    e.execute(&format!("zz__=double({expression});")).unwrap();
    e.variables["zz__"].data().to_vec()
}
fn close(got: &[f64], expected: &[f64], tol: f64) {
    assert_eq!(got.len(), expected.len());
    for (i, (&g, &x)) in got.iter().zip(expected).enumerate() {
        assert!((g - x).abs() <= tol, "pixel {i}: {g} != {x}");
    }
}

#[test]
#[allow(clippy::approx_constant)] // Preserve independent reference literals.
fn distance_four_metrics_and_nearest_indices() {
    // SciPy edt/cdt and public quasi-Euclidean formula, column-major.
    #[rustfmt::skip]
    let expected = [
        [1.4142135623730951,1.,1.4142135623730951,2.23606797749979,3.1622776601683795,1.,0.,1.,2.,2.23606797749979,1.4142135623730951,1.,1.4142135623730951,1.,1.4142135623730951,2.23606797749979,2.,1.,0.,1.,3.1622776601683795,2.23606797749979,1.4142135623730951,1.,1.4142135623730951],
        [2.,1.,2.,3.,4.,1.,0.,1.,2.,3.,2.,1.,2.,1.,2.,3.,2.,1.,0.,1.,4.,3.,2.,1.,2.],
        [1.,1.,1.,2.,3.,1.,0.,1.,2.,2.,1.,1.,1.,1.,1.,2.,2.,1.,0.,1.,3.,2.,1.,1.,1.],
        [1.4142135623730951,1.,1.4142135623730951,2.414213562373095,3.414213562373095,1.,0.,1.,2.,2.414213562373095,1.4142135623730951,1.,1.4142135623730951,1.,1.4142135623730951,2.414213562373095,2.,1.,0.,1.,3.414213562373095,2.414213562373095,1.4142135623730951,1.,1.4142135623730951],
    ];
    let mut e = run("BW=false(5);BW(2,2)=true;BW(4,4)=true;");
    for (m, method) in ["euclidean", "cityblock", "chessboard", "quasi-euclidean"]
        .iter()
        .enumerate()
    {
        for class in ["logical", "uint8", "uint16", "double", "single"] {
            e.execute(&format!("[D,ix]=bwdist({class}(BW),'{method}');"))
                .unwrap();
            close(&values(&mut e, "D"), &expected[m], 1e-6);
            assert_eq!(
                values(&mut e, "[isa(D,'single') isa(ix,'uint32')]"),
                [1., 1.]
            );
            for (p, x) in values(&mut e, "ix").into_iter().enumerate() {
                let q = x as usize - 1;
                assert!([6, 18].contains(&q));
                let dr = (p % 5).abs_diff(q % 5) as f64;
                let dc = (p / 5).abs_diff(q / 5) as f64;
                let implied = match m {
                    0 => (dr * dr + dc * dc).sqrt(),
                    1 => dr + dc,
                    2 => dr.max(dc),
                    _ => dr.max(dc) + (std::f64::consts::SQRT_2 - 1.) * dr.min(dc),
                };
                assert!((implied - expected[m][p]).abs() < 1e-6);
            }
        }
        e.execute(&format!("[D,ix]=bwdist(false(5),'{method}');"))
            .unwrap();
        assert!(
            values(&mut e, "D")
                .iter()
                .all(|x| x.is_infinite() && *x > 0.)
        );
        assert_eq!(values(&mut e, "ix"), [0.; 25]);
        e.execute(&format!("[D,ix]=bwdist(true(5),'{method}');"))
            .unwrap();
        assert_eq!(values(&mut e, "D"), [0.; 25]);
        assert_eq!(
            values(&mut e, "ix"),
            (1..=25).map(f64::from).collect::<Vec<_>>()
        );
    }
    assert_eq!(values(&mut e, "size(bwdist(false(0,5)))"), [0., 5.]);
}

#[test]
fn watershed_hand_9x9_plateaus_and_overlapping_blobs() {
    let mut e = run("A=repmat([2 1 0 1 2 1 0 1 2],9,1);L=watershed(A,4);");
    let expected: Vec<_> = [1., 1., 1., 1., 0., 2., 2., 2., 2.]
        .into_iter()
        .flat_map(|x| [x; 9])
        .collect();
    assert_eq!(values(&mut e, "L"), expected);
    assert_eq!(values(&mut e, "watershed(A,8)"), expected);
    assert_eq!(values(&mut e, "watershed(ones(5))"), [1.; 25]);
    e.execute("[X,Y]=meshgrid(1:16,1:16);BW=(X-6).^2+(Y-8).^2<=25 | (X-11).^2+(Y-8).^2<=25;D=-bwdist(~BW);L=watershed(D);L(~BW)=0;").unwrap();
    assert_eq!(values(&mut e, "max(L(:))"), [2.]);
    assert_eq!(values(&mut e, "L(8,[6 11])"), [1., 2.]);
    assert!(values(&mut e, "nnz(BW & L==0)")[0] > 0.);
    assert_eq!(values(&mut e, "isequal(L,watershed(D).*double(BW))"), [1.]);
}

#[test]
fn chan_vese_seeded_noisy_disc_and_edge_square() {
    // Analytic disc and explicitly seeded portable noise; independent
    // scikit-image morphological_chan_vese achieves IoU=1 on this fixture.
    let mut source = String::from("I=zeros(64);");
    let mut state = 19u32;
    // Keep each statement within the engine's 256-node expression limit.
    for column in 1..=64 {
        source.push_str(&format!("I(:,{column})=["));
        for _ in 0..64 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            source.push_str(&format!(
                "{:.15} ",
                (f64::from(state) / 4294967296. - 0.5) * 0.15
            ));
        }
        source.push_str("]';");
    }
    source.push_str("[X,Y]=meshgrid(0:63,0:63);truth=(X-31).^2+(Y-31).^2<=17^2;I=I+double(truth);seed=(X-31).^2+(Y-31).^2<=23^2;B=activecontour(I,seed,100);");
    let mut e = run(&source);
    let iou = values(&mut e, "nnz(B & truth)/nnz(B | truth)")[0];
    assert!(iou > 0.95, "disc IoU {iou}");
    assert_eq!(values(&mut e, "isa(B,'logical')"), [1.]);
    e.execute("I=zeros(16);I(5:12,5:12)=1;seed=false(16);seed(3:14,3:14)=true;B=activecontour(I,seed,200,'edge');truth=logical(I);").unwrap();
    let iou = values(&mut e, "nnz(B & truth)/nnz(B | truth)")[0];
    assert!(iou > 0.85, "edge square IoU {iou}");
    e.execute(
        "C=activecontour(cat(3,I,I,I),seed,100,'Chan-Vese','SmoothFactor',0,'ContractionBias',0);",
    )
    .unwrap();
    assert_eq!(values(&mut e, "isequal(C,truth)"), [1.]);
    assert_eq!(
        values(&mut e, "isequal(activecontour(I,double(seed)),truth)"),
        [1.]
    );
    e.execute("S=activecontour(I,seed,1);T=activecontour(I,seed,100);")
        .unwrap();
    assert!(values(&mut e, "nnz(S)")[0] > values(&mut e, "nnz(T)")[0]);
    e.execute("M=false(7);M(4,4)=true;I=zeros(7);").unwrap();
    assert_eq!(
        values(&mut e, "nnz(activecontour(I,M,1,'ContractionBias',-1))"),
        [9.]
    );
    assert_eq!(
        values(&mut e, "nnz(activecontour(I,M,1,'ContractionBias',1))"),
        [0.]
    );
    assert_eq!(
        values(&mut e, "nnz(activecontour(I,M,1,'SmoothFactor',1))"),
        [0.]
    );
}

#[test]
fn kmeans_flat_regions_classes_and_rng() {
    let mut e = run(
        "I=uint8(repmat([10 10 10 100 100 100 220 220 220],6,1));rng(17);[L,C]=imsegkmeans(I,3);rng(17);[M,D]=imsegkmeans(I,3);",
    );
    assert_eq!(
        values(
            &mut e,
            "[isequal(L,M) isequal(C,D) isa(L,'uint8') isa(C,'uint8')]"
        ),
        [1.; 4]
    );
    assert_eq!(values(&mut e, "sort(C)'"), [10., 100., 220.]);
    // The same core still supports its ordinary (no image threshold) entry.
    e.execute("[K,KC]=kmeans(double(I(:)),3,'Replicates',3);")
        .unwrap();
    close(&values(&mut e, "sort(KC)'"), &[10., 100., 220.], 1e-10);
    assert_eq!(values(&mut e, "size(C)"), [3., 1.]);
    assert_eq!(values(&mut e, "size(L)"), [6., 9.]);
    assert_eq!(
        values(&mut e, "isequal(reshape(C(double(L(:))),6,9),I)"),
        [1.]
    );
    e.execute("[L,C]=imsegkmeans(single(cat(3,I,2*double(I))),3,'NumAttempts',2,'MaxIterations',20,'Threshold',1e-6,'NormalizeInput',false);").unwrap();
    close(&values(&mut e, "sort(C(:,1))'"), &[10., 100., 220.], 1e-6);
    assert_eq!(values(&mut e, "[isa(C,'single') size(C)]"), [1., 3., 2.]);
    e.execute("[L,C]=imsegkmeans(uint16(I),1);").unwrap();
    assert_eq!(values(&mut e, "C"), [110.]);
    assert_eq!(values(&mut e, "isa(C,'uint16')"), [1.]);
    e.execute("[L,C]=imsegkmeans(single(ones(5)),2);").unwrap();
    close(&values(&mut e, "C"), &[1., 1.], 1e-6);
}

#[test]
fn grayconnected_seed_relative_tolerance_and_classes() {
    let mut e = run(
        "I=[0 .05 .2 .7 .8; .05 .1 .2 .8 .9; .2 .2 .2 .9 1; .7 .8 .9 .1 .1; .8 .9 1 .1 .1];B=grayconnected(I,1,1);",
    );
    assert_eq!(values(&mut e, "nnz(B)"), [4.]);
    assert_eq!(values(&mut e, "B(4,4)"), [0.]);
    assert_eq!(
        values(&mut e, "nnz(grayconnected(single(I),1,1,.11))"),
        [4.]
    );
    assert_eq!(
        values(&mut e, "nnz(grayconnected(uint8(I*100),1,1))"),
        [13.]
    );
    assert_eq!(
        values(&mut e, "nnz(grayconnected(uint16(I*100),1,1))"),
        [13.]
    );
    assert_eq!(values(&mut e, "nnz(grayconnected(I,1,1,0))"), [1.]);
    assert_eq!(values(&mut e, "isa(B,'logical')"), [1.]);
}

#[test]
fn boundaries_square_holes_nested_parents_and_connectivity() {
    let mut e = run("BW=false(5);BW(2:4,2:4)=true;[B,L,N,A]=bwboundaries(BW);");
    assert_eq!(
        values(&mut e, "[size(B) N nnz(A) issparse(A) islogical(A)]"),
        [1., 1., 1., 0., 1., 1.]
    );
    // Moore tracing starts at (2,2), clockwise, and repeats its start.
    assert_eq!(
        values(&mut e, "B{1}"),
        [
            2., 2., 2., 3., 4., 4., 4., 3., 2., 2., 3., 4., 4., 4., 3., 2., 2., 2.
        ]
    );
    assert_eq!(values(&mut e, "L"), values(&mut e, "double(BW)"));
    e.execute("BW=false(9);BW(2:8,2:8)=true;BW(3:7,3:7)=false;BW(4:6,4:6)=true;[B,L,N,A]=bwboundaries(BW);").unwrap();
    assert_eq!(values(&mut e, "[numel(B) N max(L(:))]"), [3., 2., 3.]);
    assert_eq!(
        values(&mut e, "full(A)"),
        [0., 0., 1., 0., 0., 0., 0., 1., 0.]
    );
    assert_eq!(values(&mut e, "[L(2,2) L(3,3) L(4,4)]"), [1., 3., 2.]);
    e.execute("[B,L,N,A]=bwboundaries(BW,'noholes');").unwrap();
    assert_eq!(values(&mut e, "[numel(B) N L(3,3)]"), [2., 2., 0.]);
    assert_eq!(values(&mut e, "full(A)"), [0., 1., 0., 0.]);
    e.execute("[B,L,N]=bwboundaries(logical(eye(5)),4);")
        .unwrap();
    assert_eq!(values(&mut e, "N"), [5.]);
    e.execute("[B,L,N]=bwboundaries(logical(eye(5)),8);")
        .unwrap();
    assert_eq!(values(&mut e, "N"), [1.]);
    assert_eq!(values(&mut e, "isequal(B{1}(1,:),B{1}(end,:))"), [1.]);
    e.execute("[B,L,N,A]=bwboundaries(false(5));").unwrap();
    assert_eq!(values(&mut e, "[numel(B) N numel(A)]"), [0.; 3]);
}

#[test]
fn perimeter_area_filters_selection_and_ties() {
    let mut e = run("BW=false(5);BW(2:4,2:4)=true;P=bwperim(BW);");
    assert_eq!(values(&mut e, "[nnz(P) P(3,3) bwarea(BW)]"), [8., 0., 9.]);
    close(
        &values(&mut e, "[bwarea(1) bwarea(eye(2)) bwarea([1 1;1 0])]"),
        &[1., 2.25, 3.125],
        1e-12,
    );
    assert_eq!(values(&mut e, "nnz(bwperim(true(5)))"), [16.]);
    e.execute("BW=false(9);BW(2:3,2:3)=true;BW(6:7,2:3)=true;BW(4,7)=true;F=bwareafilt(BW,1);")
        .unwrap();
    assert_eq!(values(&mut e, "[nnz(F) F(2,2) F(6,2)]"), [4., 1., 0.]);
    assert_eq!(values(&mut e, "nnz(bwareafilt(BW,[4 4]))"), [8.]);
    assert_eq!(values(&mut e, "nnz(bwareafilt(BW,1,'smallest',4))"), [1.]);
    assert_eq!(
        values(&mut e, "isequal(bwpropfilt(BW,'Area',1,'largest'),F)"),
        [1.]
    );
    assert_eq!(values(&mut e, "nnz(bwpropfilt(BW,'Extent',[1 1]))"), [9.]);
    e.execute("I=zeros(9);I(6:7,2:3)=5;F=bwpropfilt(BW,I,'MeanIntensity',[4 6],4);")
        .unwrap();
    assert_eq!(values(&mut e, "[nnz(F) F(6,2)]"), [4., 1.]);
    e.execute("[F,ix]=bwselect(BW,[2 7],[6 4],8);").unwrap();
    assert_eq!(values(&mut e, "nnz(F)"), [5.]);
    assert_eq!(values(&mut e, "ix"), [15., 16., 24., 25., 58.]);
    for property in [
        "EquivDiameter",
        "FilledArea",
        "MajorAxisLength",
        "MinorAxisLength",
        "Eccentricity",
        "Orientation",
        "Perimeter",
    ] {
        assert_eq!(
            values(
                &mut e,
                &format!("nnz(bwpropfilt(BW,'{property}',[-Inf Inf]))")
            ),
            [9.]
        );
    }
}

#[test]
fn segmentation_rejects_invalid_inputs_and_options() {
    for source in [
        "bwdist(ones(2,2,2))",
        "bwdist(false(2048,1025))",
        "bwboundaries(false(1025,512))",
        "bwdist(1,'bogus')",
        "[a,b,c]=bwdist(1)",
        "watershed(ones(5),6)",
        "watershed(NaN)",
        "grayconnected(zeros(5),0,1)",
        "grayconnected(zeros(5),1,1,-1)",
        "grayconnected(zeros(5),6,1)",
        "activecontour(zeros(5),false(4))",
        "activecontour(zeros(5),ones(5)*.5)",
        "activecontour(zeros(5),false(5),0)",
        "activecontour(zeros(5),false(5),'bogus')",
        "activecontour(zeros(5),false(5),'SmoothFactor',-1)",
        "activecontour(zeros(5,5,3),false(5),'edge')",
        "activecontour(zeros(5),false(5),'typo',1)",
        "imsegkmeans(zeros(5),2)",
        "imsegkmeans(single(zeros(5)),26)",
        "imsegkmeans(single(zeros(5)),2,'Threshold',0)",
        "imsegkmeans(single(zeros(5)),2,'typo',1)",
        "bwboundaries(true(5),6)",
        "bwboundaries(true(5),'typo')",
        "bwperim(true(5),6)",
        "bwarea(ones(2,2,2))",
        "bwareafilt(true(5),0)",
        "bwareafilt(true(5),[3 2])",
        "bwareafilt(true(5),1,'typo')",
        "bwpropfilt(true(5),'Centroid',1)",
        "bwpropfilt(true(5),zeros(4),'MeanIntensity',1)",
        "bwselect(true(5),[1 2],1,8)",
    ] {
        assert!(Engine::default().execute(source).is_err(), "{source}");
    }
}

#[test]
fn segmentation_help_examples_run() {
    for name in [
        "bwdist",
        "watershed",
        "activecontour",
        "imsegkmeans",
        "grayconnected",
        "bwboundaries",
        "bwperim",
        "bwarea",
        "bwareafilt",
        "bwpropfilt",
        "bwselect",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        let mut e = Engine::default();
        let help = e
            .execute(&format!("help {name}"))
            .unwrap()
            .output
            .join("\n");
        assert!(help.contains("Syntax:"), "{name}");
        for example in help.lines().filter_map(|s| s.strip_prefix("Example: ")) {
            e.execute(example)
                .unwrap_or_else(|err| panic!("{name}: {example}: {err}"));
        }
    }
}

#[test]
fn segmentation_benchmark_and_cancellation() {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    // Release-mode benchmark under explicit, sequential 1- and 8-worker
    // contexts. No nested pools, no global pool initialisation.
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let mut e=run("[X,Y]=meshgrid(1:64,1:64);BW=(X-22).^2+(Y-32).^2<100 | (X-42).^2+(Y-32).^2<100;I=single(BW);");
            let begin=Instant::now();
            for _ in 0..5 { e.execute("D=bwdist(~BW);W=watershed(-D);B=activecontour(I,BW,20);[L,C]=imsegkmeans(I,2);[P,Q,N,A]=bwboundaries(BW);F=bwareafilt(BW,1);P=bwperim(BW);T=bwarea(BW);F=bwpropfilt(BW,'Area',1);S=bwselect(BW,22,32,8);G=grayconnected(I,32,22);D=bwdist(BW,'cityblock');D=bwdist(BW,'chessboard');D=bwdist(BW,'quasi-euclidean');E=activecontour(I,BW,10,'edge');").unwrap(); }
            eprintln!("image_segment workers={workers}, 64x64 all-segmentation pipeline x5: {:?}", begin.elapsed());
        });
    }
    let mut e = Engine::default();
    e.execute("[X,Y]=meshgrid(1:512,1:512);I=double(X>256);M=(X-256).^2+(Y-256).^2<200^2;")
        .unwrap();
    let cancel = e.cancellation();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        cancel.store(true, Ordering::Relaxed);
    });
    let begin = Instant::now();
    let error = e.execute("activecontour(I,M,200000,'edge');").unwrap_err();
    timer.join().unwrap();
    assert!(error.to_string().contains("cancelled"), "{error}");
    assert!(begin.elapsed() < Duration::from_secs(5));
    e.cancellation().store(false, Ordering::Relaxed);
    e.execute("bwdist(true(5));").unwrap();
}
