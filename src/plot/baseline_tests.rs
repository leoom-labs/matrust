use super::*;
use crate::plot::{Figure, Series};
use std::{cell::Cell, sync::atomic::Ordering};

fn style() -> BaselineStyle {
    BaselineStyle {
        color: [0.15, 0.25, 0.35],
        pattern: LinePattern::Dashed,
        width_points: 1.25,
        visible: true,
    }
}
fn axes() -> Axes {
    Axes {
        series: vec![
            Series::new(vec![[1., 2.], [2., 3.]], PlotKind::Bar),
            Series::new(vec![[1., 4.], [2., 5.]], PlotKind::Stem),
        ],
        ..Axes::default()
    }
}
fn shared(axes: &Axes, value: BaselineValue) -> Baseline {
    Baseline::new(
        value,
        style(),
        axes.series.iter().map(|s| s.identity.clone()).collect(),
    )
    .unwrap()
}

#[test]
fn empty_metadata_preserves_existing_geometry_svg_and_native_bytes() {
    let stop = AtomicBool::new(false);
    let mut figure = Figure {
        axes: axes(),
        ..Figure::default()
    };
    figure.series[1].base_value = Some(-0.0);
    let svg = figure.svg();
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("baselines"));
    figure.axes.replace_baselines(Vec::new(), &stop).unwrap();
    assert_eq!(svg, figure.svg());
    assert_eq!(bytes, crate::files::encode_native_figure(&figure).unwrap());
    let restored = crate::files::decode_native_figure(&bytes).unwrap();
    assert!(restored.axes.baselines.is_empty());
    assert_eq!(
        restored
            .axes
            .effective_baseline(0, AxisTransform::default())
            .unwrap()
            .to_bits(),
        0f64.to_bits()
    );
    assert_eq!(
        restored
            .axes
            .effective_baseline(1, AxisTransform::default())
            .unwrap()
            .to_bits(),
        (-0f64).to_bits()
    );
    for negative in [false, true] {
        let axis = AxisTransform {
            scale: AxisScale::Log,
            negative,
        };
        assert_eq!(
            figure.axes.effective_baseline(0, axis).unwrap(),
            if negative { -1. } else { 1. }
        );
    }
}

#[test]
fn one_shared_object_updates_two_effective_bases_without_copying_or_restyling_series() {
    let stop = AtomicBool::new(false);
    let mut axes = axes();
    let original = axes.clone();
    let baseline = shared(&axes, BaselineValue::Value(-0.0));
    let identity = baseline.identity.clone();
    axes.replace_baselines(vec![baseline], &stop).unwrap();
    let clone = axes.clone();
    assert!(clone.baselines[0].identity.same_object(&identity));
    assert!(clone.same_snapshot(&axes));
    assert!(!axes.same_metadata(&original));
    for index in 0..2 {
        assert!(
            axes.baseline_for_series(&axes.series[index].identity)
                .unwrap()
                .identity
                .same_object(&identity)
        );
        assert_eq!(
            axes.effective_baseline(index, AxisTransform::default())
                .unwrap()
                .to_bits(),
            (-0f64).to_bits()
        );
    }
    let mut replacement = axes.baselines.clone();
    replacement[0].value = BaselineValue::Value(7.5);
    replacement[0].style.visible = false;
    axes.replace_baselines(replacement, &stop).unwrap();
    for index in 0..2 {
        assert_eq!(
            axes.effective_baseline(index, AxisTransform::default())
                .unwrap(),
            7.5
        );
        assert!(
            axes.series[index]
                .points
                .same_storage(&original.series[index].points)
        );
        assert_eq!(axes.series[index].visible, original.series[index].visible);
        assert_eq!(axes.series[index].color, original.series[index].color);
        assert_eq!(axes.series[index].pattern, original.series[index].pattern);
    }
    assert_eq!(
        axes.baselines[0].style.color[0].to_bits(),
        0.15f64.to_bits()
    );
    assert!(!axes.same_snapshot(&clone));
    let mut positive_zero = clone.clone();
    positive_zero.baselines[0].value = BaselineValue::Value(0.0);
    assert!(!positive_zero.same_metadata(&clone));
}

