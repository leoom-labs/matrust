use matrust::{Engine, syntax};
use std::{fs, path::Path};

fn fixture(name: &str, source: &str) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(format!("{name}.m")), source).unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    (dir, e)
}

#[test]
fn parent_and_child_share_reads_and_writes_but_formals_and_private_names_are_local() {
    let (_dir, mut e) = fixture(
        "shared",
        "
function [x,y,z] = shared(x)
    hidden = bump(10);
    x = x * 2;
    z = bump(1);
    y = exist('child_only','var');
    function xout = bump(hidden)
        child_only = 3;
        x = x + child_only;
        xout = x + hidden;
    end
end",
    );
    e.execute("[a,b,c]=shared(2);").unwrap();
    assert_eq!(e.variables["a"].data(), &[13.]);
    assert_eq!(e.variables["b"].data(), &[0.]);
    assert_eq!(e.variables["c"].data(), &[14.]);
}

#[test]
fn input_and_output_shadow_even_when_the_parent_uses_the_same_names() {
    let (_dir, mut e) = fixture(
        "shadows",
        "
function y=shadows(x)
    b=50; r=child(3); y=[x b r];
    function b=child(x)
        x=x+1; b=2*x;
    end
end",
    );
    e.execute("answer=shadows(11);").unwrap();
    assert_eq!(e.variables["answer"].data(), &[11., 50., 8.]);
}

#[test]
fn a_grandchild_can_call_an_uncle_and_read_a_grandparent() {
    let (_dir, mut e) = fixture(
        "tree",
        "
function y=tree(x)
    y=left();
    function v=left()
        v=leaf();
        function w=leaf()
            w=right()+x;
        end
    end
    function v=right()
        x=x+1; v=x;
    end
end
function v=right()
    v=99;
end",
    );
    e.execute("answer=tree(4);").unwrap();
    assert_eq!(e.variables["answer"].data(), &[10.]);
    for private in ["left", "right", "leaf"] {
        let err = e.execute(&format!("answer={private}();")).unwrap_err();
        assert!(
            err.contains(private)
                && (err.contains("nrecognized")
                    || err.contains("ndefined")
                    || err.contains("nknown")),
            "{err}"
        );
    }
}

#[test]
fn bindings_and_sibling_names_follow_the_nearest_scope() {
    let (_dir, mut e) = fixture(
        "nearest",
        "
function y=nearest(x)
    y=[first(2) second() x];
    function v=first(x)
        v=helper();
        function w=helper()
            x=x+10; w=x;
        end
    end
    function v=second()
        v=helper();
        function w=helper()
            x=x+1; w=x;
        end
    end
end",
    );
    e.execute("answer=nearest(5);").unwrap();
    assert_eq!(e.variables["answer"].data(), &[12., 6., 6.]);
}

#[test]
fn callbacks_use_live_slots_through_arrayfun_and_fzero() {
    let (_dir, mut e) = fixture(
        "callbacks",
        "
function [a,r,n]=callbacks()
    n=0; offset=1;
    h=@plus_offset;
    offset=2;
    a=arrayfun(h,[1 2 3]);
    r=fzero(@residual,[0 4]);
    function y=plus_offset(v)
        n=n+1; y=v+offset;
    end
    function y=residual(v)
        y=v-offset;
    end
end",
    );
    e.execute("[a,r,n]=callbacks();").unwrap();
    assert_eq!(e.variables["a"].data(), &[3., 4., 5.]);
    assert!((e.variables["r"].data()[0] - 2.).abs() < 1e-10);
    assert_eq!(e.variables["n"].data(), &[3.]);
}

#[test]
fn returned_counters_keep_distinct_invocations_alive_across_runs() {
    let (_dir, mut e) = fixture(
        "makeCounter",
        "
function c=makeCounter, n=0; c=@inc;
    function y=inc, n=n+1; y=n; end
end",
    );
    e.execute("c=makeCounter(); d=makeCounter(); alias=c;")
        .unwrap();
    e.execute("a=c(); b=alias(); first=d();").unwrap();
    e.execute("third=c(); second=d();").unwrap();
    for (name, value) in [
        ("a", 1.),
        ("b", 2.),
        ("third", 3.),
        ("first", 1.),
        ("second", 2.),
    ] {
        assert_eq!(e.variables[name].data(), &[value]);
    }
}

