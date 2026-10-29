//! Replays the reference's integer and single corpus (`tests/fixtures/integers/expected.txt`,
//! 236 cases from `tests/fixtures/integers/cases.txt` via `gen.py`) through a fresh engine per case.
//! Block format: `@@CASE <id>|<statement>`, the exact `evalc` console text,
//! `@@CLASS <class> <size>` (absent when it errored), `@@END`; an error case
//! has `ERR <message>` instead. Ids from `LONG_FROM` on ran under `format long`
//! (the `LONG` prefix in cases.txt). The test compares the console text byte
//! for byte, the class and size of `x`, and for errors the message text.
use matrust::{Array, Engine};

// Count this thread's allocations only while checking the display preflight.
// Other integration tests can run concurrently without affecting the bound.
struct CountingAllocator;
thread_local! {
    static ALLOCATED: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
fn record_allocation(size: usize) {
    let _ = ALLOCATED.try_with(|counter| {
        if let Some(bytes) = counter.get() {
            counter.set(Some(bytes.saturating_add(size)));
        }
    });
}
// SAFETY: every operation delegates to System with the original layout and
// pointer. The thread-local counter never allocates or dereferences the pointer.
unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        record_allocation(size);
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const REFERENCE: &str = include_str!("fixtures/integers/expected.txt");
/// First case after the `LONG` marker in cases.txt.
const LONG_FROM: usize = 232;

/// Cases that cannot pass for a reason outside this task; the orchestrator decides.
const KNOWN: &[(usize, &str)] = &[];
const REFERENCE_2: &str = include_str!("fixtures/integers/expected-2.txt");
const KNOWN_2: &[(usize, &str)] = &[];

struct Case {
    id: usize,
    statement: String,
    expected: String,
    class: Option<(String, (usize, usize))>,
    error: Option<String>,
}

fn cases() -> Vec<Case> {
    parse_cases(REFERENCE)
}

fn parse_cases(reference: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut rest = reference;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + "@@CASE ".len()..];
        let (header, after) = rest.split_once('\n').expect("case header line");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("case end marker");
        let (expected, class) = match body.split_once("@@CLASS ") {
            Some((text, class)) => {
                let (name, size) = class.trim_end().split_once(' ').expect("class size");
                let dims: Vec<usize> = size
                    .trim_matches(['[', ']'])
                    .split(' ')
                    .map(|d| d.parse().expect("dimension"))
                    .collect();
                (text.to_owned(), Some((name.to_owned(), (dims[0], dims[1]))))
            }
            None => (body.to_owned(), None),
        };
        let error = expected
            .strip_prefix("ERR ")
            .map(|m| m.strip_suffix('\n').unwrap_or(m).to_owned());
        cases.push(Case {
            id: id.parse().expect("numeric id"),
            statement: statement.into(),
            expected,
            class,
            error,
        });
        rest = after;
    }
    cases
}

fn run(case: &Case) -> std::result::Result<(), String> {
    run_with_format(case, case.id >= LONG_FROM)
}

fn run_with_format(case: &Case, long: bool) -> std::result::Result<(), String> {
    let mut engine = Engine::default();
    if long {
        engine.execute("format long").expect("format long");
    }
    match (engine.execute(&case.statement), &case.error) {
        (Ok(result), None) => {
            let text: String = result.output.iter().map(|e| format!("{e}\n")).collect();
            if text != case.expected {
                return Err(format!(
                    "text\n  expected {:?}\n  actual   {:?}",
                    case.expected, text
                ));
            }
            let x: Array = engine.array("x").ok_or("no variable x")?;
            let (class, size) = case.class.clone().expect("class line");
            if x.class() != class || (x.rows(), x.cols()) != size {
                return Err(format!(
                    "class/size: expected {class} {size:?}, actual {} {:?}",
                    x.class(),
                    (x.rows(), x.cols())
                ));
            }
            Ok(())
        }
        (Err(e), Some(message)) => {
            // The engine reports where the statement failed: `Line 1, column 1: ...`.
            let e = match e.split_once(": ") {
                Some((place, rest)) if place.starts_with("Line ") => rest.to_owned(),
                _ => e,
            };
            if e == *message {
                Ok(())
            } else {
                Err(format!("error\n  expected {message:?}\n  actual   {e:?}"))
            }
        }
        (Ok(result), Some(message)) => Err(format!(
            "expected error {message:?}, got output {:?}",
            result.output
        )),
        (Err(e), None) => Err(format!("unexpected error {e:?}")),
    }
}