#[test]
fn membership_is_identity_owned_reorder_safe_and_removal_preserves_standalone_objects() {
    let stop = AtomicBool::new(false);
    let mut axes = axes();
    let first = Baseline::new(
        BaselineValue::Value(2.),
        style(),
        vec![axes.series[0].identity.clone()],
    )
    .unwrap();
    let mut second = Baseline::new(
        BaselineValue::Value(2.),
        style(),
        vec![axes.series[1].identity.clone()],
    )
    .unwrap();
    second.style.color = [0.8, 0.2, 0.1];
    let standalone = Baseline::new(BaselineValue::Automatic, style(), vec![]).unwrap();
    let ids = [
        first.identity.clone(),
        second.identity.clone(),
        standalone.identity.clone(),
    ];
    axes.replace_baselines(vec![first, second, standalone], &stop)
        .unwrap();
    assert_eq!(axes.baselines.len(), 3); // Equal values do not merge distinct objects.
    axes.series.swap(0, 1);
    assert!(
        axes.baseline_for_series(&axes.series[0].identity)
            .unwrap()
            .identity
            .same_object(&ids[1])
    );
    axes.remove_series(0).unwrap();
    assert_eq!(axes.baselines.len(), 2);
    assert!(axes.baselines[0].identity.same_object(&ids[0]));
    axes.remove_series(0).unwrap();
    assert_eq!(axes.baselines.len(), 1);
    assert!(axes.baselines[0].identity.same_object(&ids[2]));
    axes.validate().unwrap();
}

#[test]
fn validation_rejects_foreign_duplicate_conflicting_and_invalid_style_metadata_atomically() {
    let stop = AtomicBool::new(false);
    let mut axes = axes();
    let original = axes.clone();
    let valid = shared(&axes, BaselineValue::Value(0.));
    let mut foreign = valid.clone();
    foreign.members[0] = ObjectIdentity::default();
    let mut duplicate_member = valid.clone();
    duplicate_member
        .members
        .push(duplicate_member.members[0].clone());
    let mut duplicate_identity = valid.clone();
    duplicate_identity.identity = axes.identity.clone();
    for records in [
        vec![foreign],
        vec![duplicate_member],
        vec![duplicate_identity],
        vec![valid.clone(), valid],
    ] {
        assert!(axes.replace_baselines(records, &stop).is_err());
        assert!(axes.same_snapshot(&original));
    }
    for invalid in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut bad = style();
        bad.color[0] = invalid;
        assert!(Baseline::new(BaselineValue::Automatic, bad, vec![]).is_err());
    }
    assert!(Baseline::new(BaselineValue::Value(f64::INFINITY), style(), vec![]).is_err());
    let mut bad = style();
    bad.width_points = 0.;
    assert!(Baseline::new(BaselineValue::Automatic, bad, vec![]).is_err());
    axes.series[1].base_value = Some(-0.0);
    assert!(
        axes.replace_baselines(vec![shared(&axes, BaselineValue::Value(0.))], &stop)
            .is_err()
    );
    assert!(
        axes.replace_baselines(vec![shared(&axes, BaselineValue::Automatic)], &stop)
            .is_err()
    );
    axes.replace_baselines(vec![shared(&axes, BaselineValue::Value(-0.0))], &stop)
        .unwrap();
    axes.series[0].kind = PlotKind::Line;
    assert!(axes.validate().is_err());
}

