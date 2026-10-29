use super::*;
use crate::{Engine, files};
use std::sync::atomic::AtomicBool;

#[test]
fn independent_grid_mode_table_and_alpha_keep_original_values() {
    for x_mode in [ColorMode::Auto, ColorMode::Manual] {
        for y_mode in [ColorMode::Auto, ColorMode::Manual] {
            for color_mode in [ColorMode::Auto, ColorMode::Manual] {
                for (alpha, alpha_mode) in [
                    (0., ColorMode::Manual),
                    (0.4, ColorMode::Manual),
                    (1., ColorMode::Manual),
                    (0.15, ColorMode::Auto),
                ] {
                    let grid = AxesGridStyle {
                        color: [0.2, 0.4, 0.6],
                        alpha,
                        x_mode,
                        y_mode,
                        color_mode,
                        alpha_mode,
                    };
                    let mut figure = Figure::default();
                    figure.axes.colors = Some(AxesColors {
                        x: Some([1., 0., 0.]),
                        y: Some([0., 0., 1.]),
                        grid: Some(grid),
                        ..Default::default()
                    });
                    figure
                        .add(vec![Series::new(vec![[0., 1.]], PlotKind::Line)])
                        .unwrap();
                    let resolved = figure
                        .axes
                        .resolve_colors(figure.resolve_appearance(BaseTheme::Dark));
                    assert_eq!(
                        resolved.grid_x,
                        if color_mode == ColorMode::Auto && x_mode == ColorMode::Manual {
                            [255, 0, 0]
                        } else {
                            [51, 102, 153]
                        }
                    );
                    assert_eq!(
                        resolved.grid_y,
                        if color_mode == ColorMode::Auto && y_mode == ColorMode::Manual {
                            [0, 0, 255]
                        } else {
                            [51, 102, 153]
                        }
                    );
                    assert_eq!(resolved.grid_alpha.to_bits(), alpha.to_bits());
                    let native =
                        files::decode_native_figure(&files::encode_native_figure(&figure).unwrap())
                            .unwrap();
                    assert_eq!(native.colors, figure.colors);
                }
            }
        }
    }
}

#[test]
fn new_auto_and_legacy_first_edits_have_distinct_documented_grid_policies() {
    for policy in 0..3 {
        let legacy = policy != 0;
        let mut engine = Engine::default();
        engine.execute("plot([0 1],[2 3]);a=gca;grid on;").unwrap();
        if policy == 1 {
            engine.figure.as_mut().unwrap().appearance = None;
        }
        if policy == 2 {
            let f = engine.figure.as_mut().unwrap();
            f.set_figure_theme(FigureTheme::Light);
            f.set_canvas_mode(ColorMode::Manual, BaseTheme::Light);
        }
        let untouched = engine.figure.clone().unwrap();
        engine
            .execute("alpha=get(a,'GridAlpha');mode=get(a,'GridAlphaMode');")
            .unwrap();
        let crate::Array::Numeric(alpha) = engine.array("alpha").unwrap() else {
            panic!("numeric alpha")
        };
        assert_eq!(alpha.data(), &[if legacy { 1. } else { 0.15 }]);
        let crate::Array::Character(mode) = engine.array("mode").unwrap() else {
            panic!("character mode")
        };
        assert_eq!(
            mode.to_utf8(&AtomicBool::new(false)).unwrap(),
            if legacy { "manual" } else { "auto" }
        );
        let f = engine.figure.as_ref().unwrap();
        assert_eq!(
            f.axes
                .resolve_colors(f.resolve_appearance(BaseTheme::Light))
                .grid_alpha,
            alpha.data()[0]
        );
        assert!(f.colors.is_none());
        assert!(untouched.same_snapshot(f));
        let before = engine.figure.clone().unwrap();
        let old_grid = before.resolve_appearance(BaseTheme::Light).grid;
        engine.execute("set(a,'XColor',[1 0 0]);").unwrap();
        let f = engine.figure.as_ref().unwrap();
        let grid = f.colors.unwrap().grid.unwrap();
        assert_eq!(grid.x_mode, ColorMode::Manual);
        assert_eq!(
            grid.color_mode,
            if legacy {
                ColorMode::Manual
            } else {
                ColorMode::Auto
            }
        );
        let resolved = f
            .axes
            .resolve_colors(f.resolve_appearance(BaseTheme::Light));
        assert_eq!(resolved.grid_x, if legacy { old_grid } else { [255, 0, 0] });
        assert_eq!(resolved.grid_alpha, if legacy { 1. } else { 0.15 });
        assert!(before.series[0].points.same_storage(&f.series[0].points));
        engine
            .execute("set(a,'GridColor',[0 1 0],'GridAlpha',0.4,'YColor',[0 0 1]);")
            .unwrap();
        let f = engine.figure.as_ref().unwrap();
        let resolved = f
            .axes
            .resolve_colors(f.resolve_appearance(BaseTheme::Light));
        assert_eq!(resolved.grid_x, [0, 255, 0]);
        assert_eq!(resolved.grid_y, [0, 255, 0]);
        assert_eq!(resolved.grid_alpha, 0.4);
        engine
            .execute("set(a,'GridColorMode','auto','GridAlphaMode','auto');")
            .unwrap();
        let f = engine.figure.as_ref().unwrap();
        let resolved = f
            .axes
            .resolve_colors(f.resolve_appearance(BaseTheme::Light));
        assert_eq!(resolved.grid_x, [255, 0, 0]);
        assert_eq!(resolved.grid_y, [0, 0, 255]);
        assert_eq!(resolved.grid_alpha, 0.15);
        let before = f.clone();
        assert!(
            engine
                .execute("set(a,'GridAlpha',0.7,'XColorMode','invalid');")
                .is_err()
        );
        assert!(before.same_snapshot(engine.figure.as_ref().unwrap()));
    }
}

