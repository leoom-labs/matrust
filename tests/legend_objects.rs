use matrust::{
    Array, Engine, files,
    plot::{LegendState, PlotKind, Series},
};
use std::sync::atomic::AtomicBool;
const SETUP: &str = "f=figure(1);a=gca;h=plot(a,[0 1],[1 2],[0 1],[2 3],[0 1],[3 4]);hold(a,'on');";
fn entries(e: &Engine) -> Vec<String> {
    e.figure
        .as_ref()
        .unwrap()
        .legend_entries()
        .map(|(_, s)| s.label.clone())
        .collect()
}
fn text(e: &Engine, name: &str) -> String {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!()
    };
    c.to_utf8(&AtomicBool::new(false)).unwrap()
}

#[test]
fn explicit_subset_cell_labels_aliases_and_queries_retain_order_and_shared_samples() {
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    let before = e.figure.clone().unwrap();
    e.execute("C={h(3),h(1)};L={'third','first'};lg=legend(a,[C{:}],L,'Interpreter','none','Location','southwest','AutoUpdate','off');alias=lg;holder={lg};again=holder{1};kind=get(lg,'Type');labels=get(lg,'String');first=labels{1};").unwrap();
    assert_eq!(entries(&e), ["third", "first"]);
    assert_eq!(text(&e, "kind"), "legend");
    assert_eq!(text(&e, "first"), "third");
    for n in ["alias", "again"] {
        let (Array::Graphics(a), Array::Graphics(b)) =
            (e.array("lg").unwrap(), e.array(n).unwrap())
        else {
            panic!()
        };
        assert!(
            a.scalar_handle()
                .unwrap()
                .same_object(b.scalar_handle().unwrap())
        );
    }
    for (a, b) in before.series.iter().zip(&e.figure.as_ref().unwrap().series) {
        assert!((a.points).same_storage(&b.points));
    }
    e.execute("figure(2);set(lg,'String',{'THIRD','FIRST'},'Visible','off');visible=get(alias,'Visible');").unwrap();
    assert_eq!(e.current_figure, 2);
    assert_eq!(text(&e, "visible"), "off");
    let f = e.figures().find(|(id, _)| *id == 1).unwrap().1;
    assert_eq!(
        f.legend_entries()
            .map(|(_, s)| s.label.as_str())
            .collect::<Vec<_>>(),
        ["THIRD", "FIRST"]
    );
}

#[test]
fn legend_autoupdate_empty_labels_reorder_remove_and_hide_off_have_real_identity() {
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("lg=legend(a,[h(3) h(1)],{'third',''},'Interpreter','none','AutoUpdate','off');q=plot(a,[0 1],[4 5],'DisplayName','fourth');").unwrap();
    assert_eq!(entries(&e), ["third"]);
    e.execute("set(lg,'AutoUpdate','on');r=plot(a,[0 1],[5 6],'DisplayName','fifth');legend(a,'hide');alive=isgraphics(lg);").unwrap();
    assert!(!e.figure.as_ref().unwrap().legend);
    assert_eq!(e.logical_variables["alive"].data(), [1]);
    assert_eq!(entries(&e), ["third", "fifth"]);
    let f = e.figure.as_mut().unwrap();
    f.series.swap(0, 2);
    f.validate().unwrap();
    assert_eq!(
        f.legend_entries().map(|(i, _)| i).collect::<Vec<_>>(),
        [0, 4]
    );
    f.remove_series(0).unwrap();
    f.validate().unwrap();
    assert_eq!(entries(&e), ["fifth"]);
    e.execute("legend(a,'show');legend(a,'off');dead=isgraphics(lg);fresh=legend(a,'show');")
        .unwrap();
    assert_eq!(e.logical_variables["dead"].data(), [0]);
    assert!(e.execute("set(lg,'Visible','on');").is_err());
    e.execute("hold(a,'off');plot(a,[0 1],[8 9]);stale=isgraphics(fresh);")
        .unwrap();
    assert_eq!(e.logical_variables["stale"].data(), [0]);
}

#[test]
fn legend_invalid_members_properties_and_labels_fail_without_publication() {
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("lg=legend(a,[h(3) h(1)],{'third','first'},'Interpreter','none');figure(2);b=gca;q=plot(b,[0 1],[7 8]);").unwrap();
    let before: Vec<_> = e.figures().map(|(n, f)| (n, f.clone())).collect();
    let alias = e.array("lg").unwrap();
    for script in [
        "legend(a,[h(1) q],{'x','y'},'Interpreter','none');",
        "legend(a,[h(1) h(1)],{'x','y'},'Interpreter','none');",
        "legend(a,h(1),{'x','y'},'Interpreter','none');",
        "legend(a,h(1),{1},'Interpreter','none');",
        "set(lg,'Visible','off','Interpreter','tex');",
        "set(lg,'String',{'changed'},'Visible','off');",
        "set(lg,'Box','off');",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(e.current_figure, 2);
        for (n, f) in &before {
            assert!(f.same_snapshot(e.figures().find(|(id, _)| id == n).unwrap().1));
        }
        assert!(alias.same_snapshot(&e.array("lg").unwrap()));
    }
}