#[test]
fn matches_second_reference_corpus() {
    let all = parse_cases(REFERENCE_2);
    assert_eq!(all.len(), 43);
    assert!(all.iter().enumerate().all(|(i, c)| c.id == i + 1));
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in all {
        let known = KNOWN_2.iter().find(|(id, _)| *id == case.id);
        // Only these three statements have LONG prefixes in cases2.txt.
        match (run_with_format(&case, (31..=33).contains(&case.id)), known) {
            (Ok(()), None) => passed += 1,
            (Ok(()), Some((id, _))) => {
                failures.push(format!("case {id} passes; remove KNOWN_2 entry"))
            }
            (Err(e), None) => failures.push(format!("case {} `{}`: {e}", case.id, case.statement)),
            (Err(_), Some(_)) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed:\n{}",
        failures.join("\n")
    );
    assert_eq!(passed + KNOWN_2.len(), 43);
}

#[test]
fn finding_01_empty_bit_operations() {
    for op in ["bitand", "bitor", "bitxor", "bitshift"] {
        for (a, b) in [
            ("uint8([])", "uint8(1)"),
            ("uint8(1)", "uint8([])"),
            ("uint8([])", "uint8([])"),
        ] {
            let mut engine = Engine::default();
            engine.execute(&format!("x={op}({a},{b});")).unwrap();
            let x = engine.array("x").unwrap();
            assert_eq!((x.class(), x.rows(), x.cols()), ("uint8", 0, 0));
        }
    }
}

fn integers(engine: &Engine, name: &str, class: &str, expected: &[i128]) {
    let Some(Array::Typed(a)) = engine.array(name) else {
        panic!("{name} lost its typed storage")
    };
    assert_eq!(a.class().name(), class, "{name}");
    assert_eq!(
        (0..a.len()).map(|i| a.int(i)).collect::<Vec<_>>(),
        expected,
        "{name}"
    );
}

#[test]
fn finding_02_indexed_assignment_evaluates_original_target() {
    let mut e = Engine::default();
    e.execute("x=int8([10 20]); x(x==20)=5;").unwrap();
    integers(&e, "x", "int8", &[10, 5]);
    e.execute("x(end)=7; x(x==10)=[]; x(end+1)=8;").unwrap();
    integers(&e, "x", "int8", &[7, 8]);
    assert!(e.execute("x([x 0])=1;").is_err());
    integers(&e, "x", "int8", &[7, 8]);
    e.execute("x=int8([10 20;30 40]); x(x(:,2)==40,end)=9;")
        .unwrap();
    integers(&e, "x", "int8", &[10, 30, 20, 9]);
}

#[test]
fn finding_03_exact_64_bit_everyday_functions() {
    for class in ["int64", "uint64"] {
        let mut e = Engine::default();
        e.execute(&format!("h=intmax('{class}'); a=[h-1 h h-2 h-1]; [s,ix]=sort(a); [u,ia,ic]=unique(a); [mx,mi]=max(a); [mn,ni]=min(a); z=isequal(h,h-1);")).unwrap();
        let h = if class == "int64" {
            i64::MAX as i128
        } else {
            u64::MAX as i128
        };
        integers(&e, "s", class, &[h - 2, h - 1, h - 1, h]);
        integers(&e, "u", class, &[h - 2, h - 1, h]);
        integers(&e, "mx", class, &[h]);
        integers(&e, "mn", class, &[h - 2]);
        assert_eq!(
            e.array("ix")
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[3., 1., 4., 2.]
        );
        assert_eq!(
            e.array("ic")
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[2., 3., 1., 2.]
        );
        assert_eq!(
            e.array("z")
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[0.]
        );
        e.execute("s=sort(a,'descend'); u=unique(a,'stable'); m=max(a,h-1); n=min(a,h-1); c=cumsum([h-2 1 1]);").unwrap();
        integers(&e, "s", class, &[h, h - 1, h - 1, h - 2]);
        integers(&e, "u", class, &[h - 1, h, h - 2]);
        integers(&e, "m", class, &[h - 1, h, h - 1, h - 1]);
        integers(&e, "n", class, &[h - 1, h - 1, h - 2, h - 1]);
        integers(&e, "c", class, &[h - 2, h - 1, h]);
        for function in ["sum", "prod"] {
            let error = e
                .execute(&format!("x={function}(a,'native');"))
                .unwrap_err();
            assert!(
                error.contains(&format!(
                    "{function} is not supported for {class} values above 2^53 yet"
                )),
                "{error}"
            );
        }
        let error = e
            .execute(&format!(
                "x=sum({class}([9007199254740992 1 -9007199254740992]),'native');"
            ))
            .unwrap_err();
        assert!(error.contains("above 2^53 yet"), "{error}");
        let err = e.execute("x=median(a);").unwrap_err();
        assert!(
            err.contains(&format!(
                "median is not supported for {class} values above 2^53 yet"
            )),
            "{err}"
        );
        e.execute("b=[h h-2;h-1 h-3]; [s,ix]=sort(b,2); [m,mi]=max(b,[],2); c=cumsum(b,3);")
            .unwrap();
        integers(&e, "s", class, &[h - 2, h - 3, h, h - 1]);
        integers(&e, "m", class, &[h, h - 1]);
        integers(&e, "c", class, &[h, h - 1, h - 2, h - 3]);
    }
    let mut e = Engine::default();
    e.execute("x=cumsum(int64([9007199254740992 1 1])); t=isequal(intmax('int64'),double(intmax('int64')));").unwrap();
    integers(
        &e,
        "x",
        "int64",
        &[9007199254740992, 9007199254740993, 9007199254740994],
    );
    assert_eq!(
        e.array("t")
            .unwrap()
            .to_numeric(&Default::default())
            .unwrap()
            .data(),
        &[0.]
    );
}

#[test]
fn finding_04_single_matrix_power_uses_matrix_product() {
    let mut e = Engine::default();
    e.execute("x=single([1 2;3 4])^2; z=single([1 2;3 4])^0; y=single([1 2;3 4]).^2;")
        .unwrap();
    for (name, expected) in [
        ("x", vec![7., 15., 10., 22.]),
        ("z", vec![1., 0., 0., 1.]),
        ("y", vec![1., 9., 4., 16.]),
    ] {
        let a = e.array(name).unwrap();
        assert_eq!(a.class(), "single");
        assert_eq!(a.to_numeric(&Default::default()).unwrap().data(), expected);
    }
    assert!(e.execute("x=single([1 2])^2;").is_err());
}

#[test]
fn cumulative_integer_saturation_is_exact_with_one_and_four_workers() {
    use matrust::{NumericClass, TypedArray};
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let four = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let (rows, cols) = (1024, 512);
    let cancel = Default::default();
    for class in NumericClass::INTEGERS {
        let high = class.max();
        let input = TypedArray::from_ints(
            class,
            rows,
            cols,
            (0..rows * cols).map(|i: usize| if i.is_multiple_of(rows) { high - 2 } else { 1 }),
        )
        .unwrap();
        let serial = one
            .install(|| input.cumulative_integer(1, false, &cancel))
            .unwrap();
        let parallel = four
            .install(|| input.cumulative_integer(1, false, &cancel))
            .unwrap();
        for i in 0..input.len() {
            let expected = high - 2 + (i % rows).min(2) as i128;
            assert_eq!(serial.int(i), expected, "{class:?} at {i}, one worker");
            assert_eq!(parallel.int(i), expected, "{class:?} at {i}, four workers");
        }
        assert_eq!(input.int(1), 1, "input must stay immutable");
    }
}

#[test]
fn finding_05_integer_power_preserves_exponent_parity() {
    let mut e = Engine::default();
    for (expr, expected) in [
        ("int8(-1)^3", -1),
        ("int8(-1)^256", 1),
        ("int8(-2)^257", -128),
        ("int8(-2)^256", 127),
        ("int64(-1)^intmax('int64')", -1),
        ("(-1)^intmax('int64')", -1),
        ("(-2)^int64(257)", i64::MIN as i128),
    ] {
        e.execute(&format!("x={expr};")).unwrap();
        let Some(Array::Typed(a)) = e.array("x") else {
            panic!("typed")
        };
        assert_eq!(a.int(0), expected, "{expr}");
    }
    for expr in ["int8(2).^-1", "int8(2).^int8(-1)", "int8(2).^0.5"] {
        assert!(
            e.execute(expr)
                .unwrap_err()
                .contains("Integers can only be raised to positive integral powers.")
        );
    }
}

#[test]
fn finding_06_range_counts_checked_before_narrowing() {
    let mut e = Engine::default();
    for expr in [
        "uint64(0):intmax('uint64')",
        "intmin('int64'):intmax('int64')",
        "single(0):single(1e-40):single(1)",
    ] {
        let error = e.execute(&format!("x={expr};")).unwrap_err();
        assert!(error.contains("element limit"), "{expr}: {error}");
        assert!(e.array("x").is_none());
    }
    e.execute("x=uint64(3):-1:uint64(1);").unwrap();
    integers(&e, "x", "uint64", &[3, 2, 1]);
}

#[test]
fn finding_07_scalar_update_keeps_untouched_integer_bits() {
    use matrust::{NumericClass, TypedArray};
    let cancel = Default::default();
    for class in [NumericClass::Int64, NumericClass::Uint64] {
        let high = class.max();
        let original: Array =
            TypedArray::from_ints(class, 1, 3, [high - 1, high, high - 2].into_iter())
                .unwrap()
                .into();
        let updated = original.with_value(0, 1, 3., &cancel).unwrap();
        let (Array::Typed(a), Array::Typed(b)) = (&original, &updated) else {
            panic!("typed")
        };
        assert_eq!([b.int(0), b.int(1), b.int(2)], [high - 1, 3, high - 2]);
        assert_eq!(a.int(1), high);
        assert!(!a.same_storage(b));
        assert!(original.with_value(1, 0, 3., &cancel).is_err());
        assert!(original.with_value(0, 3, 3., &cancel).is_err());
    }
}

#[test]
fn finding_08_single_determinant_cooperative_cancellation() {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    let mut e = Engine::default();
    e.execute("a=single(eye(2048)+ones(2048)/2048);").unwrap();
    let before = e.array("a").unwrap();
    let cancel = e.cancellation();
    let stop = cancel.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2));
        stop.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let error = e.execute("d=det(a);").unwrap_err();
    worker.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let (Array::Typed(a), Array::Typed(b)) = (before, e.array("a").unwrap()) else {
        panic!("typed")
    };
    assert!(a.same_storage(&b));
    cancel.store(false, Ordering::Relaxed);
    e.execute("d=det(single([1 2;3 4]));").unwrap();
    let value = e
        .array("d")
        .unwrap()
        .to_numeric(&Default::default())
        .unwrap()
        .data()[0];
    assert!((value + 2.).abs() < 1e-6);
}

