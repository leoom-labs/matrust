use matrust::{Array, Engine};

fn as_bool(a: Array) -> bool {
    match a {
        Array::Logical(m) => m.as_scalar().unwrap(),
        _ => panic!("expected logical scalar"),
    }
}

fn as_text(a: Array) -> String {
    match a {
        Array::Character(c) => String::from_utf16_lossy(c.units()),
        _ => panic!("expected character row"),
    }
}

fn as_vec(a: Array) -> Vec<f64> {
    match a {
        Array::Numeric(m) => m.data().to_vec(),
        _ => panic!("expected numeric matrix"),
    }
}

#[test]
fn fix16_review_engine_tiledlayout_preserves_figure_properties_and_annotations() {
    let mut e = Engine::default();
    e.execute(
        "f=figure('Position',[1 1 600 400],'Resize','off'); \
         a=annotation('arrow',[0.1 0.5],[0.2 0.8]); \
         t=tiledlayout(1,1); \
         pos=get(f,'Position'); \
         rsz=get(f,'Resize'); \
         alive=ishandle(a); \
         ax=a.X;",
    )
    .unwrap();
    assert_eq!(as_vec(e.array("pos").unwrap()), vec![1., 1., 600., 400.]);
    assert_eq!(as_text(e.array("rsz").unwrap()), "off");
    assert!(as_bool(e.array("alive").unwrap()));
    assert_eq!(as_vec(e.array("ax").unwrap()), vec![0.1, 0.5]);
}

#[test]
fn fix16_review_engine_clf_preserves_figure_properties_and_reset_keeps_position_units() {
    let mut e = Engine::default();
    e.execute(
        "f=figure('Units','inches','Position',[1 2 6 4],'Resize','off','Name','MyFig'); \
         plot(1:3); \
         clf(f); \
         pos1=get(f,'Position'); \
         units1=get(f,'Units'); \
         rsz1=get(f,'Resize'); \
         name1=get(f,'Name'); \
         clf(f,'reset'); \
         pos2=get(f,'Position'); \
         units2=get(f,'Units'); \
         rsz2=get(f,'Resize'); \
         name2=get(f,'Name');",
    )
    .unwrap();
    assert_eq!(as_vec(e.array("pos1").unwrap()), vec![1., 2., 6., 4.]);
    assert_eq!(as_text(e.array("units1").unwrap()), "inches");
    assert_eq!(as_text(e.array("rsz1").unwrap()), "off");
    assert_eq!(as_text(e.array("name1").unwrap()), "MyFig");

    assert_eq!(as_vec(e.array("pos2").unwrap()), vec![1., 2., 6., 4.]);
    assert_eq!(as_text(e.array("units2").unwrap()), "inches");
    assert_eq!(as_text(e.array("rsz2").unwrap()), "on");
    assert_eq!(as_text(e.array("name2").unwrap()), "");
}

#[test]
fn fix16_review_engine_xaxis_limits_propagates_through_linkaxes() {
    let mut e = Engine::default();
    e.execute(
        "a=subplot(1,2,1); plot(1:3); \
         b=subplot(1,2,2); plot(1:3); \
         linkaxes([a b],'x'); \
         a.XAxis.Limits=[10 20]; \
         b_xlim=b.XLim; \
         b_limits=b.XAxis.Limits; \
         b.XAxis.Limits=[30 40]; \
         a_xlim=a.XLim;",
    )
    .unwrap();
    assert_eq!(as_vec(e.array("b_xlim").unwrap()), vec![10., 20.]);
    assert_eq!(as_vec(e.array("b_limits").unwrap()), vec![10., 20.]);
    assert_eq!(as_vec(e.array("a_xlim").unwrap()), vec![30., 40.]);
}

#[test]
fn fix16_review_engine_line_xdata_datetime_get_and_set() {
    let mut e = Engine::default();
    e.execute(
        "d=datetime(2024,1,1:3); \
         h=plot(d,1:3); \
         xd=h.XData; \
         is_dt1=isa(xd,'datetime'); \
         eq1=isequal(xd,d); \
         d2=datetime(2024,2,1:3); \
         h.XData=d2; \
         xd2=h.XData; \
         is_dt2=isa(xd2,'datetime'); \
         eq2=isequal(xd2,d2);",
    )
    .unwrap();
    assert!(as_bool(e.array("is_dt1").unwrap()));
    assert!(as_bool(e.array("eq1").unwrap()));
    assert!(as_bool(e.array("is_dt2").unwrap()));
    assert!(as_bool(e.array("eq2").unwrap()));
}

#[test]
fn fix16_review_engine_xticks_and_xticklabels_mode_query() {
    let mut e = Engine::default();
    e.execute(
        "plot(1:3); \
         m_ticks_auto=xticks('mode'); \
         m_labels_auto=xticklabels('mode'); \
         xticks([1 2 3]); \
         xticklabels({'a','b','c'}); \
         m_ticks_manual=xticks('mode'); \
         xticklabels('mode'); \
         m_labels_manual=xticklabels('mode'); \
         lbls=xticklabels; \
         ok_len=numel(lbls)==3; \
         ok_first=strcmp(lbls{1},'a');",
    )
    .unwrap();
    assert_eq!(as_text(e.array("m_ticks_auto").unwrap()), "auto");
    assert_eq!(as_text(e.array("m_labels_auto").unwrap()), "auto");
    assert_eq!(as_text(e.array("m_ticks_manual").unwrap()), "manual");
    assert_eq!(as_text(e.array("m_labels_manual").unwrap()), "manual");
    assert!(as_bool(e.array("ok_len").unwrap()));
    assert!(as_bool(e.array("ok_first").unwrap()));
}

#[test]
fn fix16_review_engine_categorical_bar_dimension_mismatch_errors() {
    let mut e = Engine::default();
    let err = e
        .execute("bar(categorical({'a','b','c'}),[1 2])")
        .expect_err("dimension mismatch should return an error, not panic");
    assert!(!err.is_empty());
}

#[test]
fn fix16_review_engine_linkaxes_across_two_figures_syncs_both_ways() {
    let mut e = Engine::default();
    e.execute(
        "figure; a=gca; plot(a,1:3); \
         figure; b=gca; plot(b,1:5); \
         linkaxes([a b],'x'); \
         init_a=a.XLim; \
         init_b=b.XLim; \
         a.XLim=[10 20]; \
         b_after_a=b.XLim; \
         b.XAxis.Limits=[30 40]; \
         a_after_b=a.XLim;",
    )
    .unwrap();
    assert_eq!(as_vec(e.array("init_a").unwrap()), vec![1., 5.]);
    assert_eq!(as_vec(e.array("init_b").unwrap()), vec![1., 5.]);
    assert_eq!(as_vec(e.array("b_after_a").unwrap()), vec![10., 20.]);
    assert_eq!(as_vec(e.array("a_after_b").unwrap()), vec![30., 40.]);
}
