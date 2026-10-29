//! Paired original traversal and candidate; all HDF fixtures are private tests.
use super::super::{Entry, MAX_VARIABLES, open};
use super::*;
use hdf5::{LinkType, LocationType, types::FixedAscii};
use std::{collections::BTreeSet, path::Path, sync::atomic::Ordering};

// Exact preserved traversal, with test-only visibility/lint annotations.
mod legacy {
    use super::*;
    pub(super) fn inspect(
        file: &File,
        dataset: &Dataset,
        cancel: &AtomicBool,
        visits: &mut usize,
    ) -> Result<usize> {
        let mut budget = Budget::default();
        inspect_node(
            file,
            dataset,
            0,
            &mut Vec::new(),
            &mut budget,
            visits,
            cancel,
        )?;
        Ok(budget.bytes)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn inspect_node(
        file: &File,
        dataset: &Dataset,
        depth: usize,
        active: &mut Vec<u64>,
        budget: &mut Budget,
        visits: &mut usize,
        cancel: &AtomicBool,
    ) -> Result<()> {
        check_cancel(cancel)?;
        *visits = visits.checked_add(1).ok_or("Cell inspection overflow")?;
        if *visits > MAX_VISITS {
            return Err("MAT cell inspection exceeds 262144 visited objects".into());
        }
        let info = metadata(dataset, "cell child")?;
        if let Some(reason) = &info.unavailable {
            return Err(reason.clone());
        }
        if info.class != "cell" {
            return budget.charge(
                usize::try_from(info.storage_bytes().ok_or("Unknown cell child size")?)
                    .map_err(|_| "Cell child size overflow")?,
                0,
            );
        }
        if depth >= 16 {
            return Err("Cell nesting exceeds 16 levels".into());
        }
        let address = key(file, dataset)?;
        if active.contains(&address) {
            return Err("Cyclic HDF5 cell references are not supported".into());
        }
        let count = info.dimensions[0] * info.dimensions[1];
        budget.charge(
            count
                .checked_mul(crate::containers::SLOT)
                .ok_or("Cell slot byte overflow")?,
            count,
        )?;
        active.push(address);
        for reference in children(file, dataset, &info, cancel)? {
            check_cancel(cancel)?;
            let child = target(file, &reference)?;
            inspect_node(file, &child, depth + 1, active, budget, visits, cancel)?;
        }
        active.pop();
        check_cancel(cancel)
    }
}

// Exact preserved catalog walk; only name/path resolution is adapted.
fn legacy_catalog(path: &Path, cancel: &AtomicBool) -> Result<Vec<Entry>> {
    let file = open(path)?;
    let mut entries = Vec::new();
    let mut visited = 0;
    let mut container_visits = 0;
    h(file.iter_visit_default(|name, link| {
        check_cancel(cancel).map_err(hdf5::Error::from)?;
        visited += 1;
        if visited > MAX_VARIABLES + 1 {
            return Err("MAT file exceeds 10000 variables".into());
        }
        if name == "#refs#" {
            return Ok(());
        }
        if !crate::files::valid_name(name) {
            return Err("Invalid MAT variable name".into());
        }
        let info = if link.link_type != LinkType::Hard {
            MatVariable {
                name: name.into(),
                dimensions: vec![],
                class: "link".into(),
                complex: false,
                decoded_bytes: None,
                unavailable: Some("External and soft HDF5 links are not supported".into()),
                compressed: false,
            }
        } else if file.loc_type_by_name(name)? == LocationType::Dataset {
            let dataset = file.dataset(name)?;
            let mut info = metadata(&dataset, name).map_err(hdf5::Error::from)?;
            if info.class == "cell" && info.unavailable.is_none() {
                match legacy::inspect(&file, &dataset, cancel, &mut container_visits) {
                    Ok(bytes) => info.decoded_bytes = Some(bytes as u64),
                    Err(reason) => info.unavailable = Some(reason),
                }
                check_cancel(cancel).map_err(hdf5::Error::from)?;
            }
            info
        } else {
            MatVariable {
                name: name.into(),
                dimensions: vec![],
                class: "group".into(),
                complex: false,
                decoded_bytes: None,
                unavailable: Some("Structured and sparse HDF5 variables are not supported".into()),
                compressed: false,
            }
        };
        if entries.len() >= MAX_VARIABLES {
            return Err("MAT file exceeds 10000 variables".into());
        }
        entries.push(Entry {
            info,
            offset: 0,
            length: 0,
            kind: 0,
        });
        Ok(())
    }))?;
    check_cancel(cancel)?;
    Ok(entries)
}

fn class(ds: &Dataset, value: &str) {
    ds.new_attr::<FixedAscii<32>>()
        .create("MATLAB_class")
        .unwrap()
        .write_scalar(&FixedAscii::<32>::from_ascii(value.as_bytes()).unwrap())
        .unwrap();
}
fn leaf(file: &File, name: &str, count: usize) -> Dataset {
    let ds = file
        .new_dataset::<f64>()
        .shape((1, count))
        .create(name)
        .unwrap();
    class(&ds, "double");
    ds
}
fn cell(file: &File, name: &str, names: &[&str]) -> Dataset {
    let ds = file
        .new_dataset::<ObjectReference1>()
        .shape((names.len(), 1))
        .create(name)
        .unwrap();
    class(&ds, "cell");
    let references = names
        .iter()
        .map(|name| file.reference::<ObjectReference1>(name).unwrap())
        .collect::<Vec<_>>();
    ds.write_raw(&references).unwrap();
    ds
}
fn fixture(build: impl FnOnce(&File)) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.h5");
    let file = File::create(&path).unwrap();
    file.create_group("#refs#").unwrap();
    build(&file);
    file.flush().unwrap();
    drop(file);
    (dir, path)
}
fn paired_catalog(path: &Path) -> Vec<MatVariable> {
    let stop = AtomicBool::new(false);
    let old = legacy_catalog(path, &stop)
        .unwrap()
        .into_iter()
        .map(|e| e.info)
        .collect::<Vec<_>>();
    let new = super::super::inspect(path, &stop)
        .unwrap()
        .into_iter()
        .map(|e| e.info)
        .collect::<Vec<_>>();
    assert_eq!(
        new, old,
        "Entire catalog, including later roots, must match"
    );
    new
}
fn sequence(
    file: &File,
    names: &[&str],
    start: usize,
    inspector: &mut Inspector<'_>,
) -> (Vec<MatVariable>, usize) {
    let stop = AtomicBool::new(false);
    let mut old_visits = start;
    let mut visits = start;
    let mut result = Vec::new();
    for name in names {
        let ds = file.dataset(name).unwrap();
        let mut old = metadata(&ds, name).unwrap();
        let mut new = old.clone();
        if old.class == "cell" && old.unavailable.is_none() {
            let old_result = legacy::inspect(file, &ds, &stop, &mut old_visits);
            let new_result = inspector.inspect(&ds, &stop, &mut visits);
            assert_eq!(new_result, old_result, "result for {name}");
            assert_eq!(visits, old_visits, "ordered failure prefix for {name}");
            match old_result {
                Ok(n) => old.decoded_bytes = Some(n as u64),
                Err(e) => old.unavailable = Some(e),
            }
            match new_result {
                Ok(n) => new.decoded_bytes = Some(n as u64),
                Err(e) => new.unavailable = Some(e),
            }
        }
        assert_eq!(new, old);
        result.push(new);
    }
    (result, visits)
}

