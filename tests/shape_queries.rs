use matrust::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Engine, GraphicsArray, LogicalMatrix,
    Matrix,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

const PREDICATES: [&str; 5] = ["isscalar", "isvector", "isrow", "iscolumn", "ismatrix"];
const QUERIES: [&str; 6] = [
    "isscalar", "isvector", "isrow", "iscolumn", "ismatrix", "ndims",
];
// Independent expected rows: scalar, vector, row, column, matrix. In
// particular, zero elements do not erase a declared row/column dimension.
const SHAPES: [(usize, usize, [u8; 5]); 9] = [
    (0, 0, [0, 0, 0, 0, 1]),
    (1, 0, [0, 1, 1, 0, 1]),
    (0, 1, [0, 1, 0, 1, 1]),
    (0, 3, [0, 0, 0, 0, 1]),
    (3, 0, [0, 0, 0, 0, 1]),
    (1, 1, [1, 1, 1, 1, 1]),
    (1, 3, [0, 1, 1, 0, 1]),
    (3, 1, [0, 1, 0, 1, 1]),
    (2, 3, [0, 0, 0, 0, 1]),
];

fn assert_results(engine: &Engine, expected: [u8; 5]) {
    for (name, expected) in PREDICATES.into_iter().zip(expected) {
        let Array::Logical(value) = engine.array(&format!("q_{name}")).unwrap() else {
            panic!("{name} must return a logical scalar");
        };
        assert_eq!((value.rows(), value.cols()), (1, 1), "{name}");
        assert_eq!(value.data(), &[expected], "{name}");
    }
    let Array::Numeric(value) = engine.array("q_ndims").unwrap() else {
        panic!("ndims must return a double scalar");
    };
    assert_eq!((value.rows(), value.cols()), (1, 1));
    assert_eq!(value.data(), &[2.]);
}

fn queries(expression: &str) -> String {
    QUERIES
        .into_iter()
        .map(|name| format!("q_{name}={name}({expression});"))
        .collect()
}

fn check_array(engine: &mut Engine, value: Array, expected: [u8; 5]) {
    engine
        .merge_workspace(BTreeMap::from([
            ("A".into(), value.clone()),
            ("alias".into(), value.clone()),
        ]))
        .unwrap();
    let figure = engine.figure.clone();
    let selected = engine.current_figure;
    engine.execute(&queries("A")).unwrap();
    assert_results(engine, expected);
    assert!(engine.array("A").unwrap().same_snapshot(&value));
    assert!(engine.array("alias").unwrap().same_snapshot(&value));
    assert_eq!(engine.current_figure, selected);
    if let Some(figure) = figure {
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&figure));
    }
}

#[test]
fn all_six_array_variants_preserve_shapes_and_shared_storage() {
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            let cancel = AtomicBool::new(false);
            let mut engine = Engine::default();
            engine.execute("f=figure(2);plot([1 2],[3 4]);").unwrap();
            let Array::Graphics(handles) = engine.array("f").unwrap() else {
                panic!("expected typed Figure handle");
            };
            let handle = handles.get_linear(0).unwrap().clone();
            for (rows, cols, expected) in SHAPES {
                let count = rows * cols;
                let numeric = Matrix::from_column_vec(rows, cols, vec![f64::NAN; count]).unwrap();
                check_array(&mut engine, numeric.into(), expected);
                check_array(
                    &mut engine,
                    LogicalMatrix::filled(rows, cols, false, &cancel)
                        .unwrap()
                        .into(),
                    expected,
                );
                check_array(
                    &mut engine,
                    ComplexMatrix::filled(rows, cols, Complex64::new(-0.0, f64::NAN), &cancel)
                        .unwrap()
                        .into(),
                    expected,
                );
                // Lone surrogates are stored data, not valid UTF8 text. Querying shape
                // must neither decode nor flatten these rows, columns or matrices.
                check_array(
                    &mut engine,
                    CharacterRow::from_column_major(rows, cols, vec![0xd800; count], &cancel)
                        .unwrap()
                        .into(),
                    expected,
                );
                // Unset/empty cell slots do not change the cell container's shape.
                check_array(
                    &mut engine,
                    CellArray::empty(rows, cols, &cancel).unwrap().into(),
                    expected,
                );
                check_array(
                    &mut engine,
                    GraphicsArray::from_shared(rows, cols, vec![handle.clone(); count], &cancel)
                        .unwrap()
                        .into(),
                    expected,
                );
            }
        });
    }
}

