#![cfg(feature = "plot-export")]
use matrust::{Engine, export, plot::Canvas};
use std::sync::atomic::AtomicBool;

#[test]
fn vector_canvas_matches_raster_geometry_for_single_log_and_layout_figures() {
    let cancel = AtomicBool::new(false);
    for script in [
        "plot([0 1 2],[0 1 0]);title('Canvas <test>');xlabel('Time');ylabel('Value');",
        "semilogy(1:10,1:10);title('Log axis');",
        "tiledlayout(2,2);nexttile;plot(1:10);nexttile;stem([1 3 2]);nexttile([1 2]);stairs([3 2 1]);",
    ] {
        let mut e = Engine::default();
        e.execute(script).unwrap();
        let figure = e.figure.as_ref().unwrap();
        assert_eq!(
            figure.svg_on_canvas(Canvas::default()).unwrap(),
            figure.svg()
        );
        for (width, height) in [(960, 960), (1200, 560), (1440, 840)] {
            let svg = figure
                .svg_on_canvas(Canvas {
                    width: width as f32,
                    height: height as f32,
                })
                .unwrap();
            let tree = resvg::usvg::Tree::from_str(&svg, &export::svg_render_options()).unwrap();
            assert_eq!(
                (tree.size().width(), tree.size().height()),
                (width as f32, height as f32)
            );
            let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).unwrap();
            resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
            let raster = export::render_figure_with_cancel(figure, width, height, &cancel).unwrap();
            assert_eq!(
                pixmap.data(),
                raster.as_raw(),
                "Canvas {width}x{height}: {script}"
            );
        }
    }
}
#[test]
fn vector_sizes_validate_before_publication_and_legacy_exports_remain_canonical() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("plot(1:5);").unwrap();
    let figure = e.figure.as_ref().unwrap();
    let path = dir.path().join("plot.svg");
    std::fs::write(&path, b"keep").unwrap();
    for canvas in [
        Canvas {
            width: f32::NAN,
            height: 560.,
        },
        Canvas {
            width: f32::INFINITY,
            height: 560.,
        },
        Canvas {
            width: 63.,
            height: 560.,
        },
        Canvas {
            width: 8192.,
            height: 8192.,
        },
        Canvas {
            width: 8193.,
            height: 560.,
        },
        Canvas {
            width: 8191.5,
            height: 3906.25,
        },
    ] {
        assert!(
            export::export_vector_with_cancel(figure, &path, canvas, &AtomicBool::new(false))
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
    assert!(
        export::export_vector_with_cancel(figure, &path, Canvas::default(), &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    assert!(
        export::export_vector_with_cancel(
            figure,
            &dir.path().join("bad.png"),
            Canvas::default(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let canvas = Canvas {
        width: 595.2756,
        height: 841.8898,
    };
    export::export_vector_with_cancel(figure, &path, canvas, &AtomicBool::new(false)).unwrap();
    let svg = std::fs::read_to_string(&path).unwrap();
    let tree = resvg::usvg::Tree::from_str(&svg, &export::svg_render_options()).unwrap();
    assert_eq!(
        (tree.size().width(), tree.size().height()),
        (canvas.width, canvas.height)
    );
    let pdf = dir.path().join("plot.pdf");
    export::export_vector_with_cancel(figure, &pdf, canvas, &AtomicBool::new(false)).unwrap();
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    export::export_figure(figure, &path, 1600, 1600).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), figure.svg());
}
