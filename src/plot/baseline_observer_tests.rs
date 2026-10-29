use super::*;
use crate::plot::{Figure, Position};
use std::cell::Cell;

fn charts() -> Axes {
    let mut axes = Axes::default();
    axes.add(vec![
        Series::new(vec![[1., 2.]], PlotKind::Bar),
        Series::new(vec![[2., 3.]], PlotKind::Bar),
        Series::new(vec![[1., 4.]], PlotKind::Stem),
        Series::new(vec![[2., 5.]], PlotKind::Stem),
    ])
    .unwrap();
    axes
}

#[test]
fn observing_is_metadata_only_and_does_not_change_bytes_or_default_appearance() {
    let axes = charts();
    let before = axes.clone();
    let figure = crate::plot::Figure {
        axes: axes.clone(),
        ..Default::default()
    };
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    for _ in 0..20 {
        let a = axes.observe_series_baseline(0).unwrap();
        let b = axes.observe_series_baseline(1).unwrap();
        assert!(a.identity.same_object(b.identity));
        assert!(a.style.visible);
        let c = axes.observe_series_baseline(2).unwrap();
        let d = axes.observe_series_baseline(3).unwrap();
        assert!(!c.identity.same_object(d.identity));
        assert!(c.style.visible);
        assert_eq!(c.style.width_points, 0.75);
        assert_eq!(c.style.display_rgb(), [119; 3]);
        assert!(
            axes.observe_baseline(c.identity)
                .unwrap()
                .identity
                .same_object(c.identity)
        );
    }
    assert!(axes.baselines.is_empty());
    assert!(axes.same_snapshot(&before));
    assert_eq!(crate::files::encode_native_figure(&figure).unwrap(), bytes);
    // Inconsistent public payload/sign metadata is deliberately not inspected
    // by an observer. It is rejected by real validation/edits instead.
    let mut inconsistent = axes;
    inconsistent.yscale = AxisScale::Log;
    inconsistent.ylim = Some([-1., 1.]);
    assert!(inconsistent.observe_series_baseline(2).is_ok());
}

#[test]
fn bar_epoch_survives_reorder_and_partial_delete_but_never_reappears() {
    let mut axes = charts();
    let id = axes.observe_series_baseline(0).unwrap().identity.clone();
    axes.series.swap(0, 1);
    axes.remove_series(0).unwrap();
    assert!(axes.observe_baseline(&id).is_ok());
    axes.add(vec![Series::new(vec![[3., 4.]], PlotKind::Bar)])
        .unwrap();
    assert!(
        axes.observe_series_baseline(3)
            .unwrap()
            .identity
            .same_object(&id)
    );
    axes.remove_series(3).unwrap();
    axes.remove_series(0).unwrap();
    assert!(axes.observe_baseline(&id).is_err());
    axes.add(vec![Series::new(vec![[1., 9.]], PlotKind::Bar)])
        .unwrap();
    assert!(
        !axes
            .observe_series_baseline(2)
            .unwrap()
            .identity
            .same_object(&id)
    );
    assert!(axes.observe_baseline(&id).is_err());
}

