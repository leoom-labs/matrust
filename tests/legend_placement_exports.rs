#![cfg(feature = "plot-export")]
use matrust::{
    Engine, export, files,
    plot::{Figure, LegendLocation, LegendRect, PlotKind, Series},
};
use resvg::usvg::roxmltree::{Document, Node};
use std::sync::atomic::AtomicBool;

fn rectangle(node: Node<'_, '_>) -> LegendRect {
    let number = |key| node.attribute(key).unwrap().parse::<f64>().unwrap();
    let x = number("x");
    let y = number("y");
    LegendRect::new([x, y, x + number("width"), y + number("height")])
}
fn transformed_rectangle(node: Node<'_, '_>) -> LegendRect {
    let mut rect = rectangle(node);
    for parent in node.ancestors().skip(1) {
        if let Some(transform) = parent.attribute("transform") {
            let values: Vec<f64> = transform
                .strip_prefix("matrix(")
                .and_then(|s| s.strip_suffix(')'))
                .expect("legend transform must be an affine matrix")
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            assert_eq!(values.len(), 6);
            assert_eq!((values[1], values[2]), (0., 0.));
            assert!(values[0] > 0. && values[3] > 0.);
            rect = LegendRect::new([
                rect.left * values[0] + values[4],
                rect.top * values[3] + values[5],
                rect.right * values[0] + values[4],
                rect.bottom * values[3] + values[5],
            ]);
        }
    }
    rect
}

#[test]
fn svg_outside_legends_are_disjoint_from_data_and_keep_requested_locations() {
    for location in LegendLocation::ALL.into_iter().filter(|v| v.is_outside()) {
        let mut e = Engine::default();
        e.execute(&format!(
            "plot([0 1],[0 1]);legend('Fréquence Δ','Location','{}');",
            location.name()
        ))
        .unwrap();
        let f = e.figure.unwrap();
        let original = f.clone();
        let svg = f.svg();
        let document = Document::parse(&svg).unwrap();
        let data = rectangle(
            document
                .descendants()
                .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some("axes"))
                .unwrap()
                .children()
                .find(|n| n.has_tag_name("rect"))
                .unwrap(),
        );
        let legend = transformed_rectangle(
            document
                .descendants()
                .find(|n| n.has_tag_name("rect") && n.attribute("stroke") == Some("#aaa"))
                .unwrap(),
        );
        assert!(LegendRect::new([0., 0., 960., 560.]).contains(legend));
        assert!(!legend.overlaps(data));
        match location.outside_side().unwrap() {
            0 => assert!(legend.right < data.left),
            1 => assert!(legend.bottom < data.top),
            2 => assert!(legend.left > data.right),
            _ => assert!(legend.top > data.bottom),
        }
        assert!(
            document
                .descendants()
                .any(|n| n.has_tag_name("title") && n.text() == Some("Fréquence Δ"))
        );
        assert!(f.same_snapshot(&original));
        let script = files::matlab_plot_script(&f, &AtomicBool::new(false)).unwrap();
        assert!(script.contains(&format!("'Location','{}'", location.name())));
        // Ordinary automatic axes must remain ordinary after unchanged script replay.
        // Forcing an OuterPosition here would invent a custom layout and its controls.
        assert!(f.layout.is_none() && f.placement.is_none());
        assert!(script.contains("mr_ax1 = gca;"));
        let mut replay = Engine::default();
        replay.execute(&script).unwrap();
        let recreated = replay.figure.as_ref().unwrap();
        recreated.validate().unwrap();
        assert!(recreated.layout.is_none() && recreated.placement.is_none());
        assert_eq!(recreated.all_axes().count(), 1);
        assert_eq!(recreated.legend_location, location);
        assert_eq!(recreated.legend, f.legend);
        assert_eq!(
            recreated
                .legend_entries()
                .map(|(i, s)| (i, s.label.as_str()))
                .collect::<Vec<_>>(),
            f.legend_entries()
                .map(|(i, s)| (i, s.label.as_str()))
                .collect::<Vec<_>>()
        );
        assert_eq!(recreated.series.len(), f.series.len());
        for (actual, expected) in recreated.series.iter().zip(&f.series) {
            assert_eq!(actual.kind, expected.kind);
            assert_eq!(
                actual.color,
                f.resolve_appearance(matrust::plot::BaseTheme::Light)
                    .series_color(expected)
            );
            assert!(actual.explicit_color && actual.auto_palette.is_none());
            assert_eq!(actual.pattern, expected.pattern);
            assert_eq!(actual.visible, expected.visible);
            assert_eq!(actual.points.len(), expected.points.len());
            assert!(
                actual
                    .points
                    .iter()
                    .zip(expected.points.iter())
                    .all(|(a, b)| a.map(f64::to_bits) == b.map(f64::to_bits))
            );
        }
        assert_eq!(
            recreated.svg(),
            svg,
            "Requested outside geometry and appearance must survive replay"
        );
        assert!(f.same_snapshot(&original));
    }
}

