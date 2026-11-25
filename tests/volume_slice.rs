//! SciPy and affine-field checks of slice sampling and graphics data.
use matrust::{
    Engine, files,
    plot::{PlotKind, view3d},
};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/volume/slice_scipy.json")).unwrap()
}

fn setup() -> String {
    let f = fixture();
    let values = f["v"].as_array().unwrap();
    let mut script = String::from("v=[];");
    for chunk in values.chunks(50) {
        let data = chunk
            .iter()
            .map(|v| v.as_f64().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        script.push_str(&format!("v=[v {data}];"));
    }
    script.push_str("V=reshape(v,7,6,5);");
    script
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-12, "{actual} != {expected}");
}

fn assert_fixture(e: &Engine, name: &str, f: &Value, key: &str) {
    let actual = e.variables[name].data();
    let expected = f[key].as_array().unwrap();
    assert_eq!(actual.len(), expected.len(), "{key}");
    for (&a, b) in actual.iter().zip(expected) {
        assert_close(a, b.as_f64().unwrap());
    }
}

#[test]
fn volume_slice_scipy_and_surface_properties() {
    let mut e = Engine::default();
    e.execute(&format!("{} h=slice(V,2.4,3.2,2.3); cx=get(h(1),'CData'); cy=get(h(2),'CData'); c=get(h(3),'CData'); x=get(h(3),'XData'); y=get(h(3),'YData'); z=get(h(3),'ZData');", setup())).unwrap();
    assert!(e.figure.as_ref().unwrap().svg().contains("<svg"));
    let f = fixture();
    assert_fixture(&e, "cx", &f, "random_x");
    assert_fixture(&e, "cy", &f, "random_y");
    assert_fixture(&e, "c", &f, "linear");
    assert_eq!((e.variables["x"].rows(), e.variables["x"].cols()), (7, 6));
    assert!(e.variables["z"].data().iter().all(|&v| v == 2.3));
    assert_eq!(e.variables["x"].data()[7], 2.);
    assert_eq!(e.variables["y"].data()[1], 2.);
    e.execute("h=slice(V,[],[],2.3,'nearest'); c=get(h,'CData');")
        .unwrap();
    assert_fixture(&e, "c", &f, "nearest");
}

#[test]
fn volume_slice_planes_order_and_errors() {
    let mut e = Engine::default();
    e.execute(&format!("{} h=slice(V,[2 3],4,2); dims=size(h); a=get(h(1),'XData'); b=get(h(2),'XData'); c=get(h(3),'YData'); d=get(h(4),'ZData');",setup())).unwrap();
    assert_eq!(e.variables["dims"].data(), [4., 1.]);
    for (name, expected) in [("a", 2.), ("b", 3.), ("c", 4.), ("d", 2.)] {
        assert!(e.variables[name].data().iter().all(|&v| v == expected));
    }
    for (source, message) in [
        ("slice(V,1,2);", "expected V and three slice positions"),
        ("slice(V,1,2,3,'bad');", "method must be"),
        (
            "slice(V,ones(2),ones(3),ones(2));",
            "matrices of equal size",
        ),
        ("slice(ones(3),1,2,3);", "V must be a 3-D array"),
        (
            "slice(V,[NaN Inf],[],[]);",
            "slice positions must be finite",
        ),
        (
            "slice(V,ones(2),ones(2),Inf*ones(2));",
            "slice positions must be finite",
        ),
        ("slice(1:5,1:7,1:5,V,[],[],2);", "grid must match V"),
        ("slice(V,ones(1,65),[],[]);", "64-series limit"),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(message), "{source} -> {error}");
    }
    e.execute("empty=slice(V,[],[],[]); empty_size=size(empty);")
        .unwrap();
    assert_eq!(e.variables["empty_size"].data(), [0., 1.]);

    e.execute(
        "figure; subplot(1,2,1); ax=gca; subplot(1,2,2); other=gca; plot(other,1:3);
         h=slice(ax,V,[],[],2); in_ax=double(isequal(get(h,'Parent'),ax));
         shading(ax,'interp');",
    )
    .unwrap();
    assert_eq!(e.variables["in_ax"].data(), [1.]);
    assert_eq!(e.figure.as_ref().unwrap().series_count(), 2);
    let error = e.execute("slice(ax,V,ones(1,64),[],[]);").unwrap_err();
    assert!(
        error.contains("only 63 fit") && error.contains("64-series limit"),
        "{error}"
    );
    assert_eq!(e.figure.as_ref().unwrap().series_count(), 2);
}

