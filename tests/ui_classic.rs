//! Engine 9a2: Classic `figure` + `uicontrol`, `findobj`, `gcbo`/`gcbf`, and `guidata`.
use matrust::Engine;
use std::process::Command;

fn run(code: &str) -> Engine {
    let mut engine = Engine::default();
    engine
        .execute(code)
        .unwrap_or_else(|e| panic!("{e}\n---\n{code}"));
    engine
}

fn scalar_num(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}

fn scalar_bool(e: &Engine, name: &str) -> bool {
    e.logical_variables[name].data()[0] != 0
}

fn char_var(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap_or_else(|| panic!("missing {name}")) {
        matrust::Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        other => panic!("expected char for {name}, got {other:?}"),
    }
}

const CLASSIC_COURSE_SCRIPT: &str = r#"
fig = figure('Visible', 'off', 'Name', 'CourseGUI');

% 1. Pushbutton with string callback evaluated in base workspace
n = 0;
btn_str = uicontrol(fig, 'Style', 'pushbutton', 'String', 'Inc', ...
    'Position', [20 20 80 30], 'Tag', 'btn_inc', 'Callback', 'n = n + 1;');

% 2. Pushbutton with function-handle callback checking gcbo, gcbf, and ActionData
cb_gcbo_ok = false;
cb_gcbf_ok = false;
cb_ev_name = '';
btn_fn = uicontrol(fig, 'Style', 'pushbutton', 'String', 'Check', ...
    'Units', 'normalized', 'Position', [0.1 0.1 0.2 0.1], ...
    'Tag', 'btn_check', 'BackgroundColor', [0.8 0.9 1.0], ...
    'ForegroundColor', [0 0 0.5], 'FontSize', 11, ...
    'HorizontalAlignment', 'center', 'UserData', 77, ...
    'Callback', @on_btn_check);

% 3. Popupmenu with cellstr String {'a','b','c'}; choose 'c' -> Value 3 and callback sees it
popup_seen_val = 0;
pop = uicontrol(fig, 'Style', 'popupmenu', 'String', {'a', 'b', 'c'}, ...
    'Tag', 'my_popup', 'Callback', @on_popup);

% Also pipe-delimited popupmenu String 'x|y|z'
pop_pipe = uicontrol(fig, 'Style', 'popupmenu', 'String', 'x|y|z', 'Tag', 'pipe_popup');

% 4. Edit with String
edit_seen = '';
ed = uicontrol(fig, 'Style', 'edit', 'String', 'hello', 'Tag', 'my_edit', ...
    'Callback', @(s, e) assignin('base', 'edit_seen', s.String));

% 5. Text, Slider (Min/Max/Value), Checkbox, Togglebutton, Radiobutton, Listbox
txt = uicontrol(fig, 'Style', 'text', 'String', 'Status', 'Tag', 'my_text');
sld = uicontrol(fig, 'Style', 'slider', 'Min', 10, 'Max', 50, 'Value', 20, ...
    'Tag', 'my_slider', 'Callback', @(s, e) assignin('base', 'sld_seen', s.Value));
sld_seen = 0;
chk = uicontrol(fig, 'Style', 'checkbox', 'String', 'Opt', 'Value', 0, ...
    'Tag', 'my_chk', 'Callback', @(s, e) assignin('base', 'chk_seen', s.Value));
chk_seen = -1;
tog = uicontrol(fig, 'Style', 'togglebutton', 'String', 'Tog', 'Value', 0, 'Tag', 'my_tog');
rad = uicontrol(fig, 'Style', 'radiobutton', 'String', 'Rad', 'Value', 0, 'Tag', 'my_rad');
lst = uicontrol(fig, 'Style', 'listbox', 'String', {'p', 'q', 'r'}, 'Value', 1, 'Tag', 'my_lst');

