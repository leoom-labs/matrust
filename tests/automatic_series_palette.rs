use matrust::{
    Array, Engine, files,
    plot::{BaseTheme, Figure, FigureTheme, PlotKind, Series},
};
use std::sync::atomic::{AtomicBool, Ordering};

const LIGHT: [[u8; 3]; 7] = [
    [17, 113, 190],
    [221, 84, 0],
    [237, 177, 32],
    [133, 22, 209],
    [59, 170, 50],
    [47, 190, 239],
    [209, 4, 139],
];
const DARK: [[u8; 3]; 7] = [
    [86, 180, 233],
    [240, 143, 89],
    [245, 205, 87],
    [190, 139, 242],
    [126, 207, 112],
    [90, 214, 245],
    [244, 128, 199],
];
fn colors(f: &Figure, host: BaseTheme) -> Vec<[u8; 3]> {
    let appearance = f.resolve_appearance(host);
    f.series
        .iter()
        .map(|s| appearance.series_color(s))
        .collect()
}
fn point() -> Series {
    Series::new(vec![[-0., 2.], [1., 3.]], PlotKind::Line)
}
fn legacy() -> Figure {
    Figure {
        appearance: None,
        axes: Default::default(),
        ..Figure::default()
    }
}
fn bytes_equal(a: &Figure, b: &Figure) {
    assert_eq!(a.series.len(), b.series.len());
    for (a, b) in a.series.iter().zip(&b.series) {
        assert_eq!(a.points.len(), b.points.len());
        for (a, b) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
}

#[test]
fn automatic_cycle_retains_slots_across_delete_and_manual_palette_equal_colors() {
    let mut e = Engine::default();
    e.execute("h1=plot([0 1],[1 2]);hold on;h2=plot([0 1],[2 3],'Color',[17 113 190]/255);h3=plot([0 1],[3 4],'r');").unwrap();
    for _ in 0..6 {
        e.execute("plot([0 1],[4 5]);").unwrap();
    }
    let before = e.figure.clone().unwrap();
    assert_eq!(
        colors(&before, BaseTheme::Light),
        vec![
            LIGHT[0],
            LIGHT[0],
            [255, 0, 0],
            LIGHT[3],
            LIGHT[4],
            LIGHT[5],
            LIGHT[6],
            LIGHT[0],
            LIGHT[1]
        ]
    );
    assert_eq!(
        colors(&before, BaseTheme::Dark),
        vec![
            DARK[0],
            LIGHT[0],
            [255, 0, 0],
            DARK[3],
            DARK[4],
            DARK[5],
            DARK[6],
            DARK[0],
            DARK[1]
        ]
    );
    assert!(before.series[1].explicit_color && before.series[1].auto_palette.is_none());
    // Deletion is the existing public metadata API; there is no delete builtin.
    let cursor = before.axes.palette_cycle;
    let removed = e.figure.as_mut().unwrap().remove_series(0).unwrap();
    assert!(removed.identity.same_object(&before.series[0].identity));
    assert!(removed.points.same_storage(&before.series[0].points));
    assert_eq!(e.figure.as_ref().unwrap().axes.palette_cycle, cursor);
    e.execute("plot([0 1],[9 10]);").unwrap();
    let after = e.figure.as_ref().unwrap();
    for (old, new) in before.series[1..].iter().zip(&after.series) {
        assert!(old.identity.same_object(&new.identity));
        assert!(old.points.same_storage(&new.points));
        assert_eq!(old.auto_palette, new.auto_palette);
    }
    assert_eq!(colors(after, BaseTheme::Dark).last(), Some(&DARK[2]));
    let retained = after.clone();
    e.set_graphics_theme_context(BaseTheme::Dark);
    assert!(retained.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute("hold off;plot([0 1],[3 2]);").unwrap();
    assert_eq!(
        colors(e.figure.as_ref().unwrap(), BaseTheme::Dark),
        vec![DARK[0]]
    );
}

#[test]
fn independent_axes_and_clear_inherit_only_saved_document_palette_policy() {
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([0 1],[1 2]);hold on;plot([0 1],[2 3]);subplot(1,2,2);stem([0 1],[3 4]);").unwrap();
    let f = e.figure.as_ref().unwrap();
    for (_, a) in f.all_axes() {
        assert_eq!(a.series[0].auto_palette.unwrap().slot, 0);
    }
    e.execute("subplot(1,2,1);plot([0 1],[3 4]);").unwrap();
    assert_eq!(
        e.figure
            .as_ref()
            .unwrap()
            .series
            .last()
            .unwrap()
            .auto_palette
            .unwrap()
            .slot,
        2
    );
    e.execute("clf;plot([0 1],[4 5]);").unwrap();
    assert_eq!(
        colors(e.figure.as_ref().unwrap(), BaseTheme::Dark),
        vec![DARK[0]]
    );
    e.execute("tiledlayout(1,2);nexttile;plot([0 1],[1 2]);nexttile;plot([0 1],[2 3]);")
        .unwrap();
    for (_, a) in e.figure.as_ref().unwrap().all_axes() {
        assert_eq!(a.series[0].auto_palette.unwrap().slot, 0);
    }

    let mut old = legacy();
    old.add(vec![point(), point()]).unwrap();
    old.series.remove(0);
    old.add(vec![point()]).unwrap();
    assert_eq!(old.series[1].color, matrust::plot::COLORS[1]);
    old.set_figure_theme(FigureTheme::Dark);
    assert_eq!(old.appearance.unwrap().series_palette, None);
    old.select_subplot(
        matrust::plot::Position::from_grid(1, 2, &[2]).unwrap(),
        false,
    )
    .unwrap();
    old.add(vec![point()]).unwrap();
    assert!(old.palette_cycle.is_none() && old.series[0].auto_palette.is_none());
    assert_eq!(
        colors(&old, BaseTheme::Dark),
        vec![matrust::plot::COLORS[0]]
    );
}

#[test]
fn face_query_and_manual_set_use_context_without_recoloring_shared_data() {
    let mut e = Engine::default();
    e.execute("h=bar([1 2],[2 3]);c=get(h,'FaceColor');")
        .unwrap();
    let before = e.figure.clone().unwrap();
    let Array::Numeric(c) = e.array("c").unwrap() else {
        panic!("RGB matrix")
    };
    assert_eq!(c.data(), &LIGHT[0].map(|c| f64::from(c) / 255.));
    e.set_graphics_theme_context(BaseTheme::Dark);
    e.execute("c=get(h,'FaceColor');").unwrap();
    let Array::Numeric(c) = e.array("c").unwrap() else {
        panic!("RGB matrix")
    };
    assert_eq!(c.data(), &DARK[0].map(|c| f64::from(c) / 255.));
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute("set(h,'FaceColor',[17 113 190]/255);").unwrap();
    let after = e.figure.as_ref().unwrap();
    assert_eq!(colors(after, BaseTheme::Dark), vec![LIGHT[0]]);
    assert!(
        before.series[0]
            .points
            .same_storage(&after.series[0].points)
    );
    assert!(
        before.series[0]
            .identity
            .same_object(&after.series[0].identity)
    );
}

fn rewrite_metadata(bytes: &[u8], mutate: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes[12..12 + n]).unwrap();
    mutate(&mut value);
    let text = serde_json::to_vec(&value).unwrap();
    let mut result = bytes[..8].to_vec();
    result.extend_from_slice(&(text.len() as u32).to_le_bytes());
    result.extend_from_slice(&text);
    result.extend_from_slice(&bytes[12 + n..]);
    result
}
#[test]
fn native_capabilities_preserve_cursor_and_reject_forged_or_invalid_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intent.mfig");
    let mut f = Figure::default();
    f.add(vec![point(), point(), point()]).unwrap();
    f.series.remove(0);
    files::save_figure(&path, &f).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGV"));
    let mut reopened = files::load_figure(&path).unwrap();
    bytes_equal(&f, &reopened);
    assert_eq!(reopened.axes.palette_cycle, f.axes.palette_cycle);
    assert_eq!(colors(&reopened, BaseTheme::Dark), vec![DARK[1], DARK[2]]);
    reopened.add(vec![point()]).unwrap();
    assert_eq!(colors(&reopened, BaseTheme::Dark).last(), Some(&DARK[3]));
    for bad in [
        rewrite_metadata(&bytes, |v| {
            v["series"][0]["auto_palette"]["slot"] = 7.into()
        }),
        rewrite_metadata(&bytes, |v| v["palette_cycle"]["next_slot"] = 7.into()),
        rewrite_metadata(&bytes, |v| {
            v["series"][0]["auto_palette"]["version"] = "Unknown".into()
        }),
        {
            let mut b = bytes.clone();
            b[7] = b'T';
            b
        },
    ] {
        std::fs::write(&path, bad).unwrap();
        assert!(files::load_figure(&path).is_err());
    }
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([0 1],[1 2]);subplot(1,2,2);plot([0 1],[2 3]);")
        .unwrap();
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGW"));
    assert_eq!(files::load_figure(&path).unwrap().all_axes().count(), 2);
    // Validate the second child's bounded provenance before reading even the
    // first child's intentionally missing sample payload.
    let outer = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut cursor = 12 + outer;
    let mut children = Vec::new();
    for _ in 0..2 {
        let length = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        children.push(bytes[cursor..cursor + length].to_vec());
        cursor += length;
    }
    assert_eq!(cursor, bytes.len());
    let first_metadata = u32::from_le_bytes(children[0][8..12].try_into().unwrap()) as usize;
    children[0].truncate(12 + first_metadata);
    children[1] = rewrite_metadata(&children[1], |v| {
        v["series"][0]["auto_palette"]["slot"] = 7.into()
    });
    let mut bad = bytes[..12 + outer].to_vec();
    for child in children {
        bad.extend_from_slice(&(child.len() as u32).to_le_bytes());
        bad.extend_from_slice(&child);
    }
    std::fs::write(&path, bad).unwrap();
    assert!(
        files::load_figure(&path)
            .unwrap_err()
            .contains("palette slot")
    );
    let mut nested = bytes.clone();
    nested[16 + outer..24 + outer].copy_from_slice(b"MATRFIGW");
    std::fs::write(&path, nested).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
    bytes[7] = b'U';
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGW"));
    let old = legacy();
    files::save_figure(&path, &old).unwrap();
    let original = std::fs::read(&path).unwrap();
    let old = files::load_figure(&path).unwrap();
    assert!(old.appearance.is_none() && old.palette_cycle.is_none());
    files::save_figure(&path, &old).unwrap();
    assert_eq!(original, std::fs::read(&path).unwrap());
}