#[test]
fn fixed_rgb_updates_are_atomic_keep_identity_and_invalidate_metadata_without_copying_samples() {
    let mut engine = Engine::default();
    engine.execute("plot([-0 1],[2 3]);a=gca;").unwrap();
    let original = engine.figure.clone().unwrap();
    let stamp = SnapshotStamp::new(&original);
    engine.execute("set(a,'Color',[1 1 1],'XColor',[-0 0.15 1],'YColor',[0.2 0.4 0.6]);title(a,'','Color',[0.125 0.25 0.5]);xlabel(a,'Time','Interpreter','none','Color',[1 0 0]);ylabel(a,'Value','Color',[0 1 0]);").unwrap();
    let fixed = engine.figure.clone().unwrap();
    assert!(!stamp.matches(&fixed));
    assert!(fixed.identity.same_object(&original.identity));
    assert!(fixed.axes.identity.same_object(&original.axes.identity));
    assert!(
        fixed.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
    assert_eq!(
        fixed.colors.unwrap().x.unwrap()[0].to_bits(),
        (-0.0_f64).to_bits()
    );
    for bad in [
        "set(a,'Color',[0 0 0],'YColor',[2 0 0]);",
        "title(a,'changed','Color',[1 0 0],'Interpreter','tex');",
        "xlabel(a,'changed','Color','none');",
        "set(a,'XColor',[0 NaN 1]);",
        "set(a,'Color',true(1,4));",
        "set(a,'GridAlpha',true(1,4));",
    ] {
        assert!(engine.execute(bad).is_err());
        assert!(fixed.same_snapshot(engine.figure.as_ref().unwrap()));
    }
    engine.set_graphics_theme_context(BaseTheme::Dark);
    engine
        .execute("x=get(a,'XColor');background=get(a,'Color');")
        .unwrap();
    let crate::Array::Numeric(x) = engine.array("x").unwrap() else {
        panic!("numeric XColor");
    };
    let crate::Array::Numeric(background) = engine.array("background").unwrap() else {
        panic!("numeric Color");
    };
    assert_eq!(x.data()[0].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(background.data(), &[1., 1., 1.]);
    let resolved = fixed
        .axes
        .resolve_colors(fixed.resolve_appearance(BaseTheme::Dark));
    assert_eq!(resolved.background, [255; 3]);
    assert_eq!(resolved.title, [32, 64, 128]);
    assert!(fixed.same_snapshot(engine.figure.as_ref().unwrap()));
}

#[test]
fn native_colors_preserve_raw_presence_and_reject_downgraded_capability_before_samples() {
    let mut engine = Engine::default();
    engine
        .execute("plot([0 1],[2 3]);a=gca;set(a,'Color',[1 1 1]);title(a,'','Color',[-0 0.15 1]);")
        .unwrap();
    let mut source = engine.figure.unwrap();
    source.appearance.as_mut().unwrap().series_palette = None;
    for axes in source.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    let bytes = files::encode_native_figure(&source).unwrap();
    assert!(bytes.starts_with(b"MATRFIGT"));
    let decoded = files::decode_native_figure(&bytes).unwrap();
    assert!(source.same_metadata(&decoded));
    assert_eq!(files::encode_native_figure(&decoded).unwrap(), bytes);
    for old in b"1359BDFHJL NPR".iter().filter(|b| **b != b' ') {
        let mut bad = bytes.clone();
        bad[7] = *old;
        let n = u32::from_le_bytes(bad[8..12].try_into().unwrap()) as usize;
        bad.truncate(12 + n);
        let error = files::decode_native_figure(&bad).unwrap_err();
        assert!(
            error.contains("Axes RGB metadata requires MFIGT"),
            "{error}"
        );
    }
    let mut absent = source.clone();
    absent.colors = None;
    assert!(
        files::encode_native_figure(&absent)
            .unwrap()
            .starts_with(b"MATRFIGR")
    );
    assert!(!source.same_metadata(&absent));
}

#[test]
fn sixteen_axes_native_colors_have_one_figure_theme_and_reject_child_role_mismatch() {
    let mut engine = Engine::default();
    for i in 1..=16 {
        engine
            .execute(&format!(
                "subplot(4,4,{i});plot([0 1],[{i} {}]);a=gca;set(a,'XColor',[{} 0.25 0.5]);",
                i + 1,
                f64::from(i) / 16.
            ))
            .unwrap();
    }
    let mut source = engine.figure.unwrap();
    source.appearance.as_mut().unwrap().series_palette = None;
    for axes in source.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    let bytes = files::encode_native_figure(&source).unwrap();
    assert!(bytes.starts_with(b"MATRFIGU"));
    let decoded = files::decode_native_figure(&bytes).unwrap();
    assert!(source.same_metadata(&decoded));
    assert_eq!(files::encode_native_figure(&decoded).unwrap(), bytes);
    let top = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let first = 12 + top + 4;
    assert_eq!(&bytes[first..first + 8], b"MATRFIGT");
    let mut bad = bytes.clone();
    bad[7] = b'S';
    assert!(
        files::decode_native_figure(&bad)
            .unwrap_err()
            .contains("MFIGU")
    );
    let mut bad = bytes.clone();
    bad[first + 7] = b'R';
    assert!(
        files::decode_native_figure(&bad)
            .unwrap_err()
            .contains("MFIGT")
    );
    let mut bad = bytes;
    bad[first + 7] = b'U';
    assert!(files::decode_native_figure(&bad).is_err());
}

#[test]
fn rgb_scripts_replay_twice_through_inline_and_real_companion_documents() {
    let mut engine = Engine::default();
    engine.execute("plot([-0 1],[2 3],'Color',[0 0 1]);a=gca;set(a,'Color',[1 1 1],'XColor',[-0 0.15 1],'YColor',[0.25 0.5 0.75]);title(a,'','Color',[0.125 0.25 0.5]);xlabel(a,'Time','Color',[1 0 0]);ylabel(a,'Value','Color',[0 1 0]);").unwrap();
    let source = engine.figure.clone().unwrap();
    let directory = tempfile::tempdir().unwrap();
    for companion in [false, true] {
        let mut current = source.clone();
        for pass in 0..2 {
            let mut replay = Engine::default();
            if companion {
                let path = directory.path().join(format!("colors_{pass}.m"));
                files::write_matlab_plot_bundle_with_cancel(
                    &path,
                    &current,
                    &AtomicBool::new(false),
                )
                .unwrap();
                let document = crate::document::Document::open(&path).unwrap();
                replay
                    .execute_document(&document.text, document.path.as_deref().unwrap())
                    .unwrap();
            } else {
                replay
                    .execute(&files::matlab_plot_script(&current, &AtomicBool::new(false)).unwrap())
                    .unwrap();
            }
            current = replay.figure.unwrap();
            assert_eq!(current.colors, source.colors);
            assert_eq!(
                (&current.title, &current.xlabel, &current.ylabel),
                (&source.title, &source.xlabel, &source.ylabel)
            );
            assert_eq!(current.appearance, source.appearance);
            for (a, b) in current.series[0]
                .points
                .iter()
                .zip(source.series[0].points.iter())
            {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            }
        }
    }
    assert!(source.same_snapshot(engine.figure.as_ref().unwrap()));
}

#[test]
fn session_store_keeps_fixed_rgb_and_rebinds_real_axes_aliases() {
    use crate::session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, restore_figures},
        workspace,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("axes_colors.json");
    let mut source = Engine::default();
    source.execute("plot([-0 1],[2 3]);a=gca;alias=a;set(a,'Color',[1 1 1],'XColor',[-0 0.15 1]);title(a,'','Color',[0.125 0.25 0.5]);").unwrap();
    let original = source.figure.clone().unwrap();
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
            figures: vec![FigureDocument {
                window: None,
                id: source.current_figure,
                figure: original.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            }],
        }),
        layout: Default::default(),
        version: 1,
        documents: vec![crate::document::Document::new()],
        active: 0,
        folder: directory.path().to_path_buf(),
        recent: vec![],
        history: vec![],
        model: crate::simulation::Model::default(),
        model_path: None,
        model_baseline: None,
    };
    SessionStore::new(&path).save(state).unwrap();
    assert!(original.same_snapshot(source.figure.as_ref().unwrap()));
    let saved = Session::load(&path).unwrap();
    let stop = AtomicBool::new(false);
    let report = restore_figures(&path, saved.figure_manifest.as_ref().unwrap(), &stop);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let bindings = report.workspace_bindings().clone();
    let mut restored = Engine::default();
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
    let restored_workspace = workspace::restore_with_figures(
        &path,
        saved.workspace_manifest.as_ref().unwrap(),
        &bindings,
        &stop,
    )
    .unwrap();
    assert!(restored_workspace.warnings.is_empty());
    restored.merge_workspace(restored_workspace.values).unwrap();
    let before = restored.figure.clone().unwrap();
    assert!(original.same_metadata(&before));
    assert_eq!(
        files::encode_native_figure(&original).unwrap(),
        files::encode_native_figure(&before).unwrap()
    );
    assert!(!original.axes.identity.same_object(&before.axes.identity));
    let crate::Array::Graphics(a) = restored.array("a").unwrap() else {
        panic!("axes handle");
    };
    let crate::Array::Graphics(alias) = restored.array("alias").unwrap() else {
        panic!("axes alias");
    };
    assert!(a.same_storage(&alias));
    restored.set_graphics_theme_context(BaseTheme::Dark);
    restored
        .execute("x=get(alias,'XColor');set(a,'YColor',[0 1 0]);")
        .unwrap();
    assert_eq!(
        restored.variables["x"].data()[0].to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(&restored.variables["x"].data()[1..], &[0.15, 1.]);
    let after = restored.figure.as_ref().unwrap();
    assert_eq!(after.colors.unwrap().y, Some([0., 1., 0.]));
    assert!(before.axes.identity.same_object(&after.axes.identity));
    assert!(
        before.series[0]
            .points
            .same_storage(&after.series[0].points)
    );
}

