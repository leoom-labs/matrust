#![cfg(feature = "plot-export")]
use matrust::{Engine, export, plot::Canvas};
use std::sync::atomic::{AtomicBool, Ordering};

fn engine(path: &std::path::Path) -> Engine {
    let mut engine = Engine::default();
    engine.set_file_directory(path).unwrap();
    engine
        .execute("plot([0 1 2],[0 2 0],'r-o');title('Color EPS');")
        .unwrap();
    engine
}

fn bounds(text: &str, name: &str) -> Vec<f64> {
    text.lines()
        .find_map(|line| line.strip_prefix(name))
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect()
}

#[test]
fn sized_color_eps_has_point_bounds_model_color_and_shared_immutable_data() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let mut figure = engine.figure.clone().unwrap();
    figure.set_canvas_color([17, 85, 153]);
    let original = figure.clone();
    let canvas = Canvas {
        width: 612.5,
        height: 792.25,
    };
    let path = dir.path().join("page.EPS");
    export::export_vector_with_cancel(&figure, &path, canvas, &AtomicBool::new(false)).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.starts_with("%!PS-Adobe-3.0 EPSF-3.0\n"));
    assert_eq!(bounds(&text, "%%BoundingBox:"), [0., 0., 613., 793.]);
    assert_eq!(
        bounds(&text, "%%HiResBoundingBox:"),
        [0., 0., 612.5, 792.25]
    );
    let first_color = text
        .lines()
        .find_map(|line| line.strip_suffix(" setrgbcolor"))
        .unwrap();
    let components: Vec<f64> = first_color
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(components.len(), 3);
    for (actual, byte) in components.iter().zip(figure.color) {
        assert!((actual - f64::from(byte) / 255.).abs() < 1e-10);
    }
    assert!(text.contains("%%LanguageLevel: 3\n") && text.ends_with("%%EOF\n"));
    assert!(figure.same_snapshot(&original));
    assert!((figure.series[0].points).same_storage(&original.series[0].points));
}

#[test]
fn explicit_epsc_respects_format_over_extension_and_preserves_file_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    engine.execute("savefig(gcf,'editable.mfig');").unwrap();
    assert_eq!(engine.take_figure_file_events().len(), 1);
    let original = engine.figure.clone().unwrap();
    engine
        .execute("saveas(gcf,'color','EPSC');saveas(gcf,'misleading.svg','epsc');")
        .unwrap();
    let first = std::fs::read(dir.path().join("color.eps")).unwrap();
    assert_eq!(
        first,
        std::fs::read(dir.path().join("misleading.svg")).unwrap()
    );
    let text = std::str::from_utf8(&first).unwrap();
    assert_eq!(bounds(text, "%%BoundingBox:"), [0., 0., 960., 560.]);
    assert!(engine.take_figure_file_events().is_empty());
    assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    assert!(
        (original.series[0].points).same_storage(&engine.figure.as_ref().unwrap().series[0].points)
    );
    // The Rust format API selects color EPS explicitly too, independently of suffix.
    let api = dir.path().join("api.bin");
    export::export_figure_as_with_cancel(
        &original,
        &api,
        "epsc",
        1440,
        840,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(first, std::fs::read(api).unwrap());
    // A later script error does not undo a completed external export.
    assert!(
        engine
            .execute("saveas(gcf,'completed.eps','epsc');unknown_function();")
            .is_err()
    );
    assert_eq!(
        first,
        std::fs::read(dir.path().join("completed.eps")).unwrap()
    );
}

#[test]
fn unsupported_eps_forms_tight_export_print_and_stale_handles_do_not_publish() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    let path = dir.path().join("kept.eps");
    std::fs::write(&path, b"sentinel").unwrap();
    let original = engine.figure.clone().unwrap();
    for script in [
        "saveas(gcf,'kept.eps')",
        "saveas(gcf,'kept.eps','eps')",
        "saveas(gcf,'kept.eps','eps2')",
        "saveas(gcf,'kept.eps','epsc2')",
        "saveas(gcf,'kept.eps','epsc','extra')",
        "saveas(999,'kept.eps','epsc')",
        "exportgraphics(gcf,'kept.eps','ContentType','vector')",
        "print(gcf,'kept.eps','-depsc')",
    ] {
        assert!(engine.execute(script).is_err(), "{script}");
        assert_eq!(std::fs::read(&path).unwrap(), b"sentinel", "{script}");
        assert!(engine.take_figure_file_events().is_empty());
        assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    }
    engine
        .execute("old=gcf;close(old);figure(1);plot([2 1]);")
        .unwrap();
    assert!(engine.execute("saveas(old,'kept.eps','epsc')").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"sentinel");
}

#[test]
fn eps_failures_and_cancel_preserve_destinations_and_legacy_svg_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    let figure = engine.figure.clone().unwrap();
    let path = dir.path().join("kept.eps");
    std::fs::write(&path, b"sentinel").unwrap();
    for canvas in [
        Canvas {
            width: 63.,
            height: 560.,
        },
        Canvas {
            width: f32::NAN,
            height: 560.,
        },
        Canvas {
            width: 8192.,
            height: 8192.,
        },
    ] {
        assert!(
            export::export_vector_with_cancel(&figure, &path, canvas, &AtomicBool::new(false))
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
    }
    assert!(
        export::export_vector_with_cancel(
            &figure,
            &path,
            Canvas::default(),
            &AtomicBool::new(true)
        )
        .is_err()
    );
    engine.cancellation().store(true, Ordering::Relaxed);
    assert!(engine.execute("saveas(gcf,'kept.eps','epsc')").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
    engine.cancellation().store(false, Ordering::Relaxed);
    engine.execute("saveas(gcf,'unchanged.svg');").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("unchanged.svg")).unwrap(),
        figure.svg()
    );
    let count = std::fs::read_dir(dir.path()).unwrap().count();
    let blocked = dir.path().join("directory.eps");
    std::fs::create_dir(&blocked).unwrap();
    assert!(
        export::export_vector_with_cancel(
            &figure,
            &blocked,
            Canvas::default(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), count + 1);
}
