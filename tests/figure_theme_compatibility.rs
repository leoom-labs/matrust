//! Independent public compatibility cases for the pending figure-theme layers.
use matrust::{Array, Engine, files};
use std::sync::atomic::AtomicBool;

fn text(engine: &Engine, name: &str) -> String {
    let Array::Character(value) = engine.array(name).unwrap() else {
        panic!("Expected character query {name}");
    };
    value.to_utf8(&AtomicBool::new(false)).unwrap()
}

#[test]
fn engine_context_queries_and_auto_to_manual_capture_effective_values_without_sample_copies() {
    use matrust::plot::BaseTheme;
    let mut engine = Engine::default();
    assert_eq!(engine.graphics_theme_context(), BaseTheme::Light);
    engine
        .execute("f=figure;plot([-0 1 2],[2 4 3]);theme=get(f,'Theme');")
        .unwrap();
    assert_eq!(text(&engine, "theme"), "light");
    let before = engine.figure.clone().unwrap();
    let points = before.series[0].points.clone();
    engine.set_graphics_theme_context(BaseTheme::Dark);
    assert!(before.same_snapshot(engine.figure.as_ref().unwrap()));
    engine
        .execute("theme=get(f,'Theme');mode=get(f,'ThemeMode');")
        .unwrap();
    assert_eq!(text(&engine, "theme"), "dark");
    assert_eq!(text(&engine, "mode"), "auto");
    let effective = engine
        .figure
        .as_ref()
        .unwrap()
        .resolve_appearance(BaseTheme::Dark);
    engine
        .execute("set(f,'ColorMode','manual','ThemeMode','manual');")
        .unwrap();
    let captured = engine.figure.clone().unwrap();
    assert!(!before.same_snapshot(&captured));
    assert_eq!(captured.color, effective.canvas);
    assert!(captured.identity.same_object(&before.identity));
    assert!(points.same_storage(&captured.series[0].points));
    engine.set_graphics_theme_context(BaseTheme::Light);
    engine
        .execute("theme=get(f,'Theme');mode=get(f,'ThemeMode');canvas=get(f,'ColorMode');")
        .unwrap();
    assert_eq!(text(&engine, "theme"), "dark");
    assert_eq!(text(&engine, "mode"), "manual");
    assert_eq!(text(&engine, "canvas"), "manual");
    assert_eq!(
        engine
            .figure
            .as_ref()
            .unwrap()
            .resolve_appearance(BaseTheme::Light),
        effective
    );
    let cancel = engine.cancellation();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        engine
            .execute("set(f,'ThemeMode','auto','ColorMode','auto');")
            .is_err()
    );
    assert!(captured.same_snapshot(engine.figure.as_ref().unwrap()));
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    engine
        .execute("set(f,'ThemeMode','auto','ColorMode','auto');theme=get(f,'Theme');")
        .unwrap();
    assert_eq!(text(&engine, "theme"), "light");
    assert!(points.same_storage(&engine.figure.as_ref().unwrap().series[0].points));
}

#[test]
fn explicit_canvas_colors_remain_manual_including_white_and_invalid_sets_are_atomic() {
    for (literal, expected) in [
        ("[1 1 1]", [255, 255, 255]),
        ("[0 0 0]", [0, 0, 0]),
        ("[17 129 241]/255", [17, 129, 241]),
    ] {
        let mut engine = Engine::default();
        engine
            .execute("f=figure;h=plot([-0 1 2],[2 4 3]);")
            .unwrap();
        let original = engine.figure.clone().unwrap();
        let points = original.series[0].points.clone();
        engine
            .execute(&format!(
                "set(f,'Color',{literal});set(f,'Theme','dark');mode=get(f,'ColorMode');"
            ))
            .unwrap();
        assert_eq!(text(&engine, "mode"), "manual");
        let dark = engine.figure.as_ref().unwrap();
        assert_eq!(dark.color, expected);
        assert!(points.same_storage(&dark.series[0].points));
        assert!(dark.identity.same_object(&original.identity));
        let snapshot = dark.clone();
        assert!(
            engine
                .execute("set(f,'ColorMode','auto','Theme','invalid');")
                .is_err()
        );
        assert!(snapshot.same_snapshot(engine.figure.as_ref().unwrap()));
        engine
            .execute("set(f,'Theme','light');mode=get(f,'ColorMode');")
            .unwrap();
        assert_eq!(text(&engine, "mode"), "manual");
        assert_eq!(engine.figure.as_ref().unwrap().color, expected);
        assert!(points.same_storage(&engine.figure.as_ref().unwrap().series[0].points));
    }
}

