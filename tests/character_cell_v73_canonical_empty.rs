use hdf5::{ObjectReference1, types::FixedAscii};
use matrust::{Array, files};
use std::{
    collections::BTreeSet,
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn names(s: &[&str]) -> BTreeSet<String> {
    s.iter().map(|s| (*s).into()).collect()
}
fn empty(value: &Array) {
    let Array::Numeric(a) = value else {
        panic!("Unset importer value must be numeric empty")
    };
    assert_eq!((a.rows(), a.cols(), a.len()), (0, 0, 0));
}
#[test]
fn genuine_unset_slots_import_as_explicit_model_empty_with_shared_reference_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authentic.mat");
    // Vendor input SHA444842b8...; public API yields null pointers, not double arrays.
    // Numeric0x0 below is MatRust's explicit importer policy, not a reference assertion.
    std::fs::write(
        &path,
        include_bytes!("fixtures/character-cell-unset-v73.mat"),
    )
    .unwrap();
    let stop = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&path, &stop).unwrap();
    assert_eq!(catalog.variables().count(), 3);
    assert!(catalog.variables().all(|v| v.unavailable.is_none()));
    let loaded =
        files::load_selected_arrays(&catalog, &names(&["AllUnset", "Mixed", "Nested"]), &stop)
            .unwrap();
    let Array::Cell(all) = &loaded["AllUnset"] else {
        panic!()
    };
    assert_eq!((all.rows(), all.cols()), (2, 3));
    for i in 0..6 {
        empty(all.get_linear(i).unwrap());
        assert!(Arc::ptr_eq(
            all.get_linear(0).unwrap(),
            all.get_linear(i).unwrap()
        ));
    }
    let Array::Cell(mixed) = &loaded["Mixed"] else {
        panic!()
    };
    assert_eq!((mixed.rows(), mixed.cols()), (2, 3));
    for i in [0, 3, 5] {
        empty(mixed.get_linear(i).unwrap());
        assert!(Arc::ptr_eq(
            all.get_linear(0).unwrap(),
            mixed.get_linear(i).unwrap()
        ));
    }
    empty(mixed.get_linear(1).unwrap());
    let Array::Character(text) = mixed.get_linear(2).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(text.units(), &[65, 0, 955, 55296]);
    let Array::Cell(inner) = mixed.get_linear(4).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!((inner.rows(), inner.cols()), (1, 3));
    assert!(Arc::ptr_eq(
        inner.get_linear(0).unwrap(),
        all.get_linear(0).unwrap()
    ));
    empty(inner.get_linear(1).unwrap());
    let Array::Character(text) = inner.get_linear(2).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(text.units(), &[65, 0, 955, 55296]);
    let Array::Cell(nested) = &loaded["Nested"] else {
        panic!()
    };
    assert_eq!((nested.rows(), nested.cols()), (1, 2));
    let Array::Cell(inner) = nested.get_linear(0).unwrap().as_ref() else {
        panic!()
    };
    assert_eq!((inner.rows(), inner.cols()), (1, 2));
    for value in [
        inner.get_linear(0).unwrap(),
        inner.get_linear(1).unwrap(),
        nested.get_linear(1).unwrap(),
    ] {
        assert!(Arc::ptr_eq(value, all.get_linear(0).unwrap()));
    }
    stop.store(true, Ordering::Relaxed);
    assert!(files::load_selected_arrays(&catalog, &names(&["AllUnset"]), &stop).is_err());
    stop.store(false, Ordering::Relaxed);
    let retry = files::load_selected_arrays(&catalog, &names(&["AllUnset"]), &stop).unwrap();
    let Array::Cell(retry) = &retry["AllUnset"] else {
        panic!()
    };
    assert_eq!(retry.len(), 6);
    empty(retry.get_linear(0).unwrap());
}
fn class(ds: &hdf5::Dataset, name: &str) {
    ds.new_attr::<FixedAscii<32>>()
        .create("MATLAB_class")
        .unwrap()
        .write_scalar(&FixedAscii::<32>::from_ascii(name).unwrap())
        .unwrap();
}
fn synthetic(path: &Path, variant: usize) {
    {
        let f = hdf5::File::with_options()
            .with_fcpl(|p| p.userblock(512))
            .create(path)
            .unwrap();
        f.create_group("#refs#").unwrap();
        let sentinel = match variant {
            2 => {
                let d = f.new_dataset::<u32>().shape(2).create("/#refs#/a").unwrap();
                d.write_raw(&[0u32, 0]).unwrap();
                d
            }
            3 => {
                let d = f
                    .new_dataset::<u64>()
                    .shape((1, 2))
                    .create("/#refs#/a")
                    .unwrap();
                d.write_raw(&[0u64, 0]).unwrap();
                d
            }
            6 => {
                let d = f
                    .new_dataset::<u64>()
                    .shape(2)
                    .chunk(2)
                    .deflate(1)
                    .create("/#refs#/a")
                    .unwrap();
                d.write_raw(&[0u64, 0]).unwrap();
                d
            }
            _ => {
                let d = f.new_dataset::<u64>().shape(2).create("/#refs#/a").unwrap();
                d.write_raw(&[0u64, if variant == 1 { 3 } else { 0 }])
                    .unwrap();
                d
            }
        };
        class(&sentinel, "canonical empty");
        if variant != 4 {
            sentinel
                .new_attr::<u8>()
                .create("MATLAB_empty")
                .unwrap()
                .write_scalar(&if variant == 5 { 0 } else { 1 })
                .unwrap();
        }
        let cell = f
            .new_dataset::<ObjectReference1>()
            .shape((1, 1))
            .create("C")
            .unwrap();
        class(&cell, "cell");
        cell.write_raw(&[f.reference::<ObjectReference1>("/#refs#/a").unwrap()])
            .unwrap();
        if variant == 0 {
            f.link_hard("/#refs#/a", "RootSentinel").unwrap();
        }
    }
    let mut header = [0u8; 128];
    let desc = b"MATLAB 7.3 MAT-file,";
    header[..desc.len()].copy_from_slice(desc);
    header[124..].copy_from_slice(&[0, 2, b'I', b'M']);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .write_all(&header)
        .unwrap();
}
#[test]
fn canonical_empty_root_misuse_and_nonexact_sentinel_variants_reject() {
    let dir = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    for variant in 0..7 {
        let path = dir.path().join(format!("variant{variant}.mat"));
        synthetic(&path, variant);
        let catalog = files::inspect_workspace(&path, &stop).unwrap();
        if variant == 0 {
            let loaded = files::load_selected_arrays(&catalog, &names(&["C"]), &stop).unwrap();
            let Array::Cell(c) = &loaded["C"] else {
                panic!()
            };
            empty(c.get_linear(0).unwrap());
            assert!(
                catalog
                    .variables()
                    .find(|v| v.name == "RootSentinel")
                    .unwrap()
                    .unavailable
                    .is_some()
            );
            assert!(
                files::load_selected_arrays(&catalog, &names(&["RootSentinel"]), &stop).is_err()
            );
        } else {
            assert!(
                catalog
                    .variables()
                    .find(|v| v.name == "C")
                    .unwrap()
                    .unavailable
                    .is_some(),
                "variant{variant}"
            );
            assert!(files::load_selected_arrays(&catalog, &names(&["C"]), &stop).is_err());
        }
    }
}