#[test]
fn dense_outside_metadata_and_bounded_ink_survive_exports_with_atomic_cancel() {
    let mut f = Figure::default();
    for i in 0..64 {
        let mut s = Series::new(vec![[0., i as f64], [1., i as f64 + 1.]], PlotKind::Line);
        s.label = format!("Series {i}: Δ & < > {}", "Long label ".repeat(12));
        s.visible = i != 7;
        f.add(vec![s]).unwrap();
    }
    f.legend = true;
    f.legend_location = LegendLocation::Eastoutside;
    f.validate().unwrap();
    let svg = f.svg();
    let document = Document::parse(&svg).unwrap();
    let labels: Vec<_> = document
        .descendants()
        .filter(|n| n.has_tag_name("title"))
        .map(|n| n.text().unwrap())
        .collect();
    assert_eq!(labels.len(), 64);
    assert!(
        f.series
            .iter()
            .zip(labels)
            .all(|(s, label)| s.label == label)
    );
    // Complete labels above are SVG title metadata, not a promise that every
    // glyph is readable at every output size. Visible labels are abbreviated;
    // PNG/PDF have no hover title recovery. Visual fixtures assess that limit.
    let legend = transformed_rectangle(
        document
            .descendants()
            .find(|n| n.has_tag_name("rect") && n.attribute("stroke") == Some("#aaa"))
            .unwrap(),
    );
    let data = rectangle(
        document
            .descendants()
            .find(|n| n.has_tag_name("clipPath") && n.attribute("id") == Some("axes"))
            .unwrap()
            .children()
            .find(|n| n.has_tag_name("rect"))
            .unwrap(),
    );
    assert!(LegendRect::new([0., 0., 960., 560.]).contains(legend));
    assert!(!legend.overlaps(data));
    assert!(legend.left > data.right);
    let dir = tempfile::tempdir().unwrap();
    for extension in ["svg", "png", "pdf"] {
        let path = dir.path().join(format!("outside.{extension}"));
        export::export_figure(&f, &path, 960, 560).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        if extension == "png" {
            let image = image::load_from_memory(&bytes).unwrap();
            assert_eq!((image.width(), image.height()), (960, 560));
        }
        if extension == "pdf" {
            assert!(bytes.starts_with(b"%PDF-"));
        }
        assert!(
            export::export_figure_with_cancel(&f, &path, 960, 560, &AtomicBool::new(true)).is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    let file = dir.path().join("outside.mfig");
    files::save_figure(&file, &f).unwrap();
    let loaded = files::load_figure(&file).unwrap();
    assert_eq!(loaded.svg(), svg);
    let unsupported = dir.path().join("outside.fig");
    std::fs::write(&unsupported, b"keep existing file").unwrap();
    assert!(files::save_matlab_figure(&unsupported, &f).is_err());
    assert_eq!(std::fs::read(&unsupported).unwrap(), b"keep existing file");
}

#[test]
fn grid_exports_keep_readable_panel_text_without_scaling_every_axes_to_a_full_figure() {
    let mut e = Engine::default();
    e.execute("subplot(2,2,1);plot([0 1],[0 1]);legend('North','Location','northoutside');subplot(2,2,2);plot([0 1],[1 0]);legend('East','Location','eastoutside');subplot(2,2,3);plot([0 1],[1 0]);legend('West','Location','westoutside');subplot(2,2,4);plot([0 1],[0 1]);legend('South','Location','southoutside');sgtitle('Grid');").unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    let document = Document::parse(&svg).unwrap();
    let panels: Vec<_> = document
        .descendants()
        .filter(|n| n.has_tag_name("svg") && n.parent().is_some_and(|p| p.has_tag_name("svg")))
        .collect();
    assert_eq!(panels.len(), 4);
    for panel in panels {
        let view: Vec<f64> = panel
            .attribute("viewBox")
            .unwrap()
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        let width: f64 = panel.attribute("width").unwrap().parse().unwrap();
        let height: f64 = panel.attribute("height").unwrap().parse().unwrap();
        // Text is 12 figure units before the user's final canvas scaling,
        // rather than six units caused by a second per-panel half-scale.
        assert_eq!((width, height), (view[2], view[3]));
        let data = rectangle(
            panel
                .descendants()
                .find(|n| n.has_tag_name("clipPath"))
                .unwrap()
                .children()
                .find(|n| n.has_tag_name("rect"))
                .unwrap(),
        );
        let key = transformed_rectangle(
            panel
                .descendants()
                .find(|n| n.has_tag_name("rect") && n.attribute("stroke") == Some("#aaa"))
                .unwrap(),
        );
        assert!(data.width() >= 16. && data.height() >= 16.);
        assert!(LegendRect::new([0., 0., width, height]).contains(key));
        assert!(!key.overlaps(data));
    }
    // Extreme supported grids must still encode bounded nonempty panels.
    for (rows, cols) in [(16, 1), (1, 16), (4, 4)] {
        let mut e = Engine::default();
        for i in 1..=16 {
            e.execute(&format!("subplot({rows},{cols},{i});plot([0 1],[0 1]);legend('Key','Location','eastoutside');")).unwrap();
        }
        let svg = e.figure.unwrap().svg();
        let document = Document::parse(&svg).unwrap();
        assert_eq!(
            document
                .descendants()
                .filter(|n| {
                    n.has_tag_name("clipPath")
                        && n.attribute("id").is_some_and(|id| !id.contains("-legend-"))
                })
                .count(),
            16
        );
        assert_eq!(
            document
                .descendants()
                .filter(|n| n.has_tag_name("title") && n.text() == Some("Key"))
                .count(),
            16
        );
        for panel in document
            .descendants()
            .filter(|n| n.has_tag_name("svg") && n.parent().is_some_and(|p| p.has_tag_name("svg")))
        {
            let view: Vec<f64> = panel
                .attribute("viewBox")
                .unwrap()
                .split_whitespace()
                .map(|x| x.parse().unwrap())
                .collect();
            assert!(view.iter().all(|x| x.is_finite()));
            assert!((320. - 1e-9..=8192.).contains(&view[2]));
            assert!((220. - 1e-9..=8192.).contains(&view[3]));
            let frame = LegendRect::new([0., 0., view[2], view[3]]);
            let data = rectangle(
                panel
                    .descendants()
                    .find(|n| n.has_tag_name("clipPath"))
                    .unwrap()
                    .children()
                    .find(|n| n.has_tag_name("rect"))
                    .unwrap(),
            );
            let key = transformed_rectangle(
                panel
                    .descendants()
                    .find(|n| n.has_tag_name("rect") && n.attribute("stroke") == Some("#aaa"))
                    .unwrap(),
            );
            assert!(data.width() >= 16. && data.height() >= 16.);
            assert!(frame.contains(data) && frame.contains(key));
            assert!(!key.overlaps(data));
        }
    }
}

#[test]
fn explicit_outer_position_remains_explicit_in_unchanged_export_replay() {
    let mut source = Engine::default();
    source.execute("plot([0 1],[0 1]);a=gca;legend(a,'outside','Location','eastoutside');set(a,'OuterPosition',[0 0 1 1],'ActivePositionProperty','outerposition');").unwrap();
    let original = source.figure.as_ref().unwrap();
    let placement = original.placement.unwrap();
    assert_eq!(
        placement.constraint,
        matrust::plot::PositionConstraint::OuterPosition
    );
    let script = files::matlab_plot_script(original, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("'ActivePositionProperty','outerposition'"));
    let mut replay = Engine::default();
    replay.execute(&script).unwrap();
    let recreated = replay.figure.as_ref().unwrap();
    recreated.validate().unwrap();
    assert_eq!(
        recreated.placement.unwrap().requested_outer,
        placement.requested_outer
    );
    assert_eq!(
        recreated.placement.unwrap().constraint,
        placement.constraint
    );
    assert_eq!(recreated.layout.is_some(), original.layout.is_some());
    assert_eq!(recreated.all_axes().count(), original.all_axes().count());
    assert_eq!(recreated.legend_location, original.legend_location);
    assert_eq!(recreated.svg(), original.svg());
}
