use matrust::{
    Engine,
    plot::{AxesPlacement, Figure, LegendLocation, Position},
};

fn setup(existing_legend: bool) -> (Engine, Figure, Position, AxesPlacement) {
    let mut e = Engine::default();
    e.execute("subplot('Position',[0.1 0.2 0.3 0.6]);plot([0 1],[0 1]);a=gca;subplot('Position',[0.6 0.2 0.3 0.6]);plot([0 1],[1 0]);b=gca;").unwrap();
    if existing_legend {
        e.execute("lg=legend(a,'Left','Location','northeast');")
            .unwrap();
    }
    let original = e.figure.as_ref().unwrap().clone();
    let left = original
        .all_axes()
        .find(|(_, a)| !a.identity.same_object(&original.axes.identity))
        .unwrap()
        .0;
    let mut expected = original.clone();
    expected.select_subplot(left, false).unwrap();
    let allocation = expected.initial_axes_placement().unwrap();
    assert!(expected.axes.placement.is_none());
    (e, original, left, allocation)
}

fn assert_target(e: &Engine, original: &Figure, left: Position, expected: AxesPlacement) {
    let f = e.figure.as_ref().unwrap();
    f.validate().unwrap();
    assert_eq!(f.active_position(), original.active_position());
    assert!(f.axes.identity.same_object(&original.axes.identity));
    assert!(f.axes.same_snapshot(&original.axes));
    let target = f.all_axes().find(|(p, _)| *p == left).unwrap().1;
    assert_eq!(target.legend_location, LegendLocation::Westoutside);
    assert_eq!(target.placement, Some(expected));
    assert_eq!(target.series[0].label, "Left");
    for (position, axes) in original.all_axes() {
        let updated = f.all_axes().find(|(p, _)| *p == position).unwrap().1;
        assert!(axes.identity.same_object(&updated.identity));
        assert!((axes.series[0].points).same_storage(&updated.series[0].points));
    }
    // Compare resolved geometry against a direct model operation on the same
    // target, then restore the original active axes. This is independent of
    // the runtime handle-targeting/restoration implementation under test.
    let mut direct = original.clone();
    direct.select_subplot(left, false).unwrap();
    direct.axes.legend = true;
    direct.axes.legend_location = LegendLocation::Westoutside;
    direct.axes.series[0].label = "Left".into();
    direct.prepare_legend_layout().unwrap();
    direct
        .select_subplot(original.active_position(), false)
        .unwrap();
    let actual = f.resolved_axes_geometry().unwrap();
    let control = direct.resolved_axes_geometry().unwrap();
    for (position, g) in actual {
        let expected = control.iter().find(|(p, _)| *p == position).unwrap().1;
        assert_eq!(g.data, expected.data);
        assert_eq!(g.outer, expected.outer);
        assert_eq!(g.legend, expected.legend);
    }
}

#[test]
fn inactive_custom_legend_creation_prepares_target_and_restores_active_axes() {
    let (mut e, original, left, expected) = setup(false);
    e.execute("lg=legend(a,'Left','Location','westoutside');")
        .unwrap();
    assert_target(&e, &original, left, expected);
}

#[test]
fn inactive_legend_location_set_prepares_target_and_failure_is_atomic() {
    let (mut e, original, left, expected) = setup(true);
    e.execute("set(lg,'Location','westoutside');").unwrap();
    assert_target(&e, &original, left, expected);
    let before = e.figure.as_ref().unwrap().clone();
    assert!(
        e.execute("set(lg,'Location','eastoutside','Box','off');")
            .is_err()
    );
    assert!(e.figure.as_ref().unwrap().same_snapshot(&before));
}
