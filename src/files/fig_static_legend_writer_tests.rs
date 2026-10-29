use super::*;
use crate::plot::{BaseTheme, FigureTheme, LegendLocation, SnapshotStamp};
use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

thread_local! { static CANCEL_BEFORE_PUBLISH: Cell<bool> = const { Cell::new(false) }; }
pub(super) fn before_publish(cancel: &AtomicBool) {
    if CANCEL_BEFORE_PUBLISH.with(|flag| flag.replace(false)) {
        cancel.store(true, Ordering::Relaxed);
    }
}

fn source(duplicate: bool) -> crate::Engine {
    let mut engine = crate::Engine::default();
    engine.execute("h=plot([-0 1 2],[2 3 1],[-0 1 2],[4 1 5]);set(h(2),'Visible','off');grid on;xlabel('Time');ylabel('Value');set(gca,'XColor',[0.8 0.1 0.2],'YColor',[0.1 0.2 0.8],'GridAlpha',0.4);").unwrap();
    engine.execute(if duplicate {
        "lg=legend(h,{'same_\\label','same_\\label'},'Interpreter','none','AutoUpdate','off','Location','northeast');"
    } else {
        "lg=legend(h,{'First','Second'},'Interpreter','none','AutoUpdate','off','Location','northeast');"
    }).unwrap();
    assert!(
        !engine
            .figure
            .as_ref()
            .unwrap()
            .legend_state
            .as_ref()
            .unwrap()
            .auto_update
    );
    engine
}
fn assert_semantics(before: &Figure, after: &Figure) {
    assert!(after.legend && after.legend_location.is_default());
    assert!(!after.legend_state.as_ref().unwrap().auto_update);
    assert_eq!(
        after.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
        (0..before.series.len()).collect::<Vec<_>>()
    );
    assert_eq!(after.series.len(), before.series.len());
    for (a, b) in before.series.iter().zip(&after.series) {
        assert_eq!(
            (
                a.kind,
                &a.label,
                a.visible,
                before.resolve_appearance(BaseTheme::Light).series_color(a),
                a.pattern
            ),
            (b.kind, &b.label, b.visible, b.color, b.pattern)
        );
        // FIG and its reconstruction script retain resolved RGB, not auto intent.
        assert!(b.explicit_color && b.auto_palette.is_none());
        assert_eq!(a.points.len(), b.points.len());
        for (a, b) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
    let a = before
        .axes
        .resolve_colors(before.resolve_appearance(BaseTheme::Light));
    let b = after
        .axes
        .resolve_colors(after.resolve_appearance(BaseTheme::Light));
    assert_eq!(
        (
            a.x,
            a.y,
            a.xlabel,
            a.ylabel,
            a.grid_x,
            a.grid_y,
            a.grid_alpha
        ),
        (
            b.x,
            b.y,
            b.xlabel,
            b.ylabel,
            b.grid_x,
            b.grid_y,
            b.grid_alpha
        )
    );
}
fn assert_graph(bytes: &[u8], expected: &Figure) {
    let expanded = mat::expand(bytes).unwrap();
    let hierarchy = expanded.hierarchy().unwrap();
    let root = object(&hierarchy).unwrap();
    let siblings = children(root).unwrap();
    assert_eq!(siblings.len(), 2);
    assert_eq!(string(&siblings[0], "type", "").unwrap(), "axes");
    assert_eq!(string(&siblings[1], "type", "").unwrap(), "scribe.legend");
    let axes = &siblings[0];
    let legend = &siblings[1];
    let ap = object(field(axes, "properties").unwrap()).unwrap();
    let lp = object(field(legend, "properties").unwrap()).unwrap();
    assert_eq!(
        scalar(ap, "LegendPeerHandle", -1.).unwrap(),
        scalar(legend, "handle", -2.).unwrap()
    );
    assert_eq!(
        scalar(lp, "Axes", -1.).unwrap(),
        scalar(axes, "handle", -2.).unwrap()
    );
    assert_eq!(string(lp, "Location", "").unwrap(), "northeast");
    assert_eq!(string(lp, "Visible", "").unwrap(), "on");
    assert_eq!(string(lp, "Interpreter", "").unwrap(), "none");
    assert_eq!(string(lp, "Orientation", "").unwrap(), "vertical");
    let Value::Cell(labels) = field(lp, "String").unwrap() else {
        panic!("real MAT cell labels required")
    };
    assert_eq!(
        labels.iter().map(|v| v.text().unwrap()).collect::<Vec<_>>(),
        expected
            .series
            .iter()
            .map(|s| s.label.as_str())
            .collect::<Vec<_>>()
    );
    let application = object(field(lp, "ApplicationData").unwrap()).unwrap();
    assert_eq!(application.len(), 1);
    assert!(
        !application.contains_key("PeerAxes") && !application.contains_key("PostDeserializeFcn")
    );
    let members = field(application, "PlotChildren").unwrap();
    let Value::Number { count, .. } = members else {
        panic!("numeric member handles required")
    };
    assert_eq!(*count, expected.series.len());
    let data: Vec<_> = children(axes)
        .unwrap()
        .iter()
        .filter(|n| string(n, "type", "").unwrap() == "graph2d.lineseries")
        .collect();
    assert_eq!(data.len(), expected.series.len());
    let mut handles = vec![
        scalar(root, "handle", -1.).unwrap(),
        scalar(axes, "handle", -1.).unwrap(),
        scalar(legend, "handle", -1.).unwrap(),
    ];
    for (index, node) in children(axes).unwrap().iter().enumerate() {
        let handle = scalar(node, "handle", -1.).unwrap();
        assert!(handle.is_finite() && !handles.contains(&handle));
        handles.push(handle);
        if index < data.len() {
            assert_eq!(members.number(index), Some(handle));
        }
    }
    assert_eq!(
        validate_legend_membership(lp, &legend_series_handles(&[axes]).unwrap(), 0).unwrap(),
        Some((0..data.len()).collect())
    );
}

#[test]
fn public_static_legend_duplicate_labels_hidden_series_close_open_resave_and_script() {
    let dir = tempfile::tempdir().unwrap();
    for duplicate in [false, true] {
        let mut engine = source(duplicate);
        engine.set_file_directory(dir.path()).unwrap();
        let original = engine.figure.as_ref().unwrap().clone();
        let stamp = SnapshotStamp::new(&original);
        for round in 0..2 {
            let before = engine.figure.as_ref().unwrap().clone();
            engine.execute("savefig('static.fig');").unwrap();
            assert!(before.same_snapshot(engine.figure.as_ref().unwrap()));
            assert!(
                before.series[0]
                    .points
                    .same_storage(&engine.figure.as_ref().unwrap().series[0].points)
            );
            if round == 0 {
                assert!(stamp.matches(engine.figure.as_ref().unwrap()));
            }
            assert_graph(
                &std::fs::read(dir.path().join("static.fig")).unwrap(),
                &original,
            );
            engine.execute("close all;openfig('static.fig');").unwrap();
            let reopened = engine.figure.as_ref().unwrap();
            assert_semantics(&original, reopened);
            let script =
                crate::files::matlab_plot_script(reopened, &AtomicBool::new(false)).unwrap();
            let mut replay = crate::Engine::default();
            replay.execute(&script).unwrap();
            assert_semantics(reopened, replay.figure.as_ref().unwrap());
        }
        assert!(stamp.matches(&original));
    }
}

#[test]
fn unsupported_legend_states_and_pre_or_late_cancel_preserve_destination_then_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.fig");
    let source = source(false).figure.unwrap();
    let before = source.clone();
    for variant in 0..9 {
        let mut figure = source.clone();
        match variant {
            0 => figure.legend_state.as_mut().unwrap().auto_update = true,
            1 => figure.legend_state.as_mut().unwrap().members.reverse(),
            2 => {
                figure.legend_state.as_mut().unwrap().members.pop();
            }
            3 => figure.legend = false,
            4 => figure.legend_location = LegendLocation::parse("northwest").unwrap(),
            5 => figure.series[0].kind = PlotKind::Scatter,
            6 => figure.series[0].label.clear(),
            7 => {
                // Dark static legends are admitted; live membership remains unsupported.
                figure.appearance.as_mut().unwrap().theme = FigureTheme::Dark;
                figure.legend_state.as_mut().unwrap().auto_update = true;
            }
            8 => figure.legend_state = None,
            _ => unreachable!(),
        }
        std::fs::write(&path, b"original").unwrap();
        let snapshot = figure.clone();
        assert!(save_matlab_figure(&path, &figure).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(snapshot.same_snapshot(&figure));
        save_matlab_figure(&path, &source).unwrap();
        assert_semantics(&source, &load_matlab_figure(&path).unwrap());
    }
    for late in [false, true] {
        std::fs::write(&path, b"original").unwrap();
        let cancel = AtomicBool::new(!late);
        CANCEL_BEFORE_PUBLISH.with(|flag| flag.set(late));
        assert_eq!(
            save_matlab_figure_with_cancel(&path, &source, &cancel).unwrap_err(),
            "Cancelled"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        cancel.store(false, Ordering::Relaxed);
        save_matlab_figure_with_cancel(&path, &source, &cancel).unwrap();
        assert_semantics(&source, &load_matlab_figure(&path).unwrap());
    }
    assert!(before.same_snapshot(&source));
}

#[test]
fn literal_cell_encoder_retains_unicode_and_enforces_label_count_and_bytes() {
    let labels = vec!["λ_\\literal"; 64];
    let value = mat::legend_text_cells(&labels).unwrap();
    let bytes = mat::file(mat::structure("hgS_070000", vec![("labels", value)])).unwrap();
    let expanded = mat::expand(&bytes).unwrap();
    let hierarchy = expanded.hierarchy().unwrap();
    let Value::Cell(actual) = field(object(&hierarchy).unwrap(), "labels").unwrap() else {
        panic!("cell")
    };
    assert_eq!(
        actual.iter().map(|v| v.text().unwrap()).collect::<Vec<_>>(),
        labels
    );
    assert!(mat::legend_text_cells(&[]).is_err());
    assert!(mat::legend_text_cells(&vec!["label"; 65]).is_err());
    let long = "x".repeat(4097);
    assert!(mat::legend_text_cells(&[&long]).is_err());
}

#[test]
fn public_one_and_sixty_four_member_boundaries_and_subplot_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boundary.fig");
    for count in [1, 64] {
        let mut engine = crate::Engine::default();
        engine.execute("plot([0 1],[2 3]);hold on;").unwrap();
        for _ in 1..count {
            engine.execute("plot([0 1],[2 3]);").unwrap();
        }
        let labels = (0..count)
            .map(|index| format!("'Line {}'", index + 1))
            .collect::<Vec<_>>()
            .join(",");
        engine
            .execute(&format!(
                "legend({{{labels}}},'Interpreter','none','AutoUpdate','off');"
            ))
            .unwrap();
        let figure = engine.figure.as_ref().unwrap();
        assert_eq!(figure.series.len(), count);
        let before = figure.clone();
        save_matlab_figure(&path, figure).unwrap();
        assert_graph(&std::fs::read(&path).unwrap(), figure);
        assert_semantics(figure, &load_matlab_figure(&path).unwrap());
        assert!(before.same_snapshot(figure));
        for (before, after) in before.series.iter().zip(&figure.series) {
            assert!(before.points.same_storage(&after.points));
        }
    }
    let mut engine = crate::Engine::default();
    engine.execute("subplot(1,2,1);plot([0 1],[2 3]);legend('One','Interpreter','none','AutoUpdate','off');subplot(1,2,2);plot([0 1],[4 5]);").unwrap();
    let figure = engine.figure.as_ref().unwrap();
    assert!(figure.layout.is_some());
    let before = figure.clone();
    std::fs::write(&path, b"original subplot destination").unwrap();
    assert!(save_matlab_figure(&path, figure).is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"original subplot destination"
    );
    assert!(before.same_snapshot(figure));
}

#[test]
fn no_legend_plain_and_subplot_bytes_match_exact_predecessor_encoder() {
    for command in [
        "plot([-0 1],[2 3]);title('Plain');grid off;",
        "plot([-0 1],[2 3]);grid on;set(gca,'GridColor',[0.2 0.3 0.4],'GridAlpha',0.4);",
        "subplot(1,2,1);plot([0 1],[2 3]);subplot(1,2,2);plot([0 1],[4 5]);subplot(1,2,1);",
    ] {
        let mut engine = crate::Engine::default();
        engine.execute(command).unwrap();
        let figure = engine.figure.as_ref().unwrap();
        assert_eq!(
            encode_matlab_figure_with_theme(figure, BaseTheme::Light).unwrap(),
            predecessor_no_legend_encoder(figure, BaseTheme::Light).unwrap()
        );
        let mut legacy = figure.clone();
        legacy.appearance = None;
        for axes in legacy.all_axes_mut() {
            axes.colors = None;
        }
        assert_eq!(
            encode_matlab_figure_with_theme(&legacy, BaseTheme::Light).unwrap(),
            predecessor_no_legend_encoder(&legacy, BaseTheme::Light).unwrap()
        );
    }
}
// Exact graphics18 root encoder, changing only its name and the new None argument.
fn predecessor_no_legend_encoder(figure: &Figure, host: crate::plot::BaseTheme) -> Result<Vec<u8>> {
    figure.validate()?;
    if let Some(reason) = matlab_figure_export_restriction_with_theme(figure, host) {
        return Err(reason.into());
    }
    let appearance = figure.resolve_appearance(host);
    let mut panels: Vec<_> = figure.all_axes().collect();
    panels.sort_by_key(|(p, _)| *p);
    let scale = panels.iter().fold(1_f64, |s, (p, _)| {
        let [l, t, r, b] = p.rectangle();
        s.min(r - l).min(b - t)
    });
    let mut next_handle = if figure.layout.is_some() { 100. } else { 10. };
    let mut root_children = Vec::with_capacity(panels.len());
    let mut current = 0;
    for (i, (position, axes)) in panels.into_iter().enumerate() {
        let handle = i as f64 + 2.;
        if position == figure.active_position() {
            current = i;
        }
        let normalized = if figure.layout.is_some() {
            Some(position.normalized_plot_box().unwrap_or_else(|| {
                let [l, t, r, b] = position.rectangle();
                [
                    l + 70. * scale / 960.,
                    1. - b + 70. * scale / 560.,
                    r - l - 110. * scale / 960.,
                    b - t - 130. * scale / 560.,
                ]
            }))
        } else {
            None
        };
        root_children.push(encode_axes(
            axes,
            handle,
            &mut next_handle,
            normalized,
            appearance,
            None,
        )?);
    }
    let mut props = vec![
        ("Visible", mat::text("off")),
        (
            "Color",
            num(&appearance.canvas.map(|c| f64::from(c) / 255.)),
        ),
    ];
    if !figure.name.is_empty() {
        props.push(("Name", mat::text(&figure.name)));
    }
    // Keep the selected axes first for MatRust reopening. A live CurrentAxes
    // property must not be assigned before the child graphics objects exist.
    root_children.rotate_left(current);
    let root = node("figure", 1., props, root_children, &[]);
    mat::file(mat::structure("hgS_070000", root))
}

#[test]
fn automatic_legend_native_intent_and_static_rgb_keep_legacy_fallback_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let source = source(false).figure.unwrap();
    let before = source.clone();
    assert_eq!(source.axes.palette_cycle.unwrap().next_slot, 2);
    assert_eq!(source.series[0].auto_palette.unwrap().slot, 0);
    assert!(!source.series[0].explicit_color);
    assert_eq!(source.series[0].color, [0, 114, 189]);
    assert_eq!(
        source
            .resolve_appearance(BaseTheme::Light)
            .series_color(&source.series[0]),
        [17, 113, 190]
    );
    let bytes = crate::files::encode_native_figure(&source).unwrap();
    assert!(bytes.starts_with(b"MATRFIGV"));
    let native = crate::files::decode_native_figure(&bytes).unwrap();
    assert!(source.same_metadata(&native));
    assert_eq!(native.axes.palette_cycle.unwrap().next_slot, 2);
    for (original, reopened) in source.series.iter().zip(&native.series) {
        assert_eq!(original.auto_palette, reopened.auto_palette);
        assert_eq!(original.points.len(), reopened.points.len());
        for (a, b) in original.points.iter().zip(reopened.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
    let path = dir.path().join("modern.fig");
    save_matlab_figure(&path, &source).unwrap();
    let projected = load_matlab_figure(&path).unwrap();
    assert_eq!(projected.series[0].color, [17, 113, 190]);
    assert_semantics(&source, &projected);

    // An old document with no palette provenance keeps its stored legacy RGB.
    let mut legacy = source.clone();
    legacy.appearance = None;
    legacy.axes.palette_cycle = None;
    for series in &mut legacy.series {
        series.auto_palette = None;
    }
    assert!(!legacy.has_palette_metadata());
    let bytes = crate::files::encode_native_figure(&legacy).unwrap();
    assert!(bytes.starts_with(b"MATRFIGT")); // Explicit axes RGB remains present.
    assert!(legacy.same_metadata(&crate::files::decode_native_figure(&bytes).unwrap()));
    save_matlab_figure(&path, &legacy).unwrap();
    let projected = load_matlab_figure(&path).unwrap();
    assert_eq!(projected.series[0].color, [0, 114, 189]);
    assert_semantics(&legacy, &projected);
    assert!(source.same_snapshot(&before));
    for (a, b) in source.series.iter().zip(&before.series) {
        assert!(a.points.same_storage(&b.points));
    }
}
