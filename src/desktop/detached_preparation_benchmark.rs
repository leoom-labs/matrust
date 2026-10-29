//! Parent-owned release measurement of actual detached cache preparation.
//! Explicitly ignored by correctness gates: no execution during source drafting.
use super::*;
use std::{hint::black_box, time::Instant};

fn control(figure: &Figure) -> Vec<Vec<PlotPoint>> {
    let budget = 8192 / figure.series_count().max(1);
    let mut result = Vec::new();
    for (_, axes) in figure.all_axes() {
        let transforms = axes.axis_transforms();
        black_box(axes.bounds());
        for series in &axes.series {
            result.push(
                series
                    .display_points_axes(budget, transforms)
                    .into_iter()
                    .map(|[x, y]| PlotPoint::new(x, y))
                    .collect(),
            );
        }
    }
    result
}

#[test]
#[ignore = "parent-owned release preparation measurements; never part of correctness gate timings"]
fn detached_display_preparation_release_measurements() {
    if cfg!(debug_assertions) {
        panic!("Run this measurement only in release mode");
    }
    let cases = [
        ("line_8192", "plot(1:8192);"),
        ("line_1000000", "plot(1:1000000);"),
        ("negative_log_1000000", "semilogy(1:1000000,-(1:1000000));"),
        (
            "sixty_four_lines_1000000",
            "hold on;for k=1:64;plot(1:15625);end;",
        ),
        ("bars_10000", "bar(1:10000);"),
        (
            "patch_10000_faces",
            "X=repmat([0;1;1;0],1,10000);Y=repmat([0;0;1;1],1,10000);patch('XData',X,'YData',Y,'FaceColor',[1 0 0],'EdgeColor','none');",
        ),
        (
            "four_axes_1000000",
            "subplot(2,2,1);plot(1:250000);subplot(2,2,2);plot(1:250000);subplot(2,2,3);plot(1:250000);subplot(2,2,4);plot(1:250000);",
        ),
        (
            "negative_log_best_stem_1000000",
            "h=stem(1:1000000,-(1:1000000));set(gca,'YScale','log');set(h,'BaseValue',-1);legend('Signal','Location','best');",
        ),
        (
            "best_patch_10000_faces",
            "X=repmat([0;1;1;0],1,10000);Y=repmat([0;0;1;1],1,10000);patch('XData',X,'YData',Y,'FaceColor',[1 0 0],'EdgeColor','none');legend('Faces','Location','best');",
        ),
    ];
    let mut schedule = Vec::new();
    for (index, _) in cases.iter().enumerate() {
        for workers in [1, 8] {
            for variant in ["existing_serial_api", "detached_prepare"] {
                schedule.push((index, workers, variant));
            }
        }
    }
    if std::env::var_os("MATRUST_BENCH_REVERSE").is_some() {
        schedule.reverse();
    }
    // Fixtures and thread-pool creation are outside all samples. This code does
    // not parallelize preparation: worker count is configuration, not a speedup claim.
    let fixtures: Vec<_> = cases
        .iter()
        .map(|(_, script)| {
            let mut engine = matrust::Engine::default();
            engine.execute(script).unwrap();
            let figure = engine.figure.unwrap();
            figure.validate().unwrap();
            figure
        })
        .collect();
    let pools = [
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap(),
        rayon::ThreadPoolBuilder::new()
            .num_threads(8)
            .build()
            .unwrap(),
    ];
    let cancel = AtomicBool::new(false);
    println!(
        "case,workers,variant,original_points,series,axes,display_points,retained_bytes,repeat,elapsed_ns"
    );
    for (index, workers, variant) in schedule {
        let figure = &fixtures[index];
        let pool = &pools[usize::from(workers == 8)];
        let expected = pool.install(|| control(figure));
        let expected_count: usize = expected.iter().map(Vec::len).sum();
        drop(expected);
        let warm = pool
            .install(|| Geometry::prepare(1, figure, &cancel))
            .unwrap();
        assert_eq!(warm.retained_points(), expected_count);
        drop(warm);
        for repeat in 0..7 {
            let (elapsed, points, bytes) = pool.install(|| {
                if variant == "detached_prepare" {
                    let start = Instant::now();
                    let output =
                        black_box(Geometry::prepare(1, black_box(figure), &cancel).unwrap());
                    let elapsed = start.elapsed();
                    let points = output.retained_points();
                    let bytes = output.retained_bytes();
                    drop(output);
                    (elapsed, points, bytes)
                } else {
                    let start = Instant::now();
                    let output = black_box(control(black_box(figure)));
                    let elapsed = start.elapsed();
                    let points = output.iter().map(Vec::len).sum();
                    let bytes = output
                        .iter()
                        .map(|v| v.capacity() * std::mem::size_of::<PlotPoint>())
                        .sum();
                    drop(output);
                    (elapsed, points, bytes)
                }
            });
            assert_eq!(points, expected_count);
            assert!(bytes <= ONE_CACHE_BYTES);
            println!(
                "{},{workers},{variant},{},{},{},{points},{bytes},{repeat},{}",
                cases[index].0,
                figure.point_count(),
                figure.series_count(),
                figure.all_axes().count(),
                elapsed.as_nanos()
            );
        }
    }
}
