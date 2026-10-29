use matrust::{
    Engine, files,
    plot::{Axes, Figure, PlotKind, Series},
};
use std::sync::atomic::AtomicBool;
fn export(figure: &Figure) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reconstruction.m");
    files::write_matlab_plot_script_with_cancel(&path, figure, &AtomicBool::new(false)).unwrap();
    std::fs::read_to_string(path).unwrap()
}
#[test]
fn matlab_plot_script_preserves_samples_bits_and_quotes_untrusted_labels() {
    let mut figure = Figure::default();
    let points = vec![
        [f64::MIN_POSITIVE, -0.],
        [f64::from_bits(1), f64::MAX],
        [3., f64::NAN],
        [4., f64::INFINITY],
        [5., f64::NEG_INFINITY],
    ];
    let mut series = Series::new(points.clone(), PlotKind::Line);
    series.label = "O'Brien');delete('important');%".into();
    series.visible = false;
    figure.add(vec![series]).unwrap();
    figure.title = "title\n';error('injected');\u{03bb}\u{1f680}".into();
    let script = export(&figure);
    let data = script
        .split("mr_data = [\n")
        .nth(1)
        .unwrap()
        .split("];\n")
        .next()
        .unwrap();
    let decoded: Vec<_> = data
        .split_whitespace()
        .map(|s| s.trim_end_matches(';').parse::<f64>().unwrap())
        .collect();
    for (a, b) in decoded.iter().zip(points.iter().flatten()) {
        if b.is_nan() {
            assert!(a.is_nan());
        } else {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
    assert!(script.contains("'O''Brien'');delete(''important'');%'"));
    assert!(!script.contains("title\n';error"));
    assert!(script.contains("char([116 105 116 108 101 10"));
    assert!(script.contains("'Visible','off'"));
}
#[test]
fn matlab_plot_script_reconstructs_types_layout_labels_legends_and_hold() {
    let mut engine = Engine::default();
    engine.execute("subplot(2,1,1);plot([1 2 3],[3 2 1],'r--o');hold on;scatter([1 2],[2 3]);stairs([1 2],[3 4]);stem([1 2],[2 4]);legend('Line','Scatter','Steps','Stems');title('Top');grid on;subplot(2,1,2);bar([1 3],[2 4]);title('Bottom');").unwrap();
    let figure = engine.figure.as_ref().unwrap();
    let original = figure.clone();
    let script = export(figure);
    for command in [
        "plot(mr_ax",
        "scatter(mr_ax",
        "stairs(mr_ax",
        "stem(mr_ax",
        "bar(mr_ax",
        "legend(mr_ax",
        "'XGrid','on'",
        "'LineStyle','--'",
        "'CurrentAxes',mr_ax1",
        "'NextPlot','replace'",
        "'NextPlot','add'",
    ] {
        assert!(script.contains(command), "missing {command}");
    }
    assert_eq!(script.matches("subplot(").count(), 2);
    assert_eq!(script.matches(" = gca;").count(), 2);
    assert!(!script.contains(" = axes("));
    assert_eq!(script.matches("mr_data = [\n").count(), 4);
    assert!(original.same_snapshot(figure));
    assert!(script.contains("'GroupWidth',0.8,'BarWidth',1"));
    assert!(script.contains("'EdgeColor','none'"));
}
#[test]
fn matlab_plot_script_handles_empty_axes_shared_titles_and_signed_log_baselines() {
    let empty = export(&Figure::default());
    assert!(empty.contains("mr_series = cell(1,0);"));
    let mut engine = Engine::default();
    engine.execute("tiledlayout(2,1);nexttile;semilogy([1 2],[-1 -10]);hold on;stem([1 2],[-2 -5]);nexttile;plot([1 2],[3 4]);sgtitle('Shared');").unwrap();
    let script = export(engine.figure.as_ref().unwrap());
    assert!(script.contains("sgtitle(mr_figure,'Shared','Interpreter','none');"));
    assert!(!script.contains("annotation(mr_figure,'textbox'"));
    assert!(script.contains("'BaseValue',-1.00000000000000000e0"));
    assert!(script.contains("'YScale','log'"));
    let mut figure = Figure::default();
    figure
        .add(vec![Series::new(vec![], PlotKind::Line)])
        .unwrap();
    assert!(export(&figure).contains("mr_data = reshape(mr_data,[],2);"));
}
#[test]
fn matlab_plot_script_cancellation_and_invalid_figures_preserve_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.m");
    std::fs::write(&path, b"previous").unwrap();
    let mut f = Figure::default();
    assert!(
        files::write_matlab_plot_script_with_cancel(&path, &f, &AtomicBool::new(true))
            .unwrap_err()
            .contains("Cancelled")
    );
    f.title = "x".repeat(100_000);
    assert!(
        files::write_matlab_plot_script_with_cancel(&path, &f, &AtomicBool::new(false)).is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), b"previous");
}

