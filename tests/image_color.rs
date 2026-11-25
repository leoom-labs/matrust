//! Independent fixtures: bash tool/ref-python.sh tool/refs/image/image_color.py.
//! Float tolerances permit platform libm differences; integer/logical exact.
#![allow(clippy::excessive_precision)]
use matrust::{Engine, Matrix};
use std::time::Instant;

fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}\n{err}"));
    e
}
fn values(e: &mut Engine, source: &str) -> Vec<f64> {
    e.execute(&format!("zz__ = double({source});"))
        .unwrap_or_else(|err| panic!("{source}\n{err}"));
    e.variables["zz__"].data().to_vec()
}
fn close(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len());
    for (i, (&x, &y)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (x - y).abs() <= tolerance,
            "at {i}: got {x:.17}, expected {y:.17}, tolerance {tolerance}"
        );
    }
}
fn error(source: &str) -> String {
    Engine::default().execute(source).unwrap_err()
}
const RGB: &str = "[0 0 0; 1 1 1; 1 0 0; 0 1 0; 0 0 1; .2 .3 .4]";
const FIXED: &str =
    "[0 1 2 3 4;45 46 47 48 49;120 121 122 123 124;190 191 192 193 194;251 252 253 254 255]";
const LABELS: &str = "[0 0 0 0 0;0 1 1 0 0;0 1 1 2 0;0 0 2 2 0;0 0 0 0 0]";

#[test]
fn lab_published_matrices_colormaps_and_whitepoints() {
    let mut e = run(&format!("C={RGB};"));
    for (options, expected) in [
        ("", LAB_D65),
        (",'WhitePoint','d50'", LAB_D50),
        (",'ColorSpace','adobe-rgb-1998'", LAB_ADOBE),
        (",'ColorSpace','linear-rgb'", LAB_LINEAR),
    ] {
        close(
            &values(&mut e, &format!("rgb2lab(C{options})")),
            expected,
            1e-10,
        );
        close(
            &values(&mut e, &format!("lab2rgb(rgb2lab(C{options}){options})")),
            &values(&mut run(&format!("C={RGB};")), "C"),
            1e-10,
        );
    }
    for white in [
        "'d65'",
        "'d50'",
        "'a'",
        "'c'",
        "'e'",
        "'icc'",
        "[.9642 1 .8251]",
    ] {
        e.execute(&format!(
            "R=lab2rgb(rgb2lab(C,'WhitePoint',{white}),'WhitePoint',{white});"
        ))
        .unwrap();
        close(&values(&mut e, "R-C"), &[0.; 18], 1e-10);
    }
    assert_eq!(
        values(
            &mut e,
            "isequal(rgb2lab(C,'WhitePoint','d50'),rgb2lab(C,'WhitePoint',[.9642 1 .8251]))"
        ),
        [1.]
    );
}

#[test]
fn lab_images_classes_outputtypes_and_unclipped_float_gamut() {
    let mut e = run("C=reshape(mod((0:74)*37,256),5,5,3)/255;");
    for class in ["double", "single", "uint8", "uint16"] {
        let expression = match class {
            "uint8" => "uint8(C*255)",
            "uint16" => "uint16(C*65535)",
            "single" => "single(C)",
            _ => "C",
        };
        e.execute(&format!("A={expression}; L=rgb2lab(A); R=lab2rgb(L);"))
            .unwrap();
        assert_eq!(values(&mut e, "size(L)"), [5., 5., 3.]);
        // Review item 11: single input gives single Lab; every other class gives double.
        let lab_class = if class == "single" {
            "single"
        } else {
            "double"
        };
        assert_eq!(values(&mut e, &format!("isa(L(:),'{lab_class}')")), [1.]);
        let tol = if class == "single" { 1e-5 } else { 1e-10 };
        close(&values(&mut e, "double(R)-im2double(A)"), &[0.; 75], tol);
        for output in ["double", "single", "uint8", "uint16"] {
            e.execute(&format!("Q=lab2rgb(L,'OutputType','{output}');"))
                .unwrap();
            assert_eq!(values(&mut e, &format!("isa(Q(:),'{output}')")), [1.]);
            let expected = match output {
                "uint8" => "im2uint8(A)",
                "uint16" => "im2uint16(A)",
                "single" => "im2single(A)",
                _ => "im2double(A)",
            };
            close(
                &values(&mut e, &format!("double(Q)-double({expected})")),
                &[0.; 75],
                if class == "single" {
                    1e-5
                } else if output == "single" {
                    1e-6
                } else {
                    1e-10
                },
            );
        }
    }
    assert_eq!(
        values(&mut e, "isa(lab2rgb(single([50 0 0])),'single')"),
        [1.]
    );
    // Single in, single out for the YCbCr pair too; imsegkmeans accepts single Lab.
    assert_eq!(
        values(
            &mut e,
            "[isa(rgb2ycbcr(single(C)),'single') isa(ycbcr2rgb(rgb2ycbcr(single(C))),'single')]"
        ),
        [1., 1.]
    );
    e.execute("S=imsegkmeans(rgb2lab(single(C)),2);").unwrap();
    assert_eq!(values(&mut e, "size(S)"), [5., 5.]);
    assert_eq!(
        values(
            &mut e,
            "any(lab2rgb([50 150 150])>1) | any(lab2rgb([50 150 150])<0)"
        ),
        [1.]
    );
}

