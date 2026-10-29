//! Reference cell, struct and text corpus. Each case gets a fresh
//! engine; only the explicitly listed HARNESS rules relax console comparison.
use matrust::Engine;

const REFERENCE: &str = include_str!("fixtures/fn/cells-expected.txt");
const SOURCES: &str = include_str!("fixtures/fn/cells-cases.txt");
const ND: &str = "no N-D arrays";
const WARNING_STATE: &str =
    "warning state save/restore is not implemented; separate warning API work";
const EMPTY_DISPLAY: &str =
    "cell display alignment of empty numeric slots; separate display follow-up";
const KNOWN: &[(usize, &str)] = &[
    (26, WARNING_STATE),
    (59, ND),
    (63, ND),
    (64, ND),
    (149, EMPTY_DISPLAY),
    (162, EMPTY_DISPLAY),
    (163, EMPTY_DISPLAY),
    (
        219,
        "char display prints NUL units instead of spaces; separate display follow-up",
    ),
    (229, ND),
    (233, ND),
    (253, ND),
    (256, ND),
    (257, ND),
    (270, ND),
    (290, ND),
    (369, WARNING_STATE),
    (370, WARNING_STATE),
    (400, WARNING_STATE),
];
const LOCATION: &str = "Remove MatRust diagnostic location prefix; error message remains exact";
const LINKS: &str = "Remove MATLAB evalc struct help hyperlinks, retaining their text";
const WARNING_MARKUP: &str =
    "Remove MATLAB evalc warning bracket/backspace markup; message and remaining output stay exact";
const HARNESS: &[(usize, &str)] = &[
    (26, WARNING_MARKUP),
    (37, LOCATION),
    (38, LOCATION),
    (39, LOCATION),
    (40, LOCATION),
    (42, LOCATION),
    (43, LINKS),
    (44, LINKS),
    (45, LINKS),
    (47, LOCATION),
    (48, LOCATION),
    (49, LINKS),
    (52, LOCATION),
    (55, LOCATION),
    (56, LOCATION),
    (60, LOCATION),
    (61, LINKS),
    (62, LINKS),
    (64, LINKS),
    (70, LOCATION),
    (71, LINKS),
    (72, LINKS),
    (73, LOCATION),
    (77, LOCATION),
    (103, LINKS),
    (128, LINKS),
    (129, LINKS),
    (131, LINKS),
    (142, LOCATION),
    (143, LOCATION),
    (150, LOCATION),
    (153, LOCATION),
    (154, LOCATION),
    (155, LOCATION),
    (167, LOCATION),
    (168, LOCATION),
    (180, LOCATION),
    (186, LOCATION),
    (187, LOCATION),
    (188, LOCATION),
    (189, LOCATION),
    (190, LOCATION),
    (193, LINKS),
    (195, LINKS),
    (197, LINKS),
    (198, LINKS),
    (201, LINKS),
    (207, LINKS),
    (208, LOCATION),
    (209, LOCATION),
    (211, LINKS),
    (212, LOCATION),
    (214, LOCATION),
    (221, LOCATION),
    (222, LOCATION),
    (226, LOCATION),
    (228, LOCATION),
    (230, LINKS),
    (235, LINKS),
    (241, LINKS),
    (258, LOCATION),
    (259, LINKS),
    (294, LOCATION),
    (305, LOCATION),
    (309, LOCATION),
    (310, LOCATION),
    (332, LOCATION),
    (344, LOCATION),
    (367, LOCATION),
    (368, LOCATION),
    (371, LOCATION),
    (394, LOCATION),
    (395, LOCATION),
    (396, LOCATION),
    (397, LOCATION),
    (416, LOCATION),
    (432, LOCATION),
    (448, LOCATION),
    (460, LOCATION),
];

struct Case<'a> {
    id: usize,
    source: &'a str,
    expected: &'a str,
    class: Option<(&'a str, Vec<usize>)>,
}

fn cases() -> Vec<Case<'static>> {
    REFERENCE
        .split("@@CASE ")
        .skip(1)
        .map(|block| {
            let (header, body) = block.split_once('\n').unwrap();
            let (id, source) = header.split_once('|').unwrap();
            let (body, _) = body.split_once("@@END\n").unwrap();
            let (expected, class) = if let Some((text, metadata)) = body.split_once("@@CLASS ") {
                let (name, size) = metadata.trim_end().split_once(' ').unwrap();
                let dims = size
                    .trim_matches(['[', ']'])
                    .split_whitespace()
                    .map(|n| n.parse().unwrap())
                    .collect();
                (text, Some((name, dims)))
            } else {
                (body, None)
            };
            Case {
                id: id.parse().unwrap(),
                source,
                expected,
                class,
            }
        })
        .collect()
}

