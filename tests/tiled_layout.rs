use matrust::{
    Engine, files,
    plot::{Arrangement, Figure, TileGrid},
};

#[test]
fn tiles_track_occupancy_spans_selection_and_overlap_replacement() {
    let mut e = Engine::default();
    e.execute("tiledlayout(2,3);sgtitle('Overview');").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 0);
    e.execute("nexttile([1 2]);plot([2 5]);title('Wide');")
        .unwrap();
    let points = e.figure.as_ref().unwrap().series[0].points.clone();
    e.execute("nexttile;title('Empty but occupied');nexttile;plot([7 9]);nexttile(1);")
        .unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.all_axes().count(), 3);
    assert_eq!(f.title, "Wide");
    assert!((points).same_storage(&f.series[0].points));
    let grid = f.layout.as_ref().unwrap().tiled.unwrap();
    assert_eq!(grid.cell(f.active_position()).unwrap(), (1, [1, 2]));
    e.execute("nexttile(1,[1 2]);").unwrap();
    assert!((points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    e.execute("nexttile(2);").unwrap(); // Interior of the wide axes replaces it.
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.all_axes().count(), 3);
    assert_eq!(f.point_count(), 2);
    assert!(f.series.is_empty());
    assert_eq!(f.layout.as_ref().unwrap().title, "Overview");
    e.execute("nexttile;plot([8 6]);").unwrap(); // First hole is tile 1.
    assert_eq!(
        grid.cell(e.figure.as_ref().unwrap().active_position())
            .unwrap()
            .0,
        1
    );
}

#[test]
fn growing_layouts_reflow_without_copying_samples_and_preserve_spans() {
    for kind in ["flow", "vertical", "horizontal"] {
        let mut e = Engine::default();
        e.execute(&format!("tiledlayout('{kind}');nexttile;plot([1 3]);"))
            .unwrap();
        let points = e.figure.as_ref().unwrap().series[0].points.clone();
        e.execute("for k=2:8;nexttile;plot([k k+1]);end;").unwrap();
        let f = e.figure.as_ref().unwrap();
        assert_eq!(f.all_axes().count(), 8);
        assert!(
            f.all_axes()
                .any(|(_, a)| (points).same_storage(&a.series[0].points))
        );
        let g = f.layout.as_ref().unwrap().tiled.unwrap();
        if kind == "flow" {
            assert_eq!((g.rows, g.columns), (3, 3));
        }
        if kind == "vertical" {
            assert_eq!((g.rows, g.columns), (8, 1));
        }
        if kind == "horizontal" {
            assert_eq!((g.rows, g.columns), (1, 8));
        }
        f.validate().unwrap();
    }
    let mut e = Engine::default();
    e.execute("nexttile;plot([1 2]);nexttile([2 3]);plot([3 4]);nexttile;")
        .unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.all_axes().count(), 3);
    let g = f.layout.as_ref().unwrap().tiled.unwrap();
    assert!(f.all_axes().any(|(p, _)| g.cell(p).unwrap().1 == [2, 3]));
}