% Fire callbacks via mrUiFire
mrUiFire(btn_str, 'press');
mrUiFire(btn_str, 'press');
mrUiFire(btn_fn, 'press');
mrUiFire(pop, 'choose', 'c');
mrUiFire(pop_pipe, 'choose', 'y');
mrUiFire(ed, 'type', 'world');
mrUiFire(sld, 'slide', 35);
mrUiFire(chk, 'toggle');
mrUiFire(tog, 'toggle');
mrUiFire(rad, 'toggle');
mrUiFire(lst, 'choose', 3);

pop_val = pop.Value;
pop_pipe_val = pop_pipe.Value;
ed_str = ed.String;
sld_val = sld.Value;
chk_val = chk.Value;
tog_val = tog.Value;
rad_val = rad.Value;
lst_val = lst.Value;
btn_units = btn_fn.Units;
btn_ud = btn_fn.UserData;

% findobj by Tag
found_pop = findobj(fig, 'Tag', 'my_popup');
findobj_ok = (numel(found_pop) == 1) && (found_pop == pop);
found_all_btns = findobj(fig, 'Type', 'uicontrol', 'Style', 'pushbutton');
n_pushbuttons = numel(found_all_btns);

% guidata round trip via child control handle and figure handle
guidata(fig, struct('count', 42, 'label', 'saved'));
gd_from_ctrl = guidata(btn_str);
gd_count = gd_from_ctrl.count;
gd_label = gd_from_ctrl.label;
guidata(pop, struct('count', 99, 'label', 'updated'));
gd_from_fig = guidata(fig);
gd_count2 = gd_from_fig.count;

% Outside callback, gcbo and gcbf are empty
outside_gcbo_empty = isempty(gcbo);
outside_gcbf_empty = isempty(gcbf);

% close(fig) invalidates all controls
close(fig);
all_invalid = ~isvalid(btn_str) && ~isvalid(btn_fn) && ~isvalid(pop) && ...
    ~isvalid(pop_pipe) && ~isvalid(ed) && ~isvalid(txt) && ~isvalid(sld) && ...
    ~isvalid(chk) && ~isvalid(tog) && ~isvalid(rad) && ~isvalid(lst);

function on_btn_check(src, ev)
    assignin('base', 'cb_gcbo_ok', gcbo == src);
    assignin('base', 'cb_gcbf_ok', gcbf == src.Parent);
    assignin('base', 'cb_ev_name', ev.EventName);
end

function on_popup(src, ~)
    assignin('base', 'popup_seen_val', src.Value);
end
"#;

#[test]
fn ui_classic_course_script() {
    let e = run(CLASSIC_COURSE_SCRIPT);
    assert_eq!(scalar_num(&e, "n"), 2.0);
    assert!(scalar_bool(&e, "cb_gcbo_ok"));
    assert!(scalar_bool(&e, "cb_gcbf_ok"));
    assert_eq!(char_var(&e, "cb_ev_name"), "Action");
    assert_eq!(scalar_num(&e, "pop_val"), 3.0);
    assert_eq!(scalar_num(&e, "popup_seen_val"), 3.0);
    assert_eq!(scalar_num(&e, "pop_pipe_val"), 2.0);
    assert_eq!(char_var(&e, "ed_str"), "world");
    assert_eq!(char_var(&e, "edit_seen"), "world");
    assert_eq!(scalar_num(&e, "sld_val"), 35.0);
    assert_eq!(scalar_num(&e, "sld_seen"), 35.0);
    assert_eq!(scalar_num(&e, "chk_val"), 1.0);
    assert_eq!(scalar_num(&e, "chk_seen"), 1.0);
    assert_eq!(scalar_num(&e, "tog_val"), 1.0);
    assert_eq!(scalar_num(&e, "rad_val"), 1.0);
    assert_eq!(scalar_num(&e, "lst_val"), 3.0);
    assert_eq!(char_var(&e, "btn_units"), "normalized");
    assert_eq!(scalar_num(&e, "btn_ud"), 77.0);
    assert!(scalar_bool(&e, "findobj_ok"));
    assert_eq!(scalar_num(&e, "n_pushbuttons"), 2.0);
    assert_eq!(scalar_num(&e, "gd_count"), 42.0);
    assert_eq!(char_var(&e, "gd_label"), "saved");
    assert_eq!(scalar_num(&e, "gd_count2"), 99.0);
    assert!(scalar_bool(&e, "outside_gcbo_empty"));
    assert!(scalar_bool(&e, "outside_gcbf_empty"));
    assert!(scalar_bool(&e, "all_invalid"));
}

