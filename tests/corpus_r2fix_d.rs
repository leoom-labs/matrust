use matrust::Engine;

#[test]
fn w12_optimset_resolves_unique_prefix_on_creation_and_merge() {
    let mut e = Engine::default();
    e.execute(
        "o=optimset('Disp','off'); assert(strcmp(o.Display,'off') && ~isfield(o,'Disp'));
         m=optimset(o,struct('dIsP','iter')); assert(strcmp(m.Display,'iter'));
         n=optimset(m,'disp',[]); assert(isempty(n.Display) && numfields(n)==numfields(o));",
    )
    .unwrap();
}

#[test]
fn w13_outputfcn_is_accepted_and_can_be_cleared() {
    let mut e = Engine::default();
    e.execute(
        "cb=@(x,v,s)false;
         o=optimoptions('fminunc','OutputFcn',cb,'Display','off');
         [msg,id]=lastwarn; assert(strcmp(id,'optimoptions:fminunc:OutputFcn'));
         assert(isequal(o.OutputFcn,cb)); x=fminunc(@(x)x^2,0,o);
         c=optimoptions('fmincon','OutputFcn',cb);",
    )
    .unwrap();
    let update = e
        .execute("d=optimoptions(c,'OutputFcn',[]); assert(isempty(d.OutputFcn));")
        .unwrap();
    assert!(update.output.is_empty(), "{:?}", update.output);
    e.execute(
        "c=optimoptions('fmincon','ConstraintTolerance',1e-5,'OutputFcn',cb);
         d=optimoptions(c,'OutputFcn',[]); assert(isempty(d.OutputFcn) && d.ConstraintTolerance==1e-5);",
    )
    .unwrap();
}

#[test]
fn w14_dbstack_reports_the_physical_line_for_each_line_ending() {
    for ending in ["\r", "\r\n", "\n"] {
        let source = [
            "n=f();",
            "function y=f",
            "s=dbstack;",
            "y=s(1).line;",
            "end",
        ]
        .join(ending);
        let mut e = Engine::default();
        e.execute(&source).unwrap();
        assert_eq!(e.variables["n"].data(), &[3.], "{ending:?}");
    }
}

#[test]
fn r1_2_undefined_parenthesis_target_has_no_end() {
    let mut e = Engine::default();
    for rhs in ["struct('a',1)", "{1}", "'x'", "string('x')", "gca", "7"] {
        e.execute(&format!("x={rhs};")).unwrap();
        for index in ["end+1", "1,end+1"] {
            let err = e.execute(&format!("clear A; A({index})=x;")).unwrap_err();
            assert!(
                err.contains("end is only valid inside indexing"),
                "{rhs}: {err}"
            );
            assert!(!e.contains_array("A"), "{rhs}");
        }
        e.execute("p=1; A(p(end))=x; A(end+1)=x; assert(numel(A)==2);")
            .unwrap();
    }
}

#[test]
fn r1_3_same_named_nested_functions_have_independent_persistents() {
    let mut e = Engine::default();
    e.execute(
        "v=[f() g() f() g()];
         function y=f(); y=k(); function v=k(); persistent n; v=numel(n); n=1; end; end
         function y=g(); y=k(); function v=k(); persistent n; v=numel(n); n=1; end; end",
    )
    .unwrap();
    assert_eq!(e.variables["v"].data(), &[0., 0., 1., 1.]);
}

#[test]
fn r1_4_nested_nargin_shadows_bare_and_parenthesized_calls() {
    let mut e = Engine::default();
    e.execute("v=f(); function y=f; y=[nargin nargin()]; function z=nargin; z=7; end; end")
        .unwrap();
    assert_eq!(e.variables["v"].data(), &[7., 7.]);
}

#[test]
fn r1_5_clear_with_spaced_multiplication_is_an_expression() {
    let mut e = Engine::default();
    e.execute("clear * 2\nfunction v=clear;v=3;end").unwrap();
    assert_eq!(e.variables["ans"].data(), &[6.]);
    let mut e = Engine::default();
    e.execute("a_tmp=1; keep=2; clear *_tmp;").unwrap();
    assert!(!e.contains_array("a_tmp") && e.contains_array("keep"));
    e.execute("x_a=1; clear *x*;").unwrap();
    assert!(!e.contains_array("x_a") && e.contains_array("keep"));
    e.execute("keep=2; clear * ;").unwrap();
    assert!(!e.contains_array("keep"));
    e.execute("keep=2; clear *").unwrap();
    assert!(!e.contains_array("keep"));
    let mut e = Engine::default();
    e.execute("n=2; clear * n\nfunction v=clear;v=3;end")
        .unwrap();
    assert_eq!(e.variables["ans"].data(), &[6.]);
    assert!(e.contains_array("n"));
}

#[test]
fn r1_14_point_location_through_a_struct_returns_both_outputs() {
    let mut e = Engine::default();
    e.execute(
        "s.t=delaunayTriangulation([0 0;1 0;0 1]);
         [k,b]=s.t.pointLocation([0 0]); assert(k==1 && isequal(b,[1 0 0]));",
    )
    .unwrap();
}

#[test]
fn f3_fminunc_warns_at_solve_time_for_an_ignored_outputfcn() {
    let mut e = Engine::default();
    for options in [
        "optimset('OutputFcn',@(x,v,s)true,'Display','off')",
        "struct('outputfcn',@(x,v,s)true)",
    ] {
        e.execute(&format!(
            "lastwarn('',''); [x,~,flag]=fminunc(@(x)(x-2)^2,1,{options});
             [msg,id]=lastwarn; assert(strcmp(id,'optimoptions:fminunc:OutputFcn'));
             assert(contains(msg,'OutputFcn') && flag>0 && abs(x-2)<1e-4);"
        ))
        .unwrap();
    }
    e.execute(
        "lastwarn('',''); x=fminunc(@(x)(x-2)^2,1,optimset('OutputFcn',[],'Display','off'));
         assert(isempty(lastwarn));",
    )
    .unwrap();
}

#[test]
fn f4_missing_struct_field_takes_end_as_zero() {
    let mut e = Engine::default();
    e.execute(
        "s.a=1; s.b(end+1)={1}; assert(iscell(s.b) && isequal(size(s.b),[1 1]) && s.b{1}==1);
         t.a=1; t.b(end+1)=\"x\"; assert(isstring(t.b) && isequal(size(t.b),[1 1]) && t.b==\"x\");
         s.b(end+1)={2}; t.b(end+1)=\"y\"; assert(numel(s.b)==2 && numel(t.b)==2);",
    )
    .unwrap();
    let err = e.execute("clear A; A(end+1)={1};").unwrap_err();
    assert!(err.contains("end is only valid inside indexing"), "{err}");
}
