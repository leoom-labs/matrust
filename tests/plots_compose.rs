//! line, quiver/quiver3, contour3, fsurf, fimplicit, animatedline/addpoints and
//! print. Quiver reference: GNU Octave 10.3 (tool/refs/quiver/make_refs.m).
use matrust::{Array, Engine, Matrix, plot::Figure};

fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
    e
}
fn error_of(source: &str) -> String {
    match Engine::default().execute(source) {
        Ok(_) => panic!("{source} should fail"),
        Err(e) => e.to_string(),
    }
}
fn matrix(e: &Engine, name: &str) -> Matrix {
    match e.array(name) {
        Some(Array::Numeric(m)) => m,
        other => panic!("{name} is not numeric: {other:?}"),
    }
}
fn num(e: &Engine, name: &str) -> Vec<f64> {
    matrix(e, name).data().to_vec()
}
fn figure(e: &Engine) -> &Figure {
    e.figure.as_ref().expect("a figure")
}
fn close(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.is_nan() && w.is_nan()) || (g - w).abs() < 1e-12,
            "{got:?} vs {want:?}"
        );
    }
}

#[test]
fn quiver_scales_and_draws_heads_as_octave() {
    let e = run(
        "[x,y]=meshgrid(0:1,0:1); u=[1 0;2 1]; v=[0 1;1 -1]; h=quiver(x,y,u,v); \
                 X=h.XData; Y=h.YData; t=h.Type;",
    );
    // Octave 10.3: shafts (child 3) and heads (child 2), arrow k in column order.
    let nan = f64::NAN;
    let shaft_x = [
        [0., 0.284604989415154],
        [0., 0.569209978830308],
        [1., 1.],
        [1., 1.28460498941515],
    ];
    let shaft_y = [
        [0., 0.],
        [1., 1.28460498941515],
        [0., 0.284604989415154],
        [1., 0.715395010584846],
    ];
    let head_x = [
        [0.189736659610103, 0.284604989415154, 0.189736659610103],
        [0.411096095821889, 0.569209978830308, 0.347850542618522],
        [1.03162277660168, 1., 0.968377223398316],
        [1.15811388300842, 1.28460498941515, 1.22135943621179],
    ];
    let head_y = [
        [-0.0316227766016838, 0., 0.0316227766016838],
        [1.12649110640674, 1.28460498941515, 1.25298221281347],
        [0.189736659610103, 0.284604989415154, 0.189736659610103],
        [0.778640563788213, 0.715395010584846, 0.841886116991581],
    ];
    let (mut wx, mut wy) = (Vec::new(), Vec::new());
    for k in 0..4 {
        wx.extend(shaft_x[k]);
        wx.push(nan);
        wx.extend(head_x[k]);
        wx.push(nan);
        wy.extend(shaft_y[k]);
        wy.push(nan);
        wy.extend(head_y[k]);
        wy.push(nan);
    }
    // Octave rounds its printout to 15 digits.
    for (g, w) in num(&e, "X")
        .iter()
        .zip(&wx)
        .chain(num(&e, "Y").iter().zip(&wy))
    {
        assert!(
            (g.is_nan() && w.is_nan()) || (g - w).abs() < 1e-13,
            "{g} vs {w}"
        );
    }
    assert_eq!(num(&e, "X").len(), 28);
    assert_eq!(figure(&e).axes.series.len(), 1);
}

#[test]
fn quiver_options_scale_zero_no_heads_and_quiver3() {
    let e = run(
        "h=quiver([1 2],[1 1],[1 0],[0 1],0,'ShowArrowHead','off','Color','r'); X=h.XData; Y=h.YData; c=h.Color;",
    );
    close(&num(&e, "X"), &[1., 2., f64::NAN, 2., 2., f64::NAN]);
    close(&num(&e, "Y"), &[1., 1., f64::NAN, 1., 2., f64::NAN]);
    assert_eq!(num(&e, "c"), [1., 0., 0.]);
    let e = run("h=quiver3([0 0],[0 1],[0 0],[0 0],[0 0],[1 1],'off'); Z=h.ZData;");
    let z = num(&e, "Z");
    assert_eq!(z.len(), 14);
    close(&z[..3], &[0., 1., f64::NAN]);
    assert_eq!(figure(&e).axes.view, matrust::plot::view3d::VIEW_3D);
    assert!(error_of("quiver(1)").contains("quiver"));
    assert!(error_of("quiver([1 2],[1 2 3])").contains("same size"));
}

