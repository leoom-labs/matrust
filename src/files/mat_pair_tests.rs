//! Private cfg(test) experiment. Public selected loading always remains serial.
use super::*;
use std::{
    hint::black_box,
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

pub(super) const MIN_PAIR_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadPhase {
    Decoded,
    Finished,
}

pub(super) type ReadProbe<'a> = Option<&'a (dyn Fn(&str, ReadPhase, &[f64]) + Sync)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strategy {
    Serial,
    Pair,
}

fn selected_pair<'a>(
    catalog: &'a MatCatalog,
    names: &BTreeSet<String>,
    workers: usize,
) -> Option<[&'a Entry; 2]> {
    if catalog.v73 || names.len() != 2 || workers != 8 {
        return None;
    }
    let mut entries = catalog
        .entries
        .iter()
        .filter(|entry| names.contains(&entry.info.name));
    let pair = [entries.next()?, entries.next()?];
    if entries.next().is_some()
        || pair.iter().any(|entry| {
            entry.kind != 15
                || !entry.info.compressed
                || entry.info.class != "double"
                || entry.info.complex
                || entry.info.unavailable.is_some()
                || entry
                    .info
                    .decoded_bytes
                    .is_none_or(|bytes| bytes < MIN_PAIR_BYTES)
        })
    {
        return None;
    }
    Some(pair)
}

fn pair_job(
    catalog: &MatCatalog,
    entry: &Entry,
    cancel: &AtomicBool,
    preserve_logical: bool,
    probe: ReadProbe<'_>,
) -> Result<Array> {
    let result = (|| {
        check_cancel(cancel)?;
        // Independent open, not try_clone: each reader owns its seek position.
        let file = File::open(&catalog.path).map_err(|error| error.to_string())?;
        if stamp(&file)? != catalog.stamp {
            return Err("MAT file changed since inspection; inspect it again".into());
        }
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let value = read_selected_entry(&mut reader, entry, cancel, preserve_logical, probe)?;
        check_cancel(cancel)?;
        if stamp(reader.get_ref())? != catalog.stamp {
            return Err("MAT file changed while loading".into());
        }
        Ok(value)
    })();
    if let Some(probe) = probe {
        probe(&entry.info.name, ReadPhase::Finished, &[]);
    }
    result
}

fn load_experiment(
    catalog: &MatCatalog,
    names: &BTreeSet<String>,
    cancel: &AtomicBool,
    preserve_logical: bool,
    strategy: Strategy,
    probe: ReadProbe<'_>,
) -> Result<BTreeMap<String, Array>> {
    // Exact common production preflight, before scheduling or payload allocation.
    let file = selected_file(catalog, names, cancel, preserve_logical)?;
    let pair = (strategy == Strategy::Pair)
        .then(|| selected_pair(catalog, names, crate::parallel::current_worker_count()))
        .flatten();
    let Some([first, second]) = pair else {
        return load_prepared(catalog, names, cancel, preserve_logical, file, probe);
    };
    let (first_result, second_result) = rayon::join(
        || pair_job(catalog, first, cancel, preserve_logical, probe),
        || pair_job(catalog, second, cancel, preserve_logical, probe),
    );
    // Both jobs are joined before propagating either failure. Catalog order,
    // never completion order, selects simultaneous decode errors.
    let first_value = first_result?;
    let second_value = second_result?;
    check_cancel(cancel)?;
    if stamp(&file)? != catalog.stamp
        || stamp(&File::open(&catalog.path).map_err(|error| error.to_string())?)? != catalog.stamp
    {
        return Err("MAT file changed while loading".into());
    }
    Ok(BTreeMap::from([
        (first.info.name.clone(), first_value),
        (second.info.name.clone(), second_value),
    ]))
}

const BITS: [u64; 8] = [
    0,
    0x8000_0000_0000_0000,
    0x7ff8_0000_0000_1234,
    0x400a_0000_0000_0000,
    0xc01e_0000_0000_0000,
    0x7fef_ffff_ffff_ffff,
    1,
    0x7ff0_0000_0000_0000,
];
const PAIR_ELEMENTS: usize = MIN_PAIR_BYTES as usize / 8;

#[derive(Clone, Copy)]
struct Spec<'a> {
    name: &'a str,
    rows: usize,
    cols: usize,
    compressed: bool,
}

fn element(writer: &mut impl Write, kind: u32, bytes: &[u8]) {
    writer.write_all(&kind.to_le_bytes()).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .unwrap();
    writer.write_all(bytes).unwrap();
    writer
        .write_all(&[0; 7][..(8 - bytes.len() % 8) % 8])
        .unwrap();
}

