use super::*;
use std::sync::atomic::Ordering;

const REORDERED: &[u8] = include_bytes!("../../tests/fixtures/fig/reordered.fig");
const SUBSET: &[u8] = include_bytes!("../../tests/fixtures/fig/subset.fig");

fn assert_members(figure: &Figure, indices: &[usize]) {
    assert_eq!(
        figure.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
        indices
    );
    let state = figure.legend_state.as_ref().unwrap();
    assert!(
        !state.auto_update,
        "import retains the explicit saved selection"
    );
    assert_eq!(figure.series[0].label, "First");
    assert_eq!(figure.series[1].label, "Second");
    assert!(figure.series[0].visible && !figure.series[1].visible);
    assert_eq!(
        figure.series[0].points.iter().collect::<Vec<_>>(),
        vec![[1., 1.], [10., 2.], [100., 4.]]
    );
    assert_eq!(
        figure.series[1].points.iter().collect::<Vec<_>>(),
        vec![[1., 3.], [10., 4.], [100., 6.]]
    );
}

fn selected_offset(expanded: &mat::Expanded) -> usize {
    let root = expanded.hierarchy().unwrap();
    let all = children(object(&root).unwrap()).unwrap();
    let legend = all
        .iter()
        .find(|n| n["type"].text() == Some("scribe.legend"))
        .unwrap();
    let props = object(field(legend, "properties").unwrap()).unwrap();
    let app = object(field(props, "ApplicationData").unwrap()).unwrap();
    let Value::Number { bytes, kind: 9, .. } = &app["PlotChildren"] else {
        panic!("double handles")
    };
    bytes.as_ptr() as usize - expanded.bytes.as_ptr() as usize
}

fn bad_selected(value: f64) -> Vec<u8> {
    let mut expanded = mat::expand(REORDERED).unwrap();
    let offset = selected_offset(&expanded);
    expanded.bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    let mut result = REORDERED[..128].to_vec();
    result.extend(expanded.bytes);
    result
}

#[test]
fn derived_ordered_and_subset_figs_keep_samples_labels_and_hidden_members() {
    // These are explicitly authored projections of authentic reference fields,
    // not independently generated MATLAB subset/reordered fixtures.
    for (bytes, indices, visible) in [(REORDERED, &[1, 0][..], true), (SUBSET, &[1][..], false)] {
        let figure = decode_matlab_figure(bytes).unwrap();
        assert_members(&figure, indices);
        assert_eq!(figure.legend, visible);
        let mut shown = figure.clone();
        shown.legend = true;
        let svg = shown.svg();
        assert!(
            svg.contains("Second"),
            "hidden series remains a legend member"
        );
        if indices.len() == 2 {
            assert!(svg.find("Second").unwrap() < svg.find("First").unwrap());
        } else {
            assert!(!svg.contains("First"));
        }
        let old = shown.clone();
        shown
            .add(vec![Series::new(vec![[1., 9.]], PlotKind::Line)])
            .unwrap();
        assert_eq!(
            shown.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
            indices
        );
        for (a, b) in old.series.iter().zip(&shown.series) {
            assert!(a.points.same_storage(&b.points));
        }
    }
}

#[test]
fn public_disk_open_native_reopen_and_rejected_fig_write_preserve_membership() {
    let directory = tempfile::tempdir().unwrap();
    for (bytes, indices) in [(REORDERED, &[1, 0][..]), (SUBSET, &[1][..])] {
        let source = directory.path().join("derived.fig");
        let native = directory.path().join("native.mfig");
        std::fs::write(&source, bytes).unwrap();
        let quote = |p: &Path| p.to_string_lossy().replace('\\', "/").replace('\'', "''");
        let mut engine = crate::Engine::default();
        engine
            .execute(&format!("openfig('{}');", quote(&source)))
            .unwrap();
        let original = engine.figure.as_ref().unwrap().clone();
        assert_members(&original, indices);
        assert_eq!(engine.take_figure_file_events().len(), 1);
        engine
            .execute(&format!("savefig('{}');", quote(&native)))
            .unwrap();
        assert!(
            crate::plot::SnapshotStamp::new(&original).matches(engine.figure.as_ref().unwrap())
        );
        let reopened = super::super::load_figure(&native).unwrap();
        assert_members(&reopened, indices);
        assert_eq!(reopened.legend, original.legend);
        assert_eq!(reopened.svg(), original.svg());
        let mut second = crate::Engine::default();
        second
            .execute(&format!("openfig('{}');", quote(&native)))
            .unwrap();
        assert_members(second.figure.as_ref().unwrap(), indices);
        // Public FIG writing remains rejected and preserves the source file.
        assert!(
            engine
                .execute(&format!("savefig('{}');", quote(&source)))
                .is_err()
        );
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert!(
            crate::plot::SnapshotStamp::new(&original).matches(engine.figure.as_ref().unwrap())
        );
    }
}

