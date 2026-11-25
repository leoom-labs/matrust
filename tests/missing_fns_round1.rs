use matrust::{
    Array, Engine,
    plot::{LinePattern, PlotKind, view3d},
};

fn run(source: &str) -> Engine {
    let mut engine = Engine::default();
    engine.execute(source).unwrap();
    engine
}

#[test]
fn gpu_queries_report_no_devices() {
    let engine = run(
        "n=gpuDeviceCount; na=gpuDeviceCount(\"available\"); gpu=canUseGPU; \
                      try; gpuDevice; catch err; id=err.identifier; msg=err.message; end; \
                      assert(strcmp(id,'parallel:gpu:device:NoDevicesAvailable')); \
                      assert(strcmp(msg,'No supported GPU device was found.'));",
    );
    assert_eq!(engine.variables["n"].data(), &[0.]);
    assert_eq!(engine.variables["na"].data(), &[0.]);
    assert!(matches!(engine.array("gpu"), Some(Array::Logical(m)) if !m.as_scalar().unwrap()));
}

#[test]
fn ishold_reads_current_and_inactive_axes_without_selecting_them() {
    let engine = run("empty = ishold; f=figure(1); a=subplot(1,2,1); hold on; \
                      b=subplot(1,2,2); assert(ishold(a)); assert(~ishold(b)); \
                      assert(~ishold); assert(gca==b); \
                      figure(2); assert(ishold(a)); assert(gcf~=f); hold on; assert(ishold());");
    assert!(matches!(engine.array("empty"), Some(Array::Logical(m)) if !m.as_scalar().unwrap()));
}

#[test]
fn pcolor_keeps_surface_data_and_uses_only_the_first_vertex_color() {
    // MathWorks pcolor: the last row and column supply vertices but no flat face colours.
    let engine = run("C=[1 2 9;3 4 9;9 9 9]; h=pcolor(C); \
                      assert(strcmp(get(h,'Type'),'surface')); assert(strcmp(h.FaceColor,'flat')); \
                      assert(isequal(h.CData,C)); assert(isequal(h.ZData,zeros(3))); \
                      assert(isequal(h.XData,1:3)); assert(isequal(h.YData,(1:3)')); \
                      colormap([0 0 0;1 0 0;0 1 0;0 0 1]); clim([1 5]);");
    let figure = engine.figure.unwrap();
    let series = &figure.axes.series[0];
    assert_eq!(figure.axes.view, view3d::VIEW_2D);
    assert_eq!(series.patch_face_count(), 4);
    assert_eq!(series.face_edge_color, Some([0; 3]));
    assert_eq!(
        &**series.face_colors.as_ref().unwrap(),
        &[[0, 0, 0], [0, 255, 0], [255, 0, 0], [0, 0, 255]]
    );

    let engine = run("h=pcolor([2 4],[10 20],[1 NaN;NaN NaN]);");
    let series = &engine.figure.as_ref().unwrap().axes.series[0];
    assert_eq!(series.patch_face_count(), 1);
    assert_eq!(series.points.get(0), Some([2., 10.]));
    assert_eq!(series.points.get(2), Some([4., 20.]));
    assert!(series.z.iter().all(|&value| value == 0.));
}

#[test]
fn sphere_uses_latitudes_as_rows_and_bare_calls_draw() {
    // Octave's documented spherical-to-Cartesian formula, with quarter-turn
    // longitude samples and the poles at latitude +/- pi/2.
    let engine = run("[X,Y,Z]=sphere(4); [xd,yd,zd]=sphere; \
                      assert(isequal(size(xd),[21 21])); assert(isequal(size(yd),size(zd))); \
                      assert(max(abs(X(:).^2 + Y(:).^2 + Z(:).^2-1)) < 1e-14);");
    let x = &engine.variables["X"];
    let y = &engine.variables["Y"];
    let z = &engine.variables["Z"];
    assert!(engine.figure.is_none());
    assert_eq!((x.rows(), x.cols()), (5, 5));
    assert_eq!(x.get(0, 0), 0.);
    assert_eq!(x.get(2, 0), 1.);
    assert_eq!(y.get(2, 1), 1.);
    assert!(
        z.data()
            .as_chunks::<5>()
            .0
            .iter()
            .all(|col| col[0] == -1. && col[4] == 1.)
    );
    for (source, count) in [("sphere;", 400), ("sphere(4);", 16)] {
        let engine = run(source);
        let figure = engine.figure.unwrap();
        assert_eq!(figure.axes.series[0].patch_face_count(), count);
    }
}

