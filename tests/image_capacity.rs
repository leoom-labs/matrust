//! A 4000x3000 RGB photo (36,000,000 elements) through the image paths of the
//! apps, under the desktop/CLI limits (`Limits::local()`). Images are built
//! arithmetically; expected values are closed forms computed here, or the same
//! function on a small crop of the image (exact for pixel-wise functions).
use matrust::{Engine, engine::Limits};

const H: usize = 3000;
const W: usize = 4000;
/// R(i,j) = mod(i+j, 256), G(i,j) = mod(i, 256), B(i,j) = mod(j, 256) with
/// zero-based i (row) and j (column).
const BUILD: &str = "[X, Y] = meshgrid(0:3999, 0:2999); R = uint8(mod(X + Y, 256)); \
    G = uint8(mod(Y, 256)); B = uint8(mod(X, 256)); clear X Y; I = cat(3, R, G, B);";

fn local() -> Engine {
    Engine::with_limits(Limits::local())
}
fn ok(e: &mut Engine, source: &str) {
    if let Err(err) = e.execute(source) {
        panic!("{source}\n{err}");
    }
}
fn num(e: &mut Engine, expr: &str) -> Vec<f64> {
    ok(e, &format!("zz__ = double({expr});"));
    let v = e.variables["zz__"].data().to_vec();
    ok(e, "clear zz__");
    v
}
fn one(e: &mut Engine, expr: &str) -> f64 {
    let v = num(e, expr);
    assert_eq!(v.len(), 1, "{expr}");
    v[0]
}
fn pixel(i: usize, j: usize) -> [f64; 3] {
    [((i + j) % 256) as f64, (i % 256) as f64, (j % 256) as f64]
}
/// Sum over the whole image of each channel.
fn channel_sums() -> [f64; 3] {
    let mut s = [0.; 3];
    for i in 0..H {
        for j in 0..W {
            let p = pixel(i, j);
            for c in 0..3 {
                s[c] += p[c];
            }
        }
    }
    s
}
fn rgb() -> Engine {
    let mut e = local();
    ok(&mut e, BUILD);
    ok(&mut e, "clear R G B");
    e
}

#[test]
fn rgb_build_double_and_reductions() {
    let mut e = rgb();
    assert_eq!(num(&mut e, "size(I)"), [H as f64, W as f64, 3.]);
    assert_eq!(num(&mut e, "squeeze(I(2999, 3999, :))'"), pixel(2998, 3998));
    let sums = channel_sums();
    ok(&mut e, "D = double(I);");
    assert_eq!(one(&mut e, "isa(D, 'double')"), 1.);
    assert_eq!(num(&mut e, "squeeze(sum(D, [1 2]))'"), sums);
    assert_eq!(num(&mut e, "squeeze(sum(I, [1 2], 'double'))'"), sums);
    // sum/min/max over the channel dimension and over rows.
    assert_eq!(num(&mut e, "size(sum(I, 3))"), [H as f64, W as f64]);
    assert_eq!(one(&mut e, "sum(sum(sum(D, 3)))"), sums.iter().sum::<f64>());
    let (mut mx, mut mn) = (0., 0.);
    for i in 0..H {
        for j in 0..W {
            let p = pixel(i, j);
            mx += p.iter().cloned().fold(0., f64::max);
            mn += p.iter().cloned().fold(255., f64::min);
        }
    }
    assert_eq!(one(&mut e, "sum(sum(double(max(I, [], 3))))"), mx);
    assert_eq!(one(&mut e, "sum(sum(double(min(I, [], 3))))"), mn);
    assert_eq!(one(&mut e, "sum(sum(min(D, [], 3)))"), mn);
    assert_eq!(num(&mut e, "size(max(D, [], 1))"), [1., W as f64, 3.]);
    assert_eq!(
        num(&mut e, "squeeze(max(I, [], [1 2]))'"),
        [255., 255., 255.]
    );
    // cat(3, ...) of double planes and workspace store of the results.
    ok(
        &mut e,
        "E = cat(3, D(:,:,3), D(:,:,2), D(:,:,1)); K = E; S = I;",
    );
    assert_eq!(num(&mut e, "squeeze(E(10, 20, :))'"), {
        let p = pixel(9, 19);
        [p[2], p[1], p[0]]
    });
    assert_eq!(one(&mut e, "isequal(K, E) && isequal(S, I)"), 1.);
    // isreal on N-D input.
    assert_eq!(one(&mut e, "isreal(D)"), 1.);
    assert_eq!(one(&mut e, "isreal(I)"), 1.);
}