#[test]
fn finding_09_nested_typed_leaf_assignment() {
    let mut e = Engine::default();
    e.execute(
        "s.x=int8([1 2]); s.x(2)=3; x=s.x; s.inner.x=single([1 2]); s.inner.x(2)=4; y=s.inner.x;",
    )
    .unwrap();
    integers(&e, "x", "int8", &[1, 3]);
    assert_eq!(e.array("y").unwrap().class(), "single");
    assert_eq!(
        e.array("y")
            .unwrap()
            .to_numeric(&Default::default())
            .unwrap()
            .data(),
        &[1., 4.]
    );
}

#[test]
fn finding_10_for_ranges_preserve_class_and_validate_step() {
    let mut e = Engine::default();
    for class in ["int8", "uint64", "single"] {
        e.execute(&format!("for k={class}(1):{class}(3), x=k; end;"))
            .unwrap();
        assert_eq!(e.array("x").unwrap().class(), class);
    }
    let error = e
        .execute("for k=int8(1):0.5:int8(3), x=k; end;")
        .unwrap_err();
    assert!(
        error.contains("Double operands interacting with int8 operands must have integer values."),
        "{error}"
    );
    e.execute("for k=int64(9007199254740993):int64(9007199254740995), x=k; end;")
        .unwrap();
    integers(&e, "x", "int64", &[9007199254740995]);
}

/// Char matrices, char ranges and char loops (main's char semantics) next to
/// the integer rules of cases 38, 88, 149 and second-corpus case 1.
#[test]
fn char_semantics_and_integer_types_coexist() {
    let mut e = Engine::default();
    e.execute(
        "a=int8('a'); b=int8(100)+'a'; m=int8(['ab';'cd']); r=int8('a'):int8('c'); \
         c=[int8([65;66]) ['a';'b']]; s=''; for v='a':'c', s=[s v]; end; k=int8(0); \
         for v=int8(1):int8(3), k=k+v; end; t=['ab';'cd']'; l=int8(2)&&'a';",
    )
    .unwrap();
    integers(&e, "a", "int8", &[97]);
    integers(&e, "b", "int8", &[127]);
    integers(&e, "m", "int8", &[97, 99, 98, 100]);
    integers(&e, "r", "int8", &[97, 98, 99]);
    integers(&e, "k", "int8", &[6]);
    for (name, rows, cols, codes) in [
        ("c", 2, 2, &[65, 66, 97, 98][..]),
        ("s", 1, 3, &[97, 98, 99]),
        ("t", 2, 2, &[97, 98, 99, 100]),
    ] {
        let Some(Array::Character(value)) = e.array(name) else {
            panic!("{name} must be char")
        };
        assert_eq!(
            (value.rows(), value.cols(), value.units()),
            (rows, cols, codes)
        );
    }
    assert_eq!(e.array("l").unwrap().class(), "logical");
    for statement in ["x=['ab';'cd']+int8(1);", "x=int8(1)+['ab';'cd'];"] {
        let error = e.execute(statement).unwrap_err();
        assert!(
            error.contains(
                "Integers can only be combined with integers of the same class, or scalar doubles."
            ),
            "{error}"
        );
    }
}