#[test]
fn ycbcr_bt601_classes_colormaps_images_and_inverse() {
    let mut e = run(&format!("C={RGB};"));
    close(&values(&mut e, "rgb2ycbcr(C)"), YCBCR, 1e-10);
    close(
        &values(&mut e, "ycbcr2rgb(rgb2ycbcr(C))-C"),
        &[0.; 18],
        1e-10,
    );
    // First five rows are exact integer primaries; noninteger source rounding
    // is independently reproduced by NumPy in the reference generator.
    for (class, scale, expected) in [("uint8", 255, YCBCR_U8), ("uint16", 65535, YCBCR_U16)] {
        e.execute(&format!(
            "A={class}(C*{scale}); Y=rgb2ycbcr(A); B=ycbcr2rgb(Y);"
        ))
        .unwrap();
        assert_eq!(values(&mut e, "Y"), expected);
        assert_eq!(
            values(&mut e, &format!("isa(Y,'{class}') && isa(B,'{class}')")),
            [1.]
        );
    }
    close(&values(&mut e, "rgb2ycbcr(single(C))"), YCBCR, 1e-6);
    e.execute("A=reshape(repmat(C(1:5,:),5,1),5,5,3); B=ycbcr2rgb(rgb2ycbcr(A));")
        .unwrap();
    close(&values(&mut e, "B-A"), &[0.; 75], 1e-10);
    assert_eq!(values(&mut e, "size(B)"), [5., 5., 3.]);
    close(
        &values(&mut e, "ycbcr2rgb([16 128 128;235 128 128]/255)"),
        &[0., 1., 0., 1., 0., 1.],
        1e-10,
    );
}

#[test]
fn hsv_colormaps_cover_integer_single_and_double() {
    let mut e = run("C=[1 0 0;0 1 0;0 0 1;1 1 1;0 0 0];");
    for input in ["C", "single(C)", "uint8(C*255)", "uint16(C*65535)"] {
        e.execute(&format!("H=rgb2hsv({input}); R=hsv2rgb(H);"))
            .unwrap();
        close(
            &values(&mut e, "H"),
            &[
                0.,
                1. / 3.,
                2. / 3.,
                0.,
                0.,
                1.,
                1.,
                1.,
                0.,
                0.,
                1.,
                1.,
                1.,
                1.,
                0.,
            ],
            1e-6,
        );
        close(&values(&mut e, "R-C"), &[0.; 15], 1e-6);
        assert_eq!(values(&mut e, "size(H)"), [5., 3.]);
    }
}

#[test]
fn multithresh_matches_skimage_classes_metric() {
    let mut e = run(&format!("A={FIXED};"));
    for class in ["uint8", "uint16", "double", "single"] {
        let input = match class {
            "uint16" => "uint16(A*257)",
            "uint8" => "uint8(A)",
            "single" => "single(A/255)",
            _ => "A/255",
        };
        e.execute(&format!("I={input}; [t,m]=multithresh(I);"))
            .unwrap();
        let scale = match class {
            "uint8" => 255.,
            "uint16" => 65535.,
            _ => 1.,
        };
        // N=1 uses the observed range like N>1 (skimage threshold_multiotsu, classes=2).
        close(&values(&mut e, "m"), &[MULTI_METRICS[0]], 1e-6);
        assert_eq!(values(&mut e, &format!("isa(t,'{class}')")), [1.]);
        for (n, reference) in [(1, MULTI_1), (2, MULTI_2), (3, MULTI_3)] {
            let expected: Vec<f64> = reference.iter().map(|v| v * scale / 255.).collect();
            close(
                &values(&mut e, &format!("multithresh(I,{n})")),
                &expected,
                if class == "single" { 1e-6 } else { 1e-10 },
            );
        }
    }
    // Review item 10: two values split inside [10, 20) (skimage: MULTI_PAIR) with em = 1.
    e.execute("[t,m]=multithresh([10 20],1);").unwrap();
    let t = values(&mut e, "t")[0];
    assert!((10. ..20.).contains(&t) && t <= MULTI_PAIR[0], "{t}");
    close(&values(&mut e, "m"), &[1.], 1e-12);
    e.execute("[t,m]=multithresh(ones(5),3);").unwrap();
    assert_eq!(values(&mut e, "t"), [1., 1., 1.]);
    assert_eq!(values(&mut e, "m"), [0.]);
    e.execute("[t,m]=multithresh(uint8([0 128 255]),2);")
        .unwrap();
    close(&values(&mut e, "m"), &[1.], 1e-12);
    assert_eq!(
        values(&mut e, "size(multithresh(uint8(0:255),20))"),
        [1., 20.]
    );
}

