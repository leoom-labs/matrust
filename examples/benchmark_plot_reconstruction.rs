//! Whole-operation costs; adaptive bundle and standalone outputs are unequal work.
use matrust::{
    Engine, Matrix,
    document::Document,
    files,
    plot::{Axes, Figure, PlotKind, Series},
};
use std::{hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};

struct Case {
    name: &'static str,
    figure: Figure,
    inline: String,
    inline_admitted: bool,
}

fn line(points: usize) -> Figure {
    let mut figure = Figure {
        axes: Axes {
            title: "Reconstruction benchmark".into(),
            ..Axes::default()
        },
        ..Figure::default()
    };
    figure
        .add(vec![Series::new(
            (0..points)
                .map(|i| {
                    if i == 0 {
                        [-0.0, -0.0]
                    } else {
                        [i as f64 + 0.25, (i % 19) as f64 - 7.25]
                    }
                })
                .collect(),
            PlotKind::Line,
        )])
        .unwrap();
    figure
}

fn case(name: &'static str, figure: Figure, cancel: &AtomicBool) -> Case {
    figure.validate().unwrap();
    let inline = files::matlab_plot_script(&figure, cancel).unwrap();
    let inline_admitted = matrust::syntax::parse(&inline).is_ok();
    Case {
        name,
        figure,
        inline,
        inline_admitted,
    }
}

