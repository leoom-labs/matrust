#![cfg(feature = "plot-export")]
use matrust::Engine;
use std::sync::atomic::Ordering;

fn svg_pixels(svg: &str) -> resvg::tiny_skia::Pixmap {
    let tree = resvg::usvg::Tree::from_str(svg, &matrust::export::svg_render_options()).unwrap();
    let mut pixels =
        resvg::tiny_skia::Pixmap::new(tree.size().width() as u32, tree.size().height() as u32)
            .unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    pixels
}

#[test]
fn explicit_parent_current_color_controls_figure_and_axes_margins_without_expanding_crop() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(directory.path()).unwrap();
    engine.execute("f=figure(1);set(f,'Color',[.2 .4 .6]);a=gca;plot(a,[0 1],[0 1]);grid off;title('Colored parent');").unwrap();
    let before = engine.figure.clone().unwrap();
    for target in ["f", "a"] {
        engine.execute(&format!("exportgraphics({target},'{target}-current.svg','BackgroundColor','current');exportgraphics({target},'{target}-default.svg');exportgraphics({target},'{target}-white.svg','BackgroundColor','white');exportgraphics({target},'{target}-none.svg','BackgroundColor','none');exportgraphics({target},'{target}-current.png','BackgroundColor','current','Resolution',96);exportgraphics({target},'{target}-white.png','BackgroundColor','white','Resolution',96);exportgraphics({target},'{target}-none.png','BackgroundColor','none','Resolution',96);exportgraphics({target},'{target}-current.pdf','BackgroundColor','current','ContentType','vector');")).unwrap();
        let load = |policy| {
            std::fs::read_to_string(directory.path().join(format!("{target}-{policy}.svg")))
                .unwrap()
        };
        let current = load("current");
        assert_eq!(current, load("default"));
        let parsed = resvg::usvg::roxmltree::Document::parse(&current).unwrap();
        let root = parsed.root_element();
        for policy in ["white", "none"] {
            let other = load(policy);
            let doc = resvg::usvg::roxmltree::Document::parse(&other).unwrap();
            for attr in ["width", "height", "viewBox"] {
                assert_eq!(root.attribute(attr), doc.root_element().attribute(attr));
            }
        }
        let current_pixels = svg_pixels(&current);
        let white_pixels = svg_pixels(&load("white"));
        let none_pixels = svg_pixels(&load("none"));
        assert_eq!(&current_pixels.data()[..4], &[51, 102, 153, 255]);
        assert_eq!(&white_pixels.data()[..4], &[255; 4]);
        assert_eq!(none_pixels.data()[3], 0);
        // The margin changes, but the model's white data area stays white.
        let clip = parsed
            .descendants()
            .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some("axes"))
            .unwrap();
        let rect = clip.children().find(|n| n.has_tag_name("rect")).unwrap();
        let value = |name| rect.attribute(name).unwrap().parse::<f64>().unwrap();
        let view: Vec<f64> = root
            .attribute("viewBox")
            .unwrap()
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect();
        let x = (value("x") + value("width") * 0.25 - view[0]) as usize;
        let y = (value("y") + value("height") * 0.25 - view[1]) as usize;
        let at = (y * current_pixels.width() as usize + x) * 4;
        assert_eq!(&current_pixels.data()[at..at + 4], &[255; 4]);
        let png = image::open(directory.path().join(format!("{target}-current.png")))
            .unwrap()
            .to_rgb8();
        assert_eq!(
            png.dimensions(),
            (current_pixels.width(), current_pixels.height())
        );
        for (rgb, rgba) in png.pixels().zip(current_pixels.data().as_chunks::<4>().0) {
            assert_eq!(&rgb.0, &rgba[..3]);
        }
        let white = image::open(directory.path().join(format!("{target}-white.png")))
            .unwrap()
            .to_rgb8();
        let none = image::open(directory.path().join(format!("{target}-none.png")))
            .unwrap()
            .to_rgb8();
        assert_eq!(white, none);
        assert_eq!(white.get_pixel(0, 0).0, [255; 3]);
        assert!(
            std::fs::read(directory.path().join(format!("{target}-current.pdf")))
                .unwrap()
                .starts_with(b"%PDF-")
        );
        assert!(before.same_snapshot(engine.figure.as_ref().unwrap()));
        assert!(
            (before.series[0].points)
                .same_storage(&engine.figure.as_ref().unwrap().series[0].points)
        );
    }
    assert!(engine.take_figure_file_events().is_empty());
}

#[test]
fn current_color_underlays_follow_nested_axes_viewports_and_keep_cancellation_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(directory.path()).unwrap();
    engine.execute("figure(1);set(gcf,'Color',[.2 .4 .6]);subplot(1,2,1);a=gca;plot([1 2]);subplot(1,2,2);plot([2 1]);exportgraphics(gcf,'whole.svg');exportgraphics(a,'selected.svg');").unwrap();
    for (name, expected) in [("whole.svg", 2), ("selected.svg", 1)] {
        let svg = std::fs::read_to_string(directory.path().join(name)).unwrap();
        let doc = resvg::usvg::roxmltree::Document::parse(&svg).unwrap();
        let axes: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("svg") && n != &doc.root_element())
            .collect();
        assert_eq!(axes.len(), expected);
        for axes in axes {
            let fill = axes
                .children()
                .find(|n| n.has_tag_name("rect") && n.attribute("fill") == Some("white"))
                .unwrap();
            let clip = axes
                .children()
                .find(|n| n.has_tag_name("defs"))
                .unwrap()
                .descendants()
                .find(|n| n.has_tag_name("rect"))
                .unwrap();
            for attr in ["x", "y", "width", "height"] {
                assert_eq!(fill.attribute(attr), clip.attribute(attr));
            }
        }
    }
    let path = directory.path().join("kept.svg");
    std::fs::write(&path, b"original").unwrap();
    engine.cancellation().store(true, Ordering::Relaxed);
    assert!(
        engine
            .execute("exportgraphics(a,'kept.svg','BackgroundColor','current');")
            .is_err()
    );
    engine.cancellation().store(false, Ordering::Relaxed);
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
}
