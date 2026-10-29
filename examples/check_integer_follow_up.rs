//! Local release proof, separate from the orchestrator-owned desktop acceptance gate.
//! TMPDIR="$PWD/tmp" cargo run --release --example check_integer_follow_up -- 1
//! Repeat with 4 workers; both runs compare against the same expected values.
use matrust::{Array, Engine, NumericClass, Scalar, TypedArray};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

fn corpus(fixture: &str, second: bool) -> usize {
    let mut passed = 0;
    for block in fixture.split("@@CASE ").skip(1) {
        let (header, body) = block.split_once('\n').unwrap();
        let (id, statement) = header.split_once('|').unwrap();
        let id: usize = id.parse().unwrap();
        let body = body.split_once("@@END\n").unwrap().0;
        let mut e = Engine::default();
        if if second {
            (31..=33).contains(&id)
        } else {
            id >= 232
        } {
            e.execute("format long").unwrap();
        }
        let result = e.execute(statement);
        if let Some(error) = body.strip_prefix("ERR ") {
            let got = result.unwrap_err();
            let got = got
                .split_once(": ")
                .map_or(got.as_str(), |(_, error)| error);
            assert_eq!(got, error.trim_end_matches('\n'), "case {id}: {statement}");
        } else {
            let (expected, class) = body.split_once("@@CLASS ").unwrap();
            let result = result.unwrap_or_else(|error| panic!("case {id}: {statement}: {error}"));
            let text: String = result
                .output
                .iter()
                .map(|line| format!("{line}\n"))
                .collect();
            assert_eq!(text, expected, "case {id}: {statement}");
            let (class, shape) = class.trim().split_once(' ').unwrap();
            let shape: Vec<usize> = shape
                .trim_matches(['[', ']'])
                .split(' ')
                .map(|n| n.parse().unwrap())
                .collect();
            let a = e.array("x").unwrap();
            assert_eq!(
                (a.class(), a.rows(), a.cols()),
                (class, shape[0], shape[1]),
                "case {id}"
            );
        }
        passed += 1;
    }
    passed
}
fn state(values: BTreeMap<String, Array>) -> matrust::session::Session {
    matrust::session::Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(values),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: None,
        layout: Default::default(),
        version: 1,
        documents: vec![matrust::document::Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Default::default(),
        model_path: None,
        model_baseline: None,
    }
}
fn compare(a: &Array, b: &Array) {
    let (Array::Typed(a), Array::Typed(b)) = (a, b) else {
        panic!("typed")
    };
    assert_eq!(
        (a.class(), a.rows(), a.cols()),
        (b.class(), b.rows(), b.cols())
    );
    assert_eq!(a.to_le_bytes(), b.to_le_bytes());
}
fn main() {
    let workers = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let passed = corpus(
        include_str!("../tests/fixtures/integers/expected.txt"),
        false,
    ) + corpus(
        include_str!("../tests/fixtures/integers/expected-2.txt"),
        true,
    );
    assert_eq!(passed, 279);
    println!("workers={workers}: {passed}/279 reference cases passed");
    let mut e = Engine::default();
    e.execute("x=zeros(1,1e6,'int8'); for k=1:1e5, x(k)=int8(k); end;")
        .unwrap();
    let Some(Array::Typed(x)) = e.array("x") else {
        panic!("int8")
    };
    for i in 0..x.len() {
        assert_eq!(
            x.int(i),
            if i < 100000 {
                (i + 1).min(127) as i128
            } else {
                0
            }
        );
    }
    e.execute("x(1)=-7;").unwrap();
    assert_eq!(x.int(0), 1);
    for operands in [
        "int8([1 2]),int8(0)",
        "int8([]),int8(0)",
        "int8(0),int8([])",
    ] {
        assert!(
            e.execute(&format!("idivide({operands},'bogus')"))
                .unwrap_err()
                .contains("unknown rounding option")
        );
    }
    for range in [
        "int8(1):single(3)",
        "single(1):uint64(3)",
        "int16(1):single(1):int16(3)",
    ] {
        assert!(
            e.execute(&format!("x={range};"))
                .unwrap_err()
                .contains("Colon operands must be all the same type")
        );
    }
    e.execute("a=[int64(9007199254740993) int64(9007199254740995)]; b=a; a([2 1 2])=a([1 2 2]); a(end+2)=int64(9007199254740997); a([2 2])=[];").unwrap();
    let Some(Array::Typed(a)) = e.array("a") else {
        panic!("int64")
    };
    assert_eq!(
        (0..a.len()).map(|i| a.int(i)).collect::<Vec<_>>(),
        [9007199254740995, 0, 9007199254740997]
    );
    e.execute("n(:,2)=[uint64(9007199254740993);uint64(9007199254740995)]; n(n>0)=uint64(18446744073709551615);").unwrap();
    let Some(Array::Typed(n)) = e.array("n") else {
        panic!("uint64")
    };
    assert_eq!((n.rows(), n.cols()), (2, 2));
    assert_eq!(
        (0..n.len()).map(|i| n.int(i)).collect::<Vec<_>>(),
        [0, 0, u64::MAX as i128, u64::MAX as i128]
    );
    println!(
        "native indexed loop contents, shared snapshots, growth/deletion, idivide and mixed colon checks passed"
    );
    e.execute("p=zeros(1,1000000,'int8'); q=ones(1,1000000,'int8'); p(:)=q;")
        .unwrap();
    let rhs = e.array("q").unwrap();
    assert!(rhs.same_snapshot(&e.array("p").unwrap()));
    e.execute("p(1:3)='abc'; p(4:6)=logical([1 0 1]);").unwrap();
    let Some(Array::Typed(p)) = e.array("p") else {
        panic!("int8")
    };
    assert_eq!(
        (0..6).map(|i| p.int(i)).collect::<Vec<_>>(),
        [97, 98, 99, 1, 0, 1]
    );
    assert_eq!(rhs.scalar(0, 0).unwrap(), Scalar::Integer(1));
    println!(
        "complete replacement shares RHS; logical/character RHS conversion preserves class and aliases"
    );
    let cancel = AtomicBool::new(false);
    let mut values = BTreeMap::new();
    for class in NumericClass::INTEGERS {
        let a: Array = TypedArray::from_ints(
            class,
            2,
            2,
            [class.min(), class.max(), class.max() - 1, 0].into_iter(),
        )
        .unwrap()
        .into();
        values.insert(class.name().to_owned(), a);
        values.insert(
            format!("empty_{}", class.name()),
            TypedArray::from_ints(class, 0, 7, std::iter::empty())
                .unwrap()
                .into(),
        );
    }
    let bits = [
        0x7fc00123u32,
        0x80000000,
        1,
        0x7f800000,
        0xff800000,
        0x3dcccccd,
    ];
    let s = TypedArray::from_le_bytes(
        NumericClass::Single,
        2,
        3,
        &bits
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    values.insert("single".into(), s.clone().into());
    values.insert("single_alias".into(), s.reshaped(3, 2).unwrap().into());
    values.insert(
        "empty_single".into(),
        TypedArray::from_reals(NumericClass::Single, 3, 0, std::iter::empty())
            .unwrap()
            .into(),
    );
    values.insert(
        "large".into(),
        TypedArray::from_ints(
            NumericClass::Uint64,
            1024,
            1025,
            (0..1024 * 1025).map(|i| u64::MAX as i128 - i),
        )
        .unwrap()
        .into(),
    );
    for class in [NumericClass::Int64, NumericClass::Uint64] {
        let a = values[class.name()].clone();
        let scalar = Scalar::parse_typed_cell("9007199254740993", class).unwrap();
        assert_eq!(scalar, Scalar::Integer(9007199254740993));
        assert_eq!(
            a.with_scalar(0, 0, scalar, &cancel)
                .unwrap()
                .scalar(0, 0)
                .unwrap(),
            scalar
        );
        e.merge_workspace([("edit".into(), a)].into()).unwrap();
        let target =
            matrust::variable_edit::Target::new("edit".into(), e.array("edit").unwrap(), vec![])
                .unwrap();
        let text = format!(
            "{}\t9007199254740993\n9007199254740995\t{}",
            class.min(),
            class.max()
        );
        let rect = matrust::variable_edit::Rect::new(0, 0, 2, 2).unwrap();
        matrust::variable_edit::paste_tsv(&mut e, &target, rect, &text).unwrap();
        let target =
            matrust::variable_edit::Target::new("edit".into(), e.array("edit").unwrap(), vec![])
                .unwrap();
        assert_eq!(
            matrust::variable_edit::copy_tsv(&e, &target, rect).unwrap(),
            text
        );
        assert_eq!(
            e.array("edit").unwrap().scalar(0, 1).unwrap(),
            Scalar::Integer(9007199254740993)
        );
    }
    println!("exact 64-bit scalar edits and desktop clipboard path passed");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = matrust::session::SessionStore::new(&path);
    store.save(state(values.clone())).unwrap();
    let manifest = matrust::session::Session::load(&path)
        .unwrap()
        .workspace_manifest
        .unwrap();
    assert_eq!(manifest.version, 7);
    let recovered = matrust::session::workspace::restore(&path, &manifest, &cancel).unwrap();
    for (name, a) in &values {
        compare(a, &recovered[name]);
    }
    let (Array::Typed(a), Array::Typed(b)) = (&recovered["single"], &recovered["single_alias"])
    else {
        panic!("typed")
    };
    assert!(a.same_storage(b));
    store.save(state(values.clone())).unwrap();
    assert_eq!(
        manifest.entries,
        matrust::session::Session::load(&path)
            .unwrap()
            .workspace_manifest
            .unwrap()
            .entries
    );
    let mut invalid = manifest.clone();
    invalid
        .entries
        .values_mut()
        .find(|r| r.numeric_class == Some(NumericClass::Uint64))
        .unwrap()
        .logical = true;
    assert!(matrust::session::workspace::restore(&path, &invalid, &cancel).is_err());
    println!(
        "version 7 session exact bytes, native classes, empty shapes, aliases, weak-cache reuse and invalid flags passed"
    );
    let path = dir.path().join("native.mat");
    matrust::files::save_array_workspace_v73(&path, &values, &cancel).unwrap();
    let catalog = matrust::files::inspect_workspace(&path, &cancel).unwrap();
    let names: BTreeSet<_> = values.keys().cloned().collect();
    let loaded = matrust::files::load_selected_arrays(&catalog, &names, &cancel).unwrap();
    for (name, a) in &values {
        compare(a, &loaded[name]);
    }
    println!(
        "MAT v7.3 exact class/shape/bytes round trip, including >8 MiB uint64 and single NaN payloads, passed"
    );
}