#[test]
fn ruler_rgb_falls_back_only_for_its_label_independently_of_grid_mode() {
    for direction in 0..2 {
        for mode in [ColorMode::Auto, ColorMode::Manual] {
            for host in [BaseTheme::Light, BaseTheme::Dark] {
                let mut figure = Figure::default();
                figure
                    .add(vec![Series::new(vec![[-0., 1.], [2., 3.]], PlotKind::Line)])
                    .unwrap();
                let mut colors = AxesColors::default();
                let supplied = [-0., 0.25, 0.75];
                if direction == 0 {
                    colors.x = Some(supplied);
                } else {
                    colors.y = Some(supplied);
                }
                let mut grid =
                    AxesGridStyle::from_appearance(figure.resolve_appearance(host), true);
                if direction == 0 {
                    grid.x_mode = mode;
                } else {
                    grid.y_mode = mode;
                }
                colors.grid = Some(grid);
                figure.colors = Some(colors);
                let original = figure.clone();
                let appearance = figure.resolve_appearance(host);
                let resolved = figure.resolve_colors(appearance);
                assert_eq!([resolved.xlabel, resolved.ylabel][direction], [0, 64, 191]);
                assert_eq!(
                    [resolved.xlabel, resolved.ylabel][1 - direction],
                    appearance.text
                );
                assert_eq!(figure.colors, original.colors);
                assert_eq!(
                    figure
                        .colors
                        .unwrap()
                        .x
                        .or(figure.colors.unwrap().y)
                        .unwrap()[0]
                        .to_bits(),
                    (-0.0_f64).to_bits()
                );
                let reopened =
                    files::decode_native_figure(&files::encode_native_figure(&figure).unwrap())
                        .unwrap();
                assert_eq!(reopened.colors, figure.colors);
                assert_eq!(
                    [
                        reopened.resolve_colors(appearance).xlabel,
                        reopened.resolve_colors(appearance).ylabel
                    ][direction],
                    [0, 64, 191]
                );
                if direction == 0 {
                    figure.colors.as_mut().unwrap().xlabel = Some([0., 1., 0.]);
                } else {
                    figure.colors.as_mut().unwrap().ylabel = Some([0., 1., 0.]);
                }
                let resolved = figure.resolve_colors(appearance);
                assert_eq!([resolved.xlabel, resolved.ylabel][direction], [0, 255, 0]);
                assert_eq!([resolved.x, resolved.y][direction], [0, 64, 191]);
                assert!(
                    figure.series[0]
                        .points
                        .same_storage(&original.series[0].points)
                );
            }
        }
    }
}