#[test]
fn finding_11_complex_typed_construction_errors() {
    for expr in [
        "complex(single(1),single(2))",
        "complex(int8(1),int8(2))",
        "complex(single(1))",
        "single(1+2i)",
        "int8(1+2i)",
    ] {
        let error = Engine::default().execute(expr).unwrap_err();
        assert!(error.contains("not supported yet"), "{expr}: {error}");
    }
}

#[test]
fn finding_12_recursive_equality_for_typed_values() {
    let mut e = Engine::default();
    e.execute("c={int8(1),single([1 2])}; s.x=c; t=isequal(s,s); u=isequal(c,{int8(2),single([1 2])}); n={single(NaN)}; z=isequal(n,n); h=intmax('uint64'); w=isequal({h},{h-1});").unwrap();
    for (name, expected) in [("t", 1.), ("u", 0.), ("z", 0.), ("w", 0.)] {
        assert_eq!(
            e.array(name)
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[expected]
        );
    }
}

#[test]
fn finding_13_sortrows_preserves_class_in_all_output_forms() {
    for class in ["int8", "single", "int64", "uint64"] {
        let mut e = Engine::default();
        e.execute(&format!(
            "a={class}([2 1;1 3;2 0]); x=sortrows(a); [y,i]=sortrows(a); z=sortrows(a,-1);"
        ))
        .unwrap();
        for name in ["x", "y", "z"] {
            assert_eq!(e.array(name).unwrap().class(), class);
        }
        assert_eq!(
            e.array("x")
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[1., 2., 2., 3., 0., 1.]
        );
    }
    let mut e = Engine::default();
    e.execute("h=intmax('uint64'); x=sortrows([h;h-1;h-2]);")
        .unwrap();
    let h = u64::MAX as i128;
    integers(&e, "x", "uint64", &[h - 2, h - 1, h]);
}

#[test]
fn finding_14_idivide_rejects_invalid_operand_classes_and_shapes() {
    for other in ["[2 5]", "true", "'a'", "single(2)", "uint8(2)"] {
        for expr in [
            format!("idivide(int8(10),{other})"),
            format!("idivide({other},int8(10))"),
        ] {
            let error = Engine::default().execute(&expr).unwrap_err();
            assert!(
                error.contains("Integers can only be combined"),
                "{expr}: {error}"
            );
        }
    }
    let mut e = Engine::default();
    e.execute("x=idivide(int8([10 -7]),2); y=idivide(10,int8([2 5]));")
        .unwrap();
    integers(&e, "x", "int8", &[5, -3]);
    integers(&e, "y", "int8", &[5, 2]);
}

#[test]
fn finding_15_large_literal_constructor_obeys_shadowing() {
    for class in ["int64", "uint64"] {
        let mut e = Engine::default();
        e.execute(&format!("{class}=@(v) 42; x={class}(9007199254740993);"))
            .unwrap();
        assert_eq!(
            e.array("x")
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[42.]
        );
        e.execute(&format!("{class}=@(v) v; x={class}(9007199254740993);"))
            .unwrap();
        assert_eq!(
            e.array("x")
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[9007199254740992.]
        );
        let mut e = Engine::default();
        e.execute(&format!(
            "x={class}(9007199254740993);\nfunction y={class}(v)\ny=42;\nend"
        ))
        .unwrap();
        assert_eq!(e.array("x").unwrap().class(), "double");
        assert_eq!(
            e.array("x")
                .unwrap()
                .to_numeric(&Default::default())
                .unwrap()
                .data(),
            &[42.]
        );
    }
}

#[test]
fn finding_16_num2cell_keeps_each_typed_scalar() {
    let mut e = Engine::default();
    e.execute(
        "h=intmax('int64'); c=num2cell([h h-1]); x=c{2}; d=num2cell(single([1 2;3 4])); y=d{2,1};",
    )
    .unwrap();
    integers(&e, "x", "int64", &[i64::MAX as i128 - 1]);
    assert_eq!(e.array("y").unwrap().class(), "single");
    assert_eq!(
        e.array("y")
            .unwrap()
            .to_numeric(&Default::default())
            .unwrap()
            .data(),
        &[3.]
    );
    e.execute("c=num2cell(int8([1 2])); x=c{2};").unwrap();
    integers(&e, "x", "int8", &[2]);
}

#[test]
fn finding_17_integer_display_preflights_output_budget() {
    use matrust::{NumericClass, TypedArray, display_format};
    let fmt = Default::default();
    for class in [
        NumericClass::Int8,
        NumericClass::Int64,
        NumericClass::Uint64,
    ] {
        let n = 2 * 1024 * 1024 / class.display_width() + 1;
        let a: Array = TypedArray::from_ints(class, 1, n, std::iter::repeat(class.max()))
            .unwrap()
            .into();
        ALLOCATED.with(|counter| counter.set(Some(0)));
        let result = display_format::echo("x", &a, &fmt);
        let allocated = ALLOCATED.with(|counter| counter.replace(None).unwrap());
        assert!(
            allocated < 4096,
            "oversized display allocated {allocated} bytes before rejection"
        );
        let error = result.unwrap_err();
        assert!(
            error.contains("display") || error.contains("output"),
            "{error}"
        );
        assert!(display_format::disp(&a, &fmt).is_err());
        let small: Array = TypedArray::scalar_int(class, class.min()).into();
        assert!(
            display_format::echo("x", &small, &fmt)
                .unwrap()
                .contains(&class.min().to_string())
        );
    }
}

#[test]
fn corpus_has_every_case() {
    let all = cases();
    assert_eq!(all.len(), 236);
    assert!(all.iter().enumerate().all(|(i, c)| c.id == i + 1));
    assert_eq!(all.iter().filter(|c| c.error.is_some()).count(), 14);
}