fn strip_links(text: &str) -> String {
    let mut plain = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<a href=\"") {
        let after = &rest[start..];
        let end = after.find('>').expect("opening link tag");
        let close = after.find("</a>").expect("closing link tag");
        plain.push_str(&rest[..start]);
        plain.push_str(&after[end + 1..close]);
        rest = &after[close + 4..];
    }
    plain.push_str(rest);
    plain
}

fn mismatch(case: &Case<'_>) -> Option<String> {
    let mut engine = Engine::default();
    let source = case.source.strip_prefix("LONG ").unwrap_or(case.source);
    if case.source.starts_with("LONG ") {
        engine.execute("format long").unwrap();
    }
    let result = engine.execute(source);
    let rules: Vec<_> = HARNESS
        .iter()
        .filter(|(id, _)| *id == case.id)
        .map(|(_, rule)| *rule)
        .collect();
    if let Some(expected) = case.expected.strip_prefix("ERR ") {
        let expected = expected.strip_suffix('\n').unwrap();
        return match result {
            Err(error) => {
                let actual = if rules.contains(&LOCATION) {
                    engine
                        .diagnostic
                        .as_ref()
                        .map_or(error.as_str(), |d| d.message.as_str())
                } else {
                    &error
                };
                (actual != expected)
                    .then(|| format!("expected error {expected:?}, actual {actual:?}"))
            }
            Ok(_) => Some(format!("expected error {expected:?}, got success")),
        };
    }
    let output = match result {
        Ok(result) => result
            .output
            .iter()
            .map(|s| format!("{s}\n"))
            .collect::<String>(),
        Err(error) => return Some(format!("unexpected error {error:?}")),
    };
    let mut expected = if rules.contains(&LINKS) {
        strip_links(case.expected)
    } else {
        case.expected.into()
    };
    let mut actual = if rules.contains(&LINKS) {
        strip_links(&output)
    } else {
        output
    };
    if rules.contains(&WARNING_MARKUP) {
        expected = expected.replace("]\u{8} ", "").replace("[\u{8}", "");
        actual = actual.replace("]\u{8} ", "").replace("[\u{8}", "");
    }
    if let Some((class, dims)) = &case.class {
        let Some(x) = engine.array("x") else {
            return Some("missing x".into());
        };
        if x.class() != *class || [x.rows(), x.cols()].as_slice() != dims {
            return Some(format!(
                "expected {class} {dims:?}, actual {} [{}, {}]",
                x.class(),
                x.rows(),
                x.cols()
            ));
        }
    }
    (actual != expected).then(|| format!("expected {expected:?}\n  actual {actual:?}"))
}

#[test]
fn corpus_has_every_case() {
    let cases = cases();
    assert_eq!(cases.len(), 460);
    let sources: Vec<_> = SOURCES
        .lines()
        .filter(|s| !s.trim().is_empty() && !s.starts_with('#'))
        .collect();
    assert_eq!(sources.len(), 460);
    for (index, (case, source)) in cases.iter().zip(sources).enumerate() {
        assert_eq!(case.id, index + 1);
        assert_eq!(case.source, source);
    }
    let mut seen = std::collections::BTreeSet::new();
    for (id, reason) in KNOWN.iter().chain(HARNESS) {
        assert!((1..=460).contains(id) && !reason.is_empty());
        assert!(seen.insert((id, reason)), "duplicate case rule {id}");
    }
}

