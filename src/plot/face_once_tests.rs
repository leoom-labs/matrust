//! Frozen pre-optimization scalar reference plus independent parity cases.
use super::*;
use std::cell::Cell;
use std::sync::{Arc, atomic::AtomicBool};

impl Axes {
    fn scalar_reference_bounds(
        &self,
        axes: [AxisTransform; 2],
        check: &impl Fn() -> Result<()>,
    ) -> Result<([f64; 2], [f64; 2])> {
        check()?;
        if self.baselines.len() > MAX_BASELINES {
            return Err("Baseline record bound exceeded".into());
        }
        let (mut x, mut y) = (
            [f64::INFINITY, f64::NEG_INFINITY],
            [f64::INFINITY, f64::NEG_INFINITY],
        );
        for (series_index, s) in self.series.iter().enumerate().filter(|(_, s)| s.visible) {
            let baseline = matches!(s.kind, PlotKind::Bar | PlotKind::Stem)
                .then(|| self.render_baseline(series_index, axes[1]))
                .transpose()?;
            for (i, [a, b]) in s.points.iter().enumerate() {
                if i.is_multiple_of(4096) {
                    check()?;
                }
                if s.vertex_face_accepts(i, axes) {
                    let (lo, hi) = if s.kind == PlotKind::Bar {
                        (
                            a + s.bar_offset - s.bar_width / 2.,
                            a + s.bar_offset + s.bar_width / 2.,
                        )
                    } else {
                        (a, a)
                    };
                    x[0] = x[0].min(if axes[0].accepts(lo) { lo } else { a });
                    x[1] = x[1].max(if axes[0].accepts(hi) { hi } else { a });
                    y[0] = y[0].min(b);
                    y[1] = y[1].max(b);
                    if let Some(baseline) = baseline
                        && axes[1].accepts(baseline)
                    {
                        y[0] = y[0].min(baseline);
                        y[1] = y[1].max(baseline);
                    }
                }
            }
        }
        // Independent visible baseline records affect y bounds, never x extents.
        for baseline in &self.baselines {
            check()?;
            if baseline.style.visible && baseline.style.pattern != LinePattern::None {
                let value = baseline.value.resolve(axes[1])?;
                y[0] = y[0].min(value);
                y[1] = y[1].max(value);
            }
        }
        fn expand(mut range: [f64; 2], axis: AxisTransform) -> [f64; 2] {
            if axis.scale == AxisScale::Log {
                if !range.iter().all(|x| x.is_finite()) {
                    return if axis.negative {
                        [-10., -1.]
                    } else {
                        [1., 10.]
                    };
                }
                if axis.forward(range[0]) == axis.forward(range[1]) {
                    let center = axis.forward(range[0]);
                    return [axis.inverse(center - 0.5), axis.inverse(center + 0.5)];
                }
                return range;
            }
            if !range.iter().all(|v| v.is_finite()) {
                return [0., 1.];
            }
            if range[0] == range[1] {
                let d = (range[0].abs() * 0.05).max(0.5);
                range = [(range[0] - d).max(-f64::MAX), (range[1] + d).min(f64::MAX)];
            }
            range
        }
        check()?;
        Ok((
            self.xlim.unwrap_or_else(|| expand(x, axes[0])),
            self.ylim.unwrap_or_else(|| expand(y, axes[1])),
        ))
    }
}

impl Series {
    fn scalar_reference_display(
        &self,
        max_points: usize,
        axes: [AxisTransform; 2],
        check: &impl Fn() -> Result<()>,
    ) -> Result<Vec<[f64; 2]>> {
        check()?;
        if !self.visible {
            return Ok(Vec::new());
        }
        if self.kind == PlotKind::Patch {
            let output: Vec<_> = self
                .points
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i.is_multiple_of(4096) {
                        check()?;
                    }
                    Ok(if self.vertex_face_accepts(i, axes) {
                        [axes[0].forward(p[0]), axes[1].forward(p[1])]
                    } else {
                        [f64::NAN; 2]
                    })
                })
                .collect::<Result<_>>()?;
            check()?;
            return Ok(output);
        }
        let budget = match self.kind {
            PlotKind::Stairs => max_points.max(18).div_ceil(2),
            PlotKind::Stem => max_points.max(18) / 3,
            _ => max_points,
        };
        let mut result = self.display_points_filtered(
            budget,
            |p| axes[0].accepts(p[0]) && axes[1].accepts(p[1]),
            check,
        )?;
        // Bars need their original edges transformed separately by the renderer.
        if self.kind != PlotKind::Bar {
            for (index, p) in result.iter_mut().enumerate() {
                if index.is_multiple_of(4096) {
                    check()?;
                }
                *p = [axes[0].forward(p[0]), axes[1].forward(p[1])];
            }
        }
        check()?;
        let output = match self.kind {
            PlotKind::Stairs => stair_geometry_checked(&result, check)?,
            PlotKind::Stem => {
                let mut output = Vec::with_capacity(result.len() * 3);
                for (index, p) in result.into_iter().enumerate() {
                    if index.is_multiple_of(4096) {
                        check()?;
                    }
                    output.extend(if p.iter().all(|v| v.is_finite()) {
                        [
                            [p[0], self.base_value.map_or(0., |v| axes[1].forward(v))],
                            p,
                            [f64::NAN; 2],
                        ]
                    } else {
                        [[f64::NAN; 2]; 3]
                    });
                }
                output
            }
            _ => result,
        };
        check()?;
        Ok(output)
    }
}

