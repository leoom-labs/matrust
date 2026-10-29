//! Release-only public typed MAT load benchmark for the controlled fixture set.
//! Args: <data-directory> <v6|v7|v73|v73-plain> <small|A> <1|8>
//!       <inspect|selected|inspect_load> <timing|profile>
use matrust::{Array, files};
use std::{
    collections::BTreeMap,
    collections::BTreeSet,
    hint::black_box,
    io::Write,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

enum Outcome {
    Catalog(files::MatCatalog),
    Values(BTreeMap<String, Array>),
}

fn verify(value: &Outcome, name: &str) {
    match value {
        Outcome::Catalog(catalog) => {
            assert_eq!(catalog.variables().len(), 5);
            let names: BTreeSet<_> = catalog
                .variables()
                .map(|variable| variable.name.as_str())
                .collect();
            assert_eq!(names, BTreeSet::from(["A", "B", "C", "D", "small"]));
            for variable in catalog.variables() {
                let (rows, cols) = if variable.name == "small" {
                    (262144usize, 8usize)
                } else {
                    (1048576usize, 16usize)
                };
                assert_eq!(variable.dimensions.as_slice(), [rows, cols]);
                assert_eq!(variable.class, "double");
                assert!(!variable.complex);
                assert_eq!(variable.decoded_bytes, Some((rows * cols * 8) as u64));
                assert!(variable.unavailable.is_none(), "{:?}", variable.unavailable);
            }
        }
        Outcome::Values(values) => {
            assert_eq!(values.len(), 1);
            let value = &values[name];
            let matrix = value.as_numeric().expect("Exact real-double fixture");
            let (rows, cols) = if name == "small" {
                (262144, 8)
            } else {
                (1048576, 16)
            };
            assert_eq!((matrix.rows(), matrix.cols()), (rows, cols));
            for (i, sample) in matrix.data().iter().enumerate() {
                let expected = if name == "small" { i } else { i % rows };
                assert_eq!(sample.to_bits(), (expected as f64 / 1024.).to_bits());
            }
            // Snapshot copying must retain the immutable payload.
            assert!(value.same_snapshot(&value.clone()));
        }
    }
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("Release benchmark only");
    }
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        7,
        "See source usage; no fixture generation occurs here"
    );
    let format = args[2].as_str();
    assert!(["v6", "v7", "v73", "v73-plain"].contains(&format));
    let name = args[3].as_str();
    assert!(["small", "A"].contains(&name));
    let workers: usize = args[4].parse().unwrap();
    assert!([1, 8].contains(&workers));
    let operation = args[5].as_str();
    assert!(["inspect", "selected", "inspect_load"].contains(&operation));
    let mode = args[6].as_str();
    assert!(["timing", "profile"].contains(&mode));
    assert!(mode != "profile" || operation == "inspect_load");
    let path = Path::new(&args[1]).join(format!("selective-{format}.mat"));
    let file_bytes = std::fs::metadata(&path).unwrap().len();
    let expected_file_bytes = match format {
        "v6" => 553648544,
        "v7" => 95044377,
        "v73" => 18618672,
        "v73-plain" => 553652736,
        _ => unreachable!("Validated fixture format"),
    };
    assert_eq!(
        file_bytes, expected_file_bytes,
        "Controlled fixture length changed"
    );
    let decoded_bytes = if name == "small" {
        16 * 1024 * 1024
    } else {
        128 * 1024 * 1024
    };
    let cancel = AtomicBool::new(false);
    let names = BTreeSet::from([name.to_owned()]);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    pool.install(|| {
        // The selected-only case intentionally begins from an untimed catalog.
        let catalog = (operation == "selected").then(|| files::inspect_workspace(&path, &cancel).unwrap());
        let run = || match operation {
            "inspect" => Outcome::Catalog(files::inspect_workspace(&path, &cancel).unwrap()),
            "selected" => Outcome::Values(files::load_selected_arrays(catalog.as_ref().unwrap(), &names, &cancel).unwrap()),
            _ => {
                let catalog = files::inspect_workspace(&path, &cancel).unwrap();
                Outcome::Values(files::load_selected_arrays(&catalog, &names, &cancel).unwrap())
            }
        };
        println!("format,name,workers,operation,phase,index,file_bytes,decoded_bytes,elapsed_ns");
        let start = Instant::now();
        let first = run();
        let elapsed = start.elapsed().as_nanos();
        verify(&first, name);
        println!("{format},{name},{workers},{operation},first_cache_uncontrolled,0,{file_bytes},{decoded_bytes},{elapsed}");
        if mode == "profile" {
            println!("READY_RETAINED,{decoded_bytes}");
            std::io::stdout().flush().unwrap();
            // A bounded observation window, not an unbounded stdin handshake.
            // The parent retains the exact process handle and queries OS peaks.
            std::thread::sleep(Duration::from_secs(2));
            black_box(&first);
            return;
        }
        drop(first);
        for round in 0..9 {
            let start = Instant::now();
            let value = run();
            let elapsed = start.elapsed().as_nanos();
            verify(&value, name);
            let phase = if round < 2 { "warmup" } else { "retained" };
            let index = if round < 2 { round } else { round - 2 };
            println!("{format},{name},{workers},{operation},{phase},{index},{file_bytes},{decoded_bytes},{elapsed}");
            // Validation, printing, snapshot check and result drop are untimed.
            drop(value);
        }
    });
}