#[test]
fn limits_and_logarithmic_resolution_are_bounded_and_do_not_rewrite_source_bits() {
    let stop = AtomicBool::new(false);
    let mut axes = Axes {
        series: (0..64)
            .map(|_| Series::new(vec![[-2., -4.]], PlotKind::Bar))
            .collect(),
        ..Axes::default()
    };
    let records = axes
        .series
        .iter()
        .map(|s| {
            Baseline::new(BaselineValue::Automatic, style(), vec![s.identity.clone()]).unwrap()
        })
        .collect();
    axes.replace_baselines(records, &stop).unwrap();
    axes.validate().unwrap();
    let original = axes.clone();
    let mut excess = axes.baselines.clone();
    excess.push(Baseline::new(BaselineValue::Automatic, style(), vec![]).unwrap());
    assert!(axes.replace_baselines(excess, &stop).is_err());
    assert!(axes.same_snapshot(&original));
    let mut excess_members = axes.baselines.clone();
    excess_members[0]
        .members
        .push(axes.series[1].identity.clone());
    assert!(axes.replace_baselines(excess_members, &stop).is_err());
    axes.yscale = AxisScale::Log;
    axes.ylim = Some([-10., -1.]);
    let transforms = axes.axis_transforms();
    for index in 0..64 {
        assert_eq!(axes.effective_baseline(index, transforms[1]).unwrap(), -1.);
    }
    let mut invalid = axes.baselines.clone();
    invalid[0].value = BaselineValue::Value(1.);
    assert!(axes.replace_baselines(invalid, &stop).is_err());
    assert!(
        axes.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
    assert_eq!(
        axes.series[0].points.get(0).unwrap()[1].to_bits(),
        (-4f64).to_bits()
    );
    axes.ylim = Some([1., 10.]);
    axes.validate().unwrap();
    assert_eq!(
        axes.effective_baseline(0, axes.axis_transforms()[1])
            .unwrap(),
        1.
    );
}

#[test]
fn cancellation_during_membership_and_at_final_publication_preserves_old_objects_and_samples() {
    let mut axes = axes();
    let stop = AtomicBool::new(false);
    let first = shared(&axes, BaselineValue::Value(0.));
    axes.replace_baselines(vec![first], &stop).unwrap();
    let original = axes.clone();
    // Precheck, validation entry, object, member0, member1, validation end, publication.
    for stop_at in [4, 7] {
        let checks = Cell::new(0);
        let replacement = shared(&axes, BaselineValue::Value(3.));
        let error = axes
            .replace_baselines_checked(vec![replacement], &|| {
                checks.set(checks.get() + 1);
                if checks.get() == stop_at {
                    stop.store(true, Ordering::Relaxed);
                }
                super::super::display_check(&stop)
            })
            .unwrap_err();
        assert_eq!(error, "Figure display preparation cancelled");
        assert_eq!(checks.get(), stop_at);
        assert!(axes.same_snapshot(&original));
        assert!(
            axes.baselines[0]
                .identity
                .same_object(&original.baselines[0].identity)
        );
        assert!(
            axes.series[0]
                .points
                .same_storage(&original.series[0].points)
        );
        stop.store(false, Ordering::Relaxed);
    }
    axes.replace_baselines(vec![shared(&axes, BaselineValue::Value(3.))], &stop)
        .unwrap();
    assert_eq!(
        axes.effective_baseline(0, AxisTransform::default())
            .unwrap(),
        3.
    );
    assert!(
        axes.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
}

#[test]
fn baseline_identity_cannot_alias_a_figure_object_and_metadata_clones_keep_sample_sharing() {
    let stop = AtomicBool::new(false);
    let mut figure = Figure {
        axes: axes(),
        ..Figure::default()
    };
    let mut record = shared(&figure.axes, BaselineValue::Automatic);
    record.identity = figure.identity.clone();
    // Axes validation cannot see its parent; complete Figure validation must.
    figure.axes.replace_baselines(vec![record], &stop).unwrap();
    assert!(
        figure
            .validate()
            .unwrap_err()
            .contains("identity appears twice")
    );
    let valid = shared(&figure.axes, BaselineValue::Automatic);
    figure.axes.replace_baselines(vec![valid], &stop).unwrap();
    figure.validate().unwrap();
    let cloned = figure.clone();
    assert!(cloned.same_snapshot(&figure));
    assert!(
        cloned.axes.baselines[0]
            .identity
            .same_object(&figure.axes.baselines[0].identity)
    );
    assert!(
        cloned.series[0]
            .points
            .same_storage(&figure.series[0].points)
    );
}

#[test]
fn public_baseline_value_transaction_updates_all_members_and_keeps_samples() {
    let stop = AtomicBool::new(false);
    let mut axes = axes();
    let record = shared(&axes, BaselineValue::Value(0.));
    let id = record.identity.clone();
    axes.replace_baselines(vec![record], &stop).unwrap();
    let original = axes.clone();
    let mut edited_style = style();
    edited_style.color = [0.123456789012345, 0.2, 1.];
    edited_style.visible = false;
    axes.edit_baseline(&id, Some(-0.0), Some(edited_style), &stop)
        .unwrap();
    assert!(axes.baselines[0].identity.same_object(&id));
    assert_eq!(
        axes.baselines[0].style.color[0].to_bits(),
        edited_style.color[0].to_bits()
    );
    assert!(!axes.baselines[0].style.visible);
    for (index, series) in axes.series.iter().enumerate() {
        assert_eq!(
            axes.effective_baseline(index, AxisTransform::default())
                .unwrap()
                .to_bits(),
            (-0f64).to_bits()
        );
        if series.kind == PlotKind::Stem {
            assert_eq!(series.base_value.map(f64::to_bits), Some((-0f64).to_bits()));
        } else {
            assert!(series.base_value.is_none());
        }
        assert!(
            series
                .identity
                .same_object(&original.series[index].identity)
        );
        assert!(series.points.same_storage(&original.series[index].points));
        assert_eq!(series.visible, original.series[index].visible);
        assert_eq!(series.color, original.series[index].color);
        assert!(axes.baselines[0].members[index].same_object(&series.identity));
    }
    axes.validate().unwrap();
    let bits = axes
        .series
        .iter()
        .map(|s| s.base_value.map(f64::to_bits))
        .collect::<Vec<_>>();
    edited_style.visible = true;
    axes.edit_baseline(&id, None, Some(edited_style), &stop)
        .unwrap();
    assert_eq!(
        axes.series
            .iter()
            .map(|s| s.base_value.map(f64::to_bits))
            .collect::<Vec<_>>(),
        bits
    );
}

#[test]
fn public_baseline_transaction_stop_invalid_and_stale_do_not_publish() {
    let mut axes = axes();
    let record = shared(&axes, BaselineValue::Automatic);
    let id = record.identity.clone();
    axes.replace_baselines(vec![record], &AtomicBool::new(false))
        .unwrap();
    let original = axes.clone();
    let calls = Cell::new(0usize);
    // Discover the finite metadata checkpoint count without timing or threads.
    let mut successful = original.clone();
    successful
        .edit_baseline_checked(&id, Some(2.), None, &|| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
    assert!(calls.get() > 4);
    for stop_at in 1..=calls.get() {
        let progress = Cell::new(0usize);
        assert!(
            axes.edit_baseline_checked(&id, Some(2.), None, &|| {
                progress.set(progress.get() + 1);
                if progress.get() == stop_at {
                    Err("Cancelled".into())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        assert_eq!(progress.get(), stop_at);
        assert!(axes.same_snapshot(&original));
        for (a, b) in axes.series.iter().zip(&original.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            axes.edit_baseline(&id, Some(value), None, &AtomicBool::new(false))
                .is_err()
        );
        assert!(axes.same_snapshot(&original));
    }
    assert!(
        axes.edit_baseline(
            &ObjectIdentity::default(),
            Some(3.),
            None,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let mut invalid = style();
    invalid.width_points = 0.;
    assert!(
        axes.edit_baseline(&id, Some(3.), Some(invalid), &AtomicBool::new(false))
            .is_err()
    );
    assert!(axes.same_snapshot(&original));
    axes.edit_baseline(&id, Some(2.), None, &AtomicBool::new(false))
        .unwrap();
    assert!(axes.same_snapshot(&successful));
}

#[test]
fn public_baseline_style_preserves_automatic_and_wrong_log_sign_rolls_back() {
    let stop = AtomicBool::new(false);
    let mut axes = axes();
    axes.yscale = AxisScale::Log;
    let record = shared(&axes, BaselineValue::Automatic);
    let id = record.identity.clone();
    axes.replace_baselines(vec![record], &stop).unwrap();
    let mut hidden = style();
    hidden.visible = false;
    axes.edit_baseline(&id, None, Some(hidden), &stop).unwrap();
    assert!(matches!(axes.baselines[0].value, BaselineValue::Automatic));
    assert!(axes.series.iter().all(|s| s.base_value.is_none()));
    let original = axes.clone();
    assert!(axes.edit_baseline(&id, Some(-1.), None, &stop).is_err());
    assert!(axes.same_snapshot(&original));
    axes.edit_baseline(&id, Some(0.5), None, &stop).unwrap();
    assert!(axes.series.iter().enumerate().all(|(i, _)| {
        axes.effective_baseline(i, axes.axis_transforms()[1])
            .unwrap()
            == 0.5
    }));
    assert!(axes.series[0].base_value.is_none());
    assert_eq!(axes.series[1].base_value, Some(0.5));
    axes.validate().unwrap();
}
