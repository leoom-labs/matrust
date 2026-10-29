use matrust::Engine;
#[cfg(feature = "plot-export")]
use std::sync::atomic::Ordering;

fn engine(folder: &std::path::Path) -> Engine {
    let mut engine = Engine::default();
    engine.set_file_directory(folder).unwrap();
    engine.set_function_directory(folder).unwrap();
    engine
        .execute("plot([0 1 2],[0 1 0]);title('Whole figure export');xlabel('Time');")
        .unwrap();
    engine
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_whole_figures_preserve_samples_selection_and_file_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    // This historical byte-equivalence check uses a fixed legacy white figure:
    // new Auto Light canvas is intentionally distinct from explicit white export.
    engine.figure.as_mut().unwrap().appearance = None;
    let original = engine.figure.clone().unwrap();
    engine.execute("exportgraphics(gcf,'quoted % '' name.SVG','ContentType','vector','BackgroundColor',[1 1 1]);").unwrap();
    let first = std::fs::read_to_string(dir.path().join("quoted % ' name.SVG")).unwrap();
    assert!(first.contains("Whole figure export"));
    assert!(first.contains("viewBox=\"") && !first.contains("viewBox=\"0 0 960 560\""));
    assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    engine.execute("figure(2);subplot(2,1,1);plot([3 4]);title('Upper panel');subplot(2,1,2);plot([5 6]);title('Lower panel');").unwrap();
    let second = engine.figure.clone().unwrap();
    engine.execute("exportgraphics(1,'first.svg','backgroundcolor','current','contenttype','auto');exportgraphics(2,'layout.svg');").unwrap();
    assert_eq!(engine.current_figure, 2);
    assert!(second.same_snapshot(engine.figure.as_ref().unwrap()));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("first.svg")).unwrap(),
        first
    );
    let layout = std::fs::read_to_string(dir.path().join("layout.svg")).unwrap();
    assert!(layout.contains("Upper panel") && layout.contains("Lower panel"));
    assert!(engine.take_figure_file_events().is_empty());
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_auto_current_canvas_differs_from_explicit_white_without_editing_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    let original = engine.figure.clone().unwrap();
    engine
        .execute("exportgraphics(gcf,'auto-current.svg','BackgroundColor','current');exportgraphics(gcf,'explicit-white.svg','BackgroundColor','white');")
        .unwrap();
    let current = std::fs::read_to_string(dir.path().join("auto-current.svg")).unwrap();
    let white = std::fs::read_to_string(dir.path().join("explicit-white.svg")).unwrap();
    // The canvas precedes axes definitions. This distinguishes the requested
    // background intent without confusing it with the white data rectangle.
    let current_canvas = current.split_once("<defs>").unwrap().0;
    let white_canvas = white.split_once("<defs>").unwrap().0;
    assert!(current_canvas.contains("fill=\"#f5f5f5\""));
    assert!(white_canvas.contains("fill=\"white\""));
    assert_ne!(current, white);
    let after = engine.figure.as_ref().unwrap();
    assert!(original.same_snapshot(after));
    assert!(
        original.series[0]
            .points
            .same_storage(&after.series[0].points)
    );
    assert!(engine.take_figure_file_events().is_empty());
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_rejects_unsupported_semantics_before_replacing_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    let original = engine.figure.clone().unwrap();
    let kept = dir.path().join("keep.svg");
    std::fs::write(&kept, b"original").unwrap();
    for script in [
        "exportgraphics()",
        "exportgraphics(gcf)",
        "exportgraphics(gcf,4)",
        "exportgraphics([gca gca],'keep.svg')",
        "exportgraphics([1 2],'keep.svg')",
        "exportgraphics(true,'keep.svg')",
        "exportgraphics(999,'keep.svg')",
        "exportgraphics(1.5,'keep.svg')",
        "exportgraphics(0/0,'keep.svg')",
        "exportgraphics(gcf,'keep.svg','Resolution',0)",
        "exportgraphics(gcf,'keep.svg','Resolution',1.5)",
        "exportgraphics(gcf,'keep.svg','Resolution',1/0)",
        "exportgraphics(gcf,'keep.svg','Resolution',65536)",
        "exportgraphics(gcf,'keep.svg','Resolution',true)",
        "exportgraphics(gcf,'keep.svg','Resolution',96,'resolution',192)",
        "exportgraphics(gcf,'keep.svg','ContentType','image')",
        "exportgraphics(gcf,'keep.svg','ContentType','unknown')",
        "exportgraphics(gcf,'keep.svg','ContentType',3)",
        "exportgraphics(gcf,'keep.svg','BackgroundColor',[1 0 0])",
        "exportgraphics(gcf,'keep.svg','BackgroundColor',[1;1;1])",
        "exportgraphics(gcf,'keep.svg','BackgroundColor',[1 1 0/0])",
        "exportgraphics(gcf,'keep.svg','ContentType','auto','contenttype','auto')",
        "exportgraphics(gcf,'keep.svg','BackgroundColor','white','BACKGROUNDcolor','w')",
        "exportgraphics(gcf,'keep.svg','Append',true)",
        "exportgraphics(gcf,'keep.svg','Width',500)",
        "exportgraphics(gcf,'keep.svg',7,'white')",
        "exportgraphics(gcf,'keep.svg','ContentType')",
        "out=exportgraphics(gcf,'keep.svg')",
        "disp(exportgraphics(gcf,'keep.svg'))",
        "exportgraphics(gcf,'https://example.invalid/keep.svg')",
        "exportgraphics(gcf,'missing/keep.svg')",
        "exportgraphics(gcf,'')",
        "exportgraphics(gcf,'no_extension')",
        "exportgraphics(gcf,'bad.fig')",
        "exportgraphics(gcf,'bad.bmp')",
        "exportgraphics(gcf,'bad.png','ContentType','vector')",
    ] {
        assert!(engine.execute(script).is_err(), "{script}");
        assert_eq!(std::fs::read(&kept).unwrap(), b"original", "{script}");
        assert!(
            original.same_snapshot(engine.figure.as_ref().unwrap()),
            "{script}"
        );
        assert!(engine.take_figure_file_events().is_empty());
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_cancel_and_atomic_failure_preserve_destination_and_relative_folders() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut engine = engine(a.path());
    let file = a.path().join("output.svg");
    std::fs::write(&file, b"original").unwrap();
    engine.cancellation().store(true, Ordering::Relaxed);
    assert!(engine.execute("exportgraphics(gcf,'output.svg');").is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"original");
    engine.cancellation().store(false, Ordering::Relaxed);
    let directory = a.path().join("directory.svg");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("keep"), b"unchanged").unwrap();
    assert!(
        engine
            .execute("exportgraphics(gcf,'directory.svg');")
            .is_err()
    );
    assert_eq!(std::fs::read(directory.join("keep")).unwrap(), b"unchanged");
    engine.set_file_directory(b.path()).unwrap();
    engine
        .execute("exportgraphics(gcf,'output.svg','BackgroundColor','WHITE');")
        .unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"original");
    assert!(b.path().join("output.svg").is_file());
    assert_eq!(std::fs::read_dir(a.path()).unwrap().count(), 2);
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_variable_and_function_shadowing_and_prior_publication() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    assert!(
        engine
            .execute("exportgraphics(gcf,'completed.svg');unknown_function();")
            .is_err()
    );
    assert!(dir.path().join("completed.svg").is_file());
    engine
        .execute("exportgraphics=[11 22];v=exportgraphics(2);")
        .unwrap();
    assert_eq!(engine.variables["v"].as_scalar().unwrap(), 22.);
    engine
        .execute("clear;exportgraphics=logical([0 1]);v=exportgraphics(2);")
        .unwrap();
    assert_eq!(engine.logical_variables["v"].data()[0], 1);
    engine
        .execute("clear;function y=exportgraphics(x)\ny=x+5;\nend\nv=exportgraphics(2);")
        .unwrap();
    assert_eq!(engine.variables["v"].as_scalar().unwrap(), 7.);
    std::fs::write(
        dir.path().join("exportgraphics.m"),
        "function y=exportgraphics(x)\ny=x*3;\nend\n",
    )
    .unwrap();
    let mut external = Engine::default();
    external.set_function_directory(dir.path()).unwrap();
    external.execute("v=exportgraphics(4);").unwrap();
    assert_eq!(external.variables["v"].as_scalar().unwrap(), 12.);
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_codec_metadata_dimensions_and_vector_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    let original = engine.figure.clone().unwrap();
    for (extension, format) in [
        ("png", "png"),
        ("JPG", "jpeg"),
        ("jpeg", "jpeg"),
        ("tif", "tiff"),
        ("TIFF", "tiff"),
        ("pdf", "pdf"),
    ] {
        let file = dir.path().join(format!("actual.{extension}"));
        let content = if format == "pdf" { "vector" } else { "image" };
        engine.execute(&format!("exportgraphics(gcf,'actual.{extension}','ContentType','{content}','BackgroundColor','w','Resolution',192);")).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        if format == "pdf" {
            assert!(bytes.starts_with(b"%PDF"));
            assert!(
                !bytes
                    .windows(b"/Subtype /Image".len())
                    .any(|w| w == b"/Subtype /Image")
            );
        } else {
            let image = image::load_from_memory(&bytes).unwrap().to_rgb8();
            assert!(image.width() < 1920 && image.height() <= 1120);
            assert_eq!(image.get_pixel(0, 0).0, [255; 3]);
            assert!(image.pixels().any(|p| p.0.iter().any(|v| *v < 100)));
            match format {
                "png" => {
                    let reader = png::Decoder::new(std::io::Cursor::new(&bytes))
                        .read_info()
                        .unwrap();
                    let density = reader.info().pixel_dims.unwrap();
                    assert_eq!(density.unit, png::Unit::Meter);
                    assert_eq!(density.xppu, (192_f64 / 0.0254).round() as u32);
                    assert_eq!(density.xppu, density.yppu);
                }
                "jpeg" => {
                    let at = bytes.windows(5).position(|w| w == b"JFIF\0").unwrap();
                    assert_eq!(bytes[at + 7], 1);
                    assert_eq!(u16::from_be_bytes([bytes[at + 8], bytes[at + 9]]), 192);
                    assert_eq!(u16::from_be_bytes([bytes[at + 10], bytes[at + 11]]), 192);
                }
                _ => {
                    let mut decoder =
                        tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes)).unwrap();
                    assert_eq!(
                        decoder
                            .get_tag_u32(tiff::tags::Tag::ResolutionUnit)
                            .unwrap(),
                        2
                    );
                    assert_eq!(
                        decoder
                            .get_tag_u32_vec(tiff::tags::Tag::XResolution)
                            .unwrap(),
                        [192, 1]
                    );
                    assert_eq!(
                        decoder
                            .get_tag_u32_vec(tiff::tags::Tag::YResolution)
                            .unwrap(),
                        [192, 1]
                    );
                }
            }
        }
        assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
        assert!(engine.take_figure_file_events().is_empty());
    }
}