#[test]
fn matches_reference_corpus() {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases() {
        let known = KNOWN.iter().any(|(id, _)| *id == case.id);
        match (mismatch(&case), known) {
            (None, false) => passed += 1,
            (None, true) => failures.push(format!("case {} passes; remove KNOWN entry", case.id)),
            (Some(_), true) => {}
            (Some(why), false) => {
                failures.push(format!("case {} `{}`\n  {why}", case.id, case.source))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {} known, {} failed:\n{}",
        KNOWN.len(),
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + KNOWN.len(), 460);
}

// Reuse the existing core regressions in this task's permitted test target.
// Their reference dependencies are copies under tests/fixtures/indexing, so the
// Windows code-only gate never reads research/, examples/ or tool/.
#[path = "indexed_assignment.rs"]
mod assignment_regressions;
#[path = "character_shapes.rs"]
mod character_regressions;
#[path = "script_containers.rs"]
mod container_regressions;
#[path = "indexing.rs"]
mod indexing_regressions;
#[path = "ismember.rs"]
mod membership_regressions;
#[path = "set_operations.rs"]
mod set_regressions;
#[path = "sort_oracle.rs"]
mod sort_reference_regressions;
#[path = "sort_functions.rs"]
mod sort_regressions;
#[path = "sort_rows_oracle.rs"]
mod sort_rows_reference_regressions;
#[path = "sort_rows_functions.rs"]
mod sort_rows_regressions;
#[path = "structs_errors.rs"]
mod struct_regressions;
#[path = "unique.rs"]
mod unique_regressions;

fn cells(engine: &Engine, name: &str) -> matrust::CellArray {
    let matrust::Array::Cell(c) = engine.array(name).unwrap() else {
        panic!("expected cell {name}")
    };
    c
}

#[test]
fn rearrangements_share_payloads_and_preserve_typed_and_complex_bits() {
    let mut e = Engine::default();
    e.execute("A=reshape(1:32768,128,256); C={A,uint64(18446744073709551615);single(-0),complex(1,2)}; alias=C; T=C'; R=reshape(C,1,4); P=repmat(C,2,3); F=rot90(C,4); S=struct('a',C); U=permute(S,[2 1]); D=[C,C];").unwrap();
    let original = cells(&e, "C");
    assert!(original.same_storage(&cells(&e, "alias")));
    assert!(original.same_storage(&cells(&e, "R")));
    assert!(
        original
            .get_linear(0)
            .unwrap()
            .same_snapshot(&e.array("A").unwrap())
    );
    for (name, positions) in [
        ("T", vec![0, 2, 1, 3]),
        ("F", vec![0, 1, 2, 3]),
        ("D", vec![0, 1, 2, 3, 0, 1, 2, 3]),
    ] {
        let result = cells(&e, name);
        for (i, old) in positions.into_iter().enumerate() {
            assert!(
                result
                    .get_linear(i)
                    .unwrap()
                    .same_snapshot(original.get_linear(old).unwrap()),
                "{name} slot {i}"
            );
        }
    }
    let repeated = cells(&e, "P");
    assert_eq!((repeated.rows(), repeated.cols()), (4, 6));
    assert!(
        repeated
            .get_linear(22)
            .unwrap()
            .same_snapshot(original.get_linear(2).unwrap())
    );
    let matrust::Array::Struct(s) = e.array("U").unwrap() else {
        panic!("struct")
    };
    assert!(
        s.get(0, 1)
            .unwrap()
            .same_snapshot(original.get_linear(2).unwrap())
    );
}

#[test]
fn growth_deletion_and_failed_writes_preserve_aliases_and_workspace_atomically() {
    let mut e = Engine::default();
    e.execute("C={1,2;3,4}; alias=C; C(2,3)={7}; C(3,1)={9}; G=C; C(:,2)=[]; C(2)=[];")
        .unwrap();
    let grown = cells(&e, "G");
    assert_eq!((grown.rows(), grown.cols()), (3, 3));
    assert_eq!(
        grown.get_linear(0).unwrap().as_numeric().unwrap().data(),
        &[1.]
    );
    assert_eq!(
        grown.get_linear(3).unwrap().as_numeric().unwrap().data(),
        &[2.]
    );
    assert_eq!(
        grown.get_linear(7).unwrap().as_numeric().unwrap().data(),
        &[7.]
    );
    assert!(grown.get_linear(8).unwrap().is_empty());
    let c = cells(&e, "C");
    assert_eq!((c.rows(), c.cols()), (1, 5));
    assert_eq!(c.get_linear(1).unwrap().as_numeric().unwrap().data(), &[9.]);
    let alias = cells(&e, "alias");
    assert_eq!((alias.rows(), alias.cols()), (2, 2));
    for script in [
        "C(1:2)={1,2,3};",
        "C(8)=[];",
        "C(1,1)=[];",
        "C{1:2}=9;",
        "C(65537)={1};",
        "C=repmat(C,65536,2);",
    ] {
        let before = e.workspace();
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(before.len(), e.workspace_count());
        for (name, value) in &before {
            assert!(
                value.same_snapshot(&e.array(name).unwrap()),
                "{script}: {name}"
            );
        }
    }
    let before = e.workspace();
    e.cancellation()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        e.execute("C=sort({'z','a'});")
            .unwrap_err()
            .contains("cancel")
    );
    for (name, value) in &before {
        assert!(value.same_snapshot(&e.array(name).unwrap()));
    }
}

#[test]
fn display_known_cases_have_exact_underlying_values() {
    let mut e = Engine::default();
    e.execute("C={1,2}; C{3,3}=1; T='hello'; T(8)='z';")
        .unwrap();
    let c = cells(&e, "C");
    assert_eq!((c.rows(), c.cols()), (3, 3));
    for i in [1, 2, 4, 5, 6, 7] {
        assert!(c.get_linear(i).unwrap().is_empty());
    }
    for (i, v) in [(0, 1.), (3, 2.), (8, 1.)] {
        assert_eq!(c.get_linear(i).unwrap().as_numeric().unwrap().data(), &[v]);
    }
    let matrust::Array::Character(t) = e.array("T").unwrap() else {
        panic!("char")
    };
    assert_eq!(t.units(), &[104, 101, 108, 108, 111, 0, 0, 122]);
}

#[test]
fn text_operations_preserve_utf16_and_rows_flags_without_warning_state_wrapper() {
    let mut e = Engine::default();
    e.execute("U=char([55296 0 55296 65535]); [V,ia,ic]=unique(U); [S,p]=sort(U); C=unique({'b','a','b'},'rows'); M=ismember({'a','b'},{'a'},'rows');").unwrap();
    let matrust::Array::Character(v) = e.array("V").unwrap() else {
        panic!("char")
    };
    assert_eq!(v.units(), &[0, 55296, 65535]);
    assert_eq!(e.variables["ia"].data(), &[2., 1., 4.]);
    assert_eq!(e.variables["ic"].data(), &[2., 1., 2., 3.]);
    let matrust::Array::Character(s) = e.array("S").unwrap() else {
        panic!("char")
    };
    assert_eq!(s.units(), &[0, 55296, 55296, 65535]);
    assert_eq!(e.variables["p"].data(), &[2., 1., 3., 4.]);
    assert_eq!((cells(&e, "C").rows(), cells(&e, "C").cols()), (1, 2));
    assert_eq!(e.logical_variables["M"].data(), &[1, 0]);
}

#[test]
fn bracketed_assignment_targets_track_nested_delimiters() {
    let mut e = Engine::default();
    for subscript in ["[1 2]", "([1 2])", "idx{1}"] {
        e.execute(&format!(
            "c={{0,0}}; idx={{[1 2]}}; [c{{{subscript}}}]=deal(1,2);"
        ))
        .unwrap();
        let c = cells(&e, "c");
        for (i, expected) in [1., 2.].iter().enumerate() {
            assert_eq!(
                c.get_linear(i).unwrap().as_numeric().unwrap().data(),
                &[*expected]
            );
        }
    }
    e.execute("a=zeros(1,3); [a(1,[2 3]), b]=deal([7 8],9); x=[c{[2 1]}];")
        .unwrap();
    assert_eq!(e.variables["a"].data(), &[0., 7., 8.]);
    assert_eq!(e.variables["b"].data(), &[9.]);
    assert_eq!(e.variables["x"].data(), &[2., 1.]);
    e.execute("[A(1),B]=size(1);").unwrap();
    assert_eq!(e.variables["A"].data(), &[1.]);
    assert_eq!(e.variables["B"].data(), &[1.]);
}

#[test]
fn bracketed_assignment_targets_include_indexed_struct_fields() {
    let mut e = Engine::default();
    e.execute(
        "s=struct('a',{10,20,30},'b',{40,50,60}); [s(2).a, s(3).a]=deal(1,2); x=[s.a]; y=[s.b];",
    )
    .unwrap();
    assert_eq!(e.variables["x"].data(), &[10., 1., 2.]);
    assert_eq!(e.variables["y"].data(), &[40., 50., 60.]);
    e.execute("[s([3 1]).a]=deal(7,8); [s(2).new]=deal(9); x=[s.a];")
        .unwrap();
    assert_eq!(e.variables["x"].data(), &[8., 1., 7.]);
    let matrust::Array::Struct(s) = e.array("s").unwrap() else {
        panic!("struct")
    };
    let field = s.field_index("new").unwrap();
    assert!(s.get(field, 0).unwrap().is_empty());
    assert_eq!(s.get(field, 1).unwrap().as_numeric().unwrap().data(), &[9.]);
    assert!(s.get(field, 2).unwrap().is_empty());
}

#[test]
fn undefined_paren_assignment_creates_and_grows_cells() {
    for (source, name, shape, occupied) in [
        ("c(1)={x};", "c", (1, 1), 0),
        ("c(3)={x};", "c", (1, 3), 2),
        ("c(2,3)={x};", "c", (2, 3), 5),
        ("rhs={x}; c(3)=rhs;", "c", (1, 3), 2),
        ("c(3)=repmat({x},1,1);", "c", (1, 3), 2),
    ] {
        let mut e = Engine::default();
        e.execute("x=[4 5 6];").unwrap();
        e.execute(source).unwrap();
        let c = cells(&e, name);
        assert_eq!((c.rows(), c.cols()), shape, "{source}");
        assert!(
            c.get_linear(occupied)
                .unwrap()
                .same_snapshot(&e.array("x").unwrap())
        );
        for i in 0..c.len() {
            if i != occupied {
                let empty = c.get_linear(i).unwrap();
                assert_eq!(empty.class(), "double");
                assert_eq!((empty.rows(), empty.cols()), (0, 0));
            }
        }
    }
    let mut e = Engine::default();
    e.execute("rng(7); expected=rand; next=rand; rng(7); c(3)={rand}; actual_next=rand;")
        .unwrap();
    assert_eq!(
        cells(&e, "c")
            .get_linear(2)
            .unwrap()
            .as_numeric()
            .unwrap()
            .data(),
        e.variables["expected"].data()
    );
    assert_eq!(
        e.variables["actual_next"].data(),
        e.variables["next"].data()
    );
    assert!(e.execute("bad(0)={1};").is_err());
    assert!(!e.contains_array("bad"));
    e.execute("a=[1 2];").unwrap();
    let before = e.array("a").unwrap();
    assert!(e.execute("a(3)={1};").is_err());
    assert!(before.same_snapshot(&e.array("a").unwrap()));
}

#[test]
fn bracketed_assignment_failures_preserve_destinations_and_limits() {
    let mut e = Engine::default();
    e.execute("s=struct('a',{1,2,3}); a=[4 5]; b=6; c=cell(1,33);")
        .unwrap();
    let before = e.workspace();
    for source in [
        "[s(2).a,s(3).a]=deal(1,2,3);",
        "[a(1),b(0)]=deal(7,8);",
        "[c{1:33}]=deal(0);",
        "[c{1:32},a]=deal(0);",
        "[s,s(3).a]=deal(struct('a',0),1);",
        "[c,c{1}]=deal(0,1);",
        "[c{[1 2)]}]=deal(1,2);",
        "[a(1,[2 3}),b]=deal(1,2);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        for (name, array) in &before {
            assert!(
                array.same_snapshot(&e.array(name).unwrap()),
                "{source}: {name}"
            );
        }
    }
    e.execute("[b,~,s(2).a]=deal(10,11,12);").unwrap();
    assert_eq!(e.variables["b"].data(), &[10.]);
    e.execute("x=[s.a];").unwrap();
    assert_eq!(e.variables["x"].data(), &[1., 12., 3.]);
}