#[test]
fn rgb_class_conversions_and_color_spaces() {
    let mut e = rgb();
    ok(
        &mut e,
        "D = im2double(I); C = I(1:4, 1:5, :); Cd = im2double(C);",
    );
    assert_eq!(
        num(&mut e, "squeeze(D(3, 4, :))'"),
        pixel(2, 3).map(|v| v / 255.)
    );
    assert_eq!(one(&mut e, "isequal(im2uint8(D), I)"), 1.);
    // Pixel-wise functions: the corner of the big result equals the crop's result.
    for f in ["rgb2gray(I)", "rgb2gray(D)", "rgb2ycbcr(I)", "rgb2ycbcr(D)"] {
        ok(&mut e, &format!("Y = {f};"));
        let small = f.replace("(I)", "(C)").replace("(D)", "(Cd)");
        assert_eq!(
            one(&mut e, &format!("isequal(Y(1:4, 1:5, :), {small})")),
            1.,
            "{f}"
        );
        assert_eq!(
            num(&mut e, "[size(Y, 1) size(Y, 2)]"),
            [H as f64, W as f64],
            "{f}"
        );
        ok(&mut e, "clear Y");
    }
    ok(&mut e, "L = rgb2lab(D);");
    assert_eq!(num(&mut e, "size(L)"), [H as f64, W as f64, 3.]);
    assert_eq!(one(&mut e, "isequal(L(1:4, 1:5, :), rgb2lab(Cd))"), 1.);
    assert!(one(&mut e, "max(abs(lab2rgb(L) - D), [], 'all')") < 1e-9);
    ok(&mut e, "clear L; V = rgb2hsv(D);");
    assert_eq!(one(&mut e, "isequal(V(1:4, 1:5, :), rgb2hsv(Cd))"), 1.);
    assert!(one(&mut e, "max(abs(hsv2rgb(V) - D), [], 'all')") < 1e-12);
    ok(&mut e, "clear V; V8 = rgb2hsv(I);");
    assert_eq!(num(&mut e, "size(V8)"), [H as f64, W as f64, 3.]);
}

#[test]
fn rgb_compare_logical_indexing_and_masked_assignment() {
    let mut e = rgb();
    ok(
        &mut e,
        "m = I(:,:,1) > 100 & I(:,:,2) < 200 | I(:,:,3) == 7; D = im2double(I);",
    );
    let mut count = 0usize;
    let mut red = 0.;
    for i in 0..H {
        for j in 0..W {
            let p = pixel(i, j);
            if (p[0] > 100. && p[1] < 200.) || p[2] == 7. {
                count += 1;
                red += p[0];
            }
        }
    }
    assert_eq!(one(&mut e, "nnz(m)"), count as f64);
    let r = channel_sums()[0];
    // Logical indexing of one channel and of the whole image.
    ok(
        &mut e,
        "R = I(:,:,1); v = R(m); M = cat(3, m, m, m); w = I(M);",
    );
    assert_eq!(one(&mut e, "numel(v)"), count as f64);
    assert_eq!(one(&mut e, "sum(double(v))"), red);
    assert_eq!(one(&mut e, "numel(w)"), 3. * count as f64);
    // Masked assignment on uint8 and double images.
    ok(&mut e, "J = I; J(M) = 0; Dd = D; Dd(M) = 1;");
    assert_eq!(one(&mut e, "sum(sum(double(J(:,:,1))))"), r - red);
    assert_eq!(one(&mut e, "nnz(Dd(:) == 1) >= 3 * nnz(m)"), 1.);
    assert_eq!(one(&mut e, "nnz(D(:,:,1) > 0.5 | D(:,:,3) <= 0.25)"), {
        let mut n = 0;
        for i in 0..H {
            for j in 0..W {
                let p = pixel(i, j);
                n += usize::from(p[0] / 255. > 0.5 || p[2] / 255. <= 0.25);
            }
        }
        n as f64
    });
}