#[test]
fn genuinely_absent_legacy_native_appearance_remains_legacy_after_save_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("historical.mfig");
    // Authored version-one fixture, not bytes produced by the new serializer.
    // Appearance, color, grid and modern series metadata are genuinely absent.
    let metadata = br#"{"title":"historical","xlabel":"x","ylabel":"y","count":2}"#;
    let mut bytes = b"MATRFIG1".to_vec();
    bytes.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    bytes.extend_from_slice(metadata);
    for value in [-0.0_f64, 2.0, 1.0, 3.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(&path, &bytes).unwrap();
    let historical = files::load_figure(&path).unwrap();
    assert!(historical.appearance.is_none());
    assert_eq!(historical.color, [255; 3]);
    assert!(historical.xgrid && historical.ygrid);
    assert_eq!(
        historical.series[0].points.iter().next().unwrap()[0].to_bits(),
        (-0.0_f64).to_bits()
    );
    let svg = historical.svg();
    files::save_figure(&path, &historical).unwrap();
    let reopened = files::load_figure(&path).unwrap();
    assert!(reopened.appearance.is_none());
    assert_eq!(reopened.color, historical.color);
    assert_eq!(reopened.svg(), svg);
    assert_eq!(reopened.series[0].points, historical.series[0].points);
    let script = files::matlab_plot_script(&historical, &AtomicBool::new(false)).unwrap();
    let mut replay = Engine::default();
    replay.set_graphics_theme_context(matrust::plot::BaseTheme::Dark);
    replay.execute(&script).unwrap();
    // Native keeps None; genuine public script expresses equivalent fixed
    // Light/manual intent rather than inventing a hidden legacy setter.
    assert_eq!(
        replay
            .figure
            .as_ref()
            .unwrap()
            .svg_with_theme(matrust::plot::BaseTheme::Dark),
        svg
    );
}

