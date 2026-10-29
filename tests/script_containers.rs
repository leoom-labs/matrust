use matrust::{Array, CellArray, CharacterRow, Engine, Matrix, files};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn cell(e: &Engine, name: &str) -> CellArray {
    let Array::Cell(c) = e.array(name).unwrap() else {
        panic!("Expected cell {name}")
    };
    c
}
fn chars(e: &Engine, name: &str) -> CharacterRow {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!("Expected char {name}")
    };
    c
}
fn unchanged(e: &Engine, previous: &BTreeMap<String, Array>) {
    assert_eq!(e.workspace_count(), previous.len());
    for (name, value) in previous {
        assert!(value.same_snapshot(&e.array(name).unwrap()), "{name}");
    }
}

#[test]
fn character_storage_preserves_utf16_units_aliases_and_exact_edits() {
    let mut e = Engine::default();
    e.execute("s='can''t';alias=s;u=char([0 10 946 55357 56832 55296]);v=u;part=u(3:5);s(1)='C';kind=class(u);n=numel(u);[r,c]=size(u);").unwrap();
    assert_eq!(
        chars(&e, "s").to_utf8(&AtomicBool::new(false)).unwrap(),
        "Can't"
    );
    assert_eq!(
        chars(&e, "alias").to_utf8(&AtomicBool::new(false)).unwrap(),
        "can't"
    );
    assert_eq!(chars(&e, "u").units(), &[0, 10, 946, 55357, 56832, 55296]);
    assert!(e.array("u").unwrap().same_snapshot(&e.array("v").unwrap()));
    assert_eq!(chars(&e, "part").units(), &[946, 55357, 56832]);
    assert!(chars(&e, "u").to_utf8(&AtomicBool::new(false)).is_err());
    assert!(chars(&e, "u").preview(128).contains("\\uD800"));
    assert_eq!(chars(&e, "kind").units(), &[99, 104, 97, 114]);
    assert_eq!(e.variables["n"].data(), &[6.]);
    assert_eq!(e.variables["r"].data(), &[1.]);
    assert_eq!(e.variables["c"].data(), &[6.]);
    let before = e.workspace();
    for source in [
        "s=\"unsupported string\";",
        "s=char([0 65536]);",
        "s=char(1.5);",
        "s(1)='two';",
        "s=[s; 'different width'];",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        unchanged(&e, &before);
    }
    e.execute("empty='';converted=char(\"explicit\");joined=['ab' 'cd'];")
        .unwrap();
    assert_eq!(
        (chars(&e, "empty").rows(), chars(&e, "empty").cols()),
        (0, 0)
    );
    assert_eq!(
        chars(&e, "converted").units(),
        &[101, 120, 112, 108, 105, 99, 105, 116]
    );
    assert_eq!(chars(&e, "joined").units(), &[97, 98, 99, 100]);
}

#[test]
fn cell_column_major_braces_parentheses_lists_and_shared_numeric_payloads() {
    let mut e = Engine::default();
    e.execute("A=[1 2;3 4];Z=complex([1 -0],[2 3]);L=logical([0 1]);C={A,'first';Z,L};copy=C;P=C([2 1],:);[x,y]=C{1:2};D={C{:,2}};N=[D{2}];").unwrap();
    let c = cell(&e, "C");
    assert_eq!((c.rows(), c.cols()), (2, 2));
    for (index, name) in [(0, "A"), (1, "Z"), (3, "L")] {
        assert!(
            c.get_linear(index)
                .unwrap()
                .same_snapshot(&e.array(name).unwrap())
        );
    }
    assert!(
        e.array("C")
            .unwrap()
            .same_snapshot(&e.array("copy").unwrap())
    );
    assert!(e.array("x").unwrap().same_snapshot(&e.array("A").unwrap()));
    assert!(e.array("y").unwrap().same_snapshot(&e.array("Z").unwrap()));
    let p = cell(&e, "P");
    for (got, original) in [1, 0, 3, 2].into_iter().enumerate() {
        assert!(
            p.get_linear(got)
                .unwrap()
                .same_snapshot(c.get_linear(original).unwrap())
        );
    }
    assert_eq!((cell(&e, "D").rows(), cell(&e, "D").cols()), (1, 2));
    assert_eq!(e.logical_variables["N"].data(), &[0, 1]);
    e.execute("C{1}=99;C(2)={42};").unwrap();
    assert_eq!(
        cell(&e, "C")
            .get_linear(0)
            .unwrap()
            .as_numeric()
            .unwrap()
            .data(),
        &[99.]
    );
    assert_eq!(
        cell(&e, "C")
            .get_linear(1)
            .unwrap()
            .as_numeric()
            .unwrap()
            .data(),
        &[42.]
    );
    assert!(cell(&e, "copy").same_storage(&c));
    assert!(
        c.get_linear(0)
            .unwrap()
            .same_snapshot(&e.array("A").unwrap())
    );
}

