use matrust::plot::text_markup::{Interpreter, TextSource};
use matrust::plot::{
    AnnotationKind, FigureUnits, LegendBox, LegendOrientation, RulerKind, ScatterCData, WindowState,
};
use matrust::{Array, Engine, files};
use std::sync::atomic::AtomicBool;

fn quote(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn fig_roundtrip(e: &Engine, dir: &std::path::Path, name: &str) -> Engine {
    let path = dir.join(format!("{name}.fig"));
    let fig = e.figure.as_ref().unwrap();
    files::save_matlab_figure(&path, fig).unwrap();
    let mut e2 = Engine::default();
    e2.execute(&format!("openfig('{}');", quote(&path)))
        .unwrap();
    e2
}

fn script_roundtrip(e: &Engine, dir: &std::path::Path, name: &str) -> Engine {
    let path = dir.join(format!("{name}.m"));
    let fig = e.figure.as_ref().unwrap();
    files::write_matlab_plot_script_with_cancel(&path, fig, &AtomicBool::new(false)).unwrap();
    let script = std::fs::read_to_string(&path).unwrap();
    let mut e2 = Engine::default();
    e2.execute(&script).unwrap();
    e2
}

#[test]
fn fix16_review_export_6_xline_rectangle_and_annotation_arrow() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("plot(1:4); xline(2); rectangle('Position',[1 1 2 1]); annotation('arrow',[.2 .8],[.2 .8]);")
        .unwrap();

    for reopened in [
        fig_roundtrip(&e, dir.path(), "shapes"),
        script_roundtrip(&e, dir.path(), "shapes"),
    ] {
        let fig = reopened.figure.as_ref().unwrap();
        assert_eq!(fig.axes.extras.constant_lines.len(), 1);
        assert!((fig.axes.extras.constant_lines[0].value - 2.0).abs() < 1e-12);
        assert_eq!(fig.axes.extras.rectangles.len(), 1);
        assert_eq!(fig.axes.extras.rectangles[0].position, [1.0, 1.0, 2.0, 1.0]);
        assert_eq!(fig.annotations.len(), 1);
        assert_eq!(fig.annotations[0].kind, AnnotationKind::Arrow);
        assert_eq!(fig.annotations[0].x_endpoints(), [0.2, 0.8]);
        assert_eq!(fig.annotations[0].y_endpoints(), [0.2, 0.8]);
    }
}

#[test]
fn fix16_review_export_7_figure_window_properties() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute(
        "f=figure('Position',[1 2 600 400],'Resize','off','NumberTitle','off','Visible','off','WindowState','maximized'); plot(1:2);",
    )
    .unwrap();

    for reopened in [
        fig_roundtrip(&e, dir.path(), "win"),
        script_roundtrip(&e, dir.path(), "win"),
    ] {
        let fig = reopened.figure.as_ref().unwrap();
        assert_eq!(fig.position, [1.0, 2.0, 600.0, 400.0]);
        assert_eq!(fig.units, FigureUnits::Pixels);
        assert!(!fig.resize);
        assert!(!fig.number_title);
        assert!(!fig.visible);
        assert_eq!(fig.window_state, WindowState::Maximized);
    }
}

#[test]
fn fix16_review_export_8_datetime_duration_categorical_rulers_and_ticks() {
    let dir = tempfile::tempdir().unwrap();

    // Datetime + manual ticks + tick labels + rotation + interpreter
    let mut e_dt = Engine::default();
    e_dt.execute(
        "d=datetime(2024,1,1:3); plot(d,1:3); xticks(d); xticklabels({'a','b','c'}); xtickangle(45); ax=gca; ax.TickLabelInterpreter='latex';",
    )
    .unwrap();
    for mut reopened in [
        fig_roundtrip(&e_dt, dir.path(), "dt"),
        script_roundtrip(&e_dt, dir.path(), "dt"),
    ] {
        {
            let fig = reopened.figure.as_ref().unwrap();
            assert!(matches!(
                fig.axes.extras.rulers[0].kind,
                RulerKind::Datetime { .. }
            ));
            assert_eq!(
                fig.axes.extras.rulers[0].ticks.as_ref().map(Vec::len),
                Some(3)
            );
            assert_eq!(
                fig.axes.extras.rulers[0].tick_labels,
                Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])
            );
            assert!((fig.axes.extras.rulers[0].tick_label_rotation - 45.0).abs() < 1e-12);
            assert_eq!(fig.axes.extras.tick_label_interpreter, Interpreter::Latex);
        }
        reopened.execute("xt = get(gca, 'XTick');").unwrap();
        assert!(matches!(reopened.workspace()["xt"], Array::Datetime(_)));
    }

    // Duration
    let mut e_dur = Engine::default();
    e_dur.execute("t=seconds(1:3); plot(t,1:3);").unwrap();
    for mut reopened in [
        fig_roundtrip(&e_dur, dir.path(), "dur"),
        script_roundtrip(&e_dur, dir.path(), "dur"),
    ] {
        assert!(matches!(
            reopened.figure.as_ref().unwrap().axes.extras.rulers[0].kind,
            RulerKind::Duration { .. }
        ));
        reopened.execute("xt = get(gca, 'XTick');").unwrap();
        assert!(matches!(reopened.workspace()["xt"], Array::Duration(_)));
    }

    // Categorical
    let mut e_cat = Engine::default();
    e_cat
        .execute("c=categorical({'low','mid','high'}); plot(c,1:3);")
        .unwrap();
    for mut reopened in [
        fig_roundtrip(&e_cat, dir.path(), "cat"),
        script_roundtrip(&e_cat, dir.path(), "cat"),
    ] {
        assert_eq!(
            reopened.figure.as_ref().unwrap().axes.extras.rulers[0].kind,
            RulerKind::Categorical {
                categories: vec!["high".into(), "low".into(), "mid".into()],
                ordinal: false,
            }
        );
        reopened.execute("xt = get(gca, 'XTick');").unwrap();
        assert!(matches!(reopened.workspace()["xt"], Array::Categorical(_)));
    }
}

