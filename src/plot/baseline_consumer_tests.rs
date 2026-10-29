use super::*;
use crate::plot::{Figure, Series};
use std::sync::Arc;
fn scene() -> Figure {
    let mut f = Figure {
        axes: crate::plot::Axes {
            series: vec![
                Series::new(vec![[1., 2.], [2., 4.]], PlotKind::Bar),
                Series::new(vec![[1., 3.], [2., 5.]], PlotKind::Stem),
            ],
            ..crate::plot::Axes::default()
        },
        ..Figure::default()
    };
    let b = Baseline::new(
        BaselineValue::Value(-3.),
        BaselineStyle {
            color: [0.15, 0.25, 0.35],
            pattern: LinePattern::DashDot,
            width_points: 1.125,
            visible: true,
        },
        f.series.iter().map(|s| s.identity.clone()).collect(),
    )
    .unwrap();
    f.replace_baselines(vec![b], &AtomicBool::new(false))
        .unwrap();
    f
}
#[test]
fn prepared_stem_bounds_svg_and_cache_follow_shared_metadata_without_sample_changes() {
    let mut f = scene();
    let old = f.clone();
    let stamp = crate::plot::SnapshotStamp::new(&f);
    let (axes, (_, y)) = f.display_axes_cancellable(&AtomicBool::new(false)).unwrap();
    assert_eq!(y, [-3., 5.]);
    let points = f
        .display_series_points_cancellable(1, 1024, axes, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(points[0], [1., -3.]);
    assert_eq!(points[1], [1., 3.]);
    let svg = f.svg();
    assert_eq!(svg.matches("data-matrust-baseline").count(), 1);
    assert!(svg.contains("stroke=\"rgb(15%,25%,35%)\""));
    assert!(svg.contains("stroke-dasharray=\"8 4 2 4\""));
    assert!(
        !svg.contains("stroke=\"#777777\""),
        "Explicit record suppresses legacy baseline path; axes border is independent"
    );
    assert!(stamp.matches(&f));
    f.baselines[0].style.visible = false;
    assert!(!stamp.matches(&f));
    assert!(!f.svg().contains("data-matrust-baseline"));
    assert!(!f.svg().contains("stroke=\"#777777\""));
    assert_eq!(
        f.bounds().1,
        [-3., 5.],
        "hidden stroke still owns the member geometry base"
    );
    for (a, b) in old.series.iter().zip(&f.series) {
        assert!(a.points.same_storage(&b.points));
    }
    assert!(
        f.display_series_points_cancellable(1, 1024, axes, &AtomicBool::new(true))
            .is_err()
    );
}
#[test]
fn distinct_equal_standalone_records_draw_independently_and_log_geometry_uses_resolved_base() {
    let mut f = scene();
    f.series[0].points = Arc::new(vec![[1., -2.], [2., -4.]]).into();
    f.series[1].points = Arc::new(vec![[1., -3.], [2., -5.]]).into();
    f.yscale = AxisScale::Log;
    f.baselines[0].value = BaselineValue::Automatic;
    let mut standalone =
        Baseline::new(BaselineValue::Value(-1.), f.baselines[0].style, vec![]).unwrap();
    standalone.style.pattern = LinePattern::Dotted;
    f.baselines.push(standalone);
    f.validate().unwrap();
    let axes = f.axis_transforms();
    assert!(axes[1].negative);
    assert_eq!(f.effective_baseline(0, axes[1]).unwrap(), -1.);
    let geometry = f.display_series_points(1, 1024, axes);
    assert_eq!(geometry[0][1].to_bits(), axes[1].forward(-1.).to_bits());
    assert_eq!(f.svg().matches("data-matrust-baseline").count(), 2);
    assert_eq!(f.series.len(), 2);
    assert_eq!(f.legend_entries().count(), 0);
}
fn metadata(bytes: &[u8]) -> serde_json::Value {
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    serde_json::from_slice(&bytes[12..12 + n]).unwrap()
}
fn replace_metadata(bytes: &[u8], value: &serde_json::Value) -> Vec<u8> {
    let old = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let json = serde_json::to_vec(value).unwrap();
    let mut out = bytes[..8].to_vec();
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend(json);
    out.extend_from_slice(&bytes[12 + old..]);
    out
}
#[test]
fn native_p_q_preserve_exact_rgb_signed_zero_and_membership_reject_downgrade_and_corruption() {
    let mut f = scene();
    f.appearance = None; // Exercise the pre-theme native capability.
    f.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in f.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    f.baselines[0].value = BaselineValue::Value(-0.);
    f.baselines[0].style.color[1] = -0.;
    f.series[0].face_edge_color = Some([4, 5, 6]);
    let bytes = crate::files::encode_native_figure(&f).unwrap();
    assert!(bytes.starts_with(b"MATRFIGP"));
    let got = crate::files::decode_native_figure(&bytes).unwrap();
    assert!(f.same_metadata(&got));
    assert!(
        !got.baselines[0]
            .identity
            .same_object(&f.baselines[0].identity)
    );
    assert!(got.baselines[0].members[0].same_object(&got.series[0].identity));
    assert!(got.baselines[0].members[1].same_object(&got.series[1].identity));
    assert_eq!(
        got.baselines[0].style.color.map(f64::to_bits),
        f.baselines[0].style.color.map(f64::to_bits)
    );
    assert_eq!(
        got.effective_baseline(0, AxisTransform::default())
            .unwrap()
            .to_bits(),
        (-0f64).to_bits()
    );
    for header in [b"MATRFIGL", b"MATRFIGJ", b"MATRFIGH"] {
        let mut bad = bytes.clone();
        bad[..8].copy_from_slice(header);
        assert!(crate::files::decode_native_figure(&bad).is_err());
    }
    for members in [
        serde_json::json!([0, 0]),
        serde_json::json!([0, 2]),
        serde_json::json!(vec![0; 65]),
    ] {
        let mut m = metadata(&bytes);
        m["baselines"][0]["members"] = members;
        assert!(crate::files::decode_native_figure(&replace_metadata(&bytes, &m)).is_err());
    }
    let mut m = metadata(&bytes);
    m["baselines"][0]["color"] = serde_json::json!([0.15, 1.01, 0.]);
    assert!(crate::files::decode_native_figure(&replace_metadata(&bytes, &m)).is_err());
    let left_axes = f.axes.clone();
    f.select_subplot(crate::plot::Position::from_grid(1, 2, &[1]).unwrap(), false)
        .unwrap();
    f.axes = left_axes;
    f.select_subplot(crate::plot::Position::from_grid(1, 2, &[2]).unwrap(), false)
        .unwrap();
    let bytes = crate::files::encode_native_figure(&f).unwrap();
    assert!(bytes.starts_with(b"MATRFIGQ"));
    let got = crate::files::decode_native_figure(&bytes).unwrap();
    assert!(f.same_metadata(&got));
    let mut bad = bytes.clone();
    bad[..8].copy_from_slice(b"MATRFIGM");
    assert!(crate::files::decode_native_figure(&bad).is_err());
}
#[test]
fn session_checkpoint_restores_shared_baseline_membership_and_reuses_unchanged_snapshot() {
    use crate::session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, restore_figures},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let f = scene();
    let shared = f.series[0].points.clone();
    let session = Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace_manifest: None,
        workspace: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: Some(FigureSession {
            current: 1,
            figures: vec![FigureDocument {
                window: None,
                id: 1,
                figure: f.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: None,
            }],
        }),
        layout: Default::default(),
        version: 1,
        documents: vec![crate::document::Document::new()],
        active: 0,
        folder: dir.path().to_path_buf(),
        recent: vec![],
        history: vec![],
        model: Default::default(),
        model_path: None,
        model_baseline: None,
    };
    let mut store = SessionStore::new(&path);
    store.save(session.clone()).unwrap();
    let first = Session::load(&path).unwrap().figure_manifest.unwrap();
    store.save(session).unwrap();
    let saved = Session::load(&path).unwrap();
    assert_eq!(
        first.entries[0].file,
        saved.figure_manifest.as_ref().unwrap().entries[0].file
    );
    let result = restore_figures(
        &path,
        saved.figure_manifest.as_ref().unwrap(),
        &AtomicBool::new(false),
    );
    assert!(result.warnings.is_empty());
    let restored = &result.figures.figures[0].figure;
    assert!(f.same_metadata(restored));
    assert_eq!(f.svg(), restored.svg());
    for (index, member) in restored.baselines[0].members.iter().enumerate() {
        assert!(member.same_object(&restored.series[index].identity));
        assert!(!member.same_object(&f.series[index].identity));
    }
    assert!(shared.same_storage(&f.series[0].points));
}