#[test]
fn matlab_plot_script_rejects_invalid_and_shadowing_file_names_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let f = Figure::default();
    for name in [
        "figure-1.m",
        "1bad.m",
        "if.m",
        "plot.m",
        "Figure.m",
        "sgtitle.m",
        "file.txt",
        "a b.m",
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, "previous").unwrap();
        assert!(
            files::write_matlab_plot_script_with_cancel(&path, &f, &AtomicBool::new(false))
                .is_err(),
            "{name}"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "previous");
    }
    let format = matrust::session::ExportFormat::MatlabScript;
    assert_eq!(format.file_name(4), "figure_4.m");
    assert!(!format.raster() && !format.vector());
    let settings = matrust::session::ExportSettings {
        format,
        ..Default::default()
    };
    let restored: matrust::session::ExportSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(restored, settings);
}

// Genuine disk bundle tests. These use the ordinary parser and graphics calls;
// no generated-source recognition, alternate interpreter, or resource bypass.
fn bundle_text(engine: &Engine, name: &str) -> String {
    let matrust::Array::Character(value) = engine.array(name).unwrap() else {
        panic!()
    };
    value.to_utf8(&AtomicBool::new(false)).unwrap()
}

#[test]
fn compact_bundle_large_samples_relocate_and_replay_through_document() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    let relocated = directory.path().join("moved \u{03bb} ' folder");
    std::fs::create_dir(&original).unwrap();
    let cancel = AtomicBool::new(false);
    let mut source = Figure {
        axes: Axes {
            title: "Title ' \u{03bb}\nsecond line".into(),
            ..Axes::default()
        },
        ..Figure::default()
    };
    for kind in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::Stairs,
        PlotKind::Stem,
    ] {
        let points = (0..30_000)
            .map(|i| [i as f64 + 1., (i % 19) as f64 - 7.])
            .collect();
        let mut series = Series::new(points, kind);
        series.label = format!("large {kind:?} ' \u{03bb}");
        source.add(vec![series]).unwrap();
    }
    let x = matrust::Matrix::from_column_vec(3, 1, vec![1., 2., 1.]).unwrap();
    let y = matrust::Matrix::from_column_vec(3, 1, vec![1., 1., 2.]).unwrap();
    source
        .add(vec![
            Series::patch_from_matrices(&x, &y, [50, 90, 120], &cancel).unwrap(),
        ])
        .unwrap();
    let before = source.clone();
    let inline = files::matlab_plot_script(&source, &cancel).unwrap();
    assert!(inline.len() > 1_000_000);
    let bundle = files::write_matlab_plot_bundle_with_cancel(
        &original.join("reconstruction.m"),
        &source,
        &cancel,
    )
    .unwrap();
    assert!(bundle.data.is_file());
    let catalog = files::inspect_workspace(&bundle.data, &cancel).unwrap();
    assert_eq!(
        catalog
            .variables()
            .filter(|v| v.name.starts_with("mr_samples_"))
            .count(),
        5
    );
    std::fs::rename(&original, &relocated).unwrap();
    let path = relocated.join("reconstruction.m");
    let document = matrust::document::Document::open(&path).unwrap();
    assert!(document.text.len() < 100_000);
    assert!(document.text.contains("mfilename('fullpath')"));
    let mut engine = Engine::default();
    // Deliberately point ordinary file operations elsewhere; script identity
    // resolves the moved bundle, without process-global chdir.
    engine.set_file_directory(directory.path()).unwrap();
    let cwd = std::env::current_dir().unwrap();
    engine.execute_document(&document.text, &path).unwrap();
    assert_eq!(std::env::current_dir().unwrap(), cwd);
    let actual = engine.figure.as_ref().unwrap();
    assert_eq!(actual.series.len(), source.series.len());
    assert_eq!(actual.title, source.title);
    for (expected, actual) in source.series.iter().zip(&actual.series) {
        assert_eq!(expected.kind, actual.kind);
        assert_eq!(expected.points.len(), actual.points.len());
        for (a, b) in expected
            .points
            .iter()
            .flatten()
            .zip(actual.points.iter().flatten())
        {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        assert_eq!(expected.label, actual.label);
        assert_eq!(expected.color, actual.color);
        assert_eq!(expected.visible, actual.visible);
    }
    assert!(source.same_snapshot(&before));
    for (a, b) in source.series.iter().zip(&before.series) {
        assert!((a.points).same_storage(&b.points));
    }
}