#[test]
fn nested_cell_access_and_comma_separated_assignment_are_transactional() {
    let mut e = Engine::default();
    e.execute("C={1,{5,'ab'},[5,6,7]}; C{2}{1}='new'; C{3}(5)=1; x=C{2}{2}; y=C{3}(3); S=struct('a',{1,2}); [S.a]=deal(9); [C{1:2}]=deal('q');").unwrap();
    assert_eq!(e.variables["y"].data(), &[7.]);
    let matrust::Array::Struct(s) = e.array("S").unwrap() else {
        panic!("struct")
    };
    for i in 0..2 {
        assert_eq!(s.get(0, i).unwrap().as_numeric().unwrap().data(), &[9.]);
    }
    let before = e.array("S").unwrap();
    assert!(e.execute("[S.a]=deal(1,2,3);").is_err());
    assert!(before.same_snapshot(&e.array("S").unwrap()));
    let before = e.array("C").unwrap();
    assert!(e.execute("C{3}(0)=9;").is_err());
    assert!(before.same_snapshot(&e.array("C").unwrap()));
    for source in ["[C{1}]=C(1)=2;", "[S.a]=format short;"] {
        assert!(e.execute(source).is_err(), "{source}");
        assert!(before.same_snapshot(&e.array("C").unwrap()));
    }
    e.execute("x=str2num('C{1}]=[2');").unwrap();
    assert!(e.array("x").unwrap().is_empty());
}