#[test]
fn recursive_parent_activations_and_escaped_handles_keep_their_own_variables() {
    let (_dir, mut e) = fixture(
        "recurse",
        "
function [r,h]=recurse(n)
    x=n; h=@inc;
    if n>1
        [sub,other]=recurse(n-1);
        r=[sub other() inc()];
    else
        r=inc();
    end
    function v=inc
        x=x+1; v=x;
    end
end",
    );
    e.execute("[r,h]=recurse(3); last=h();").unwrap();
    assert_eq!(e.variables["r"].data(), &[2., 3., 3., 4., 4.]);
    assert_eq!(e.variables["last"].data(), &[5.]);
}

#[test]
fn file_local_factory_returns_sibling_handles_that_share_one_frame() {
    let (_dir, mut e) = fixture(
        "lik_shape",
        "
function h=lik_shape(x)
    h=make_pair(x*2);
end
function h=make_pair(x)
    h={@read_value,@update};
    function v=read_value, v=x; end
    function v=update(t), x=x+t; v=read_value(); end
end
function v=read_value, v=-1; end",
    );
    e.execute("p=lik_shape(3); read=p{1}; update=p{2}; a=read(); b=update(4); c=read();")
        .unwrap();
    assert_eq!(e.variables["a"].data(), &[6.]);
    assert_eq!(e.variables["b"].data(), &[10.]);
    assert_eq!(e.variables["c"].data(), &[10.]);
}

#[test]
fn mixed_endings_conditional_definitions_and_duplicate_siblings_are_rejected() {
    for source in [
        "function a; function b; end",
        "function a; end; function b",
        "function a; function b; end; function b; end; end",
        "function a; if true; function b; end; end; end",
        "function a; for k=1:2; function b; end; end; end",
        "function a; while true; function b; end; end; end",
        "function a; try; function b; end; catch; end; end",
        "function a; switch 1; case 1; function b; end; end; end",
        "x=1; function a; function b; end",
    ] {
        assert!(syntax::parse(source).is_err(), "{source}");
    }
}

#[test]
fn gpstuff_lik_binomial_function_file() {
    let corpus = Path::new("/build/matrust-corpus");
    if !corpus.exists() {
        return;
    }
    let path = corpus.join("gpstuff/gp/lik_binomial.m");
    let source = fs::read_to_string(&path).unwrap();
    let call = source
        .lines()
        .find(|line| line.contains("=init_binomial_norm("))
        .unwrap()
        .trim();
    let offset = source.find(call).unwrap();
    let mut e = Engine::default();
    e.set_function_directory(path.parent().unwrap()).unwrap();
    e.execute("lik=lik_binomial(); ll=lik.fh.ll; p=exp(ll(lik,1,0,2));")
        .unwrap();
    assert!((e.variables["p"].data()[0] - 0.5).abs() < 1e-12);
    e.execute("yy=1; myy_i=0; sigma2ii=1; N=2; i=1;").unwrap();
    e.execute_selection(&source, offset..offset + call.len())
        .unwrap();
    e.execute("density=tf(0);").unwrap();
    // One success in two fair trials, times the standard normal density at zero.
    let expected = 0.5 / (2. * std::f64::consts::PI).sqrt();
    assert!((e.variables["density"].data()[0] - expected).abs() < 1e-12);
}

#[test]
fn nested_handles_and_anonymous_snapshots_keep_distinct_capture_rules() {
    let (_dir, mut e) = fixture(
        "captures",
        "
function [read,snap,mixed,factory]=captures(x)
    read=@live; snap=@() x; mixed=@() x+live(); factory=@() @live; x=x+1;
    function y=live, y=x; end
end",
    );
    e.execute("[h,s,m,f]=captures(10); a=h(); b=s(); c=m(); hh=f(); d=hh();")
        .unwrap();
    assert_eq!(e.variables["a"].data(), &[11.]);
    assert_eq!(e.variables["b"].data(), &[10.]);
    assert_eq!(e.variables["c"].data(), &[21.]);
    assert_eq!(e.variables["d"].data(), &[11.]);
}

