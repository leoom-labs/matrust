#![cfg(feature = "plot-export")]
use matrust::{
    Engine, export, files,
    plot::{Figure, LegendLocation, PlotKind, Series},
};
use resvg::usvg::roxmltree::Document;
use std::{collections::HashSet, sync::atomic::AtomicBool};

fn dense() -> Figure {
    let mut f = Figure::default();
    for index in 0..64 {
        let mut s = Series::new(vec![[0., index as f64], [1., index as f64]], PlotKind::Line);
        s.label = format!("Series {index:02} {}", "Wide label <&> ".repeat(20));
        s.visible = index != 3;
        s.line_width = Some(50.);
        s.markers = true;
        s.marker_size = Some(100.);
        f.add(vec![s]).unwrap();
    }
    f.legend = true;
    f
}

#[test]
fn export_legend_dense_labels_titles_visibility_and_native_data_are_preserved() {
    let mut f = dense();
    let before = f.clone();
    let svg = f.svg();
    let xml = Document::parse(&svg).unwrap();
    let titles: Vec<_> = xml
        .descendants()
        .filter(|n| n.has_tag_name("title"))
        .map(|n| n.text().unwrap())
        .collect();
    assert_eq!(titles.len(), 64);
    for (s, label) in f.series.iter().zip(titles) {
        assert_eq!(s.label, label);
    }
    assert_eq!(svg.matches("-legend-").count(), 128);
    assert!(svg.contains("\u{2026}"));
    assert!(!svg.contains("<script"));
    assert!(f.same_snapshot(&before));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("labels.mfig");
    files::save_figure(&path, &f).unwrap();
    let restored = files::load_figure(&path).unwrap();
    for (a, b) in f.series.iter().zip(&restored.series) {
        assert_eq!(a.label, b.label);
        assert_eq!(a.points, b.points);
        assert_eq!(a.visible, b.visible);
    }
    f.series[0].label.clear();
    let svg = f.svg();
    let xml = Document::parse(&svg).unwrap();
    assert_eq!(
        xml.descendants()
            .filter(|n| n.has_tag_name("title"))
            .count(),
        63
    );
    assert!((f.series[0].points).same_storage(&before.series[0].points));
    for s in &mut f.series {
        s.label.clear();
    }
    assert!(!f.svg().contains("stroke=\"#aaa\""));
}

#[test]
fn export_legend_subplots_have_unique_clips_without_rewriting_label_content() {
    let mut engine = Engine::default();
    engine.execute("subplot(1,2,1);plot([1 2]);legend('url(#axes)');subplot(1,2,2);plot([3 4]);legend('url(#axes)');").unwrap();
    let svg = engine.figure.unwrap().svg();
    let xml = Document::parse(&svg).unwrap();
    let mut ids = HashSet::new();
    for node in xml.descendants() {
        if let Some(id) = node.attribute("id") {
            assert!(ids.insert(id));
        }
    }
    for node in xml.descendants() {
        if let Some(reference) = node.attribute("clip-path") {
            let id = reference
                .strip_prefix("url(#")
                .unwrap()
                .strip_suffix(')')
                .unwrap();
            assert!(ids.contains(id));
        }
    }
    let titles: Vec<_> = xml
        .descendants()
        .filter(|n| n.has_tag_name("title"))
        .map(|n| n.text().unwrap())
        .collect();
    assert_eq!(titles, vec!["url(#axes)", "url(#axes)"]);
    assert!(ids.contains("axes0-legend-0") && ids.contains("axes1-legend-0"));
}

#[test]
fn export_legend_raster_changes_stay_inside_box_for_dense_and_small_custom_axes() {
    for extent in [1., 0.15, 1. / 256.] {
        let mut f = dense();
        f.legend_location = LegendLocation::Southwest;
        if extent < 1. {
            f.reposition_axes([0.2, 0.2, extent, extent]).unwrap();
        }
        let svg = f.svg();
        let xml = Document::parse(&svg).unwrap();
        let rect = xml
            .descendants()
            .find(|n| n.has_tag_name("rect") && n.attribute("stroke") == Some("#aaa"))
            .unwrap();
        let number = |name| rect.attribute(name).unwrap().parse::<f64>().unwrap();
        let (mut x, mut y, mut w, mut h) =
            (number("x"), number("y"), number("width"), number("height"));
        if let Some(transform) = rect.parent_element().unwrap().attribute("transform") {
            let m: Vec<f64> = transform
                .strip_prefix("matrix(")
                .unwrap()
                .strip_suffix(')')
                .unwrap()
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            x = x * m[0] + m[4];
            y = y * m[3] + m[5];
            w *= m[0];
            h *= m[3];
        }
        assert!(x >= 0. && y >= 0. && x + w <= 960. && y + h <= 560.);
        let [left, top, right, bottom] = if extent == 1. {
            [70., 60., 920., 490.]
        } else {
            [192., 560. * (0.8 - extent), 960. * (0.2 + extent), 448.]
        };
        assert!(
            x >= left - 1e-8 && y >= top - 1e-8 && x + w <= right + 1e-8 && y + h <= bottom + 1e-8,
            "legend must stay inside minimum axes: box {x},{y},{w},{h}; axes {left},{top},{right},{bottom}"
        );
        let with =
            export::render_figure_with_cancel(&f, 960, 560, &AtomicBool::new(false)).unwrap();
        for axes in f.all_axes_mut() {
            axes.legend = false;
        }
        let without =
            export::render_figure_with_cancel(&f, 960, 560, &AtomicBool::new(false)).unwrap();
        let mut changed = 0;
        for (px, py, pixel) in with.enumerate_pixels() {
            if pixel != without.get_pixel(px, py) {
                changed += 1;
                assert!(
                    (px as f64) >= x - 2.
                        && (py as f64) >= y - 2.
                        && (px as f64) <= x + w + 2.
                        && (py as f64) <= y + h + 2.,
                    "outside legend at {px},{py}; box {x},{y},{w},{h}"
                );
            }
        }
        assert!(changed > if extent >= 0.15 { 100 } else { 0 });
    }
}
