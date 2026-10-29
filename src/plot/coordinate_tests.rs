use super::*;
use std::sync::atomic::Ordering;

fn assert_coordinate(actual: [f64; 2], expected: [f64; 2]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        if expected.is_nan() {
            assert!(actual.is_nan());
        } else {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
}

fn corners(
    x: CoordinateLane,
    y: CoordinateLane,
    offset: f64,
    half: f64,
    base: f64,
) -> [[Coordinate; 2]; 4] {
    let left = x
        .clone()
        .then(ScalarStep::Add(offset))
        .unwrap()
        .then(ScalarStep::Subtract(half))
        .unwrap();
    let right = x
        .then(ScalarStep::Add(offset))
        .unwrap()
        .then(ScalarStep::Add(half))
        .unwrap();
    [
        [Coordinate::Lane(left.clone()), Coordinate::Constant(base)],
        [Coordinate::Lane(right.clone()), Coordinate::Constant(base)],
        [Coordinate::Lane(right), Coordinate::Lane(y.clone())],
        [Coordinate::Lane(left), Coordinate::Lane(y)],
    ]
}
fn matrices(view: &PatchCoordinates) -> (Matrix, Matrix) {
    let points = view.materialize(&AtomicBool::new(false)).unwrap();
    (
        Matrix::from_column_vec(
            view.corners_per_face(),
            view.face_count(),
            points.iter().map(|p| p[0]).collect(),
        )
        .unwrap(),
        Matrix::from_column_vec(
            view.corners_per_face(),
            view.face_count(),
            points.iter().map(|p| p[1]).collect(),
        )
        .unwrap(),
    )
}
fn fixed_view(count: usize) -> PatchCoordinates {
    let points = Arc::new((0..count).map(|i| [i as f64, (i % 7) as f64]).collect());
    PatchCoordinates::rectangles(
        corners(
            CoordinateLane::pair_component(&points, 0).unwrap(),
            CoordinateLane::pair_component(&points, 1).unwrap(),
            0.25,
            0.4,
            -0.0,
        ),
        count,
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn ordered_shared_coordinates_match_eager_bits_and_dense_both_windings() {
    let stop = AtomicBool::new(false);
    let xs = [
        -0.0,
        0.0,
        f64::from_bits(1),
        -f64::from_bits(1),
        1.0e16,
        -1.0e16,
        f64::MAX,
        -f64::MAX,
        f64::INFINITY,
        f64::NAN,
    ];
    let ys = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        2.0,
        -2.0,
        f64::MAX,
        -f64::MAX,
        f64::NAN,
        3.0,
    ];
    let matrix = Matrix::from_column_vec(xs.len(), 2, xs.into_iter().chain(ys).collect()).unwrap();
    let alias = matrix.clone();
    for (offset, half, base) in [(1.0, 1.0, -0.0), (-0.0, 0.0, 1.0), (1.0e308, 1.0e307, -1.0)] {
        let x = CoordinateLane::matrix_column(&matrix, 0).unwrap();
        let y = CoordinateLane::matrix_column(&matrix, 1).unwrap();
        let mut view_corners = corners(x, y, offset, half, base);
        for reverse in [false, true] {
            if reverse {
                view_corners.reverse();
            }
            let view = PatchCoordinates::rectangles(view_corners.clone(), xs.len(), &stop).unwrap();
            assert!(view.same_snapshot(&view.clone()));
            let mut expected = Vec::new();
            for (x, y) in xs.into_iter().zip(ys) {
                let mut face = [
                    [(x + offset) - half, base],
                    [(x + offset) + half, base],
                    [(x + offset) + half, y],
                    [(x + offset) - half, y],
                ];
                if reverse {
                    face.reverse();
                }
                expected.extend(face);
            }
            let dense = PatchCoordinates::dense(Arc::new(expected.clone()), 4, &stop).unwrap();
            assert!(dense.same_snapshot(&dense.clone()));
            assert!(!dense.same_snapshot(&view));
            let expanded = view.materialize(&stop).unwrap();
            assert_eq!(expanded.len(), expected.len());
            for (actual, expected) in expanded.iter().zip(&expected) {
                // Derived NaN payloads are not portable Rust arithmetic outputs.
                // Finite values, signed zero and infinities remain bit-exact.
                assert_coordinate(*actual, *expected);
            }
            for (a, b) in dense.materialize(&stop).unwrap().iter().zip(&expanded) {
                assert_coordinate(*a, *b);
            }
            assert_eq!(view.face_count(), xs.len());
            assert_eq!(view.cost().source_units(), xs.len());
            assert_eq!(
                view.cost().retained_bytes(),
                matrix.storage.data.as_vec().capacity() * 8
            );
            assert!(view.corner(xs.len(), 0).is_none());
            assert!(view.corner(0, 4).is_none());
        }
    }
    // Addition then subtraction may not be replaced by x+(offset-half).
    let ordered = CoordinateLane::matrix_column(&matrix, 0)
        .unwrap()
        .then(ScalarStep::Add(-1.0e16))
        .unwrap()
        .then(ScalarStep::Subtract(1.0))
        .unwrap();
    assert_ne!(
        ordered.value(4).unwrap().to_bits(),
        (xs[4] + (-1.0e16 - 1.0)).to_bits()
    );
    assert!(Arc::ptr_eq(&matrix.storage, &alias.storage));
    assert_eq!(
        matrix
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        xs.into_iter()
            .chain(ys)
            .map(f64::to_bits)
            .collect::<Vec<_>>()
    );
}

#[test]
fn descriptors_bound_depth_sources_shapes_and_geometry() {
    let stop = AtomicBool::new(false);
    let matrix = Matrix::from_column_vec(1, 3, vec![1.0, 2.0, 3.0]).unwrap();
    assert!(CoordinateLane::matrix_column(&matrix, 3).is_err());
    let lane = CoordinateLane::matrix_column(&matrix, 0).unwrap();
    let two = lane
        .clone()
        .then(ScalarStep::Multiply(-0.0))
        .unwrap()
        .then(ScalarStep::Add(0.0))
        .unwrap();
    assert!(two.then(ScalarStep::Add(1.0)).is_err());
    let mut value = corners(
        lane,
        CoordinateLane::matrix_column(&matrix, 1).unwrap(),
        0.,
        1.,
        0.,
    );
    value[3][1] = Coordinate::Lane(CoordinateLane::matrix_column(&matrix, 2).unwrap());
    assert!(
        PatchCoordinates::rectangles(value, 1, &stop)
            .unwrap_err()
            .contains("two independent")
    );
    let view = fixed_view(2);
    let Representation::Rectangles { corners, .. } = view.representation else {
        unreachable!()
    };
    assert!(PatchCoordinates::rectangles(*corners, 1, &stop).is_err());
    let bad = [
        [Coordinate::Constant(0.), Coordinate::Constant(0.)],
        [Coordinate::Constant(1.), Coordinate::Constant(1.)],
        [Coordinate::Constant(2.), Coordinate::Constant(0.)],
        [Coordinate::Constant(0.), Coordinate::Constant(2.)],
    ];
    assert!(PatchCoordinates::rectangles(bad, 1, &stop).is_err());
    assert!(PatchCoordinates::dense(Arc::new(vec![[0., 0.]; 4]), 3, &stop).is_err());
    assert!(PatchCoordinates::dense(Arc::new(vec![[0., 0.]; 5]), 5, &stop).is_err());
}

#[test]
fn full_backing_capacity_is_charged_and_empty_views_are_bounded() {
    let stop = AtomicBool::new(false);
    let mut values = Vec::with_capacity(4096);
    values.extend([1.0, 2.0, 99.0]);
    let matrix = Matrix::from_column_vec(1, 3, values).unwrap();
    let view = PatchCoordinates::rectangles(
        corners(
            CoordinateLane::matrix_column(&matrix, 0).unwrap(),
            CoordinateLane::matrix_column(&matrix, 1).unwrap(),
            0.,
            1.,
            0.,
        ),
        1,
        &stop,
    )
    .unwrap();
    assert_eq!(view.cost().retained_bytes(), 4096 * 8);
    assert_eq!(view.cost().source_units(), 2048);
    let mut points = Vec::with_capacity(MAX_SOURCE_UNITS + 1);
    points.push([1.0, 2.0]);
    let points = Arc::new(points);
    assert!(CoordinateLane::pair_component(&points, 0).is_err());
    assert!(GeometryCost::ordinary(&points).is_err());
    let empty = Matrix::from_column_vec(0, 2, Vec::new()).unwrap();
    let view = PatchCoordinates::rectangles(
        corners(
            CoordinateLane::matrix_column(&empty, 0).unwrap(),
            CoordinateLane::matrix_column(&empty, 1).unwrap(),
            0.,
            1.,
            -0.,
        ),
        0,
        &stop,
    )
    .unwrap();
    assert_eq!(view.face_count(), 0);
    assert_eq!(view.cost().source_units(), 0);
    assert!(view.materialize(&stop).unwrap().is_empty());
    assert!(view.corner(0, 0).is_none());
}

#[test]
fn two_bank_maximum_scenes_are_exact_bounded_and_add_is_atomic() {
    // Counts-only private constructor isolates ledger boundaries without large
    // allocation fixtures. Public callers cannot manufacture these costs.
    for width in [3, 4] {
        let line_count = MAX_SOURCE_UNITS - MAX_RECTANGLES - width * MAX_GENERAL_FACES;
        let line =
            GeometryCost::from_counts(line_count, line_count, 0, 0, line_count * 16).unwrap();
        let rectangles = GeometryCost::from_counts(
            MAX_RECTANGLES,
            MAX_RECTANGLES * 4,
            MAX_RECTANGLES,
            0,
            MAX_RECTANGLES * 16,
        )
        .unwrap();
        let dense = GeometryCost::from_counts(
            width * MAX_GENERAL_FACES,
            width * MAX_GENERAL_FACES,
            0,
            MAX_GENERAL_FACES,
            width * MAX_GENERAL_FACES * 16,
        )
        .unwrap();
        let mut a = GeometryLedger::default();
        for cost in [line, rectangles, dense] {
            a.add(cost).unwrap();
        }
        let mut b = GeometryLedger::default();
        for cost in [dense, rectangles, line] {
            b.add(cost).unwrap();
        }
        assert_eq!(a, b);
        assert_eq!(a.cost().source_units(), MAX_SOURCE_UNITS);
        assert_eq!(a.cost().expanded_vertices(), MAX_EXPANDED_VERTICES);
        assert_eq!(a.cost().retained_bytes(), MAX_RETAINED_BYTES);
        assert_eq!(
            a.cost().compact_rectangles() + a.cost().general_faces(),
            20_000
        );
        let before = a;
        assert!(
            a.add(GeometryCost::from_counts(1, 1, 0, 0, 16).unwrap())
                .is_err()
        );
        assert_eq!(a, before);
    }
    let one = fixed_view(1).cost();
    let mut ledger = GeometryLedger::default();
    for _ in 0..64 {
        ledger.add(one).unwrap();
    }
    let before = ledger;
    assert!(ledger.add(one).is_err());
    assert_eq!(ledger, before);
    assert_eq!(ledger.cost().source_units(), one.source_units() * 64);
    // Sharing never makes the second object's logical geometry free.
    assert_eq!(ledger.cost().retained_bytes(), one.retained_bytes() * 64);
    assert!(GeometryCost::from_counts(10_001, 40_004, 10_001, 0, 160_016).is_err());
    assert!(GeometryCost::from_counts(30_003, 30_003, 0, 10_001, 480_048).is_err());
    assert!(GeometryCost::from_counts(1, 1, 0, 0, 17).is_err());
    assert!(GeometryCost::from_counts(1, 3, 1, 0, 16).is_err());
    assert!(GeometryCost::from_counts(3, 2, 0, 1, 48).is_err());
}

#[test]
fn active_cancellation_during_validation_materialization_and_proof_is_atomic() {
    let view = fixed_view(1024);
    let (x, y) = matrices(&view);
    for stage in 0..3 {
        let stop = AtomicBool::new(false);
        let mut calls = 0;
        let mut check = || {
            calls += 1;
            if calls == 3 {
                stop.store(true, Ordering::Relaxed);
            }
            crate::dsp::check_cancel(&stop)
        };
        let error = match stage {
            0 => {
                let Representation::Rectangles { corners, count } = view.representation.clone()
                else {
                    unreachable!()
                };
                PatchCoordinates::rectangles_checked(*corners, count, &mut check).unwrap_err()
            }
            1 => view.materialize_checked(&mut check).unwrap_err(),
            _ => VerifiedPatchInputs::new_checked(
                x.clone(),
                y.clone(),
                view.clone(),
                usize::MAX,
                &mut check,
            )
            .unwrap_err(),
        };
        assert_eq!(error, "Calculation cancelled");
        assert_eq!(calls, 3);
        stop.store(false, Ordering::Relaxed);
        let retry = VerifiedPatchInputs::new(x.clone(), y.clone(), view.clone(), usize::MAX, &stop)
            .unwrap();
        assert!(Arc::ptr_eq(&retry.x().storage, &x.storage));
        assert_eq!(retry.proof().face_count(), 1024);
    }
    let stop = AtomicBool::new(true);
    assert_eq!(
        view.materialize(&stop).unwrap_err(),
        "Calculation cancelled"
    );
    let dense = Arc::new(view.materialize(&AtomicBool::new(false)).unwrap());
    let mut calls = 0;
    assert!(
        PatchCoordinates::dense_checked(dense.clone(), 4, || {
            calls += 1;
            if calls == 3 {
                Err("Stop after work".into())
            } else {
                Ok(())
            }
        })
        .is_err()
    );
    assert_eq!(calls, 3);
    assert_eq!(dense.len(), 4096);
}

#[test]
fn eager_proof_accounts_hidden_backing_and_drops_it_for_unsupported_operations() {
    let stop = AtomicBool::new(false);
    let source = Arc::new(vec![[1.0, -0.0], [2.0, 3.0]]);
    let weak = Arc::downgrade(&source);
    let proof = PatchCoordinates::rectangles(
        corners(
            CoordinateLane::pair_component(&source, 0).unwrap(),
            CoordinateLane::pair_component(&source, 1).unwrap(),
            0.,
            0.25,
            -0.,
        ),
        2,
        &stop,
    )
    .unwrap();
    let (x, y) = matrices(&proof);
    let visible_bytes =
        (x.storage.data.as_vec().capacity() + y.storage.data.as_vec().capacity()) * 8;
    let total = visible_bytes + source.capacity() * 16 + proof.descriptor_bytes();
    assert!(
        VerifiedPatchInputs::new(x.clone(), y.clone(), proof.clone(), total - 1, &stop).is_err()
    );
    let envelope =
        VerifiedPatchInputs::new(x.clone(), y.clone(), proof.clone(), total, &stop).unwrap();
    assert_eq!(envelope.workspace_bytes(), total);
    let wrong = x.with_value(0, 0, x.get(0, 0) + 1.0).unwrap();
    assert!(VerifiedPatchInputs::new(wrong, y.clone(), proof.clone(), usize::MAX, &stop).is_err());
    let wrong_zero = y.with_value(0, 0, 0.0).unwrap();
    assert!(
        VerifiedPatchInputs::new(x.clone(), wrong_zero, proof.clone(), usize::MAX, &stop).is_err()
    );
    drop(source);
    drop(proof);
    assert!(weak.upgrade().is_some());
    let (ordinary_x, ordinary_y) = envelope.into_dense();
    assert!(weak.upgrade().is_none());
    assert!(Arc::ptr_eq(&ordinary_x.storage, &x.storage));
    assert!(Arc::ptr_eq(&ordinary_y.storage, &y.storage));
}

#[test]
fn matrix_aliases_in_visible_inputs_remain_logically_charged_twice() {
    let stop = AtomicBool::new(false);
    let source = Matrix::from_column_vec(4, 1, vec![0., 0., 0., 0.]).unwrap();
    let proof = PatchCoordinates::rectangles(
        std::array::from_fn(|_| [Coordinate::Constant(0.), Coordinate::Constant(0.)]),
        1,
        &stop,
    )
    .unwrap();
    let metadata = proof.descriptor_bytes();
    let envelope =
        VerifiedPatchInputs::new(source.clone(), source.clone(), proof, usize::MAX, &stop).unwrap();
    assert_eq!(
        envelope.workspace_bytes(),
        source.storage.data.as_vec().capacity() * 16 + metadata
    );
    assert!(Arc::ptr_eq(&envelope.x().storage, &envelope.y().storage));
}

#[test]
fn collapsed_rectangles_and_multiplied_zero_preserve_all_original_bits() {
    let stop = AtomicBool::new(false);
    let source = Arc::new(vec![
        [1.0e300, -3.0],
        [1.0, -0.0],
        [0.0, f64::NAN],
        [-1.0e300, f64::INFINITY],
    ]);
    for width in [f64::from_bits(1), 1.0e-300, 1.0] {
        let half = width / 2.0;
        let x = CoordinateLane::pair_component(&source, 0).unwrap();
        let y = CoordinateLane::pair_component(&source, 1).unwrap();
        let left = x.clone().then(ScalarStep::Subtract(half)).unwrap();
        let right = x.then(ScalarStep::Add(half)).unwrap();
        let base = y.clone().then(ScalarStep::Multiply(0.0)).unwrap();
        let view = PatchCoordinates::rectangles(
            [
                [
                    Coordinate::Lane(left.clone()),
                    Coordinate::Lane(base.clone()),
                ],
                [Coordinate::Lane(right.clone()), Coordinate::Lane(base)],
                [Coordinate::Lane(right), Coordinate::Lane(y.clone())],
                [Coordinate::Lane(left), Coordinate::Lane(y)],
            ],
            source.len(),
            &stop,
        )
        .unwrap();
        assert_eq!(
            view.corner(0, 0).unwrap()[0].to_bits(),
            view.corner(0, 1).unwrap()[0].to_bits()
        );
        assert_eq!(view.corner(0, 0).unwrap()[1].to_bits(), (-0.0f64).to_bits());
        assert!(view.corner(2, 0).unwrap()[1].is_nan());
        for (face, [x, y]) in source.iter().copied().enumerate() {
            let expected = [
                [x - half, y * 0.0],
                [x + half, y * 0.0],
                [x + half, y],
                [x - half, y],
            ];
            for (corner, pair) in expected.iter().enumerate() {
                assert_coordinate(view.corner(face, corner).unwrap(), *pair);
            }
        }
        // Neither degeneracy nor a missing/nonfinite face rejects the stored
        // geometry. Render eligibility remains a whole-face consumer decision.
        let dense =
            PatchCoordinates::dense(Arc::new(view.materialize(&stop).unwrap()), 4, &stop).unwrap();
        assert_eq!(dense.face_count(), source.len());
    }
}

#[test]
fn matrix_lane_retention_prevents_in_place_mutation_and_cow_keeps_the_old_view() {
    let mut source = Matrix::from_column_vec(2, 1, vec![1.0, -0.0]).unwrap();
    let lane = CoordinateLane::matrix_column(&source, 0).unwrap();
    assert!(lane.same_snapshot(&lane.clone()));
    assert!(!lane.same_snapshot(&lane.clone().then(ScalarStep::Add(0.0)).unwrap()));
    assert!(!source.write_unique(&[(0, 7.0)]));
    let edited = source.with_value(0, 0, 9.0).unwrap();
    assert_eq!(lane.value(0).unwrap().to_bits(), 1.0f64.to_bits());
    assert_eq!(lane.value(1).unwrap().to_bits(), (-0.0f64).to_bits());
    assert_eq!(edited.get(0, 0), 9.0);
    assert!(!Arc::ptr_eq(&source.storage, &edited.storage));
    drop(lane);
    assert!(source.write_unique(&[(0, 7.0)]));
    assert_eq!(source.get(0, 0), 7.0);
}