#[test]
fn literals_and_public_constructors_keep_utf16_width_and_shaped_empties() {
    let mut engine = Engine::default();
    for (expression, expected) in [
        ("''", [0, 0, 0, 0, 1]),
        ("'x'", [1, 1, 1, 1, 1]),
        ("'\u{03a9}'", [1, 1, 1, 1, 1]),
        ("'\u{1f600}'", [0, 1, 1, 0, 1]),
        ("'x\u{1f600}'", [0, 1, 1, 0, 1]),
        ("char([55296 0])", [0, 1, 1, 0, 1]),
        ("char([55296;56320])", [0, 1, 0, 1, 1]),
        ("zeros(1,0)", [0, 1, 1, 0, 1]),
        ("false(0,1)", [0, 1, 0, 1, 1]),
        ("cell(0,3)", [0, 0, 0, 0, 1]),
        ("char(zeros(3,0))", [0, 0, 0, 0, 1]),
        ("{[]}", [1, 1, 1, 1, 1]),
        ("reshape(1:6,2,3)", [0, 0, 0, 0, 1]),
    ] {
        engine.execute(&queries(expression)).unwrap();
        assert_results(&engine, expected);
    }
    // The existing raw-character bound is retained, including for ndims.
    engine.execute("result=17;").unwrap();
    engine
        .execute(&format!("at_limit=ndims('{}');", "x".repeat(65_536)))
        .unwrap();
    assert_eq!(engine.variables["at_limit"].data(), &[2.]);
    let oversized = format!("result=ndims('{}');", "x".repeat(65_537));
    assert!(engine.execute(&oversized).is_err());
    assert_eq!(engine.variables["result"].data(), &[17.]);
}

#[test]
fn deleted_graphics_queries_do_not_resolve_liveness_or_change_other_figures() {
    let mut engine = Engine::default();
    engine
        .execute("f=figure(2);h=plot([1 2],[3 4]);alias=h;close(f);figure(3);plot([4 5],[6 7]);")
        .unwrap();
    let value = engine.array("alias").unwrap();
    check_array(&mut engine, value, [1, 1, 1, 1, 1]);
    engine.execute("live=isgraphics(alias);").unwrap();
    assert_eq!(engine.logical_variables["live"].data(), &[0]);
    assert_eq!(engine.figure_ids(), vec![3]);
}

