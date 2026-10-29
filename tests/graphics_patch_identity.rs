use matrust::{
    Engine, Matrix, files,
    plot::{Figure, Series, TextBox},
};
use std::sync::atomic::AtomicBool;

fn figure() -> Figure {
    let x = Matrix::from_column_slice(3, 1, &[0., 1., 2.]).unwrap();
    let y = Matrix::from_column_slice(3, 1, &[0., 2., 0.]).unwrap();
    let mut f = Figure::default();
    f.add(vec![
        Series::patch_from_matrices(&x, &y, [20, 40, 80], &AtomicBool::new(false)).unwrap(),
    ])
    .unwrap();
    f.add_textbox(TextBox::new([0., 0.9, 1., 0.1], "Literal title".into()).unwrap())
        .unwrap();
    f
}

#[test]
fn patch_and_textbox_clones_retain_tokens_but_recovery_has_fresh_identity() {
    let f = figure();
    let copy = f.clone();
    assert!(f.identity.same_object(&copy.identity));
    assert!(f.axes.identity.same_object(&copy.axes.identity));
    assert!(f.series[0].identity.same_object(&copy.series[0].identity));
    assert!(
        f.textboxes[0]
            .identity
            .same_object(&copy.textboxes[0].identity)
    );
    assert!((f.series[0].points).same_storage(&copy.series[0].points));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("objects.mfig");
    files::save_figure(&path, &f).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let restored = files::load_figure(&path).unwrap();
    assert!(!f.identity.same_object(&restored.identity));
    assert!(!f.axes.identity.same_object(&restored.axes.identity));
    assert!(
        !f.series[0]
            .identity
            .same_object(&restored.series[0].identity)
    );
    assert!(
        !f.textboxes[0]
            .identity
            .same_object(&restored.textboxes[0].identity)
    );
    assert_eq!(f.textboxes, restored.textboxes);
    assert_eq!(f.svg(), restored.svg());
    files::save_figure(&path, &restored).unwrap();
    assert_eq!(bytes, std::fs::read(&path).unwrap());
}

#[test]
fn duplicate_textbox_tokens_reject_atomically_including_cross_kind_aliases() {
    let mut f = figure();
    let before = f.clone();
    assert!(
        f.add_textbox(f.textboxes[0].clone())
            .unwrap_err()
            .contains("identity")
    );
    assert!(f.same_snapshot(&before));
    for identity in [
        &before.identity,
        &before.axes.identity,
        &before.series[0].identity,
    ] {
        let mut box_ = TextBox::new([0., 0., 0.5, 0.1], "Other".into()).unwrap();
        box_.identity = identity.clone();
        assert!(
            f.add_textbox(box_.clone())
                .unwrap_err()
                .contains("identity")
        );
        assert!(f.same_snapshot(&before));
        let mut invalid = before.clone();
        invalid.textboxes.push(box_);
        assert!(invalid.validate().unwrap_err().contains("identity"));
    }
    let independent = TextBox::new(
        before.textboxes[0].position,
        before.textboxes[0].text.clone(),
    )
    .unwrap();
    assert_eq!(independent, before.textboxes[0]);
    f.add_textbox(independent).unwrap();
    f.validate().unwrap();
}

#[test]
fn ordinary_hold_off_replacement_preserves_figure_axes_and_annotation_tokens() {
    let f = figure();
    let before = f.clone();
    let mut engine = Engine::default();
    engine.figure = Some(f);
    engine.execute("plot([0 1],[1 0]);").unwrap();
    let after = engine.figure.as_ref().unwrap();
    assert!(after.identity.same_object(&before.identity));
    assert!(after.axes.identity.same_object(&before.axes.identity));
    assert!(
        after.textboxes[0]
            .identity
            .same_object(&before.textboxes[0].identity)
    );
    assert_eq!(after.textboxes, before.textboxes);
    assert!(
        !after.series[0]
            .identity
            .same_object(&before.series[0].identity)
    );
    after.validate().unwrap();
}