#[test]
fn repeated_nested_dag_and_hard_links_keep_catalog_costs_and_skip_native_work() {
    let (_dir, path) = fixture(|f| {
        leaf(f, "#refs#/leaf", 4)
            .write_raw(&[-0., 2., f64::from_bits(1), f64::INFINITY])
            .unwrap();
        cell(f, "#refs#/inner", &["#refs#/leaf"; 4]);
        cell(f, "A", &["#refs#/inner"; 512]);
        f.link_hard("A", "Alias").unwrap();
        cell(f, "Z", &["#refs#/leaf"]);
    });
    let infos = paired_catalog(&path);
    assert!(infos.iter().all(|i| i.unavailable.is_none()));
    let file = open(&path).unwrap();
    let mut cached = Inspector::new(&file);
    let (_, count) = sequence(&file, &["A", "Alias", "Z"], 0, &mut cached);
    assert_eq!(count, 2 * (1 + 512 * 5) + 2);
    assert_eq!(cached.completed.len(), 4);
    assert_eq!(cached.probe.metadata, 4);
    assert_eq!(cached.probe.reference_reads, 3);
    assert_eq!(cached.probe.dereferences, 2);
    assert!(cached.probe.hits > 500);
    let mut uncached = Inspector::new(&file);
    uncached.probe.capacity = Some(0);
    sequence(&file, &["A", "Alias", "Z"], 0, &mut uncached);
    assert!(uncached.probe.metadata > 5000);
    assert!(uncached.probe.dereferences > 5000);
    assert!(uncached.probe.reference_reads > 1000);
}