#[test]
fn ui_classic_course_script_runs_via_matrust_batch_cli() {
    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("classic_gui.m");
    let script = format!(
        "{CLASSIC_COURSE_SCRIPT}\nfprintf('CLASSIC_OK:%.0f:%.0f:%s:%.0f:%.0f:%d:%d\\n', n, pop_val, ed_str, sld_val, gd_count2, findobj_ok, all_invalid);\n"
    );
    std::fs::write(&script_path, script).unwrap();

    let exe = env!("CARGO_BIN_EXE_matrust");
    let out = Command::new(exe)
        .arg("-batch")
        .arg(format!(
            "run('{}')",
            script_path.to_string_lossy().replace('\'', "''")
        ))
        .output()
        .expect("failed to launch matrust -batch");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "matrust -batch failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("CLASSIC_OK:2:3:world:35:99:1:1"),
        "unexpected batch stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Runs `script` through `matrust -batch` and returns stdout + stderr.
fn le9a_batch(script: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("le9a_fix.m");
    std::fs::write(&path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg("-batch")
        .arg(format!(
            "run('{}')",
            path.to_string_lossy().replace('\'', "''")
        ))
        .output()
        .expect("failed to launch matrust -batch");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "matrust -batch failed:\n{text}");
    text
}

// le9a review item 4: clf deletes the figure's uicontrols and ui components.
#[test]
fn ui_classic_clf_deletes_uicontrols_batch() {
    let out = le9a_batch(
        r#"
for run_no = 1:3
    figure(1); clf; uicontrol('String', 'a'); uicontrol('Style', 'edit');
end
n1 = numel(findobj(1, 'Type', 'uicontrol'));
f2 = figure; u = uicontrol(f2, 'String', 'x'); ax = uiaxes(f2); clf(f2);
uf = uifigure; bb = uibutton(uf); clf(uf);
fprintf('CLF:%d:%d%d%d:%d%d\n', n1, isvalid(u), isvalid(ax), isvalid(f2), isvalid(bb), isvalid(uf));
"#,
    );
    assert!(out.contains("CLF:2:001:01"), "{out}");
}

// le9a review item 5: delete(fig)/delete(gcbf) close figures; Quit idiom.
#[test]
fn ui_classic_delete_figure_and_quit_button_batch() {
    let out = le9a_batch(
        r#"
f1 = figure; b1 = uicontrol(f1, 'String', 'Quit', 'Callback', 'delete(gcbf)'); mrUiFire(b1, 'press');
f2 = figure; b2 = uicontrol('String', 'Quit', 'Callback', @(s, e) delete(gcbf)); mrUiFire(b2, 'press');
f3 = figure; delete(f3);
f4 = figure; delete(gcf);
f5 = figure; u5 = uicontrol(f5); delete(u5);
uf = uifigure; ub = uibutton(uf, 'ButtonPushedFcn', @(s, e) delete(gcbf)); mrUiFire(ub, 'press');
fprintf('DEL:%d%d:%d%d:%d:%d:%d%d:%d\n', isvalid(b1), isvalid(f1), isvalid(b2), isvalid(f2), ...
    isvalid(f3), isvalid(f4), isvalid(u5), isvalid(f5), isvalid(uf));
"#,
    );
    assert!(out.contains("DEL:00:00:0:0:01:0"), "{out}");
    assert!(!out.contains("Warning"), "{out}");
}