fn matrix(writer: &mut impl Write, spec: Spec<'_>) {
    let mut metadata = Vec::new();
    element(&mut metadata, 6, &[6, 0, 0, 0, 0, 0, 0, 0]);
    element(
        &mut metadata,
        5,
        &[
            (spec.rows as i32).to_le_bytes(),
            (spec.cols as i32).to_le_bytes(),
        ]
        .concat(),
    );
    element(&mut metadata, 1, spec.name.as_bytes());
    let count = spec.rows * spec.cols;
    writer.write_all(&14u32.to_le_bytes()).unwrap();
    writer
        .write_all(&((metadata.len() + 8 + count * 8) as u32).to_le_bytes())
        .unwrap();
    writer.write_all(&metadata).unwrap();
    writer.write_all(&9u32.to_le_bytes()).unwrap();
    writer
        .write_all(&((count * 8) as u32).to_le_bytes())
        .unwrap();
    let mut block = [0u8; 65536];
    for (i, bytes) in block.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        bytes.copy_from_slice(&BITS[i % BITS.len()].to_le_bytes());
    }
    for start in (0..count).step_by(8192) {
        writer
            .write_all(&block[..(count - start).min(8192) * 8])
            .unwrap();
    }
}

fn fixture(path: &Path, specs: &[Spec<'_>]) {
    let mut file = File::create(path).unwrap();
    let mut header = [b' '; 128];
    header[116..124].fill(0);
    header[124..128].copy_from_slice(&[0, 1, b'I', b'M']);
    file.write_all(&header).unwrap();
    for &spec in specs {
        if spec.compressed {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            matrix(&mut encoder, spec);
            let encoded = encoder.finish().unwrap();
            file.write_all(&15u32.to_le_bytes()).unwrap();
            file.write_all(&(encoded.len() as u32).to_le_bytes())
                .unwrap();
            file.write_all(&encoded).unwrap();
        } else {
            matrix(&mut file, spec);
        }
    }
}

fn specs() -> [Spec<'static>; 2] {
    [
        Spec {
            name: "Z",
            rows: PAIR_ELEMENTS,
            cols: 1,
            compressed: true,
        },
        Spec {
            name: "A",
            rows: 1,
            cols: PAIR_ELEMENTS + 8,
            compressed: true,
        },
    ]
}

fn names(catalog: &MatCatalog) -> BTreeSet<String> {
    catalog
        .variables()
        .map(|value| value.name.clone())
        .collect()
}

fn pool(workers: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
}

fn verify(values: &BTreeMap<String, Array>, specs: &[Spec<'_>]) {
    assert_eq!(values.len(), specs.len());
    for spec in specs {
        let value = &values[spec.name];
        let matrix = value.as_numeric().unwrap();
        assert_eq!((matrix.rows(), matrix.cols()), (spec.rows, spec.cols));
        for (i, value) in matrix.data().iter().enumerate() {
            assert_eq!(value.to_bits(), BITS[i % BITS.len()], "{} {i}", spec.name);
        }
        assert!(value.same_snapshot(&value.clone()));
    }
}

#[test]
fn pair_real_chunks_preserve_ieee_bits_shape_and_public_serial_loading() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.mat");
    let specs = specs();
    fixture(&path, &specs);
    let original = std::fs::read(&path).unwrap();
    let cancel = AtomicBool::new(false);
    let catalog = inspect_workspace(&path, &cancel).unwrap();
    let names = names(&catalog);
    assert_eq!(catalog.entries[0].info.name, "Z");
    assert_eq!(names.first().unwrap(), "A");
    for workers in [1, 8] {
        pool(workers).install(|| {
            for strategy in [Strategy::Serial, Strategy::Pair] {
                let completed = AtomicUsize::new(0);
                let chunks = AtomicUsize::new(0);
                let observer = |_: &str, phase, data: &[f64]| match phase {
                    ReadPhase::Finished => {
                        completed.fetch_add(1, Ordering::Relaxed);
                    }
                    ReadPhase::Decoded => {
                        assert!(!data.is_empty());
                        assert_eq!(data[0].to_bits(), BITS[0]);
                        chunks.fetch_add(1, Ordering::Relaxed);
                    }
                };
                let values =
                    load_experiment(&catalog, &names, &cancel, true, strategy, Some(&observer))
                        .unwrap();
                verify(&values, &specs);
                assert_eq!(
                    completed.load(Ordering::Relaxed),
                    if workers == 8 && strategy == Strategy::Pair {
                        2
                    } else {
                        0
                    }
                );
                assert!(chunks.load(Ordering::Relaxed) >= 512);
            }
            verify(
                &load_selected_arrays(&catalog, &names, &cancel).unwrap(),
                &specs,
            );
        });
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn pair_admission_keeps_exact_budget_and_serial_fallbacks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.mat");
    fixture(&path, &specs());
    let cancel = AtomicBool::new(false);
    let catalog = inspect_workspace(&path, &cancel).unwrap();
    let selected = names(&catalog);
    assert!(selected_pair(&catalog, &selected, 8).is_some());
    for workers in [0, 1, 2, 7, 16, 64] {
        assert!(selected_pair(&catalog, &selected, workers).is_none());
    }
    for mode in 0..7 {
        let mut changed = catalog.clone();
        match mode {
            0 => changed.v73 = true,
            1 => changed.entries[0].info.decoded_bytes = Some(MIN_PAIR_BYTES - 1),
            2 => changed.entries[0].info.complex = true,
            3 => changed.entries[0].info.class = "logical".into(),
            4 => changed.entries[0].kind = 14,
            5 => changed.entries[0].info.compressed = false,
            _ => changed.entries[0].info.unavailable = Some("unsupported".into()),
        }
        assert!(selected_pair(&changed, &selected, 8).is_none());
    }
    let mut three = catalog.clone();
    let mut extra = three.entries[0].clone();
    extra.info.name = "third".into();
    three.entries.push(extra);
    assert!(selected_pair(&three, &names(&three), 8).is_none());
    let mut boundary = catalog.clone();
    for entry in &mut boundary.entries {
        entry.info.decoded_bytes = Some(128 * 1024 * 1024);
    }
    // Metadata-only budget boundary; do not pretend this decodes 256 MiB.
    assert!(selected_file(&boundary, &selected, &cancel, true).is_ok());
    boundary.entries[1].info.decoded_bytes = Some(128 * 1024 * 1024 + 8);
    assert!(
        load_experiment(&boundary, &selected, &cancel, true, Strategy::Pair, None)
            .unwrap_err()
            .contains("256 MiB")
    );
    assert!(selected_file(&catalog, &BTreeSet::new(), &cancel, true).is_err());
    assert!(selected_file(&catalog, &BTreeSet::from(["missing".into()]), &cancel, true).is_err());
    let stopped = AtomicBool::new(true);
    assert!(
        load_experiment(&catalog, &selected, &stopped, true, Strategy::Pair, None)
            .unwrap_err()
            .contains("cancelled")
    );
    let mut fallback = specs();
    fallback[0].compressed = false;
    fallback[1].cols = PAIR_ELEMENTS - 1;
    fixture(&path, &fallback);
    let catalog = inspect_workspace(&path, &cancel).unwrap();
    pool(8).install(|| {
        let observer = |_: &str, phase, _: &[f64]| assert!(phase != ReadPhase::Finished);
        verify(
            &load_experiment(
                &catalog,
                &names(&catalog),
                &cancel,
                true,
                Strategy::Pair,
                Some(&observer),
            )
            .unwrap(),
            &fallback,
        );
        let one = BTreeSet::from([fallback[0].name.into()]);
        verify(
            &load_experiment(&catalog, &one, &cancel, true, Strategy::Pair, None).unwrap(),
            &fallback[..1],
        );
    });
}

#[test]
fn pair_errors_follow_catalog_order_and_validate_checksums_and_trailers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.mat");
    fixture(&path, &specs());
    let cancel = AtomicBool::new(false);
    let catalog = inspect_workspace(&path, &cancel).unwrap();
    let original = std::fs::read(&path).unwrap();
    let selected = names(&catalog);
    pool(8).install(|| {
        for corrupt in [[true, false], [false, true], [true, true]] {
            let mut bytes = original.clone();
            for (entry, bad) in catalog.entries.iter().zip(corrupt) {
                if bad {
                    bytes[entry.offset as usize + 8 + entry.length as usize - 1] ^= 1;
                }
            }
            std::fs::write(&path, bytes).unwrap();
            let fresh = inspect_workspace(&path, &cancel).unwrap();
            let expected = load_selected_arrays(&fresh, &selected, &cancel).unwrap_err();
            assert!(expected.starts_with(if corrupt[0] { "Z:" } else { "A:" }));
            let completed = AtomicUsize::new(0);
            let observer = |_: &str, phase, _: &[f64]| {
                if phase == ReadPhase::Finished {
                    completed.fetch_add(1, Ordering::Relaxed);
                }
            };
            let actual = load_experiment(
                &fresh,
                &selected,
                &cancel,
                true,
                Strategy::Pair,
                Some(&observer),
            )
            .unwrap_err();
            assert_eq!(actual, expected);
            assert_eq!(completed.load(Ordering::Relaxed), 2);
            if corrupt == [true, false] {
                verify(
                    &load_experiment(
                        &fresh,
                        &BTreeSet::from(["A".into()]),
                        &cancel,
                        true,
                        Strategy::Pair,
                        None,
                    )
                    .unwrap(),
                    &specs()[1..],
                );
            }
        }
        let first = &catalog.entries[0];
        let mut bytes = original.clone();
        bytes.insert(first.offset as usize + 8 + first.length as usize, 0);
        bytes[first.offset as usize + 4..first.offset as usize + 8]
            .copy_from_slice(&(first.length + 1).to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let fresh = inspect_workspace(&path, &cancel).unwrap();
        let expected = load_selected_arrays(&fresh, &selected, &cancel).unwrap_err();
        assert!(expected.contains("trailing data"));
        assert_eq!(
            load_experiment(&fresh, &selected, &cancel, true, Strategy::Pair, None).unwrap_err(),
            expected
        );
        // Corrupt only the cached extent to exercise the unchanged outer-tag guard.
        std::fs::write(&path, &original).unwrap();
        let mut fresh = inspect_workspace(&path, &cancel).unwrap();
        fresh.entries[0].length -= 1;
        assert_eq!(
            load_experiment(&fresh, &selected, &cancel, true, Strategy::Pair, None).unwrap_err(),
            "MAT array changed since inspection"
        );
        let mut fresh = inspect_workspace(&path, &cancel).unwrap();
        fresh.entries[0].info.dimensions.swap(0, 1);
        assert_eq!(
            load_experiment(&fresh, &selected, &cancel, true, Strategy::Pair, None).unwrap_err(),
            "Z: MAT metadata changed since inspection"
        );
    });
}

#[test]
fn pair_cancellation_follows_real_decoded_work_and_joins_both_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.mat");
    fixture(&path, &specs());
    let cancel = AtomicBool::new(false);
    let catalog = inspect_workspace(&path, &cancel).unwrap();
    let selected = names(&catalog);
    let original = std::fs::read(&path).unwrap();
    let chunks = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let observer = |_: &str, phase, data: &[f64]| match phase {
        ReadPhase::Decoded => {
            assert_eq!(data.len(), 8192);
            for (i, value) in data.iter().enumerate() {
                assert_eq!(value.to_bits(), BITS[i % BITS.len()]);
            }
            chunks.fetch_add(1, Ordering::SeqCst);
            cancel.store(true, Ordering::SeqCst);
        }
        ReadPhase::Finished => {
            completed.fetch_add(1, Ordering::SeqCst);
        }
    };
    let result = pool(8).install(|| {
        load_experiment(
            &catalog,
            &selected,
            &cancel,
            true,
            Strategy::Pair,
            Some(&observer),
        )
    });
    assert!(result.unwrap_err().contains("cancelled"));
    assert!((1..=2).contains(&chunks.load(Ordering::SeqCst)));
    assert_eq!(completed.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    cancel.store(false, Ordering::SeqCst);
    verify(
        &pool(8)
            .install(|| load_experiment(&catalog, &selected, &cancel, true, Strategy::Pair, None))
            .unwrap(),
        &specs(),
    );
}

#[test]
fn pair_rejects_observed_file_changes_before_publishing_any_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.mat");
    fixture(&path, &specs());
    let cancel = AtomicBool::new(false);
    let catalog = inspect_workspace(&path, &cancel).unwrap();
    let selected = names(&catalog);
    let changed = AtomicBool::new(false);
    let completed = AtomicUsize::new(0);
    let observer = |_: &str, phase, data: &[f64]| {
        if phase == ReadPhase::Decoded && !changed.swap(true, Ordering::SeqCst) {
            assert!(!data.is_empty());
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(&[0])
                .unwrap();
        }
        if phase == ReadPhase::Finished {
            completed.fetch_add(1, Ordering::SeqCst);
        }
    };
    let error = pool(8)
        .install(|| {
            load_experiment(
                &catalog,
                &selected,
                &cancel,
                true,
                Strategy::Pair,
                Some(&observer),
            )
        })
        .unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert!(changed.load(Ordering::SeqCst));
    assert_eq!(completed.load(Ordering::SeqCst), 2);
    assert!(
        load_experiment(&catalog, &selected, &cancel, true, Strategy::Pair, None)
            .unwrap_err()
            .contains("changed since inspection")
    );
}

fn verify_measured(values: &BTreeMap<String, Array>, selected: &BTreeSet<String>) -> String {
    assert_eq!(
        values.keys().collect::<Vec<_>>(),
        selected.iter().collect::<Vec<_>>()
    );
    let mut digest = blake3::Hasher::new();
    for (name, value) in values {
        let m = value.as_numeric().unwrap();
        let (rows, cols) = if name == "small" {
            (262144, 8)
        } else {
            (1048576, 16)
        };
        assert_eq!((m.rows(), m.cols()), (rows, cols));
        digest.update(name.as_bytes());
        digest.update(&(rows as u64).to_le_bytes());
        digest.update(&(cols as u64).to_le_bytes());
        for (i, value) in m.data().iter().enumerate() {
            let expected = (if name == "small" { i } else { i % rows }) as f64 / 1024.;
            assert_eq!(value.to_bits(), expected.to_bits(), "{name} {i}");
            digest.update(&value.to_bits().to_le_bytes());
        }
        assert!(value.same_snapshot(&value.clone()));
    }
    digest.finalize().to_hex().to_string()
}

#[test]
#[ignore = "Root-owned fixed release experiment; existing pinned MAT fixtures only"]
fn benchmark_selected_pair_fixed() {
    if cfg!(debug_assertions) {
        panic!("Release measurement required");
    }
    let data = PathBuf::from(
        std::env::var_os("MATRUST_MAT_PAIR_DATA").expect("Explicit existing fixture directory"),
    );
    let order = std::env::var("MATRUST_MAT_PAIR_ORDER").expect("Explicit normal or reverse");
    assert!(["normal", "reverse"].contains(&order.as_str()));
    let selections = ["small", "A", "A+small", "A+B"];
    let formats = ["v6", "v7", "v73", "v73-plain"];
    let mut cases = Vec::new();
    for format in formats {
        for selection in selections {
            for operation in ["selected", "inspect_load"] {
                for workers in [1, 8] {
                    cases.push((format, selection, operation, workers));
                }
            }
        }
    }
    assert_eq!(cases.len(), 64);
    let strategies = if order == "normal" {
        [Strategy::Serial, Strategy::Pair]
    } else {
        cases.reverse();
        [Strategy::Pair, Strategy::Serial]
    };
    let one = pool(1);
    let eight = pool(8);
    println!(
        "order,format,selection,operation,workers,strategy,phase,index,file_bytes,decoded_bytes,elapsed_ns,checksum"
    );
    for (format, selection, operation, workers) in cases {
        let path = data.join(format!("selective-{format}.mat"));
        let file_bytes = std::fs::metadata(&path).unwrap().len();
        let expected_bytes = match format {
            "v6" => 553648544,
            "v7" => 95044377,
            "v73" => 18618672,
            _ => 553652736,
        };
        assert_eq!(file_bytes, expected_bytes);
        let selected: BTreeSet<_> = selection.split('+').map(str::to_owned).collect();
        let decoded: u64 = selected
            .iter()
            .map(|name| if name == "small" { 16 } else { 128 })
            .sum::<u64>()
            * 1024
            * 1024;
        let cancel = AtomicBool::new(false);
        let prepared =
            (operation == "selected").then(|| inspect_workspace(&path, &cancel).unwrap());
        let caller = if workers == 1 { &one } else { &eight };
        caller.install(|| {
            for strategy in strategies {
                for repetition in 0..9 {
                    let start = Instant::now();
                    let values = if let Some(catalog) = &prepared {
                        load_experiment(catalog, &selected, &cancel, true, strategy, None)
                    } else {
                        inspect_workspace(&path, &cancel).and_then(|catalog| load_experiment(&catalog, &selected, &cancel, true, strategy, None))
                    }.unwrap();
                    let elapsed = start.elapsed().as_nanos();
                    black_box(&values);
                    let checksum = verify_measured(&values, &selected);
                    let (phase, index) = if repetition < 2 { ("warmup", repetition) } else { ("retained", repetition - 2) };
                    println!("{order},{format},{selection},{operation},{workers},{strategy:?},{phase},{index},{file_bytes},{decoded},{elapsed},{checksum}");
                    drop(values);
                }
            }
        });
    }
}
