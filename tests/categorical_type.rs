//! Categorical semantics from published MathWorks documentation; MATLAB was not run.
//! https://www.mathworks.com/help/matlab/ref/categorical.html
//! https://www.mathworks.com/help/matlab/ref/categorical.countcats.html
//! https://www.mathworks.com/help/matlab/ref/categorical.isundefined.html
//! Color fixtures follow the documented valueset=1:3, catnames=red/green/blue examples.
//! Duplicate names deliberately error, as required by this task.
use matrust::{Array, Engine};
fn output(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(r) => r.output.concat(),
        Err(err) => panic!("{code}: {err}"),
    }
}

fn run(e: &mut Engine, code: &str) {
    output(e, code);
}

#[track_caller]
fn fails(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(r) => panic!("{code}: no error, printed {:?}", r.output),
        Err(err) => err,
    }
}

#[track_caller]
fn fails_with(e: &mut Engine, code: &str, part: &str) {
    let err = fails(e, code);
    assert!(err.contains(part), "{code}: {err:?} lacks {part:?}");
}

/// `expr` is a nonempty all-true logical or numeric value.
#[track_caller]
fn check(e: &mut Engine, expr: &str) {
    run(e, &format!("t__ = {expr};"));
    let ok = match e.array("t__") {
        Some(Array::Logical(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0),
        Some(Array::Numeric(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0.),
        other => panic!("{expr}: not logical: {other:?}"),
    };
    assert!(ok, "{expr} is false");
}

#[test]
fn numeric_sorted() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([10 2.5 1 2.5]);"#);
    check(
        &mut e,
        r#"isequal(categories(C),{'1';'2.5';'10'}) && isequal(double(C),[3 2 1 2])"#,
    );
}

#[test]
fn logical_input() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([true false true]);"#);
    check(
        &mut e,
        r#"isequal(categories(C),{'0';'1'}) && isequal(double(C),[2 1 2])"#,
    );
}

#[test]
fn strings_sorted() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical(["bb" "a" "bb"]);"#);
    check(
        &mut e,
        r#"isequal(categories(C),{'a';'bb'}) && isequal(double(C),[2 1 2])"#,
    );
}

#[test]
fn cellstr_column() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a';'bb';'a'});"#);
    check(
        &mut e,
        r#"isequal(size(C),[3 1]) && isequal(double(C),[1;2;1])"#,
    );
}

#[test]
fn char_row_scalar() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical('hello');"#);
    check(&mut e, r#"isscalar(C) && isequal(char(C),'hello')"#);
}

#[test]
fn missing_numeric() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([1 NaN 2]);"#);
    check(
        &mut e,
        r#"isequal(isundefined(C),[false true false]) && isnan(double(C(2)))"#,
    );
}

#[test]
fn missing_text() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical(["a" "" string(missing)]);"#);
    check(&mut e, r#"isequal(isundefined(C),[false true true])"#);
}

#[test]
fn empty_cellstr() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a';'';'bb'});"#);
    check(
        &mut e,
        r#"isequal(isundefined(C),[false;true;false]) && isequal(categories(C),{'a';'bb'})"#,
    );
}

#[test]
fn explicit_valueset() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([1 2 3 4],[3 1 2]);"#);
    check(
        &mut e,
        r#"isequaln(double(C),[2 3 1 NaN]) && isequal(categories(C),{'3';'1';'2'})"#,
    );
}

#[test]
fn named_valueset() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical([1;3;2;1;3;1],1:3,{'red','green','blue'});"#,
    );
    check(
        &mut e,
        r#"isequal(cellstr(C),{'red';'blue';'green';'red';'blue';'red'})"#,
    );
}

#[test]
fn count_column() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical([1;3;2;1;3;1],1:3,{'red','green','blue'});"#,
    );
    check(&mut e, r#"isequal(countcats(C),[3;1;2])"#);
}

#[test]
fn count_row() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([1 3 2 1 3 1],1:3);"#);
    check(&mut e, r#"isequal(countcats(C),[3 1 2])"#);
}

