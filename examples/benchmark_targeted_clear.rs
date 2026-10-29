//! Script-level clear bookkeeping. Requires the separately reviewed targeted-clear layer.
//! Setup, pool dispatch, verification and residual engine/result destruction are untimed.
//! Retained aliases keep payload allocations alive; binding/key destruction is timed.
use matrust::{Array, Engine, LogicalMatrix, Matrix};
use std::{collections::BTreeMap, hint::black_box, sync::atomic::AtomicBool, time::Instant};

const OPERATIONS: [&str; 6] = [
    "named_one",
    "named_two",
    "named_limit_command",
    "named_limit_function",
    "all_bare",
    "all_variables",
];
const ELEMENTS: usize = 4 * 1024 * 1024;

fn workspace(count: usize, numeric: &Array, logical: &Array) -> BTreeMap<String, Array> {
    assert!(matches!(count, 16 | 10000));
    let scalar_numeric: Array = Matrix::scalar(7.).into();
    let scalar_logical: Array = LogicalMatrix::scalar(true).into();
    (0..count)
        .map(|i| {
            let large = i < 4 || i >= count - 4;
            let value = match (large, i % 2 == 0) {
                (true, true) => numeric,
                (true, false) => logical,
                (false, true) => &scalar_numeric,
                (false, false) => &scalar_logical,
            };
            (format!("v{i:05}"), value.clone())
        })
        .collect()
}

fn fresh(template: &BTreeMap<String, Array>) -> Engine {
    let mut engine = Engine::default();
    engine.merge_workspace(template.clone()).unwrap();
    assert_eq!(
        engine.variables.len() + engine.logical_variables.len(),
        template.len()
    );
    assert_eq!(
        engine.workspace_bytes(),
        template.values().map(Array::bytes).sum::<usize>()
    );
    engine
}