#[test]
fn scripts_inline_and_companion_freeze_requested_theme_while_native_retains_intent() {
    let dir = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("plot([-0 1],[1 2]);hold on;scatter([0 1],[2 3]);stem([0 1],[3 4]);stairs([0 1],[4 5]);legend('a','b','c','d');").unwrap();
    let source = e.figure.unwrap();
    let snapshot = source.clone();
    for host in [BaseTheme::Light, BaseTheme::Dark] {
        for companion in [false, true] {
            let path = dir.path().join("scene.m");
            if companion {
                files::write_matlab_plot_bundle_with_theme(&path, &source, host, &stop).unwrap();
            } else {
                files::write_matlab_plot_script_with_theme(&path, &source, host, &stop).unwrap();
            }
            let mut replay = Engine::default();
            replay
                .execute_document(&std::fs::read_to_string(&path).unwrap(), &path)
                .unwrap();
            let result = replay.figure.unwrap();
            bytes_equal(&source, &result);
            assert_eq!(colors(&result, BaseTheme::Light), colors(&source, host));
            assert_eq!(colors(&result, BaseTheme::Dark), colors(&source, host));
            assert!(
                result
                    .series
                    .iter()
                    .all(|s| s.explicit_color && s.auto_palette.is_none())
            );
            assert!(source.same_snapshot(&snapshot));
        }
    }
    let path = dir.path().join("keep.m");
    std::fs::write(&path, b"sentinel").unwrap();
    stop.store(true, Ordering::Relaxed);
    assert!(
        files::write_matlab_plot_reconstruction_with_theme(&path, &source, BaseTheme::Dark, &stop)
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
    stop.store(false, Ordering::Relaxed);
    files::write_matlab_plot_reconstruction_with_theme(&path, &source, BaseTheme::Dark, &stop)
        .unwrap();
}

#[test]
fn static_fig_projects_light_rgb_and_unsupported_dark_save_keeps_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("static.fig");
    let stop = AtomicBool::new(false);
    let mut f = Figure::default();
    f.add(vec![point(), point()]).unwrap();
    let before = f.clone();
    files::save_matlab_figure_with_theme(&path, &f, BaseTheme::Light, &stop).unwrap();
    let decoded = files::load_matlab_figure(&path).unwrap();
    assert_eq!(colors(&decoded, BaseTheme::Dark), vec![LIGHT[0], LIGHT[1]]);
    bytes_equal(&f, &decoded);
    assert!(
        decoded
            .series
            .iter()
            .all(|s| s.explicit_color && s.auto_palette.is_none())
    );
    let bytes = std::fs::read(&path).unwrap();
    let mut unsupported = f.clone();
    unsupported.series[0].kind = PlotKind::Stairs; // Dark still obeys the structural subset.
    assert!(
        files::save_matlab_figure_with_theme(&path, &unsupported, BaseTheme::Dark, &stop).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(f.same_snapshot(&before));
}

#[cfg(feature = "plot-export")]
#[test]
fn native_intent_svg_png_and_manual_markers_agree_on_both_resolved_palettes() {
    use matrust::{export, plot::Canvas};
    let mut engine = Engine::default();
    engine
        .execute("plot([0 1],[1 1],'LineWidth',3);hold on;")
        .unwrap();
    for y in 2..=7 {
        engine
            .execute(&format!("plot([0 1],[{y} {y}],'LineWidth',3);"))
            .unwrap();
    }
    engine.execute("plot([0 1],[8 8],'mo-','LineWidth',3);axis([0 1 0 9]);legend('one','two','three','four','five','six','seven','manual');").unwrap();
    let figure = engine.figure.unwrap();
    let original = figure.clone();
    let cancel = AtomicBool::new(false);
    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join("palette.mfig");
    files::save_figure(&native, &figure).unwrap();
    let reopened = files::load_figure(&native).unwrap();
    assert_eq!(figure.appearance, reopened.appearance);
    assert_eq!(figure.color, reopened.color);
    assert!(figure.axes.same_metadata(&reopened.axes));
    bytes_equal(&figure, &reopened);
    for (host, palette) in [(BaseTheme::Light, LIGHT), (BaseTheme::Dark, DARK)] {
        let svg = reopened
            .svg_on_canvas_with_theme(
                Canvas {
                    width: 640.,
                    height: 640.,
                },
                host,
            )
            .unwrap();
        for color in palette.into_iter().chain([[255, 0, 255]]) {
            let hex = format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
            assert!(
                svg.matches(&hex).count() >= 2,
                "data and legend {host:?}: {hex}"
            );
        }
        let raster =
            export::render_figure_in_theme_with_cancel(&reopened, [640, 640], host, &cancel)
                .unwrap();
        let tree = resvg::usvg::Tree::from_str(&svg, &export::svg_render_options()).unwrap();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(640, 640).unwrap();
        resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
        assert_eq!(pixmap.data(), raster.as_raw());
        for color in palette.into_iter().chain([[255, 0, 255]]) {
            assert!(
                raster
                    .pixels()
                    .any(|pixel| pixel.0 == [color[0], color[1], color[2], 255]),
                "actual ink {host:?}: {color:?}"
            );
        }
        let png = directory.path().join("palette.png");
        export::export_figure_in_theme_with_cancel(&reopened, &png, [640, 640], host, &cancel)
            .unwrap();
        assert_eq!(
            image::open(&png).unwrap().to_rgba8().as_raw(),
            raster.as_raw()
        );
    }
    assert!(figure.same_snapshot(&original));
    for (before, after) in original.series.iter().zip(&figure.series) {
        assert!(before.points.same_storage(&after.points));
    }
}