fn points_bits(points: &[[f64; 2]]) -> Vec<[u64; 2]> {
    points.iter().map(|p| p.map(f64::to_bits)).collect()
}
fn bounds_bits(bounds: ([f64; 2], [f64; 2])) -> ([u64; 2], [u64; 2]) {
    (bounds.0.map(f64::to_bits), bounds.1.map(f64::to_bits))
}
fn patch_variants(source: Vec<[f64; 2]>) -> Vec<Series> {
    use coordinates::{Coordinate, CoordinateLane, PatchCoordinates, ScalarStep};
    let stop = AtomicBool::new(false);
    let source = Arc::new(source);
    let x = CoordinateLane::pair_component(&source, 0).unwrap();
    let y = CoordinateLane::pair_component(&source, 1).unwrap();
    let right = x.clone().then(ScalarStep::Add(0.5)).unwrap();
    let top = y.clone().then(ScalarStep::Add(0.75)).unwrap();
    let geometry = PatchCoordinates::rectangles(
        [
            [Coordinate::Lane(x.clone()), Coordinate::Lane(y.clone())],
            [Coordinate::Lane(right.clone()), Coordinate::Lane(y)],
            [Coordinate::Lane(right), Coordinate::Lane(top.clone())],
            [Coordinate::Lane(x), Coordinate::Lane(top)],
        ],
        source.len(),
        &stop,
    )
    .unwrap();
    // Same values from two offset/stride-two lanes sharing a larger backing.
    // Unused prefix/suffix capacity must stay retained and charged.
    let mut raw = vec![0.; 17 + source.len() * 2 + 19];
    for (i, p) in source.iter().enumerate() {
        raw[17 + i * 2] = p[0];
        raw[18 + i * 2] = p[1];
    }
    let backing = Matrix::from_column_vec(1, raw.len(), raw).unwrap();
    let ox = CoordinateLane::matrix_strided(&backing, 17, 2, source.len()).unwrap();
    let oy = CoordinateLane::matrix_strided(&backing, 18, 2, source.len()).unwrap();
    let right = ox.clone().then(ScalarStep::Add(0.5)).unwrap();
    let top = oy.clone().then(ScalarStep::Add(0.75)).unwrap();
    let offset_geometry = PatchCoordinates::rectangles(
        [
            [Coordinate::Lane(ox.clone()), Coordinate::Lane(oy.clone())],
            [Coordinate::Lane(right.clone()), Coordinate::Lane(oy)],
            [Coordinate::Lane(right), Coordinate::Lane(top.clone())],
            [Coordinate::Lane(ox), Coordinate::Lane(top)],
        ],
        source.len(),
        &stop,
    )
    .unwrap();
    let dense = geometry.materialize(&stop).unwrap();
    let triangles: Vec<_> = dense
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|face| face[..3].iter().copied())
        .collect();
    [
        (SeriesPoints::Patch(Arc::new(geometry)), 4),
        (SeriesPoints::Patch(Arc::new(offset_geometry)), 4),
        (SeriesPoints::from(dense.clone()), 4),
        (
            SeriesPoints::Patch(Arc::new(
                PatchCoordinates::dense(Arc::new(dense), 4, &stop).unwrap(),
            )),
            4,
        ),
        (SeriesPoints::from(triangles.clone()), 3),
        (
            SeriesPoints::Patch(Arc::new(
                PatchCoordinates::dense(Arc::new(triangles), 3, &stop).unwrap(),
            )),
            3,
        ),
    ]
    .into_iter()
    .map(|(points, width)| {
        let mut series = Series::with_points(points, PlotKind::Patch);
        series.patch = Some(PatchFaces::new(width).unwrap());
        series.face_edge_color = Some([231, 33, 87]);
        series.pattern = LinePattern::DashDot;
        series
    })
    .collect()
}

