//! Public serializer/publication costs; unequal geometry controls, not kernel scaling.
use matrust::{
    Array, Engine, Matrix,
    document::Document,
    files,
    plot::{Figure, PlotKind},
};
use std::{collections::BTreeMap, hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn scene(rows: usize, members: usize, scenario: &str) -> Figure {
    assert!(rows * members <= 8192);
    let mut x = Vec::with_capacity(rows * members);
    let mut y = Vec::with_capacity(rows * members);
    for column in 0..members {
        for row in 0..rows {
            // Deliberately different X columns, finite and strictly increasing.
            x.push(1. + row as f64 * (1 + column % 3) as f64);
            y.push(if row == 0 && column == 0 {
                -0.
            } else {
                (row % 11) as f64 - 5. + column as f64 * 0.25
            });
        }
    }
    let mut engine = Engine::default();
    engine
        .merge_workspace(BTreeMap::from([
            (
                "x".into(),
                Array::Numeric(Matrix::from_column_vec(rows, members, x).unwrap()),
            ),
            (
                "y".into(),
                Array::Numeric(Matrix::from_column_vec(rows, members, y).unwrap()),
            ),
        ]))
        .unwrap();
    engine
        .execute(match scenario {
            "legacy_pitch_control" => "bar(x,y,'EdgeColor','none');",
            "recovered_relative_factors" => {
                "bar(x,y,'GroupWidth',.5,'BarWidth',.25,'EdgeColor','none');"
            }
            _ => unreachable!(),
        })
        .unwrap();
    let mut figure = engine.figure.unwrap();
    assert_eq!(figure.series.len(), members);
    assert!(
        figure
            .series
            .iter()
            .all(|series| series.points.len() == rows)
    );
    let (xlim, ylim) = figure.bounds();
    figure.xlim = Some(xlim);
    figure.ylim = Some(ylim);
    figure.validate().unwrap();
    figure
}

fn verify(source: &Figure, actual: &Figure) {
    assert_eq!(source.series.len(), actual.series.len());
    for (index, (a, b)) in source.series.iter().zip(&actual.series).enumerate() {
        assert_eq!(a.kind, PlotKind::Bar);
        assert_eq!(b.kind, PlotKind::Bar);
        assert_eq!(a.points.len(), b.points.len());
        assert_eq!(a.bar_width.to_bits(), b.bar_width.to_bits());
        assert_eq!(a.bar_offset.to_bits(), b.bar_offset.to_bits());
        // Construction controls retain automatic intent; .m outputs freeze
        // explicit RGB. Compare the actual Light appearance for both paths.
        assert_eq!(
            source
                .resolve_appearance(matrust::plot::BaseTheme::Light)
                .series_color(a),
            actual
                .resolve_appearance(matrust::plot::BaseTheme::Light)
                .series_color(b)
        );
        if a.explicit_color || a.auto_palette.is_none() {
            assert_eq!(a.color, b.color);
        }
        assert_eq!(a.face_edge_color, b.face_edge_color);
        assert_eq!(a.pattern, b.pattern);
        assert_eq!(a.visible, b.visible);
        for (left, right) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(left.map(f64::to_bits), right.map(f64::to_bits));
            let edges = |x: f64, offset: f64, width: f64| {
                [(x + offset) - width / 2., (x + offset) + width / 2.]
            };
            assert_eq!(
                edges(left[0], a.bar_offset, a.bar_width).map(f64::to_bits),
                edges(right[0], b.bar_offset, b.bar_width).map(f64::to_bits)
            );
        }
        let left = source.observe_series_baseline(index).unwrap();
        let right = actual.observe_series_baseline(index).unwrap();
        assert_eq!(left.style.visible, right.style.visible);
        assert_eq!(
            left.style.color.map(f64::to_bits),
            right.style.color.map(f64::to_bits)
        );
        assert_eq!(
            left.style.width_points.to_bits(),
            right.style.width_points.to_bits()
        );
        assert_eq!(left.style.pattern, right.style.pattern);
        assert!(
            left.identity
                .same_object(source.observe_series_baseline(0).unwrap().identity)
        );
        assert!(
            right
                .identity
                .same_object(actual.observe_series_baseline(0).unwrap().identity)
        );
    }
    assert!(actual.baselines.is_empty());
    assert_eq!(source.svg(), actual.svg());
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("Use a release artifact");
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || args == ["--reverse"]);
    let reverse = !args.is_empty();
    let mut schedule = Vec::new();
    for rows in [4, 64, 128] {
        for members in [2, 16, 64] {
            for scenario in ["legacy_pitch_control", "recovered_relative_factors"] {
                for workers in [1, 8] {
                    for operation in ["inline_encode", "bundle_publish"] {
                        schedule.push((rows, members, scenario, workers, operation));
                    }
                }
            }
        }
    }
    assert_eq!(schedule.len(), 72);
    if reverse {
        schedule.reverse();
    }
    eprintln!(
        "hardware={};arch={};os={};logical_processors={};retained=7;warmups=2;reverse={}",
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unreported".into()),
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::thread::available_parallelism().map_or(0, usize::from),
        reverse
    );
    println!("rows\tmembers\tscenario\tworkers\toperation\tsample\tnanoseconds\tscript_bytes");
    let root = tempfile::tempdir().unwrap();
    let cwd = std::env::current_dir().unwrap();
    let stop = AtomicBool::new(false);
    for (rows, members, scenario, workers, operation) in schedule {
        let source = scene(rows, members, scenario);
        let alias = source.clone();
        // Actual emitted-route check is untimed and prevents timing the old fast path
        // under the new-scenario label. No private planner hook is used.
        let probe = files::matlab_plot_script(&source, &stop).unwrap();
        assert!(probe.contains("mr_bars = bar("));
        assert!(!probe.contains("mr_h = patch("));
        assert_eq!(
            probe.contains("'GroupWidth',0.8,'BarWidth',1"),
            scenario == "legacy_pitch_control"
        );
        let mut replay = Engine::default();
        replay.execute(&probe).unwrap();
        verify(&source, replay.figure.as_ref().unwrap());
        drop(probe);
        drop(replay);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for repetition in 0..9 {
            let directory = tempfile::Builder::new()
                .prefix("relative_bar_")
                .tempdir_in(root.path())
                .unwrap();
            let path = directory.path().join("scene.m");
            // Pool entry and result destruction are outside the measured interval.
            let (nanos, text, bundle) = pool.install(|| {
                let start = Instant::now();
                let (text, bundle) = match operation {
                    "inline_encode" => (
                        Some(files::matlab_plot_script(black_box(&source), &stop).unwrap()),
                        None,
                    ),
                    "bundle_publish" => (
                        None,
                        Some(
                            files::write_matlab_plot_bundle_with_cancel(
                                &path,
                                black_box(&source),
                                &stop,
                            )
                            .unwrap(),
                        ),
                    ),
                    _ => unreachable!(),
                };
                (start.elapsed().as_nanos(), text, bundle)
            });
            black_box((&text, &bundle));
            let bytes = if let Some(text) = &text {
                let mut replay = Engine::default();
                replay.execute(text).unwrap();
                verify(&source, replay.figure.as_ref().unwrap());
                text.len()
            } else {
                let document = Document::open(&path).unwrap();
                let mut replay = Engine::default();
                replay
                    .execute_document(&document.text, document.path.as_deref().unwrap())
                    .unwrap();
                verify(&source, replay.figure.as_ref().unwrap());
                document.text.len()
            };
            assert!(source.same_snapshot(&alias));
            for (a, b) in source.series.iter().zip(&alias.series) {
                assert!(a.points.same_storage(&b.points));
            }
            assert_eq!(std::env::current_dir().unwrap(), cwd);
            if repetition >= 2 {
                println!(
                    "{rows}\t{members}\t{scenario}\t{workers}\t{operation}\t{}\t{nanos}\t{bytes}",
                    repetition - 2
                );
            }
            drop(text);
            drop(bundle);
            directory.close().unwrap();
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    root.close().unwrap();
}
