//! Public MAT format fixtures and engine round trips. No MATLAB oracle.
use matrust::{Array, Engine, files};
use std::sync::atomic::AtomicBool;

fn run(engine: &mut Engine, source: &str) {
    engine
        .execute(source)
        .unwrap_or_else(|e| panic!("{source}\n{e}"));
}

#[test]
fn struct_roundtrips_all_shapes_fields_and_containers() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    run(
        &mut e,
        "scalar=struct('z',42,'a','hello'); row=struct('z',{1,2,3},'a',{{1}, {2}, {3}}); grid=reshape(row([1 2 3 1]),2,2); empty00=struct('z',cell(0,0),'a',cell(0,0)); empty10=struct('z',cell(1,0)); nofields=struct(); nofields0=struct([]); nested=struct('inner',scalar,'cells',{{scalar, {row}}}); cells={scalar,nested}; types=struct('double',[1 2;3 4],'complex',1+2i,'logical',logical([0 1]),'char','hi','single',single([1 2]),'i8',int8(-8),'u8',uint8(8),'i16',int16(-16),'u16',uint16(16),'i32',int32(-32),'u32',uint32(32),'i64',int64(-64),'u64',uint64(64));",
    );
    run(
        &mut e,
        "lookalike=struct('MatRustHandle',1); taglike=struct('MatRustObjectKind',99);",
    );
    let names = [
        "lookalike",
        "taglike",
        "scalar",
        "row",
        "grid",
        "empty00",
        "empty10",
        "nofields",
        "nofields0",
        "nested",
        "cells",
        "types",
    ];
    let args = names
        .iter()
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(",");
    for (i, version) in ["'-v6'", "'-v7'", "'-v7','-nocompression'", "'-v7.3'"]
        .iter()
        .enumerate()
    {
        run(
            &mut e,
            &format!("save('round{i}.mat',{args},{version}); loaded=load('round{i}.mat');"),
        );
        for name in names {
            run(
                &mut e,
                &format!(
                    "assert(isequal({name},loaded.{name})); assert(isequal(size({name}),size(loaded.{name})));"
                ),
            );
        }
        run(
            &mut e,
            "assert(isequal(fieldnames(loaded.scalar),{'z';'a'})); assert(~isreal(loaded.types.complex));",
        );
        for (field, class) in [
            ("double", "double"),
            ("complex", "double"),
            ("logical", "logical"),
            ("char", "char"),
            ("single", "single"),
            ("i8", "int8"),
            ("u8", "uint8"),
            ("i16", "int16"),
            ("u16", "uint16"),
            ("i32", "int32"),
            ("u32", "uint32"),
            ("i64", "int64"),
            ("u64", "uint64"),
        ] {
            run(
                &mut e,
                &format!(
                    "assert(isa(loaded.types.{field},'{class}')); assert(isequal(size(types.{field}),size(loaded.types.{field})));"
                ),
            );
        }
        let cat = files::inspect_workspace(
            &dir.path().join(format!("round{i}.mat")),
            &AtomicBool::new(false),
        )
        .unwrap();
        for info in cat.variables() {
            let original = e.array(&info.name).unwrap();
            assert_eq!(info.class, original.class());
            assert_eq!(info.dimensions.as_slice(), original.dims().as_ref());
            assert_eq!(info.storage_bytes(), Some(original.bytes() as u64));
            assert_eq!(info.unavailable, None);
        }
    }
    run(
        &mut e,
        "save command.mat scalar; clear scalar; load command.mat; assert(scalar.z==42);",
    );
    let out = e.execute("whos -file round0.mat").unwrap();
    let display = out.output.join("\n");
    let scalar = display
        .lines()
        .find(|line| line.starts_with("scalar "))
        .unwrap();
    let columns = scalar.split_whitespace().collect::<Vec<_>>();
    assert_eq!(columns[1], "1x1");
    assert_eq!(columns[2], e.array("scalar").unwrap().bytes().to_string());
    assert_eq!(columns[4], "struct");
}

#[test]
fn struct_save_fields_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    run(&mut e, "s=struct('z',42,'a',struct('x',7),'b','hello');");
    for version in ["-v6", "-v7", "-v7.3"] {
        run(
            &mut e,
            &format!(
                "save('fields.mat','-struct','s','{version}'); t=load('fields.mat'); assert(isequal(s,t)); save('selected.mat','-struct','s','a','b','{version}'); t=load('selected.mat'); assert(~isfield(t,'z')); assert(isequal(t.a,s.a));"
            ),
        );
        run(&mut e, "bad=struct('child',struct('callback',@sin));");
        let error = e
            .execute(&format!("save('bad.mat','bad','{version}')"))
            .unwrap_err();
        assert!(
            error.contains("child")
                && error.contains("callback")
                && error.contains("function handles"),
            "{error}"
        );
        assert!(!dir.path().join("bad.mat").exists());
    }
    run(&mut e, "many=struct('x',{1,2}); n=1;");
    for source in ["many", "n"] {
        let error = e
            .execute(&format!("save('bad.mat','-struct','{source}')"))
            .unwrap_err();
        assert!(error.contains("scalar struct"), "{error}");
    }
    run(
        &mut e,
        "save command_fields.mat -struct s a b; t=load('command_fields.mat'); assert(isequal(t.a,s.a)); assert(~isfield(t,'z'));",
    );
    assert!(
        e.execute("save('bad.mat','-struct','s','missing')")
            .unwrap_err()
            .contains("field")
    );
}