#[test]
fn matches_reference() {
    let mut failures = Vec::new();
    let mut passed = 0;
    for case in cases() {
        let known = KNOWN.iter().find(|(id, _)| *id == case.id);
        match (run(&case), known) {
            (Ok(()), None) => passed += 1,
            (Ok(()), Some((id, _))) => {
                failures.push(format!("case {id} passes now; remove it from KNOWN"))
            }
            (Err(e), None) => failures.push(format!("case {} `{}`: {e}", case.id, case.statement)),
            (Err(_), Some(_)) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + KNOWN.len(), 236);
}

/// One variable of every new class, from the engine itself.
fn typed_workspace() -> std::collections::BTreeMap<String, Array> {
    let mut engine = Engine::default();
    engine
        .execute(
            "a8 = int8([-128 0 127]); b8 = uint8([0 255]); a16 = int16([-32768; 32767]); \
             b16 = uint16(65535); a32 = int32([-5 6; 7 -8]); b32 = uint32(4294967295); \
             a64 = int64(9007199254740993); b64 = uint64(18446744073709551615); \
             s = single([pi -1e-3 Inf]); e8 = int8([]);",
        )
        .expect("build workspace");
    engine.workspace()
}

#[test]
fn mat_v5_round_trips_every_integer_and_single_class() {
    use std::collections::BTreeSet;
    use std::sync::atomic::AtomicBool;
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let source = typed_workspace();
    assert_eq!(source.len(), 10);
    for compressed in [true, false] {
        let path = dir.path().join(format!("typed{compressed}.mat"));
        if compressed {
            matrust::files::save_array_workspace(&path, &source, &stop).unwrap();
        } else {
            matrust::files::save_array_workspace_uncompressed(&path, &source, &stop).unwrap();
        }
        let catalog = matrust::files::inspect_workspace(&path, &stop).unwrap();
        assert!(catalog.variables().all(|x| x.unavailable.is_none()));
        let names: BTreeSet<String> = source.keys().cloned().collect();
        let loaded = matrust::files::load_selected_arrays(&catalog, &names, &stop).unwrap();
        for (name, a) in &source {
            let b = &loaded[name];
            assert_eq!(
                (a.class(), a.rows(), a.cols()),
                (b.class(), b.rows(), b.cols()),
                "{name}"
            );
            let (Array::Typed(x), Array::Typed(y)) = (a, b) else {
                panic!("{name} lost its class");
            };
            assert_eq!(x.to_le_bytes(), y.to_le_bytes(), "{name}");
        }
    }
}

#[test]
fn save_and_load_commands_keep_classes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.mat");
    let path = path.to_string_lossy().replace('\\', "/");
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "x = int64(9007199254740993); y = single(0.1); save('{path}'); clear; load('{path}');"
        ))
        .unwrap();
    let out = engine.execute("c = class(x); d = class(y); z = x").unwrap();
    assert_eq!(out.output.join("\n"), "\nz =\n\n     9007199254740993\n");
    let Some(Array::Character(c)) = engine.array("c") else {
        panic!("class text");
    };
    assert_eq!(String::from_utf16_lossy(c.units()), "int64");
}

#[test]
fn storage_uses_the_class_element_size() {
    let w = typed_workspace();
    assert_eq!(w["a8"].bytes(), 3);
    assert_eq!(w["a16"].bytes(), 4);
    assert_eq!(w["a32"].bytes(), 16);
    assert_eq!(w["a64"].bytes(), 8);
    assert_eq!(w["s"].bytes(), 12);
    assert_eq!(w["a8"].class(), "int8");
    assert_eq!(w["s"].class(), "single");
}

#[test]
fn metadata_queries_borrow_typed_storage() {
    let mut engine = Engine::default();
    engine.execute("a=int64(ones(1,1048576));").unwrap();
    let before = engine.array("a").unwrap();
    ALLOCATED.with(|counter| counter.set(Some(0)));
    let result = engine.execute(
        "s=size(a); n=numel(a); l=length(a); d=ndims(a); t=[isscalar(a) isvector(a) isrow(a) iscolumn(a) ismatrix(a) isempty(a) isreal(a) islogical(a) isnumeric(a)];",
    );
    let allocated = ALLOCATED.with(|counter| counter.replace(None).unwrap());
    result.unwrap();
    assert!(
        allocated < 65536,
        "metadata queries allocated {allocated} bytes"
    );
    assert_eq!(engine.variables["s"].data(), &[1., 1048576.]);
    assert_eq!(engine.variables["n"].data(), &[1048576.]);
    assert_eq!(engine.variables["l"].data(), &[1048576.]);
    assert_eq!(engine.variables["d"].data(), &[2.]);
    assert_eq!(
        engine
            .array("t")
            .unwrap()
            .to_numeric(&Default::default())
            .unwrap()
            .data(),
        &[0., 1., 1., 0., 1., 0., 1., 0., 1.],
    );
    let (Array::Typed(a), Some(Array::Typed(b))) = (before, engine.array("a")) else {
        panic!("typed storage");
    };
    assert!(a.same_storage(&b));
}

#[test]
fn typed_indexing_uses_numeric_selection_limits() {
    let n = 65537;
    for class in ["int8", "uint64", "single"] {
        let mut e = Engine::default();
        let value = if class == "single" {
            "single(0.25)".to_owned()
        } else {
            format!("intmax('{class}')-1")
        };
        e.execute(&format!("a=repmat({value},1,{n});")).unwrap();
        let before = e.array("a").unwrap();
        e.execute("b=a(:); c=a(1:end); d=a(true(1,65537));")
            .unwrap();
        let Array::Typed(original) = &before else {
            panic!("typed input")
        };
        for (name, rows, cols) in [("b", n, 1), ("c", 1, n), ("d", 1, n)] {
            let Some(Array::Typed(result)) = e.array(name) else {
                panic!("typed output")
            };
            assert_eq!(
                (result.class().name(), result.rows(), result.cols()),
                (class, rows, cols)
            );
            assert_eq!(result.to_le_bytes(), original.to_le_bytes());
        }
        let Some(Array::Typed(after)) = e.array("a") else {
            panic!("typed input")
        };
        assert!(original.same_storage(&after));
    }
}

