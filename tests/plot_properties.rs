use matrust::{
    Engine, files,
    plot::{LinePattern, PlotKind},
};

#[test]
fn plot_properties_apply_to_all_new_pairs_and_columns_with_explicit_precedence() {
    let mut e = Engine::default();
    e.execute("plot([1 2],[3 4],'ro',[2 3],[5 6],'b--','LineWidth',2.5,'Color',[0.1 0.5 1],'LineStyle','-.','DisplayName','Measured');").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 2);
    for s in &f.series {
        assert_eq!(s.line_width, Some(2.5));
        assert_eq!(s.color, [26, 128, 255]);
        assert_eq!(s.pattern, LinePattern::DashDot);
        assert_eq!(s.kind, PlotKind::Line);
        assert_eq!(s.label, "Measured");
    }
    assert!(f.series[0].markers);
    assert!(!f.series[1].markers);
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
        &[[2., 5.], [3., 6.]]
    );
    e.execute("plot([1 2;3 4;5 6],'linewidth',1,'LINEWIDTH',4,'Color','#09F');")
        .unwrap();
    let f = e.figure.unwrap();
    assert_eq!(f.series.len(), 2);
    assert_eq!(
        f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
        &[[1., 2.], [2., 4.], [3., 6.]]
    );
    assert!(
        f.series
            .iter()
            .all(|s| s.line_width == Some(4.) && s.color == [0, 153, 255])
    );
}

#[test]
fn plot_properties_hold_preserves_old_shared_samples_and_other_axes() {
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([1 2]);subplot(1,2,2);plot([3 4]);hold on;")
        .unwrap();
    let original = e.figure.clone().unwrap();
    e.execute("plot([5 6],'LineWidth',3,'Visible','off','DisplayName','Hidden');")
        .unwrap();
    let f = e.figure.unwrap();
    assert!((f.series[0].points).same_storage(&original.series[0].points));
    assert_eq!(f.series[0].line_width, None);
    assert!(!f.series[1].visible);
    assert_eq!(f.series[1].label, "Hidden");
    assert!(
        f.layout.as_ref().unwrap().others[0]
            .axes
            .same_snapshot(&original.layout.as_ref().unwrap().others[0].axes)
    );
}

#[test]
fn plot_properties_supported_plot_families_keep_geometry_and_style() {
    for command in ["plot", "semilogx", "semilogy", "loglog", "stairs", "stem"] {
        let mut e = Engine::default();
        e.execute(&format!(
            "{command}([1 2],[3 4],'r--o','Color','magenta','LineWidth',2,'Visible',1);"
        ))
        .unwrap();
        let s = &e.figure.as_ref().unwrap().series[0];
        assert_eq!(s.color, [255, 0, 255]);
        assert_eq!(s.line_width, Some(2.));
        assert_eq!(
            s.points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 3.], [2., 4.]]
        );
        assert!(s.visible && s.markers);
    }
    let mut e = Engine::default();
    e.execute("stem([1 2],'filled','LineWidth',2,'Visible',false);")
        .unwrap();
    let s = &e.figure.as_ref().unwrap().series[0];
    assert!(s.marker_filled && !s.visible);
    e.execute("scatter([1 2],[3 4],'MarkerEdgeColor','#123456','LineWidth',3);")
        .unwrap();
    assert_eq!(
        e.figure.as_ref().unwrap().series[0].color,
        [0x12, 0x34, 0x56]
    );
    e.execute("bar([1 2;3 4],'FaceColor','cyan','DisplayName','Bars','Visible','off');")
        .unwrap();
    assert!(
        e.figure
            .as_ref()
            .unwrap()
            .series
            .iter()
            .all(|s| s.color == [0, 255, 255] && !s.visible)
    );
    assert!(e.figure.as_ref().unwrap().series[0].bar_offset < 0.);
}

#[test]
fn plot_properties_invalid_pairs_preserve_existing_figure_and_data() {
    let mut e = Engine::default();
    e.execute("plot([1 2]);title('Keep');hold on;").unwrap();
    let before = e.figure.clone().unwrap();
    for source in [
        "plot([1 2],'LineWidth')",
        "plot([1 2],'LineWidth',0)",
        "plot([1 2],'LineWidth',NaN)",
        "plot([1 2],'LineWidth',[1 2])",
        "plot([1 2],'Color',[0 1 2])",
        "plot([1 2],'Color',[0 NaN 1])",
        "plot([1 2],'Color','#zff')",
        "plot([1 2],'Color','none')",
        "plot([1 2],'Visible',2)",
        "plot([1 2],'Visible','maybe')",
        "plot([1 2],'DisplayName',12)",
        "plot([1 2],'MarkerIndices',8)",
        "plot([1 2],'LineStyle','invalid')",
        "plot([1 2],'LineWidth',2,[1 2],[3 4])",
        "plot('Color','red')",
        "bar([1 2],'LineWidth',0)",
        "bar([1 2],'Color','red')",
        "scatter([1 2],'Color','red')",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()), "{source}");
        assert!(
            (before.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points)
        );
    }
    let source = format!("plot([1 2]{});", ",'Visible',1".repeat(17));
    assert!(e.execute(&source).unwrap_err().contains("16 plot property"));
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
}

#[test]
fn plot_properties_named_colors_and_native_fig_roundtrips() {
    let mut e = Engine::default();
    e.execute(
        "plot([1 2],[3 4],'bo-','Color','red','LineWidth',3,'DisplayName','Saved','Visible','on');",
    )
    .unwrap();
    let f = e.figure.as_ref().unwrap();
    let dir = tempfile::tempdir().unwrap();
    for ext in ["mfig", "fig"] {
        let path = dir.path().join(format!("properties.{ext}"));
        files::save_figure(&path, f).unwrap();
        let read = files::load_figure(&path).unwrap();
        assert_eq!(read.svg(), f.svg());
        assert_eq!(read.series[0].line_width, Some(3.));
    }
    for (color, rgb) in [
        ("RED", [255, 0, 0]),
        ("g", [0, 255, 0]),
        ("#abc", [170, 187, 204]),
        ("#A0b1C2", [160, 177, 194]),
    ] {
        e.execute(&format!("plot([1 2],'Color','{color}');"))
            .unwrap();
        assert_eq!(e.figure.as_ref().unwrap().series[0].color, rgb);
    }
}