#[test]
fn bundle_publication_failures_preserve_previous_script_and_generation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reconstruction.m");
    let mut source = Figure::default();
    source
        .add(vec![Series::new(vec![[1., 2.]], PlotKind::Line)])
        .unwrap();
    let bundle =
        files::write_matlab_plot_bundle_with_cancel(&path, &source, &AtomicBool::new(false))
            .unwrap();
    let script = std::fs::read(&path).unwrap();
    let data = std::fs::read(&bundle.data).unwrap();
    assert!(
        files::write_matlab_plot_bundle_with_cancel(&path, &source, &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), script);
    assert_eq!(std::fs::read(&bundle.data).unwrap(), data);
    // Failure at final publication, after the new MAT generation was written.
    let blocked = directory.path().join("blocked.m");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("keep"), b"untouched").unwrap();
    let before = std::fs::read_dir(directory.path()).unwrap().count();
    assert!(
        files::write_matlab_plot_bundle_with_cancel(&blocked, &source, &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), before);
    assert_eq!(std::fs::read(blocked.join("keep")).unwrap(), b"untouched");
    assert_eq!(std::fs::read(&path).unwrap(), script);
    assert_eq!(std::fs::read(&bundle.data).unwrap(), data);
    std::fs::remove_file(&bundle.data).unwrap();
    let mut engine = Engine::default();
    engine.execute("plot([1 2],[3 4]);").unwrap();
    let old = engine.figure.clone().unwrap();
    assert!(
        engine
            .execute_document(std::str::from_utf8(&script).unwrap(), &path)
            .is_err()
    );
    assert!(engine.figure.as_ref().unwrap().same_snapshot(&old));
}

