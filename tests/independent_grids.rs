use matrust::{Array, Engine, files};
use std::sync::atomic::AtomicBool;
fn text(e: &Engine, name: &str) -> String {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!()
    };
    c.to_utf8(&AtomicBool::new(false)).unwrap()
}
#[test]
fn independent_grid_commands_are_atomic_and_toggle_each_direction() {
    let mut e = Engine::default();
    e.execute("f=figure;a=gca;h=plot(a,[1 2],[3 4]);set(a,'XGrid','off','YGrid',true);x=get(a,'XGrid');y=get(a,'YGrid');").unwrap();
    assert_eq!(text(&e, "x"), "off");
    assert_eq!(text(&e, "y"), "on");
    let before = e.figure.clone().unwrap();
    let data = before.series[0].points.clone();
    assert!(e.execute("set(a,'XGrid','on','YGrid',2);").is_err());
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute("grid;").unwrap();
    assert!(e.figure.as_ref().unwrap().xgrid);
    assert!(!e.figure.as_ref().unwrap().ygrid);
    e.execute("grid on;").unwrap();
    assert!(e.figure.as_ref().unwrap().xgrid && e.figure.as_ref().unwrap().ygrid);
    e.execute("grid(a,'off');").unwrap();
    assert!(!e.figure.as_ref().unwrap().xgrid && !e.figure.as_ref().unwrap().ygrid);
    e.execute("figure(2);set(a,'XGrid',1,'YGrid',0);").unwrap();
    assert_eq!(e.current_figure, 2);
    let f = e.figures().find(|(id, _)| *id == 1).unwrap().1;
    assert!(f.xgrid && !f.ygrid);
    assert!((data).same_storage(&f.series[0].points));
}

#[test]
fn svg_draws_only_requested_grid_directions_with_linear_and_log_ticks() {
    for script in ["plot([1 2 3],[2 4 3]);", "loglog([1 10 100],[1 10 100]);"] {
        let mut e = Engine::default();
        e.execute(script).unwrap();
        for x in [false, true] {
            for y in [false, true] {
                let mut f = e.figure.clone().unwrap();
                f.xgrid = x;
                f.ygrid = y;
                let svg = f.svg();
                let mut vertical = 0;
                let mut horizontal = 0;
                for part in svg.split("<path d=\"").skip(1) {
                    let tag = part.split('>').next().unwrap();
                    if tag.contains("stroke=\"#262626\"") {
                        let path = tag.split('"').next().unwrap();
                        vertical += usize::from(path.contains('V'));
                        horizontal += usize::from(path.contains('H'));
                    }
                }
                assert_eq!(vertical > 0, x, "{script}: x={x} y={y}");
                assert_eq!(horizontal > 0, y, "{script}: x={x} y={y}");
            }
        }
    }
}

#[test]
fn independent_grid_native_and_fig_roundtrips_preserve_both_flags_and_script_properties() {
    let directory = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("plot([1 2],[3 4]);").unwrap();
    for x in [false, true] {
        for y in [false, true] {
            let mut f = e.figure.clone().unwrap();
            f.appearance = None; // Exercise the pre-theme native capability.
            f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
            for axes in f.all_axes_mut() {
                axes.palette_cycle = None;
                for series in &mut axes.series {
                    series.auto_palette = None;
                }
            }
            f.xgrid = x;
            f.ygrid = y;
            for extension in ["mfig", "fig"] {
                let path = directory.path().join(format!("grid-{x}-{y}.{extension}"));
                files::save_figure(&path, &f).unwrap();
                let loaded = files::load_figure(&path).unwrap();
                assert_eq!((loaded.xgrid, loaded.ygrid), (x, y));
                assert_eq!(loaded.series[0].points, f.series[0].points);
                if extension == "mfig" {
                    let mut bytes = std::fs::read(&path).unwrap();
                    if x == y {
                        assert_eq!(&bytes[..8], b"MATRFIG4");
                        assert!(!String::from_utf8_lossy(&bytes).contains("ygrid"));
                    } else {
                        assert_eq!(&bytes[..8], b"MATRFIGJ");
                        bytes[7] = b'4';
                        std::fs::write(&path, bytes).unwrap();
                        assert!(
                            files::load_figure(&path)
                                .unwrap_err()
                                .contains("Advanced figure metadata requires MFIGJ")
                        );
                    }
                }
            }
            let script = files::matlab_plot_script(&f, &AtomicBool::new(false)).unwrap();
            assert!(script.contains(&format!(
                "'XGrid','{}','YGrid','{}'",
                if x { "on" } else { "off" },
                if y { "on" } else { "off" }
            )));
        }
    }
    e.execute("subplot(1,2,1);plot([1 2],[3 4]);subplot(1,2,2);plot([1 2],[4 3]);")
        .unwrap();
    let mut f = e.figure.unwrap();
    f.appearance = None; // Exercise the pre-theme native capability.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    f.xgrid = false;
    f.ygrid = true;
    let path = directory.path().join("subplot.mfig");
    files::save_figure(&path, &f).unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[..8], b"MATRFIGK");
    let loaded = files::load_figure(&path).unwrap();
    for ((p, a), (q, b)) in f.all_axes().zip(loaded.all_axes()) {
        assert_eq!(p, q);
        assert_eq!((a.xgrid, a.ygrid), (b.xgrid, b.ygrid));
    }
}