#[test]
fn whos_queries_indexing_and_overflow_edges() {
    let mut engine = Engine::default();
    let text = |e: &mut Engine, s: &str| e.execute(s).unwrap().output.join("\n");
    engine.execute("x = int16([1 2 3; 4 5 6]);").unwrap();
    let whos = text(&mut engine, "whos");
    assert!(whos.contains("12 bytes  int16"), "{whos}");
    assert_eq!(text(&mut engine, "y = x(2, end)"), "\ny =\n\n      6\n");
    assert_eq!(
        text(&mut engine, "y = x(:, 2)'"),
        "\ny =\n\n      2      5\n"
    );
    assert_eq!(text(&mut engine, "n = numel(x)"), "\nn =\n\n     6\n");
    assert_eq!(
        text(&mut engine, "t = isa(x, 'numeric')"),
        "\nt =\n\n     1\n"
    );
    assert_eq!(
        text(&mut engine, "[m, i] = max(int8([3 9 2]))"),
        "\nm =\n\n    9\n\n\ni =\n\n     2\n"
    );
    // Saturating edges never panic.
    for (source, expected) in [
        ("y = intmin('int64') / -1", "  9223372036854775807"),
        ("y = abs(intmin('int64'))", "  9223372036854775807"),
        ("y = -intmin('int32')", "  2147483647"),
        (
            "y = intmax('uint64') * intmax('uint64')",
            " 18446744073709551615",
        ),
        (
            "y = idivide(intmin('int64'), int64(-1))",
            "  9223372036854775807",
        ),
        ("y = intmax('int64') - 1", "  9223372036854775806"),
    ] {
        assert_eq!(
            text(&mut engine, source),
            format!("\ny =\n\n{expected}\n"),
            "{source}"
        );
    }
    let error = engine.execute("y = int8(1+2i)").unwrap_err();
    assert!(error.contains("not supported yet"), "{error}");
    // Loops, cells and plotting see the values.
    assert_eq!(
        text(
            &mut engine,
            "s = 0; for v = int8([1 2 3]), s = s + double(v); end, t = s"
        ),
        "\nt =\n\n     6\n"
    );
    assert_eq!(text(&mut engine, "c = {int8(5)}"), "\nc = \n\n    [5]\n");
    engine.execute("plot(int8([1 5 3]))").unwrap();
    assert_eq!(text(&mut engine, "disp(int8([1 -2]))"), "    1   -2\n");
    assert_eq!(text(&mut engine, "fprintf('%d|', uint8([7 8]))"), "7|8|");
    assert_eq!(
        text(&mut engine, "y = x(x > 4)'"),
        "\ny =\n\n      5      6\n"
    );
    engine.execute("x(1, :) = [];").unwrap();
    assert_eq!(
        text(&mut engine, "y = x"),
        "\ny =\n\n      4      5      6\n"
    );
}

#[test]
fn scalar_indexed_writes_reuse_unique_native_storage_and_detach_shared_snapshots() {
    use matrust::typed_array::TypedData;
    fn pointer(e: &Engine) -> usize {
        let Some(Array::Typed(a)) = e.array("x") else {
            panic!("typed x")
        };
        match a.data() {
            TypedData::Int8(d) => d.as_ptr() as usize,
            _ => panic!("int8"),
        }
    }
    let mut e = Engine::default();
    e.execute("x=zeros(1,1e6,'int8');").unwrap();
    let before = pointer(&e);
    ALLOCATED.with(|counter| counter.set(Some(0)));
    let result = e.execute("x(19)=int8(7);");
    let allocated = ALLOCATED.with(|counter| counter.replace(None).unwrap());
    result.unwrap();
    assert!(
        allocated < 32 * 1024,
        "scalar edit allocated {allocated} bytes"
    );
    assert_eq!(before, pointer(&e));
    e.execute("for k=1:1000, x(k)=int8(k); end;").unwrap();
    assert_eq!(before, pointer(&e));
    let snapshot = e.array("x").unwrap();
    e.execute("x(19)=int8(-7);").unwrap();
    assert_ne!(before, pointer(&e));
    assert_eq!(
        snapshot.scalar(0, 18).unwrap(),
        matrust::Scalar::Integer(19)
    );
    let detached = pointer(&e);
    e.execute("x(20)=int8(-8);").unwrap();
    assert_eq!(detached, pointer(&e));
}

#[test]
fn native_indexed_assignment_keeps_aliases_indices_growth_deletion_and_bits() {
    let mut e = Engine::default();
    e.execute("a=[int64(9007199254740993) int64(9007199254740995)]; b=a; a([2 1 2])=a([1 2 2]);")
        .unwrap();
    integers(&e, "a", "int64", &[9007199254740995, 9007199254740995]);
    integers(&e, "b", "int64", &[9007199254740993, 9007199254740995]);
    e.execute("a(end+2)=int64(9007199254740997); a([2 2])=[];")
        .unwrap();
    integers(&e, "a", "int64", &[9007199254740995, 0, 9007199254740997]);
    e.execute("m=int16([1 2;3 4]); m(3,3)=int16(9); m([2 2],:)=[]; m(:,2)=[];")
        .unwrap();
    integers(&e, "m", "int16", &[1, 0, 0, 9]);
    let m = e.array("m").unwrap();
    assert_eq!((m.rows(), m.cols()), (2, 2));
    for bad in [
        "m([1 99],1)=int16([1 2 3]);",
        "m(0)=int16(2);",
        "m(1,1)=[];",
    ] {
        assert!(e.execute(bad).is_err(), "{bad}");
        assert!(m.same_snapshot(&e.array("m").unwrap()));
    }
    e.execute("n(:,2)=[uint64(9007199254740993);uint64(9007199254740995)]; n(n>0)=uint64(18446744073709551615);").unwrap();
    integers(
        &e,
        "n",
        "uint64",
        &[0, 0, u64::MAX as i128, u64::MAX as i128],
    );
    let words = [0x7fc00123u32, 0x80000000, 1, 0x7f800000];
    let raw: Vec<u8> = words.iter().flat_map(|x| x.to_le_bytes()).collect();
    let single =
        matrust::TypedArray::from_le_bytes(matrust::NumericClass::Single, 1, 4, &raw).unwrap();
    e.merge_workspace([("s".into(), single.into())].into())
        .unwrap();
    e.execute("s(3)=single(7);").unwrap();
    let Some(Array::Typed(s)) = e.array("s") else {
        panic!("single")
    };
    let mut expected = raw;
    expected[8..12].copy_from_slice(&7f32.to_le_bytes());
    assert_eq!(s.to_le_bytes(), expected);
}

