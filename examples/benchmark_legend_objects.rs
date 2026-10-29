//! Serial bounded metadata work under 1/8 configured pools; no rendering.
use matrust::{Array, Engine};
use std::{hint::black_box, time::Instant};
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.is_empty() || args == ["--reverse"]);
    let mut workers = vec![1, 8];
    let mut counts = vec![0usize, 1, 16, 64];
    let mut operations = vec!["create", "reorder", "visibility", "string_query"];
    if !args.is_empty() {
        workers.reverse();
        counts.reverse();
        operations.reverse();
    }
    println!("workers,series,operation,sample,microseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &count in &counts {
            let mut setup =
                String::from("f=figure;a=gca;hold(a,'on');members=cell(1,0);labels=cell(1,0);");
            if count != 0 {
                setup = format!(
                    "f=figure;a=gca;hold(a,'on');members=cell(1,{count});labels=cell(1,{count});"
                );
                for i in 1..=count {
                    setup.push_str(&format!(
                        "members{{{i}}}=plot(a,[0 1],[1 2]);labels{{{i}}}='series{i}';"
                    ));
                }
            }
            let create =
                "lg=legend(a,[members{:}],labels,'Interpreter','none','AutoUpdate','off');";
            for &operation in &operations {
                for iteration in 0..9 {
                    let mut e = Engine::default();
                    e.execute(&setup).unwrap();
                    if operation != "create" {
                        e.execute(create).unwrap();
                    }
                    let before = e.figure.clone().unwrap();
                    let retained = e.array("members").unwrap();
                    let script = match operation {
                        "create" => create.to_owned(),
                        "reorder" => format!(
                            "indices=[{}];lg=legend(a,[members{{indices}}],labels(indices),'Interpreter','none','AutoUpdate','off');",
                            (1..=count)
                                .rev()
                                .map(|i| i.to_string())
                                .collect::<Vec<_>>()
                                .join(" ")
                        ),
                        "visibility" => "set(lg,'Visible','off');".into(),
                        "string_query" => "result=get(lg,'String');".into(),
                        _ => unreachable!(),
                    };
                    // Includes pool entry, parser, metadata validation/copy/publication;
                    // setup, shared input/previous graph, checks and final drops outside.
                    let start = Instant::now();
                    let result = pool.install(|| e.execute(black_box(&script)));
                    let elapsed = start.elapsed();
                    result.unwrap();
                    let after = e.figure.as_ref().unwrap();
                    after.validate().unwrap();
                    assert_eq!(after.legend_members().count(), count);
                    assert!(retained.same_snapshot(&e.array("members").unwrap()));
                    for (a, b) in before.series.iter().zip(&after.series) {
                        assert!((a.points).same_storage(&b.points));
                    }
                    let expected: Vec<_> = if operation == "reorder" {
                        (0..count).rev().collect()
                    } else {
                        (0..count).collect()
                    };
                    assert_eq!(
                        after.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
                        expected
                    );
                    if operation == "visibility" {
                        assert!(!after.legend);
                    }
                    if operation == "string_query" {
                        let Array::Cell(c) = e.array("result").unwrap() else {
                            panic!()
                        };
                        assert_eq!(c.len(), count);
                    }
                    if iteration >= 2 {
                        println!(
                            "{workers},{count},{operation},{},{:.6}",
                            iteration - 2,
                            elapsed.as_secs_f64() * 1e6
                        );
                    }
                }
            }
        }
    }
}