#[test]
fn named_document_paths_restore_after_error_and_respect_function_scope() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("named.m");
    let mut engine = Engine::default();
    engine
        .execute_document(
            "name=mfilename(); folder=fileparts(mfilename('fullpath'));",
            &path,
        )
        .unwrap();
    assert_eq!(bundle_text(&engine, "name"), "named");
    assert!(std::path::Path::new(&bundle_text(&engine, "folder")).ends_with(directory.path()));
    assert!(engine.execute_document("unknown_call();", &path).is_err());
    engine.execute("name=mfilename('fullpath');").unwrap();
    assert_eq!(bundle_text(&engine, "name"), "");
    std::fs::write(
        directory.path().join("path_helper.m"),
        "function value=path_helper()\nvalue=mfilename();\nend\n",
    )
    .unwrap();
    engine.set_function_directory(directory.path()).unwrap();
    engine
        .execute_document("name=path_helper(); outer=mfilename();", &path)
        .unwrap();
    assert_eq!(bundle_text(&engine, "name"), "path_helper");
    assert_eq!(bundle_text(&engine, "outer"), "named");
    engine.execute("folder=fileparts('data/subdir/'); joined=fullfile('data//./subdir','../leaf/.'); trailing=fullfile('data','subdir/');").unwrap();
    assert_eq!(bundle_text(&engine, "folder"), "data/subdir");
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(
        bundle_text(&engine, "joined"),
        format!("data{sep}subdir{sep}..{sep}leaf{sep}.")
    );
    assert_eq!(
        bundle_text(&engine, "trailing"),
        format!("data{sep}subdir{sep}")
    );
    engine
        .execute(
            "joined=fullfile('a','/b'); unc=fullfile('//server/share','folder','..','tail/.');",
        )
        .unwrap();
    assert_eq!(bundle_text(&engine, "joined"), format!("a{sep}b"));
    assert_eq!(
        bundle_text(&engine, "unc"),
        format!("{sep}{sep}server{sep}share{sep}folder{sep}..{sep}tail{sep}.")
    );
    #[cfg(windows)]
    {
        engine.execute(r"root=fileparts('C:\'); folder=fileparts('C:\data\'); joined=fullfile('C:\data\\.\subdir','leaf');").unwrap();
        assert_eq!(bundle_text(&engine, "root"), r"C:\");
        assert_eq!(bundle_text(&engine, "folder"), r"C:\data");
        assert_eq!(bundle_text(&engine, "joined"), r"C:\data\subdir\leaf");
        engine
            .execute(r"joined=fullfile('c:\folder1','\\\folder2\\');")
            .unwrap();
        assert_eq!(bundle_text(&engine, "joined"), r"c:\folder1\folder2\");
        assert!(engine.execute(r"joined=fullfile('a','C:\b');").is_err());
        engine
            .execute(
                "drive=fullfile('C://folder','sub'); repeated=fullfile('C:////folder','leaf');",
            )
            .unwrap();
        assert_eq!(bundle_text(&engine, "drive"), r"C:\folder\sub");
        assert_eq!(bundle_text(&engine, "repeated"), r"C:\folder\leaf");
        assert!(
            engine
                .execute("bad=fullfile('https://example.com','data');")
                .is_err()
        );
        assert!(
            engine
                .execute(r"joined=fullfile('a','\\?\UNC\server\share');")
                .is_err()
        );
    }
}

#[test]
fn bundle_replays_bar_and_general_patch_in_separate_bounded_geometry_banks() {
    let directory = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut source = Figure::default();
    source
        .add(vec![Series::new(
            (0..10_000).map(|i| [i as f64 + 1., 2.]).collect(),
            PlotKind::Bar,
        )])
        .unwrap();
    // Arbitrary imported geometry deliberately retains the compact Patch route.
    source.series[0].bar_offset = 0.125;
    // Historical fill-only arbitrary geometry retains the compact Patch route.
    source.series[0].bar_baseline_visible = false;
    let x = matrust::Matrix::from_column_vec(3, 1, vec![1., 2., 1.]).unwrap();
    let y = matrust::Matrix::from_column_vec(3, 1, vec![1., 1., 2.]).unwrap();
    source
        .add(vec![
            Series::patch_from_matrices(&x, &y, [1, 2, 3], &cancel).unwrap(),
        ])
        .unwrap();
    source.validate().unwrap();
    let path = directory.path().join("capacity_case.m");
    files::write_matlab_plot_bundle_with_cancel(&path, &source, &cancel).unwrap();
    let document = matrust::document::Document::open(&path).unwrap();
    let mut engine = Engine::default();
    engine.execute_document(&document.text, &path).unwrap();
    let actual = engine.figure.as_ref().unwrap();
    assert_eq!(actual.series.len(), 2);
    assert_eq!(actual.series[0].patch_face_count(), 10_000);
    assert!(actual.series[0].points.patch().is_some());
    assert_eq!(actual.series[1].points, source.series[1].points);
    let ledger = actual.geometry_ledger().unwrap().cost();
    assert_eq!(ledger.compact_rectangles(), 10_000);
    assert_eq!(ledger.general_faces(), 1);
    assert_eq!(source.series.len(), 2);
}

fn adaptive_large_figure() -> Figure {
    let mut figure = Figure::default();
    figure
        .add(vec![Series::new(
            (0..30_000).map(|i| [i as f64, (i as f64).sin()]).collect(),
            PlotKind::Line,
        )])
        .unwrap();
    figure.title = "Adaptive disk reconstruction".into();
    figure
}

#[test]
fn adaptive_script_keeps_small_inline_bytes_and_large_bundle_disk_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adaptive_plot.m");
    let cancel = AtomicBool::new(false);
    let mut small = Figure::default();
    small
        .add(vec![Series::new(vec![[1., 2.], [2., 4.]], PlotKind::Line)])
        .unwrap();
    let expected = files::matlab_plot_script(&small, &cancel).unwrap();
    assert!(
        files::write_matlab_plot_reconstruction_with_cancel(&path, &small, &cancel)
            .unwrap()
            .is_none()
    );
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    let large = adaptive_large_figure();
    let alias = large.clone();
    assert!(files::matlab_plot_script(&large, &cancel).unwrap().len() > 1_000_000);
    let saved = files::write_matlab_plot_reconstruction_with_cancel(&path, &large, &cancel)
        .unwrap()
        .unwrap();
    let doc = matrust::document::Document::open(&path).unwrap();
    assert!(doc.text.len() < 1_000_000);
    assert!(saved.data.is_file());
    let mut engine = Engine::default();
    engine
        .execute_document(&doc.text, doc.path.as_ref().unwrap())
        .unwrap();
    let actual = engine.figure.unwrap();
    assert_eq!(actual.title, large.title);
    assert_eq!(actual.series[0].points.len(), 30_000);
    assert_eq!(
        actual.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        large.series[0].points.iter().collect::<Vec<_>>().as_slice()
    );
    assert!(large.same_snapshot(&alias));
    assert!((large.series[0].points).same_storage(&alias.series[0].points));
}