#[test]
fn cell_construction_empty_shapes_and_storage_limits_are_explicit() {
    let mut e = Engine::default();
    e.execute("C=cell(2,3);a=cell(0,3);b=cell([4 0]);c=cell(0);literal={};flag=iscell(C);other=ischar(C);kind=class(C);").unwrap();
    let c = cell(&e, "C");
    assert_eq!((c.rows(), c.cols()), (2, 3));
    for i in 0..6 {
        let a = c.get_linear(i).unwrap();
        assert!(matches!(a.as_ref(), Array::Numeric(_)));
        assert_eq!((a.rows(), a.cols()), (0, 0));
    }
    assert!(Arc::ptr_eq(
        c.get_linear(0).unwrap(),
        c.get_linear(5).unwrap()
    ));
    for (name, shape) in [
        ("a", (0, 3)),
        ("b", (4, 0)),
        ("c", (0, 0)),
        ("literal", (0, 0)),
    ] {
        let a = cell(&e, name);
        assert_eq!((a.rows(), a.cols()), shape);
    }
    assert_eq!(e.logical_variables["flag"].data(), &[1]);
    assert_eq!(e.logical_variables["other"].data(), &[0]);
    let before = e.workspace();
    for source in [
        "C=cell(257);",
        "C=cell(65537,0);",
        "C=cell([2 3 4]);",
        "C=cell(true);",
        "C={1,2;3};",
        "C{1:2}=0;",
        "C(1)=3;",
        "[C,kind]=C{1:3};",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        unchanged(&e, &before);
    }
    let cancel = AtomicBool::new(false);
    let mut nested: Array = Matrix::scalar(1.).into();
    for _ in 0..16 {
        nested = CellArray::from_shared(1, 1, vec![Arc::new(nested)], &cancel)
            .unwrap()
            .into();
    }
    assert!(
        CellArray::from_shared(1, 1, vec![Arc::new(nested)], &cancel)
            .unwrap_err()
            .contains("16 nesting")
    );
    let child: Array = CellArray::empty(1, 32768, &cancel).unwrap().into();
    assert!(
        CellArray::from_shared(
            1,
            2,
            vec![Arc::new(child.clone()), Arc::new(child)],
            &cancel
        )
        .is_err()
    );
    assert!(CharacterRow::from_text(&"x".repeat(65537), &cancel).is_err());
}