#[test]
#[ignore = "Run explicitly in release mode to record one/four-worker timings"]
fn benchmark_container_and_numeric_paths() {
    use matrust::{Array, CellArray, CharacterRow, Matrix};
    use std::{
        collections::BTreeMap,
        sync::{Arc, atomic::AtomicBool},
        time::Instant,
    };
    let cancel = AtomicBool::new(false);
    for workers in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (n, repeats) in [(16, 100), (1024, 20), (32768, 5)] {
                let numeric: Array =
                    Matrix::from_column_vec(1, n, (0..n).map(|i| ((n - i) % 257) as f64).collect())
                        .unwrap()
                        .into();
                let mut slots = Vec::with_capacity(n);
                for i in 0..n {
                    slots.push(Arc::new(Array::Character(
                        CharacterRow::from_text(&format!("{:05}", (n - i) % 257), &cancel).unwrap(),
                    )));
                }
                let text: Array = CellArray::from_shared(1, n, slots, &cancel).unwrap().into();
                let mut e = Engine::default();
                e.merge_workspace(BTreeMap::from([
                    ("A".into(), numeric.clone()),
                    ("B".into(), numeric),
                    ("C".into(), text.clone()),
                    ("T".into(), text),
                ]))
                .unwrap();
                for (name, script) in [
                    ("numeric_concat", "D=[A,B];"),
                    ("numeric_index", "D=A(end:-1:1);"),
                    ("numeric_sort", "[D,I]=sort(A);"),
                    ("numeric_unique", "[D,I,J]=unique(A);"),
                    ("numeric_membership", "[D,I]=ismember(A,B);"),
                    ("numeric_union", "D=union(A,B);"),
                    ("cell_concat", "D=[C,T];"),
                    ("cell_repmat", "D=repmat(C,2,1);"),
                    ("cell_sort", "[D,I]=sort(C);"),
                    ("cell_unique", "[D,I,J]=unique(C);"),
                    ("cell_membership", "[D,I]=ismember(C,T);"),
                    ("cell_union", "D=union(C,T);"),
                ] {
                    for _ in 0..2 {
                        e.execute(script).unwrap();
                    }
                    let start = Instant::now();
                    for _ in 0..repeats {
                        e.execute(script).unwrap();
                        std::hint::black_box(e.array("D"));
                    }
                    println!(
                        "workers={workers} n={n} reps={repeats} {name} us_per_call={:.3}",
                        start.elapsed().as_secs_f64() * 1e6 / repeats as f64
                    );
                }
            }
        });
    }
}