#[test]
fn surfl_colors_depend_on_surface_normals() {
    // For z = x, the oriented unit normal is (-1,0,1)/sqrt(2).
    let engine = run("[X,Y]=meshgrid(1:3); h=surfl(X,Y,X); c=get(h,'CData'); \
                      assert(strcmp(h.Type,'surface')); assert(isequal(h.ZData,X)); \
                      h2=surfl([0 0 2;0 0 2;0 0 2]); slope=h2.CData;");
    let expected = 0.2 + 0.8 / 12_f64.sqrt();
    for &value in engine.variables["c"].data() {
        assert!((value - expected).abs() < 1e-14);
    }
    let slope = &engine.variables["slope"];
    assert!(slope.get(0, 0) > slope.get(0, 1));
    let figure = engine.figure.unwrap();
    assert_eq!(figure.axes.view, view3d::VIEW_3D);
    assert_eq!(figure.axes.series[0].patch_face_count(), 4);
}

#[test]
fn triangle_and_tetrahedron_plots_expose_indexed_patch_geometry() {
    let engine = run("T=[1 2 3;2 4 3]; V=[0 0 0;1 0 1;0 1 2;1 1 3]; \
                      colormap([1 0 0;0 0 1]); \
                      h=trisurf(T,V(:,1),V(:,2),V(:,3),[0 1 0 1]); \
                      assert(strcmp(h.Type,'patch')); assert(isequal(h.Faces,T)); \
                      assert(isequal(get(h,'Vertices'),V)); \
                      assert(isequal(h.FaceVertexCData,[0;1;0;1])); \
                      hold on; m=trimesh(T,V(:,1),V(:,2),V(:,3)); \
                      assert(strcmp(m.FaceColor,'none')); assert(isequal(m.Faces,T)); \
                      d=tetramesh([1 2 3 4],V); assert(strcmp(d.Type,'patch')); \
                      assert(isequal(d.Vertices,V)); \
                      assert(isequal(d.Faces,[1 3 2;1 2 4;1 4 3;2 3 4]));");
    let figure = engine.figure.unwrap();
    let [triangles, wire, tetra] = figure.axes.series.as_slice() else {
        panic!("expected three patches")
    };
    assert_eq!(triangles.kind, PlotKind::Patch);
    assert_eq!(&*triangles.z, &[0., 1., 2., 1., 3., 2.]);
    assert_eq!(
        &**triangles.face_colors.as_ref().unwrap(),
        &[[255, 0, 0], [0, 0, 255]]
    );
    assert_eq!(wire.pattern, LinePattern::Solid);
    assert_eq!(wire.patch_face_count(), 2);
    assert_eq!(tetra.patch_face_count(), 4);
}

#[test]
fn eqtflength_aligns_coefficients_and_trims_common_zero_tails() {
    // The published signal reference pads [0 0 2] to match [4 0 3 -1].
    let engine = run("[b,a]=eqtflength([1 2],[1 2 3 0]); \
                      [p,q]=eqtflength([0;0;2],[4;0;3;-1]); \
                      assert(isequal(p,[0 0 2 0])); assert(isequal(q,[4 0 3 -1])); \
                      [c,d]=eqtflength([1i 0 0],[1 2]); \
                      assert(isequal(c,[1i 0])); assert(isequal(d,[1 2])); \
                      first=eqtflength([2 0],[1 2 3]); assert(isequal(first,[2 0 0]));");
    assert_eq!(engine.variables["b"].data(), &[1., 2., 0.]);
    assert_eq!(engine.variables["a"].data(), &[1., 2., 3.]);
}

#[test]
fn invalid_forms_fail_before_replacing_existing_plots() {
    let mut engine = run("f=figure(1); ax=gca; h=plot(1:3);");
    for source in [
        "pcolor([1 2],[1 2],[1 2 3;4 5 6]);",
        "sphere(1000);",
        "surfl([1 2 3]);",
        "trisurf([1 2 4],[0 1 0],[0 0 1],[0 1 2]);",
        "tetramesh([1 2 3 0],ones(4,3));",
        "b=eqtflength(ones(2),[1 2]);",
        "held=ishold(f);",
    ] {
        assert!(engine.execute(source).is_err(), "{source}");
        engine
            .execute("assert(isgraphics(h)); assert(gca==ax);")
            .unwrap();
    }
}