#[test]
fn rgb_geometry_and_contrast() {
    let mut e = rgb();
    ok(&mut e, "D = im2double(I);");
    for img in ["I", "D"] {
        ok(
            &mut e,
            &format!("Z = imresize({img}, 0.5); X = imcrop({img}, [11 21 99 49]);"),
        );
        assert_eq!(num(&mut e, "size(Z)"), [1500., 2000., 3.], "{img}");
        assert_eq!(one(&mut e, &format!("isa(Z, class({img}))")), 1., "{img}");
        assert_eq!(num(&mut e, "size(X)"), [50., 100., 3.], "{img}");
        assert_eq!(
            one(&mut e, &format!("isequal(X, {img}(21:70, 11:110, :))")),
            1.,
            "{img}"
        );
        // Every channel spans 0..255, so stretchlim saturates 1 % at each end.
        ok(
            &mut e,
            &format!("lim = stretchlim({img}); A = imadjust({img}, lim);"),
        );
        assert_eq!(num(&mut e, "size(lim)"), [2., 3.], "{img}");
        assert_eq!(num(&mut e, "size(A)"), [H as f64, W as f64, 3.], "{img}");
        assert_eq!(one(&mut e, &format!("isa(A, class({img}))")), 1., "{img}");
        ok(&mut e, "clear Z X A lim");
    }
    ok(
        &mut e,
        "Gr = I(:,:,2); h = imhist(Gr); hd = imhist(D(:,:,2));",
    );
    // G(i,j) = mod(i, 256): 3000 rows = 11 full cycles + 184 rows.
    let expected: Vec<f64> = (0..256)
        .map(|v| (W * (11 + usize::from(v < 184))) as f64)
        .collect();
    assert_eq!(num(&mut e, "h'"), expected);
    assert_eq!(num(&mut e, "hd'"), expected);
    ok(
        &mut e,
        "bw = imbinarize(Gr, 0.5); bwd = imbinarize(D(:,:,2), 0.5);",
    );
    let above = (0..H).filter(|i| i % 256 >= 128).count() * W;
    assert_eq!(one(&mut e, "nnz(bw)"), above as f64);
    assert_eq!(one(&mut e, "nnz(bwd)"), above as f64);
    assert_eq!(one(&mut e, "islogical(imbinarize(Gr))"), 1.);
}