fn cases(cancel: &AtomicBool) -> Vec<Case> {
    // Untimed, bounded deterministic search finds adjacent point counts around
    // the byte limit. AST/token admission is separately observed, never assumed.
    let (mut lo, mut hi) = (2, 50_000);
    assert!(files::matlab_plot_script(&line(lo), cancel).unwrap().len() <= 1_000_000);
    assert!(files::matlab_plot_script(&line(hi), cancel).unwrap().len() > 1_000_000);
    for _ in 0..16 {
        if hi - lo == 1 {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        if files::matlab_plot_script(&line(mid), cancel).unwrap().len() <= 1_000_000 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    assert_eq!(hi, lo + 1);
    let small = case("small_inline", line(8), cancel);
    assert!(small.inline_admitted);
    let below = case("byte_boundary_below", line(lo), cancel);
    let above = case("byte_boundary_above", line(hi), cancel);
    assert!(below.inline.len() <= 1_000_000 && above.inline.len() > 1_000_000);

    let mut metadata = Figure {
        name: "Sixty four series".into(),
        ..Figure::default()
    };
    metadata.title = "Metadata admission".into();
    for i in 0..64 {
        let mut series = Series::new(
            vec![[1.0, i as f64], [2.0, i as f64 + 0.25]],
            PlotKind::Line,
        );
        series.label = format!("Series {i:02} with bounded descriptive metadata");
        metadata.add(vec![series]).unwrap();
    }
    let mut mixed = Figure {
        name: "Mixed ' \u{03bb}\nfigure".into(),
        ..Figure::default()
    };
    mixed.title = "Original samples".into();
    mixed.xlabel = "X".into();
    mixed.ylabel = "Y".into();
    mixed.ygrid = false;
    for (index, kind) in [
        PlotKind::Line,
        PlotKind::Scatter,
        PlotKind::Stairs,
        PlotKind::Stem,
    ]
    .into_iter()
    .enumerate()
    {
        let mut series = Series::new(
            (0..30_000)
                .map(|i| {
                    if i == 0 {
                        [-0.0, -0.0]
                    } else {
                        [i as f64 + 0.25, (i % 19) as f64 - 7.25 + index as f64]
                    }
                })
                .collect(),
            kind,
        );
        series.label = format!("Mixed {index}");
        if kind == PlotKind::Stem {
            series.base_value = Some(-2.0);
        }
        mixed.add(vec![series]).unwrap();
    }
    let x = Matrix::from_column_vec(3, 1, vec![1.0, 2.0, 1.0]).unwrap();
    let y = Matrix::from_column_vec(3, 1, vec![1.0, 1.0, 2.0]).unwrap();
    mixed
        .add(vec![
            Series::patch_from_matrices(&x, &y, [50, 90, 120], cancel).unwrap(),
        ])
        .unwrap();
    vec![
        small,
        below,
        above,
        case("metadata_64", metadata, cancel),
        case("mixed_120003", mixed, cancel),
    ]
}

fn replay(path: &Path, expected: &Figure, directory: &Path) {
    let document = Document::open(path).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(directory).unwrap();
    engine.execute_document(&document.text, path).unwrap();
    let actual = engine.figure.as_ref().unwrap();
    assert_eq!(
        (
            &actual.name,
            &actual.title,
            &actual.xlabel,
            &actual.ylabel,
            actual.color
        ),
        (
            &expected.name,
            &expected.title,
            &expected.xlabel,
            &expected.ylabel,
            expected.color
        )
    );
    assert_eq!(
        (actual.xgrid, actual.ygrid),
        (expected.xgrid, expected.ygrid)
    );
    assert!(actual.layout.is_none());
    assert_eq!(actual.series.len(), expected.series.len());
    for (a, b) in actual.series.iter().zip(&expected.series) {
        assert_eq!(
            (a.kind, a.color, a.visible, a.pattern, &a.label),
            (b.kind, b.color, b.visible, b.pattern, &b.label)
        );
        assert_eq!(
            a.base_value.map(f64::to_bits),
            b.base_value.map(f64::to_bits)
        );
        assert_eq!(a.patch_face_count(), b.patch_face_count());
        assert_eq!(a.points.len(), b.points.len());
        for (x, y) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(x.map(f64::to_bits), y.map(f64::to_bits));
        }
    }
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("Run this harness in release mode");
    }
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(arguments.is_empty() || arguments == ["--reverse"]);
    let reverse = !arguments.is_empty();
    let cancel = AtomicBool::new(false);
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let cases = cases(&cancel);
    let mut schedule = Vec::new();
    for index in 0..cases.len() {
        for workers in [1, 8] {
            for operation in [
                "adaptive_publish",
                "standalone_publish",
                "standalone_serialize",
            ] {
                schedule.push((index, workers, operation));
            }
        }
    }
    if reverse {
        schedule.reverse();
    }
    println!(
        "case\tworkers\toperation\tsample\tnanoseconds\tpoints\tseries\tinline_bytes\tinline_admitted\troute\tscript_bytes\tdata_bytes\tfiles"
    );
    for (index, workers, operation) in schedule {
        let case = &cases[index];
        let snapshot = case.figure.clone();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for repetition in 0..9 {
            let directory = tempfile::tempdir_in(root.path()).unwrap();
            let path = directory.path().join("reconstruction.m");
            // Includes pool entry and whole public preparation/publication.
            // Setup, retained output destruction, validation, cleanup and printing
            // are outside timing; internal temporary destruction remains included.
            let started = Instant::now();
            let (bundle, serialized) = pool.install(|| match operation {
                "adaptive_publish" => (
                    files::prepare_matlab_plot_reconstruction(&path, &case.figure, &cancel)
                        .unwrap()
                        .publish(true, &cancel, || Ok(()))
                        .unwrap(),
                    None,
                ),
                "standalone_publish" => {
                    files::write_matlab_plot_script_with_cancel(&path, &case.figure, &cancel)
                        .unwrap();
                    (None, None)
                }
                "standalone_serialize" => (
                    None,
                    Some(files::matlab_plot_script(&case.figure, &cancel).unwrap()),
                ),
                _ => unreachable!(),
            });
            let nanoseconds = started.elapsed().as_nanos();
            black_box((&bundle, &serialized));
            let (route, script_bytes, data_bytes, file_count) = if let Some(text) = &serialized {
                assert_eq!(text, &case.inline);
                assert!(!path.exists());
                ("memory_inline", text.len() as u64, 0, 0)
            } else {
                let bytes = std::fs::read(&path).unwrap();
                let data_bytes = if let Some(bundle) = &bundle {
                    assert_eq!(bundle.script, path);
                    assert_eq!(bundle.data, bundle.companion_directory.join("data.mat"));
                    assert_eq!(
                        std::fs::read_dir(&bundle.companion_directory)
                            .unwrap()
                            .count(),
                        2
                    );
                    std::fs::metadata(&bundle.data).unwrap().len()
                } else {
                    assert_eq!(bytes, case.inline.as_bytes());
                    0
                };
                if operation == "adaptive_publish" {
                    assert_eq!(bundle.is_none(), case.inline_admitted);
                    replay(&path, &case.figure, root.path());
                } else if case.inline_admitted {
                    replay(&path, &case.figure, root.path());
                }
                assert_eq!(
                    std::fs::read_dir(directory.path()).unwrap().count(),
                    if bundle.is_some() { 2 } else { 1 }
                );
                (
                    if bundle.is_some() {
                        "bundle"
                    } else {
                        "standalone_inline"
                    },
                    bytes.len() as u64,
                    data_bytes,
                    if bundle.is_some() { 3 } else { 1 },
                )
            };
            assert!(case.figure.same_snapshot(&snapshot));
            for (a, b) in case.figure.series.iter().zip(&snapshot.series) {
                assert!((a.points).same_storage(&b.points));
            }
            assert_eq!(std::env::current_dir().unwrap(), cwd);
            if repetition >= 2 {
                println!(
                    "{}\t{workers}\t{operation}\t{}\t{nanoseconds}\t{}\t{}\t{}\t{}\t{route}\t{script_bytes}\t{data_bytes}\t{file_count}",
                    case.name,
                    repetition - 2,
                    case.figure.point_count(),
                    case.figure.series.len(),
                    case.inline.len(),
                    case.inline_admitted
                );
            }
            drop(serialized);
            drop(bundle);
            directory.close().unwrap();
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    root.close().unwrap();
}