#[test]
fn face_once_matches_frozen_scalar_for_storage_variants_exceptional_domains_and_limits() {
    let source = vec![
        [-8., -5.],
        [1., 2.],
        [-0.0, 0.0],
        [0.0, -0.0],
        [f64::from_bits(1), -f64::from_bits(1)],
        [f64::MAX / 2., f64::MAX / 2.],
        [-f64::MAX / 2., -f64::MAX / 2.],
        [f64::from_bits(0x7ff8_0000_0000_007b), 2.],
        [1., f64::INFINITY],
        [f64::NEG_INFINITY, -2.],
    ];
    let log = |negative| AxisTransform {
        scale: AxisScale::Log,
        negative,
    };
    for series in patch_variants(source) {
        let shared = series.points.clone();
        for transforms in [
            [AxisTransform::default(); 2],
            [log(false); 2],
            [log(true); 2],
            [log(false), log(true)],
            [log(true), AxisTransform::default()],
        ] {
            for visible in [true, false] {
                let mut candidate = series.clone();
                candidate.visible = visible;
                let before = candidate.clone();
                let expected = candidate
                    .scalar_reference_display(1, transforms, &|| Ok(()))
                    .unwrap();
                let got = candidate
                    .display_points_axes_checked(1, transforms, &|| Ok(()))
                    .unwrap();
                assert_eq!(points_bits(&got), points_bits(&expected));
                assert_eq!(got.len(), if visible { candidate.points.len() } else { 0 });
                for (xlim, ylim) in [
                    (None, None),
                    (Some([-9., 9.]), None),
                    (None, Some([-7., 7.])),
                    (Some([1., 8.]), Some([2., 9.])),
                ] {
                    let axes = Axes {
                        series: vec![candidate.clone()],
                        xlim,
                        ylim,
                        ..Default::default()
                    };
                    let expected = axes
                        .scalar_reference_bounds(transforms, &|| Ok(()))
                        .unwrap();
                    let got = axes
                        .bounds_with_axes_checked(transforms, &|| Ok(()))
                        .unwrap();
                    assert_eq!(bounds_bits(got), bounds_bits(expected));
                    assert_eq!(
                        axes.geometry_ledger().unwrap(),
                        axes.clone().geometry_ledger().unwrap()
                    );
                }
                assert!(candidate.points.same_storage(&before.points));
                assert!(candidate.points.same_storage(&shared));
            }
        }
    }
    for series in patch_variants(Vec::new()) {
        assert_eq!(series.display_points(1), Vec::<[f64; 2]>::new());
        let axes = Axes {
            series: vec![series],
            ..Default::default()
        };
        assert_eq!(
            bounds_bits(axes.bounds()),
            bounds_bits(
                axes.scalar_reference_bounds([AxisTransform::default(); 2], &|| Ok(()))
                    .unwrap()
            )
        );
    }
}

#[test]
fn face_once_keeps_malformed_public_models_on_the_exact_scalar_fallback() {
    let compact = patch_variants(vec![[1., 2.], [3., 4.], [5., 6.]]).remove(0);
    let mut width_mismatch = compact.clone();
    width_mismatch.patch = Some(PatchFaces::new(3).unwrap());
    let mut absent = compact.clone();
    absent.patch = None;
    let mut incomplete = Series::new(vec![[1., 2.], [3., 4.], [5., 6.]], PlotKind::Patch);
    incomplete.patch = Some(PatchFaces::new(4).unwrap());
    for series in [width_mismatch, absent, incomplete] {
        assert!(series.complete_patch_width().is_none());
        assert!(series.validate_patch().is_err());
        let transforms = [AxisTransform::default(); 2];
        assert_eq!(
            points_bits(
                &series
                    .display_points_axes_checked(1, transforms, &|| Ok(()))
                    .unwrap()
            ),
            points_bits(
                &series
                    .scalar_reference_display(1, transforms, &|| Ok(()))
                    .unwrap()
            )
        );
        let axes = Axes {
            series: vec![series],
            ..Default::default()
        };
        assert_eq!(
            bounds_bits(
                axes.bounds_with_axes_checked(transforms, &|| Ok(()))
                    .unwrap()
            ),
            bounds_bits(
                axes.scalar_reference_bounds(transforms, &|| Ok(()))
                    .unwrap()
            )
        );
    }
}