#[test]
fn container_function_frames_shadowing_clear_and_failures_preserve_callers() {
    let mut e = Engine::default();
    e.execute("A=[1 2];C={A,'label'};s='outside';[D,t]=update(C,s);Q={2,3};sum=add(Q{:});function [y,t]=update(x,s);x{2}='inside';y=x;t=s;clear s;end\nfunction y=add(a,b);y=a+b;end").unwrap();
    assert_eq!(chars(&e, "t").units(), chars(&e, "s").units());
    assert!(
        cell(&e, "D")
            .get_linear(0)
            .unwrap()
            .same_snapshot(&e.array("A").unwrap())
    );
    assert_eq!(e.variables["sum"].data(), &[5.]);
    let before = e.workspace();
    assert!(
        e.execute("C=bad(C);function y=bad(x);clear;x={9};y=missing;end")
            .is_err()
    );
    unchanged(&e, &before);
    assert!(
        e.execute("C={1}+effect();function y=effect;disp('MUST_NOT_RUN');y=1;end")
            .is_err()
    );
    assert!(e.failed_output.output.is_empty());
    unchanged(&e, &before);
    e.execute("variables=C;all=s;clear variables all;cell=C;K=cell(1);clear cell;U=cell(1,1);clear C D s t Q;").unwrap();
    assert!(e.array("variables").is_none() && e.array("all").is_none());
    assert_eq!(cell(&e, "K").len(), 1);
    assert_eq!(cell(&e, "U").len(), 1);
    e.rename_array("U", "renamed").unwrap();
    assert!(e.array("U").is_none());
    let retained = e.array("renamed").unwrap();
    // Legacy public-map mutation must not make lookup disagree with the full
    // snapshot or evaluator when a name already belongs to a private container.
    e.variables.insert("renamed".into(), Matrix::scalar(123.));
    assert!(retained.same_snapshot(&e.workspace()["renamed"]));
    assert!(retained.same_snapshot(&e.array("renamed").unwrap()));
    e.execute("observed=renamed;").unwrap();
    assert!(retained.same_snapshot(&e.array("observed").unwrap()));
    e.execute("renamed=9;").unwrap();
    assert_eq!(
        e.array("renamed").unwrap().as_numeric().unwrap().data(),
        &[9.]
    );
    e.execute("clear;").unwrap();
    assert_eq!(e.workspace_count(), 0);
}

#[test]
fn real_only_operations_reject_empty_containers_before_shortcuts_or_publication() {
    let mut e = Engine::default();
    e.execute("C=cell(0,3);s='';R=17;").unwrap();
    let before = e.workspace();
    for source in [
        "R=double(C);",
        "R=real(C);",
        "R=logical(C);",
        "R=C+1;",
        "R=sum(C);",
        "for x=C;R=0;end",
        "plot(C);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        unchanged(&e, &before);
    }
    let token = e.cancellation();
    token.store(true, Ordering::Relaxed);
    assert!(e.execute("C{1}=4;").unwrap_err().contains("cancel"));
    unchanged(&e, &before);
    token.store(false, Ordering::Relaxed);
    e.execute("C=cell(1,1);C{1}=4;").unwrap();
    assert_eq!(
        cell(&e, "C")
            .get_linear(0)
            .unwrap()
            .as_numeric()
            .unwrap()
            .data(),
        &[4.]
    );
}

#[test]
fn unsupported_container_file_saves_are_atomic_and_selected_numeric_saves_work() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("A=[1 2];C={A};s='title';").unwrap();
    let before = e.workspace();
    for (name, save) in [("plain.csv", 2), ("strict.csv", 3)] {
        let path = dir.path().join(name);
        std::fs::write(&path, b"retained destination").unwrap();
        let result = match save {
            2 => files::write_array_csv(&path, &before["C"], &cancel),
            _ => files::write_matrix_text(&path, &before["s"], ',', &cancel),
        };
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"retained destination");
    }
    let v73 = dir.path().join("v73.mat");
    files::save_array_workspace_v73(&v73, &before, &cancel).unwrap();
    let catalog = files::inspect_workspace(&v73, &cancel).unwrap();
    let restored =
        files::load_selected_arrays(&catalog, &before.keys().cloned().collect(), &cancel).unwrap();
    assert!(matches!(restored["C"], matrust::Array::Cell(_)));
    let matrust::Array::Character(text) = &restored["s"] else {
        panic!("Character lost")
    };
    assert_eq!(text.units(), &[116, 105, 116, 108, 101]);
    unchanged(&e, &before);
    let path = dir.path().join("selected.mat");
    files::save_array_workspace(
        &path,
        &BTreeMap::from([("A".into(), before["A"].clone())]),
        &cancel,
    )
    .unwrap();
    let catalog = files::inspect_workspace(&path, &cancel).unwrap();
    assert_eq!(catalog.variables().count(), 1);
    assert_eq!(catalog.variables().next().unwrap().name, "A");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        4,
        "No abandoned temporary files"
    );
}