#[test]
fn contour3_draws_each_level_at_its_height() {
    let e = run("[C,h]=contour3(peaks(20),[-2 0 2]); C2=contourc(peaks(20),[-2 0 2]);");
    assert_eq!(num(&e, "C"), num(&e, "C2"));
    let f = figure(&e);
    assert_eq!(f.axes.view, matrust::plot::view3d::VIEW_3D);
    assert_eq!(f.axes.series.len(), 3);
    for (s, level) in f.axes.series.iter().zip([-2., 0., 2.]) {
        assert_eq!(s.z.len(), s.points.len());
        assert!(s.z.iter().all(|&z| z == level));
    }
    // Held 2-D axes keep their view.
    let e = run("plot(1:3); hold on; contour3(peaks(10));");
    assert_eq!(figure(&e).axes.view, matrust::plot::view3d::VIEW_2D);
}

#[test]
fn line_adds_to_the_axes_without_hold() {
    let e = run(
        "plot(1:3); h=line([1 3],[3 1],'Color','r'); g=line('XData',[0 1],'YData',[2 2]); \
                 k=line([0 1],[0 1],[0 1]); n=numel(get(gca,'Children')); Z=k.ZData; c=h.Color;",
    );
    assert_eq!(num(&e, "n"), [4.]);
    assert_eq!(num(&e, "Z"), [0., 1.]);
    assert_eq!(num(&e, "c"), [1., 0., 0.]);
    assert!(!figure(&e).hold);
}

#[test]
fn fsurf_and_fimplicit_sample_the_function() {
    let e = run("h=fsurf(@(x,y) x+y,[-1 1],'MeshDensity',3); X=h.XData; Z=h.ZData;");
    assert_eq!(matrix(&e, "Z").rows(), 3);
    close(&num(&e, "Z"), &[-2., -1., 0., -1., 0., 1., 0., 1., 2.]);
    // A function that is not vectorized falls back to point-by-point calls.
    let e = run("h=fsurf(@(x,y) x*y,[0 1 0 2],'MeshDensity',2); Z=h.ZData;");
    close(&num(&e, "Z"), &[0., 0., 0., 2.]);
    // The zero set of x^2+y^2-4 lies on the circle of radius 2.
    let e = run("h=fimplicit(@(x,y) x.^2+y.^2-4); X=h.XData; Y=h.YData; L=get(gca,'XLim');");
    let (x, y) = (num(&e, "X"), num(&e, "Y"));
    assert!(x.len() > 100);
    for (a, b) in x.iter().zip(&y).filter(|(a, _)| !a.is_nan()) {
        assert!(((a * a + b * b).sqrt() - 2.).abs() < 1e-2);
    }
    assert_eq!(num(&e, "L"), [-5., 5.]);
    assert!(error_of("fsurf(3)").contains("function handle"));
}

#[test]
fn animatedline_keeps_the_newest_points() {
    let e = run(
        "plot(1:3); a=animatedline('MaximumNumPoints',5); for k=1:8, addpoints(a,k,k^2); end; \
                 [x,y]=getpoints(a); n=numel(get(gca,'Children')); clearpoints(a); [x2,y2]=getpoints(a); \
                 b=animatedline([1 2],[3 4]); addpoints(b,3,5); [p,q]=getpoints(b);",
    );
    assert_eq!(num(&e, "x"), [4., 5., 6., 7., 8.]);
    assert_eq!(num(&e, "y"), [16., 25., 36., 49., 64.]);
    assert_eq!(num(&e, "n"), [2.]);
    assert!(num(&e, "x2").is_empty());
    assert_eq!(num(&e, "q"), [3., 4., 5.]);
    assert!(error_of("h=plot(1:3); addpoints(h,1,2)").contains("not an animated line"));
}