#[test]
fn imquantize_boundary_inclusion_values_class_index_nd() {
    let mut e = run("A=reshape(0:24,5,5); [Q,I]=imquantize(A,[5 12 20],uint16([10 20 30 40]));");
    assert_eq!(
        values(&mut e, "I"),
        [
            1., 1., 1., 1., 1., 1., 2., 2., 2., 2., 2., 2., 2., 3., 3., 3., 3., 3., 3., 3., 3., 4.,
            4., 4., 4.
        ]
    );
    assert_eq!(values(&mut e, "Q-uint16(I*10)"), [0.; 25]);
    assert_eq!(
        values(&mut e, "isa(Q(:),'uint16') && isa(I,'double')"),
        [1.]
    );
    for class in ["double", "single", "uint8", "uint16", "logical"] {
        e.execute(&format!(
            "V={class}([0 1]); [Q,I]=imquantize(reshape(0:24,5,1,5),12,V);"
        ))
        .unwrap();
        assert_eq!(values(&mut e, &format!("isa(Q(:),'{class}')")), [1.]);
        assert_eq!(values(&mut e, "size(Q)"), [5., 1., 5.]);
    }
    e.execute("[Q,I]=imquantize(A,[5 12 20]);").unwrap();
    assert_eq!(values(&mut e, "isequal(Q,I)"), [1.]);
}

#[test]
fn adaptive_statistics_match_scipy_and_binarization_polarity() {
    let mut e = run("A=reshape(0:24,5,5)/24;");
    for (statistic, expected) in [
        ("mean", ADAPT_MEAN),
        ("median", ADAPT_MEDIAN),
        ("gaussian", ADAPT_GAUSSIAN),
    ] {
        e.execute(&format!(
            "T=adaptthresh(A,.5,'NeighborhoodSize',[3 5],'Statistic','{statistic}');"
        ))
        .unwrap();
        close(&values(&mut e, "T"), expected, 1e-12);
        assert_eq!(values(&mut e, "isa(T,'double')"), [1.]);
        e.execute(&format!("B=imbinarize(A,'adaptive','Sensitivity',.5,'NeighborhoodSize',[3 5],'Statistic','{statistic}');")).unwrap();
        assert_eq!(values(&mut e, "B"), values(&mut e, "A>T"));
        e.execute(&format!("D=imbinarize(A,'adaptive','Sensitivity',.5,'NeighborhoodSize',[3 5],'Statistic','{statistic}','ForegroundPolarity','dark');")).unwrap();
        assert_eq!(values(&mut e, "D"), values(&mut e, "A<T"));
        assert_eq!(values(&mut e, "isequal(imbinarize(A,T),A>T)"), [1.]);
    }
    for input in ["uint8(A*240)", "uint16(A*65520)", "single(A)"] {
        e.execute(&format!(
            "I={input}; T=adaptthresh(I,.5,'NeighborhoodSize',1);"
        ))
        .unwrap();
        close(&values(&mut e, "T-im2double(I)"), &[0.; 25], 1e-12);
    }
    assert_eq!(
        values(&mut e, "all(all(adaptthresh(A,.8)<=adaptthresh(A,.2)))"),
        [1.]
    );
    assert_eq!(
        values(
            &mut e,
            "all(all(adaptthresh(A,.8,'ForegroundPolarity','dark')>=adaptthresh(A,.2,'ForegroundPolarity','dark')))"
        ),
        [1.]
    );
    assert_eq!(
        values(
            &mut e,
            "size(adaptthresh(ones(5,5,5),'NeighborhoodSize',[3 3 3]))"
        ),
        [5., 5., 5.]
    );
    close(
        &values(&mut e, "adaptthresh(.4,.5,'NeighborhoodSize',100001)"),
        &[0.4],
        1e-12,
    );
}