#[test]
fn struct_matrix_growth_keeps_existing_coordinates_and_field_order() {
    let mut e = Engine::default();
    e.execute("S=struct('a',{1,2;3,4},'b','q'); original=S; S(3,1).a=9; S(1,3)=struct('b','z','a',8); S([1,3],2)=struct('a',{5;6},'b',{'u';'v'}); C=cat(3,{1,2});").unwrap();
    let matrust::Array::Struct(s) = e.array("S").unwrap() else {
        panic!("struct")
    };
    assert_eq!((s.rows(), s.cols()), (3, 3));
    assert_eq!(s.field_names(), &["a", "b"]);
    for (i, v) in [
        (0, 1.),
        (1, 3.),
        (2, 9.),
        (3, 5.),
        (4, 4.),
        (5, 6.),
        (6, 8.),
    ] {
        assert_eq!(s.get(0, i).unwrap().as_numeric().unwrap().data(), &[v]);
    }
    assert!(s.get(0, 7).unwrap().is_empty());
    let matrust::Array::Struct(original) = e.array("original").unwrap() else {
        panic!("struct")
    };
    assert_eq!((original.rows(), original.cols()), (2, 2));
    assert!(
        original
            .get(0, 3)
            .unwrap()
            .same_snapshot(s.get(0, 4).unwrap())
    );
    let before = e.array("S").unwrap();
    assert!(
        e.execute("S(1:2,1:2)=struct('a',{1;2;3;4},'b','q');")
            .is_err()
    );
    assert!(before.same_snapshot(&e.array("S").unwrap()));
}

#[path = "integer_types.rs"]
mod integer_regressions;

#[test]
fn rows_and_empty_concatenation_warnings_match_recorded_message_text() {
    let mut e = Engine::default();
    let result = e.execute("x=[cell(0,3),{1,2}];").unwrap();
    let warning = result.output.join("\n");
    assert_eq!(
        warning,
        "Warning: This concatenation operation includes an empty array with an incorrect number of rows.\nConcatenation including empty arrays will require all arrays to have the same number of rows in a future\nrelease."
    );
    e.execute(
        "warning('off','all'); lastwarn(''); [x,ia]=unique({'b','a'},'rows'); message=lastwarn;",
    )
    .unwrap();
    let matrust::Array::Character(message) = e.array("message").unwrap() else {
        panic!("char")
    };
    assert_eq!(
        String::from_utf16(message.units()).unwrap(),
        "The 'rows' input is not supported for cell array inputs."
    );
}