#[cfg(not(feature = "plot-export"))]
#[test]
fn exportgraphics_missing_optional_codecs_fail_without_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    for extension in ["png", "jpg", "tif", "pdf", "svg"] {
        assert!(
            engine
                .execute(&format!("exportgraphics(gcf,'output.{extension}');"))
                .unwrap_err()
                .contains("plot-export")
        );
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_shared_crop_scales_rasters_and_preserves_transparent_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    engine.execute("set(gcf,'Color',[.2 .4 .6]);exportgraphics(gcf,'clear.svg','BackgroundColor','none','Resolution',65535);exportgraphics(gcf,'low.png','Resolution',96);exportgraphics(gcf,'high.png','Resolution',192);exportgraphics(gcf,'default.png');exportgraphics(gcf,'none.png','BackgroundColor','none','ContentType','image','Resolution',96);exportgraphics(gcf,'white.png','BackgroundColor','white','Resolution',96);").unwrap();
    let svg = std::fs::read_to_string(dir.path().join("clear.svg")).unwrap();
    let tree = resvg::usvg::Tree::from_str(&svg, &matrust::export::svg_render_options()).unwrap();
    let (w, h) = (tree.size().width() as u32, tree.size().height() as u32);
    assert!(w < 960 && h <= 560);
    let low = image::open(dir.path().join("low.png")).unwrap().to_rgb8();
    let high = image::open(dir.path().join("high.png")).unwrap();
    let default = image::open(dir.path().join("default.png")).unwrap();
    assert_eq!(low.dimensions(), (w, h));
    assert_eq!((high.width(), high.height()), (2 * w, 2 * h));
    assert_eq!(
        (default.width(), default.height()),
        (
            (w as f64 * 150. / 96.).ceil() as u32,
            (h as f64 * 150. / 96.).ceil() as u32
        )
    );
    let none = image::open(dir.path().join("none.png")).unwrap().to_rgb8();
    let white = image::open(dir.path().join("white.png")).unwrap().to_rgb8();
    assert_eq!(none, white);
    assert_eq!(low.get_pixel(0, 0).0, [51, 102, 153]);
    assert_eq!(none.get_pixel(0, 0).0, [255; 3]);
    assert_ne!(low, none);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    assert_eq!(pixmap.data()[3], 0);
    assert!(pixmap.data().as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    engine.execute("exportgraphics(gcf,'transparent.pdf','ContentType','vector','BackgroundColor','none');").unwrap();
    assert!(
        std::fs::read(dir.path().join("transparent.pdf"))
            .unwrap()
            .starts_with(b"%PDF")
    );
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_crops_clipped_outliers_white_ink_and_unicode_decorations() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    engine.execute("plot([-1000000000000 0 1 1000000000000],[1000000000000 0 1 -1000000000000],'Color','white','LineWidth',4);xlim([0 1]);ylim([0 1]);title('Fréquence Δ signal');xlabel('Long label with spaces and accents ééé');legend('White clipped data');exportgraphics(gcf,'white.svg','BackgroundColor','none');").unwrap();
    let white = std::fs::read_to_string(dir.path().join("white.svg")).unwrap();
    assert!(white.contains("Fréquence Δ signal") && white.contains("White clipped data"));
    let tree = resvg::usvg::Tree::from_str(&white, &matrust::export::svg_render_options()).unwrap();
    assert!(tree.size().width() <= 960. && tree.size().height() <= 560.);
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(tree.size().width() as u32, tree.size().height() as u32)
            .unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    assert!(
        pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] == 255 && p[..3] == [255, 255, 255])
    );
    // All-invisible data still has visible axes/decorations; crop that content.
    engine.execute("plot([0 1],[0 1],'LineStyle','none','Marker','none');title('Invisible data; visible axes');exportgraphics(gcf,'invisible.svg');").unwrap();
    assert!(
        std::fs::read_to_string(dir.path().join("invisible.svg"))
            .unwrap()
            .contains("Invisible data; visible axes")
    );
    let mut empty = Engine::default();
    empty.set_file_directory(dir.path()).unwrap();
    assert!(empty.execute("exportgraphics(gcf,'empty.svg');").is_err());
    assert!(!dir.path().join("empty.svg").exists());
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_large_resolution_is_bounded_and_saveas_output_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    let original = engine.figure.clone().unwrap();
    engine
        .execute("saveas(gcf,'before.svg');saveas(gcf,'before.png');")
        .unwrap();
    let kept = dir.path().join("keep.png");
    std::fs::write(&kept, b"original").unwrap();
    assert!(
        engine
            .execute("exportgraphics(gcf,'keep.png','Resolution',65535);")
            .is_err()
    );
    assert_eq!(std::fs::read(kept).unwrap(), b"original");
    engine
        .execute(
            "exportgraphics(gcf,'cropped.svg');saveas(gcf,'after.svg');saveas(gcf,'after.png');",
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("before.svg")).unwrap(),
        original.svg()
    );
    for ext in ["svg", "png"] {
        assert_eq!(
            std::fs::read(dir.path().join(format!("before.{ext}"))).unwrap(),
            std::fs::read(dir.path().join(format!("after.{ext}"))).unwrap()
        );
    }
    assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
}

#[cfg(feature = "plot-export")]
#[test]
fn exportgraphics_svg_budget_rejects_dense_marker_geometry_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine(dir.path());
    let file = dir.path().join("keep.svg");
    std::fs::write(&file, b"original").unwrap();
    engine
        .execute(
            "x=ones(1,500000);plot(x,x,'LineStyle','none','Marker','*');xlim([0 2]);ylim([0 2]);",
        )
        .unwrap();
    let original = engine.figure.clone().unwrap();
    let error = engine
        .execute("exportgraphics(gcf,'keep.svg');")
        .unwrap_err();
    assert!(error.contains("allocation budget"), "{error}");
    assert_eq!(std::fs::read(file).unwrap(), b"original");
    assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