#[test]
fn idivide_validates_rounding_before_zero_divisors_or_empty_iteration() {
    for (a, b) in [
        ("int8([1 2])", "int8(0)"),
        ("int8([])", "int8(0)"),
        ("int8(0)", "int8([])"),
        ("int8([])", "int8([])"),
        ("int8(1)", "int8(2)"),
    ] {
        let source = format!("x=idivide({a},{b},'bogus');");
        let error = Engine::default().execute(&source).unwrap_err();
        assert!(
            error.contains("idivide: unknown rounding option 'bogus'"),
            "{source}: {error}"
        );
        for mode in ["fix", "floor", "ceil", "round"] {
            Engine::default()
                .execute(&format!("x=idivide({a},{b},'{mode}');"))
                .unwrap();
        }
    }
}

#[test]
fn integer_single_colon_mixtures_use_the_mixed_class_error_in_every_position() {
    let expected =
        "Colon operands must be all the same type, or mixed with real double scalar values.";
    for class in matrust::NumericClass::INTEGERS {
        let class = class.name();
        for range in [
            format!("{class}(1):single(3)"),
            format!("single(1):{class}(3)"),
            format!("{class}(1):single(1):{class}(3)"),
            format!("single(1):{class}(1):single(3)"),
            format!("1:single(1):{class}(3)"),
            format!("single(1):1:{class}(3)"),
        ] {
            let error = Engine::default()
                .execute(&format!("x={range};"))
                .unwrap_err();
            assert!(error.contains(expected), "{range}: {error}");
        }
    }
    let mut e = Engine::default();
    e.execute("a=int16(1):2:int16(5); b=single(1):2:single(5);")
        .unwrap();
    integers(&e, "a", "int16", &[1, 3, 5]);
    assert_eq!(e.array("b").unwrap().class(), "single");
}

#[test]
fn desktop_integer_cell_literals_and_scalar_edits_are_exact_above_two_to_53() {
    use matrust::{
        NumericClass, Scalar, TypedArray,
        variable_edit::{self, Rect, Target},
    };
    let cancel = Default::default();
    for class in [NumericClass::Int64, NumericClass::Uint64] {
        let original: Array = TypedArray::from_ints(
            class,
            2,
            2,
            [class.min(), class.max(), 9007199254740993, class.max() - 1].into_iter(),
        )
        .unwrap()
        .into();
        let literal = "9007199254740995";
        let value = Scalar::parse_typed_cell(literal, class).unwrap();
        assert_eq!(value, Scalar::Integer(9007199254740995));
        let updated = original.with_scalar(0, 1, value, &cancel).unwrap();
        assert_eq!(updated.scalar(0, 1).unwrap(), value);
        assert_eq!(
            original.scalar(0, 1).unwrap(),
            Scalar::Integer(9007199254740993)
        );
        assert_eq!(
            updated.scalar(1, 1).unwrap(),
            Scalar::Integer(class.max() - 1)
        );
        assert!(!original.same_snapshot(&updated));
        let mut e = Engine::default();
        e.merge_workspace([("x".into(), original.clone())].into())
            .unwrap();
        let target = Target::new("x".into(), e.array("x").unwrap(), vec![]).unwrap();
        variable_edit::apply_literal(&mut e, &target, 0, 1, literal).unwrap();
        let target = Target::new("x".into(), e.array("x").unwrap(), vec![]).unwrap();
        assert_eq!(
            variable_edit::cell_literal(target.leaf().unwrap(), 0, 1).unwrap(),
            literal
        );
        let text = format!(
            "{}\t9007199254740993\n9007199254740995\t{}",
            class.min(),
            class.max()
        );
        variable_edit::paste_tsv(&mut e, &target, Rect::new(0, 0, 2, 2).unwrap(), &text).unwrap();
        integers(
            &e,
            "x",
            class.name(),
            &[class.min(), 9007199254740995, 9007199254740993, class.max()],
        );
        let target = Target::new("x".into(), e.array("x").unwrap(), vec![]).unwrap();
        assert_eq!(
            variable_edit::copy_tsv(&e, &target, Rect::new(0, 0, 2, 2).unwrap()).unwrap(),
            text
        );
        for (literal, expected) in [
            ("9007199254740993e0", 9007199254740993),
            ("9007199254740993.5", 9007199254740994),
            ("9.007199254740993e15", 9007199254740993),
            ("-9007199254740993.5", class.saturate(-9007199254740994)),
            ("1e10000", class.max()),
            ("-1e10000", class.min()),
            ("5e-1", 1),
            ("4e-1", 0),
            ("-5e-1", class.saturate(-1)),
        ] {
            assert_eq!(
                Scalar::parse_typed_cell(literal, class).unwrap(),
                Scalar::Integer(expected),
                "{literal}"
            );
        }
        for bad in ["1+2", "1e", "1e2e3", "--1", "1.2.3", "x", "."] {
            assert!(Scalar::parse_typed_cell(bad, class).is_err(), "{bad}");
        }
    }
}