#[test]
fn labels_overlays_and_pixels_have_exact_documented_colors() {
    let mut e = run(&format!(
        "L={LABELS}; C=[1 0 0;0 1 0]; A=uint8(100*ones(5)); R=label2rgb(L,C);"
    ));
    assert_eq!(values(&mut e, "isa(R(:),'uint8')"), [1.]);
    assert_eq!(values(&mut e, "size(R)"), [5., 5., 3.]);
    assert_eq!(
        values(&mut e, "impixel(R,[1 2 4],[1 2 3])"),
        [255., 255., 0., 255., 0., 255., 255., 0., 0.]
    );
    assert_eq!(
        values(&mut e, "impixel(label2rgb(L,C,'blue'),1,1)"),
        [0., 0., 255.]
    );
    assert_eq!(
        values(
            &mut e,
            "impixel(labeloverlay(A,L,'Colormap',C,'Transparency',.25),[1 2 4],[1 2 3])"
        ),
        [100., 216., 25., 100., 25., 216., 100., 25., 25.]
    );
    assert_eq!(
        values(
            &mut e,
            "impixel(labeloverlay(A,L,'Colormap',C,'IncludedLabels',2),[2 4],[2 3])"
        ),
        [100., 50., 100., 178., 100., 50.]
    );
    assert_eq!(
        values(&mut e, "impixel(imoverlay(A,L==1,'red'),[1 2],[1 2])"),
        [100., 255., 100., 0., 100., 0.]
    );
    for input in [
        "double(A)/255",
        "single(A)/255",
        "uint16(double(A)*257)",
        "A",
    ] {
        assert_eq!(
            values(
                &mut e,
                &format!("impixel(imoverlay({input},L==1,[0 0 1]),[1 2],[1 2])")
            ),
            [100., 0., 100., 0., 100., 255.]
        );
    }
    assert_eq!(
        values(&mut e, "impixel(imoverlay(false(5),L==1,'#f80'),2,2)"),
        [255., 136., 0.]
    );
    assert_eq!(values(&mut e, "impixel(label2rgb(L),1,1)"), [255.; 3]);
    assert_eq!(
        values(&mut e, "size(label2rgb(L,'spring','c'))"),
        [5., 5., 3.]
    );
    e.execute("rng(42); R1=label2rgb(L,C,'k','shuffle'); U1=rand; rng(42); R2=label2rgb(L,C,'k','shuffle'); U2=rand; rng(42); U0=rand;").unwrap();
    assert_eq!(values(&mut e, "isequal(R1,R2) && U1==U2 && U1~=U0"), [1.]);
}

#[test]
fn boundarymask_matches_skimage_connectivities_without_image_frame() {
    let mut e = run(&format!("L={LABELS};"));
    assert_eq!(values(&mut e, "boundarymask(L,4)"), BOUNDARY4);
    assert_eq!(values(&mut e, "boundarymask(L)"), BOUNDARY8);
    assert_eq!(values(&mut e, "nnz(boundarymask(ones(5)))"), [0.]);
    assert_eq!(
        values(
            &mut e,
            "isequal(boundarymask(logical(L),4),boundarymask(double(L~=0),4))"
        ),
        [1.]
    );
    assert_eq!(values(&mut e, "isa(boundarymask(L),'logical')"), [1.]);
}

#[test]
fn impixel_indexed_gray_rgb_outside_and_coordinate_outputs() {
    let mut e = run("A=reshape(uint16(1:25),5,5); [c,r,P]=impixel(A,[1 3 5],[1 3 5]);");
    assert_eq!(
        values(&mut e, "P"),
        [1., 13., 25., 1., 13., 25., 1., 13., 25.]
    );
    assert_eq!(values(&mut e, "c"), [1., 3., 5.]);
    assert_eq!(values(&mut e, "r"), [1., 3., 5.]);
    assert_eq!(values(&mut e, "isa(P,'double')"), [1.]);
    assert_eq!(values(&mut e, "all(isnan(impixel(A,0,2)))"), [1.]);
    assert_eq!(values(&mut e, "impixel(A,2.6,2.6)"), [13.; 3]);
    e.execute("M=[1 0 0;0 1 0;0 0 1]; X=repmat([1 2 3 2 1],5,1);")
        .unwrap();
    for input in ["X", "uint8(X-1)", "uint16(X-1)"] {
        assert_eq!(
            values(&mut e, &format!("impixel({input},M,[1 2 3],[3 3 3])")),
            [1., 0., 0., 0., 1., 0., 0., 0., 1.]
        );
    }
    assert_eq!(
        values(&mut e, "isa(impixel(X,single(M),1,1),'single')"),
        [1.]
    );
    assert_eq!(values(&mut e, "impixel(true(5),3,3)"), [1.; 3]);
}