#[test]
fn prepared_reconstruction_keeps_new_generation_owned_until_atomic_publication() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reviewed.m");
    let cancel = AtomicBool::new(false);
    let figure = adaptive_large_figure();
    let old = files::write_matlab_plot_reconstruction_with_cancel(&path, &figure, &cancel)
        .unwrap()
        .unwrap();
    let old_script = std::fs::read(&path).unwrap();
    let old_data = std::fs::read(&old.data).unwrap();
    let entries = || {
        std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let before = entries();
    let prepared = files::prepare_matlab_plot_reconstruction(&path, &figure, &cancel).unwrap();
    assert!(entries().len() > before.len());
    let mut checked = false;
    assert_eq!(
        prepared
            .publish(true, &cancel, || {
                checked = true;
                cancel.store(true, Ordering::Relaxed);
                Ok(())
            })
            .err()
            .unwrap(),
        "Calculation cancelled"
    );
    assert!(checked);
    assert_eq!(entries(), before);
    assert_eq!(std::fs::read(&path).unwrap(), old_script);
    assert_eq!(std::fs::read(&old.data).unwrap(), old_data);
    cancel.store(false, Ordering::Relaxed);
    let prepared = files::prepare_matlab_plot_reconstruction(&path, &figure, &cancel).unwrap();
    cancel.store(true, Ordering::Relaxed);
    let mut touched = false;
    assert!(
        prepared
            .publish(true, &cancel, || {
                touched = true;
                Ok(())
            })
            .is_err()
    );
    assert!(
        !touched,
        "Pre-cancelled publication must not invoke the destination callback"
    );
    assert_eq!(entries(), before);
    cancel.store(false, Ordering::Relaxed);
    let prepared = files::prepare_matlab_plot_reconstruction(&path, &figure, &cancel).unwrap();
    assert_eq!(
        prepared
            .publish(true, &cancel, || Err(
                "Destination changed after review".into()
            ))
            .err()
            .unwrap(),
        "Destination changed after review"
    );
    assert_eq!(entries(), before);
    let absent = dir.path().join("created_elsewhere.m");
    let prepared = files::prepare_matlab_plot_reconstruction(&absent, &figure, &cancel).unwrap();
    std::fs::write(&absent, b"External writer").unwrap();
    assert!(prepared.publish(false, &cancel, || Ok(())).is_err());
    assert_eq!(std::fs::read(&absent).unwrap(), b"External writer");
    assert_eq!(entries().len(), before.len() + 1);
    assert_eq!(std::fs::read(&old.data).unwrap(), old_data);
}

#[test]
fn compact_reconstruction_admits_maximum_series_axes_and_textbox_metadata_without_raising_parser_bounds()
 {
    let cancel = AtomicBool::new(false);
    let mut engine = Engine::default();
    let label = "\u{03bb}".repeat(2048); // exact 4096 UTF-8 bytes per model label
    for panel in 1..=16 {
        engine.execute(&format!("subplot(4,4,{panel});plot([1 2],[3 4]);hold on;plot([1 2],[4 5]);plot([1 2],[5 6]);plot([1 2],[6 7]);")).unwrap();
        let figure = engine.figure.as_mut().unwrap();
        figure.title = label.clone();
        figure.xlabel = label.clone();
        figure.ylabel = label.clone();
        figure.legend = true;
        for series in &mut figure.series {
            series.label = label.clone();
        }
    }
    let figure = engine.figure.as_mut().unwrap();
    figure.name = label.clone();
    figure.layout.as_mut().unwrap().title = label;
    for i in 0..64 {
        let text = format!("{i:02}{}", "x".repeat(254));
        figure
            .add_textbox(matrust::plot::TextBox::new([0., 0., 0.1, 0.1], text).unwrap())
            .unwrap();
    }
    figure.validate().unwrap();
    assert_eq!(figure.series_count(), 64);
    assert_eq!(figure.all_axes().count(), 16);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata_max.m");
    let saved = files::write_matlab_plot_reconstruction_with_cancel(&path, figure, &cancel)
        .unwrap()
        .unwrap();
    assert!(saved.data.exists());
    let document = matrust::document::Document::open(&path).unwrap();
    assert!(document.text.len() < 1_000_000);
    let mut replay = Engine::default();
    replay
        .execute_document(&document.text, document.path.as_ref().unwrap())
        .unwrap();
    let actual = replay.figure.unwrap();
    assert_eq!(actual.series_count(), 64);
    assert_eq!(actual.all_axes().count(), 16);
    assert_eq!(actual.textboxes, figure.textboxes);
    assert_eq!(actual.name, figure.name);
    for ((_, expected), (_, actual)) in figure.all_axes().zip(actual.all_axes()) {
        assert_eq!(actual.title, expected.title);
        assert_eq!(actual.xlabel, expected.xlabel);
        assert_eq!(actual.ylabel, expected.ylabel);
        assert_eq!(actual.series.len(), expected.series.len());
        for (expected, actual) in expected.series.iter().zip(&actual.series) {
            assert_eq!(actual.label, expected.label);
            assert_eq!(actual.points, expected.points);
        }
    }
}