#[test]
fn mat_v73_roundtrips_native_classes_exact_bits_and_empty_shapes() {
    use matrust::{NumericClass, TypedArray};
    use std::{collections::BTreeSet, sync::atomic::AtomicBool};
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed-v73.mat");
    let mut source = typed_workspace();
    for class in NumericClass::INTEGERS
        .into_iter()
        .chain([NumericClass::Single])
    {
        source.insert(
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
    source.insert(
        "bits".into(),
        TypedArray::from_le_bytes(
            NumericClass::Single,
            2,
            3,
            &bits
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .into(),
    );
    // Crosses the writer's 8 MiB native tile boundary.
    source.insert(
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
    matrust::files::save_array_workspace_v73(&path, &source, &cancel).unwrap();
    let file = hdf5::File::open(&path).unwrap();
    let mut classes = BTreeSet::new();
    for (name, value) in &source {
        let dataset = file.dataset(name).unwrap();
        let Array::Typed(a) = value else {
            panic!("typed")
        };
        if !value.is_empty() {
            assert_eq!(dataset.dtype().unwrap().size(), a.class().size(), "{name}");
        }
        // MATLAB stores MATLAB_class at its exact length, without NUL padding.
        let attr = dataset.attr("MATLAB_class").unwrap();
        assert_eq!(
            attr.dtype().unwrap().to_descriptor().unwrap(),
            hdf5::types::TypeDescriptor::FixedAscii(a.class().name().len()),
            "{name}"
        );
        assert_eq!(
            attr.dtype().unwrap().size(),
            a.class().name().len(),
            "{name}"
        );
        let text = attr.read_scalar::<hdf5::types::FixedAscii<16>>().unwrap();
        assert_eq!(text.as_bytes(), a.class().name().as_bytes(), "{name}");
        classes.insert(a.class().name());
    }
    assert_eq!(classes.len(), 9);
    drop(file);
    let catalog = matrust::files::inspect_workspace(&path, &cancel).unwrap();
    assert!(catalog.variables().all(|v| v.unavailable.is_none()));
    let names: BTreeSet<_> = source.keys().cloned().collect();
    let loaded = matrust::files::load_selected_arrays(&catalog, &names, &cancel).unwrap();
    for (name, value) in source {
        let Array::Typed(a) = value else {
            panic!("typed")
        };
        let Array::Typed(b) = &loaded[&name] else {
            panic!("{name} lost its class")
        };
        assert_eq!(
            (a.class(), a.rows(), a.cols()),
            (b.class(), b.rows(), b.cols()),
            "{name}"
        );
        assert_eq!(a.to_le_bytes(), b.to_le_bytes(), "{name}");
    }
}

#[test]
fn int64_writes_at_the_element_limit_copy_or_grow_once_within_the_budget() {
    use matrust::{NumericClass, Scalar, TypedArray};
    let n = matrust::matrix::MAX_ELEMENTS;
    assert_eq!(n, 16_777_216);
    let mut e = Engine::default();
    let array: Array = TypedArray::from_ints(
        NumericClass::Int64,
        1,
        n,
        std::iter::repeat(9007199254740993),
    )
    .unwrap()
    .into();
    e.merge_workspace([("x".into(), array.clone())].into())
        .unwrap();
    // `array` is an outside snapshot, as the desktop holds for every variable:
    // the write detaches with one native copy (128 MiB of a 256 MiB budget).
    ALLOCATED.with(|counter| counter.set(Some(0)));
    let result = e.execute("x(1)=int64(7);");
    let allocated = ALLOCATED.with(|counter| counter.replace(None).unwrap());
    result.unwrap();
    assert!(
        (8 * n..8 * n + 64 * 1024).contains(&allocated),
        "shared write allocated {allocated} bytes"
    );
    let x = e.array("x").unwrap();
    assert!(!array.same_snapshot(&x));
    assert_eq!(
        array.scalar(0, 0).unwrap(),
        Scalar::Integer(9007199254740993)
    );
    assert_eq!(x.scalar(0, 0).unwrap(), Scalar::Integer(7));
    assert_eq!(
        x.scalar(0, n - 1).unwrap(),
        Scalar::Integer(9007199254740993)
    );
    drop((array, x));

    // A copy that would exceed the budget is still rejected before it is
    // allocated: the RHS already spent 128 MiB of this statement's budget.
    let snapshot = e.array("x").unwrap();
    ALLOCATED.with(|counter| counter.set(Some(0)));
    let result = e.execute("x(2)=int64(numel(ones(1,16777216)));");
    let allocated = ALLOCATED.with(|counter| counter.replace(None).unwrap());
    assert!(result.unwrap_err().contains("evaluation budget"));
    assert!(
        allocated < 8 * n + 64 * 1024,
        "rejected copy already allocated {allocated} bytes"
    );
    assert!(snapshot.same_snapshot(&e.array("x").unwrap()));
    drop(snapshot);

    // Growth to the limit allocates the grown array once, also with a snapshot.
    e.execute("clear x; y=zeros(1,16777215,'int64'); y(end)=int64(-3);")
        .unwrap();
    let before = e.array("y").unwrap();
    ALLOCATED.with(|counter| counter.set(Some(0)));
    let result = e.execute("y(16777216)=int64(5);");
    let allocated = ALLOCATED.with(|counter| counter.replace(None).unwrap());
    result.unwrap();
    assert!(
        (8 * n..8 * n + 64 * 1024).contains(&allocated),
        "growth allocated {allocated} bytes"
    );
    let y = e.array("y").unwrap();
    assert_eq!((y.rows(), y.cols()), (1, n));
    assert_eq!(y.scalar(0, n - 2).unwrap(), Scalar::Integer(-3));
    assert_eq!(y.scalar(0, n - 1).unwrap(), Scalar::Integer(5));
    assert_eq!((before.rows(), before.cols()), (1, n - 1));
}

#[test]
fn full_typed_indexed_replacement_shares_rhs_and_converted_sources_keep_the_class() {
    let mut e = Engine::default();
    e.execute("a=zeros(1,1000000,'int8'); b=ones(1,1000000,'int8');")
        .unwrap();
    let rhs = e.array("b").unwrap();
    ALLOCATED.with(|counter| counter.set(Some(0)));
    let result = e.execute("a(:)=b;");
    let allocated = ALLOCATED.with(|counter| counter.replace(None).unwrap());
    result.unwrap();
    assert!(
        allocated < 32 * 1024,
        "full replacement copied {allocated} bytes"
    );
    assert!(rhs.same_snapshot(&e.array("a").unwrap()));
    e.execute("a(1:3)='abc'; a(4:6)=logical([1 0 1]); small=a(1:6);")
        .unwrap();
    integers(&e, "small", "int8", &[97, 98, 99, 1, 0, 1]);
    assert_eq!(rhs.scalar(0, 0).unwrap(), matrust::Scalar::Integer(1));
}