#[test]
fn theme_and_canvas_intent_survive_native_and_two_genuine_script_cycles() {
    let directory = tempfile::tempdir().unwrap();
    for setup in [
        "set(f,'ThemeMode','auto','ColorMode','auto');",
        "set(f,'Theme','dark','Color',[1 1 1]);",
        "set(f,'Theme','light','Color',[17 129 241]/255);",
    ] {
        let mut source = Engine::default();
        source.execute("f=figure;plot([-0 1 2],[2 4 3]);").unwrap();
        source.execute(setup).unwrap();
        source
            .execute("theme=get(f,'Theme');mode=get(f,'ThemeMode');canvas=get(f,'ColorMode');")
            .unwrap();
        let expected = [
            text(&source, "theme"),
            text(&source, "mode"),
            text(&source, "canvas"),
        ];
        let figure = source.figure.clone().unwrap();
        let path = directory.path().join("intent.mfig");
        files::save_figure(&path, &figure).unwrap();
        let native = files::load_figure(&path).unwrap();
        assert_eq!(native.appearance, figure.appearance);
        assert_eq!(native.color, figure.color);
        for companion in [false, true] {
            let mut current = native.clone();
            for pass in 0..2 {
                let mut replay = Engine::default();
                if companion {
                    let script_path = directory.path().join(format!("theme_{pass}.m"));
                    files::write_matlab_plot_bundle_with_cancel(
                        &script_path,
                        &current,
                        &AtomicBool::new(false),
                    )
                    .unwrap();
                    let document = matrust::document::Document::open(&script_path).unwrap();
                    replay
                        .execute_document(&document.text, document.path.as_deref().unwrap())
                        .unwrap();
                } else {
                    let script =
                        files::matlab_plot_script(&current, &AtomicBool::new(false)).unwrap();
                    replay.execute(&script).unwrap();
                }
                replay.execute("theme=get(gcf,'Theme');mode=get(gcf,'ThemeMode');canvas=get(gcf,'ColorMode');").unwrap();
                assert_eq!(
                    [
                        text(&replay, "theme"),
                        text(&replay, "mode"),
                        text(&replay, "canvas")
                    ],
                    expected
                );
                current = replay.figure.unwrap();
                assert_eq!(current.appearance, figure.appearance);
                assert_eq!(current.color, figure.color);
                for (actual, wanted) in current.series[0]
                    .points
                    .iter()
                    .zip(figure.series[0].points.iter())
                {
                    assert_eq!(actual.map(f64::to_bits), wanted.map(f64::to_bits));
                }
                assert_eq!(
                    current.series[0].points.len(),
                    figure.series[0].points.len()
                );
            }
        }
        assert!(figure.same_snapshot(source.figure.as_ref().unwrap()));
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn manual_nonwhite_raster_margins_match_sized_svg_and_cancel_preserves_destination() {
    use matrust::{export, plot::Canvas};
    let mut engine = Engine::default();
    engine
        .execute("f=figure('Color',[17 129 241]/255);plot([0 1],[0 1]);")
        .unwrap();
    let figure = engine.figure.as_ref().unwrap();
    let snapshot = figure.clone();
    let canvas = Canvas {
        width: 640.,
        height: 640.,
    };
    let svg = figure.svg_on_canvas(canvas).unwrap();
    let tree = resvg::usvg::Tree::from_str(&svg, &export::svg_render_options()).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(640, 640).unwrap();
    resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
    let raster =
        export::render_figure_with_cancel(figure, 640, 640, &AtomicBool::new(false)).unwrap();
    assert_eq!(raster.get_pixel(0, 0).0, [17, 129, 241, 255]);
    assert_eq!(raster.get_pixel(639, 639).0, [17, 129, 241, 255]);
    assert_eq!(pixmap.data(), raster.as_raw());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("keep.png");
    std::fs::write(&path, b"existing destination").unwrap();
    assert!(
        export::export_figure_with_cancel(figure, &path, 640, 640, &AtomicBool::new(true)).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
    assert!(snapshot.same_snapshot(figure));
    assert!(
        snapshot.series[0]
            .points
            .same_storage(&figure.series[0].points)
    );
}

#[cfg(feature = "plot-export")]
#[test]
fn automatic_export_context_is_explicit_and_default_wrappers_remain_light() {
    use matrust::{
        export,
        plot::{BaseTheme, Canvas},
    };
    let mut engine = Engine::default();
    engine.execute("plot([0 1],[0 1]);").unwrap();
    let figure = engine.figure.as_ref().unwrap();
    let snapshot = figure.clone();
    let cancel = AtomicBool::new(false);
    let light =
        export::render_figure_in_theme_with_cancel(figure, [640, 640], BaseTheme::Light, &cancel)
            .unwrap();
    assert_eq!(
        export::render_figure_with_cancel(figure, 640, 640, &cancel)
            .unwrap()
            .as_raw(),
        light.as_raw()
    );
    let dark =
        export::render_figure_in_theme_with_cancel(figure, [640, 640], BaseTheme::Dark, &cancel)
            .unwrap();
    assert_ne!(dark.as_raw(), light.as_raw());
    for (theme, raster) in [(BaseTheme::Light, &light), (BaseTheme::Dark, &dark)] {
        let palette = figure.resolve_appearance(theme);
        assert_eq!(
            raster.get_pixel(0, 0).0,
            [palette.canvas[0], palette.canvas[1], palette.canvas[2], 255]
        );
        let svg = figure
            .svg_on_canvas_with_theme(
                Canvas {
                    width: 640.,
                    height: 640.,
                },
                theme,
            )
            .unwrap();
        let tree = resvg::usvg::Tree::from_str(&svg, &export::svg_render_options()).unwrap();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(640, 640).unwrap();
        resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
        assert_eq!(pixmap.data(), raster.as_raw());
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dark.png");
    export::export_figure_in_theme_with_cancel(figure, &path, [640, 640], BaseTheme::Dark, &cancel)
        .unwrap();
    assert_eq!(
        image::open(&path).unwrap().to_rgba8().as_raw(),
        dark.as_raw()
    );
    assert!(snapshot.same_snapshot(figure));
    assert!(
        snapshot.series[0]
            .points
            .same_storage(&figure.series[0].points)
    );
}

#[test]
fn light_static_fig_export_preserves_explicit_canvas_and_never_mutates_source() {
    use matrust::plot::BaseTheme;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("static.fig");
    for properties in ["", ",'Color',[1 1 1]", ",'Color',[17 129 241]/255"] {
        let mut engine = Engine::default();
        assert_eq!(engine.graphics_theme_context(), BaseTheme::Light);
        engine
            .execute(&format!(
                "f=figure('Name','static'{properties});plot([0 1],[2 3],'Color',[1 0 0]);"
            ))
            .unwrap();
        let source = engine.figure.as_ref().unwrap();
        let before = source.clone();
        let effective = source.resolve_appearance(BaseTheme::Light);
        files::save_figure(&path, source).unwrap();
        let loaded = files::load_figure(&path).unwrap();
        assert_eq!(loaded.color, effective.canvas);
        assert!(loaded.appearance.is_none());
        assert_eq!(loaded.series[0].color, [255, 0, 0]);
        assert_eq!(loaded.series[0].points, source.series[0].points);
        assert!(before.same_snapshot(source));
        assert!(
            before.series[0]
                .points
                .same_storage(&source.series[0].points)
        );
        engine.execute("set(f,'Theme','dark');").unwrap();
        engine.figure.as_mut().unwrap().series[0].kind = matrust::plot::PlotKind::Stairs;
        let dark = engine.figure.as_ref().unwrap();
        let dark_before = dark.clone();
        std::fs::write(&path, b"preserved destination").unwrap();
        let error = files::save_figure(&path, dark).unwrap_err();
        assert!(error.contains("MFIG"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"preserved destination");
        assert!(dark_before.same_snapshot(dark));
    }
}

#[test]
fn ordinary_clf_keeps_figure_intent_name_and_identity_for_the_next_plot() {
    use matrust::plot::BaseTheme;
    let mut engine = Engine::default();
    engine.set_graphics_theme_context(BaseTheme::Dark);
    engine
        .execute("f=figure('Name','retained','Color',[1 1 1],'Theme','dark');plot([0 1],[2 3]);")
        .unwrap();
    let before = engine.figure.clone().unwrap();
    let samples = before.series[0].points.clone();
    engine
        .execute("clf;name=get(f,'Name');mode=get(f,'ColorMode');theme=get(f,'Theme');")
        .unwrap();
    assert_eq!(text(&engine, "name"), "retained");
    assert_eq!(text(&engine, "mode"), "manual");
    assert_eq!(text(&engine, "theme"), "dark");
    let cleared = engine.figure.as_ref().unwrap();
    assert_eq!(cleared.series_count(), 0);
    assert_eq!(cleared.appearance, before.appearance);
    assert_eq!(cleared.color, before.color);
    assert!(cleared.identity.same_object(&before.identity));
    engine.execute("plot([4 5],[6 7]);").unwrap();
    assert_eq!(
        engine.figure.as_ref().unwrap().appearance,
        before.appearance
    );
    assert_eq!(engine.figure.as_ref().unwrap().color, [255; 3]);
    assert!(samples.same_storage(&before.series[0].points));
    assert_eq!(samples.iter().collect::<Vec<_>>(), vec![[0., 2.], [1., 3.]]);
}

#[test]
fn session_store_restores_exact_theme_intent_and_typed_aliases_under_a_new_host_context() {
    use matrust::{
        document::Document,
        plot::BaseTheme,
        session::{
            Session, SessionStore,
            figures::{FigureDocument, FigureSession, restore_figures},
            workspace,
        },
        simulation::Model,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("themes.json");
    let mut source = Engine::default();
    source.set_graphics_theme_context(BaseTheme::Dark);
    source.execute("A=[-0 1 2];AA=A;fa=figure(1);plot(A,[2 4 3]);alias=fa;fd=figure(2);set(fd,'Theme','dark');plot(A,[4 3 2]);fw=figure(3);set(fw,'Color',[1 1 1]);plot(A,[3 2 4]);holders={fa,fd,fw,alias};").unwrap();
    let originals: Vec<_> = source.figures().map(|(id, f)| (id, f.clone())).collect();
    let state = Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(source.workspace()),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: Some(FigureSession {
            current: source.current_figure,
            figures: source
                .figures()
                .map(|(id, f)| FigureDocument {
                    window: None,
                    id,
                    figure: f.clone(),
                    path: None,
                    source_version: None,
                    dirty: true,
                    view: None,
                })
                .collect(),
        }),
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: directory.path().to_path_buf(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    };
    SessionStore::new(&path).save(state).unwrap();
    for (id, before) in &originals {
        let current = source.figures().find(|(n, _)| n == id).unwrap().1;
        assert!(before.same_snapshot(current));
        assert!(
            before.series[0]
                .points
                .same_storage(&current.series[0].points)
        );
    }
    let saved = Session::load(&path).unwrap();
    let stop = AtomicBool::new(false);
    let report = restore_figures(&path, saved.figure_manifest.as_ref().unwrap(), &stop);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let bindings = report.workspace_bindings().clone();
    let mut restored = Engine::default(); // Recovery does not persist a prior host's transient context.
    assert_eq!(restored.graphics_theme_context(), BaseTheme::Light);
    restored
        .restore_figures(
            report.figures.current,
            report
                .figures
                .figures
                .into_iter()
                .map(|d| (d.id, d.figure))
                .collect(),
        )
        .unwrap();
    let recovered = workspace::restore_with_figures(
        &path,
        saved.workspace_manifest.as_ref().unwrap(),
        &bindings,
        &stop,
    )
    .unwrap();
    assert!(recovered.warnings.is_empty(), "{:?}", recovered.warnings);
    restored.merge_workspace(recovered.values).unwrap();
    for (id, before) in &originals {
        let current = restored.figures().find(|(n, _)| n == id).unwrap().1;
        assert!(!before.identity.same_object(&current.identity));
        assert_eq!(before.appearance, current.appearance);
        assert_eq!(before.color, current.color);
        let original_path = directory.path().join(format!("before_{id}.mfig"));
        let restored_path = directory.path().join(format!("restored_{id}.mfig"));
        files::save_figure(&original_path, before).unwrap();
        files::save_figure(&restored_path, current).unwrap();
        assert_eq!(
            std::fs::read(&original_path).unwrap(),
            std::fs::read(&restored_path).unwrap()
        );
        assert_eq!(
            before.series[0].points.len(),
            current.series[0].points.len()
        );
        for (a, b) in before.series[0]
            .points
            .iter()
            .zip(current.series[0].points.iter())
        {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
    let Array::Graphics(fa) = restored.array("fa").unwrap() else {
        panic!("typed figure")
    };
    let Array::Graphics(alias) = restored.array("alias").unwrap() else {
        panic!("typed alias")
    };
    assert!(fa.same_storage(&alias));
    let Array::Cell(holders) = restored.array("holders").unwrap() else {
        panic!("typed cell")
    };
    assert!(
        holders
            .get_linear(0)
            .unwrap()
            .same_snapshot(&restored.array("fa").unwrap())
    );
    assert!(
        holders
            .get_linear(3)
            .unwrap()
            .same_snapshot(&restored.array("alias").unwrap())
    );
    assert!(
        restored
            .array("A")
            .unwrap()
            .same_snapshot(&restored.array("AA").unwrap())
    );
    restored.execute("ta=get(fa,'Theme');td=get(fd,'Theme');cw=get(fw,'Color');mw=get(fw,'ColorMode');live=isgraphics(alias);").unwrap();
    assert_eq!(text(&restored, "ta"), "light");
    assert_eq!(text(&restored, "td"), "dark");
    assert_eq!(text(&restored, "mw"), "manual");
    assert_eq!(restored.variables["cw"].data(), &[1., 1., 1.]);
    assert_eq!(restored.logical_variables["live"].data(), &[1]);
    let snapshots: Vec<_> = restored.figures().map(|(id, f)| (id, f.clone())).collect();
    restored.set_graphics_theme_context(BaseTheme::Dark);
    restored
        .execute("ta=get(alias,'Theme');cw=get(fw,'Color');")
        .unwrap();
    assert_eq!(text(&restored, "ta"), "dark");
    assert_eq!(restored.variables["cw"].data(), &[1., 1., 1.]);
    for (id, before) in snapshots {
        let current = restored.figures().find(|(n, _)| *n == id).unwrap().1;
        assert!(before.same_snapshot(current));
        assert!(
            before.series[0]
                .points
                .same_storage(&current.series[0].points)
        );
    }
}
