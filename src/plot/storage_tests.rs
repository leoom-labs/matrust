//! End-to-end public reconstruction tests for the resource-equivalent banks.
use super::*;
use crate::{
    Engine, Matrix,
    document::Document,
    files,
    plot::{Figure, LinePattern, PlotKind, Series},
};

fn bits(a: &SeriesPoints, b: &SeriesPoints) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b.iter()) {
        assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
    }
}
fn replay(path: &std::path::Path) -> Figure {
    let doc = Document::open(path).unwrap();
    let mut engine = Engine::default();
    engine
        .execute_document(&doc.text, doc.path.as_deref().unwrap())
        .unwrap();
    engine.figure.unwrap()
}
fn maximum(width: usize) -> Figure {
    let stop = AtomicBool::new(false);
    let faces = 10_000;
    let line_count = 1_000_000 - faces - faces * width;
    let mut line = Series::new(
        (0..line_count)
            .map(|i| [i as f64 + 1., (i % 19) as f64 + 1.])
            .collect(),
        PlotKind::Line,
    );
    line.label = "original line".into();
    line.pattern = LinePattern::DashDot;
    let mut bar = Series::new(
        (0..faces)
            .map(|i| [i as f64 + 1., (i % 13) as f64 + 1.])
            .collect(),
        PlotKind::Bar,
    );
    bar.bar_offset = 0.125;
    // Historical fill-only arbitrary geometry exercises exact Patch storage.
    bar.bar_baseline_visible = false;
    bar.bar_width = 0.75;
    bar.color = [31, 73, 127];
    bar.label = "rectangles".into();
    let mut x = Vec::with_capacity(faces * width);
    let mut y = Vec::with_capacity(faces * width);
    for face in 0..faces {
        let left = face as f64 + 1.;
        let points = [[left, 1.], [left + 0.5, 1.], [left + 0.5, 2.], [left, 2.]];
        for point in &points[..width] {
            x.push(point[0]);
            y.push(point[1]);
        }
    }
    let patch = Series::patch_from_matrices(
        &Matrix::from_column_vec(width, faces, x).unwrap(),
        &Matrix::from_column_vec(width, faces, y).unwrap(),
        [91, 41, 17],
        &stop,
    )
    .unwrap();
    let mut f = Figure::default();
    f.appearance.as_mut().unwrap().series_palette = None;
    f.axes.palette_cycle = None;
    f.add(vec![line, bar, patch]).unwrap();
    f.validate().unwrap();
    assert_eq!(
        f.geometry_ledger().unwrap().cost().source_units(),
        1_000_000
    );
    f
}
fn check_bar(source: &Series, patch: &Series) {
    assert_eq!(patch.kind, PlotKind::Patch);
    assert_eq!(patch.patch_face_count(), source.points.len());
    assert!(patch.points.patch().is_some());
    assert_eq!(patch.color, source.color);
    assert_eq!(patch.label, source.label);
    for (index, [x, y]) in source.points.iter().enumerate() {
        let left = (x + source.bar_offset) - source.bar_width / 2.;
        let right = (x + source.bar_offset) + source.bar_width / 2.;
        let expected = [[left, 0.], [right, 0.], [right, y], [left, y]];
        let face = patch.points.face(index, 4).unwrap();
        for (actual, expected) in face.as_slice().iter().zip(expected) {
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        }
    }
}
#[test]
fn maximum_two_bank_bundle_replay_native_reopen_and_second_export_retain_real_coordinates() {
    for width in [3, 4] {
        let directory = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let source = maximum(width);
        let alias = source.clone();
        let path = directory.path().join("first.m");
        assert!(
            files::write_matlab_plot_reconstruction_with_cancel(&path, &source, &stop)
                .unwrap()
                .is_some()
        );
        let first = replay(&path);
        first.validate().unwrap();
        assert_eq!(first.series.len(), 3);
        bits(&source.series[0].points, &first.series[0].points);
        bits(&source.series[2].points, &first.series[2].points);
        check_bar(&source.series[1], &first.series[1]);
        let cost = first.geometry_ledger().unwrap().cost();
        assert_eq!(cost.source_units(), 1_000_000);
        assert_eq!(cost.expanded_vertices(), 1_030_000);
        assert_eq!(cost.compact_rectangles(), 10_000);
        assert_eq!(cost.general_faces(), 10_000);
        assert_eq!(cost.retained_bytes(), 16_000_000);
        // Keep the historical compact capability independent of modern intent,
        // while the original native + second-script cycle retains exact metadata.
        assert!(
            first
                .series
                .iter()
                .all(|s| s.explicit_color && s.auto_palette.is_none())
        );
        let mut legacy = first.clone();
        legacy.appearance = None;
        legacy.axes.palette_cycle = None;
        let bytes = files::encode_native_figure(&legacy).unwrap();
        assert!(bytes.starts_with(b"MATRFIGN"));
        let legacy_loaded = files::decode_native_figure(&bytes).unwrap();
        assert!(legacy.same_metadata(&legacy_loaded));
        assert_eq!(legacy_loaded.geometry_ledger().unwrap().cost(), cost);
        for (a, b) in legacy.series.iter().zip(&legacy_loaded.series) {
            bits(&a.points, &b.points);
        }
        let mut theme_only = first.clone();
        theme_only.appearance.as_mut().unwrap().series_palette = None;
        theme_only.axes.palette_cycle = None;
        let bytes = files::encode_native_figure(&theme_only).unwrap();
        assert!(bytes.starts_with(b"MATRFIGR"));
        assert!(theme_only.same_metadata(&files::decode_native_figure(&bytes).unwrap()));
        let bytes = files::encode_native_figure(&first).unwrap();
        assert!(bytes.starts_with(b"MATRFIGV"));
        let loaded = files::decode_native_figure(&bytes).unwrap();
        assert!(first.same_metadata(&loaded));
        assert_eq!(loaded.geometry_ledger().unwrap().cost(), cost);
        for (a, b) in first.series.iter().zip(&loaded.series) {
            bits(&a.points, &b.points);
        }
        let second_path = directory.path().join("second.m");
        assert!(
            files::write_matlab_plot_reconstruction_with_cancel(&second_path, &loaded, &stop)
                .unwrap()
                .is_some()
        );
        let second = replay(&second_path);
        assert!(loaded.same_metadata(&second));
        assert_eq!(second.geometry_ledger().unwrap().cost(), cost);
        for (a, b) in loaded.series.iter().zip(&second.series) {
            bits(&a.points, &b.points);
        }
        for (a, b) in source.series.iter().zip(&alias.series) {
            assert!(a.points.same_storage(&b.points));
        }
        // Resource admission remains atomic at the exact envelope.
        let original = second.clone();
        let mut rejected = second;
        assert!(
            rejected
                .add(vec![Series::new(vec![[1., 1.]], PlotKind::Line)])
                .is_err()
        );
        assert!(rejected.same_snapshot(&original));
    }
}
#[test]
fn compact_pair_sources_stride_two_and_nonfinite_constants_use_public_operations_losslessly() {
    use crate::plot::{
        PatchFaces,
        coordinates::{Coordinate, CoordinateLane, PatchCoordinates, ScalarStep},
    };
    let stop = AtomicBool::new(false);
    let raw = Arc::new(vec![
        [-0., -0.],
        [1.0e16, f64::from_bits(1)],
        [3., f64::from_bits(0x7ff8_1234_0000_0001)],
    ]);
    let x = CoordinateLane::pair_component(&raw, 0).unwrap();
    let y = CoordinateLane::pair_component(&raw, 1).unwrap();
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
    let bottom = Coordinate::Lane(y.clone().then(ScalarStep::Multiply(0.)).unwrap());
    let top = Coordinate::Lane(y);
    let coordinates = PatchCoordinates::rectangles(
        [
            [left.clone(), bottom.clone()],
            [right.clone(), bottom],
            [right, top.clone()],
            [left, top],
        ],
        raw.len(),
        &stop,
    )
    .unwrap();
    let mut series =
        Series::with_points(SeriesPoints::Patch(Arc::new(coordinates)), PlotKind::Patch);
    series.patch = Some(PatchFaces::new(4).unwrap());
    series.pattern = LinePattern::None;
    let mut f = Figure::default();
    f.add(vec![series]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pairs.m");
    assert!(
        files::write_matlab_plot_reconstruction_with_cancel(&path, &f, &stop)
            .unwrap()
            .is_some()
    );
    let g = replay(&path);
    assert!(g.series[0].points.patch().is_some());
    assert_eq!(f.series[0].points.len(), g.series[0].points.len());
    for (index, (before, after)) in f.series[0]
        .points
        .iter()
        .zip(g.series[0].points.iter())
        .enumerate()
    {
        for axis in 0..2 {
            // Only the bottom y*0 arithmetic on the missing third face has
            // deliberately unspecified derived NaN payload bits. Untouched
            // top/source NaNs and every finite/signed-zero value stay exact.
            if axis == 1 && matches!(index, 8 | 9) {
                assert!(before[axis].is_nan() && after[axis].is_nan());
            } else {
                assert_eq!(before[axis].to_bits(), after[axis].to_bits());
            }
        }
    }
    let data = g.series[0]
        .points
        .patch()
        .unwrap()
        .script_coordinates()
        .unwrap();
    assert_eq!(data.sources.len(), 1);
    assert_eq!(data.sources[0].len(), raw.len() * 2);
    for i in 0..raw.len() * 2 {
        assert_eq!(
            data.sources[0].value(i).to_bits(),
            raw[i / 2][i % 2].to_bits()
        );
    }
    let constant = f64::from_bits(0x7ff8_4321_0000_0001);
    let corners =
        std::array::from_fn(|_| [Coordinate::Constant(1.), Coordinate::Constant(constant)]);
    let mut series = Series::with_points(
        SeriesPoints::Patch(Arc::new(
            PatchCoordinates::rectangles(corners, 2, &stop).unwrap(),
        )),
        PlotKind::Patch,
    );
    series.patch = Some(PatchFaces::new(4).unwrap());
    series.pattern = LinePattern::None;
    let mut f = Figure::default();
    f.add(vec![series]).unwrap();
    let path = directory.path().join("constants.m");
    assert!(
        files::write_matlab_plot_reconstruction_with_cancel(&path, &f, &stop)
            .unwrap()
            .is_some()
    );
    let g = replay(&path);
    assert!(g.series[0].points.patch().is_some());
    bits(&f.series[0].points, &g.series[0].points);
}

fn small_compact() -> Figure {
    let mut engine = Engine::default();
    engine.execute("x=[1;2;3];y=[2;-1;4];a=x-.25;b=x+.25;c=y*0;patch('XData',[a b b a].','YData',[c c y y].','FaceColor',[.2 .4 .6],'EdgeColor','none');title('Compact coordinates');xlabel('x');ylabel('y');").unwrap();
    let figure = engine.figure.unwrap();
    assert!(figure.series[0].points.patch().is_some());
    figure
}
#[test]
fn inline_descriptor_replay_dense_render_and_original_sample_picking_agree() {
    use crate::plot::SampleQuery;
    let stop = AtomicBool::new(false);
    let source = small_compact();
    let alias = source.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("small.m");
    assert!(
        files::write_matlab_plot_reconstruction_with_cancel(&path, &source, &stop)
            .unwrap()
            .is_none()
    );
    let replayed = replay(&path);
    assert!(replayed.series[0].points.patch().is_some());
    assert!(source.same_metadata(&replayed));
    bits(&source.series[0].points, &replayed.series[0].points);
    let mut dense = source.clone();
    dense.series[0].points = source.series[0].points.materialize(&stop).unwrap().into();
    assert_eq!(source.bounds(), dense.bounds());
    assert_eq!(source.svg(), dense.svg());
    for pointer in [[0.75, 0.], [2.25, -1.], [3.25, 4.]] {
        let query = SampleQuery {
            transforms: source.axis_transforms(),
            bounds: [0., 4., -2., 5.],
            pointer,
            pixels: [640., 480.],
        };
        let a = source.nearest_sample(query, &stop).unwrap().unwrap();
        let b = dense.nearest_sample(query, &stop).unwrap().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.distance_squared, 0.);
        assert_eq!(
            source.series[a.series].points.get(a.index).unwrap(),
            a.point
        );
    }
    assert!(source.same_snapshot(&alias));
}
#[test]
fn descriptor_recovery_reuses_checkpoint_and_native_versions_reject_downgrade() {
    use crate::plot::Position;
    use crate::session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, restore_figures},
    };
    let stop = AtomicBool::new(false);
    let mut source = small_compact();
    source.appearance = None; // Historical codec fixture, independent of modern theme intent.
    assert!(source.series.iter().all(|s| s.auto_palette.is_none()));
    source.axes.palette_cycle = None;
    let original_storage = source.series[0].points.downgrade();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.json");
    let state = Session {
        layout_presets: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: directory.path().into(),
        recent: vec![],
        history: vec![],
        model: Default::default(),
        model_path: None,
        model_baseline: None,
        layout: Default::default(),
        export: Default::default(),
        preview: None,
        eps_renderer: None,
        workspace: None,
        workspace_manifest: None,
        figure_manifest: None,
        figures: Some(FigureSession {
            current: 1,
            figures: vec![FigureDocument {
                id: 1,
                figure: source.clone(),
                path: None,
                source_version: None,
                dirty: true,
                view: None,
                window: None,
            }],
        }),
    };
    let mut store = SessionStore::new(&path);
    store.save(state.clone()).unwrap();
    let first = std::fs::read(&path).unwrap();
    store.save(state).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first);
    let loaded = Session::load(&path).unwrap();
    let recovered = restore_figures(&path, loaded.figure_manifest.as_ref().unwrap(), &stop);
    assert!(recovered.warnings.is_empty(), "{:?}", recovered.warnings);
    let restored = &recovered.figures.figures[0].figure;
    assert!(source.same_metadata(restored));
    bits(&source.series[0].points, &restored.series[0].points);
    assert!(restored.series[0].points.patch().is_some());
    assert_eq!(
        source.geometry_ledger().unwrap().cost(),
        restored.geometry_ledger().unwrap().cost()
    );
    let single = files::encode_native_figure(&source).unwrap();
    assert!(single.starts_with(b"MATRFIGN"));
    let mut downgraded = single.clone();
    downgraded[..8].copy_from_slice(b"MATRFIGJ");
    assert!(
        files::decode_native_figure(&downgraded)
            .unwrap_err()
            .contains("Compact")
    );
    let mut panels = Figure {
        appearance: None,
        axes: crate::plot::Axes::default(),
        ..Figure::default()
    };
    panels
        .select_subplot(Position::from_grid(1, 2, &[1]).unwrap(), false)
        .unwrap();
    panels.add(source.series.clone()).unwrap();
    panels
        .select_subplot(Position::from_grid(1, 2, &[2]).unwrap(), false)
        .unwrap();
    let mut second_series = source.series.clone();
    for series in &mut second_series {
        series.identity = Default::default();
    }
    panels.add(second_series).unwrap();
    let bytes = files::encode_native_figure(&panels).unwrap();
    assert!(bytes.starts_with(b"MATRFIGO"));
    let reopened = files::decode_native_figure(&bytes).unwrap();
    assert!(panels.same_metadata(&reopened));
    assert_eq!(
        panels.geometry_ledger().unwrap(),
        reopened.geometry_ledger().unwrap()
    );
    let mut downgraded = bytes.clone();
    downgraded[..8].copy_from_slice(b"MATRFIGK");
    assert!(
        files::decode_native_figure(&downgraded)
            .unwrap_err()
            .contains("Compact")
    );
    // Each child is locally valid at 9.6 MB retained capacity, but the combined
    // 19.2 MB exceeds the global ledger. Its second child must fail preflight.
    let outer_size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut aggregate = bytes[..12 + outer_size].to_vec();
    let mut cursor = 12 + outer_size;
    for _ in 0..2 {
        let child_size = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let child = &bytes[cursor..cursor + child_size];
        cursor += child_size;
        let meta_size = u32::from_le_bytes(child[8..12].try_into().unwrap()) as usize;
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&child[12..12 + meta_size]).unwrap();
        let sources = metadata["series"][0]["coordinates"]["sources"]
            .as_array_mut()
            .unwrap();
        assert!(sources.iter().all(serde_json::Value::is_object));
        for source in sources {
            source["capacity"] = 600_000.into();
        }
        let json = serde_json::to_vec(&metadata).unwrap();
        let mut replacement = b"MATRFIGN".to_vec();
        replacement.extend((json.len() as u32).to_le_bytes());
        replacement.extend(json);
        replacement.extend_from_slice(&child[12 + meta_size..]);
        aggregate.extend((replacement.len() as u32).to_le_bytes());
        aggregate.extend(replacement);
    }
    assert_eq!(cursor, bytes.len());
    assert!(
        files::decode_native_figure(&aggregate)
            .unwrap_err()
            .contains("resource")
    );
    // A valid outer envelope cannot authorize a descriptor's unchecked claims.
    let size = u32::from_le_bytes(single[8..12].try_into().unwrap()) as usize;
    let metadata: serde_json::Value = serde_json::from_slice(&single[12..12 + size]).unwrap();
    for (field, value) in [("capacity", u32::MAX), ("len", u32::MAX)] {
        let mut invalid = metadata.clone();
        invalid["series"][0]["coordinates"]["sources"][0][field] = value.into();
        let json = serde_json::to_vec(&invalid).unwrap();
        let mut bytes = b"MATRFIGN".to_vec();
        bytes.extend((json.len() as u32).to_le_bytes());
        bytes.extend(json);
        bytes.extend_from_slice(&single[12 + size..]);
        assert!(files::decode_native_figure(&bytes).is_err());
    }
    drop(panels);
    drop(source);
    assert!(
        original_storage.upgrade().is_none(),
        "Recovery cache must not retain compact descriptors or raw backings"
    );
    drop(store);
}
#[test]
fn companion_keeps_noncanonical_nan_step_operand_bits_without_promising_derived_nan_bits() {
    use crate::plot::{
        PatchFaces,
        coordinates::{Coordinate, CoordinateLane, PatchCoordinates, ScalarStep, ScriptCoordinate},
    };
    let stop = AtomicBool::new(false);
    let operand = f64::from_bits(0x7ff8_1234_5555_0001);
    let raw = Arc::new(vec![[1., 2.], [2., 3.]]);
    let x = Coordinate::Lane(
        CoordinateLane::pair_component(&raw, 0)
            .unwrap()
            .then(ScalarStep::Add(operand))
            .unwrap(),
    );
    let y = Coordinate::Lane(CoordinateLane::pair_component(&raw, 1).unwrap());
    let coordinates = PatchCoordinates::rectangles(
        [
            [x.clone(), y.clone()],
            [x.clone(), y.clone()],
            [x.clone(), y.clone()],
            [x, y],
        ],
        2,
        &stop,
    )
    .unwrap();
    let mut series =
        Series::with_points(SeriesPoints::Patch(Arc::new(coordinates)), PlotKind::Patch);
    series.patch = Some(PatchFaces::new(4).unwrap());
    series.pattern = LinePattern::None;
    let mut f = Figure::default();
    f.add(vec![series]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("operand.m");
    assert!(
        files::write_matlab_plot_reconstruction_with_cancel(&path, &f, &stop)
            .unwrap()
            .is_some()
    );
    let g = replay(&path);
    let descriptor = g.series[0]
        .points
        .patch()
        .unwrap()
        .script_coordinates()
        .unwrap();
    for corner in descriptor.corners {
        let ScriptCoordinate::Lane { steps, .. } = corner[0] else {
            panic!("retained ordered lane")
        };
        assert_eq!(steps[0].unwrap().operand().to_bits(), operand.to_bits());
        assert!(steps[1].is_none());
    }
    for point in g.series[0].points.iter() {
        assert!(point[0].is_nan());
    }
    assert_eq!(descriptor.sources.len(), 1);
    for i in 0..4 {
        assert_eq!(
            descriptor.sources[0].value(i).to_bits(),
            raw[i / 2][i % 2].to_bits()
        );
    }
}

#[test]
fn single_face_transpose_retains_four_lanes_and_dense_bits_at_tiny_width() {
    let stop = AtomicBool::new(false);
    for (center, width) in [(0., f64::from_bits(1)), (1.0e16, 0.25)] {
        let x = Matrix::scalar(center);
        let y = Matrix::scalar(-0.);
        let left = x.coordinate_subtract(&Matrix::scalar(width)).unwrap();
        let right = x.coordinate_add(&Matrix::scalar(width)).unwrap();
        let zero = y.coordinate_multiply(&Matrix::scalar(0.)).unwrap();
        let corners_x = Matrix::concatenate(vec![vec![left.clone(), right.clone(), right, left]])
            .unwrap()
            .transpose();
        let corners_y = Matrix::concatenate(vec![vec![zero.clone(), zero, y.clone(), y]])
            .unwrap()
            .transpose();
        let series = Series::patch_from_matrices(&corners_x, &corners_y, [1, 2, 3], &stop).unwrap();
        assert!(series.points.patch().is_some());
        assert_eq!(series.patch_face_count(), 1);
        for (i, point) in series.points.iter().enumerate() {
            assert_eq!(
                point.map(f64::to_bits),
                [corners_x.data()[i].to_bits(), corners_y.data()[i].to_bits()]
            );
        }
        let twice_x = corners_x.transpose().transpose();
        let twice_y = corners_y.transpose().transpose();
        assert!(
            Series::patch_from_matrices(&twice_x, &twice_y, [1, 2, 3], &stop)
                .unwrap()
                .points
                .patch()
                .is_some()
        );
    }
}

#[test]
fn ordinary_stride_two_gather_preserves_dense_order_and_other_strides_drop_optional_facts() {
    use crate::Array;
    let mut engine = Engine::default();
    engine
        .execute("x=(1:40).';kept=x;two=x(2:2:20);three=x(1:3:40);back=x(40:-1:1);")
        .unwrap();
    let numeric = |name: &str| {
        let Array::Numeric(value) = engine.array(name).unwrap() else {
            panic!("numeric fixture")
        };
        value
    };
    let source = numeric("x");
    let alias = numeric("kept");
    assert_eq!(source.data().as_ptr(), alias.data().as_ptr());
    for (name, start, step, count, proof) in [
        ("two", 2, 2, 10, true),
        ("three", 1, 3, 14, false),
        ("back", 40, -1, 40, false),
    ] {
        let actual = numeric(name);
        assert_eq!(actual.has_coordinate_proof(), proof);
        assert_eq!(actual.len(), count);
        for (index, &value) in actual.data().iter().enumerate() {
            assert_eq!(
                value.to_bits(),
                ((start + index as i32 * step) as f64).to_bits()
            );
        }
    }
    // Fact propagation keeps the same authoritative eager output semantics.
    assert_eq!(source.data().as_ptr(), alias.data().as_ptr());
}

#[test]
fn second_export_short_lane_keeps_full_backing_above_one_million_scalars() {
    use crate::plot::{
        PatchFaces,
        coordinates::{Coordinate, CoordinateLane, PatchCoordinates, ScalarStep},
    };
    let stop = AtomicBool::new(false);
    let source =
        Matrix::from_column_vec(10_000, 101, (0..1_010_000).map(|i| i as f64 + 1.).collect())
            .unwrap();
    let alias = source.clone();
    let lane = CoordinateLane::matrix_column(&source, 100).unwrap();
    let left = Coordinate::Lane(lane.clone().then(ScalarStep::Subtract(0.25)).unwrap());
    let right = Coordinate::Lane(lane.then(ScalarStep::Add(0.25)).unwrap());
    let bottom = Coordinate::Constant(-0.);
    let top = Coordinate::Constant(2.);
    let coordinates = PatchCoordinates::rectangles(
        [
            [left.clone(), bottom.clone()],
            [right.clone(), bottom],
            [right, top.clone()],
            [left, top],
        ],
        10_000,
        &stop,
    )
    .unwrap();
    let mut series =
        Series::with_points(SeriesPoints::Patch(Arc::new(coordinates)), PlotKind::Patch);
    series.patch = Some(PatchFaces::new(4).unwrap());
    series.pattern = LinePattern::None;
    let mut figure = Figure::default();
    figure.add(vec![series]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    for pass in 0..2 {
        let path = directory.path().join(format!("offset_{pass}.m"));
        assert!(
            files::write_matlab_plot_reconstruction_with_cancel(&path, &figure, &stop)
                .unwrap()
                .is_some()
        );
        let restored = replay(&path);
        let descriptor = restored.series[0].points.patch().unwrap();
        bits(&figure.series[0].points, &restored.series[0].points);
        assert_eq!(descriptor.cost().retained_bytes(), 1_010_000 * 8);
        let script = descriptor.script_coordinates().unwrap();
        assert_eq!(script.sources.len(), 1);
        assert_eq!(script.sources[0].len(), source.len());
        for (index, value) in source.data().iter().enumerate() {
            assert_eq!(script.sources[0].value(index).to_bits(), value.to_bits());
        }
        figure = restored;
    }
    assert_eq!(source.data().as_ptr(), alias.data().as_ptr());
}

#[test]
fn tiny_bar_width_and_missing_face_replay_do_not_narrow_valid_bar_geometry() {
    let stop = AtomicBool::new(false);
    let nan = f64::from_bits(0x7ff8_0123_4567_0001);
    for width in [f64::from_bits(1), 0.25] {
        let mut series = Series::new(vec![[-0., -0.], [1.0e16, 2.], [3., nan]], PlotKind::Bar);
        series.bar_width = width;
        // Historical fill-only arbitrary widths preserve the original Patch contract.
        series.bar_baseline_visible = false;
        let mut source = Figure::default();
        source.add(vec![series]).unwrap();
        source.validate().unwrap();
        let original = source.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tiny.m");
        assert!(
            files::write_matlab_plot_reconstruction_with_cancel(&path, &source, &stop)
                .unwrap()
                .is_some()
        );
        let restored = replay(&path);
        let patch = &restored.series[0];
        assert!(patch.points.patch().is_some());
        assert_eq!(patch.patch_face_count(), 3);
        let script = patch.points.patch().unwrap().script_coordinates().unwrap();
        // First Bar replay retains the MAT n-by-2 matrix: X column, then Y.
        // Check every original bit, including signed zero and the NaN payload.
        assert!(
            script.sources.iter().any(|backing| {
                backing.len() == source.series[0].points.len() * 2
                    && source.series[0]
                        .points
                        .iter()
                        .enumerate()
                        .all(|(index, point)| {
                            backing.value(index).to_bits() == point[0].to_bits()
                                && backing
                                    .value(source.series[0].points.len() + index)
                                    .to_bits()
                                    == point[1].to_bits()
                        })
            }),
            "width_bits={:016x}; source_pairs={:?}; backing_scalar_bits={:?}; script={}",
            width.to_bits(),
            source.series[0]
                .points
                .iter()
                .map(|point| point.map(f64::to_bits))
                .collect::<Vec<_>>(),
            script
                .sources
                .iter()
                .map(|backing| (0..backing.len())
                    .map(|index| backing.value(index).to_bits())
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            std::fs::read_to_string(&path).unwrap()
        );
        for (index, [x, y]) in source.series[0].points.iter().enumerate() {
            let expected = [
                [(x + 0.) - width / 2., 0.],
                [(x + 0.) + width / 2., 0.],
                [(x + 0.) + width / 2., y],
                [(x + 0.) - width / 2., y],
            ];
            for (point, expected) in patch
                .points
                .face(index, 4)
                .unwrap()
                .as_slice()
                .iter()
                .zip(expected)
            {
                assert_eq!(point.map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
        let shown = patch
            .display_points_axes_cancellable(8192, restored.axis_transforms(), &stop)
            .unwrap();
        assert!(
            shown[8..12]
                .iter()
                .all(|point| point.iter().all(|value| value.is_nan()))
        );
        assert!(source.same_snapshot(&original));
    }
}
