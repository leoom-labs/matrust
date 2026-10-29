//! Included by the existing plot_script target; no additional integration target.
use super::*;

fn expression_budget_bar_scene(rows: usize, members: usize, relative: bool) -> Figure {
    use matrust::{Array, Matrix};
    use std::collections::BTreeMap;
    let mut x = Vec::with_capacity(rows * members);
    let mut y = Vec::with_capacity(rows * members);
    for column in 0..members {
        for row in 0..rows {
            x.push(1. + row as f64 * (1 + column % 3) as f64);
            y.push(if row == 0 && column == 0 {
                -0.
            } else if row == rows / 2 && column == 0 {
                f64::NAN
            } else {
                (row % 11) as f64 - 5. + column as f64 * 0.25
            });
        }
    }
    let mut engine = Engine::default();
    engine
        .merge_workspace(BTreeMap::from([
            (
                "x".into(),
                Array::Numeric(Matrix::from_column_vec(rows, members, x).unwrap()),
            ),
            (
                "y".into(),
                Array::Numeric(Matrix::from_column_vec(rows, members, y).unwrap()),
            ),
        ]))
        .unwrap();
    engine
        .execute(if relative {
            "bar(x,y,'GroupWidth',.5,'BarWidth',.25,'EdgeColor','none');"
        } else {
            "bar(x,y,'EdgeColor','none');"
        })
        .unwrap();
    let mut figure = engine.figure.unwrap();
    let (x, y) = figure.bounds();
    figure.xlim = Some(x);
    figure.ylim = Some(y);
    figure
}

#[test]
fn bar_inline_literals_respect_unchanged_positive_and_unary_node_boundaries() {
    // The matrix itself is one expression. Unary minus adds another per value.
    for (value, admitted, rejected) in [("1", 255, 256), ("-1", 127, 128)] {
        let statement = |count| format!("A=[{}];", vec![value; count].join(";"));
        matrust::syntax::parse(&statement(admitted)).unwrap();
        assert!(
            matrust::syntax::parse(&statement(rejected))
                .unwrap_err()
                .contains("256 expression nodes")
        );
    }
    let stop = AtomicBool::new(false);
    for rows in [119, 120, 121, 127, 128] {
        let source = expression_budget_bar_scene(rows, 1, false);
        let text = files::matlab_plot_script(&source, &stop).unwrap();
        assert_eq!(text.contains("mr_bar_values_1"), rows > 120);
        matrust::syntax::parse(&text).unwrap();
        let mut replay = Engine::default();
        replay.execute(&text).unwrap();
        assert_bar_scene(&source, replay.figure.as_ref().unwrap());
        assert!(
            !replay
                .workspace()
                .keys()
                .any(|name| name.starts_with("mr_bar_values_"))
        );
    }
}