fn cancellation_counter(counter: &Cell<usize>, stop: usize) -> Result<()> {
    counter.set(counter.get() + 1);
    if counter.get() == stop {
        Err("face-once deterministic cancellation".into())
    } else {
        Ok(())
    }
}
#[test]
fn face_once_preserves_every_scalar_checkpoint_and_retry_alias() {
    let source: Vec<_> = (0..2051).map(|i| [i as f64 + 1., i as f64 + 2.]).collect();
    let transforms = [AxisTransform::default(); 2];
    for series in patch_variants(source) {
        let alias = series.points.clone();
        let counter = Cell::new(0);
        let expected = series
            .scalar_reference_display(1, transforms, &|| {
                cancellation_counter(&counter, usize::MAX)
            })
            .unwrap();
        let total = counter.get();
        for stop in 1..=total + 1 {
            let a = Cell::new(0);
            let b = Cell::new(0);
            let expected_result =
                series.scalar_reference_display(1, transforms, &|| cancellation_counter(&a, stop));
            let actual_result = series
                .display_points_axes_checked(1, transforms, &|| cancellation_counter(&b, stop));
            assert_eq!(a.get(), b.get());
            assert_eq!(expected_result.as_ref().err(), actual_result.as_ref().err());
            if let (Ok(a), Ok(b)) = (expected_result, actual_result) {
                assert_eq!(points_bits(&a), points_bits(&b));
            }
        }
        assert_eq!(
            points_bits(&series.display_points(1)),
            points_bits(&expected)
        );
        let axes = Axes {
            series: vec![series],
            ..Default::default()
        };
        counter.set(0);
        let expected_bounds = axes
            .scalar_reference_bounds(transforms, &|| cancellation_counter(&counter, usize::MAX))
            .unwrap();
        for stop in 1..=counter.get() + 1 {
            let a = Cell::new(0);
            let b = Cell::new(0);
            let expected_result =
                axes.scalar_reference_bounds(transforms, &|| cancellation_counter(&a, stop));
            let actual_result =
                axes.bounds_with_axes_checked(transforms, &|| cancellation_counter(&b, stop));
            assert_eq!(a.get(), b.get());
            assert_eq!(expected_result.as_ref().err(), actual_result.as_ref().err());
            if let (Ok(a), Ok(b)) = (expected_result, actual_result) {
                assert_eq!(bounds_bits(a), bounds_bits(b));
            }
        }
        assert_eq!(bounds_bits(axes.bounds()), bounds_bits(expected_bounds));
        assert!(alias.same_storage(&axes.series[0].points));
    }
}

#[test]
fn face_once_bounds_preserve_baselines_manual_modes_and_complete_outlined_svg() {
    let stop = AtomicBool::new(false);
    for (scale, sign) in [
        (AxisScale::Linear, 1.),
        (AxisScale::Log, 1.),
        (AxisScale::Log, -1.),
    ] {
        for patch in patch_variants(vec![[sign * 2., sign * 3.], [sign * 5., sign * 6.]]) {
            let mut figure = Figure {
                axes: Axes {
                    xscale: scale,
                    yscale: scale,
                    ..Axes::default()
                },
                ..Figure::default()
            };
            figure
                .add(vec![
                    patch,
                    Series::new(vec![[sign * 3., sign * 4.]], PlotKind::Stem),
                    Series::new(vec![[sign * 7., sign * 8.]], PlotKind::Bar),
                ])
                .unwrap();
            let style = BaselineStyle {
                color: [0.15, 0.25, 0.35],
                pattern: LinePattern::Dashed,
                width_points: 1.25,
                visible: true,
            };
            let member = figure.series[1].identity.clone();
            figure
                .replace_baselines(
                    vec![
                        Baseline::new(BaselineValue::Value(sign * 1.5), style, vec![member])
                            .unwrap(),
                        Baseline::new(BaselineValue::Value(sign * 12.), style, Vec::new()).unwrap(),
                    ],
                    &stop,
                )
                .unwrap();
            let shared = figure.clone();
            for manual in [false, true] {
                let mut scene = figure.clone();
                if manual {
                    let limits = if sign > 0. { [1., 10.] } else { [-10., -1.] };
                    scene.xlim = Some(limits);
                    scene.ylim = Some(limits);
                }
                scene.validate().unwrap();
                let transforms = scene.axis_transforms();
                let expected = scene
                    .scalar_reference_bounds(transforms, &|| Ok(()))
                    .unwrap();
                assert_eq!(bounds_bits(scene.bounds()), bounds_bits(expected));
                let mut fixed = scene.clone();
                fixed.xlim = Some(expected.0);
                fixed.ylim = Some(expected.1);
                let actual_svg = scene.svg();
                assert!(!actual_svg.is_empty());
                assert_eq!(actual_svg, fixed.svg());
                assert_eq!(
                    scene.geometry_ledger().unwrap(),
                    fixed.geometry_ledger().unwrap()
                );
                assert!(
                    scene.series[0]
                        .points
                        .same_storage(&shared.series[0].points)
                );
            }
            assert!(figure.same_snapshot(&shared));
        }
    }
}