#[test]
fn print_writes_png_pdf_and_svg_files() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().display().to_string();
    let script = format!(
        "plot(1:3); print('-dpng','{base}/a'); print(gcf,'{base}/b.pdf','-dpdf'); \
         print('{base}/c.svg','-dsvg','-r300'); print('{base}/d.png'); print('{base}/e','-depsc');"
    );
    run(&script);
    for name in ["a.png", "b.pdf", "c.svg", "d.png", "e.eps"] {
        let len = std::fs::metadata(dir.path().join(name))
            .map(|m| m.len())
            .unwrap_or(0);
        assert!(len > 100, "{name} is empty");
    }
    assert!(error_of("plot(1:3); print").contains("printer"));
    assert!(error_of("plot(1:3); print('-dbmp','x')").contains("not supported yet"));
}

#[test]
fn review_fixes_rollback_lexing_defaults_and_markers() {
    // A failed print leaves the figures and the handle as they were.
    let e = run(
        "figure(1); h=plot(1:3); try; print(2,'x','-dbmp'); catch; end; ok=double(isgraphics(h)); n=get(gcf,'Number');",
    );
    assert_eq!(num(&e, "ok"), [1.]);
    assert_eq!(num(&e, "n"), [1.]);
    // A failed call after a leading axes handle restores the current figure.
    let e = run(
        "figure(1); a=gca; figure(2); try; line(a,[1 2],[3 4],'Bogus',1); catch; end; n=get(gcf,'Number');",
    );
    assert_eq!(num(&e, "n"), [2.]);
    // A variable named print still indexes with spaces.
    let e = run("print={10,20}; v = print {1};");
    assert_eq!(num(&e, "v"), [10.]);
    // Bare constructor, default line data, 2-D getpoints z.
    let e = run(
        "h=animatedline; addpoints(h,1,2); [x,y,z]=getpoints(h); g=line(); X=g.XData; Y=g.YData;",
    );
    assert_eq!((num(&e, "x"), num(&e, "z")), (vec![1.], vec![0.]));
    assert_eq!((num(&e, "X"), num(&e, "Y")), (vec![0., 1.], vec![0., 1.]));
    // Markers sit at the bases and the heads are hidden.
    let e = run("h=quiver([1 2],[0 0],0,'o'); X=h.XData; n=numel(get(gca,'Children'));");
    close(&num(&e, "X"), &[1., 2., f64::NAN, 2., 4., f64::NAN]);
    assert_eq!(num(&e, "n"), [2.]);
    // quiver3 expands a scalar Z over the grid.
    let e = run("h=quiver3(0,ones(2),zeros(2),ones(2)); Z=h.ZData;");
    assert_eq!(num(&e, "Z").len(), 4 * 7);
    // A function that is a matrix product on square grids is evaluated pointwise.
    let e = run("h=fsurf(@(x,y)(1-x.^2).*y.*(x*y),[-1 1],'MeshDensity',3); Z=h.ZData;");
    close(&num(&e, "Z"), &[0.; 9]);
}

#[test]
fn print_keeps_an_explicit_filename_and_records_of_deleted_lines_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().display().to_string();
    run(&format!("plot(1:3); print('{base}/out.pdf','-dpng');"));
    let bytes = std::fs::read(dir.path().join("out.pdf")).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
    assert!(!dir.path().join("out.pdf.png").exists());
    let e = run(
        "a=animatedline(); for k=1:1030; figure(2); b=animatedline(); clf; end; figure(1); addpoints(a,1,1); [x,y]=getpoints(a);",
    );
    assert_eq!(num(&e, "y"), [1.]);
}