#[test]
fn struct_sparse_fields_follow_existing_version_support() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    run(
        &mut e,
        "s.numeric=sparse([1 0;0 2]); s.logical=sparse(logical([1 0;0 1])); assert(issparse(s.numeric)); assert(issparse(s.logical));",
    );
    for v in ["-v6", "-v7"] {
        run(
            &mut e,
            &format!(
                "save('s.mat','s','{v}'); t=load('s.mat'); assert(isequal(t.s,s)); assert(issparse(t.s.numeric)); assert(issparse(t.s.logical)); changed=t.s; changed.numeric=sparse([9 0;0 2]); assert(~isequal(s,changed));"
            ),
        );
    }
    let error = e.execute("save('s.mat','s','-v7.3')").unwrap_err();
    assert!(
        error.contains("numeric") && error.contains("sparse variables cannot be saved to v7.3 yet"),
        "{error}"
    );
}

#[test]
fn scipy_and_h5py_struct_fixtures_load() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    for (name, data) in [
        (
            "v6",
            include_bytes!("fixtures/struct_mat/scipy-v6.mat").as_slice(),
        ),
        (
            "v7",
            include_bytes!("fixtures/struct_mat/scipy-v7.mat").as_slice(),
        ),
        (
            "v73",
            include_bytes!("fixtures/struct_mat/h5py-v73.mat").as_slice(),
        ),
    ] {
        std::fs::write(dir.path().join(format!("{name}.mat")), data).unwrap();
        run(
            &mut e,
            &format!(
                "load('{name}.mat'); assert(scalar.z==42); assert(isequal(scalar.text,'hello')); assert(scalar.nested.answer==7); assert(isequal(fieldnames(scalar),{{'z';'text';'nested'}}));"
            ),
        );
        if name == "v73" {
            run(
                &mut e,
                "assert(isequal(size(records),[1 2])); assert(records(2).z==2); assert(records(2).nested.answer==11); assert(isempty(sentinel.cell{1}));",
            );
        } else {
            run(
                &mut e,
                "assert(isequal(size(records),[2 2])); assert(records(3).z==3); assert(isequal(records(4).a,'r4')); assert(isequal(fieldnames(records),{'z';'a'}));",
            );
        }
    }
}

#[test]
fn v73_struct_output_has_public_group_and_reference_layout() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    run(
        &mut e,
        "scalar=struct('z',42,'text','hello','nested',struct('answer',7)); records=struct('z',{1,2},'nested',{struct('answer',10),struct('answer',11)}); save('layout.mat','scalar','records','-v7.3');",
    );
    let file = hdf5::File::open(dir.path().join("layout.mat")).unwrap();
    let scalar = file.group("scalar").unwrap();
    let names = scalar
        .attr("MATLAB_fields")
        .unwrap()
        .read_raw::<hdf5::types::VarLenArray<hdf5::types::FixedAscii<1>>>()
        .unwrap();
    assert_eq!(names.len(), 3);
    assert_eq!(
        scalar.dataset("z").unwrap().read_raw::<f64>().unwrap(),
        [42.]
    );
    assert!(scalar.group("nested").is_ok());
    let refs = file.dataset("records/nested").unwrap();
    assert_eq!(refs.shape(), [2, 1]);
    assert_eq!(refs.read_raw::<hdf5::ObjectReference1>().unwrap().len(), 2);
    assert!(file.group("#refs#").is_ok());
    drop(file);
    // Optional manual interoperability capture; test artifacts still live in tempdir.
    if let Ok(path) = std::env::var("MATRUST_STRUCT_CHECK_OUTPUT") {
        std::fs::copy(dir.path().join("layout.mat"), path).unwrap();
    }
    assert!(matches!(e.array("scalar").unwrap(), Array::Struct(_)));
}

#[test]
fn struct_classdef_field_keeps_object_rejection_and_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("SavedObject.m"),
        "classdef SavedObject\nproperties\nx=1\nend\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    e.set_function_directory(dir.path()).unwrap();
    run(
        &mut e,
        "obj=SavedObject(); s=struct('nested',struct('object',obj));",
    );
    for v in ["-v6", "-v7", "-v7.3"] {
        let error = e
            .execute(&format!("save('bad.mat','obj','{v}')"))
            .unwrap_err();
        assert!(
            error.contains("save: objects of class 'SavedObject' cannot be saved yet"),
            "{error}"
        );
        let error = e
            .execute(&format!("save('bad.mat','s','{v}')"))
            .unwrap_err();
        assert!(
            error.contains("s.nested.object")
                && error.contains("objects of class 'SavedObject' cannot be saved yet"),
            "{error}"
        );
        assert!(!dir.path().join("bad.mat").exists());
    }
}