#[test]
fn cycles_null_group_and_external_targets_leave_following_valid_roots_unchanged() {
    let (_dir, path) = fixture(|f| {
        leaf(f, "#refs#/leaf", 1);
        cell(f, "A", &["#refs#/leaf"]);
        let direct = cell(f, "Cycle", &["#refs#/leaf"]);
        direct
            .write_raw(&[f.reference::<ObjectReference1>("Cycle").unwrap()])
            .unwrap();
        let left = cell(f, "#refs#/left", &["#refs#/leaf"]);
        cell(f, "#refs#/right", &["#refs#/leaf", "#refs#/left"]);
        left.write_raw(&[f.reference::<ObjectReference1>("#refs#/right").unwrap()])
            .unwrap();
        cell(f, "Indirect", &["#refs#/left"]);
        let null = f
            .new_dataset::<ObjectReference1>()
            .shape((1, 1))
            .create("Null")
            .unwrap();
        class(&null, "cell");
        cell(f, "Group", &["#refs#"]);
        // Top-level external links remain rejected before following the target.
        f.link_external("unopened-other.h5", "/A", "External")
            .unwrap();
        cell(f, "Z", &["#refs#/leaf"]);
    });
    let infos = paired_catalog(&path);
    for name in ["Cycle", "Indirect", "Null", "Group", "External"] {
        assert!(
            infos
                .iter()
                .find(|i| i.name == name)
                .unwrap()
                .unavailable
                .is_some()
        );
    }
    assert!(
        infos
            .iter()
            .find(|i| i.name == "Z")
            .unwrap()
            .unavailable
            .is_none()
    );
    let file = open(&path).unwrap();
    let mut cached = Inspector::new(&file);
    sequence(
        &file,
        &["A", "Cycle", "Indirect", "Group", "Z"],
        0,
        &mut cached,
    );
    let address = key(&file, &file.dataset("A").unwrap()).unwrap();
    // Even an existing completed entry cannot bypass the active-stack test.
    let mut budget = Budget::default();
    let mut visits = 0;
    assert!(
        cached
            .hit(
                address,
                0,
                &[address],
                &mut budget,
                &mut visits,
                &AtomicBool::new(false)
            )
            .unwrap()
            .is_none()
    );
    assert_eq!((budget.bytes, budget.slots, visits), (0, 0, 0));
}

