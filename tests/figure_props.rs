use matrust::Engine;

#[test]
fn figure_position_get_set_dot_and_indexed_assign() {
    let mut e = Engine::default();
    e.execute(
        "f = figure;
         p0 = get(gcf, 'Position');
         ip0 = get(f, 'InnerPosition');
         op0 = get(f, 'OuterPosition');
         set(gcf, 'Position', [100 100 600 400]);
         p1 = get(gcf, 'Position');
         f.Position(3) = 800;
         p2 = f.Position;
         ip2 = f.InnerPosition;
         op2 = f.OuterPosition;",
    )
    .unwrap();
    assert_eq!(e.variables["p0"].data(), [100., 100., 960., 560.]);
    assert_eq!(e.variables["ip0"].data(), [100., 100., 960., 560.]);
    assert_eq!(e.variables["op0"].data(), [100., 100., 960., 560.]);
    assert_eq!(e.variables["p1"].data(), [100., 100., 600., 400.]);
    assert_eq!(e.variables["p2"].data(), [100., 100., 800., 400.]);
    assert_eq!(e.variables["ip2"].data(), [100., 100., 800., 400.]);
    assert_eq!(e.variables["op2"].data(), [100., 100., 800., 400.]);

    assert!(e.execute("set(gcf, 'Position', [0 0 0 100]);").is_err());
    assert!(e.execute("set(gcf, 'Position', [0 0 100 -10]);").is_err());
    assert!(e.execute("set(gcf, 'Position', [0 0 NaN 100]);").is_err());
    assert!(e.execute("set(gcf, 'Position', [1 2 3]);").is_err());
}

#[test]
fn figure_units_convert_position_and_round_trip() {
    let mut e = Engine::default();
    e.execute(
        "f = figure('Position', [192 108 960 540]);
         f.Units = 'normalized';
         norm = f.Position;
         f.Units = 'inches';
         inch = f.Position;
         f.Units = 'centimeters';
         cm = f.Position;
         f.Units = 'points';
         pts = f.Position;
         f.Units = 'characters';
         chars = f.Position;
         f.Units = 'pixels';
         px = f.Position;",
    )
    .unwrap();
    assert_eq!(e.variables["norm"].data(), [0.1, 0.1, 0.5, 0.5]);
    assert_eq!(e.variables["inch"].data(), [2.0, 1.125, 10.0, 5.625]);
    for (a, b) in e.variables["cm"]
        .data()
        .iter()
        .zip([5.08, 2.8575, 25.4, 14.2875])
    {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }
    assert_eq!(e.variables["pts"].data(), [144.0, 81.0, 720.0, 405.0]);
    for (a, b) in e.variables["chars"].data().iter().zip([
        192.0 / 5.0,
        108.0 / 13.0,
        960.0 / 5.0,
        540.0 / 13.0,
    ]) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }
    for (a, b) in e.variables["px"]
        .data()
        .iter()
        .zip([192.0, 108.0, 960.0, 540.0])
    {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }
}

#[test]
fn figure_name_numbertitle_visible_resize_windowstate_color() {
    let mut e = Engine::default();
    e.execute(
        "f = figure('Name', 'A', 'NumberTitle', 'off', 'Visible', 'off', 'Resize', 'off', 'WindowState', 'maximized', 'Color', [0.2 0.4 0.6]);
         ok_name = double(strcmp(f.Name, 'A'));
         ok_nt = double(strcmp(f.NumberTitle, 'off'));
         ok_vis = double(strcmp(f.Visible, 'off'));
         ok_res = double(strcmp(f.Resize, 'off'));
         ok_ws = double(strcmp(f.WindowState, 'maximized'));
         c = f.Color;
         f.WindowState = 'fullscreen';
         ok_ws2 = double(strcmp(get(f, 'WindowState'), 'fullscreen'));
         set(f, 'Visible', 'on', 'NumberTitle', 'on', 'Resize', 'on', 'WindowState', 'minimized');
         ok_vis2 = double(strcmp(get(f, 'Visible'), 'on'));
         ok_nt2 = double(strcmp(get(f, 'NumberTitle'), 'on'));
         ok_res2 = double(strcmp(get(f, 'Resize'), 'on'));
         ok_ws3 = double(strcmp(get(f, 'WindowState'), 'minimized'));",
    )
    .unwrap();
    for key in [
        "ok_name", "ok_nt", "ok_vis", "ok_res", "ok_ws", "ok_ws2", "ok_vis2", "ok_nt2", "ok_res2",
        "ok_ws3",
    ] {
        assert_eq!(e.variables[key].data(), [1.], "{key} failed");
    }
    let c = e.variables["c"].data();
    assert!((c[0] - 51. / 255.).abs() < 1e-6);
    assert!((c[1] - 102. / 255.).abs() < 1e-6);
    assert!((c[2] - 153. / 255.).abs() < 1e-6);
}

#[test]
fn figure_props_saved_and_loaded_with_savefig_openfig() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "f = figure('Name', 'Saved', 'NumberTitle', 'off', 'Visible', 'off', 'Resize', 'off', 'WindowState', 'maximized', 'Units', 'inches', 'Position', [1 2 8 5]);
         plot([1 2 3], [4 5 6]);
         savefig(f, 'props.mrfig');
         close(f);
         g = openfig('props.mrfig');
         p = g.Position;
         ok = double(strcmp(g.Name, 'Saved') && strcmp(g.NumberTitle, 'off') && strcmp(g.Visible, 'off') && strcmp(g.Resize, 'off') && strcmp(g.WindowState, 'maximized') && strcmp(g.Units, 'inches'));",
    )
    .unwrap();
    assert_eq!(e.variables["ok"].data(), [1.]);
    assert_eq!(e.variables["p"].data(), [1., 2., 8., 5.]);
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_and_saveas_follow_figure_position_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    // Invisible figures are still exportable, and exports without an explicit
    // size use Position width/height.
    // saveas PNG uses exact Position pixel dimensions (600x400).
    // exportgraphics tight-crops to ink bounds inside the 600x400 canvas and
    // scales by Resolution/96 (default Resolution = 150 DPI).
    e.execute(
        "f = figure('Position', [100 100 600 400], 'Visible', 'off');
         plot([1 2 3], [1 4 9]);
         saveas(f, 'saveas600.png');
         exportgraphics(f, 'eg600.png');
         exportgraphics(f, 'eg600_96.png', 'Resolution', 96);
         set(f, 'Position', [100 100 300 200]);
         exportgraphics(f, 'eg300_96.png', 'Resolution', 96);",
    )
    .unwrap();

    let saveas_img = image::open(dir.path().join("saveas600.png")).unwrap();
    assert_eq!((saveas_img.width(), saveas_img.height()), (600, 400));

    let eg600 = image::open(dir.path().join("eg600.png")).unwrap();
    let eg600_96 = image::open(dir.path().join("eg600_96.png")).unwrap();
    let eg300_96 = image::open(dir.path().join("eg300_96.png")).unwrap();
    assert!(eg600_96.width() <= 600 && eg600_96.width() > 300);
    assert!(eg300_96.width() <= 300 && eg300_96.width() > 0);
    assert_eq!(
        eg600.width(),
        (eg600_96.width() * 150).div_ceil(96),
        "default exportgraphics Resolution is 150 DPI over 96 DPI base"
    );
}