#[test]
fn script_shared_title_replays_with_full_annotation_count_and_text_budgets() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for (count, text) in [(64, "x".to_owned()), (4, "x".repeat(4096))] {
        let mut source = Engine::default();
        source.execute("subplot(1,1,1);plot([1 2],[3 4]);").unwrap();
        let figure = source.figure.as_mut().unwrap();
        figure.layout.as_mut().unwrap().title = "Shared title".into();
        for _ in 0..count {
            figure
                .add_textbox(matrust::plot::TextBox::new([0., 0., 0.1, 0.1], text.clone()).unwrap())
                .unwrap();
        }
        figure.validate().unwrap();
        let path = dir.path().join(format!("shared_{count}.m"));
        files::write_matlab_plot_reconstruction_with_cancel(&path, figure, &cancel).unwrap();
        let document = matrust::document::Document::open(&path).unwrap();
        let mut replay = Engine::default();
        replay
            .execute_document(&document.text, document.path.as_ref().unwrap())
            .unwrap();
        let actual = replay.figure.as_ref().unwrap();
        assert_eq!(actual.textboxes, figure.textboxes);
        assert_eq!(actual.textboxes.len(), count);
        assert_eq!(actual.layout.as_ref().unwrap().title, "Shared title");
        assert_eq!(actual.series[0].points, figure.series[0].points);
    }
}

#[test]
fn path_preflight_rejects_shared_large_aliases_before_utf16_projection() {
    use matrust::{Array, CellArray, CharacterRow};
    use std::{collections::BTreeMap, sync::Arc};
    let cancel = AtomicBool::new(false);
    // Invalid UTF-16 makes any accidental projection observable as the wrong
    // error. Sixty-four cell slots share this single immutable allocation.
    let value = Array::Character(CharacterRow::from_units(vec![0xD800; 32769], &cancel).unwrap());
    let shared = Arc::new(value.clone());
    let cells = Array::Cell(CellArray::from_shared(1, 64, vec![shared; 64], &cancel).unwrap());
    let mut engine = Engine::default();
    engine
        .merge_workspace(BTreeMap::from([
            ("A".into(), value.clone()),
            ("C".into(), cells.clone()),
        ]))
        .unwrap();
    engine.execute("h=figure;").unwrap();
    let figure = engine.figure.clone().unwrap();
    for (source, expected) in [
        ("out=fullfile(C{:});", "Path is oversized"),
        ("out=fullfile(h,C{:});", "1 to 64"),
        ("out=fullfile(h,A);", "Expected character text"),
        ("out=fileparts(A,A);", "requires one character path"),
        ("out=mfilename(A,A);", "supports no argument"),
    ] {
        let error = engine.execute(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
        assert!(!error.contains("surrogate"), "{source}: {error}");
        assert!(engine.array("out").is_none());
        assert!(engine.array("A").unwrap().same_snapshot(&value));
        assert!(engine.array("C").unwrap().same_snapshot(&cells));
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&figure));
    }
    // Each argument passes the individual unit bound, but their raw join
    // cannot fit. Neither invalid alias is decoded before aggregate rejection.
    let half = Array::Character(CharacterRow::from_units(vec![0xD800; 16385], &cancel).unwrap());
    engine
        .merge_workspace(BTreeMap::from([("B".into(), half.clone())]))
        .unwrap();
    let error = engine.execute("out=fullfile(B,B);").unwrap_err();
    assert!(
        error.contains("fullfile path exceeds 32768 bytes"),
        "{error}"
    );
    assert!(engine.array("B").unwrap().same_snapshot(&half));
}

#[test]
fn path_limits_count_decoded_bytes_without_flattening_or_changing_aliases() {
    use matrust::{Array, CharacterRow};
    use std::collections::BTreeMap;
    let cancel = AtomicBool::new(false);
    let mut engine = Engine::default();
    for (unit, count, accepted) in [
        (0x61, 32768, true),
        (0x61, 32769, false),
        (0x03BB, 16384, true),
        (0x03BB, 16385, false),
    ] {
        let value = Array::Character(CharacterRow::from_units(vec![unit; count], &cancel).unwrap());
        engine
            .merge_workspace(BTreeMap::from([("A".into(), value.clone())]))
            .unwrap();
        let result = engine.execute("out=fullfile(A);");
        if accepted {
            result.unwrap();
            let Array::Character(output) = engine.array("out").unwrap() else {
                panic!("char output");
            };
            assert_eq!(output.units(), vec![unit; count]);
        } else {
            assert!(result.unwrap_err().contains("Path is oversized"));
        }
        assert!(engine.array("A").unwrap().same_snapshot(&value));
    }
    for (rows, cols, units, expected) in [
        (1, 1, vec![0xD800], "unpaired UTF-16 surrogate"),
        (1, 2, vec![0x61, 0], "contains NUL"),
        (2, 1, vec![0x61, 0x62], "matrices are not flattened"),
    ] {
        let value =
            Array::Character(CharacterRow::from_column_major(rows, cols, units, &cancel).unwrap());
        engine
            .merge_workspace(BTreeMap::from([("A".into(), value.clone())]))
            .unwrap();
        let previous = engine.array("out").unwrap();
        let error = engine.execute("out=fullfile(A);").unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(engine.array("A").unwrap().same_snapshot(&value));
        assert!(engine.array("out").unwrap().same_snapshot(&previous));
    }
}

