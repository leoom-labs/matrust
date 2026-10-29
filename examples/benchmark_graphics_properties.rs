//! Runtime property transactions, not renderer or MATLAB timings.
use matrust::{Array, Engine, Matrix};
use std::{collections::BTreeMap, hint::black_box, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut workers = vec![1, 8];
    let mut sizes = vec![0usize, 1024, 1_000_000];
    let mut operations = vec!["figure_create", "figure_set", "stem_set", "literal_title"];
    if !args.is_empty() {
        workers.reverse();
        sizes.reverse();
        operations.reverse();
    }
    println!("workers,points,operation,sample,microseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &size in &sizes {
            let x =
                Matrix::from_column_vec(size, 1, (0..size).map(|i| i as f64).collect()).unwrap();
            let y = Matrix::filled(size, 1, 3.).unwrap();
            for &operation in &operations {
                for iteration in 0..9 {
                    let mut e = Engine::default();
                    e.merge_workspace(BTreeMap::from([
                        ("X".into(), Array::from(x.clone())),
                        ("Y".into(), Array::from(y.clone())),
                    ]))
                    .unwrap();
                    e.execute("f=figure('Name','baseline');a=gca;h=stem(a,X,Y,'Color',[0 0 1],'Marker','o','MarkerFaceColor',[0 0 1]);").unwrap();
                    let before = e.figure.clone().unwrap();
                    let id = e.current_figure;
                    let script = match operation {
                        "figure_create" => "new=figure('Name','new','Color',[1 0 0]);",
                        "figure_set" => "set(f,'Name','changed','Color',[.2 .4 .6]);",
                        "stem_set" => "set(h,'BaseValue',-2,'MarkerFaceColor','none');",
                        "literal_title" => "title(a,'literal _ title','Interpreter','none');",
                        _ => unreachable!(),
                    };
                    // Setup, input generation/sharing, inspection and final drops
                    // excluded. Pool entry, parse, bounded validation/publication timed.
                    let start = Instant::now();
                    let result = pool.install(|| e.execute(black_box(script)));
                    let elapsed = start.elapsed();
                    result.unwrap();
                    let f = e.figures().find(|(n, _)| *n == id).unwrap().1;
                    f.validate().unwrap();
                    assert_eq!(f.series.len(), before.series.len());
                    for (a, b) in before.series.iter().zip(&f.series) {
                        assert!((a.points).same_storage(&b.points));
                    }
                    assert!(Array::from(x.clone()).same_snapshot(&e.array("X").unwrap()));
                    match operation {
                        "figure_create" => assert_eq!(e.figure.as_ref().unwrap().name, "new"),
                        "figure_set" => {
                            assert_eq!(f.name, "changed");
                            assert_eq!(f.color, [51, 102, 153]);
                        }
                        "stem_set" => {
                            assert_eq!(f.series[0].base_value, Some(-2.));
                            assert!(!f.series[0].marker_filled);
                        }
                        "literal_title" => assert_eq!(f.title, "literal _ title"),
                        _ => unreachable!(),
                    }
                    if iteration >= 2 {
                        println!(
                            "{workers},{size},{operation},{},{:.6}",
                            iteration - 2,
                            elapsed.as_secs_f64() * 1e6
                        );
                    }
                }
            }
        }
    }
}