fn command(operation: &str, count: usize) -> (String, usize) {
    let remove = match operation {
        "named_one" => 1,
        "named_two" => 2,
        "named_limit_command" | "named_limit_function" => 255.min(count - 2),
        "all_bare" | "all_variables" => count,
        _ => unreachable!(),
    };
    let source = match operation {
        "all_bare" => "clear;".to_owned(),
        "all_variables" => "clear variables;".to_owned(),
        "named_limit_function" => format!(
            "clear({});",
            (0..remove)
                .map(|i| format!("'v{i:05}'"))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => format!(
            "clear {};",
            (0..remove)
                .map(|i| format!("v{i:05}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    };
    (source, remove)
}

fn verify(
    engine: &Engine,
    template: &BTreeMap<String, Array>,
    removed: usize,
    numeric: &Array,
    logical: &Array,
) -> (usize, usize, usize) {
    assert_eq!(
        engine.variables.len() + engine.logical_variables.len(),
        template.len() - removed
    );
    let mut bytes = 0;
    let mut numeric_aliases = 0;
    let mut logical_aliases = 0;
    for (i, (name, expected)) in template.iter().enumerate() {
        let actual = engine.array(name);
        if i < removed {
            assert!(actual.is_none(), "Binding {name} was retained");
        } else {
            let actual = actual.unwrap();
            assert!(
                actual.same_snapshot(expected),
                "Binding {name} copied or changed"
            );
            bytes += actual.bytes();
            numeric_aliases += usize::from(actual.same_snapshot(numeric));
            logical_aliases += usize::from(actual.same_snapshot(logical));
        }
    }
    assert_eq!(engine.workspace_bytes(), bytes);
    assert!(engine.array("ans").is_none());
    if removed < template.len() {
        assert!(numeric_aliases > 0 && logical_aliases > 0);
    } else {
        assert_eq!((numeric_aliases, logical_aliases), (0, 0));
    }
    (bytes, numeric_aliases, logical_aliases)
}

fn preflight(template: &BTreeMap<String, Array>, numeric: &Array, logical: &Array) {
    // Explicit variables/all are names when those bindings exist. This is untimed.
    let mut engine = fresh(template);
    engine
        .execute("variables=1;all=logical(1);clear variables all;")
        .unwrap();
    verify(&engine, template, 0, numeric, logical);
    engine.execute("clear variables;").unwrap();
    verify(&engine, template, template.len(), numeric, logical);
    let mut engine = fresh(template);
    engine.execute("clear;").unwrap();
    verify(&engine, template, template.len(), numeric, logical);
}

// Runtime guard permits debug compilation but rejects debug timing.
#[allow(clippy::assertions_on_constants)]
fn main() {
    // Keep debug builds compilable for all-target Clippy, but reject timing them.
    assert!(!cfg!(debug_assertions), "Release build required");
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.is_empty() || (args.len() == 1 && args[0] == "--reverse"),
        "Only --reverse is supported"
    );
    let reverse = !args.is_empty();
    let cancel = AtomicBool::new(false);
    let numeric_matrix = Matrix::filled(ELEMENTS, 1, 0.25).unwrap();
    let logical_matrix = LogicalMatrix::filled(ELEMENTS, 1, true, &cancel).unwrap();
    let numeric_hash = blake3::hash(bytemuck::cast_slice(numeric_matrix.data()));
    let logical_hash = blake3::hash(logical_matrix.data());
    let numeric: Array = numeric_matrix.clone().into();
    let logical: Array = logical_matrix.clone().into();
    let mut workers = [1, 8];
    let mut sizes = [16, 10000];
    if reverse {
        workers.reverse();
        sizes.reverse();
    }
    let pools: Vec<_> = workers
        .iter()
        .map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(*n)
                .build()
                .unwrap()
        })
        .collect();
    println!(
        "bindings,workers,operation,removed,logical_bytes_before,logical_bytes_after,shared_numeric_aliases_after,shared_logical_aliases_after,round,position,nanoseconds"
    );
    for (pool, workers) in pools.iter().zip(workers) {
        for count in sizes {
            let template = workspace(count, &numeric, &logical);
            let before_bytes: usize = template.values().map(Array::bytes).sum();
            assert!(before_bytes <= 256 * 1024 * 1024);
            assert_eq!(
                template
                    .values()
                    .filter(|a| a.same_snapshot(&numeric))
                    .count(),
                4
            );
            assert_eq!(
                template
                    .values()
                    .filter(|a| a.same_snapshot(&logical))
                    .count(),
                4
            );
            let commands: Vec<_> = OPERATIONS.iter().map(|op| command(op, count)).collect();
            // Entire case runs inside an already-created pool. Timer excludes pool.install.
            pool.install(|| {
                if count == 16 { preflight(&template, &numeric, &logical); }
                for round in 0..9 {
                    for position in 0..OPERATIONS.len() {
                        let offset = (round + position) % OPERATIONS.len();
                        let index = if reverse { OPERATIONS.len() - 1 - offset } else { offset };
                        let (source, removed) = &commands[index];
                        let mut engine = fresh(&template);
                        let started = Instant::now();
                        let execution = engine.execute(black_box(source.as_str()));
                        let elapsed = started.elapsed().as_nanos();
                        let execution = execution.unwrap();
                        assert!(execution.output.is_empty() && !execution.clear_console);
                        let (after_bytes, numeric_aliases, logical_aliases) = verify(&engine, &template, *removed, &numeric, &logical);
                        if round >= 2 {
                            assert!(elapsed > 0);
                            println!("{count},{workers},{},{removed},{before_bytes},{after_bytes},{numeric_aliases},{logical_aliases},{},{position},{elapsed}", OPERATIONS[index], round - 2);
                        }
                        // Result and remaining bindings are destroyed after elapsed is captured.
                        drop(execution);
                        drop(engine);
                    }
                }
            });
            assert_eq!(
                blake3::hash(bytemuck::cast_slice(numeric_matrix.data())),
                numeric_hash
            );
            assert_eq!(blake3::hash(logical_matrix.data()), logical_hash);
        }
    }
}
