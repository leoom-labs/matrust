//! Cross-track boundaries: constrained solvers must retain N-D/string guards.
use matrust::Engine;

#[test]
fn constrained_multi_output_keeps_nd_guard() {
    for source in [
        "[x,f]=linprog(ones(1,1,2));",
        "[x,f]=quadprog(ones(1,1,2),[1;1]);",
        "[x,f]=lsqlin(ones(1,1,2),1);",
        "[x,f]=intlinprog(ones(1,1,2),1);",
        "[x,f]=fmincon(@(x) sum(x(:).^2),ones(1,1,2));",
    ] {
        let error = Engine::default().execute(source).unwrap_err();
        assert!(
            error.contains("does not support N-D arrays yet"),
            "{source}: {error}"
        );
    }
}

#[test]
fn constrained_multi_output_keeps_string_array_guard() {
    for source in [
        "[x,f]=linprog([\"1\" \"2\"]);",
        "[x,f]=lsqlin([\"1\" \"2\"],1);",
    ] {
        let error = Engine::default().execute(source).unwrap_err();
        assert!(
            error.contains("does not support string arrays yet"),
            "{source}: {error}"
        );
    }
}

#[test]
fn constrained_options_keep_scalar_string_arguments() {
    let mut engine = Engine::default();
    engine.execute("o=optimoptions(\"linprog\",\"Display\",\"off\"); [x,f,flag]=linprog(1,[],[],[],[],0,1,o);").unwrap();
    assert_eq!(engine.variables["x"].data(), &[0.0]);
    assert_eq!(engine.variables["flag"].data(), &[1.0]);
}