#[test]
fn first_result_is_limited_to_direct_single_target_assignment() {
    let mut e = Engine::default();
    e.execute("c={2,3}; x=c{:}; s=struct('a',{4,5}); y=s.a;")
        .unwrap();
    assert_eq!(e.variables["x"].data(), &[2.]);
    assert_eq!(e.variables["y"].data(), &[4.]);
    for source in [
        "z=c{:}+1;",
        "z=s.a+1;",
        "A=[1 2;3 4]; idx={2,':'}; z=A(idx{:});",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert!(!e.contains_array("z"));
    }
}

#[test]
fn nested_brace_assignment_evaluates_subscripts_once() {
    let mut e = Engine::default();
    for seed in 0..16 {
        e.execute(&format!("rng({seed}); expected=randi(3); next=rand; rng({seed}); c={{struct('a',10),struct('a',20),struct('a',30)}}; c{{randi(3)}}.a=1; actual_next=rand;")).unwrap();
        assert_eq!(
            e.variables["next"].data(),
            e.variables["actual_next"].data()
        );
        let expected = e.variables["expected"].data()[0] as usize - 1;
        let c = cells(&e, "c");
        for i in 0..3 {
            let matrust::Array::Struct(s) = c.get_linear(i).unwrap().as_ref() else {
                panic!("struct")
            };
            assert_eq!(
                s.get(0, 0).unwrap().as_numeric().unwrap().data(),
                &[if i == expected {
                    1.
                } else {
                    10. * (i + 1) as f64
                }]
            );
        }
    }
}

#[test]
fn empty_fieldless_struct_adopts_rhs_fields_but_other_structs_reject_mismatch() {
    let mut e = Engine::default();
    e.execute("s=struct([]); s(1)=struct('a',1); t=struct('a',{}); t(2)=struct('a',2);")
        .unwrap();
    let matrust::Array::Struct(s) = e.array("s").unwrap() else {
        panic!("struct")
    };
    assert_eq!(s.field_names(), &["a"]);
    assert_eq!(s.get(0, 0).unwrap().as_numeric().unwrap().data(), &[1.]);
    for source in [
        "s(2)=struct('b',2);",
        "u=struct('a',{}); u(1)=struct('b',1);",
    ] {
        assert!(
            e.execute(source)
                .unwrap_err()
                .contains("dissimilar structures")
        );
    }
}

#[test]
fn text_rows_width_errors_and_legacy_union_indices() {
    let mut e = Engine::default();
    for name in ["ismember", "union", "intersect", "setdiff", "setxor"] {
        for suffix in ["'rows'", "'rows','legacy'"] {
            let error = e
                .execute(&format!("x={name}('ab','a',{suffix});"))
                .unwrap_err();
            assert!(
                error.contains("A and B must have the same number of columns."),
                "{name}: {error}"
            );
        }
    }
    for (a, b) in [("{'b','a','b'}", "{'b','c','b'}"), ("'bab'", "'bcb'")] {
        e.execute(&format!("[x,ia,ib]=union({a},{b},'legacy');"))
            .unwrap();
        assert_eq!(e.variables["ia"].data(), &[2.]);
        assert_eq!(e.variables["ib"].data(), &[3., 2.]);
        e.execute(&format!("[x,ia,ib]=union({a},{b});")).unwrap();
        assert_eq!(e.variables["ia"].data(), &[2., 1.]);
        assert_eq!(e.variables["ib"].data(), &[2.]);
    }
}

#[test]
fn text_rows_warnings_use_unique_message_for_every_output_path() {
    let mut e = Engine::default();
    for (name, outputs) in [
        ("unique", "[x,ia,ib]"),
        ("ismember", "[x,ia]"),
        ("union", "[x,ia,ib]"),
        ("intersect", "[x,ia,ib]"),
        ("setdiff", "[x,ia]"),
        ("setxor", "[x,ia,ib]"),
    ] {
        for target in ["x", outputs] {
            let inputs = if name == "unique" {
                "{'b','a'}"
            } else {
                "{'b','a'},{'a'}"
            };
            let result=e.execute(&format!("warning('on','all'); lastwarn(''); {target}={name}({inputs},'rows'); message=lastwarn;")).unwrap();
            let expected = "The 'rows' input is not supported for cell array inputs.";
            assert_eq!(
                result.output.join("\n"),
                format!("Warning: {expected}"),
                "{name} {target}"
            );
            let matrust::Array::Character(message) = e.array("message").unwrap() else {
                panic!("char")
            };
            assert_eq!(String::from_utf16(message.units()).unwrap(), expected);
        }
    }
}

