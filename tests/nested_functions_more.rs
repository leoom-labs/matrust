use matrust::{Engine, syntax};
use std::{fs, path::Path};

fn fixture(dir: &Path, name: &str, source: &str) {
    fs::write(dir.join(format!("{name}.m")), source).unwrap();
}

fn workspace(dir: &Path) -> Engine {
    let mut engine = Engine::default();
    engine.set_function_directory(dir).unwrap();
    engine.set_file_directory(dir).unwrap();
    engine
}

#[test]
fn script_local_functions_can_nest_and_only_share_with_their_lexical_parent() {
    let dir = tempfile::tempdir().unwrap();
    let source = "
script_only = 100;
[answer,visible,h] = local_counter(7);
function [result,visible,h] = local_counter(seed)
    h = @script_bump;
    [result,visible] = h();
    function [value,visible] = script_bump()
        seed -1
        seed = ans;
        value = seed;
        visible = exist('script_only','var');
        if visible
            value = script_only;
        end
    end
end";
    fixture(dir.path(), "nested_script", source);
    for code in ["nested_script", source] {
        let mut e = workspace(dir.path());
        e.execute(code).unwrap();
        e.execute("assert(answer==6 && visible==0 && script_only==100); assert(h()==5);")
            .unwrap();
        let error = e.execute("script_bump();").unwrap_err();
        assert!(
            error.contains("Undefined function or variable 'script_bump'."),
            "{error}"
        );
    }
    for invalid in [
        "x=1; function a; function b; end",
        "x=1; function a; if true; function b; end; end; end",
        "x=1; function a; function b; end; function b; end; end",
    ] {
        assert!(syntax::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn nested_handle_metadata_and_counts_follow_the_bound_definition() {
    let dir = tempfile::tempdir().unwrap();
    fixture(
        dir.path(),
        "make_inc",
        "
function [h,before] = make_inc(x)
    local_only = 99;
    h = @inc;
    byname = str2func('inc');
    before = functions(h);
    assert(strcmp(func2str(h),'inc') && isa(h,'function_handle'));
    assert(nargin(h)==2 && nargout(h)==2);
    assert(byname(2,0)==x);
    h = byname;
    function [y,updated] = inc(delta,~)
        x = x+delta;
        y = x;
        updated = x;
    end
end
function y = inc()
    y = -1;
end",
    );
    let mut e = workspace(dir.path());
    e.execute("[h,before]=make_inc(10);").unwrap();
    e.execute(
        "
assert(strcmp(func2str(h),'inc') && isa(h,'function_handle'));
assert(nargin(h)==2 && nargout(h)==2);
assert(strcmp(before.type,'nested') && strcmp(before.function,'make_inc/inc'));
assert(isequal(size(before.workspace),[1 1]) && iscell(before.workspace));
assert(isstruct(before.workspace{1}) && before.workspace{1}.x==10);
assert(~isfield(before.workspace{1},'local_only'));
[value,updated]=h(4,0);
after=functions(h);
assert(value==16 && updated==16 && after.workspace{1}.x==16);
assert(strcmp(after.type,'nested') && strcmp(after.function,'make_inc/inc'));
assert(before.workspace{1}.x==10);
nested_file=after.file;",
    )
    .unwrap();
    let matrust::Array::Character(file) = e.array("nested_file").unwrap() else {
        panic!("expected a character row");
    };
    let path = file.to_utf8(&Default::default()).unwrap();
    assert_eq!(Path::new(&path), dir.path().join("make_inc.m"));
}

#[test]
fn nested_handle_workspace_distinguishes_active_and_parked_invocations() {
    let dir = tempfile::tempdir().unwrap();
    fixture(
        dir.path(),
        "other_frame",
        "
function [h,view] = other_frame(x,other)
    h = @read_value;
    view = functions(other);
    function y = read_value(varargin)
        y = x;
    end
end",
    );
    let mut e = workspace(dir.path());
    e.execute(
        "
[first,unused]=other_frame(5,@sin);
[second,view]=other_frame(11,first);
assert(view.workspace{1}.x==5 && first()==5 && second()==11);
assert(nargin(first)==-1 && nargout(first)==1);",
    )
    .unwrap();
}

#[test]
fn eval_and_assignin_check_the_destination_static_workspace() {
    let dir = tempfile::tempdir().unwrap();
    fixture(
        dir.path(),
        "dynamic_writes",
        r#"
function result = dynamic_writes(which)
    x = 1;
    if false
        later = 0;
    end
    eval('later=3; if false; never_created=4; end');
    switch which
        case 1
            eval('new_parent=1;');
        case 2
            child('new_child(1)=1;');
        case 3
            caller_write('new_caller');
        case 4
            caller_eval('new_evalin=1;');
        case 5
            eval('new_struct.value=1;');
        case 6
            eval('for new_index=1:2; end');
        case 7
            eval('global new_global;');
        case 8
            eval('try; error(''caught''); catch new_error; end');
        case 9
            eval('[new_output,x(1)]=deal(1,2);');
        case 10
            eval('sin=1;');
            x = sin(0);
        otherwise
            eval('x=x+2;');
            child('x=x+4;');
            caller_write('x');
            caller_eval('x=x+1;');
            assignin('base','in_base',17);
    end
    result = x+later;
    function child(code)
        assert(x(1)>0);
        eval(code);
    end
    function caller_eval(code)
        evalin('caller',code);
    end
end
function caller_write(name)
    assignin('caller',name,10);
end"#,
    );
    let mut e = workspace(dir.path());
    for (case, name) in [
        (1, "new_parent"),
        (2, "new_child"),
        (3, "new_caller"),
        (4, "new_evalin"),
        (5, "new_struct"),
        (6, "new_index"),
        (7, "new_global"),
        (8, "new_error"),
        (9, "new_output"),
        (10, "sin"),
    ] {
        let code = format!(
            r#"
try
    dynamic_writes({case});
catch err
    assert(strcmp(err.message,'Attempt to add "{name}" to a static workspace.'));
    assert(strcmp(err.identifier,'MATLAB:err_static_workspace_violation'));
    caught=1;
end
assert(caught==1);
clear caught;"#
        );
        e.execute(&code).unwrap();
    }
    e.execute("assert(dynamic_writes(0)==14 && in_base==17); eval('base_new=1;');")
        .unwrap();
    assert_eq!(e.variables["base_new"].data(), &[1.]);
}

#[test]
fn load_checks_static_names_before_merging_and_supports_selected_names_and_outputs() {
    let dir = tempfile::tempdir().unwrap();
    fixture(
        dir.path(),
        "load_static",
        r#"
function result = load_static(whole_file)
    existing = 1;
    try
        if whole_file
            load state.mat;
        else
            load state.mat selected;
            result = eval('selected');
            return
        end
    catch err
        assert(strcmp(err.identifier,'MATLAB:err_static_workspace_violation'));
        assert(strcmp(err.message,'Attempt to add "unexpected" to a static workspace.'));
        assert(existing==1 && exist('unexpected','var')==0);
        data = load('state.mat');
        result = data.unexpected;
    end
    function child
        assert(existing==1);
    end
end"#,
    );
    let mut e = workspace(dir.path());
    e.execute(
        "
existing=20; unexpected=40; selected=60;
save state.mat existing unexpected selected;
assert(load_static(true)==40 && load_static(false)==60);",
    )
    .unwrap();
}