#[test]
fn fix16_review_export_9_scatter_indexed_cdata_sizes_colormap_clim() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("scatter(1:2,1:2,[4 9],[10;20],'filled'); colormap([1 0 0;0 1 0]); clim([10 20]);")
        .unwrap();

    for reopened in [
        fig_roundtrip(&e, dir.path(), "scatter_idx"),
        script_roundtrip(&e, dir.path(), "scatter_idx"),
    ] {
        let fig = reopened.figure.as_ref().unwrap();
        let s = &fig.axes.series[0];
        assert_eq!(s.marker_sizes.as_deref(), Some(&[4.0, 9.0][..]));
        assert_eq!(
            s.scatter_cdata,
            Some(ScatterCData::Indexed(vec![10.0, 20.0].into()))
        );
        assert_eq!(
            fig.axes.extras.colormap.as_deref().map(Vec::as_slice),
            Some(&[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]][..])
        );
        assert_eq!(fig.axes.extras.clim, Some([10.0, 20.0]));
    }
}

#[test]
fn fix16_review_export_10_legend_layout_and_interpreter() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute(
        "plot(1:2); legend('x_1','AutoUpdate','off','Orientation','horizontal','Box','off','Interpreter','latex','NumColumns',2);",
    )
    .unwrap();

    for reopened in [
        fig_roundtrip(&e, dir.path(), "repro_legend"),
        script_roundtrip(&e, dir.path(), "repro_legend"),
    ] {
        let fig = reopened.figure.as_ref().unwrap();
        assert!(fig.axes.legend);
        assert!(!fig.axes.legend_state.as_ref().unwrap().auto_update);
        assert_eq!(
            fig.axes.extras.legend_orientation,
            LegendOrientation::Horizontal
        );
        assert_eq!(fig.axes.extras.legend_box, LegendBox::Off);
        assert_eq!(fig.axes.extras.legend_interpreter, Interpreter::Latex);
        assert_eq!(fig.axes.extras.legend_num_columns, Some(2));
    }
}

#[test]
fn fix16_review_export_11_multiline_title_and_interpreter() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("plot(1:2); title({'a','b'},'Interpreter','latex');")
        .unwrap();

    for mut reopened in [
        fig_roundtrip(&e, dir.path(), "repro_title"),
        script_roundtrip(&e, dir.path(), "repro_title"),
    ] {
        {
            let fig = reopened.figure.as_ref().unwrap();
            assert_eq!(
                fig.axes.extras.label_lines[0],
                Some(TextSource::Cell(vec!["a".into(), "b".into()]))
            );
            assert_eq!(fig.axes.extras.label_interpreters[0], Interpreter::Latex);
        }
        reopened
            .execute("t = get(gca,'Title'); s = t.String; interp = t.Interpreter;")
            .unwrap();
        assert_eq!(reopened.workspace()["interp"].to_string(), "'latex'");
        match &reopened.workspace()["s"] {
            Array::Cell(c) => assert_eq!(c.len(), 2),
            Array::Str(sa) => assert_eq!(sa.rows(), 2),
            other => panic!("expected 2-row title String/Cell, got {other:?}"),
        }
    }
}

#[test]
fn fix16_review_export_24_scatter3_zdata_and_per_point_props() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.execute("scatter3(1:2,1:2,[7 8],[4 9],[10;20]);").unwrap();

    for reopened in [
        fig_roundtrip(&e, dir.path(), "scatter3"),
        script_roundtrip(&e, dir.path(), "scatter3"),
    ] {
        let fig = reopened.figure.as_ref().unwrap();
        let s = &fig.axes.series[0];
        assert_eq!(&*s.z, &[7.0, 8.0]);
        assert_eq!(s.marker_sizes.as_deref(), Some(&[4.0, 9.0][..]));
        assert_eq!(
            s.scatter_cdata,
            Some(ScatterCData::Indexed(vec![10.0, 20.0].into()))
        );
        assert_ne!(fig.axes.view, [0.0, 90.0]);
    }
}