#[test]
fn new_cartesian_axes_start_without_grid_in_ordinary_and_layout_routes() {
    let axes = matrust::plot::Axes::default();
    assert!(!axes.xgrid && !axes.ygrid);
    for script in [
        "plot([1 2],[3 4]);",
        "loglog([1 10],[2 20]);",
        "subplot(1,2,1);plot([1 2]);subplot(1,2,2);plot([2 1]);",
        "tiledlayout(1,2);nexttile;plot([1 2]);nexttile;plot([2 1]);",
    ] {
        let mut engine = Engine::default();
        engine.execute(script).unwrap();
        let figure = engine.figure.as_ref().unwrap();
        assert!(
            figure.all_axes().all(|(_, a)| !a.xgrid && !a.ygrid),
            "{script}"
        );
        assert!(!figure.svg().contains("stroke=\"#262626\""), "{script}");
        engine
            .execute("a=gca;x=get(a,'XGrid');y=get(a,'YGrid');")
            .unwrap();
        assert_eq!(
            (text(&engine, "x"), text(&engine, "y")),
            ("off".into(), "off".into())
        );
    }
}

#[test]
fn modern_grid_uses_matlab_grid_color_at_alpha_015_and_legacy_light_stays_opaque_grey() {
    // MATLAB GridColor [0.15 0.15 0.15] at GridAlpha 0.15 blends to about #dedede on white.
    for (theme, stroke, color) in [
        ("", "stroke=\"#262626\" stroke-opacity=\"0.15\"", 0.15),
        (
            "figure('Theme','dark');",
            "stroke=\"#ffffff\" stroke-opacity=\"0.15\"",
            1.,
        ),
        // Explicit Light with a manual Color is the legacy policy: opaque grid, old grey.
        (
            "figure('Theme','light','Color',[1 1 1]);",
            "stroke=\"#dedede\"/>",
            222. / 255.,
        ),
    ] {
        let mut engine = Engine::default();
        engine
            .execute(&format!(
                "{theme}plot([1 2 3],[2 4 3]);grid on;c=get(gca,'GridColor');"
            ))
            .unwrap();
        let svg = engine.figure.as_ref().unwrap().svg();
        let grid = svg
            .split("<path d=\"M")
            .skip(1)
            .filter(|part| part.split_inclusive('>').next().unwrap().contains(stroke))
            .count();
        assert!(grid >= 4, "{theme}: {grid} grid lines");
        let Array::Numeric(c) = engine.array("c").unwrap() else {
            panic!("numeric GridColor")
        };
        assert_eq!(c.data().len(), 3);
        // The token is stored as bytes, so get returns 38/255 for 0.15.
        assert!(
            c.data().iter().all(|v| (v - color).abs() <= 1. / 255.),
            "{theme}: {:?}",
            c.data()
        );
    }
}

#[test]
fn default_off_grid_toggle_and_explicit_axes_properties_preserve_shared_samples() {
    let mut engine = Engine::default();
    engine.execute("plot([1 2],[3 4]);a=gca;").unwrap();
    let points = engine.figure.as_ref().unwrap().series[0].points.clone();
    engine.execute("grid;").unwrap();
    assert!(engine.figure.as_ref().unwrap().xgrid && engine.figure.as_ref().unwrap().ygrid);
    assert!(
        engine
            .figure
            .as_ref()
            .unwrap()
            .svg()
            .contains("stroke=\"#262626\"")
    );
    engine
        .execute("grid off;set(a,'YGrid','on');hold on;plot([1 2],[4 3]);")
        .unwrap();
    let figure = engine.figure.as_ref().unwrap();
    assert_eq!((figure.xgrid, figure.ygrid), (false, true));
    assert!(points.same_storage(&figure.series[0].points));
    let script = files::matlab_plot_script(figure, &AtomicBool::new(false)).unwrap();
    let mut replay = Engine::default();
    replay.execute(&script).unwrap();
    assert_eq!(
        (
            replay.figure.as_ref().unwrap().xgrid,
            replay.figure.as_ref().unwrap().ygrid
        ),
        (false, true)
    );
    assert_eq!(replay.figure.as_ref().unwrap().svg(), figure.svg());
}

#[test]
fn historical_missing_native_grid_stays_on_while_new_saves_explicitly_store_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-grid.mfig");
    // Authored original-version fixture: absent grid historically meant on.
    let metadata = br#"{"title":"old","xlabel":"x","ylabel":"y","count":1}"#;
    let mut bytes = b"MATRFIG1".to_vec();
    bytes.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    bytes.extend_from_slice(metadata);
    bytes.extend_from_slice(&1f64.to_le_bytes());
    bytes.extend_from_slice(&2f64.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    let old = files::load_figure(&path).unwrap();
    assert!(old.xgrid && old.ygrid && old.series[0].visible);
    files::save_figure(&path, &old).unwrap();
    let reopened = files::load_figure(&path).unwrap();
    assert!(reopened.xgrid && reopened.ygrid && reopened.series[0].visible);
    assert_eq!(reopened.svg(), old.svg());

    let mut engine = Engine::default();
    engine.execute("plot([1 2],[3 4]);").unwrap();
    let new = engine.figure.as_ref().unwrap();
    files::save_figure(&path, new).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let metadata: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
    assert_eq!(metadata["grid"], false);
    assert!(metadata.get("ygrid").is_none());
    let reopened = files::load_figure(&path).unwrap();
    assert!(!reopened.xgrid && !reopened.ygrid && reopened.series[0].visible);
    assert_eq!(reopened.svg(), new.svg());
}
