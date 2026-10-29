use matrust::{
    Array, CellArray, CharacterRow, Complex64, ComplexMatrix, Engine, GraphicsArray, LogicalMatrix,
    Matrix,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn check(engine: &mut Engine, value: Array, empty: bool) {
    engine
        .merge_workspace(BTreeMap::from([("A".into(), value.clone())]))
        .unwrap();
    engine.execute("result=isempty(A);").unwrap();
    let Array::Logical(result) = engine.array("result").unwrap() else {
        panic!("isempty must return a logical scalar");
    };
    assert_eq!((result.rows(), result.cols()), (1, 1));
    assert_eq!(result.data(), &[u8::from(empty)]);
    assert!(engine.array("A").unwrap().same_snapshot(&value));
}

#[test]
fn all_admitted_array_types_use_shape_not_values_or_character_decoding() {
    let cancel = AtomicBool::new(false);
    let mut engine = Engine::default();
    for (rows, cols) in [(0, 0), (0, 7), (7, 0), (1, 1), (2, 3)] {
        let count = rows * cols;
        let empty = count == 0;
        let numeric = Matrix::from_column_vec(rows, cols, vec![f64::NAN; count]).unwrap();
        check(&mut engine, numeric.into(), empty);
        check(
            &mut engine,
            LogicalMatrix::filled(rows, cols, false, &cancel)
                .unwrap()
                .into(),
            empty,
        );
        check(
            &mut engine,
            ComplexMatrix::filled(rows, cols, Complex64::new(-0.0, f64::NAN), &cancel)
                .unwrap()
                .into(),
            empty,
        );
        // Unpaired surrogate code units are legal stored character data and
        // must not be projected to UTF8 just to answer a shape query.
        check(
            &mut engine,
            CharacterRow::from_column_major(rows, cols, vec![0xd800; count], &cancel)
                .unwrap()
                .into(),
            empty,
        );
        // Nonempty cell arrays remain nonempty even when every slot is unset.
        check(
            &mut engine,
            CellArray::empty(rows, cols, &cancel).unwrap().into(),
            empty,
        );
        if empty {
            check(
                &mut engine,
                GraphicsArray::from_shared(rows, cols, vec![], &cancel)
                    .unwrap()
                    .into(),
                true,
            );
        }
    }
    engine
        .execute(
            "a=isempty([]);b=isempty([0 NaN Inf]);c=isempty('');d=isempty('text');e=isempty({[]});",
        )
        .unwrap();
    for (name, expected) in [("a", 1), ("b", 0), ("c", 1), ("d", 0), ("e", 0)] {
        assert_eq!(engine.logical_variables[name].data(), &[expected]);
    }
}

#[test]
fn graphics_liveness_is_independent_of_array_emptiness() {
    let mut engine = Engine::default();
    engine.execute("f=figure(2);alias=f;before=isempty(f);close(f);after=isempty(alias);live=isgraphics(alias);").unwrap();
    assert_eq!(engine.logical_variables["before"].data(), &[0]);
    assert_eq!(engine.logical_variables["after"].data(), &[0]);
    assert_eq!(engine.logical_variables["live"].data(), &[0]);
    assert!(
        engine
            .array("f")
            .unwrap()
            .same_snapshot(&engine.array("alias").unwrap())
    );
}

#[test]
fn invalid_calls_and_stop_preserve_destination_input_and_figure_then_retry() {
    let mut engine = Engine::default();
    engine
        .execute("result=true;A=char([55296 0]);f=figure;h=plot([1 2],[3 4]);")
        .unwrap();
    let before = engine.workspace();
    let figure = engine.figure.clone().unwrap();
    for source in [
        "result=isempty();",
        "result=isempty(A,A);",
        "result=isempty(\"unsupported string\");",
        "result=isempty(h,A);",
    ] {
        let error = engine.execute(source).unwrap_err();
        if !source.contains("unsupported string") {
            assert!(
                error.contains("exactly one array input"),
                "{source}: {error}"
            );
        }
        for (name, value) in &before {
            assert!(
                engine.array(name).unwrap().same_snapshot(value),
                "{source}: {name}"
            );
        }
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&figure));
    }
    let stop = engine.cancellation();
    stop.store(true, Ordering::Relaxed);
    assert!(
        engine
            .execute("result=isempty(A);")
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
    engine.execute("result=isempty(A);").unwrap();
    assert_eq!(engine.logical_variables["result"].data(), &[0]);
    assert!(engine.array("A").unwrap().same_snapshot(&before["A"]));
}

#[test]
fn local_and_file_function_resolution_keep_precedence_over_builtin() {
    let mut local = Engine::default();
    local
        .execute("out=isempty([1 2]);function y=isempty(x);y=23;end")
        .unwrap();
    assert_eq!(local.variables["out"].data(), &[23.]);
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("isempty.m"),
        "function y=isempty(x)\ny=42;\nend\n",
    )
    .unwrap();
    let mut file = Engine::default();
    file.set_function_directory(directory.path()).unwrap();
    file.execute("out=isempty([]);").unwrap();
    assert_eq!(file.variables["out"].data(), &[42.]);
}