#[test]
fn volume_slice_tilted_surface_and_explicit_flow() {
    let mut e = Engine::default();
    e.execute("[X,Y,Z]=meshgrid(1:4,1:4,1:4); V=X+2*Y+3*Z; [A,B]=meshgrid(1:3,1:3); C=1+(A+B)/4; h=slice(X,Y,Z,V,A,B,C,'cubic'); q=get(h,'CData'); expected=A+2*B+3*C;").unwrap();
    let q = e.variables["q"].data();
    let expected = e.variables["expected"].data();
    assert_eq!(q.len(), expected.len());
    for (&a, &b) in q.iter().zip(expected) {
        assert_close(a, b);
    }

    e.execute(
        "[X,Y,Z,V]=flow(8); h=slice(X,Y,Z,V,4.8,0.35,0);
         cx=get(h(1),'CData'); cy=get(h(2),'CData'); cz=get(h(3),'CData');
         x=get(h(1),'XData'); y=get(h(2),'YData'); z=get(h(3),'ZData');",
    )
    .unwrap();
    let f = fixture();
    for (data, key, shape) in [
        ("cx", "flow_x", (8, 8)),
        ("cy", "flow_y", (16, 8)),
        ("cz", "flow_z", (8, 16)),
    ] {
        let m = &e.variables[data];
        assert_eq!((m.rows(), m.cols()), shape);
        assert_fixture(&e, data, &f, key);
    }
    for (data, position) in [("x", 4.8), ("y", 0.35), ("z", 0.)] {
        assert!(e.variables[data].data().iter().all(|&v| v == position));
    }
}

fn affine_volume() -> Engine {
    let mut e = Engine::default();
    e.execute("[X,Y,Z]=meshgrid(1:5,1:4,1:4); V=X+2*Y+3*Z;")
        .unwrap();
    e
}

fn check_contour(
    e: &mut Engine,
    index: usize,
    expected_level: f64,
    on_slice: impl Fn(f64, f64, f64) -> bool,
) -> Vec<[f64; 2]> {
    e.execute(&format!(
        "p=h({index}); px=get(p,'XData'); py=get(p,'YData'); pz=get(p,'ZData');
         lvl=get(p,'CData'); type_ok=strcmp(p.Type,'patch');
         face_ok=strcmp(get(p,'FaceColor'),'none'); edge_ok=strcmp(p.EdgeColor,'flat');
         properties_ok=double(type_ok && face_ok && edge_ok);",
    ))
    .unwrap();
    assert_eq!(e.variables["properties_ok"].data(), [1.]);
    let [level] = *e.variables["lvl"].data() else {
        panic!("CData must be one contour level");
    };
    assert_close(level, expected_level);
    let [x, y, z] = ["px", "py", "pz"].map(|name| e.variables[name].data());
    assert!(x.len() >= 2);
    assert_eq!(x.len(), y.len());
    assert_eq!(x.len(), z.len());
    let mut xy = Vec::with_capacity(x.len());
    for ((&x, &y), &z) in x.iter().zip(y).zip(z) {
        assert_close(x + 2. * y + 3. * z, level);
        assert!(on_slice(x, y, z), "off slice: [{x}, {y}, {z}]");
        xy.push([x, y]);
    }
    xy
}