#[test]
fn a_caught_child_error_preserves_shared_writes_and_restores_the_caller() {
    let (_dir, mut e) = fixture(
        "attempt",
        "
function h=attempt
    x=1;
    try
        change();
    catch
        x=x+3;
    end
    h=@read;
    function change
        x=7; error('stop');
    end
    function y=read
        y=x;
    end
end",
    );
    e.execute("h=attempt(); answer=h();").unwrap();
    assert_eq!(e.variables["answer"].data(), &[10.]);
}

#[test]
fn cyclic_closures_are_callable_and_release_their_captured_storage() {
    let (_dir, mut e) = fixture(
        "cycle",
        "
function h=cycle()
    data=ones(1,4096); h=@visit;
    function y=visit(n)
        if n>0
            y=h(n-1);
        else
            y=data(1);
        end
    end
end",
    );
    e.execute("h=cycle();").unwrap();
    assert!(e.workspace_bytes() >= 4096 * 8);
    e.execute("answer=h(1);").unwrap();
    assert_eq!(e.variables["answer"].data(), &[1.]);
    e.execute("clear h answer").unwrap();
    assert_eq!(e.workspace_bytes(), 0);
}

#[test]
fn variable_command_disambiguation_survives_nested_headers_and_ends() {
    let (_dir, mut e) = fixture(
        "command_form",
        "
function y=command_form(x)
    nested();
    function nested()
        x -1
    end
    x -2
    y=ans;
end",
    );
    let out = e.execute("r=command_form(5);").unwrap();
    assert!(
        out.output.iter().any(|s| s.contains("4")),
        "{:?}",
        out.output
    );
    assert_eq!(e.variables["r"].data(), &[3.]);
}

#[test]
fn named_handles_bind_the_nested_definition_while_feval_text_finds_the_file_local() {
    let (_dir, mut e) = fixture(
        "lookup",
        "
function y=lookup(x)
    h=@helper; byname=str2func('helper');
    y=[helper() h() feval('helper') byname()];
    function v=helper, v=x; end
end
function v=helper, v=9; end",
    );
    e.execute("r=lookup(4);").unwrap();
    assert_eq!(e.variables["r"].data(), &[4., 4., 9., 4.]);
}

#[test]
fn quadrature_ode_optimization_and_cell_callbacks_share_the_parent_parameter() {
    let (_dir, mut e) = fixture(
        "solvers",
        "
function out=solvers(k)
    c=cellfun(@linear,{1,2});
    q=quadgk(@linear,0,1);
    [t,v]=ode45(@derivative,[0 1],0);
    m=fminsearch(@objective,1);
    out=[c q v(end) m];
    function r=linear(x), r=k.*x; end
    function r=derivative(t,v), r=k; end
    function r=objective(x), r=(x-k).^2; end
end",
    );
    e.execute("r=solvers(3);").unwrap();
    let actual = e.variables["r"].data();
    assert_eq!(actual.len(), 5);
    for (a, b) in actual.iter().zip([3., 6., 1.5, 3., 3.]) {
        assert!((a - b).abs() < 1e-4, "{actual:?}");
    }
}

#[test]
fn child_can_initialize_a_later_parent_binding_and_replace_its_type() {
    let (_dir, mut e) = fixture(
        "initialize",
        "
function y=initialize
    seed();
    function seed
        state={4,5};
    end
    y=state{2};
    state=7;
    function update
        state=state+2;
    end
    update(); y=[y state];
end",
    );
    e.execute("r=initialize();").unwrap();
    assert_eq!(e.variables["r"].data(), &[5., 9.]);
}

#[test]
fn escaped_frames_count_toward_the_workspace_limit_and_clear_releases_them() {
    let (_dir, mut e) = fixture(
        "buffer",
        "
function h=buffer()
    data=ones(1,4096); h=@read;
    function y=read, y=data(1); end
end",
    );
    let mut limits = e.limits_active();
    limits.memory_bytes = 60000;
    e.limits_set(limits);
    e.execute("first=buffer();").unwrap();
    let err = e.execute("second=buffer();").unwrap_err();
    assert!(err.contains("limit") || err.contains("exceed"), "{err}");
    e.execute("r=first(); clear first; second=buffer(); s=second();")
        .unwrap();
    assert_eq!(e.variables["r"].data(), &[1.]);
    assert_eq!(e.variables["s"].data(), &[1.]);
}
