//! Parent-owned genuine bundle producer and relocated public-document replay.
use matrust::{
    Engine, Matrix,
    document::Document,
    files,
    plot::{Figure, PlotKind, Series},
};
use std::{path::PathBuf, sync::atomic::AtomicBool};

fn main() {
    let mut args = std::env::args().skip(1);
    let output = PathBuf::from(args.next().expect("fresh fixture output directory"));
    assert!(args.next().is_none());
    std::fs::create_dir(&output).unwrap();
    let original = output.join("original");
    std::fs::create_dir(&original).unwrap();
    let cancel = AtomicBool::new(false);
    let mut source = Figure {
        name: "Bundle ' \u{03bb}\nwindow".into(),
        ..Figure::default()
    };
    source.title = "Title ' \u{03bb}\nsecond line".into();
    source.xlabel = "X ' \u{03bb}\nunits".into();
    source.ylabel = "Y ' \u{03bb}\nunits".into();
    source.xgrid = true;
    source.ygrid = false;
    for (index, kind) in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::Stairs,
        PlotKind::Stem,
    ]
    .into_iter()
    .enumerate()
    {
        let points = (0..30_000)
            .map(|i| {
                if i == 0 {
                    [-0.0, -0.0]
                } else {
                    [i as f64 + 1.0, (i % 19) as f64 - 7.0 + index as f64 + 0.25]
                }
            })
            .collect();
        let mut series = Series::new(points, kind);
        series.label = format!("Series {} ' \u{03bb}\nlabel", index + 1);
        series.color = [40 + index as u8 * 30, 80, 160];
        if kind == PlotKind::Stem {
            series.base_value = Some(-2.0);
        }
        source.add(vec![series]).unwrap();
    }
    let x = Matrix::from_column_vec(3, 1, vec![1.0, 2.0, 1.0]).unwrap();
    let y = Matrix::from_column_vec(3, 1, vec![1.0, 1.0, 2.0]).unwrap();
    let mut patch = Series::patch_from_matrices(&x, &y, [50, 90, 120], &cancel).unwrap();
    patch.label = "Series 5 ' \u{03bb}\nlabel".into();
    source.add(vec![patch]).unwrap();
    source.validate().unwrap();
    assert_eq!(source.series[4].patch_face_count(), 1);
    let before = source.clone();
    let inline = files::matlab_plot_script(&source, &cancel).unwrap();
    assert!(inline.len() > 1_000_000);
    std::fs::write(output.join("inline-reference.m"), &inline).unwrap();
    let bundle = files::write_matlab_plot_bundle_with_cancel(
        &original.join("reconstruction.m"),
        &source,
        &cancel,
    )
    .unwrap();
    let generation = bundle.companion_directory.file_name().unwrap().to_owned();
    assert_eq!(bundle.script, original.join("reconstruction.m"));
    assert_eq!(bundle.data, bundle.companion_directory.join("data.mat"));
    let names = files::inspect_workspace(&bundle.data, &cancel)
        .unwrap()
        .variables()
        .map(|v| v.name.clone())
        .collect::<Vec<_>>();
    let mut expected_names = (1..=5)
        .map(|i| format!("mr_samples_{i}"))
        .chain((1..=9).map(|i| format!("mr_text_{i}")))
        .collect::<Vec<_>>();
    expected_names.sort();
    let mut observed_names = names.clone();
    observed_names.sort();
    assert_eq!(observed_names, expected_names);
    let script_before = std::fs::read(&bundle.script).unwrap();
    let data_before = std::fs::read(&bundle.data).unwrap();
    let relocated = output.join("moved \u{03bb} ' folder");
    std::fs::rename(&original, &relocated).unwrap();
    let script = relocated.join("reconstruction.m");
    let data = relocated.join(&generation).join("data.mat");
    let document = Document::open(&script).unwrap();
    assert!(document.text.len() < 100_000);
    assert_eq!(document.text.as_bytes(), script_before);
    let mut engine = Engine::default();
    engine.set_file_directory(&output).unwrap();
    let cwd = std::env::current_dir().unwrap();
    engine.execute_document(&document.text, &script).unwrap();
    assert_eq!(std::env::current_dir().unwrap(), cwd);
    let actual = engine.figure.as_ref().unwrap();
    assert_eq!(
        (&actual.name, &actual.title, &actual.xlabel, &actual.ylabel),
        (&source.name, &source.title, &source.xlabel, &source.ylabel)
    );
    assert_eq!((actual.xgrid, actual.ygrid), (source.xgrid, source.ygrid));
    assert!(actual.layout.is_none());
    assert_eq!(actual.series.len(), 5);
    for (expected, replayed) in source.series.iter().zip(&actual.series) {
        assert_eq!(
            (
                expected.kind,
                expected.color,
                expected.visible,
                &expected.label,
                expected.pattern
            ),
            (
                replayed.kind,
                replayed.color,
                replayed.visible,
                &replayed.label,
                replayed.pattern
            )
        );
        assert_eq!(
            expected.base_value.map(f64::to_bits),
            replayed.base_value.map(f64::to_bits)
        );
        assert_eq!(expected.patch_face_count(), replayed.patch_face_count());
        assert_eq!(expected.points.len(), replayed.points.len());
        for (a, b) in expected.points.iter().zip(replayed.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
    assert!(source.same_snapshot(&before));
    for (a, b) in source.series.iter().zip(&before.series) {
        assert!((a.points).same_storage(&b.points));
    }
    assert_eq!(std::fs::read(&script).unwrap(), script_before);
    assert_eq!(std::fs::read(&data).unwrap(), data_before);
    let report = serde_json::json!({
        "status": "public_bundle_and_relocated_document_replay_passed",
        "inline_bytes": inline.len(), "script_bytes": script_before.len(),
        "data_bytes": data_before.len(), "series": 5, "points": [30000,30000,30000,30000,3],
        "names": observed_names, "script": script.strip_prefix(&output).unwrap(),
        "data": data.strip_prefix(&output).unwrap(),
        "source_snapshot_and_sample_arcs_unchanged": true,
        "replayed_exact_all_xy_bits_and_labels": true,
        "limits": ["Own public parser/document replay; no MATLAB engine execution or native/render proof."]
    });
    std::fs::write(
        output.join("fixture.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
