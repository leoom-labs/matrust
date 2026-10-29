use matrust::plot::{Axes, AxisScale, AxisTransform, PlotKind, SampleQuery, Series};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};

fn main() {
    let workers: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    assert!([1, 8].contains(&workers));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    println!("scale,points,workers,warmups,repetitions,median_ms,min_ms,max_ms,index");
    for log in [false, true] {
        for count in [1000, 16384, 65536, 131072, 262144, 1000000] {
            let mut axes = Axes::default();
            if log {
                axes.xscale = AxisScale::Log;
                axes.yscale = AxisScale::Log;
            }
            axes.series.push(Series::new(
                (0..count)
                    .map(|i| {
                        let x = i as f64 / (count - 1) as f64;
                        let y = (x * 9.).sin();
                        if log {
                            [10_f64.powf(6. * x - 3.), 10_f64.powf(3. * y)]
                        } else {
                            [x, y]
                        }
                    })
                    .collect(),
                PlotKind::Line,
            ));
            let query = SampleQuery {
                transforms: [AxisTransform {
                    scale: axes.xscale,
                    negative: false,
                }; 2],
                bounds: if log {
                    [-3., 3., -3., 3.]
                } else {
                    [0., 1., -1., 1.]
                },
                pointer: [0.123, 0.321],
                pixels: [960., 560.],
            };
            let mut times = vec![];
            let mut picked = None;
            for i in 0..14 {
                let start = Instant::now();
                picked = black_box(pool.install(|| {
                    axes.nearest_sample(black_box(query), &AtomicBool::new(false))
                        .unwrap()
                }));
                if i >= 3 {
                    times.push(start.elapsed().as_secs_f64() * 1000.);
                }
            }
            times.sort_by(f64::total_cmp);
            let hit = picked.unwrap();
            assert_eq!(hit.point, axes.series[0].points.get(hit.index).unwrap());
            println!(
                "{},{count},{workers},3,11,{:.6},{:.6},{:.6},{}",
                if log { "log" } else { "linear" },
                times[5],
                times[0],
                times[10],
                hit.index
            );
        }
    }
}