#[test]
fn volume_slice_contours_match_contourc_and_render() {
    let mut e = affine_volume();
    e.execute(
        "levels=[12.5 14.5 16.5]; h=contourslice(V,[],[],2,levels);
         dims=size(h); cc=contourc(V(:,:,2),levels);
         colormap([1 0 0;0 1 0;0 0 1]); colorbar;",
    )
    .unwrap();
    assert_eq!(e.variables["dims"].data(), [3., 1.]);
    let c = e.variables["cc"].clone();
    let mut start = 0;
    for index in 1..=3 {
        let level = c.get(0, start);
        let count = c.get(1, start) as usize;
        let xy = check_contour(&mut e, index, level, |_, _, z| z == 2.);
        let end = start + count + 1;
        let expected = (start + 1..end)
            .map(|i| [c.get(0, i), c.get(1, i)])
            .collect::<Vec<_>>();
        assert_eq!(xy, expected);
        start = end;
    }
    assert_eq!(start, c.cols());

    let figure = e.figure.as_ref().unwrap();
    assert!(figure.series.iter().all(|s| s.kind == PlotKind::Patch));
    let colors = |f: &matrust::plot::Figure| {
        view3d::display_view(&f.axes).series[..3]
            .iter()
            .map(|s| s.color)
            .collect::<Vec<_>>()
    };
    assert_eq!(colors(figure), [[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
    assert!(figure.svg().contains("<svg"));

    // A folder, not an open NamedTempFile: on Windows save_figure cannot
    // replace a file that is still held open (Access is denied).
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("figure.mrfig");
    files::save_figure(&file, figure).unwrap();
    let restored = files::load_figure(&file).unwrap();
    assert_eq!(restored.series.len(), figure.series.len());
    for (saved, original) in restored.series.iter().zip(&figure.series) {
        assert_eq!(saved.kind, PlotKind::Patch);
        assert_eq!(saved.slice_contour_level, original.slice_contour_level);
        assert!(saved.points.iter().eq(original.points.iter()));
        assert_eq!(saved.z, original.z);
    }
    e.execute("view(2); colormap([0 0 1;0 1 0;1 0 0]);")
        .unwrap();
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(colors(figure), [[0, 0, 255], [0, 255, 0], [255, 0, 0]]);
    assert!(figure.svg().contains("<svg"));
}

#[test]
fn volume_slice_contours_count_surface_and_axes() {
    let mut e = affine_volume();
    e.execute(
        "subplot(1,2,1); ax=gca; subplot(1,2,2); other=gca; plot(other,1:3);
         h=contourslice(ax,X,Y,Z,V,2.25,2.5,2.75,2,'linear');
         dims=size(h); here=double(isequal(get(h(1),'Parent'),ax));",
    )
    .unwrap();
    assert_eq!(e.variables["dims"].data(), [6., 1.]);
    assert_eq!(e.variables["here"].data(), [1.]);
    let ranges = [[7.25, 22.25], [9., 22.], [11.25, 21.25]];
    let positions = [2.25, 2.5, 2.75];
    for (axis, [lo, hi]) in ranges.into_iter().enumerate() {
        for n in 1..=2 {
            let level = lo + (hi - lo) * n as f64 / 3.;
            check_contour(&mut e, axis * 2 + n, level, |x, y, z| {
                ([x, y, z][axis] - positions[axis]).abs() <= 1e-12
            });
        }
    }
    assert_eq!(e.figure.as_ref().unwrap().series_count(), 7);

    e.execute(
        "[A,B]=meshgrid(1:3,1:3); C=1+(A+B)/4;
         h=contourslice(X,Y,Z,V,A,B,C,[10 12],'cubic'); dims=size(h);",
    )
    .unwrap();
    assert_eq!(e.variables["dims"].data(), [2., 1.]);
    for (i, level) in [10., 12.].into_iter().enumerate() {
        check_contour(&mut e, i + 1, level, |x, y, z| {
            (1. + (x + y) / 4. - z).abs() <= 1e-12
        });
    }
    assert_eq!(e.figure.as_ref().unwrap().series_count(), 9);
}

#[test]
fn volume_slice_contours_default_levels_and_method() {
    let mut e = affine_volume();
    e.execute("h=contourslice(V,[],[],2); dims=size(h);")
        .unwrap();
    assert_eq!(e.variables["dims"].data(), [10., 1.]);
    for i in 1..=10 {
        check_contour(&mut e, i, 9. + 10. * i as f64 / 11., |_, _, z| z == 2.);
    }

    e.execute("h=contourslice(V,[],[],2.25,[13 13]); dims=size(h); z=get(h,'ZData');")
        .unwrap();
    assert_eq!(e.variables["dims"].data(), [1., 1.]);
    assert!(e.variables["z"].data().iter().all(|&z| z == 2.25));
    let line = e.figure.as_ref().unwrap().series.last().unwrap();
    for [x, y] in line.points.iter() {
        assert_close(x + 2. * y + 3. * 2., 13.);
    }
}

#[test]
fn volume_slice_contours_many_planes_share_level_series() {
    let mut e = Engine::default();
    e.execute("[X,Y,Z]=meshgrid(1:12,1:12,1:12); V=X+Y+Z; levels=10:19; h=contourslice(V,1:9,[],6,levels); n=numel(h);").unwrap();
    assert_eq!(e.variables["n"].data(), [10.]);
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.series.len(), 10);
    for (series, level) in figure.series.iter().zip(10..20) {
        assert_eq!(series.slice_contour_level, Some(level as f64));
        assert!(series.points.iter().any(|p| p[0].is_nan()));
        assert!(series.z.iter().any(|z| z.is_nan()));
    }
    assert!(figure.svg().contains("<svg"));
    // A folder, not an open NamedTempFile: on Windows save_figure cannot
    // replace a file that is still held open (Access is denied).
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("figure.mrfig");
    files::save_figure(&file, figure).unwrap();
    let restored = files::load_figure(&file).unwrap();
    assert_eq!(restored.series.len(), 10);
    for series in &restored.series {
        assert!(series.points.iter().any(|p| p[0].is_nan()));
        assert!(series.z.iter().any(|z| z.is_nan()));
    }
}

#[test]
#[ignore = "release-mode timing; run separately with SLICE_WORKERS=1 and 8"]
fn volume_slice_contours_benchmark() {
    let workers = std::env::var("SLICE_WORKERS").unwrap().parse().unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    e.execute("[X,Y,Z,V]=flow(40);").unwrap();
    let start = std::time::Instant::now();
    e.execute("h=contourslice(X,Y,Z,V,1:9,[],0,-1:0.1:-0.1);")
        .unwrap();
    eprintln!(
        "contourslice: workers={workers}, flow(40), ten planes and ten levels: {:?}",
        start.elapsed()
    );
}

#[test]
#[ignore = "release-mode timing; run in separate processes with SLICE_WORKERS=1 and 8"]
fn volume_slice_benchmark() {
    let workers = std::env::var("SLICE_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    e.execute("[X,Y,Z,V]=flow(40);").unwrap();
    let start = std::time::Instant::now();
    for _ in 0..10 {
        e.execute("h=slice(X,Y,Z,V,5,0,0);").unwrap();
    }
    eprintln!(
        "volume_slice: workers={workers}, flow(40), three planes, 10 calls: {:?}",
        start.elapsed()
    );
}