#[test]
fn path_preflight_preserves_local_and_file_function_shadowing_and_stop_retry() {
    use matrust::{Array, CharacterRow};
    use std::{collections::BTreeMap, sync::atomic::Ordering};
    let cancel = AtomicBool::new(false);
    let value = Array::Character(CharacterRow::from_units(vec![0xD800; 65536], &cancel).unwrap());
    let mut engine = Engine::default();
    engine
        .merge_workspace(BTreeMap::from([("A".into(), value.clone())]))
        .unwrap();
    engine
        .execute("n=fullfile(A);\nfunction out=fullfile(x)\nout=numel(x);\nend\n")
        .unwrap();
    assert_eq!(
        engine.array("n").unwrap().as_numeric().unwrap().get(0, 0),
        65536.
    );
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("fileparts.m"),
        "function out=fileparts(x)\nout=numel(x);\nend\n",
    )
    .unwrap();
    engine.set_function_directory(directory.path()).unwrap();
    engine.execute("n=fileparts(A);").unwrap();
    assert_eq!(
        engine.array("n").unwrap().as_numeric().unwrap().get(0, 0),
        65536.
    );
    let stop = engine.cancellation();
    stop.store(true, Ordering::Relaxed);
    let error = engine.execute("p=fullfile('a','b');").unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(engine.array("p").is_none());
    assert!(engine.array("A").unwrap().same_snapshot(&value));
    stop.store(false, Ordering::Relaxed);
    engine.execute("p=fullfile('a','b');").unwrap();
    assert_eq!(
        bundle_text(&engine, "p"),
        format!("a{}b", std::path::MAIN_SEPARATOR)
    );
}

#[path = "support/grouped_bar_replay_cases.rs"]
mod grouped_bar_replay_cases;