#[test]
fn materialized_bar_group_joins_new_bars_and_preserves_stem_independence() {
    let mut axes = charts();
    let before = axes.clone();
    let id = axes.observe_series_baseline(0).unwrap().identity.clone();
    let mut style = BaselineStyle::legacy_stem();
    style.color = [0.123456789012345, 0.2, 1.];
    axes.set_observed_baseline(&id, Some(-0.), Some(style), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(axes.baselines.len(), 1);
    assert_eq!(axes.baselines[0].members.len(), 2);
    assert!(axes.baselines[0].identity.same_object(&id));
    assert_eq!(
        axes.baselines[0].style.color.map(f64::to_bits),
        style.color.map(f64::to_bits)
    );
    assert!(axes.series.iter().all(|s| s.base_value.is_none()));
    for (a, b) in axes.series.iter().zip(&before.series) {
        assert!(a.points.same_storage(&b.points));
    }
    axes.add(vec![Series::new(vec![[5., 6.]], PlotKind::Bar)])
        .unwrap();
    assert_eq!(axes.baselines[0].members.len(), 3);
    assert!(
        axes.observe_series_baseline(4)
            .unwrap()
            .identity
            .same_object(&id)
    );
    assert!(axes.baseline_for_series(&axes.series[2].identity).is_none());
    axes.validate().unwrap();
}

#[test]
fn every_materialization_checkpoint_cancels_atomically_and_retry_keeps_tokens() {
    let original = charts();
    let id = original
        .observe_series_baseline(0)
        .unwrap()
        .identity
        .clone();
    let style = BaselineStyle::legacy_stem();
    let calls = Cell::new(0);
    let mut successful = original.clone();
    successful
        .set_observed_baseline_checked(&id, Some(1.), Some(style), &|| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
    for stop_at in 1..=calls.get() {
        let n = Cell::new(0);
        let mut candidate = original.clone();
        let result = candidate.set_observed_baseline_checked(&id, Some(1.), Some(style), &|| {
            n.set(n.get() + 1);
            if n.get() == stop_at {
                Err("deterministic Stop".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err(), "deterministic Stop");
        assert!(candidate.same_snapshot(&original));
        assert!(candidate.baselines.is_empty());
        assert!(
            candidate
                .observe_series_baseline(0)
                .unwrap()
                .identity
                .same_object(&id)
        );
        candidate
            .set_observed_baseline(&id, Some(1.), Some(style), &AtomicBool::new(false))
            .unwrap();
        assert!(candidate.same_metadata(&successful));
        for (a, b) in candidate.series.iter().zip(&original.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
}

#[test]
fn automatic_value_query_is_cancellable_and_explicit_value_keeps_signed_zero() {
    let mut axes = Axes {
        yscale: AxisScale::Log,
        ..Axes::default()
    };
    axes.add(vec![Series::new(
        vec![[1., -2.], [2., -3.]],
        PlotKind::Stem,
    )])
    .unwrap();
    let id = axes.observe_series_baseline(0).unwrap().identity.clone();
    assert_eq!(
        axes.baseline_value_cancellable(&id, &AtomicBool::new(false))
            .unwrap(),
        -1.
    );
    assert!(
        axes.baseline_value_cancellable(&id, &AtomicBool::new(true))
            .is_err()
    );
    let original = axes.clone();
    assert!(
        axes.set_observed_baseline(&id, Some(1.), None, &AtomicBool::new(false))
            .is_err()
    );
    assert!(axes.same_snapshot(&original));
    axes.yscale = AxisScale::Linear;
    axes.set_observed_baseline(&id, Some(-0.), None, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        axes.baseline_value_cancellable(&id, &AtomicBool::new(false))
            .unwrap()
            .to_bits(),
        (-0f64).to_bits()
    );
}

#[test]
fn observing_at_record_capacity_is_free_but_edits_reject_without_publication() {
    let mut axes = Axes::default();
    axes.add(vec![Series::new(vec![[1., 2.]], PlotKind::Stem)])
        .unwrap();
    let records = (0..MAX_BASELINES)
        .map(|_| {
            Baseline::new(
                BaselineValue::Automatic,
                BaselineStyle::legacy_stem(),
                Vec::new(),
            )
            .unwrap()
        })
        .collect();
    axes.replace_baselines(records, &AtomicBool::new(false))
        .unwrap();
    let before = axes.clone();
    let token = axes.observe_series_baseline(0).unwrap().identity.clone();
    assert_eq!(axes.baselines.len(), MAX_BASELINES);
    assert!(
        axes.set_observed_baseline(
            &token,
            None,
            Some(BaselineStyle {
                width_points: 1.25,
                ..BaselineStyle::legacy_stem()
            }),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(axes.same_snapshot(&before));
    assert!(axes.observe_baseline(&token).is_ok());
}

fn partitioned_bar_axes() -> Axes {
    let cancel = AtomicBool::new(false);
    let mut axes = Axes {
        hold: true,
        ..Axes::default()
    };
    axes.add(vec![
        Series::new(vec![[1., 2.]], PlotKind::Bar),
        Series::new(vec![[1., 3.]], PlotKind::Stem),
    ])
    .unwrap();
    let mixed = Baseline::new(
        BaselineValue::Automatic,
        BaselineStyle::legacy_stem(),
        axes.series.iter().map(|s| s.identity.clone()).collect(),
    )
    .unwrap();
    axes.replace_baselines(vec![mixed], &cancel).unwrap();
    axes.add(vec![Series::new(vec![[2., 4.]], PlotKind::Bar)])
        .unwrap();
    axes.replace_baselines(Vec::new(), &cancel).unwrap();
    assert!(
        !axes.series[0]
            .baseline_identity
            .same_object(&axes.series[2].baseline_identity)
    );
    axes
}
fn same_baseline_partition(a: &Axes, b: &Axes) {
    let ids = |axes: &Axes| {
        axes.series
            .iter()
            .map(|s| s.baseline_identity.clone())
            .chain(axes.baselines.iter().map(|b| b.identity.clone()))
            .collect::<Vec<_>>()
    };
    let a = ids(a);
    let b = ids(b);
    assert_eq!(a.len(), b.len());
    for i in 0..a.len() {
        assert!(!a[i].same_object(&b[i]), "decoded identities must be fresh");
        for j in 0..a.len() {
            assert_eq!(a[i].same_object(&a[j]), b[i].same_object(&b[j]));
        }
    }
}
#[test]
fn native_latent_partitions_and_dormant_materialized_links_survive_two_cycles_and_subplots() {
    let cancel = AtomicBool::new(false);
    for materialized in [false, true] {
        let mut axes = partitioned_bar_axes();
        if materialized {
            let id = axes.series[0].baseline_identity.clone();
            axes.set_observed_baseline(
                &id,
                None,
                Some(BaselineStyle {
                    width_points: 1.25,
                    ..BaselineStyle::legacy_stem()
                }),
                &cancel,
            )
            .unwrap();
        }
        for subplot in [false, true] {
            let mut figure = Figure {
                appearance: None,
                ..Figure::default()
            };
            if subplot {
                figure
                    .select_subplot(Position::from_grid(1, 2, &[1]).unwrap(), false)
                    .unwrap();
            }
            figure.axes = axes.clone();
            let before = figure.clone();
            let first = crate::files::encode_native_figure(&figure).unwrap();
            assert!(first.starts_with(if subplot { b"MATRFIGQ" } else { b"MATRFIGP" }));
            let reopened = crate::files::decode_native_figure(&first).unwrap();
            same_baseline_partition(&axes, &reopened.axes);
            assert!(axes.same_metadata(&reopened.axes));
            let second = crate::files::encode_native_figure(&reopened).unwrap();
            assert_eq!(
                first, second,
                "canonical local identity labels must remain stable"
            );
            let mut twice = crate::files::decode_native_figure(&second).unwrap();
            same_baseline_partition(&axes, &twice.axes);
            for (old, new) in axes.series.iter().zip(&twice.axes.series) {
                assert_eq!(
                    old.points
                        .iter()
                        .map(|p| p.map(f64::to_bits))
                        .collect::<Vec<_>>(),
                    new.points
                        .iter()
                        .map(|p| p.map(f64::to_bits))
                        .collect::<Vec<_>>()
                );
            }
            if materialized {
                let record_id = twice.axes.baselines[0].identity.clone();
                twice.axes.replace_baselines(Vec::new(), &cancel).unwrap();
                assert!(
                    twice.axes.observe_baseline(&record_id).is_ok(),
                    "dormant latent link survives record removal"
                );
            }
            assert!(figure.same_snapshot(&before));
            let mut downgraded = first;
            downgraded[7] = if subplot { b'M' } else { b'L' };
            assert!(crate::files::decode_native_figure(&downgraded).is_err());
        }
    }
}
#[test]
fn native_latent_link_limits_and_malformed_metadata_are_checked_before_payload_decode() {
    let mut axes = Axes {
        hold: true,
        ..Axes::default()
    };
    axes.add(
        (0..MAX_SERIES)
            .map(|i| Series::new(vec![[i as f64, 2.]], PlotKind::Stem))
            .collect(),
    )
    .unwrap();
    axes.replace_baselines(
        (0..MAX_BASELINES)
            .map(|_| {
                Baseline::new(
                    BaselineValue::Automatic,
                    BaselineStyle::legacy_stem(),
                    Vec::new(),
                )
                .unwrap()
            })
            .collect(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let figure = Figure {
        axes,
        appearance: None, // Historical native schema under test.
        ..Default::default()
    };
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let metadata: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
    assert_eq!(metadata["latent_baselines"][63], 127);
    let reopened = crate::files::decode_native_figure(&bytes).unwrap();
    same_baseline_partition(&figure.axes, &reopened.axes);
    for bad in [serde_json::json!([64]), serde_json::json!(vec![128; 64])].into_iter() {
        let mut modified = metadata.clone();
        modified["latent_baselines"] = bad;
        let json = serde_json::to_vec(&modified).unwrap();
        let mut corrupt = b"MATRFIGP".to_vec();
        corrupt.extend_from_slice(&(json.len() as u32).to_le_bytes());
        corrupt.extend_from_slice(&json);
        corrupt.extend_from_slice(&bytes[12 + size..]);
        assert!(crate::files::decode_native_figure(&corrupt).is_err());
    }
    let mut modified = metadata.clone();
    modified["latent_baselines"][0] = serde_json::json!(255);
    let json = serde_json::to_vec(&modified).unwrap();
    let mut corrupt = b"MATRFIGP".to_vec();
    corrupt.extend_from_slice(&(json.len() as u32).to_le_bytes());
    corrupt.extend_from_slice(&json);
    corrupt.extend_from_slice(&bytes[12 + size..]);
    assert!(crate::files::decode_native_figure(&corrupt).is_err());
}

#[test]
fn weak_snapshot_cache_observes_group_changes_with_identical_shared_payloads() {
    let axes = partitioned_bar_axes();
    let source = Figure {
        axes: axes.clone(),
        ..Default::default()
    };
    let stamp = crate::plot::SnapshotStamp::new(&source);
    let mut regrouped = Axes {
        hold: true,
        ..Axes::default()
    };
    regrouped.add(axes.series.clone()).unwrap();
    assert!(regrouped.baselines.is_empty());
    for (a, b) in axes.series.iter().zip(&regrouped.series) {
        assert!(a.points.same_storage(&b.points));
    }
    assert!(!axes.same_metadata(&regrouped));
    assert!(!stamp.matches(&Figure {
        axes: regrouped,
        ..source.clone()
    }));
    assert!(stamp.matches(&source));
}

#[test]
fn independent_stem_record_requires_native_persistence_for_dormant_identity() {
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let mut figure = Figure::default();
    figure
        .axes
        .add(vec![Series::new(vec![[1., 2.]], PlotKind::Stem)])
        .unwrap();
    let latent = figure.axes.series[0].baseline_identity.clone();
    let record = Baseline::new(
        BaselineValue::Automatic,
        BaselineStyle::legacy_stem(),
        vec![figure.axes.series[0].identity.clone()],
    )
    .unwrap();
    let explicit = record.identity.clone();
    assert!(!latent.same_object(&explicit));
    figure
        .axes
        .replace_baselines(vec![record], &cancel)
        .unwrap();
    let before = figure.clone();
    for bundled in [false, true] {
        let path = dir
            .path()
            .join(if bundled { "bundle.m" } else { "inline.m" });
        std::fs::write(&path, b"old script remains").unwrap();
        let result = if bundled {
            crate::files::write_matlab_plot_bundle_with_cancel(&path, &figure, &cancel).map(|_| ())
        } else {
            crate::files::write_matlab_plot_script_with_cancel(&path, &figure, &cancel)
        };
        assert!(result.unwrap_err().contains("dormant baseline"));
        assert_eq!(std::fs::read(&path).unwrap(), b"old script remains");
        assert!(figure.same_snapshot(&before));
    }
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    let mut reopened = crate::files::decode_native_figure(&bytes).unwrap();
    let fresh_record = reopened.axes.baselines[0].identity.clone();
    let fresh_latent = reopened.axes.series[0].baseline_identity.clone();
    assert!(!fresh_record.same_object(&fresh_latent));
    reopened
        .axes
        .replace_baselines(Vec::new(), &cancel)
        .unwrap();
    assert!(reopened.axes.observe_baseline(&fresh_record).is_err());
    assert!(
        reopened
            .axes
            .observe_series_baseline(0)
            .unwrap()
            .identity
            .same_object(&fresh_latent)
    );
    // The older ordinary Bar/Patch reconstruction scope is not narrowed.
    let mut ordinary = Figure {
        axes: partitioned_bar_axes(),
        ..Default::default()
    };
    // Explicit historical fill-only geometry retains the old Patch fallback.
    for series in &mut ordinary.series {
        series.bar_baseline_visible = false;
    }
    crate::files::write_matlab_plot_script_with_cancel(
        &dir.path().join("ordinary.m"),
        &ordinary,
        &cancel,
    )
    .unwrap();
}

fn materialized_three_bars() -> Axes {
    let mut axes = Axes {
        hold: true,
        ..Axes::default()
    };
    axes.add(
        (0..3)
            .map(|i| Series::new(vec![[i as f64, 2.]], PlotKind::Bar))
            .collect(),
    )
    .unwrap();
    axes.add(vec![Series::new(vec![[1., 3.]], PlotKind::Stem)])
        .unwrap();
    let token = axes.series[0].baseline_identity.clone();
    axes.set_observed_baseline(
        &token,
        None,
        Some(BaselineStyle {
            width_points: 1.25,
            ..BaselineStyle::legacy_stem()
        }),
        &AtomicBool::new(false),
    )
    .unwrap();
    axes
}
#[test]
fn membership_edits_rekey_only_omitted_subgroups_and_cancel_before_any_publication() {
    let original = materialized_three_bars();
    for standalone in [false, true] {
        let mut record = original.baselines[0].clone();
        if standalone {
            record.members.clear();
        } else {
            record.members.truncate(1);
        }
        let checks = Cell::new(0);
        let mut changed = original.clone();
        changed
            .replace_baselines_checked(vec![record.clone()], &|| {
                checks.set(checks.get() + 1);
                Ok(())
            })
            .unwrap();
        for stop in 1..=checks.get() {
            let mut attempt = original.clone();
            let seen = Cell::new(0);
            let error = attempt
                .replace_baselines_checked(vec![record.clone()], &|| {
                    seen.set(seen.get() + 1);
                    if seen.get() == stop {
                        Err("Stop during membership plan".into())
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error, "Stop during membership plan");
            assert!(attempt.same_snapshot(&original));
            for (a, b) in attempt.series.iter().zip(&original.series) {
                assert!(a.baseline_identity.same_object(&b.baseline_identity));
                assert!(a.points.same_storage(&b.points));
            }
            assert!(
                attempt.baselines[0]
                    .identity
                    .same_object(&original.baselines[0].identity)
            );
        }
        let old = &original.baselines[0].identity;
        assert!(changed.baselines[0].identity.same_object(old));
        assert!(
            changed.series[1]
                .baseline_identity
                .same_object(&changed.series[2].baseline_identity)
        );
        assert!(!changed.series[1].baseline_identity.same_object(old));
        assert_eq!(
            changed.series[0].baseline_identity.same_object(old),
            !standalone
        );
        assert!(
            changed.series[3]
                .baseline_identity
                .same_object(&original.series[3].baseline_identity)
        );
        for (a, b) in changed.series.iter().zip(&original.series) {
            assert!(a.points.same_storage(&b.points));
        }
        assert!(changed.observe_baseline(old).is_ok());
        assert!(changed.observe_series_baseline(1).unwrap().style.visible);
        changed.validate().unwrap();
        let figure = Figure {
            axes: changed.clone(),
            ..Default::default()
        };
        let opened = crate::files::decode_native_figure(
            &crate::files::encode_native_figure(&figure).unwrap(),
        )
        .unwrap();
        same_baseline_partition(&changed, &opened.axes);
    }
}
#[test]
fn materialization_rekeys_dormant_members_bound_to_a_different_record() {
    let cancel = AtomicBool::new(false);
    let mut axes = Axes {
        hold: true,
        ..Axes::default()
    };
    axes.add(
        (0..3)
            .map(|i| Series::new(vec![[i as f64, 2.]], PlotKind::Bar))
            .collect(),
    )
    .unwrap();
    let group = axes.series[0].baseline_identity.clone();
    let unrelated = Baseline::new(
        BaselineValue::Automatic,
        BaselineStyle::legacy_stem(),
        vec![axes.series[0].identity.clone()],
    )
    .unwrap();
    let keep = unrelated.identity.clone();
    axes.replace_baselines(vec![unrelated], &cancel).unwrap();
    let before = axes.clone();
    axes.set_observed_baseline(
        &group,
        None,
        Some(BaselineStyle {
            width_points: 1.25,
            ..BaselineStyle::legacy_stem()
        }),
        &cancel,
    )
    .unwrap();
    assert!(axes.baselines[0].identity.same_object(&keep));
    assert!(axes.baselines[1].identity.same_object(&group));
    assert!(!axes.series[0].baseline_identity.same_object(&group));
    assert!(axes.series[1].baseline_identity.same_object(&group));
    assert!(axes.series[2].baseline_identity.same_object(&group));
    for (a, b) in axes.series.iter().zip(&before.series) {
        assert!(a.points.same_storage(&b.points));
    }
    axes.replace_baselines(vec![axes.baselines[1].clone()], &cancel)
        .unwrap();
    assert!(
        !axes
            .observe_series_baseline(0)
            .unwrap()
            .identity
            .same_object(&group)
    );
    assert!(
        axes.observe_series_baseline(1)
            .unwrap()
            .identity
            .same_object(&group)
    );
    axes.validate().unwrap();
}
#[test]
fn native_identity_collision_outside_explicit_membership_is_rejected_not_repaired() {
    let mut axes = materialized_three_bars();
    let mut record = axes.baselines[0].clone();
    record.members.truncate(1);
    axes.replace_baselines(vec![record], &AtomicBool::new(false))
        .unwrap();
    let figure = Figure {
        axes,
        appearance: None, // Historical native schema under test.
        ..Default::default()
    };
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut metadata: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
    metadata["latent_baselines"][1] = serde_json::json!(0);
    let json = serde_json::to_vec(&metadata).unwrap();
    let mut corrupt = b"MATRFIGP".to_vec();
    corrupt.extend_from_slice(&(json.len() as u32).to_le_bytes());
    corrupt.extend_from_slice(&json);
    corrupt.extend_from_slice(&bytes[12 + size..]);
    assert!(
        crate::files::decode_native_figure(&corrupt)
            .unwrap_err()
            .contains("outside explicit membership")
    );
    let mut invalid = figure.clone();
    invalid.axes.series[1].baseline_identity = invalid.axes.baselines[0].identity.clone();
    assert!(
        invalid
            .validate()
            .unwrap_err()
            .contains("conflicts with explicit membership")
    );
    assert!(invalid.axes.observe_series_baseline(1).is_err());
    assert!(figure.validate().is_ok());
}

#[test]
fn new_bar_default_is_pure_deduplicated_and_legacy_native_absence_stays_off() {
    let mut axes = charts();
    let original = axes.clone();
    assert_eq!(axes.implicit_bar_baselines().collect::<Vec<_>>(), vec![0]);
    let token = axes.series[0].baseline_identity.clone();
    assert!(axes.observe_baseline(&token).unwrap().style.visible);
    assert!(axes.baselines.is_empty());
    assert!(axes.same_snapshot(&original));
    // Encode an explicitly historical state with no new optional wire field.
    for series in &mut axes.series {
        series.bar_baseline_visible = false;
    }
    let legacy = Figure {
        axes,
        appearance: None, // Historical native schema under test.
        ..Default::default()
    };
    let bytes = crate::files::encode_native_figure(&legacy).unwrap();
    assert!(
        !bytes
            .windows(b"bar_baseline_visible".len())
            .any(|v| v == b"bar_baseline_visible")
    );
    assert!(!bytes.starts_with(b"MATRFIGP"));
    let mut reopened = crate::files::decode_native_figure(&bytes).unwrap();
    assert_eq!(
        crate::files::encode_native_figure(&reopened).unwrap(),
        bytes
    );
    assert!(reopened.implicit_bar_baselines().next().is_none());
    assert!(!reopened.observe_series_baseline(0).unwrap().style.visible);
    reopened.hold = true;
    reopened
        .add(vec![Series::new(vec![[5., 6.]], PlotKind::Bar)])
        .unwrap();
    assert!(!reopened.observe_series_baseline(4).unwrap().style.visible);
    assert!(reopened.baselines.is_empty());
    reopened.series.retain(|s| s.kind != PlotKind::Bar);
    reopened
        .add(vec![Series::new(vec![[5., 6.]], PlotKind::Bar)])
        .unwrap();
    assert!(reopened.observe_series_baseline(2).unwrap().style.visible);
    assert!(reopened.baselines.is_empty());
}

#[test]
fn implicit_on_native_capability_preserves_policy_and_rejects_older_headers() {
    for subplot in [false, true] {
        let mut figure = Figure {
            appearance: None,
            axes: Axes::default(),
            ..Figure::default()
        };
        if subplot {
            figure
                .select_subplot(Position::from_grid(1, 2, &[1]).unwrap(), false)
                .unwrap();
        }
        figure
            .add(vec![Series::new(vec![[1., 2.], [2., 3.]], PlotKind::Bar)])
            .unwrap();
        let original = figure.clone();
        for _ in 0..2 {
            let bytes = crate::files::encode_native_figure(&figure).unwrap();
            assert!(bytes.starts_with(if subplot { b"MATRFIGQ" } else { b"MATRFIGP" }));
            let reopened = crate::files::decode_native_figure(&bytes).unwrap();
            assert!(reopened.same_metadata(&figure));
            assert_eq!(reopened.svg(), figure.svg());
            assert!(reopened.baselines.is_empty());
            assert_eq!(reopened.implicit_bar_baselines().count(), 1);
            let mut downgraded = bytes;
            downgraded[7] = if subplot { b'O' } else { b'N' };
            assert!(crate::files::decode_native_figure(&downgraded).is_err());
            figure = reopened;
        }
        assert!(original.series[0].bar_baseline_visible);
    }
    let mut figure = Figure {
        appearance: None,
        axes: Axes::default(),
        ..Figure::default()
    };
    figure
        .add(vec![
            Series::new(vec![[1., 2.]], PlotKind::Bar),
            Series::new(vec![[2., 3.]], PlotKind::Bar),
        ])
        .unwrap();
    let bytes = crate::files::encode_native_figure(&figure).unwrap();
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut metadata: serde_json::Value = serde_json::from_slice(&bytes[12..12 + size]).unwrap();
    metadata["series"][1]["bar_baseline_visible"] = serde_json::json!(false);
    let raw = serde_json::to_vec(&metadata).unwrap();
    let mut malformed = b"MATRFIGP".to_vec();
    malformed.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    malformed.extend_from_slice(&raw);
    // No sample payload: this precise policy error must precede sample reads.
    assert!(
        crate::files::decode_native_figure(&malformed)
            .unwrap_err()
            .contains("conflicting visibility")
    );
}

#[test]
fn removing_differently_visible_records_splits_only_omitted_group_atomically() {
    let cancel = AtomicBool::new(false);
    let mut original = Axes {
        hold: true,
        ..Axes::default()
    };
    original
        .add(
            (0..3)
                .map(|i| Series::new(vec![[i as f64, 2.]], PlotKind::Bar))
                .collect(),
        )
        .unwrap();
    let old = original.series[0].baseline_identity.clone();
    let on = Baseline::new(
        BaselineValue::Automatic,
        BaselineStyle::legacy_stem(),
        vec![original.series[0].identity.clone()],
    )
    .unwrap();
    let off = Baseline::new(
        BaselineValue::Automatic,
        BaselineStyle {
            visible: false,
            ..BaselineStyle::legacy_stem()
        },
        vec![
            original.series[1].identity.clone(),
            original.series[2].identity.clone(),
        ],
    )
    .unwrap();
    original.replace_baselines(vec![on, off], &cancel).unwrap();
    let calls = Cell::new(0);
    let mut changed = original.clone();
    changed
        .replace_baselines_checked(Vec::new(), &|| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
    assert!(changed.series[0].baseline_identity.same_object(&old));
    assert!(!changed.series[1].baseline_identity.same_object(&old));
    assert!(
        changed.series[1]
            .baseline_identity
            .same_object(&changed.series[2].baseline_identity)
    );
    assert!(changed.observe_series_baseline(0).unwrap().style.visible);
    assert!(!changed.observe_series_baseline(1).unwrap().style.visible);
    assert_eq!(changed.implicit_bar_baselines().count(), 1);
    changed.validate().unwrap();
    for stop in 1..=calls.get() {
        let mut attempt = original.clone();
        let seen = Cell::new(0);
        assert!(
            attempt
                .replace_baselines_checked(Vec::new(), &|| {
                    seen.set(seen.get() + 1);
                    if seen.get() == stop {
                        Err("policy Stop".into())
                    } else {
                        Ok(())
                    }
                })
                .is_err()
        );
        assert!(attempt.same_snapshot(&original));
        for (a, b) in attempt.series.iter().zip(&original.series) {
            assert!(a.points.same_storage(&b.points));
            assert!(a.baseline_identity.same_object(&b.baseline_identity));
        }
    }
    let figure = Figure {
        axes: changed,
        ..Default::default()
    };
    let reopened =
        crate::files::decode_native_figure(&crate::files::encode_native_figure(&figure).unwrap())
            .unwrap();
    assert!(figure.same_metadata(&reopened));
}

#[test]
fn implicit_visibility_invalidates_weak_snapshot_without_changing_geometry_or_samples() {
    let mut figure = Figure::default();
    figure
        .add(vec![Series::new(vec![[1., 2.], [2., 3.]], PlotKind::Bar)])
        .unwrap();
    let before = figure.clone();
    let stamp = crate::plot::SnapshotStamp::new(&figure);
    let bounds = figure.bounds();
    let ledger = figure.geometry_ledger().unwrap().cost();
    figure.series[0].bar_baseline_visible = false;
    assert!(!stamp.matches(&figure));
    assert!(!figure.same_metadata(&before));
    assert_eq!(figure.bounds(), bounds);
    assert_eq!(figure.geometry_ledger().unwrap().cost(), ledger);
    assert!(
        figure.series[0]
            .points
            .same_storage(&before.series[0].points)
    );
    assert!(before.svg().contains("data-matrust-implicit-bar-baseline"));
    assert!(!figure.svg().contains("data-matrust-implicit-bar-baseline"));
    assert!(figure.baselines.is_empty());
}

#[test]
fn visibility_only_public_transaction_is_latent_atomic_and_reversible() {
    let original = charts();
    let token = original.series[0].baseline_identity.clone();
    let off = BaselineStyle {
        visible: false,
        ..BaselineStyle::legacy_stem()
    };
    let calls = Cell::new(0);
    let mut hidden = original.clone();
    hidden
        .set_observed_baseline_checked(&token, None, Some(off), &|| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
    assert!(hidden.baselines.is_empty());
    assert_eq!(hidden.implicit_bar_baselines().count(), 0);
    for index in [0, 1] {
        assert!(!hidden.observe_series_baseline(index).unwrap().style.visible);
    }
    for stop in 1..=calls.get() {
        let mut attempt = original.clone();
        let seen = Cell::new(0);
        assert!(
            attempt
                .set_observed_baseline_checked(&token, None, Some(off), &|| {
                    seen.set(seen.get() + 1);
                    if seen.get() == stop {
                        Err("visibility Stop".into())
                    } else {
                        Ok(())
                    }
                })
                .is_err()
        );
        assert!(attempt.same_snapshot(&original));
    }
    hidden
        .set_observed_baseline(
            &token,
            None,
            Some(BaselineStyle::legacy_stem()),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(hidden.same_snapshot(&original));
    assert!(hidden.baselines.is_empty());
    for (a, b) in hidden.series.iter().zip(&original.series) {
        assert!(a.points.same_storage(&b.points));
    }
}