#[test]
fn png_round_trip_of_a_12_mp_photo() {
    let mut e = rgb();
    let dir = std::env::temp_dir().join(format!("matrust-capacity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("photo.png");
    let path = file.to_str().unwrap().replace('\\', "/");
    ok(
        &mut e,
        &format!("imwrite(I, '{path}'); J = imread('{path}');"),
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        one(&mut e, "strcmp(class(J), 'uint8') && isequal(J, I)"),
        1.
    );
}

/// 30-by-40 grid of 50x50 squares on a 3000x4000 binary image.
const SQUARES: &str =
    "[X, Y] = meshgrid(0:3999, 0:2999); BW = mod(Y, 100) < 50 & mod(X, 100) < 50; clear X Y";

#[test]
fn gray_labels_regions_and_label2rgb() {
    let mut e = local();
    ok(&mut e, SQUARES);
    ok(&mut e, "Lb = bwlabel(BW);");
    assert_eq!(one(&mut e, "max(Lb(:))"), 1200.);
    // Column-major labelling: the second label is the square below the first.
    assert_eq!(
        num(&mut e, "[Lb(1,1) Lb(101,1) Lb(1,101) Lb(75,75)]"),
        [1., 2., 31., 0.]
    );
    ok(
        &mut e,
        "s = regionprops(BW, 'Area', 'Centroid', 'BoundingBox');",
    );
    assert_eq!(one(&mut e, "numel(s)"), 1200.);
    assert_eq!(one(&mut e, "all([s.Area] == 2500)"), 1.);
    assert_eq!(num(&mut e, "s(1).Centroid"), [25.5, 25.5]);
    assert_eq!(num(&mut e, "s(2).Centroid"), [25.5, 125.5]);
    assert_eq!(
        num(&mut e, "s(1200).BoundingBox"),
        [3900.5, 2900.5, 50., 50.]
    );
    assert_eq!(
        one(&mut e, "nnz(bwareafilt(BW, [2000 3000]))"),
        1200. * 2500.
    );
    assert_eq!(one(&mut e, "nnz(bwareafilt(BW, [1 100]))"), 0.);
    ok(&mut e, "RGB = label2rgb(Lb);");
    assert_eq!(num(&mut e, "size(RGB)"), [H as f64, W as f64, 3.]);
    assert_eq!(one(&mut e, "isa(RGB, 'uint8')"), 1.);
    // Background is white; label 1 has the first jet(1200) colour.
    assert_eq!(num(&mut e, "squeeze(RGB(75, 75, :))'"), [255., 255., 255.]);
    assert_eq!(
        one(
            &mut e,
            "isequal(squeeze(RGB(1, 1, :))', squeeze(label2rgb(1, jet(1200)))')"
        ),
        1.
    );
}

#[test]
fn gray_watershed_of_distance_transform() {
    let mut e = local();
    ok(&mut e, SQUARES);
    ok(&mut e, "Dt = bwdist(BW);");
    assert_eq!(num(&mut e, "size(Dt)"), [H as f64, W as f64]);
    // (75,25) is 25 below the square ending at row 50; (75,75) is diagonal to its corner.
    assert_eq!(num(&mut e, "[Dt(1,1) Dt(75,25)]"), [0., 25.]);
    assert!((one(&mut e, "Dt(75,75)") - 2f64.sqrt() * 25.).abs() < 1e-4);
    ok(&mut e, "Lw = watershed(Dt);");
    assert_eq!(num(&mut e, "size(Lw)"), [H as f64, W as f64]);
    // Every square is one catchment basin.
    assert_eq!(one(&mut e, "max(Lw(:))"), 1200.);
    assert_eq!(one(&mut e, "numel(unique(Lw(BW)))"), 1200.);
}

#[test]
fn limits_still_bound_size_and_default_budget() {
    let mut e = local();
    let err = e.execute("x = zeros(7168, 7168);").unwrap_err();
    assert!(
        err.contains("Matrix exceeds the 50331648-element limit"),
        "{err}"
    );
    let err = e.execute("x = zeros(7168, 7168, 'uint8');").unwrap_err();
    assert!(err.contains("50331648-element limit"), "{err}");
    // The default (browser) limits keep the 256 MiB statement budget.
    let mut d = Engine::default();
    let err = d
        .execute("x = sum(ones(1, 300 * 1024 * 1024 / 8));")
        .unwrap_err();
    assert!(err.contains("256 MiB"), "{err}");
    // A 12 MP uint8 photo built under the local limits: under the default
    // limits its double copy (288 MB) is refused before allocation.
    let mut d = local();
    d.execute("I = zeros(3000, 4000, 3, 'uint8');").unwrap();
    d.limits_set(Limits::default());
    let err = d.execute("D = double(I);").unwrap_err();
    assert!(err.contains("256 MiB"), "{err}");
    // isreal on N-D values.
    d.execute(
        "a = isreal(ones(2,2,2)); b = isreal(complex(ones(2,2,2))); \
         c = isreal(ones(2,2,2,'int8')); f = isa(ones(2,2,2,'int8'), 'integer'); \
         g = isa(ones(2,2,2), 'uint8');",
    )
    .unwrap();
    for (name, want) in [("a", 1.), ("b", 0.), ("c", 1.), ("f", 1.), ("g", 0.)] {
        assert_eq!(one(&mut d, name), want, "{name}");
    }
}
