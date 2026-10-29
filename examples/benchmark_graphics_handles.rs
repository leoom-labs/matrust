//! Complete serial metadata statements; configured workers are not parallelism.
use matrust::{Array, CellArray, Engine};
use std::{
    collections::BTreeMap,
    hint::black_box,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    assert!(arguments.is_empty() || arguments == ["--reverse"]);
    let reverse = !arguments.is_empty();
    let mut workers = vec![1, 8];
    let mut shapes = vec![(1, 2), (16, 2), (64, 2), (1, 8192), (16, 8192), (64, 8192)];
    let mut operations = vec![
        "shared_assign_clear",
        "series_property",
        "axes_property",
        "validity",
    ];
    if reverse {
        workers.reverse();
        shapes.reverse();
        operations.reverse();
    }
    println!("workers,series,points_per_series,operation,sample,microseconds");
    for &workers in &workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &(series, points) in &shapes {
            for &operation in &operations {
                let mut engine = Engine::default();
                // Sample creation, initial references and second-figure creation
                // are outside timing. Each run uses the same bounded graph.
                pool.install(||engine.execute(&format!("f=figure(1);a=gca;H=plot(a,ones({points},{series}),'LineWidth',2);h=H(end);other=figure(2);"))).unwrap();
                let reference = engine.array("H").unwrap();
                let original = engine.figures().find(|(id, _)| *id == 1).unwrap().1.clone();
                let script = match operation {
                    "shared_assign_clear" => "alias=H;clear alias;",
                    "series_property" => "set(h,'LineWidth',3);",
                    "axes_property" => "set(a,'XLim',[0 9000]);",
                    "validity" => "live=isgraphics(H);",
                    _ => unreachable!(),
                };
                for iteration in 0..9 {
                    engine.remove_array("live");
                    engine.remove_array("alias");
                    // Pool entry, parsing, validation and replacement of small
                    // result metadata are timed. No rendering or disk I/O.
                    let start = Instant::now();
                    let result = pool.install(|| engine.execute(black_box(script)));
                    let elapsed = start.elapsed();
                    result.unwrap();
                    assert_eq!(engine.current_figure, 2);
                    assert!(reference.same_snapshot(&engine.array("H").unwrap()));
                    let actual = engine.figures().find(|(id, _)| *id == 1).unwrap().1;
                    assert!(original.identity.same_object(&actual.identity));
                    for (old, new) in original.series.iter().zip(&actual.series) {
                        assert!(old.identity.same_object(&new.identity));
                        assert!((old.points).same_storage(&new.points));
                    }
                    match operation {
                        "shared_assign_clear" => assert!(!engine.contains_array("alias")),
                        "series_property" => {
                            assert_eq!(actual.series.last().unwrap().line_width, Some(3.))
                        }
                        "axes_property" => assert_eq!(actual.xlim, Some([0., 9000.])),
                        "validity" => {
                            let Array::Logical(live) = engine.array("live").unwrap() else {
                                panic!()
                            };
                            assert_eq!((live.rows(), live.cols()), (series, 1));
                            assert!(live.data().iter().all(|&v| v == 1));
                        }
                        _ => unreachable!(),
                    }
                    if iteration >= 2 {
                        println!(
                            "{workers},{series},{points},{operation},{},{:.6}",
                            iteration - 2,
                            elapsed.as_secs_f64() * 1e6
                        );
                    }
                }
            }
        }
    }
    let mut counts = vec![1, 8193, 65536];
    if reverse {
        counts.reverse();
    }
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &parts in &counts {
            let mut engine = Engine::default();
            engine
                .execute("f=figure;a=gca;h=plot(a,[1 2],[3 4]);")
                .unwrap();
            let handle = engine.array("h").unwrap();
            let shared_handle = Arc::new(handle.clone());
            let cells: Array = CellArray::from_shared(
                1,
                parts,
                vec![shared_handle; parts],
                &AtomicBool::new(false),
            )
            .unwrap()
            .into();
            engine
                .merge_workspace(BTreeMap::from([("C".into(), cells.clone())]))
                .unwrap();
            let original = engine.figure.clone().unwrap();
            for iteration in 0..9 {
                engine.remove_array("H");
                let start = Instant::now();
                let result = pool.install(|| engine.execute(black_box("H=[C{:}];")));
                let elapsed = start.elapsed();
                result.unwrap();
                let Array::Graphics(actual) = engine.array("H").unwrap() else {
                    panic!()
                };
                let Array::Graphics(h) = &handle else {
                    panic!()
                };
                assert_eq!((actual.rows(), actual.cols()), (1, parts));
                for i in 0..parts {
                    assert!(
                        actual
                            .get_linear(i)
                            .unwrap()
                            .same_object(h.scalar_handle().unwrap())
                    );
                }
                assert!(cells.same_snapshot(&engine.array("C").unwrap()));
                assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
                if iteration >= 2 {
                    println!(
                        "{workers},1,2,many_part_concat_{parts},{},{:.6}",
                        iteration - 2,
                        elapsed.as_secs_f64() * 1e6
                    );
                }
            }
        }
    }
}