#[test]
fn struct_depth_limit_and_cancellation_are_bounded() {
    use std::{collections::BTreeMap, sync::Arc};
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut value = Array::Numeric(matrust::Matrix::scalar(7.));
    for _ in 0..16 {
        value = matrust::StructArray::from_slots(
            1,
            1,
            vec!["child".into()],
            vec![Arc::new(value)],
            &cancel,
        )
        .unwrap()
        .into();
    }
    assert!(
        matrust::StructArray::from_slots(
            1,
            1,
            vec!["child".into()],
            vec![Arc::new(value.clone())],
            &cancel
        )
        .is_err()
    );
    let data = BTreeMap::from([("s".to_owned(), value)]);
    for save in [
        files::save_array_workspace_uncompressed,
        files::save_array_workspace,
        files::save_array_workspace_v73,
    ] {
        let path = dir.path().join("deep.mat");
        save(&path, &data, &cancel).unwrap();
        let cat = files::inspect_workspace(&path, &cancel).unwrap();
        let loaded =
            files::load_selected_arrays(&cat, &data.keys().cloned().collect(), &cancel).unwrap();
        assert_eq!(loaded["s"].bytes(), data["s"].bytes());
        let diff =
            matrust::engine::compare::array_diff("s", &data["s"], &loaded["s"], &cancel).unwrap();
        assert_eq!(diff.status, matrust::engine::compare::DiffStatus::Same);
        let original = std::fs::read(&path).unwrap();
        assert!(save(&path, &data, &AtomicBool::new(true)).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}

#[test]
fn struct_distribution_and_table_fields_remain_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    run(
        &mut e,
        "distribution=makedist('Normal'); s=struct('nested',struct('distribution',distribution)); t=struct('nested',struct('table',table([1;2])));",
    );
    for v in ["-v6", "-v7", "-v7.3"] {
        let error = e
            .execute(&format!("save('bad.mat','s','{v}')"))
            .unwrap_err();
        assert!(
            error.contains("s.nested.distribution")
                && error.contains("saving structs is not supported yet"),
            "{error}"
        );
        let error = e
            .execute(&format!("save('bad.mat','t','{v}')"))
            .unwrap_err();
        assert!(
            error.contains("nested") && error.contains("table variables cannot be saved yet"),
            "{error}"
        );
        assert!(!dir.path().join("bad.mat").exists());
    }
    assert!(
        e.execute("save('bad.mat','-struct','distribution')")
            .unwrap_err()
            .contains("scalar struct")
    );
}

#[test]
fn struct_malformed_metadata_and_cyclic_references_are_rejected() {
    use hdf5::types::{FixedAscii, VarLenArray};
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    run(&mut e, "s=struct('x',1); save('v5.mat','s','-v6');");
    let path = dir.path().join("v5.mat");
    let mut bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[184..192], &[5, 0, 0, 0, 4, 0, 0, 0]);
    bytes[192..196].fill(0);
    std::fs::write(&path, bytes).unwrap();
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert!(
        catalog
            .variables()
            .next()
            .unwrap()
            .unavailable
            .as_ref()
            .unwrap()
            .contains("field-name length")
    );

    let path = dir.path().join("cycle.mat");
    let file = hdf5::File::with_options()
        .with_fcpl(|p| p.userblock(512))
        .create(&path)
        .unwrap();
    let group = file.create_group("s").unwrap();
    group
        .new_attr::<FixedAscii<6>>()
        .create("MATLAB_class")
        .unwrap()
        .write_scalar(&FixedAscii::<6>::from_ascii("struct").unwrap())
        .unwrap();
    let fields = [VarLenArray::from_slice(&[
        FixedAscii::<1>::from_ascii("x").unwrap()
    ])];
    group
        .new_attr::<VarLenArray<FixedAscii<1>>>()
        .shape(1)
        .create("MATLAB_fields")
        .unwrap()
        .write_raw(&fields)
        .unwrap();
    let references = group
        .new_dataset::<hdf5::ObjectReference1>()
        .shape((1, 1))
        .create("x")
        .unwrap();
    let reference: hdf5::ObjectReference1 = file.reference("/s").unwrap();
    references.write_raw(&[reference]).unwrap();
    drop(references);
    drop(group);
    file.close().unwrap();
    {
        use std::io::Write;
        let mut header = [0u8; 128];
        let description = b"MATLAB 7.3 MAT-file,";
        header[..description.len()].copy_from_slice(description);
        header[124..].copy_from_slice(&[0, 2, b'I', b'M']);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .write_all(&header)
            .unwrap();
    }
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    let error = catalog
        .variables()
        .next()
        .unwrap()
        .unavailable
        .as_ref()
        .unwrap();
    assert!(error.contains("Cyclic"), "{error}");
}