#[test]
fn grouped_and_relative_bar_workload_sizes_replay_twice_through_real_files() {
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    // Preserve both actual failed workload boundaries and the largest published
    // harness workloads; neither parser limits nor benchmark sizes are reduced.
    for (case, (rows, members, relative)) in [
        (128, 2, false),
        (5000, 2, false),
        (4, 64, false),
        (128, 64, true),
    ]
    .into_iter()
    .enumerate()
    {
        let source = if members == 2 && !relative {
            let mut engine = Engine::default();
            engine
                .execute(&format!("bar(1:{rows},[ones({rows},1) 2*ones({rows},1)]);"))
                .unwrap();
            let mut figure = engine.figure.unwrap();
            let (x, y) = figure.bounds();
            figure.xlim = Some(x);
            figure.ylim = Some(y);
            figure
        } else {
            expression_budget_bar_scene(rows, members, relative)
        };
        let snapshot = source.clone();
        for companion in [false, true] {
            let mut figure = source.clone();
            for pass in 0..2 {
                let path = dir
                    .path()
                    .join(format!("bounded_bar_{case}_{companion}_{pass}.m"));
                if companion {
                    files::write_matlab_plot_bundle_with_cancel(&path, &figure, &stop).unwrap();
                } else {
                    files::write_matlab_plot_script_with_cancel(&path, &figure, &stop).unwrap();
                }
                let document = matrust::document::Document::open(&path).unwrap();
                assert!(document.text.contains("mr_bars = bar("));
                assert!(!document.text.contains("mr_h = patch("));
                assert_eq!(
                    document.text.contains("'GroupWidth',0.8,'BarWidth',1"),
                    !relative
                );
                let mut replay = Engine::default();
                replay
                    .execute_document(&document.text, document.path.as_deref().unwrap())
                    .unwrap();
                assert!(
                    !replay
                        .workspace()
                        .keys()
                        .any(|name| name.starts_with("mr_bar_values_"))
                );
                let actual = replay.figure.unwrap();
                assert_bar_scene(&figure, &actual);
                assert!(actual.baselines.is_empty());
                figure = actual;
            }
        }
        assert!(source.same_snapshot(&snapshot));
        for (a, b) in source.series.iter().zip(&snapshot.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
}

fn assert_bar_scene(expected: &Figure, actual: &Figure) {
    assert_eq!(expected.series.len(), actual.series.len());
    for (left, right) in expected.series.iter().zip(&actual.series) {
        assert_eq!(left.kind, right.kind);
        assert_eq!(left.points.len(), right.points.len());
        for (a, b) in left.points.iter().zip(right.points.iter()) {
            // Bit-exact, except that every NaN is one value (MATLAB's NaN is 0xfff8...).
            let bits = |v: f64| {
                if v.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    v.to_bits()
                }
            };
            assert_eq!(a.map(bits), b.map(bits));
        }
        // Portable reconstruction freezes the source's effective Light RGB;
        // its explicit result does not preserve automatic palette provenance.
        assert_eq!(
            expected
                .resolve_appearance(matrust::plot::BaseTheme::Light)
                .series_color(left),
            right.color
        );
        assert!(right.explicit_color);
        if left.explicit_color || left.auto_palette.is_none() {
            assert_eq!(left.color, right.color);
        }
        assert_eq!(left.face_edge_color, right.face_edge_color);
        assert_eq!(left.pattern, right.pattern);
        if left.kind == PlotKind::Bar {
            assert_eq!(
                left.face_edge_width().to_bits(),
                right.face_edge_width().to_bits()
            );
        }
        assert_eq!(left.visible, right.visible);
        assert_eq!(left.label, right.label);
        if left.kind == PlotKind::Bar {
            assert_eq!(left.bar_width.to_bits(), right.bar_width.to_bits());
            assert_eq!(left.bar_offset.to_bits(), right.bar_offset.to_bits());
            // Geometry equality includes the original ordered edge operations.
            for (a, b) in left.points.iter().zip(right.points.iter()) {
                let edges = |x: f64, s: &Series| {
                    [
                        (x + s.bar_offset) - s.bar_width / 2.,
                        (x + s.bar_offset) + s.bar_width / 2.,
                    ]
                };
                assert_eq!(
                    edges(a[0], left).map(f64::to_bits),
                    edges(b[0], right).map(f64::to_bits)
                );
            }
        }
    }
    let bars: Vec<_> = expected
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == PlotKind::Bar)
        .map(|(i, _)| i)
        .collect();
    for &a in &bars {
        let left = expected.observe_series_baseline(a).unwrap();
        let right = actual.observe_series_baseline(a).unwrap();
        assert_eq!(left.style.visible, right.style.visible);
        assert_eq!(
            left.style.color.map(f64::to_bits),
            right.style.color.map(f64::to_bits)
        );
        assert_eq!(
            left.style.width_points.to_bits(),
            right.style.width_points.to_bits()
        );
        assert_eq!(left.style.pattern, right.style.pattern);
        for &b in &bars {
            assert_eq!(
                left.identity
                    .same_object(expected.observe_series_baseline(b).unwrap().identity),
                right
                    .identity
                    .same_object(actual.observe_series_baseline(b).unwrap().identity)
            );
        }
    }
    assert_eq!(expected.svg(), actual.svg());
}