#[test]
fn count_matrix() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical([1 3;2 1;3 1],1:3,{'red','green','blue'});"#,
    );
    check(
        &mut e,
        r#"isequal(countcats(C),[1 2;1 0;1 1]) && isequal(countcats(C,2),[1 0 1;1 1 0;1 0 1])"#,
    );
}

#[test]
fn count_undefined_unused() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([1;NaN;1],1:3);"#);
    check(&mut e, r#"isequal(countcats(C),[2;0;0])"#);
}

#[test]
fn class_and_shape() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([1 2;3 4]); [r,c]=size(C);"#);
    check(
        &mut e,
        r#"isequal(class(C),'categorical') && r==2 && c==2 && numel(C)==4 && ~isempty(C) && iscategorical(C) && ~iscategorical(1)"#,
    );
}

#[test]
fn empty_shape() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([]);"#);
    check(
        &mut e,
        r#"isempty(C) && isequal(size(C),[0 0]) && numel(C)==0 && isempty(categories(C))"#,
    );
}

#[test]
fn linear_index() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([1 3;2 1]); D=C([4 2]);"#);
    check(&mut e, r#"isequal(double(D),[1 2])"#);
}

#[test]
fn matrix_and_logical_index() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical([1 3;2 1]); D=C(:,2); E=C(C=='1');"#,
    );
    check(&mut e, r#"isequal(double(D),[3;1])"#);
}

#[test]
fn assignment_char_and_string() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a','b'}); C(1)='b'; C(2)="a";"#);
    check(&mut e, r#"isequal(cellstr(C),{'b','a'})"#);
}

#[test]
fn assignment_new_category() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'b','a'}); D=C; C(1)='z';"#);
    check(
        &mut e,
        r#"isequal(categories(C),{'a';'b';'z'}) && isequal(cellstr(D),{'b','a'})"#,
    );
}

#[test]
fn assignment_categorical_remap() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'a','b'}); D=categorical({'z','a'}); C(2)=D(1);"#,
    );
    check(
        &mut e,
        r#"isequal(cellstr(C),{'a','z'}) && isequal(categories(C),{'a';'b';'z'})"#,
    );
}

#[test]
fn assignment_growth_undefined() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical('a'); C(3)='b';"#);
    check(&mut e, r#"isequal(isundefined(C),[false true false])"#);
}

#[test]
fn deletion() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a','b','c'}); C(2)=[];"#);
    check(
        &mut e,
        r#"isequal(cellstr(C),{'a','c'}) && numel(categories(C))==3"#,
    );
}

#[test]
fn vertical_concat_union() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"A=categorical({'b';'a'}); B=categorical({'c';'b'}); C=[A;B];"#,
    );
    check(
        &mut e,
        r#"isequal(categories(C),{'a';'b';'c'}) && isequal(cellstr(C),{'b';'a';'c';'b'})"#,
    );
}

#[test]
fn horizontal_concat_union() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"A=categorical({'z';'b'},{'z','b'}); B=categorical({'a';'z'}); C=[A,B];"#,
    );
    check(
        &mut e,
        r#"isequal(categories(C),{'z';'b';'a'}) && isequal(double(C),[1 3;2 1])"#,
    );
}

#[test]
fn text_equality() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a','b',''});"#);
    check(
        &mut e,
        r#"isequal(C=='a',[true false false]) && isequal(C=="a",[true false false]) && isequal(C=='z',[false false false]) && isequal(C~='a',[false true true])"#,
    );
}

#[test]
fn reverse_text_equality() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a','b'});"#);
    check(
        &mut e,
        r#"isequal('a'==C,[true false]) && isequal("b"==C,[false true])"#,
    );
}

#[test]
fn categorical_equality() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'a','b',''}); D=categorical({'a','b',''},{'b','a'});"#,
    );
    check(
        &mut e,
        r#"isequal(C==D,[true true false]) && isequal(C~=D,[false false true])"#,
    );
}