#[test]
fn cached_depth_and_byte_failures_walk_the_original_prefix_before_later_roots() {
    let (_dir, path) = fixture(|f| {
        leaf(f, "#refs#/leaf", 1);
        let mut prior = "#refs#/leaf".to_owned();
        for depth in 0..16 {
            let name = if depth == 15 {
                "A".into()
            } else {
                format!("#refs#/depth{depth}")
            };
            cell(f, &name, &[&prior]);
            prior = name;
        }
        cell(f, "Deep", &["A"]);
        leaf(f, "#refs#/big", 16 * 1024 * 1024); // metadata only, no 128 MiB payload write
        cell(f, "B", &["#refs#/big"]);
        cell(f, "Budget", &["B", "B", "B"]);
        cell(f, "Z", &["#refs#/leaf"]);
    });
    let infos = paired_catalog(&path);
    assert!(
        infos
            .iter()
            .find(|i| i.name == "Deep")
            .unwrap()
            .unavailable
            .as_ref()
            .unwrap()
            .contains("16 levels")
    );
    assert!(
        infos
            .iter()
            .find(|i| i.name == "Budget")
            .unwrap()
            .unavailable
            .as_ref()
            .unwrap()
            .contains("256 MiB")
    );
    let file = open(&path).unwrap();
    let mut cached = Inspector::new(&file);
    let (infos, _) = sequence(&file, &["A", "Deep", "B", "Budget", "Z"], 0, &mut cached);
    assert!(infos.last().unwrap().unavailable.is_none());
    for name in ["Deep", "Budget"] {
        assert!(
            !cached
                .completed
                .contains_key(&key(&file, &file.dataset(name).unwrap()).unwrap())
        );
    }
}

#[test]
fn cache_hits_keep_expanded_slot_and_visit_limits_and_exact_failure_counters() {
    let (_dir, path) = fixture(|f| {
        leaf(f, "#refs#/empty", 0);
        cell(f, "A", &["#refs#/empty"; 8]);
        cell(f, "Z", &["#refs#/empty"]);
    });
    paired_catalog(&path);
    let file = open(&path).unwrap();
    let mut cached = Inspector::new(&file);
    sequence(&file, &["A", "Z"], 0, &mut cached);
    let (_, at_cap) = sequence(&file, &["A"], MAX_VISITS - 9, &mut cached);
    assert_eq!(at_cap, MAX_VISITS);
    let (failed, after) = sequence(&file, &["A", "Z"], MAX_VISITS - 8, &mut cached);
    assert!(
        failed
            .iter()
            .all(|i| i.unavailable.as_ref().unwrap().contains("visited objects"))
    );
    assert_eq!(after, MAX_VISITS + 2);
    let ds = file.dataset("A").unwrap();
    for remaining in [8, 7, 0] {
        let mut old = Budget {
            bytes: 0,
            slots: MAX_SLOTS - remaining,
        };
        let mut new = Budget {
            bytes: 0,
            slots: MAX_SLOTS - remaining,
        };
        let mut old_visits = 0;
        let mut new_visits = 0;
        let expected = legacy::inspect_node(
            &file,
            &ds,
            0,
            &mut Vec::new(),
            &mut old,
            &mut old_visits,
            &AtomicBool::new(false),
        );
        let address = key(&file, &ds).unwrap();
        let actual = match cached
            .hit(
                address,
                0,
                &[],
                &mut new,
                &mut new_visits,
                &AtomicBool::new(false),
            )
            .unwrap()
        {
            Some(_) => Ok(()),
            None => cached
                .walk(
                    &ds,
                    None,
                    0,
                    &mut Vec::new(),
                    &mut new,
                    &mut new_visits,
                    &AtomicBool::new(false),
                )
                .map(|_| ()),
        };
        assert_eq!(actual, expected);
        assert_eq!(
            (new.bytes, new.slots, new_visits),
            (old.bytes, old.slots, old_visits)
        );
    }
}

#[test]
fn saturated_cache_is_not_a_new_admission_limit() {
    let (_dir, path) = fixture(|f| {
        leaf(f, "#refs#/leaf", 1);
        cell(f, "A", &["#refs#/leaf"; 8]);
        cell(f, "Z", &["#refs#/leaf"]);
    });
    paired_catalog(&path);
    let file = open(&path).unwrap();
    let mut inspector = Inspector::new(&file);
    inspector.probe.capacity = Some(1);
    sequence(&file, &["A", "Z"], 0, &mut inspector);
    assert_eq!(inspector.completed.len(), 1);
    // Exercise the actual production entry ceiling with irrelevant addresses.
    // These entries cannot match this small file; they retain fixed summaries only.
    inspector.completed.clear();
    for i in 0..MAX_OBJECTS {
        inspector.completed.insert(
            u64::MAX - i as u64,
            Summary {
                bytes: 0,
                slots: 0,
                visits: 1,
                height: 0,
            },
        );
    }
    inspector.probe.capacity = None;
    sequence(&file, &["A", "Z"], 0, &mut inspector);
    assert_eq!(inspector.completed.len(), MAX_OBJECTS);
}