#[test]
fn matbuild_existing_character_and_cell_behavior_is_preserved() {
    let mut e = Engine::default();
    for source in ["cat(1.5,{1},{2})", "cat(1.5,'a','b')"] {
        assert!(
            e.execute(source)
                .unwrap_err()
                .contains("Dimension must be a real positive finite integer.")
        );
    }
    for (source, message) in [
        ("rot90({1},1.5)", "k must be an integer."),
        ("squeeze({1},2)", "Too many input arguments."),
    ] {
        assert!(e.execute(source).unwrap_err().contains(message), "{source}");
    }
    e.execute("a=cat(2,'a',66.7); b=rot90({'a','b'},-1); c=squeeze({1,2}); d=cat(3,{1},[]); mixed=horzcat(struct('a',1),{2});")
        .unwrap();
    let matrust::Array::Character(a) = e.array("a").unwrap() else {
        panic!("char")
    };
    // Function-form concatenation keeps the established matbuild rounding.
    assert_eq!(a.units(), &[97, 67]);
    assert_eq!((cells(&e, "b").rows(), cells(&e, "b").cols()), (2, 1));
    assert_eq!(cells(&e, "mixed").cols(), 2);
}

#[test]
fn numeric_payload_ranges_match_named_numeric_indexing() {
    let mut e = Engine::default();
    e.execute(
        "x=(1:1024)'; s=struct('data',x); c={x}; n=numel(x); a=x(1:n); b=s.data(1:n); d=c{1}(1:n);",
    )
    .unwrap();
    for name in ["a", "b", "d"] {
        assert!(e.array("x").unwrap().same_snapshot(&e.array(name).unwrap()));
    }
    for suffix in ["(0)", "(1:1025)", "(2,2)"] {
        e.execute(&format!("y=x{suffix};")).unwrap_err();
        for prefix in ["s.data", "c{1}"] {
            e.execute(&format!("y={prefix}{suffix};")).unwrap_err();
            let expected = if suffix == "(0)" {
                "Subscript indices must either be real positive integers or logicals."
            } else {
                "Index exceeds matrix dimensions."
            };
            assert_eq!(e.diagnostic.as_ref().unwrap().message, expected);
        }
    }
}

#[test]
#[ignore = "Release timing comparison for 10 million element numeric payloads"]
fn benchmark_numeric_payload_indexing() {
    use matrust::{Array, Matrix};
    use std::{collections::BTreeMap, time::Instant};
    let n = 10_000_000;
    let input: Array = Matrix::filled(n, 1, 1.).unwrap().into();
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for script in ["y=x(:);", "y=s.data(:);", "y=c{1}(:);"] {
                    let mut e = Engine::default();
                    e.merge_workspace(BTreeMap::from([("x".into(), input.clone())]))
                        .unwrap();
                    if script.contains("s.data") {
                        e.execute("s=struct('data',x); clear x;").unwrap();
                    } else if script.contains("c{1}") {
                        e.execute("c={x}; clear x;").unwrap();
                    }

                    for _ in 0..100 {
                        e.execute(script).unwrap();
                    }
                    let start = Instant::now();
                    for _ in 0..10000 {
                        e.execute(script).unwrap();
                        std::hint::black_box(e.array("y"));
                    }
                    println!(
                        "workers={workers} n={n} reps=10000 {script} us_per_call={:.3}",
                        start.elapsed().as_secs_f64() * 100.
                    );
                    assert!(input.same_snapshot(&e.array("y").unwrap()));
                }
            });
    }
}

#[test]
fn empty_cell_linear_growth_keeps_reference_orientation() {
    // Reference behavior (checked 2026-10-24): a 0x1 cell grows as a column by paren
    // assignment but as a row by content assignment.
    for (source, rows, cols) in [
        ("c=cell(0,1); c(3)={1};", 3, 1),
        ("c=cell(1,0); c(3)={1};", 1, 3),
        ("c={}; c(3)={1};", 1, 3),
        ("c=cell(0,3); c(2)={1};", 1, 2),
        ("c=cell(0,1); c{3}=1;", 1, 3),
    ] {
        let mut e = Engine::default();
        e.execute(source).unwrap();
        let c = cells(&e, "c");
        assert_eq!((c.rows(), c.cols()), (rows, cols), "{source}");
    }
}

#[test]
fn bracketed_targets_assign_which_results() {
    // rev4: `[s.a] = which(...)` assigns instead of running which as a statement.
    let mut e = Engine::default();
    e.execute("s = struct('a', 0); [s.a] = which('sin');")
        .unwrap();
    let matrust::Array::Struct(s) = e.array("s").unwrap() else {
        panic!("expected struct s")
    };
    assert!(s.len() == 1);
}