#[test]
fn container_rebase_preserves_numeric_shapes_repetition_and_two_row_sorting() {
    let mut e = Engine::default();
    e.execute("A=[3 1 2;6 4 5];B=reshape(A,3,[]);Z=zeros(size(A));O=ones([2 3]);L=logical(A);LC=repmat(L,1,1);X=complex(A,A);XC=repmat(X,[1 1]);R=repmat(A,[1 2]);[S,I]=sortrows([2 9;1 8]);C=cell(0,3);s='';sentinel=17;").unwrap();
    assert_eq!((e.variables["B"].rows(), e.variables["B"].cols()), (3, 2));
    assert_eq!(e.variables["B"].data(), e.variables["A"].data());
    assert_eq!(e.variables["Z"].data(), &[0.; 6]);
    assert_eq!(e.variables["O"].data(), &[1.; 6]);
    for (source, alias) in [("L", "LC"), ("X", "XC")] {
        assert!(
            e.array(source)
                .unwrap()
                .same_snapshot(&e.array(alias).unwrap())
        );
    }
    assert_eq!(
        e.variables["R"].data(),
        &[3., 6., 1., 4., 2., 5., 3., 6., 1., 4., 2., 5.]
    );
    assert_eq!(e.variables["S"].data(), &[1., 2., 8., 9.]);
    assert_eq!(e.variables["I"].data(), &[2., 1.]);
    let before = e.workspace();
    for source in ["sentinel=zeros(C);", "sentinel=ones(s);"] {
        assert!(e.execute(source).is_err(), "{source}");
        unchanged(&e, &before);
    }
    let cancel = AtomicBool::new(false);
    for name in ["C", "s"] {
        let input = e.array(name).unwrap();
        for (r, c) in [(1, 1), (0, 2), (2, 0)] {
            assert!(input.repeated_shape(r, c).is_err());
            assert!(input.repeated(r, c, &cancel).is_err());
            assert!(input.same_snapshot(&e.array(name).unwrap()));
        }
    }
}

#[test]
fn container_argument_lists_expand_in_builtin_multioutputs_and_file_commands() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("A=[2 NaN 1;2 0 3];options={'descend','MissingPlacement','last'};[B,I]=sort(A,1,options{:});one=sort(A,1,options{:});filename='selected.mat';name='A';version='-v7';save_options={name,version};save(filename,save_options{:});").unwrap();
    for (a, b) in e.variables["B"]
        .data()
        .iter()
        .zip(e.variables["one"].data())
    {
        assert_eq!(a.to_bits(), b.to_bits());
    }
    assert_eq!(e.variables["I"].data(), &[1., 2., 2., 1., 2., 1.]);
    let before = e.workspace();
    assert!(
        e.execute("[B,I]=sin(effect());function y=effect;disp('MUST_NOT_RUN');y=1;end")
            .is_err()
    );
    assert!(e.failed_output.output.is_empty());
    unchanged(&e, &before);
    let file = dir.path().join("selected.mat");
    let cancel = AtomicBool::new(false);
    let catalog = files::inspect_workspace(&file, &cancel).unwrap();
    assert_eq!(catalog.variables().count(), 1);
    assert_eq!(catalog.variables().next().unwrap().name, "A");
    e.execute("clear A;load(filename,name);csvname='selected.csv';writematrix(A,csvname);copy=readmatrix(csvname);").unwrap();
    for (a, b) in e.variables["A"]
        .data()
        .iter()
        .zip(e.variables["copy"].data())
    {
        assert!(a.is_nan() && b.is_nan() || a.to_bits() == b.to_bits());
    }
    assert_eq!(
        chars(&e, "filename").to_utf8(&cancel).unwrap(),
        "selected.mat"
    );
}