#[test]
fn long_numeric_target_name_skips_cache_admission_without_changing_catalog() {
    let (_dir, path) = fixture(|f| {
        let long = format!("#refs#/{}", "x".repeat(4100));
        leaf(f, &long, 1);
        leaf(f, "#refs#/short", 1);
        cell(f, "A", &[&long, &long]);
        cell(f, "Z", &["#refs#/short"]);
    });
    let infos = paired_catalog(&path);
    assert!(infos.iter().all(|i| i.unavailable.is_none()));
    let file = open(&path).unwrap();
    let mut inspector = Inspector::new(&file);
    sequence(&file, &["A", "Z"], 0, &mut inspector);
    assert_eq!(inspector.completed.len(), 3); // A, Z and short; no long leaf entry
    assert_eq!(inspector.probe.dereferences, 3);
}

#[test]
fn cancellation_on_hit_and_publication_never_publishes_an_unfinished_summary() {
    let (_dir, path) = fixture(|f| {
        leaf(f, "#refs#/leaf", 1);
        cell(f, "A", &["#refs#/leaf"; 8]);
    });
    let file = open(&path).unwrap();
    let ds = file.dataset("A").unwrap();
    let stop = AtomicBool::new(false);
    let mut inspector = Inspector::new(&file);
    inspector.probe.cancel_on_publish = true;
    assert!(
        inspector
            .inspect(&ds, &stop, &mut 0)
            .unwrap_err()
            .contains("cancel")
    );
    assert!(inspector.completed.is_empty());
    stop.store(false, Ordering::Relaxed);
    let expected = inspector.inspect(&ds, &stop, &mut 0).unwrap();
    let completed = inspector.completed.clone();
    inspector.probe.cancel_on_hit = true;
    let mut visits = 0;
    assert!(
        inspector
            .inspect(&ds, &stop, &mut visits)
            .unwrap_err()
            .contains("cancel")
    );
    assert_eq!(visits, 0);
    assert_eq!(inspector.completed, completed);
    stop.store(false, Ordering::Relaxed);
    assert_eq!(inspector.inspect(&ds, &stop, &mut 0).unwrap(), expected);
    let mut fresh = Inspector::new(&file);
    assert_eq!(fresh.inspect(&ds, &stop, &mut 0).unwrap(), expected);
}

#[test]
fn cancellation_after_native_reference_read_has_no_summary_and_retries() {
    let (_dir, path) = fixture(|f| {
        leaf(f, "#refs#/leaf", 1);
        cell(f, "A", &["#refs#/leaf"; 8]);
    });
    let file = open(&path).unwrap();
    let ds = file.dataset("A").unwrap();
    let stop = AtomicBool::new(false);
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            super::super::read_probe::ARMED.with(|v| v.set(false));
        }
    }
    let _reset = Reset;
    let mut inspector = Inspector::new(&file);
    super::super::read_probe::ARMED.with(|v| v.set(true));
    assert!(
        inspector
            .inspect(&ds, &stop, &mut 0)
            .unwrap_err()
            .contains("cancel")
    );
    assert!(stop.load(Ordering::Relaxed));
    assert!(inspector.completed.is_empty());
    assert_eq!(inspector.probe.reference_reads, 1);
    assert_eq!(inspector.probe.dereferences, 0);
    stop.store(false, Ordering::Relaxed);
    sequence(&file, &["A"], 0, &mut inspector);
    assert_eq!(inspector.completed.len(), 2);
}

