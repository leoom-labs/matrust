use super::*;
use crate::plot::coordinates::{Coordinate, CoordinateLane, ScalarStep};
use std::sync::atomic::Ordering;

fn rectangles(source: &Arc<Vec<[f64; 2]>>, stop: &AtomicBool) -> SeriesPoints {
    let x = CoordinateLane::pair_component(source, 0).unwrap();
    let y = CoordinateLane::pair_component(source, 1).unwrap();
    let left = Coordinate::Lane(
        x.clone()
            .then(ScalarStep::Add(0.25))
            .unwrap()
            .then(ScalarStep::Subtract(0.4))
            .unwrap(),
    );
    let right = Coordinate::Lane(
        x.then(ScalarStep::Add(0.25))
            .unwrap()
            .then(ScalarStep::Add(0.4))
            .unwrap(),
    );
    let bottom = Coordinate::Lane(y.clone().then(ScalarStep::Multiply(0.0)).unwrap());
    let top = Coordinate::Lane(y);
    SeriesPoints::Patch(Arc::new(
        PatchCoordinates::rectangles(
            [
                [left.clone(), bottom.clone()],
                [right.clone(), bottom],
                [right, top.clone()],
                [left, top],
            ],
            source.len(),
            stop,
        )
        .unwrap(),
    ))
}

#[test]
fn compact_points_are_real_ordered_values_and_shared_storage_not_empty_dense_placeholders() {
    let stop = AtomicBool::new(false);
    let source = Arc::new(vec![
        [-0.0, -0.0],
        [1.0e16, f64::from_bits(1)],
        [2.0, f64::from_bits(0x7ff8_1234_0000_0001)],
    ]);
    let points = rectangles(&source, &stop);
    let alias = points.clone();
    assert_eq!(points.len(), 12);
    assert!(!points.is_empty());
    assert!(points.dense().is_none());
    assert!(points.same_storage(&alias));
    let mut expected = Vec::new();
    for &[x, y] in source.iter() {
        let left = (x + 0.25) - 0.4;
        let right = (x + 0.25) + 0.4;
        expected.extend([[left, y * 0.0], [right, y * 0.0], [right, y], [left, y]]);
    }
    let expanded = points.materialize(&stop).unwrap();
    for (actual, expected) in expanded.iter().zip(&expected) {
        for axis in 0..2 {
            if expected[axis].is_nan() && actual[axis].is_nan() {
                continue;
            }
            assert_eq!(actual[axis].to_bits(), expected[axis].to_bits());
        }
    }
    assert_eq!(points.get(10).unwrap()[1].to_bits(), source[2][1].to_bits());
    assert_eq!(points.get(0).unwrap()[1].to_bits(), (-0.0f64).to_bits());
    assert!(points.get(12).is_none());
    assert!(points.range(5..13).is_none());
    assert_eq!(points.range(4..8).unwrap().len(), 4);
    let mut backwards = points.iter().rev();
    assert_eq!(
        backwards.next().unwrap()[0].to_bits(),
        expanded[11][0].to_bits()
    );
    assert_eq!(backwards.len(), 11);
    assert_eq!(points.patch().unwrap().cost().compact_rectangles(), 3);
    stop.store(true, Ordering::Relaxed);
    assert!(points.materialize(&stop).is_err());
    stop.store(false, Ordering::Relaxed);
    assert!(points.same_storage(&alias));
    assert_eq!(points.materialize(&stop).unwrap().len(), 12);
}

#[test]
fn weak_series_identity_does_not_retain_compact_backing_or_dense_samples() {
    let stop = AtomicBool::new(false);
    let source = Arc::new(vec![[1.0, 2.0]]);
    let backing = Arc::downgrade(&source);
    let points = rectangles(&source, &stop);
    let weak = points.downgrade();
    assert!(weak.matches(&points));
    let equivalent = rectangles(&source, &stop);
    assert!(!weak.matches(&equivalent));
    drop(equivalent);
    drop(source);
    assert!(backing.upgrade().is_some());
    drop(points);
    assert!(backing.upgrade().is_none());
    let source = Arc::new(vec![[1.0, 2.0]]);
    let allocation = Arc::downgrade(&source);
    let dense = SeriesPoints::from(source);
    let weak_dense = dense.downgrade();
    assert!(weak_dense.matches(&dense));
    assert!(!weak.matches(&dense));
    drop(dense);
    assert!(allocation.upgrade().is_none());
}

#[test]
fn retained_accounting_includes_descriptor_raw_capacity_and_shared_identity_once_per_owner() {
    let stop = AtomicBool::new(false);
    let mut raw = Vec::with_capacity(128);
    raw.extend([[1., 2.], [3., 4.]]);
    let raw = Arc::new(raw);
    let points = rectangles(&raw, &stop);
    let alias = points.clone();
    let mut allocations = std::collections::BTreeMap::new();
    points.visit_retained_allocations(|identity, bytes| {
        assert!(allocations.insert(identity, bytes).is_none());
    });
    assert_eq!(allocations.len(), 2);
    let expected = 128 * 16
        + std::mem::size_of::<Vec<[f64; 2]>>()
        + std::mem::size_of::<PatchCoordinates>()
        + points.patch().unwrap().owned_descriptor_bytes()
        + 4 * std::mem::size_of::<usize>();
    assert_eq!(allocations.values().sum::<usize>(), expected);
    alias.visit_retained_allocations(|identity, bytes| {
        assert_eq!(allocations.get(&identity), Some(&bytes));
    });
    assert_eq!(points.patch().unwrap().cost().retained_bytes(), 128 * 16);
    let plain = SeriesPoints::from(raw);
    let mut dense = Vec::new();
    plain.visit_retained_allocations(|identity, bytes| dense.push((identity, bytes)));
    assert_eq!(dense.len(), 1);
    assert_eq!(allocations.get(&dense[0].0), Some(&dense[0].1));
}
