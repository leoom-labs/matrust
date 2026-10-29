use matrust::{
    Engine,
    syntax::{self, Expr, StatementKind},
};

fn expression(source: &str) -> Expr {
    let mut statements = syntax::parse(source).unwrap();
    assert_eq!(statements.len(), 1);
    match statements.remove(0).kind {
        StatementKind::Evaluate(v) => v.expr,
        _ => panic!("expression"),
    }
}

#[test]
fn transpose_syntax_retains_conjugation_and_postfix_order() {
    assert!(matches!(expression("A'"), Expr::Transpose(_)));
    assert!(matches!(expression("A.'"), Expr::NonConjugateTranspose(_)));
    let Expr::Transpose(inner) = expression("A.''") else {
        panic!("outer conjugate")
    };
    assert!(matches!(*inner, Expr::NonConjugateTranspose(_)));
    let Expr::NonConjugateTranspose(inner) = expression("A'.'") else {
        panic!("outer ordinary")
    };
    assert!(matches!(*inner, Expr::Transpose(_)));
    let Expr::Unary(true, inner) = expression("-A.'") else {
        panic!("unary")
    };
    assert!(matches!(*inner, Expr::NonConjugateTranspose(_)));
    assert!(matches!(expression("'can''t'"), Expr::String(s) if s == "can't"));
    assert!(syntax::parse("A.' + 'quoted'").is_ok());
    assert!(syntax::parse("A..'").is_err());
}

#[test]
fn both_transpose_tokens_preserve_current_real_and_logical_script_behavior() {
    let mut engine = Engine::default();
    engine
        .execute("A=[1,2,3;4,5,6]; B=A'; C=A.'; mask=A>2; P=mask'; Q=mask.'; R=A.''; S=-A.';")
        .unwrap();
    assert_eq!(engine.variables["B"].data(), &[1., 2., 3., 4., 5., 6.]);
    assert_eq!(engine.variables["B"].data(), engine.variables["C"].data());
    assert_eq!(
        engine.logical_variables["P"].data(),
        engine.logical_variables["Q"].data()
    );
    assert_eq!(engine.variables["R"].data(), engine.variables["A"].data());
    assert_eq!(
        engine.variables["S"].data(),
        &[-1., -2., -3., -4., -5., -6.]
    );
    engine.execute("z=1i;w=z';v=z.';").unwrap();
    assert_eq!(engine.complex_variables["w"].data()[0].im, -1.);
    assert_eq!(engine.complex_variables["v"].data()[0].im, 1.);
}