#[test]
fn image_color_rejects_bad_dimensions_options_classes_and_work() {
    for (source, expected) in [
        ("rgb2lab(ones(5))", "P-by-3"),
        ("rgb2lab(true(5,5,3))", "class"),
        ("lab2rgb(uint8([1 2 3]))", "class"),
        ("rgb2lab([1 0 0],'WhitePoint',[0 1 1])", "WhitePoint"),
        ("rgb2lab([1 0 0],'WhitePoint','bad')", "illuminant"),
        ("lab2rgb([1 0 0],'OutputType','int8')", "OutputType"),
        ("rgb2lab([1 0 0],'ColorSpace','bad')", "ColorSpace"),
        ("rgb2ycbcr([1 0 0],'what',1)", "unknown option"),
        ("multithresh(ones(5),21)", "1 to 20"),
        ("multithresh(ones(5),1.5)", "integer"),
        ("imquantize(ones(5),[2 1])", "increasing"),
        ("imquantize(ones(5),[1 2],[1 2])", "length"),
        ("adaptthresh(ones(5),'NeighborhoodSize',4)", "positive odd"),
        ("adaptthresh(ones(5),2)", "sensitivity"),
        ("adaptthresh(ones(5),'Statistic','bad')", "Statistic"),
        (
            "adaptthresh(ones(5),'ForegroundPolarity','bad')",
            "ForegroundPolarity",
        ),
        (
            "adaptthresh(ones(5),'NeighborhoodSize',100001,'Statistic','median')",
            "limit",
        ),
        ("adaptthresh(ones(5),'unknown',2)", "unknown option"),
        ("label2rgb([-1 0])", "nonnegative"),
        ("label2rgb([1 2],[1 0 0])", "at least 2"),
        ("label2rgb([0 1],eye(3),'bad')", "color"),
        ("labeloverlay(ones(5),ones(4))", "match"),
        (
            "labeloverlay(ones(5),ones(5),'Transparency',2)",
            "Transparency",
        ),
        (
            "labeloverlay(ones(5),ones(5),'IncludedLabels',-1)",
            "IncludedLabels",
        ),
        ("boundarymask(ones(5),6)", "connectivity"),
        ("boundarymask(ones(5,5,3))", "2-D"),
        ("impixel(ones(5))", "interactive"),
        ("impixel(ones(5),[1 2],1)", "equal lengths"),
        ("[a,b]=rgb2lab([1 0 0])", "output"),
    ] {
        assert!(
            error(source).contains(expected),
            "{source}: {}",
            error(source)
        );
    }
}

#[test]
fn image_color_help_examples_execute() {
    for name in [
        "rgb2lab",
        "lab2rgb",
        "rgb2ycbcr",
        "ycbcr2rgb",
        "multithresh",
        "imquantize",
        "adaptthresh",
        "label2rgb",
        "labeloverlay",
        "imoverlay",
        "boundarymask",
        "impixel",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        let mut e = Engine::default();
        let body = e
            .execute(&format!("help {name}"))
            .unwrap()
            .output
            .join("\n");
        assert!(body.contains("Syntax:"), "{name}");
        for example in body.lines().filter_map(|l| l.strip_prefix("Example:")) {
            run(example);
        }
    }
}