#[test]
fn bad_membership_and_cancelled_open_never_publish_and_retry_works() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("derived.fig");
    let mut engine = crate::Engine::default();
    engine.execute("plot([1 2],[4 5]);").unwrap();
    let before = engine.figure.as_ref().unwrap().clone();
    let ids = engine.figure_ids();
    let quote = source
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "''");
    let command = format!("openfig('{quote}');");
    for value in [
        18.0001220703125,
        999.,
        0.0001220703125,
        f64::NAN,
        f64::INFINITY,
    ] {
        // Original selection is second,first; replacing second with first duplicates it.
        std::fs::write(&source, bad_selected(value)).unwrap();
        assert!(engine.execute(&command).is_err());
        assert_eq!(engine.figure_ids(), ids);
        assert!(crate::plot::SnapshotStamp::new(&before).matches(engine.figure.as_ref().unwrap()));
        assert!(engine.take_figure_file_events().is_empty());
    }
    std::fs::write(&source, REORDERED).unwrap();
    engine.cancellation().store(true, Ordering::Relaxed);
    assert!(engine.execute(&command).is_err());
    assert_eq!(engine.figure_ids(), ids);
    assert!(crate::plot::SnapshotStamp::new(&before).matches(engine.figure.as_ref().unwrap()));
    engine.cancellation().store(false, Ordering::Relaxed);
    engine.execute(&command).unwrap();
    assert_members(engine.figure.as_ref().unwrap(), &[1, 0]);
    assert_eq!(std::fs::read(&source).unwrap(), REORDERED);
}

#[test]
fn label_count_conflicts_and_foreign_axes_handles_remain_rejected() {
    let expanded = mat::expand(REORDERED).unwrap();
    let root = expanded.hierarchy().unwrap();
    let all = children(object(&root).unwrap()).unwrap();
    let axis = all
        .iter()
        .find(|n| n["type"].text() == Some("axes"))
        .unwrap();
    for labels in [
        vec!["Second".into(), "First".into()],
        vec!["Different".into()],
    ] {
        assert!(
            decode_axes(
                axis,
                true,
                Some(labels),
                Some(vec![1]),
                crate::plot::MAX_POINTS,
                crate::plot::MAX_SERIES,
                10000
            )
            .is_err()
        );
    }
    // A separate admitted axes is represented by a real parsed second node,
    // so the same-figure map must retain axes ownership, not merely existence.
    let second = node(
        "axes",
        7.,
        vec![],
        vec![node("line", 8., vec![], vec![], &[])],
        &[],
    );
    let bytes = mat::file(mat::structure("hgS_070000", second)).unwrap();
    let second_expanded = mat::expand(&bytes).unwrap();
    let second_value = second_expanded.hierarchy().unwrap();
    let second_axis = object(&second_value).unwrap();
    let map = legend_series_handles(&[axis, second_axis]).unwrap();
    let handles = 8_f64.to_le_bytes();
    let props = BTreeMap::from([(
        "ApplicationData".into(),
        Value::Struct(vec![BTreeMap::from([(
            "PlotChildren".into(),
            Value::Number {
                kind: 9,
                endian: mat::Endian::Little,
                bytes: &handles,
                count: 1,
            },
        )])]),
    )]);
    assert!(
        validate_legend_membership(&props, &map, 0)
            .unwrap_err()
            .contains("own axes")
    );
    assert_eq!(
        validate_legend_membership(&props, &map, 1).unwrap(),
        Some(vec![0])
    );
    // The fixed saved-list policy is bounded before any result allocation.
    let too_many = vec![0_u8; 65 * 8];
    let props = BTreeMap::from([(
        "ApplicationData".into(),
        Value::Struct(vec![BTreeMap::from([(
            "PlotChildren".into(),
            Value::Number {
                kind: 9,
                endian: mat::Endian::Little,
                bytes: &too_many,
                count: 65,
            },
        )])]),
    )]);
    assert!(
        validate_legend_membership(&props, &map, 0)
            .unwrap_err()
            .contains("1..64")
    );
}