#[test]
fn reconstruction_retains_per_axes_auto_modes_and_later_plot_growth_twice() {
    let stop = AtomicBool::new(false);
    for domain in [0, 1, -1] {
        for manual_x in [false, true] {
            for manual_y in [false, true] {
                for multiple_axes in [false, true] {
                    let panel = |mx: bool, my: bool| {
                        let values = if domain == -1 {
                            "[-2 -3 -4]"
                        } else {
                            "[2 3 4]"
                        };
                        let scale = if domain == 0 { "linear" } else { "log" };
                        let ylimits = match domain {
                            -1 => "[-8 -1]",
                            1 => "[1 8]",
                            _ => "[-8 8]",
                        };
                        format!(
                            "plot([1 2 3],{values},'LineWidth',1.5);yscale('{scale}');xlim({});ylim({});",
                            if mx { "[0 4]" } else { "'auto'" },
                            if my { ylimits } else { "'auto'" }
                        )
                    };
                    let mut source = Engine::default();
                    let script = if multiple_axes {
                        format!(
                            "subplot(1,2,1);{}subplot(1,2,2);{}",
                            panel(!manual_x, !manual_y),
                            panel(manual_x, manual_y)
                        )
                    } else {
                        panel(manual_x, manual_y)
                    };
                    source.execute(&script).unwrap();
                    let source = source.figure.unwrap();
                    for companion in [false, true] {
                        let directory = tempfile::tempdir().unwrap();
                        let mut current = source.clone();
                        for pass in 0..2 {
                            let snapshot = current.clone();
                            let path = directory.path().join(format!("axis_modes_{pass}.m"));
                            if companion {
                                let bundle = files::write_matlab_plot_bundle_with_cancel(
                                    &path, &current, &stop,
                                )
                                .unwrap();
                                assert!(bundle.data.is_file());
                            } else {
                                assert!(
                                    files::write_matlab_plot_reconstruction_with_cancel(
                                        &path, &current, &stop
                                    )
                                    .unwrap()
                                    .is_none()
                                );
                            }
                            let document = matrust::document::Document::open(&path).unwrap();
                            let mut replay = Engine::default();
                            replay
                                .execute_document(&document.text, document.path.as_deref().unwrap())
                                .unwrap();
                            let actual = replay.figure.as_ref().unwrap();
                            for ((position, a), (other, b)) in
                                current.all_axes().zip(actual.all_axes())
                            {
                                assert_eq!(position, other);
                                assert_eq!(
                                    (a.xlim, a.ylim, a.xscale, a.yscale),
                                    (b.xlim, b.ylim, b.xscale, b.yscale)
                                );
                                assert_eq!(a.bounds(), b.bounds());
                            }
                            assert_eq!(current.svg(), actual.svg());
                            assert!(current.same_snapshot(&snapshot));
                            current = actual.clone();
                            let initial_x = current.xlim;
                            let initial_y = current.ylim;
                            let values = if domain == -1 { "[-20 -30]" } else { "[20 30]" };
                            replay
                                .execute(&format!(
                                    "hold on;plot([10 11],{values},'LineWidth',1.5);"
                                ))
                                .unwrap();
                            let grown = replay.figure.as_ref().unwrap();
                            assert_eq!(grown.xlim, initial_x);
                            assert_eq!(grown.ylim, initial_y);
                            let (x, y) = grown.bounds();
                            assert_eq!(x[1], if manual_x { 4. } else { 11. });
                            if !manual_y {
                                assert_eq!(
                                    if domain == -1 { y[0] } else { y[1] },
                                    if domain == -1 { -30. } else { 30. }
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn implicit_and_explicit_marker_styles_survive_both_script_routes_twice() {
    let commands = [
        "stem([1 2],[3 4]);",
        "stem([1 2],[3 4],'.');",
        "stem([1 2],[3 4],'Marker','o','MarkerSize',7,'LineWidth',1.25);",
        "scatter([1 2],[3 4]);",
        "scatter([1 2],[3 4],'MarkerEdgeColor',[.2 .4 .6],'MarkerFaceColor',[.2 .4 .6]);",
        "scatter([1 2],[3 4],49,'Marker','s','LineWidth',1.25);",
        "plot([1 2],[3 4],'-o');",
        "plot([1 2],[3 4],'.','LineStyle','-');",
        "plot([1 2],[3 4],'Marker','d','MarkerSize',8,'LineWidth',2);",
        "stairs([1 2],[3 4],'o');",
        "stairs([1 2],[3 4],'.');",
        "stairs([1 2],[3 4],'Marker','+','MarkerSize',5,'LineWidth',0.75);",
    ];
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    for (case, command) in commands.iter().enumerate() {
        let mut engine = Engine::default();
        engine.execute(command).unwrap();
        engine
            .execute("xlim([0 3]);ylim([0 5]);legend('marker style');")
            .unwrap();
        let source = engine.figure.unwrap();
        let retained = source.clone();
        for companion in [false, true] {
            let mut current = source.clone();
            for pass in 0..2 {
                let path = dir
                    .path()
                    .join(format!("marker_{case}_{companion}_{pass}.m"));
                if companion {
                    files::write_matlab_plot_bundle_with_cancel(&path, &current, &stop).unwrap();
                } else {
                    files::write_matlab_plot_script_with_cancel(&path, &current, &stop).unwrap();
                }
                let doc = matrust::document::Document::open(&path).unwrap();
                let mut replay = Engine::default();
                replay.execute_document(&doc.text, &path).unwrap();
                let actual = replay.figure.unwrap();
                assert_eq!(current.series.len(), actual.series.len());
                for (a, b) in current.series.iter().zip(&actual.series) {
                    assert_eq!(a.kind, b.kind, "{case}/{companion}/{pass}");
                    assert_eq!(
                        a.line_width.map(f64::to_bits),
                        b.line_width.map(f64::to_bits)
                    );
                    assert_eq!(a.marker_shape, b.marker_shape);
                    assert_eq!(
                        a.marker_size.map(f64::to_bits),
                        b.marker_size.map(f64::to_bits)
                    );
                    assert_eq!(a.marker_filled, b.marker_filled);
                    assert_eq!(a.marker_radius().to_bits(), b.marker_radius().to_bits());
                    for fallback in [1., 1.7, 2.] {
                        assert_eq!(
                            a.stroke_width(fallback).to_bits(),
                            b.stroke_width(fallback).to_bits()
                        );
                    }
                    assert_eq!(a.points.len(), b.points.len());
                    for (p, q) in a.points.iter().zip(b.points.iter()) {
                        assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
                    }
                }
                assert_eq!(current.svg(), actual.svg(), "{case}/{companion}/{pass}");
                current = actual;
            }
        }
        assert!(source.same_snapshot(&retained));
        for (a, b) in source.series.iter().zip(&retained.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
}