#[test]
fn ordinal_comparisons() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'low','high','medium',''}, {'low','medium','high'},'Ordinal',true);"#,
    );
    check(
        &mut e,
        r#"isequal(C<'medium',[true false false false]) && isequal(C<='medium',[true false true false]) && isequal(C>'medium',[false true false false]) && isequal(C>='medium',[false true true false])"#,
    );
}

#[test]
fn ordinal_array_comparison() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical([1 2],1:3,'Ordinal',true); D=categorical([2 1],1:3,'Ordinal',true);"#,
    );
    check(&mut e, r#"isequal(C<D,[true false])"#);
}

#[test]
fn conversions() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a','bb',''});"#);
    check(
        &mut e,
        r#"isequal(string(C(1:2)),["a" "bb"]) && isequal(cellstr(C),{'a','bb','<undefined>'}) && isequal(char(C(2)),'bb')"#,
    );
}

#[test]
fn unique_codes() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'b','a','b',''}); U=unique(C);"#);
    check(
        &mut e,
        r#"isequal(cellstr(U),{'a','b','<undefined>'}) && isequal(categories(U),categories(C))"#,
    );
}

#[test]
fn unique_outputs() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'b','a','b'}); [U,ia,ic]=unique(C);"#,
    );
    check(
        &mut e,
        r#"isequal(double(U),[1 2]) && isequal(ia,[2;1]) && isequal(ic,[2;1;2])"#,
    );
}

#[test]
fn membership_scalar_cellstr() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a','b','c',''});"#);
    check(
        &mut e,
        r#"isequal(ismember(C,'a'),[true false false false]) && isequal(ismember(C,{'a','c'}),[true false true false])"#,
    );
}

#[test]
fn membership_outputs() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'b','a',''}); [tf,loc]=ismember(C,{'a','b'});"#,
    );
    check(
        &mut e,
        r#"isequal(tf,[true true false]) && isequal(loc,[2 1 0])"#,
    );
}

#[test]
fn sort_undefined_last() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'b','a','','b'},{'b','a'}); [S,i]=sort(C);"#,
    );
    check(
        &mut e,
        r#"isequal(cellstr(S),{'b','b','a','<undefined>'}) && isequal(i,[1 4 2 3])"#,
    );
}

#[test]
fn sort_matrix() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical([2 1;1 2]); S=sort(C);"#);
    check(&mut e, r#"isequal(double(S),[1 1;2 2])"#);
}

#[test]
fn repmat_reshape_transpose() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'a','b'}); D=repmat(C,2,1); R=reshape(D,1,4);"#,
    );
    check(
        &mut e,
        r#"isequal(double(D),[1 2;1 2]) && isequal(double(R),[1 1 2 2]) && isequal(double(C'),[1;2])"#,
    );
}

#[test]
fn isequal_missing() {
    let mut e = Engine::default();
    run(&mut e, r#"C=categorical({'a',''}); D=C;"#);
    check(
        &mut e,
        r#"~isequal(C,D) && isequaln(C,D) && isequal(C(1),D(1))"#,
    );
}

#[test]
fn table_filter() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"C=categorical({'a';'b';'a'}); T=table(C,[1;2;3],'VariableNames',{'c','n'}); S=T(T.c=='a',:);"#,
    );
    check(
        &mut e,
        r#"isequal(S.n,[1;3]) && isequal(cellstr(S.c),{'a';'a'})"#,
    );
}