#[test]
fn file_local_memos_do_not_cross_colliding_addresses_and_public_load_keeps_typed_aliases() {
    let (_one, first) = fixture(|f| {
        leaf(f, "#refs#/leaf", 1);
        cell(f, "A", &["#refs#/leaf"]);
    });
    let (_two, second) = fixture(|f| {
        leaf(f, "#refs#/leaf", 7);
        cell(f, "A", &["#refs#/leaf"]);
    });
    let a = open(&first).unwrap();
    let b = open(&second).unwrap();
    let da = a.dataset("A").unwrap();
    let db = b.dataset("A").unwrap();
    let ka = key(&a, &da).unwrap();
    let kb = key(&b, &db).unwrap();
    assert_eq!(
        ka, kb,
        "same creation order should expose colliding file-local addresses"
    );
    let mut ia = Inspector::new(&a);
    let mut ib = Inspector::new(&b);
    let stop = AtomicBool::new(false);
    assert_eq!(
        ia.inspect(&da, &stop, &mut 0).unwrap(),
        crate::containers::SLOT + 8
    );
    assert_eq!(
        ib.inspect(&db, &stop, &mut 0).unwrap(),
        crate::containers::SLOT + 56
    );
    assert!(key(&a, &db).is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed.mat");
    let text = Arc::new(Array::Character(
        CharacterRow::from_units(vec![0, 0xd800, 0xdc00, 65535], &stop).unwrap(),
    ));
    let real = Arc::new(Array::Numeric(
        crate::Matrix::from_column_vec(2, 1, vec![-0., f64::from_bits(1)]).unwrap(),
    ));
    let complex = Arc::new(Array::Complex(
        crate::ComplexMatrix::from_column_vec(1, 1, vec![crate::Complex64::new(-0., 3.)], &stop)
            .unwrap(),
    ));
    let logical = Arc::new(Array::Logical(
        crate::LogicalMatrix::from_numeric(
            &crate::Matrix::from_column_vec(1, 2, vec![0., 1.]).unwrap(),
            &stop,
        )
        .unwrap(),
    ));
    let value = Array::Cell(
        CellArray::from_shared(
            2,
            4,
            vec![
                text.clone(),
                real.clone(),
                complex.clone(),
                logical.clone(),
                text,
                real,
                complex,
                logical,
            ],
            &stop,
        )
        .unwrap(),
    );
    crate::files::save_array_workspace_v73(
        &path,
        &BTreeMap::from([("C".into(), value.clone())]),
        &stop,
    )
    .unwrap();
    paired_catalog(&path);
    let catalog = crate::files::inspect_workspace(&path, &stop).unwrap();
    let result =
        crate::files::load_selected_arrays(&catalog, &BTreeSet::from(["C".into()]), &stop).unwrap();
    let Array::Cell(cell) = &result["C"] else {
        panic!("cell")
    };
    for i in 0..4 {
        assert!(Arc::ptr_eq(
            cell.get_linear(i).unwrap(),
            cell.get_linear(i + 4).unwrap()
        ));
    }
    let Array::Character(text) = cell.get_linear(0).unwrap().as_ref() else {
        panic!("char")
    };
    assert_eq!(text.units(), &[0, 0xd800, 0xdc00, 65535]);
    let Array::Numeric(real) = cell.get_linear(1).unwrap().as_ref() else {
        panic!("real")
    };
    assert_eq!(
        real.data().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        vec![(-0f64).to_bits(), 1]
    );
    let Array::Complex(complex) = cell.get_linear(2).unwrap().as_ref() else {
        panic!("complex")
    };
    assert_eq!(
        (
            complex.data()[0].re.to_bits(),
            complex.data()[0].im.to_bits()
        ),
        ((-0f64).to_bits(), 3f64.to_bits())
    );
    let Array::Logical(logical) = cell.get_linear(3).unwrap().as_ref() else {
        panic!("logical")
    };
    assert_eq!(logical.data(), &[0, 1]);
}