#[test]
fn bad_arity_types_and_cancellation_preserve_existing_values_and_retry() {
    let mut engine = Engine::default();
    engine
        .execute("result=17;A=char([55296 0]);alias=A;figure(2);plot([1 2],[3 4]);")
        .unwrap();
    let before = engine.workspace();
    let figure = engine.figure.clone().unwrap();
    for name in QUERIES {
        for call in [
            format!("{name}()"),
            format!("{name}(A,A)"),
            format!("{name}(\"unsupported string\")"),
        ] {
            let error = engine.execute(&format!("result={call};")).unwrap_err();
            assert!(
                error.contains(if call.contains("unsupported") {
                    "MATLAB string arrays are not implemented"
                } else {
                    "exactly one array input"
                }),
                "{call}: {error}"
            );
            for (key, value) in &before {
                assert!(
                    engine.array(key).unwrap().same_snapshot(value),
                    "{call}: {key}"
                );
            }
            assert!(engine.figure.as_ref().unwrap().same_snapshot(&figure));
        }
        let stop = engine.cancellation();
        stop.store(true, Ordering::Relaxed);
        assert!(
            engine
                .execute(&format!("result={name}(A);"))
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(
            engine
                .array("result")
                .unwrap()
                .same_snapshot(&before["result"])
        );
        stop.store(false, Ordering::Relaxed);
        engine.execute(&format!("retry={name}(A);")).unwrap();
        assert!(engine.array("A").unwrap().same_snapshot(&before["A"]));
    }
}

#[test]
fn established_argument_effects_and_multiple_output_preflight_remain_distinct() {
    for name in QUERIES {
        let mut engine = Engine::default();
        engine.execute("result=17;other=23;A=char(55296);").unwrap();
        let result = engine.array("result").unwrap();
        let other = engine.array("other").unwrap();
        let error = engine
            .execute(&format!("result={name}(figure(9),A);"))
            .unwrap_err();
        assert!(error.contains("exactly one array input"), "{error}");
        // Ordinary arguments are evaluated before the builtin's arity check.
        assert_eq!(engine.figure_ids(), vec![9]);
        assert!(engine.array("result").unwrap().same_snapshot(&result));
        let error = engine
            .execute(&format!("[result,other]={name}(figure(10));"))
            .unwrap_err();
        assert!(error.contains("does not support 2 outputs"), "{error}");
        // Existing multiple-output preflight rejects before argument effects.
        assert_eq!(engine.figure_ids(), vec![9]);
        assert!(engine.array("result").unwrap().same_snapshot(&result));
        assert!(engine.array("other").unwrap().same_snapshot(&other));
        let error = engine
            .execute(&format!("result={name}(disp(123));"))
            .unwrap_err();
        assert!(error.contains("requires an admitted array"), "{error}");
        assert!(
            engine
                .failed_output
                .output
                .iter()
                .any(|s| s.contains("123"))
        );
        assert!(engine.array("result").unwrap().same_snapshot(&result));
    }
}

#[test]
fn comma_separated_lists_keep_existing_evaluation_and_exact_result_count() {
    let mut engine = Engine::default();
    engine
        .execute("A=char(55296);C={A};E=cell(0,0);D={A,A};result=17;")
        .unwrap();
    for name in QUERIES {
        engine
            .execute(&format!(
                "from_cell={name}(C{{:}});from_empty_prefix={name}(E{{:}},A);"
            ))
            .unwrap();
        let from_cell = engine.array("from_cell").unwrap();
        let from_empty = engine.array("from_empty_prefix").unwrap();
        if name == "ndims" {
            let (Array::Numeric(a), Array::Numeric(b)) = (from_cell, from_empty) else {
                panic!("ndims must return doubles");
            };
            assert_eq!(a.data(), &[2.]);
            assert_eq!(b.data(), &[2.]);
        } else {
            let (Array::Logical(a), Array::Logical(b)) = (from_cell, from_empty) else {
                panic!("predicate must return logicals");
            };
            assert_eq!(a.data(), &[1]);
            assert_eq!(b.data(), &[1]);
        }
        for argument in ["E{:}", "D{:}"] {
            let error = engine
                .execute(&format!("result={name}({argument});"))
                .unwrap_err();
            assert!(error.contains("exactly one array input"), "{error}");
            assert_eq!(engine.variables["result"].data(), &[17.]);
        }
    }
}

#[test]
fn user_functions_variables_help_and_completions_preserve_their_existing_routes() {
    let directory = tempfile::tempdir().unwrap();
    for name in QUERIES {
        let mut local = Engine::default();
        local
            .execute(&format!(
                "[a,b]={name}(3,4);function [x,y]={name}(u,v);x=u;y=v;end"
            ))
            .unwrap();
        assert_eq!(local.variables["a"].data(), &[3.]);
        assert_eq!(local.variables["b"].data(), &[4.]);
        std::fs::write(
            directory.path().join(format!("{name}.m")),
            format!("function y={name}(x,z)\ny=x+z;\nend\n"),
        )
        .unwrap();
        let mut file = Engine::default();
        file.set_function_directory(directory.path()).unwrap();
        file.execute(&format!("result={name}(7,8);")).unwrap();
        assert_eq!(file.variables["result"].data(), &[15.]);
        let mut variable = Engine::default();
        variable
            .execute(&format!("{name}=[11 19];result={name}(2);"))
            .unwrap();
        assert_eq!(variable.variables["result"].data(), &[19.]);
        let completions = matrust::session::completions(name, std::iter::empty());
        assert!(completions.iter().any(|item| item == name), "{name}");
    }
    let help = Engine::default().execute("help").unwrap().output.join("\n");
    for name in QUERIES {
        assert!(help.contains(name), "{name}");
    }
    let mut bare = Engine::default();
    bare.execute("isscalar(1);").unwrap();
    assert_eq!(bare.logical_variables["ans"].data(), &[1]);
}