#[test]
fn explicit_legend_native_roundtrip_svg_and_matlab_reexport_preserve_membership() {
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute(
        "lg=legend(a,[h(3) h(1)],{'third','first'},'Interpreter','none','AutoUpdate','off');",
    )
    .unwrap();
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
    let original = e.figure.as_ref().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legend.mfig");
    files::save_figure(&path, original).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[..8], b"MATRFIGJ");
    assert!(original.axes.same_metadata(&loaded.axes));
    assert_eq!(original.svg(), loaded.svg());
    assert!(
        !original
            .legend_state
            .as_ref()
            .unwrap()
            .identity
            .same_object(&loaded.legend_state.as_ref().unwrap().identity)
    );
    assert_eq!(
        loaded.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
        [2, 0]
    );
    let svg = loaded.svg();
    assert!(svg.find(">third</text>").unwrap() < svg.find(">first</text>").unwrap());
    assert!(!svg.contains(">data2</text>"));
    let script = files::matlab_plot_script(original, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("mr_legend_indices = [3 1];"));
    assert!(script.contains("'AutoUpdate','off'"));
    // The actual exported legend grammar executes through normal cells/CSL and
    // runtime dispatch. This does not claim the entire exporter already runs.
    e.execute("mr_series={h(1),h(2),h(3)};mr_labels={'first','second','third'};mr_legend_indices=[3 1];legend(a,[mr_series{mr_legend_indices}],mr_labels(mr_legend_indices),'Interpreter','none','Location','northeast','Visible','on','AutoUpdate','off');").unwrap();
    assert_eq!(entries(&e), ["third", "first"]);
    assert!(
        files::save_figure(
            &directory.path().join("unsupported.fig"),
            e.figure.as_ref().unwrap()
        )
        .is_err()
    );
}

#[test]
fn legend_metadata_bounds_duplicates_and_empty_subset_are_explicit() {
    let mut e = Engine::default();
    e.execute(SETUP).unwrap();
    e.execute("lg=legend(a,[],{},'Interpreter','none','AutoUpdate','off');")
        .unwrap();
    assert!(entries(&e).is_empty());
    assert!(e.figure.as_ref().unwrap().legend_state.is_some());
    let f = e.figure.as_mut().unwrap();
    let id = f.series[0].identity.clone();
    f.legend_state = Some(LegendState {
        identity: matrust::ObjectIdentity::default(),
        members: vec![id.clone(); 65],
        auto_update: false,
    });
    assert!(f.validate().unwrap_err().contains("64"));
    f.legend_state.as_mut().unwrap().members = vec![id.clone(), id];
    assert!(f.validate().unwrap_err().contains("duplicate"));
    f.legend_state.as_mut().unwrap().members = vec![matrust::ObjectIdentity::default()];
    assert!(f.validate().unwrap_err().contains("belong"));
    f.legend_state.as_mut().unwrap().members.clear();
    f.legend_state.as_mut().unwrap().identity = f.identity.clone();
    assert!(f.validate().unwrap_err().contains("identity"));
}

#[test]
fn automatic_legend_membership_is_bounded_at_maximum_and_samples_stay_shared() {
    let mut axes = matrust::plot::Axes::default();
    let base = Series::new(vec![[0., 1.]], PlotKind::Line);
    let shared = base.points.clone();
    let make = || {
        let mut s = Series::new(Vec::new(), PlotKind::Line);
        s.points = shared.clone();
        s
    };
    for _ in 0..63 {
        axes.add(vec![make()]).unwrap();
    }
    axes.legend_state = Some(LegendState {
        identity: matrust::ObjectIdentity::default(),
        members: axes.series.iter().map(|s| s.identity.clone()).collect(),
        auto_update: true,
    });
    axes.add(vec![make()]).unwrap();
    axes.validate().unwrap();
    assert_eq!(axes.legend_members().count(), 64);
    assert!(axes.add(vec![base]).is_err());
    assert_eq!(axes.legend_members().count(), 64);
    for s in &axes.series {
        assert!((shared).same_storage(&s.points));
    }
}

#[test]
fn subplot_legend_objects_use_advanced_envelope_and_malformed_members_reject() {
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([0 1],[1 2]);legend('left');subplot(1,2,2);plot([0 1],[2 1]);legend('right');").unwrap();
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
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("subplots.mfig");
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], b"MATRFIGK");
    let decoded = files::load_figure(&path).unwrap();
    for ((p, a), (q, b)) in e
        .figure
        .as_ref()
        .unwrap()
        .all_axes()
        .zip(decoded.all_axes())
    {
        assert_eq!(p, q);
        assert!(a.same_metadata(b));
        assert!(b.legend_state.is_some());
    }
    bytes[..8].copy_from_slice(b"MATRFIGI");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGK"));
    e.execute("figure(5);plot([0 1],[3 4]);legend('single');")
        .unwrap();
    files::save_figure(&path, e.figure.as_ref().unwrap()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let len = u32::from_le_bytes(original[8..12].try_into().unwrap()) as usize;
    for indices in [
        serde_json::json!([1]),
        serde_json::json!([0, 0]),
        serde_json::json!(vec![0; 65]),
    ] {
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&original[12..12 + len]).unwrap();
        metadata["legend_state"]["members"] = indices;
        let encoded = serde_json::to_vec(&metadata).unwrap();
        let mut corrupt = original[..8].to_vec();
        corrupt.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        corrupt.extend_from_slice(&encoded);
        corrupt.extend_from_slice(&original[12 + len..]);
        std::fs::write(&path, corrupt).unwrap();
        assert!(
            files::load_figure(&path)
                .unwrap_err()
                .to_ascii_lowercase()
                .contains("legend")
        );
    }
}