#[test]
fn table_column_assignment() {
    let mut e = Engine::default();
    run(
        &mut e,
        r#"T=table([1;2],'VariableNames',{'n'}); T.c=categorical({'a';'b'});"#,
    );
    check(&mut e, r#"isequal(T.c=='a',[true;false])"#);
}

#[test]
fn duplicate_names() {
    let mut e = Engine::default();
    fails_with(&mut e, r#"categorical([1 2],[1 2],{'x','x'})"#, "unique");
}

#[test]
fn duplicate_values() {
    let mut e = Engine::default();
    fails_with(&mut e, r#"categorical([1 2],[1 1])"#, "unique");
}

#[test]
fn ordinal_new_value() {
    let mut e = Engine::default();
    fails_with(
        &mut e,
        r#"C=categorical('a',{'a','b'},'Ordinal',true); C(1)='z';"#,
        "ordinal",
    );
}

#[test]
fn nonordinal_order() {
    let mut e = Engine::default();
    fails_with(&mut e, r#"C=categorical('a'); C<'b'"#, "ordinal");
}

#[test]
fn ordinal_mismatch() {
    let mut e = Engine::default();
    fails_with(
        &mut e,
        r#"A=categorical(1,[1 2],'Ordinal',true); B=categorical(1,[2 1],'Ordinal',true); A==B"#,
        "match",
    );
}

#[test]
fn invalid_names() {
    let mut e = Engine::default();
    fails_with(&mut e, r#"categorical(1,1,{''})"#, "nonempty");
}

#[test]
fn invalid_option() {
    let mut e = Engine::default();
    fails_with(&mut e, r#"categorical(1,'Protected',true)"#, "categorical");
}

#[test]
fn shape_limit() {
    let mut e = Engine::default();
    fails_with(
        &mut e,
        r#"repmat(categorical('a'),100000,100000)"#,
        "exceeds",
    );
}

#[test]
fn arithmetic_error() {
    let mut e = Engine::default();
    fails_with(&mut e, r#"categorical('a')+1"#, "unsupported");
}

#[test]
fn plot_error() {
    let mut e = Engine::default();
    run(&mut e, "plot(categorical({'b','a'},{'a','b'}));hold on;");
    let figure = e.figure.as_ref().unwrap();
    assert!(figure.extras.rulers[0].kind.is_numeric());
    assert_eq!(figure.effective_tick_labels(1), ["a", "b"]);
    assert_eq!(
        figure.series[0].points.iter().collect::<Vec<_>>(),
        [[1., 2.], [2., 1.]]
    );
    let before = figure.clone();
    fails_with(&mut e, "plot([1 2]);", "y-axis must be categorical");
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
}

#[test]
fn scalar_display() {
    let mut e = Engine::default();
    assert_eq!(
        output(&mut e, "C=categorical(\"x\")"),
        "C = \n\n  categorical\n\n     x \n"
    );
}
#[test]
fn column_display_and_disp() {
    let mut e = Engine::default();
    let shown = output(&mut e, "C=categorical({'a';'bb';''})");
    assert_eq!(
        shown,
        "C = \n\n  3x1 categorical array\n\n     a \n     bb \n     <undefined> \n"
    );
    assert_eq!(
        output(&mut e, "disp(C)"),
        "     a \n     bb \n     <undefined> "
    );
}
#[test]
fn table_display_text() {
    let mut e = Engine::default();
    let s = output(
        &mut e,
        "T=table(categorical({'red';'blue';''}),'VariableNames',{'color'})",
    );
    assert!(
        s.contains("red") && s.contains("blue") && s.contains("<undefined>"),
        "{s}"
    );
}
#[test]
fn storage_shared_and_public_accessors() {
    let mut e = Engine::default();
    run(&mut e, "C=categorical({'a','b'},'Ordinal',true); D=C;");
    let (Some(Array::Categorical(c)), Some(Array::Categorical(d))) = (e.array("C"), e.array("D"))
    else {
        panic!("categorical storage");
    };
    assert!(c.same_storage(&d));
    assert_eq!(c.dims(), [1, 2]);
    assert_eq!(c.codes(), [1, 2]);
    assert!(c.is_ordinal());
    assert_eq!(
        c.categories().as_ref(),
        &vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn help_examples() {
    let mut e = Engine::default();
    for name in [
        "categorical",
        "categories",
        "countcats",
        "iscategorical",
        "isundefined",
    ] {
        let page = matrust::help::page(name).unwrap();
        for line in page.lines().filter_map(|s| s.strip_prefix("Example: ")) {
            run(&mut e, line);
        }
    }
}

#[test]
#[ignore = "release timing probe; run separately with CATEGORICAL_WORKERS=1 and N"]
fn categorical_benchmark() {
    let workers = std::env::var("CATEGORICAL_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    matrust::initialize_worker_pool(workers).unwrap();
    let mut e = Engine::default();
    let n = 32768;
    e.variables.insert(
        "x".into(),
        matrust::Matrix::from_column_vec(n, 1, (0..n).map(|i| (i % 32) as f64).collect()).unwrap(),
    );
    for (label, script) in [
        ("construct", "C=categorical(x);"),
        ("clone", "D=C;"),
        ("compare", "tf=C=='12';"),
        ("count", "n=countcats(C);"),
        ("sort", "S=sort(C);"),
        ("unique", "U=unique(C);"),
        ("double", "d=double(C);"),
        ("repmat", "R=repmat(C,1,2);"),
    ] {
        run(&mut e, script);
        let mut timings = Vec::new();
        for _ in 0..7 {
            let start = std::time::Instant::now();
            run(&mut e, script);
            timings.push(start.elapsed().as_secs_f64() * 1000.);
        }
        timings.sort_by(f64::total_cmp);
        println!(
            "categorical workers={workers} n={n} {label} median_ms={:.4}",
            timings[3]
        );
    }
}

#[test]
fn errors_leave_assignment_atomic() {
    let mut e = Engine::default();
    run(&mut e, "C=categorical({'a','b'}); D=C;");
    fails(&mut e, "C(10:11)={'x','y','z'};");
    check(&mut e, "isequal(C,D) && isequal(categories(C),{'a';'b'})");
}
#[test]
fn save_rejected_clearly() {
    let mut e = Engine::default();
    let dir = tempfile::tempdir().unwrap();
    e.set_current_folder(dir.path()).unwrap();
    run(&mut e, "C=categorical('a');");
    fails_with(
        &mut e,
        "save('categorical.mat','C')",
        "categorical is not supported by save yet",
    );
}
#[test]
fn singleton_expansion() {
    let mut e = Engine::default();
    run(
        &mut e,
        "C=categorical({'a';'b'}); D=categorical({'a','b'});",
    );
    check(&mut e, "isequal(C==D,[true false;false true])");
}
#[test]
fn queries_and_ordinal_assignment() {
    let mut e = Engine::default();
    run(&mut e, "C=categorical({'a','b'},'Ordinal',true); C(1)='b';");
    check(
        &mut e,
        "isa(C,'categorical') && ~isnumeric(C) && ~isstring(C) && isequal(cellstr(C),{'b','b'})",
    );
    check(
        &mut e,
        "~iscategorical(table([1;2])) && ~iscategorical('a')",
    );
}
#[test]
fn concatenation_functions_and_empty() {
    let mut e = Engine::default();
    run(
        &mut e,
        "A=categorical('a'); B=categorical('b'); C=horzcat(A,B); D=cat(1,A,B); E=[[],A];",
    );
    check(
        &mut e,
        "isequal(cellstr(C),{'a','b'}) && isequal(cellstr(D),{'a';'b'}) && isequal(E,A)",
    );
}

#[test]
fn cancelled_work_leaves_values_intact() {
    use std::sync::atomic::Ordering;
    let mut e = Engine::default();
    run(&mut e, "C=categorical({'a','b'});");
    e.cancellation().store(true, Ordering::Relaxed);
    fails_with(&mut e, "C(1)='z';", "cancelled");
    e.cancellation().store(false, Ordering::Relaxed);
    check(&mut e, "isequal(cellstr(C),{'a','b'})");
}
#[test]
fn storage_rejects_bad_codes_and_dimensions() {
    use std::sync::Arc;
    let names = Arc::new(vec!["a".to_string()]);
    assert!(matrust::CategoricalArray::new(1, 1, vec![2], names.clone(), false).is_err());
    assert!(matrust::CategoricalArray::new(usize::MAX, 2, vec![], names, false).is_err());
}
#[test]
fn ordinal_concat_requires_same_categories() {
    let mut e = Engine::default();
    fails_with(
        &mut e,
        "A=categorical(1,[1 2],'Ordinal',true); B=categorical(1,[2 1],'Ordinal',true); [A B]",
        "match",
    );
}
#[test]
fn string_conversion_preserves_missing() {
    let mut e = Engine::default();
    run(&mut e, "C=categorical({'a',''});");
    check(&mut e, "isequal(ismissing(string(C)),[false true])");
}

#[test]
fn integer_categories_keep_all_bits() {
    let mut e = Engine::default();
    run(
        &mut e,
        "a=uint64(9007199254740992); C=categorical([a+uint64(1),a]);",
    );
    check(
        &mut e,
        "isequal(categories(C),{'9007199254740992';'9007199254740993'}) && isequal(double(C),[2 1])",
    );
}
#[test]
fn plot_second_argument_rejected_clearly() {
    let mut e = Engine::default();
    run(
        &mut e,
        "plot([10 20],categorical({'b','a'},{'a','b'}));hold on;",
    );
    let figure = e.figure.as_ref().unwrap();
    assert!(figure.extras.rulers[0].kind.is_numeric());
    assert_eq!(figure.effective_tick_labels(1), ["a", "b"]);
    assert_eq!(
        figure.series[0].points.iter().collect::<Vec<_>>(),
        [[10., 2.], [20., 1.]]
    );
    let before = figure.clone();
    fails_with(
        &mut e,
        "plot([10 20 30],categorical({'a','b'}));",
        "Plot vector length does not match",
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
}

#[test]
fn review_fixes_order_text_sort_and_counts() {
    let mut e = Engine::default();
    run(
        &mut e,
        "C = categorical({'lo';'hi'}, {'lo','hi'}); D = categorical(C, 'Ordinal', true);",
    );
    check(&mut e, "isequal(categories(D), {'lo';'hi'}) && D(1) < D(2)");
    check(
        &mut e,
        "isequal(categories(categorical([pi 2.5 3])), {'2.5';'3';'3.1416'})",
    );
    check(
        &mut e,
        "isequal(isundefined(sort(categorical({'a';'';'b'}), 'descend')), [true;false;false])",
    );
    check(
        &mut e,
        "isequal(countcats(categorical(\"x\", [\"x\" \"y\"])), [1;0])",
    );
}

#[test]
fn review_1111_countcats_empty_row_and_large_double_names() {
    // countcats default dim is the first dimension whose size is not 1.
    // zeros(1,0) is 1-by-0, so the counts are 1-by-3 zeros, not 3-by-0.
    // https://www.mathworks.com/help/matlab/ref/categorical.countcats.html
    // Large doubles use num2str-like text (%.17g round-trip). 1e308 is
    // 1e+308, not i128::MAX, and the next double is a different category.
    // https://www.mathworks.com/help/matlab/ref/num2str.html
    // https://www.mathworks.com/help/matlab/ref/categorical.html
    let mut e = Engine::default();
    check(
        &mut e,
        "isequal(countcats(categorical(zeros(1,0),1:3,{'a','b','c'})), zeros(1,3)) && isequal(size(countcats(categorical(zeros(1,0),1:3,{'a','b','c'}))), [1 3])",
    );
    check(
        &mut e,
        "isequal(countcats(categorical(zeros(0,1),1:3,{'a','b','c'})), zeros(3,1))",
    );
    check(
        &mut e,
        "isequal(categories(categorical(1e308)), {'1e+308'}) && isequal(categories(categorical(-1e308)), {'-1e+308'})",
    );
    // 1e308, realmax and realmax/2 all used to share the i128::MAX name.
    check(
        &mut e,
        "numel(categories(categorical([1e308, realmax, realmax/2, -1e308]))) == 4",
    );
    // Values inside the i128 window keep full integer text (2^53 is exact).
    check(
        &mut e,
        "isequal(categories(categorical(2^53)), {'9007199254740992'})",
    );
}