#[test]
fn rejected_tile_changes_are_atomic_and_limits_apply_to_all_axes() {
    let mut e = Engine::default();
    e.execute("tiledlayout(2,2);nexttile;plot([1 2]);").unwrap();
    for source in [
        "h=tiledlayout(1,1)",
        "h=nexttile(2)",
        "tiledlayout(0,2)",
        "tiledlayout(17,1)",
        "tiledlayout('bad')",
        "tiledlayout(2,2,'TileSpacing','compact')",
        "nexttile(0)",
        "nexttile(5)",
        "nexttile(2,[1 2])",
        "nexttile([0 2])",
        "nexttile([1 2 3])",
        "nexttile(1.5)",
        "sgtitle(4)",
    ] {
        let old = e.figure.clone().unwrap();
        assert!(e.execute(source).is_err(), "{source}");
        assert!(e.figure.as_ref().unwrap().same_snapshot(&old), "{source}");
    }
    e.execute("nexttile;nexttile;nexttile;").unwrap();
    let old = e.figure.clone().unwrap();
    assert!(e.execute("nexttile;").is_err());
    assert!(old.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute("tiledlayout(16,16);for k=1:16;nexttile;end;")
        .unwrap();
    let old = e.figure.clone().unwrap();
    assert!(e.execute("nexttile;").is_err());
    assert!(old.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute("tiledlayout(2,1);nexttile;plot(1:999999);nexttile;")
        .unwrap();
    let old = e.figure.clone().unwrap();
    for source in ["plot([1 2]);", "freqz(1,1,4);"] {
        assert!(e.execute(source).is_err());
        assert!(old.same_snapshot(e.figure.as_ref().unwrap()));
    }
}

#[test]
fn native_files_preserve_empty_layout_title_and_continuation_and_reject_bad_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tiled.mfig");
    let mut e = Engine::default();
    for (script, header) in [
        ("tiledlayout(2,3);sgtitle('Empty <&>');", b"MATRFIG7"),
        (include_str!("../examples/tiled_layout.m"), b"MATRFIGK"),
    ] {
        e.execute(script).unwrap();
        e.figure.as_mut().unwrap().appearance = None; // Historical native capability.
        {
            let historical = e.figure.as_mut().unwrap();
            historical.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
            for axes in historical.all_axes_mut() {
                axes.palette_cycle = None;
                for series in &mut axes.series {
                    series.auto_palette = None;
                }
            }
        }
        let f = e.figure.as_ref().unwrap();
        files::save_figure(&path, f).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], header);
        assert_eq!(
            f.all_axes().any(|(_, axes)| axes.legend_state.is_some()),
            !f.empty_layout()
        );
        let mut loaded = files::load_figure(&path).unwrap();
        assert_eq!(loaded.svg(), f.svg());
        for ((p, a), (q, b)) in f.all_axes().zip(loaded.all_axes()) {
            assert_eq!(p, q);
            assert!(a.same_metadata(b));
        }
        assert_eq!(
            loaded.layout.as_ref().unwrap().tiled,
            f.layout.as_ref().unwrap().tiled
        );
        if f.empty_layout() {
            assert_eq!(loaded.all_axes().count(), 0);
            loaded.next_tile(None, None).unwrap();
            assert_eq!(loaded.all_axes().count(), 1);
        } else {
            let mut downgraded = bytes.clone();
            downgraded[..8].copy_from_slice(b"MATRFIG7");
            std::fs::write(&path, downgraded).unwrap();
            assert_eq!(
                files::load_figure(&path).unwrap_err(),
                "Advanced subplot metadata requires MFIGK"
            );
            loaded.next_tile(Some(3), None).unwrap();
            assert_eq!(loaded.title, "Discrete filter");
            for version in [b"MATRFIG6", b"MATRFIG7"] {
                let mut nested = bytes.clone();
                let size = u32::from_le_bytes(nested[8..12].try_into().unwrap()) as usize;
                nested[16 + size..24 + size].copy_from_slice(version);
                std::fs::write(&path, nested).unwrap();
                assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
            }
        }
        let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let original: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
        for field in ["rows", "columns", "pending_first"] {
            let mut metadata = original.clone();
            metadata["tiled"][field] = if field == "pending_first" {
                serde_json::json!(!f.empty_layout())
            } else {
                serde_json::json!(0)
            };
            let json = serde_json::to_vec(&metadata).unwrap();
            // Keep the correct envelope so rejection exercises corrupt tiled
            // metadata rather than merely the advanced-version guard.
            let mut bad = bytes[..8].to_vec();
            bad.extend_from_slice(&(json.len() as u32).to_le_bytes());
            bad.extend(json);
            bad.extend_from_slice(&bytes[12 + size..]);
            std::fs::write(&path, bad).unwrap();
            assert!(files::load_figure(&path).is_err());
        }
    }
}