#[test]
fn incomplete_external_routes_reject_only_new_metadata_before_destination_publication() {
    let f = scene();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("baseline.m");
    std::fs::write(&script, b"original").unwrap();
    let stop = AtomicBool::new(false);
    assert!(crate::files::write_matlab_plot_script_with_cancel(&script, &f, &stop).is_err());
    assert_eq!(std::fs::read(&script).unwrap(), b"original");
    assert!(crate::files::write_matlab_plot_bundle_with_cancel(&script, &f, &stop).is_err());
    assert_eq!(std::fs::read(&script).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(crate::files::matlab_figure_export_restriction(&f).is_some());
    let mut legacy = f.clone();
    legacy.baselines.clear();
    assert!(crate::files::matlab_plot_script(&legacy, &stop).is_ok());
    // The historic bare-series logarithmic omission path must remain infallible.
    legacy.series.remove(0);
    legacy.series[0].base_value = Some(-2.);
    legacy.yscale = AxisScale::Log;
    legacy.ylim = Some([1., 10.]);
    let transform = legacy.axis_transforms();
    assert_eq!(legacy.render_baseline(0, transform[1]).unwrap(), -2.);
    assert!(!legacy.svg().is_empty());
}

#[test]
fn legacy_stem_default_is_shared_explicit_policy_without_materializing_records() {
    let style = BaselineStyle::legacy_stem();
    assert_eq!(style.display_rgb(), [119; 3]);
    assert_eq!(style.display_width(), 1.);
    assert_eq!(style.width_points.to_bits(), 0.75f64.to_bits());
    assert_eq!(style.pattern, LinePattern::Solid);
    assert!(style.visible);
    let mut f = Figure::default();
    f.add(vec![
        Series::new(vec![[1., 2.], [2., 4.]], PlotKind::Stem),
        Series::new(vec![[1., 3.], [2., 5.]], PlotKind::Stem),
    ])
    .unwrap();
    f.xlim = Some([0., 3.]);
    f.ylim = Some([-1., 6.]);
    let before = f.clone();
    let svg = f.svg();
    let paths: Vec<_> = svg
        .split("<path ")
        .skip(1)
        .map(|s| s.split("/>").next().unwrap())
        .filter(|s| s.contains("stroke=\"#777777\""))
        .collect();
    assert_eq!(
        paths.len(),
        1,
        "Legacy equal-coordinate de-duplication is unchanged"
    );
    assert!(paths[0].contains("stroke-width=\"1\""));
    assert!(paths[0].contains('H'));
    assert!(
        svg.contains("stroke=\"#777\""),
        "Axes border remains independent"
    );
    assert!(f.baselines.is_empty());
    assert!(f.same_snapshot(&before));
    for (a, b) in f.series.iter().zip(&before.series) {
        assert!(a.points.same_storage(&b.points));
    }
    for s in &mut f.series {
        s.visible = false;
    }
    assert!(!f.svg().contains("stroke=\"#777777\""));
}

fn compact_baseline_scene() -> Figure {
    let mut engine = crate::Engine::default();
    engine.execute("x=[1;2;3];y=[2;-1;4];a=x-.25;b=x+.25;c=y*0;patch('XData',[a b b a].','YData',[c c y y].','FaceColor',[.2 .4 .6],'EdgeColor',[.6 .2 .1],'LineStyle','--','LineWidth',1.25);hold on;s=stem([1 2],[3 4],'Color',[.1 .3 .5],'LineWidth',1.5,'Marker','o','MarkerSize',6,'MarkerFaceColor','none');bl=get(s,'BaseLine');set(bl,'Color',[.125 .25 .5],'LineWidth',1.5,'LineStyle',':','BaseValue',-0);set(gca,'XLim',[0 4],'YLim',[-2 5]);").unwrap();
    let figure = engine.figure.unwrap();
    assert!(figure.series[0].points.patch().is_some());
    assert_eq!(figure.baselines.len(), 1);
    figure
}
fn equal_storage_values(a: &Figure, b: &Figure) {
    assert!(a.same_metadata(b));
    assert_eq!(
        a.geometry_ledger().unwrap().cost(),
        b.geometry_ledger().unwrap().cost()
    );
    for ((position, a), (other, b)) in a.all_axes().zip(b.all_axes()) {
        assert_eq!(position, other);
        for (a, b) in a.series.iter().zip(&b.series) {
            assert_eq!(a.points.len(), b.points.len());
            assert_eq!(a.points.patch().is_some(), b.points.patch().is_some());
            for (p, q) in a.points.iter().zip(b.points.iter()) {
                assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
            }
        }
    }
}
#[test]
fn combined_compact_outlined_and_baseline_p_q_roundtrips_keep_all_capabilities() {
    let mut figure = compact_baseline_scene();
    figure.appearance = None; // Exercise the pre-theme native capability.
    figure.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
    for axes in figure.all_axes_mut() {
        axes.palette_cycle = None;
        for series in &mut axes.series {
            series.auto_palette = None;
        }
    }
    let original = figure.clone();
    let stamp = crate::plot::SnapshotStamp::new(&figure);
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    assert!(bytes.starts_with(b"MATRFIGP"));
    let m = metadata(&bytes);
    assert!(m["series"][0]["coordinates"].is_object());
    assert!(m["series"][0]["face_edge_color"].is_array());
    assert!(m["latent_baselines"].is_array());
    let mut restored = crate::files::decode_native_figure(&bytes).unwrap();
    equal_storage_values(&figure, &restored);
    assert_eq!(figure.svg(), restored.svg());
    for lower in [b"MATRFIGN", b"MATRFIGL", b"MATRFIGJ"] {
        let mut downgraded = bytes.clone();
        downgraded[..8].copy_from_slice(lower);
        assert!(crate::files::decode_native_figure(&downgraded).is_err());
    }
    restored.baselines[0].style.visible = false;
    assert!(!stamp.matches(&restored));
    assert!(stamp.matches(&figure));
    // Q contains both an N compact child and a P baseline child. Prefix
    // geometry accounting must remain shared across all decoded children.
    figure.baselines.clear();
    let left_axes = figure.axes.clone();
    figure
        .select_subplot(crate::plot::Position::from_grid(1, 2, &[1]).unwrap(), false)
        .unwrap();
    figure.axes = left_axes;
    figure
        .select_subplot(crate::plot::Position::from_grid(1, 2, &[2]).unwrap(), false)
        .unwrap();
    let second = compact_baseline_scene();
    assert_eq!(second.axes.palette_cycle.unwrap().next_slot, 2);
    assert!(
        second
            .series
            .iter()
            .all(|s| s.explicit_color && s.auto_palette.is_none())
    );
    figure.axes = second.axes;
    // Even manual admissions retain their modern cursor: the mixed container
    // must use W before this historical Q fixture explicitly removes that intent.
    let modern = figure.clone();
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    assert!(bytes.starts_with(b"MATRFIGW"));
    let reopened = crate::files::decode_native_figure(&bytes).unwrap();
    equal_storage_values(&figure, &reopened);
    assert_eq!(reopened.axes.palette_cycle.unwrap().next_slot, 2);
    assert_eq!(figure.svg(), reopened.svg());
    assert!(figure.same_snapshot(&modern));
    for (a, b) in figure.series.iter().zip(&modern.series) {
        assert!(a.points.same_storage(&b.points));
    }
    figure.axes.palette_cycle = None;
    assert!(!figure.has_palette_metadata());
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    assert!(bytes.starts_with(b"MATRFIGQ"));
    let restored = crate::files::decode_native_figure(&bytes).unwrap();
    equal_storage_values(&figure, &restored);
    assert_eq!(figure.svg(), restored.svg());
    for lower in [b"MATRFIGO", b"MATRFIGM", b"MATRFIGK"] {
        let mut downgraded = bytes.clone();
        downgraded[..8].copy_from_slice(lower);
        assert!(crate::files::decode_native_figure(&downgraded).is_err());
    }
    assert!(
        original.series[0]
            .points
            .same_storage(&figure.layout.as_ref().unwrap().others[0].axes.series[0].points)
    );
}
#[test]
fn combined_compact_and_stem_baseline_real_files_replay_inline_and_companion_twice() {
    let source = compact_baseline_scene();
    let before = source.clone();
    let cancel = AtomicBool::new(false);
    let directory = tempfile::tempdir().unwrap();
    for companion in [false, true] {
        let mut current = source.clone();
        for pass in 0..2 {
            let path = directory
                .path()
                .join(format!("combined_{companion}_{pass}.m"));
            if companion {
                crate::files::write_matlab_plot_bundle_with_cancel(&path, &current, &cancel)
                    .unwrap();
            } else {
                crate::files::write_matlab_plot_script_with_cancel(&path, &current, &cancel)
                    .unwrap();
            }
            let document = crate::document::Document::open(&path).unwrap();
            let mut engine = crate::Engine::default();
            engine
                .execute_document(&document.text, document.path.as_deref().unwrap())
                .unwrap();
            let next = engine.figure.unwrap();
            equal_storage_values(&current, &next);
            assert_eq!(current.svg(), next.svg());
            assert_eq!(
                next.baselines[0].style.color.map(f64::to_bits),
                [0.125f64, 0.25, 0.5].map(f64::to_bits)
            );
            assert!(
                next.baselines[0]
                    .identity
                    .same_object(&next.series[1].baseline_identity)
            );
            current = next;
        }
    }
    assert!(source.same_snapshot(&before));
    for (a, b) in source.series.iter().zip(&before.series) {
        assert!(a.points.same_storage(&b.points));
    }
}

#[test]
fn q_mixed_compact_children_reject_cumulative_capacity_before_second_payload() {
    // Three initialized scalar values per source; unused declared capacity is
    // real retained storage, but does not require giant serialized fixtures.
    for baseline_children in [[false, true], [true, false], [true, true]] {
        let mut panels = Figure {
            appearance: None,
            axes: crate::plot::Axes::default(),
            ..Figure::default()
        };
        for (index, baseline) in baseline_children.into_iter().enumerate() {
            panels
                .select_subplot(
                    crate::plot::Position::from_grid(1, 2, &[index + 1]).unwrap(),
                    false,
                )
                .unwrap();
            let mut child = compact_baseline_scene();
            child.axes.palette_cycle = None;
            for series in &mut child.series {
                series.auto_palette = None;
            }
            if !baseline {
                child.baselines.clear();
            }
            panels.axes = child.axes;
        }
        let small = crate::files::encode_native_figure(&panels).unwrap();
        assert!(small.starts_with(b"MATRFIGQ"));
        let ordinary = crate::files::decode_native_figure(&small).unwrap();
        equal_storage_values(&panels, &ordinary);
        let outer_size = u32::from_le_bytes(small[8..12].try_into().unwrap()) as usize;
        let mut cursor = 12 + outer_size;
        let mut children = Vec::new();
        for baseline in baseline_children {
            let size = u32::from_le_bytes(small[cursor..cursor + 4].try_into().unwrap()) as usize;
            cursor += 4;
            let bytes = &small[cursor..cursor + size];
            cursor += size;
            assert!(bytes.starts_with(if baseline { b"MATRFIGP" } else { b"MATRFIGN" }));
            let mut meta = metadata(bytes);
            let sources = meta["series"][0]["coordinates"]["sources"]
                .as_array_mut()
                .unwrap();
            assert_eq!(sources.len(), 2);
            for source in sources {
                assert_eq!(source["len"].as_u64(), Some(3));
                source["capacity"] = serde_json::json!(600_000);
            }
            let grown = replace_metadata(bytes, &meta);
            // A standalone decoder must accept each exact child, including
            // its P metadata, and restore its actual unused capacity charge.
            let child = crate::files::decode_native_figure(&grown).unwrap();
            let cost = child.geometry_ledger().unwrap().cost();
            assert_eq!(cost.retained_bytes(), 9_600_032);
            assert_eq!(cost.source_units(), 600_002);
            assert_eq!(child.baselines.len(), usize::from(baseline));
            assert_eq!(child.series[0].points.len(), 12);
            assert!(child.series[0].points.patch().is_some());
            drop(child); // At most one standalone 9.6 MB capacity is live here.
            children.push(grown);
        }
        assert_eq!(cursor, small.len());
        let assemble = |second: &[u8]| {
            let mut bytes = small[..12 + outer_size].to_vec();
            for child in [children[0].as_slice(), second] {
                bytes.extend_from_slice(&(child.len() as u32).to_le_bytes());
                bytes.extend_from_slice(child);
            }
            bytes
        };
        let aggregate = assemble(&children[1]);
        assert!(
            aggregate.len() < 32 * 1024,
            "Capacity claims must not expand the wire fixture"
        );
        assert!(
            crate::files::decode_native_figure(&aggregate)
                .unwrap_err()
                .contains("resource")
        );
        // Error precedence demonstrates prefix rejection before processing the
        // second payload: that same truncated child alone reports truncation.
        let truncated = &children[1][..children[1].len() - 8];
        assert!(
            crate::files::decode_native_figure(truncated)
                .unwrap_err()
                .contains("truncated")
        );
        assert!(
            crate::files::decode_native_figure(&assemble(truncated))
                .unwrap_err()
                .contains("resource")
        );
        assert!(panels.same_metadata(&ordinary));
    }
}