#[test]
fn ordinary_bar_shapes_keep_real_chart_groups_through_two_disk_replays() {
    let cases = [
        "bar([1 3 7],[2 NaN -0]);",
        "bar([1;3;7],[2;NaN;-0]);",
        "bar([2 3;4 5;6 7]);",
        "bar([1 3 7],[2 3;4 5;6 7]);",
        "bar([1 3 7],[2 4 6;3 5 7]);",
        "bar([1 2 3;4 5 6],[2 7]);",
        "bar([1 3 7;2 5 9],[2 4 6]);",
        "bar([1 2;3 5;7 9],[2 3;4 5;6 7]);",
        "bar(zeros(0,2));",
        "bar(ones(2,64));",
        "bar([1 3 7],[2 4 8]);set(gca,'XScale','log','YScale','log');",
        "bar(2);",
        "bar(5,2);",
        "bar([1 3],[2 4],[5 9],[7 8]);",
        "bar([-100 -10 -1],[-2 -4 -8]);set(gca,'XScale','log','YScale','log');",
    ];
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    for (case, command) in cases.iter().enumerate() {
        let mut engine = Engine::default();
        engine.execute(command).unwrap();
        let mut source = engine.figure.unwrap();
        let (x, y) = source.bounds();
        source.xlim = Some(x);
        source.ylim = Some(y);
        let original = source.clone();
        for companion in [false, true] {
            let mut figure = source.clone();
            for pass in 0..2 {
                let path = dir
                    .path()
                    .join(format!("batch_{case}_{companion}_{pass}.m"));
                if companion {
                    files::write_matlab_plot_bundle_with_cancel(&path, &figure, &stop).unwrap();
                } else {
                    files::write_matlab_plot_script_with_cancel(&path, &figure, &stop).unwrap();
                }
                let doc = matrust::document::Document::open(&path).unwrap();
                assert!(doc.text.contains("'GroupWidth',0.8,'BarWidth',1"));
                assert!(doc.text.contains("MATLAB R2024a+"));
                let mut replay = Engine::default();
                replay
                    .execute_document(&doc.text, doc.path.as_deref().unwrap())
                    .unwrap();
                let restored = replay.figure.unwrap();
                assert_bar_scene(&figure, &restored);
                assert!(
                    restored.baselines.is_empty(),
                    "implicit default must not materialize"
                );
                figure = restored;
            }
        }
        assert!(source.same_snapshot(&original));
        for (a, b) in source.series.iter().zip(&original.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
}

#[test]
fn genuine_bar_group_replay_keeps_mixed_order_styles_and_linked_baseline_edits() {
    let mut engine = Engine::default();
    engine.execute("a=gca;hold(a,'on');l=plot(a,[1 3],[7 8]);h=bar(a,[1 3 7],[2 3;4 5;6 7],'FaceColor',[.2 .4 .6],'EdgeColor',[.7 .2 .1],'LineStyle','--','LineWidth',2);set(h(2),'Visible','off','DisplayName','second');s=scatter(a,[1 3],[3 4]);j=bar(a,[11 15],[9 10]);b=get(h(1),'BaseLine');set(b,'Visible','on','Color',[.1 .3 .7],'LineWidth',1.25,'LineStyle',':','BaseValue',-2);legend(a,[j l h(2)],{'last bar','line','hidden bar'},'AutoUpdate','off');").unwrap();
    let mut figure = engine.figure.unwrap();
    let (x, y) = figure.bounds();
    figure.xlim = Some(x);
    figure.ylim = Some(y);
    let source = figure.clone();
    let before = source.clone();
    let dir = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    for pass in 0..2 {
        let path = dir.path().join(format!("mixed_{pass}.m"));
        files::write_matlab_plot_bundle_with_cancel(&path, &figure, &stop).unwrap();
        let doc = matrust::document::Document::open(&path).unwrap();
        let mut replay = Engine::default();
        replay.execute_document(&doc.text, &path).unwrap();
        let restored = replay.figure.unwrap();
        assert_bar_scene(&figure, &restored);
        assert_eq!(restored.baselines.len(), 1);
        assert_eq!(restored.baselines[0].members.len(), 3);
        assert_eq!(
            restored
                .legend_members()
                .map(|(i, s)| (i, s.label.as_str()))
                .collect::<Vec<_>>(),
            figure
                .legend_members()
                .map(|(i, s)| (i, s.label.as_str()))
                .collect::<Vec<_>>()
        );
        figure = restored;
    }
    assert!(source.same_snapshot(&before));
    assert!(
        source.series[1]
            .points
            .same_storage(&before.series[1].points)
    );
    // Only a separate replay engine changes. The original source remains shared.
    assert_eq!(
        before.series[1].bar_width.to_bits(),
        figure.series[1].bar_width.to_bits()
    );
}

#[test]
fn maximum_bar_count_and_noncanonical_nan_bits_use_the_companion_without_copies() {
    let stop = AtomicBool::new(false);
    let mut engine = Engine::default();
    engine
        .execute("bar(1:5000,[ones(5000,1) 2*ones(5000,1)]);hold on;plot(1:30000,ones(1,30000));")
        .unwrap();
    let mut figure = engine.figure.unwrap();
    let mut samples = figure.series[0].points.materialize(&stop).unwrap();
    samples[12][1] = f64::from_bits(0x7ff8000000001234);
    figure.series[0].points = samples.into();
    let (x, y) = figure.bounds();
    figure.xlim = Some(x);
    figure.ylim = Some(y);
    let source = figure.clone();
    let alias = source.clone();
    let dir = tempfile::tempdir().unwrap();
    for pass in 0..2 {
        let path = dir.path().join(format!("large_{pass}.m"));
        let saved =
            files::write_matlab_plot_reconstruction_with_cancel(&path, &figure, &stop).unwrap();
        assert!(saved.is_some());
        let doc = matrust::document::Document::open(&path).unwrap();
        assert!(doc.text.len() <= 1_000_000);
        let mut replay = Engine::default();
        replay.execute_document(&doc.text, &path).unwrap();
        let restored = replay.figure.unwrap();
        assert_bar_scene(&figure, &restored);
        assert_eq!(
            restored
                .series
                .iter()
                .filter(|s| s.kind == PlotKind::Bar)
                .map(|s| s.points.len())
                .sum::<usize>(),
            10_000
        );
        figure = restored;
    }
    assert!(source.same_snapshot(&alias));
    assert!(
        source.series[0]
            .points
            .same_storage(&alias.series[0].points)
    );
    assert_eq!(
        source.series[0].points.get(12).unwrap()[1].to_bits(),
        0x7ff8000000001234
    );
}

#[test]
fn relative_bar_properties_are_public_bounded_atomic_and_keep_legacy_construction() {
    let mut plain = Engine::default();
    let mut explicit = Engine::default();
    let data = "[1 3 7],[2 3;4 5;6 7]";
    plain.execute(&format!("h=bar({data});")).unwrap();
    explicit
        .execute(&format!("h=bar({data},'GroupWidth',.8,'BarWidth',1);"))
        .unwrap();
    for (a, b) in plain
        .figure
        .as_ref()
        .unwrap()
        .series
        .iter()
        .zip(&explicit.figure.as_ref().unwrap().series)
    {
        assert_eq!(a.points, b.points);
        assert_eq!(a.bar_width.to_bits(), b.bar_width.to_bits());
        assert_eq!(a.bar_offset.to_bits(), b.bar_offset.to_bits());
    }
    let before = explicit.figure.clone().unwrap();
    for command in [
        "bar([1 2],[3 4],'GroupWidth',NaN);",
        "bar([1 2],[3 4],'BarWidth',2);",
        "bar([1 2],[3 4],'GroupWidth',0);",
        "bar([1 2],[3 4],'BarWidth',.5);",
        "set(h(1),'GroupWidth',.5);",
    ] {
        assert!(explicit.execute(command).is_err(), "{command}");
        assert!(explicit.figure.as_ref().unwrap().same_snapshot(&before));
    }
    explicit
        .execute("bar([1 3],[2 3;4 5],'GroupWidth',.5,'BarWidth',.5);")
        .unwrap();
    let series = &explicit.figure.as_ref().unwrap().series;
    assert_eq!(series[0].bar_width.to_bits(), 0.25f64.to_bits());
    assert_eq!(series[0].bar_offset.to_bits(), (-0.25f64).to_bits());
    assert_eq!(series[1].bar_offset.to_bits(), 0.25f64.to_bits());
    explicit
        .execute("bar([1 3],[2 3;4 5],'GroupWidth',.5);")
        .unwrap();
    let series = &explicit.figure.as_ref().unwrap().series;
    assert_eq!(series[0].bar_width.to_bits(), (0.5f64 * 0.8).to_bits());
    assert_eq!(series[0].bar_offset.to_bits(), (-0.25f64).to_bits());
    assert_eq!(series[1].bar_offset.to_bits(), 0.25f64.to_bits());
}

#[test]
fn explicit_default_bar_record_survives_replay_while_observation_and_idempotent_sets_stay_pure() {
    let mut engine = Engine::default();
    engine.execute("h=bar([1 3],[2 4]);").unwrap();
    let original = engine.figure.clone().unwrap();
    engine
        .execute("b=get(h,'BaseLine');set(b,'Visible','on');")
        .unwrap();
    assert!(engine.figure.as_ref().unwrap().same_snapshot(&original));
    assert!(engine.figure.as_ref().unwrap().baselines.is_empty());
    engine
        .execute("set(b,'LineWidth',1.25);set(b,'LineWidth',.75,'Visible','on');")
        .unwrap();
    let mut figure = engine.figure.unwrap();
    let (x, y) = figure.bounds();
    figure.xlim = Some(x);
    figure.ylim = Some(y);
    assert_eq!(figure.baselines.len(), 1);
    let dir = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    for pass in 0..2 {
        let path = dir.path().join(format!("explicit_default_{pass}.m"));
        files::write_matlab_plot_bundle_with_cancel(&path, &figure, &stop).unwrap();
        let document = matrust::document::Document::open(&path).unwrap();
        let mut replay = Engine::default();
        replay.execute_document(&document.text, &path).unwrap();
        let restored = replay.figure.unwrap();
        assert_bar_scene(&figure, &restored);
        assert_eq!(restored.baselines.len(), 1);
        assert!(matches!(
            restored.baselines[0].value,
            matrust::plot::BaselineValue::Automatic
        ));
        figure = restored;
    }
}

#[test]
fn arbitrary_geometry_keeps_patch_route_and_unsupported_linked_records_preserve_destination() {
    let mut engine = Engine::default();
    engine.execute("h=bar([1 3],[2 4]);").unwrap();
    let mut figure = engine.figure.clone().unwrap();
    figure.series[0].bar_offset = 0.125;
    // Historical arbitrary fill-only Bar geometry intentionally uses Patch replay.
    figure.series[0].bar_baseline_visible = false;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.m");
    let stop = AtomicBool::new(false);
    files::write_matlab_plot_reconstruction_with_cancel(&path, &figure, &stop).unwrap();
    let document = matrust::document::Document::open(&path).unwrap();
    assert!(document.text.contains("patch("));
    assert!(!document.text.contains("mr_bars = bar("));
    let saved = std::fs::read(&path).unwrap();
    engine
        .execute("b=get(h,'BaseLine');set(b,'Visible','on');")
        .unwrap();
    let mut unsupported = engine.figure.unwrap();
    unsupported.series[0].bar_offset = 0.125;
    let before = unsupported.clone();
    assert!(
        files::write_matlab_plot_reconstruction_with_cancel(&path, &unsupported, &stop).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(unsupported.same_snapshot(&before));
    assert!(
        files::write_matlab_plot_reconstruction_with_cancel(&path, &figure, &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved);
}

#[test]
fn new_bar_defaults_and_explicit_off_survive_group_add_delete_and_two_script_routes() {
    let stop = AtomicBool::new(false);
    let directory = tempfile::tempdir().unwrap();
    for legacy in [false, true] {
        let mut engine = Engine::default();
        engine
            .execute("b=bar([1 3 7],[2 3;4 NaN;-0 5]);hold on;c=bar([1 3 7],[6 7 8]);")
            .unwrap();
        let mut figure = engine.figure.unwrap();
        if legacy {
            for series in &mut figure.series {
                series.bar_baseline_visible = false;
            }
        }
        // The exporter spells colors explicitly; isolate baseline topology from that existing normalization.
        for series in &mut figure.series {
            series.explicit_color = true;
        }
        let before = figure.clone();
        let token = figure.observe_series_baseline(0).unwrap().identity.clone();
        assert_eq!(
            figure.observe_baseline(&token).unwrap().style.visible,
            !legacy
        );
        assert!(figure.baselines.is_empty());
        // Different order and deletion retain the same genuine linked identity.
        figure.series.swap(0, 1);
        figure.remove_series(2).unwrap();
        assert!(figure.observe_baseline(&token).is_ok());
        // Restore creation order for the recognized grouped reconstruction recipe.
        // Arbitrary reordered geometry remains an explicit MFIG-only case.
        figure.series.swap(0, 1);
        for companion in [false, true] {
            let mut source = figure.clone();
            let (x, y) = source.bounds();
            source.xlim = Some(x);
            source.ylim = Some(y);
            for pass in 0..2 {
                let path = directory
                    .path()
                    .join(format!("policy_{legacy}_{companion}_{pass}.m"));
                if companion {
                    files::write_matlab_plot_bundle_with_cancel(&path, &source, &stop).unwrap();
                } else {
                    files::write_matlab_plot_script_with_cancel(&path, &source, &stop).unwrap();
                }
                let doc = matrust::document::Document::open(&path).unwrap();
                assert!(doc.text.contains("mr_bars = bar("));
                let mut replay = Engine::default();
                replay.execute_document(&doc.text, &path).unwrap();
                let reopened = replay.figure.unwrap();
                assert_bar_scene(&source, &reopened);
                assert_eq!(
                    reopened.observe_series_baseline(0).unwrap().style.visible,
                    !legacy
                );
                assert!(reopened.baselines.is_empty());
                // The third admitted series was deleted without rewinding the
                // source cursor. Public .m creates two fresh explicit Bar objects.
                assert_eq!(source.series.len(), 2);
                assert_eq!(
                    source.axes.palette_cycle.unwrap().next_slot,
                    if pass == 0 { 3 } else { 2 }
                );
                assert_eq!(reopened.axes.palette_cycle.unwrap().next_slot, 2);
                let mut expected_static_axes = source.axes.clone();
                expected_static_axes
                    .palette_cycle
                    .as_mut()
                    .unwrap()
                    .next_slot = 2;
                for (index, (expected, actual)) in expected_static_axes
                    .series
                    .iter()
                    .zip(&reopened.series)
                    .enumerate()
                {
                    assert!(expected.explicit_color && actual.explicit_color);
                    assert_eq!(
                        source
                            .resolve_appearance(matrust::plot::BaseTheme::Light)
                            .series_color(expected),
                        actual.color
                    );
                    // The public Bar constructor admits a slot; the subsequent
                    // explicit FaceColor set retains it on both replay cycles.
                    assert_eq!(
                        expected.auto_palette,
                        Some(matrust::plot::AutoPalette {
                            version: matrust::plot::PaletteVersion::MatRustModern1,
                            slot: index as u8,
                        })
                    );
                    assert_eq!(actual.auto_palette, expected.auto_palette);
                    for host in [
                        matrust::plot::BaseTheme::Light,
                        matrust::plot::BaseTheme::Dark,
                    ] {
                        assert_eq!(
                            reopened.resolve_appearance(host).series_color(actual),
                            actual.color
                        );
                    }
                }
                // Only the deleted source cursor is projected; latent palette
                // intent and all other axes/series/baseline metadata are compared.
                assert!(expected_static_axes.same_metadata(&reopened.axes));
                source = reopened;
            }
        }
        for series in &figure.series {
            assert!(
                before
                    .series
                    .iter()
                    .any(|old| old.points.same_storage(&series.points))
            );
        }
    }
}

#[test]
fn nondefault_relative_bar_factors_keep_exact_groups_through_both_disk_routes_twice() {
    let cases = [
        "bar([1 3 7],[2 3;4 5;6 7],'GroupWidth',.5,'BarWidth',.25);",
        "bar([1 2;3 5;7 9],[2 3;4 5;6 7],'GroupWidth',.75,'BarWidth',.5);",
        "bar([1 3 7],[2 -0 6],'GroupWidth',1,'BarWidth',.3);",
        "bar([1 3 7],[2 3 4;4 5 6;6 7 8],'GroupWidth',.7,'BarWidth',.6);",
        "a=gca;hold(a,'on');plot(a,[1 2],[8 9]);h=bar(a,[1 3 7],[2 3;4 5;6 7],'GroupWidth',.5,'BarWidth',.25,'EdgeColor',[.7 .2 .1],'LineStyle','--','LineWidth',2);set(h(2),'Visible','off','DisplayName','second');scatter(a,[1 2],[3 4]);bar(a,[11 15],[9 10],'GroupWidth',.5,'BarWidth',.5);b=get(h(1),'BaseLine');set(b,'Color',[.2 .3 .4],'LineWidth',1.25,'Visible','off');",
    ];
    let stop = AtomicBool::new(false);
    let directory = tempfile::tempdir().unwrap();
    for (case, command) in cases.iter().enumerate() {
        let mut engine = Engine::default();
        engine.execute(command).unwrap();
        let mut original = engine.figure.unwrap();
        let (x, y) = original.bounds();
        original.xlim = Some(x);
        original.ylim = Some(y);
        let alias = original.clone();
        for companion in [false, true] {
            let mut source = original.clone();
            for pass in 0..2 {
                let path = directory
                    .path()
                    .join(format!("relative_{case}_{companion}_{pass}.m"));
                if companion {
                    files::write_matlab_plot_bundle_with_cancel(&path, &source, &stop).unwrap();
                } else {
                    files::write_matlab_plot_script_with_cancel(&path, &source, &stop).unwrap();
                }
                let document = matrust::document::Document::open(&path).unwrap();
                assert!(document.text.contains("mr_bars = bar("));
                assert!(!document.text.contains("mr_h = patch("));
                let mut engine = Engine::default();
                engine
                    .execute_document(&document.text, document.path.as_deref().unwrap())
                    .unwrap();
                let restored = engine.figure.unwrap();
                assert_bar_scene(&source, &restored);
                source = restored;
            }
        }
        assert!(original.same_snapshot(&alias));
        for (left, right) in original.series.iter().zip(&alias.series) {
            assert!(left.points.same_storage(&right.points));
        }
    }
}

#[test]
fn recovered_relative_bar_recipe_preserves_raw_nan_payload_in_real_companion_twice() {
    let stop = AtomicBool::new(false);
    let mut engine = Engine::default();
    engine
        .execute("bar([1 2;3 5;7 9],[2 3;4 5;6 7],'GroupWidth',.5,'BarWidth',.25);")
        .unwrap();
    let mut source = engine.figure.unwrap();
    let mut samples = source.series[0].points.materialize(&stop).unwrap();
    samples[1][1] = f64::from_bits(0x7ff8_0123_4567_0001);
    source.series[0].points = samples.into();
    let (x, y) = source.bounds();
    source.xlim = Some(x);
    source.ylim = Some(y);
    let original = source.clone();
    let alias = source.clone();
    let directory = tempfile::tempdir().unwrap();
    for pass in 0..2 {
        let path = directory.path().join(format!("relative_nan_{pass}.m"));
        assert!(
            files::write_matlab_plot_reconstruction_with_cancel(&path, &source, &stop)
                .unwrap()
                .is_some()
        );
        let document = matrust::document::Document::open(&path).unwrap();
        let mut replay = Engine::default();
        replay.execute_document(&document.text, &path).unwrap();
        let restored = replay.figure.unwrap();
        assert_bar_scene(&source, &restored);
        assert_eq!(
            restored.series[0].points.get(1).unwrap()[1].to_bits(),
            0x7ff8_0123_4567_0001
        );
        source = restored;
    }
    assert!(original.same_snapshot(&alias));
    assert!(
        original.series[0]
            .points
            .same_storage(&alias.series[0].points)
    );
}

#[test]
fn unrepresentable_visible_bar_geometry_and_cancel_leave_existing_destination_untouched() {
    let stop = AtomicBool::new(false);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preserved.m");
    std::fs::write(&path, b"existing script\n").unwrap();
    let mut engine = Engine::default();
    engine.execute("bar([0 1],[2 3]);").unwrap();
    let source = engine.figure.unwrap();
    for (width, offset) in [
        (2., 0.),
        (0.25, 0.125),
        (f64::from_bits(1), f64::from_bits(1)),
    ] {
        let mut figure = source.clone();
        figure.series[0].bar_width = width;
        figure.series[0].bar_offset = offset;
        let before = figure.clone();
        for companion in [false, true] {
            let result = if companion {
                files::write_matlab_plot_bundle_with_cancel(&path, &figure, &stop).map(|_| ())
            } else {
                files::write_matlab_plot_script_with_cancel(&path, &figure, &stop)
            };
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"existing script\n");
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
            assert!(figure.same_snapshot(&before));
        }
    }
    let mut relative = Engine::default();
    relative
        .execute("bar([1 3],[2 3;4 5],'GroupWidth',.5,'BarWidth',.25);")
        .unwrap();
    let figure = relative.figure.unwrap();
    let before = figure.clone();
    assert!(
        files::write_matlab_plot_bundle_with_cancel(&path, &figure, &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"existing script\n");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert!(figure.same_snapshot(&before));
}