#[test]
fn implicit_axes_creation_shared_title_and_subplot_switch_do_not_hide_data() {
    let mut e = Engine::default();
    for command in [
        "plot([1 2])",
        "freqz(1,1,4)",
        "impz(1,1,4)",
        "title('First')",
        "hold on",
    ] {
        e.execute(&format!("tiledlayout(2,1);{command};")).unwrap();
        assert!(!e.figure.as_ref().unwrap().empty_layout());
        assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 1);
        e.execute("nexttile;").unwrap();
        assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 2);
    }
    e.execute("clf;plot([2 4]);sgtitle('One figure');").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().point_count(), 2);
    assert!(e.figure.as_ref().unwrap().svg().contains("One figure"));
    e.execute("tiledlayout(2,2);nexttile;plot([1 2]);nexttile;subplot(2,2,1);")
        .unwrap();
    assert!(
        e.figure
            .as_ref()
            .unwrap()
            .layout
            .as_ref()
            .unwrap()
            .tiled
            .is_some()
    );
    e.execute("subplot(3,1,3);").unwrap();
    assert!(
        e.figure
            .as_ref()
            .unwrap()
            .layout
            .as_ref()
            .unwrap()
            .tiled
            .is_none()
    );
    let mut f = Figure::tiled_layout(TileGrid::new(Arrangement::Fixed, 2, 2).unwrap()).unwrap();
    f.title = "Hidden".into();
    assert!(f.validate().is_err());
}

#[test]
fn shared_title_targets_live_figure_without_selecting_or_copying_payloads() {
    let mut e = Engine::default();
    e.execute("f=figure(1);alias=f;subplot(1,2,1);a=gca;h=plot([1 2],[3 4]);subplot(1,2,2);plot([1 2],[5 6]);figure(2);plot([8 9]);").unwrap();
    let old: Vec<_> = e.figures().map(|(id, f)| (id, f.clone())).collect();
    let handle = e.array("f").unwrap();
    e.execute("sgtitle(f,'literal _ <&>','Interpreter','none');")
        .unwrap();
    assert_eq!(e.current_figure, 2);
    assert!(
        e.figure
            .as_ref()
            .unwrap()
            .same_snapshot(&old.iter().find(|(id, _)| *id == 2).unwrap().1)
    );
    let target = e.figures().find(|(id, _)| *id == 1).unwrap().1;
    let before = &old.iter().find(|(id, _)| *id == 1).unwrap().1;
    assert!(target.identity.same_object(&before.identity));
    assert_eq!(target.active_position(), before.active_position());
    assert_eq!(target.layout.as_ref().unwrap().title, "literal _ <&>");
    for ((_, a), (_, b)) in target.all_axes().zip(before.all_axes()) {
        assert!(a.identity.same_object(&b.identity));
        assert!((a.series[0].points).same_storage(&b.series[0].points));
    }
    assert!(handle.same_snapshot(&e.array("alias").unwrap()));
    for invalid in [
        "sgtitle(a,'wrong target');",
        "sgtitle(f,'lost','Interpreter','tex');",
        "sgtitle(f,'lost','FontSize',20);",
        "sgtitle(f,'lost','Interpreter');",
        "out=sgtitle(f,'unsupported output');",
    ] {
        let all: Vec<_> = e.figures().map(|(id, f)| (id, f.clone())).collect();
        assert!(e.execute(invalid).is_err(), "{invalid}");
        assert_eq!(e.current_figure, 2);
        for (id, f) in all {
            assert!(f.same_snapshot(e.figures().find(|(n, _)| *n == id).unwrap().1));
        }
    }
    e.execute("close(f);figure(1);plot([10 20]);figure(2);")
        .unwrap();
    let all: Vec<_> = e.figures().map(|(id, f)| (id, f.clone())).collect();
    assert!(e.execute("sgtitle(alias,'stale');").is_err());
    for (id, f) in all {
        assert!(f.same_snapshot(e.figures().find(|(n, _)| *n == id).unwrap().1));
    }
    assert_eq!(e.current_figure, 2);
}