#[test]
fn seeded_64_square_reference_and_speed_sanity() {
    if let Ok(n) = std::env::var("IMAGE_COLOR_BENCH_WORKERS") {
        matrust::initialize_worker_pool(n.parse().unwrap()).unwrap();
    }
    let mut state = 123456789u32;
    let data: Vec<f64> = (0..4096)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            f64::from(state >> 24)
        })
        .collect();
    let mut e = Engine::default();
    e.variables
        .insert("A".into(), Matrix::from_column_vec(64, 64, data).unwrap());
    e.execute("A=uint8(A);").unwrap();
    assert_eq!(values(&mut e, "multithresh(A,2)"), SEEDED_2);
    assert_eq!(values(&mut e, "multithresh(A,3)"), SEEDED_3);
    let start = Instant::now();
    for _ in 0..10 {
        e.execute("T=adaptthresh(A); TM=adaptthresh(A,'Statistic','median','NeighborhoodSize',5); TG=adaptthresh(A,'Statistic','gaussian','NeighborhoodSize',5); B=imbinarize(A,T); Q=imquantize(A,multithresh(A,3)); R=label2rgb(Q); C=rgb2lab(R); V=lab2rgb(C); Y=rgb2ycbcr(R); Z=ycbcr2rgb(Y); O=labeloverlay(A,Q); D=boundarymask(Q); P=impixel(O,[1 32 64],[1 32 64]); F=imoverlay(A,B,'yellow'); H=hsv2rgb(rgb2hsv(R));").unwrap();
    }
    eprintln!("image_color 64x64 pipeline x10: {:?}", start.elapsed());
    assert!(
        start.elapsed().as_secs() < 20,
        "64x64 app pipeline took over 20s"
    );
}
#[rustfmt::skip]
const LAB_D65: &[f64] = &[0.00000000000000000e+00, 1.00000003866809323e+02, 5.32397274817355992e+01, 8.77352760634715736e+01, 3.22966488206535374e+01, 3.15799014161283438e+01, 0.00000000000000000e+00, -1.66677771584744505e-05, 8.00983555540580880e+01, -8.61887599479344004e+01, 7.91925971285530323e+01, -1.86398224901793141e+00, 0.00000000000000000e+00, 6.66687851591518665e-06, 6.72011546091294889e+01, 8.31795191510165779e+01, -1.07860581532555372e+02, -1.78506699752111189e+01];
#[rustfmt::skip]
const LAB_D50: &[f64] = &[0.00000000000000000e+00, 1.00000003829910398e+02, 5.42917528848458346e+01, 8.78183603502032497e+01, 2.95663473175898304e+01, 3.13293615506522158e+01, 0.00000000000000000e+00, -1.61126552100654408e-05, 8.08148662585741988e+01, -7.92866938449555079e+01, 6.82956240284039495e+01, -4.07337875208618705e+00, 0.00000000000000000e+00, 6.48168199113285937e-06, 6.98854454918255641e+01, 8.09891478505598883e+01, -1.12031171142655566e+02, -1.81750158132013020e+01];
#[rustfmt::skip]
const LAB_ADOBE: &[f64] = &[0.00000000000000000e+00, 1.00000003866642899e+02, 6.14260390247731323e+01, 8.33035202604303180e+01, 3.29782492410571066e+01, 3.01784393622865679e+01, 0.00000000000000000e+00, 8.68556226940597753e-07, 8.95685296812385729e+01, -1.37983917279488338e+02, 8.03101997409069526e+01, -5.69048081196460487e+00, 0.00000000000000000e+00, 6.66660491255299803e-06, 7.51464166191568950e+01, 9.08302331595990324e+01, -1.09382768198968748e+02, -2.08220170084190599e+01];
#[rustfmt::skip]
const LAB_LINEAR: &[f64] = &[0.00000000000000000e+00, 1.00000003866809323e+02, 5.32397274817355992e+01, 8.77352760634715736e+01, 3.22966488206535374e+01, 6.04226148158485046e+01, 0.00000000000000000e+00, -1.66677771584744505e-05, 8.00983555540580880e+01, -8.61887599479344004e+01, 7.91925971285530323e+01, -4.02787814257726851e+00, 0.00000000000000000e+00, 6.66687851591518665e-06, 6.72011546091294889e+01, 8.31795191510165779e+01, -1.07860581532555372e+02, -1.37957555681575972e+01];
#[rustfmt::skip]
const YCBCR: &[f64] = &[6.27450980392156854e-02, 9.21568627450980338e-01, 3.19533333333333336e-01, 5.66874509803921511e-01, 1.60650980392156861e-01, 3.04503921568627434e-01, 5.01960784313725483e-01, 5.01960784313725483e-01, 3.53737254901960785e-01, 2.10968627450980384e-01, 9.41176470588235281e-01, 5.60704705882352994e-01, 5.01960784313725483e-01, 5.01960784313725483e-01, 9.41176470588235281e-01, 1.34172549019607845e-01, 4.30533333333333323e-01, 4.50896470588235287e-01];
#[rustfmt::skip]
const YCBCR_U8: &[f64] = &[1.60000000000000000e+01, 2.35000000000000000e+02, 8.10000000000000000e+01, 1.45000000000000000e+02, 4.10000000000000000e+01, 7.80000000000000000e+01, 1.28000000000000000e+02, 1.28000000000000000e+02, 9.00000000000000000e+01, 5.40000000000000000e+01, 2.40000000000000000e+02, 1.43000000000000000e+02, 1.28000000000000000e+02, 1.28000000000000000e+02, 2.40000000000000000e+02, 3.40000000000000000e+01, 1.10000000000000000e+02, 1.15000000000000000e+02];
#[rustfmt::skip]
const YCBCR_U16: &[f64] = &[4.11200000000000000e+03, 6.03950000000000000e+04, 2.09410000000000000e+04, 3.71500000000000000e+04, 1.05280000000000000e+04, 1.99560000000000000e+04, 3.28960000000000000e+04, 3.28960000000000000e+04, 2.31820000000000000e+04, 1.38260000000000000e+04, 6.16800000000000000e+04, 3.67460000000000000e+04, 3.28960000000000000e+04, 3.28960000000000000e+04, 6.16800000000000000e+04, 8.79300000000000000e+03, 2.82150000000000000e+04, 2.95490000000000000e+04];
#[rustfmt::skip]
const MULTI_1: &[f64] = &[1.24000000000000000e+02];
#[rustfmt::skip]
const MULTI_PAIR: &[f64] = &[1.00195312500000000e+01];
#[rustfmt::skip]
const MULTI_2: &[f64] = &[4.90000000000000000e+01, 1.24000000000000000e+02];
#[rustfmt::skip]
const MULTI_3: &[f64] = &[4.90000000000000000e+01, 1.24000000000000000e+02, 1.94000000000000000e+02];
#[rustfmt::skip]
const ADAPT_MEAN: &[f64] = &[1.38888888888888895e-01, 1.66666666666666685e-01, 2.08333333333333343e-01, 2.50000000000000000e-01, 2.77777777777777790e-01, 2.63888888888888895e-01, 2.91666666666666685e-01, 3.33333333333333370e-01, 3.75000000000000000e-01, 4.02777777777777790e-01, 4.30555555555555525e-01, 4.58333333333333370e-01, 5.00000000000000000e-01, 5.41666666666666741e-01, 5.69444444444444420e-01, 5.97222222222222210e-01, 6.25000000000000111e-01, 6.66666666666666630e-01, 7.08333333333333370e-01, 7.36111111111111160e-01, 7.22222222222222210e-01, 7.50000000000000111e-01, 7.91666666666666630e-01, 8.33333333333333370e-01, 8.61111111111111049e-01];
#[rustfmt::skip]
const ADAPT_MEDIAN: &[f64] = &[4.16666666666666644e-02, 8.33333333333333287e-02, 1.25000000000000000e-01, 1.66666666666666657e-01, 1.66666666666666657e-01, 2.08333333333333343e-01, 2.50000000000000000e-01, 2.91666666666666685e-01, 3.33333333333333315e-01, 3.75000000000000000e-01, 4.16666666666666685e-01, 4.58333333333333315e-01, 5.00000000000000000e-01, 5.41666666666666630e-01, 5.83333333333333370e-01, 6.25000000000000000e-01, 6.66666666666666630e-01, 7.08333333333333370e-01, 7.50000000000000000e-01, 7.91666666666666630e-01, 8.33333333333333370e-01, 8.33333333333333370e-01, 8.75000000000000000e-01, 9.16666666666666630e-01, 9.58333333333333370e-01];
#[rustfmt::skip]
const ADAPT_GAUSSIAN: &[f64] = &[4.36856518175041189e-02, 8.48995042518884829e-02, 1.26566170918555154e-01, 1.68232837585221839e-01, 2.09446690019606196e-01, 2.10170737773933108e-01, 2.51384590208317438e-01, 2.93051256874984123e-01, 3.34717923541650864e-01, 3.75931775976035165e-01, 4.17119480898949124e-01, 4.58333333333333370e-01, 5.00000000000000111e-01, 5.41666666666666852e-01, 5.82880519101051209e-01, 6.24068224023965112e-01, 6.65282076458349358e-01, 7.06948743125016099e-01, 7.48615409791682840e-01, 7.89829262226067197e-01, 7.90553309980394081e-01, 8.31767162414778327e-01, 8.73433829081444957e-01, 9.15100495748111697e-01, 9.56314348182496055e-01];
#[rustfmt::skip]
const BOUNDARY4: &[f64] = &[0.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 0.00000000000000000e+00, 0.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 0.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 0.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 0.00000000000000000e+00, 0.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 0.00000000000000000e+00];
#[rustfmt::skip]
const BOUNDARY8: &[f64] = &[1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 0.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 0.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00, 1.00000000000000000e+00];
#[rustfmt::skip]
const SEEDED_2: &[f64] = &[8.30000000000000000e+01, 1.68000000000000000e+02];
#[rustfmt::skip]
const SEEDED_3: &[f64] = &[6.10000000000000000e+01, 1.25000000000000000e+02, 1.90000000000000000e+02];

#[test]
fn adaptive_median_cancels_without_publishing_partial_output() {
    use std::sync::atomic::Ordering;
    let mut e = run("A=ones(128); T=17;");
    let cancel = e.cancellation();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        cancel.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    let result = e.execute("T=adaptthresh(A,'Statistic','median','NeighborhoodSize',63);");
    timer.join().unwrap();
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    e.cancellation().store(false, Ordering::Relaxed);
    assert_eq!(values(&mut e, "T"), [17.]);
}
#[rustfmt::skip]
const LAB_WHITES: &[f64] = &[3.07234479061851715e+01, 3.16205644386915594e+01, 3.14347570708246309e+01, 3.13291721726295478e+01, 3.17446989597432392e+01, -7.28279686410751559e+00, -1.20727534579606610e+00, -2.66450722872182943e+00, -4.07514301837508253e+00, -7.26493999508526223e-01, -1.88081454222050581e+01, -1.78241501455250315e+01, -1.80967730184928968e+01, -1.81752235227884320e+01, -1.75764464555334570e+01];
#[rustfmt::skip]
const RGB_FROM_YCC_U8: &[f64] = &[0.00000000000000000e+00, 2.55000000000000000e+02, 2.54000000000000000e+02, 0.00000000000000000e+00, 0.00000000000000000e+00, 5.10000000000000000e+01, 0.00000000000000000e+00, 2.55000000000000000e+02, 0.00000000000000000e+00, 2.55000000000000000e+02, 0.00000000000000000e+00, 7.70000000000000000e+01, 0.00000000000000000e+00, 2.55000000000000000e+02, 0.00000000000000000e+00, 1.00000000000000000e+00, 2.55000000000000000e+02, 1.02000000000000000e+02];
#[rustfmt::skip]
const RGB_FROM_YCC_U16: &[f64] = &[0.00000000000000000e+00, 6.55350000000000000e+04, 6.55350000000000000e+04, 0.00000000000000000e+00, 0.00000000000000000e+00, 1.31070000000000000e+04, 0.00000000000000000e+00, 6.55350000000000000e+04, 1.00000000000000000e+00, 6.55350000000000000e+04, 0.00000000000000000e+00, 1.96610000000000000e+04, 0.00000000000000000e+00, 6.55350000000000000e+04, 0.00000000000000000e+00, 0.00000000000000000e+00, 6.55350000000000000e+04, 2.62150000000000000e+04];
#[rustfmt::skip]
const MULTI_METRICS: &[f64] = &[7.80927467639847217e-01, 9.31501967269202313e-01, 9.75706126095303383e-01];

#[test]
fn additional_whitepoints_inverse_integer_ycc_and_otsu_metrics() {
    let mut e = run(&format!("C={RGB}; A={FIXED};"));
    let mut actual = vec![0.; 15];
    for (i, white) in ["'a'", "'c'", "'e'", "'icc'", "[.9 1 1.2]"]
        .iter()
        .enumerate()
    {
        let v = values(&mut e, &format!("rgb2lab([.2 .3 .4],'WhitePoint',{white})"));
        for k in 0..3 {
            actual[k * 5 + i] = v[k];
        }
    }
    close(&actual, LAB_WHITES, 1e-10);
    assert_eq!(
        values(&mut e, "ycbcr2rgb(rgb2ycbcr(uint8(C*255)))"),
        RGB_FROM_YCC_U8
    );
    assert_eq!(
        values(&mut e, "ycbcr2rgb(rgb2ycbcr(uint16(C*65535)))"),
        RGB_FROM_YCC_U16
    );
    for (i, n) in [1, 2, 3].iter().enumerate() {
        e.execute(&format!("[t,m]=multithresh(uint8(A),{n});"))
            .unwrap();
        close(&values(&mut e, "m"), &[MULTI_METRICS[i]], 1e-12);
    }
}
