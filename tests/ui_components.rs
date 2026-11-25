//! Engine 9a1: UI components object model, callbacks, and `mrUiFire` (-batch).
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

#[test]
fn ui_components_all_constructors_get_set_dot_and_tree() {
    let e = run(r#"
        fig = uifigure('Name', 'App', 'Position', [50 60 800 600], 'Visible', 'off', 'Color', [0.9 0.9 0.9]);
        ok_hg = ishghandle(fig) && isgraphics(fig) && ishandle(fig) && isvalid(fig);
        fig_type = fig.Type;
        fig_name_get = get(fig, 'Name');
        fig.Name = 'Renamed';
        fig_name_dot = fig.Name;
        set(fig, 'Tag', 'mainfig', 'UserData', 42);
        fig_tag = fig.Tag;
        fig_ud = fig.UserData;

        g = uigridlayout(fig, [2 3], 'RowHeight', {'1x', 40}, 'ColumnWidth', {'fit', '1x', '2x'}, ...
            'Padding', [5 6 7 8], 'RowSpacing', 12, 'ColumnSpacing', 14);
        g_type = g.Type;
        g_parent_ok = (g.Parent == fig);
        g_rh = g.RowHeight;
        g_cw = g.ColumnWidth;
        g_pad = g.Padding;
        g_rs = g.RowSpacing;
        g_cs = g.ColumnSpacing;

        p = uipanel(g, 'Title', 'Panel 1', 'FontSize', 14, 'Tooltip', 'tip');
        p_type = p.Type;
        p.Layout.Row = 1;
        p.Layout.Column = [1 2];
        p_row = p.Layout.Row;
        p_col = p.Layout.Column;

        tg = uitabgroup(g);
        t1 = uitab(tg, 'Title', 'First');
        t2 = uitab(tg, 'Title', 'Second');
        tg_type = tg.Type;
        t1_type = t1.Type;
        tg_sel1 = (tg.SelectedTab == t1);
        tg.SelectedTab = t2;
        tg_sel2 = (tg.SelectedTab == t2);

        b_push = uibutton(p, 'push', 'Text', 'Go');
        b_state = uibutton(p, 'state', 'Text', 'Hold', 'Value', true);
        bp_type = b_push.Type;
        bs_type = b_state.Type;
        bs_val = b_state.Value;

        lbl = uilabel(p, 'Text', 'Hello', 'BackgroundColor', [1 1 0]);
        lbl_type = lbl.Type;
        lbl_text = get(lbl, 'Text');

        ef_txt = uieditfield(p, 'text', 'Value', 'abc');
        ef_num = uieditfield(p, 'numeric', 'Limits', [0 10], 'Value', 5, 'ValueDisplayFormat', '%.2f');
        ef_t_type = ef_txt.Type;
        ef_n_type = ef_num.Type;
        ef_n_val = ef_num.Value;
        ef_n_fmt = ef_num.ValueDisplayFormat;

        ta = uitextarea(p, 'Value', {'line1', 'line2'});
        ta_type = ta.Type;
        ta_n = numel(ta.Value);

        dd = uidropdown(p, 'Items', {'Red', 'Green', 'Blue'}, 'ItemsData', [10 20 30], 'Value', 20);
        dd_type = dd.Type;
        dd_val = dd.Value;
        dd_ed = uidropdown(p, 'Items', {'A', 'B'}, 'Editable', 'on', 'Value', 'Custom');
        dd_ed_val = dd_ed.Value;

        lb = uilistbox(p, 'Items', {'X', 'Y', 'Z'}, 'Multiselect', 'on', 'Value', {'X', 'Z'});
        lb_type = lb.Type;
        lb_n = numel(lb.Value);

        cb = uicheckbox(p, 'Text', 'Check', 'Value', true);
        cb_type = cb.Type;
        cb_val = cb.Value;

        bg = uibuttongroup(p, 'Title', 'Choices');
        rb1 = uiradiobutton(bg, 'Text', 'Opt 1');
        rb2 = uiradiobutton(bg, 'Text', 'Opt 2');
        bg_type = bg.Type;
        rb_type = rb1.Type;
        rb1_init = rb1.Value;
        rb2_init = rb2.Value;
        bg.SelectedObject = rb2;
        rb1_after = rb1.Value;
        rb2_after = rb2.Value;

        sld = uislider(p, 'Limits', [0 50], 'Value', 25, 'MajorTicks', [0 25 50]);
        sld_type = sld.Type;
        sld_val = sld.Value;
        sld_ticks = numel(sld.MajorTicks);

        spn = uispinner(p, 'Limits', [-10 10], 'Step', 2, 'Value', 4);
        spn_type = spn.Type;
        spn_step = spn.Step;
        spn_val = spn.Value;

        sw = uiswitch(p, 'rocker', 'Items', {'Lo', 'Hi'}, 'Value', 'Hi');
        sw_type = sw.Type;
        sw_val = sw.Value;

        tbl = uitable(p, 'Data', [1 2; 3 4], 'ColumnName', {'A', 'B'}, 'ColumnEditable', [true false]);
        tbl_type = tbl.Type;
        tbl_d = tbl.Data(2, 1);

        % Table with cell data and MATLAB table data
        tbl_cell = uitable(p, 'Data', {'a', 1; 'b', 2});
        tbl_tab = uitable(p, 'Data', table([10; 20], [30; 40]));
        tbl_tab_h = height(tbl_tab.Data);

        % Tree check: deleting a child removes it from Parent.Children
        n_before = numel(p.Children);
        delete(lbl);
        n_after = numel(p.Children);
        lbl_valid = isvalid(lbl);

        % drawnow, uiwait, uiresume in -batch
        drawnow;
        uiwait(fig);
        uiwait(fig, 0.1);
        uiresume(fig);

        % delete(fig) invalidates all descendants
        delete(fig);
        fig_valid = isvalid(fig);
        g_valid = isvalid(g);
        p_valid = isvalid(p);
        b_valid = isvalid(b_push);
        t2_valid = isvalid(t2);
        rb2_valid = isvalid(rb2);
    "#);

    assert!(scalar_bool(&e, "ok_hg"));
    assert_eq!(char_var(&e, "fig_type"), "figure");
    assert_eq!(char_var(&e, "fig_name_get"), "App");
    assert_eq!(char_var(&e, "fig_name_dot"), "Renamed");
    assert_eq!(char_var(&e, "fig_tag"), "mainfig");
    assert_eq!(scalar_num(&e, "fig_ud"), 42.0);
    assert_eq!(char_var(&e, "g_type"), "uigridlayout");
    assert!(scalar_bool(&e, "g_parent_ok"));
    assert_eq!(e.variables["g_pad"].data(), &[5., 6., 7., 8.]);
    assert_eq!(scalar_num(&e, "g_rs"), 12.0);
    assert_eq!(scalar_num(&e, "g_cs"), 14.0);
    assert_eq!(char_var(&e, "p_type"), "uipanel");
    assert_eq!(scalar_num(&e, "p_row"), 1.0);
    assert_eq!(e.variables["p_col"].data(), &[1., 2.]);
    assert_eq!(char_var(&e, "tg_type"), "uitabgroup");
    assert_eq!(char_var(&e, "t1_type"), "uitab");
    assert!(scalar_bool(&e, "tg_sel1"));
    assert!(scalar_bool(&e, "tg_sel2"));
    assert_eq!(char_var(&e, "bp_type"), "uibutton");
    assert_eq!(char_var(&e, "bs_type"), "uistatebutton");
    assert!(scalar_bool(&e, "bs_val"));
    assert_eq!(char_var(&e, "lbl_type"), "uilabel");
    assert_eq!(char_var(&e, "lbl_text"), "Hello");
    assert_eq!(char_var(&e, "ef_t_type"), "uieditfield");
    assert_eq!(char_var(&e, "ef_n_type"), "uinumericeditfield");
    assert_eq!(scalar_num(&e, "ef_n_val"), 5.0);
    assert_eq!(char_var(&e, "ef_n_fmt"), "%.2f");
    assert_eq!(char_var(&e, "ta_type"), "uitextarea");
    assert_eq!(scalar_num(&e, "ta_n"), 2.0);
    assert_eq!(char_var(&e, "dd_type"), "uidropdown");
    assert_eq!(scalar_num(&e, "dd_val"), 20.0);
    assert_eq!(char_var(&e, "dd_ed_val"), "Custom");
    assert_eq!(char_var(&e, "lb_type"), "uilistbox");
    assert_eq!(scalar_num(&e, "lb_n"), 2.0);
    assert_eq!(char_var(&e, "cb_type"), "uicheckbox");
    assert!(scalar_bool(&e, "cb_val"));
    assert_eq!(char_var(&e, "bg_type"), "uibuttongroup");
    assert_eq!(char_var(&e, "rb_type"), "uiradiobutton");
    assert!(scalar_bool(&e, "rb1_init"));
    assert!(!scalar_bool(&e, "rb2_init"));
    assert!(!scalar_bool(&e, "rb1_after"));
    assert!(scalar_bool(&e, "rb2_after"));
    assert_eq!(char_var(&e, "sld_type"), "uislider");
    assert_eq!(scalar_num(&e, "sld_val"), 25.0);
    assert_eq!(scalar_num(&e, "sld_ticks"), 3.0);
    assert_eq!(char_var(&e, "spn_type"), "uispinner");
    assert_eq!(scalar_num(&e, "spn_step"), 2.0);
    assert_eq!(scalar_num(&e, "spn_val"), 4.0);
    assert_eq!(char_var(&e, "sw_type"), "uiswitch");
    assert_eq!(char_var(&e, "sw_val"), "Hi");
    assert_eq!(char_var(&e, "tbl_type"), "uitable");
    assert_eq!(scalar_num(&e, "tbl_d"), 3.0);
    assert_eq!(scalar_num(&e, "tbl_tab_h"), 2.0);
    assert_eq!(scalar_num(&e, "n_after"), scalar_num(&e, "n_before") - 1.0);
    assert!(!scalar_bool(&e, "lbl_valid"));
    assert!(!scalar_bool(&e, "fig_valid"));
    assert!(!scalar_bool(&e, "g_valid"));
    assert!(!scalar_bool(&e, "p_valid"));
    assert!(!scalar_bool(&e, "b_valid"));
    assert!(!scalar_bool(&e, "t2_valid"));
    assert!(!scalar_bool(&e, "rb2_valid"));
}

const DONE_CHECK_SCRIPT: &str = r#"
fig = uifigure('Name', 'DoneCheck', 'Position', [20 30 600 400]);
g = uigridlayout(fig, [2 2]);
lbl = uilabel(g, 'Text', 'initial');
lbl.Layout.Row = 1;
lbl.Layout.Column = 1;

btn = uibutton(g, 'Text', 'Click', ...
    'ButtonPushedFcn', @(src, ev) set(lbl, 'Text', ['pushed:' ev.EventName]));
btn.Layout.Row = 1;
btn.Layout.Column = 2;

ef_log = [];
ef_changing = NaN;
ef = uieditfield(g, 'numeric', 'Limits', [0 100], 'Value', 10, ...
    'ValueChangingFcn', @(src, ev) assignin('base', 'ef_changing', ev.Value), ...
    'ValueChangedFcn', @(src, ev) assignin('base', 'ef_log', [ev.PreviousValue ev.Value]));
ef.Layout.Row = 2;
ef.Layout.Column = 1;

sld = uislider(g, 'Limits', [0 10], 'Value', 2);
dd = uidropdown(g, 'Items', {'Alpha', 'Beta'}, 'Value', 'Alpha');

tbl_idx = [];
tbl_new = NaN;
tbl_prev = NaN;
tbl = uitable(g, 'Data', [1 2; 3 4], 'ColumnEditable', true, ...
    'CellEditCallback', @on_cell_edit);
tbl.Layout.Row = 2;
tbl.Layout.Column = 2;

% Fire button press -> ButtonPushedFcn changes uilabel Text
mrUiFire(btn, 'press');
lbl_after_press = lbl.Text;

% Fire edit field 'type' -> ValueChangingFcn then ValueChangedFcn
mrUiFire(ef, 'type', 42);
ef_val_after = ef.Value;

% Fire table 'edit' -> CellEditCallback gets Indices / NewData / PreviousData
mrUiFire(tbl, 'edit', [2 1], 99);
tbl_val_after = tbl.Data(2, 1);

% Fire slider & dropdown
mrUiFire(sld, 'slide', 7.5);
sld_val_after = sld.Value;
mrUiFire(dd, 'choose', 'Beta');
dd_val_after = dd.Value;

% Layout.Row/Column and Position read back; get/set vs dot agree
row_rb = lbl.Layout.Row;
col_rb = tbl.Layout.Column;
pos_dot = fig.Position;
pos_get = get(fig, 'Position');
pos_agree = isequal(pos_dot, pos_get);
set(fig, 'Name', 'Updated');
name_agree = strcmp(fig.Name, get(fig, 'Name'));

% delete(fig) -> isvalid false for all children
delete(fig);
all_invalid = ~isvalid(fig) && ~isvalid(g) && ~isvalid(lbl) && ~isvalid(btn) && ...
    ~isvalid(ef) && ~isvalid(sld) && ~isvalid(dd) && ~isvalid(tbl);

function on_cell_edit(~, ev)
    assignin('base', 'tbl_idx', ev.Indices);
    assignin('base', 'tbl_new', ev.NewData);
    assignin('base', 'tbl_prev', ev.PreviousData);
end
"#;

#[test]
fn ui_components_done_check_script() {
    let e = run(DONE_CHECK_SCRIPT);
    assert_eq!(char_var(&e, "lbl_after_press"), "pushed:ButtonPushed");
    assert_eq!(scalar_num(&e, "ef_changing"), 42.0);
    assert_eq!(e.variables["ef_log"].data(), &[10.0, 42.0]);
    assert_eq!(scalar_num(&e, "ef_val_after"), 42.0);
    assert_eq!(e.variables["tbl_idx"].data(), &[2.0, 1.0]);
    assert_eq!(scalar_num(&e, "tbl_new"), 99.0);
    assert_eq!(scalar_num(&e, "tbl_prev"), 3.0);
    assert_eq!(scalar_num(&e, "tbl_val_after"), 99.0);
    assert_eq!(scalar_num(&e, "sld_val_after"), 7.5);
    assert_eq!(char_var(&e, "dd_val_after"), "Beta");
    assert_eq!(scalar_num(&e, "row_rb"), 1.0);
    assert_eq!(scalar_num(&e, "col_rb"), 2.0);
    assert!(scalar_bool(&e, "pos_agree"));
    assert!(scalar_bool(&e, "name_agree"));
    assert!(scalar_bool(&e, "all_invalid"));
}

#[test]
fn ui_components_callback_forms_and_error_isolation() {
    let mut engine = Engine::default();
    let exec = engine
        .execute(
            r#"
            fig = uifigure;
            % 1. Function handle callback
            h_called = 0;
            b1 = uibutton(fig, 'ButtonPushedFcn', @(s, e) assignin('base', 'h_called', 1));
            mrUiFire(b1, 'press');

            % 2. Cell array callback with extra args
            cell_sum = 0;
            b2 = uibutton(fig, 'ButtonPushedFcn', {@add_extra, 10, 25});
            mrUiFire(b2, 'press');

            % 3. Char string callback evaluated in base workspace
            str_hits = 0;
            b3 = uibutton(fig, 'ButtonPushedFcn', 'str_hits = str_hits + 7;');
            mrUiFire(b3, 'press');

            % 4. SelectionChangedFcn on uitabgroup and uibuttongroup
            tg = uitabgroup(fig, 'SelectionChangedFcn', @(s, e) assignin('base', 'tab_ev', {e.OldValue.Title, e.NewValue.Title}));
            t1 = uitab(tg, 'Title', 'T1');
            t2 = uitab(tg, 'Title', 'T2');
            mrUiFire(t2, 'choose');

            bg = uibuttongroup(fig, 'SelectionChangedFcn', @(s, e) assignin('base', 'rb_ev', {e.OldValue.Text, e.NewValue.Text}));
            rb1 = uiradiobutton(bg, 'Text', 'R1');
            rb2 = uiradiobutton(bg, 'Text', 'R2');
            mrUiFire(rb2, 'choose');

            % 5. Toggle checkbox, state button, switch
            cb = uicheckbox(fig, 'Value', false, 'ValueChangedFcn', @(s, e) assignin('base', 'cb_ev', [e.PreviousValue e.Value]));
            mrUiFire(cb, 'toggle');
            sb = uibutton(fig, 'state', 'Value', false, 'ValueChangedFcn', @(s, e) assignin('base', 'sb_ev', [e.PreviousValue e.Value]));
            mrUiFire(sb, 'toggle');
            sw = uiswitch(fig, 'Items', {'Off', 'On'}, 'Value', 'Off', 'ValueChangedFcn', @(s, e) assignin('base', 'sw_ev', e.Value));
            mrUiFire(sw, 'toggle');

            % 6. CloseRequestFcn prevents default deletion; close(fig,'force') deletes
            close_hits = 0;
            fig.CloseRequestFcn = 'close_hits = close_hits + 1;';
            mrUiFire(fig, 'close');
            still_valid = isvalid(fig);
            close(fig, 'force');
            now_invalid = ~isvalid(fig);

            % 7. Error inside callback reports warning naming callback and does NOT abort caller
            fig2 = uifigure;
            b_err = uibutton(fig2, 'ButtonPushedFcn', @(s, e) error('boom_inside_cb'));
            mrUiFire(b_err, 'press');
            after_err = 123;
            delete(fig2);

            function add_extra(src, ev, a, b)
                assignin('base', 'cell_sum', a + b);
            end
            "#,
        )
        .unwrap();

    assert_eq!(scalar_num(&engine, "h_called"), 1.0);
    assert_eq!(scalar_num(&engine, "cell_sum"), 35.0);
    assert_eq!(scalar_num(&engine, "str_hits"), 7.0);
    assert_eq!(engine.logical_variables["cb_ev"].data(), &[0, 1]);
    assert_eq!(engine.logical_variables["sb_ev"].data(), &[0, 1]);
    assert_eq!(char_var(&engine, "sw_ev"), "On");
    assert_eq!(scalar_num(&engine, "close_hits"), 1.0);
    assert!(scalar_bool(&engine, "still_valid"));
    assert!(scalar_bool(&engine, "now_invalid"));
    assert_eq!(scalar_num(&engine, "after_err"), 123.0);
    let out_joined = exec.output.join("\n");
    assert!(
        out_joined.contains("ButtonPushedFcn") && out_joined.contains("boom_inside_cb"),
        "expected callback warning naming ButtonPushedFcn, got: {out_joined}"
    );
}

#[test]
fn ui_components_validation_and_unknown_property_errors() {
    let mut e = Engine::default();
    let err_limits = e
        .execute("fig = uifigure; ef = uieditfield(fig, 'numeric', 'Limits', [0 10], 'Value', 20);")
        .unwrap_err();
    assert!(
        err_limits.contains("Limits"),
        "expected Limits error, got: {err_limits}"
    );

    let err_slider = e
        .execute("fig = uifigure; s = uislider(fig, 'Limits', [0 10]); s.Value = -5;")
        .unwrap_err();
    assert!(
        err_slider.contains("Limits"),
        "expected slider Limits error, got: {err_slider}"
    );

    let err_items = e
        .execute("fig = uifigure; dd = uidropdown(fig, 'Items', {'A', 'B'}, 'Value', 'C');")
        .unwrap_err();
    assert!(
        err_items.contains("Items"),
        "expected Items error, got: {err_items}"
    );

    let err_prop = e
        .execute("fig = uifigure; b = uibutton(fig); b.NoSuchProp;")
        .unwrap_err();
    assert!(
        err_prop.contains("Unrecognized property 'NoSuchProp' for class 'Button'"),
        "expected MATLAB unrecognized property error, got: {err_prop}"
    );

    let err_set_prop = e
        .execute("fig = uifigure; b = uibutton(fig); set(b, 'Bogus', 1);")
        .unwrap_err();
    assert!(
        err_set_prop.contains("Unrecognized property 'Bogus' for class 'Button'"),
        "expected MATLAB unrecognized property error on set, got: {err_set_prop}"
    );
}

#[test]
fn ui_components_done_check_runs_via_matrust_batch_cli() {
    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("ui_done_check.m");
    let script = format!(
        "{DONE_CHECK_SCRIPT}\nfprintf('OK:%s:%.0f:%.0f:%.0f:%.0f:%d\\n', lbl_after_press, ef_val_after, tbl_val_after, tbl_idx(1), tbl_idx(2), all_invalid);\n"
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
        stdout.contains("OK:pushed:ButtonPushed:42:99:2:1:1"),
        "unexpected batch stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn ui_components_uiaxes_plotting_and_layout() {
    let e = run(r#"
        fig = uifigure('Name', 'PlotApp');
        g = uigridlayout(fig, [2 2]);
        ax = uiaxes(g);
        ax.Layout.Row = 1;
        ax.Layout.Column = [1 2];
        ax_parent_is_g = (ax.Parent == g);
        ax_row = ax.Layout.Row;
        ax_col = ax.Layout.Column;
        ax_type = ax.Type;

        h1 = plot(ax, [1 2 3], [1 4 9], 'DisplayName', 'sq');
        h1_parent_is_ax = (h1.Parent == ax);
        n_one = numel(ax.Children);

        hold(ax, 'on');
        h2 = plot(ax, [1 2 3], [2 3 4], 'DisplayName', 'lin');
        n_two = numel(ax.Children);
        h2_parent_is_ax = (h2.Parent == ax);

        title(ax, 'Demo Title');
        xlabel(ax, 'X Axis');
        ylabel(ax, 'Y Axis');
        xlim(ax, [0 5]);
        ylim(ax, [0 10]);
        lg = legend(ax, 'sq', 'lin');
        lg_valid = isvalid(lg);
        xl = xlim(ax);
        yl = ylim(ax);
        t_str = ax.Title.String;
        x_str = ax.XLabel.String;
        y_str = ax.YLabel.String;

        cla(ax);
        n_after_cla = numel(ax.Children);

        % scatter, imagesc, surf into uiaxes
        sc = scatter(ax, [1 2 3], [3 2 1]);
        sc_parent_ok = (sc.Parent == ax);

        p = uipanel(g, 'Title', 'Img');
        ax2 = uiaxes(p, 'Position', [10 10 200 180]);
        ax2_parent_is_p = (ax2.Parent == p);
        ax2_pos = ax2.Position;
        im = imagesc(ax2, [1 2; 3 4]);
        im_parent_ok = (im.Parent == ax2);

        ax3 = uiaxes(g);
        [X, Y] = meshgrid(1:3, 1:3);
        sf = surf(ax3, X, Y, X + Y);
        sf_parent_ok = (sf.Parent == ax3);

        delete(fig);
        ax_valid_after = isvalid(ax);
        sc_valid_after = isvalid(sc);
        ax2_valid_after = isvalid(ax2);
    "#);

    assert!(scalar_bool(&e, "ax_parent_is_g"));
    assert_eq!(scalar_num(&e, "ax_row"), 1.0);
    assert_eq!(e.variables["ax_col"].data(), &[1.0, 2.0]);
    assert_eq!(char_var(&e, "ax_type"), "axes");
    assert!(scalar_bool(&e, "h1_parent_is_ax"));
    assert_eq!(scalar_num(&e, "n_one"), 1.0);
    assert_eq!(scalar_num(&e, "n_two"), 2.0);
    assert!(scalar_bool(&e, "h2_parent_is_ax"));
    assert!(scalar_bool(&e, "lg_valid"));
    assert_eq!(e.variables["xl"].data(), &[0.0, 5.0]);
    assert_eq!(e.variables["yl"].data(), &[0.0, 10.0]);
    assert_eq!(char_var(&e, "t_str"), "Demo Title");
    assert_eq!(char_var(&e, "x_str"), "X Axis");
    assert_eq!(char_var(&e, "y_str"), "Y Axis");
    assert_eq!(scalar_num(&e, "n_after_cla"), 0.0);
    assert!(scalar_bool(&e, "sc_parent_ok"));
    assert!(scalar_bool(&e, "ax2_parent_is_p"));
    assert_eq!(e.variables["ax2_pos"].data(), &[10.0, 10.0, 200.0, 180.0]);
    assert!(scalar_bool(&e, "im_parent_ok"));
    assert!(scalar_bool(&e, "sf_parent_ok"));
    assert!(!scalar_bool(&e, "ax_valid_after"));
    assert!(!scalar_bool(&e, "sc_valid_after"));
    assert!(!scalar_bool(&e, "ax2_valid_after"));
}

#[test]
fn ui_components_uialert_uiconfirm_uiprogressdlg_headless() {
    let mut engine = Engine::default();
    let exec = engine
        .execute(
            r#"
            fig = uifigure('Name', 'DlgApp');

            % uialert in -batch prints nothing, does not block, calls CloseFcn
            alert_closed = 0;
            uialert(fig, 'Disk full', 'Warning', 'Icon', 'warning', ...
                'CloseFcn', @(s, e) assignin('base', 'alert_closed', 1));

            % uiconfirm in -batch returns DefaultOption text (default 1) without blocking
            sel_default = uiconfirm(fig, 'Proceed?', 'Confirm', 'Options', {'Save', 'Discard', 'Cancel'});
            confirm_ev_opt = '';
            confirm_ev_idx = 0;
            sel_second = uiconfirm(fig, 'Choose one', 'Pick', ...
                'Options', {'Alpha', 'Beta', 'Gamma'}, ...
                'DefaultOption', 2, ...
                'CancelOption', 3, ...
                'CloseFcn', @on_confirm_close);

            % uiprogressdlg Value/Message/Title/Indeterminate/Cancelable/CancelRequested and close(d)
            d = uiprogressdlg(fig, 'Title', 'Running', 'Message', 'Step 1', ...
                'Value', 0.25, 'Indeterminate', 'off', 'Cancelable', 'on');
            d_val1 = d.Value;
            d_msg1 = d.Message;
            d_title1 = d.Title;
            d_indet = d.Indeterminate;
            d_canc = d.Cancelable;
            d_req1 = d.CancelRequested;

            d.Value = 0.8;
            d.Message = 'Almost done';
            d_val2 = d.Value;
            d_msg2 = d.Message;

            mrUiFire(d, 'cancel');
            d_req2 = d.CancelRequested;

            close(d);
            d_valid_after = isvalid(d);
            delete(fig);

            function on_confirm_close(~, ev)
                assignin('base', 'confirm_ev_opt', ev.SelectedOption);
                assignin('base', 'confirm_ev_idx', ev.SelectedOptionIndex);
            end
            "#,
        )
        .unwrap();

    assert!(
        exec.output.is_empty(),
        "uialert/uiconfirm/uiprogressdlg should print nothing in -batch, got: {:?}",
        exec.output
    );
    assert_eq!(scalar_num(&engine, "alert_closed"), 1.0);
    assert_eq!(char_var(&engine, "sel_default"), "Save");
    assert_eq!(char_var(&engine, "sel_second"), "Beta");
    assert_eq!(char_var(&engine, "confirm_ev_opt"), "Beta");
    assert_eq!(scalar_num(&engine, "confirm_ev_idx"), 2.0);
    assert_eq!(scalar_num(&engine, "d_val1"), 0.25);
    assert_eq!(char_var(&engine, "d_msg1"), "Step 1");
    assert_eq!(char_var(&engine, "d_title1"), "Running");
    assert_eq!(char_var(&engine, "d_indet"), "off");
    assert_eq!(char_var(&engine, "d_canc"), "on");
    assert!(!scalar_bool(&engine, "d_req1"));
    assert_eq!(scalar_num(&engine, "d_val2"), 0.8);
    assert_eq!(char_var(&engine, "d_msg2"), "Almost done");
    assert!(scalar_bool(&engine, "d_req2"));
    assert!(!scalar_bool(&engine, "d_valid_after"));
}

#[test]
fn ui_components_anonymous_callback_zero_output_function_and_demo() {
    let mut engine = Engine::default();
    let exec = engine
        .execute(
            r#"
            ran_count = 0;
            fig = uifigure('Name', 'ZeroOutCb');
            btn = uibutton(fig, 'Text', 'Go');
            btn.ButtonPushedFcn = @(src, ev) zeroOutHelper(src);
            mrUiFire(btn, 'press');
            delete(fig);

            function zeroOutHelper(~)
                assignin('base', 'ran_count', evalin('base', 'ran_count') + 1);
            end
            "#,
        )
        .unwrap();
    assert!(
        !exec
            .output
            .iter()
            .any(|line| line.contains("provides at most 0 outputs")
                || line.contains("Error executing")),
        "zero-output anonymous callback must not error: {:?}",
        exec.output
    );
    assert_eq!(scalar_num(&engine, "ran_count"), 1.0);

    // Also verify the demo script's "Confirm & Progress" button via mrUiFire in -batch.
    let demo_src = format!(
        "{}\nmrUiFire(dlgBtn, 'press');\nfinal_status = statusLbl.Text;\n",
        include_str!("../assets/examples/ui_demo.m")
    );
    let mut demo_engine = Engine::default();
    let demo_exec = demo_engine.execute(&demo_src).unwrap();
    assert!(
        !demo_exec
            .output
            .iter()
            .any(|line| line.contains("provides at most 0 outputs")
                || line.contains("Error executing")),
        "ui_demo mrUiFire(dlgBtn, 'press') must not error: {:?}",
        demo_exec.output
    );
    assert_eq!(char_var(&demo_engine, "final_status"), "Progress done");
}

#[test]
fn uiwait_dialog_figure_handle_falls_back_in_batch() {
    let mut engine = Engine::default();
    let exec = engine
        .execute("m=msgbox('x'); uiwait(m,0.1); w=waitbar(0); uiwait(w,0.1); disp('ok');")
        .unwrap();
    let text = exec.output.join("\n");
    assert!(text.contains("ok"), "{text}");
    assert!(
        !text.contains("no longer live"),
        "uiwait on a dialog handle must fall back, got: {text}"
    );
}

#[test]
fn uiresume_dialog_figure_handle_is_quiet_noop() {
    let mut engine = Engine::default();
    let exec = engine
        .execute("m=msgbox('x'); uiresume(m); w=waitbar(0); uiresume(w); disp('ok');")
        .unwrap();
    let text = exec.output.join("\n");
    assert!(text.contains("ok"), "{text}");
    assert!(
        !text.contains("no longer live") && !text.contains("uiresume requires"),
        "uiresume on a dialog handle must be a quiet no-op, got: {text}"
    );
}

#[test]
fn anonymous_zero_out_bare_name_returns_function_handle() {
    let mut engine = Engine::default();
    let exec = engine.execute("h=@sin; g=@() h; g()").unwrap();
    let text = exec.output.join("\n");
    assert!(
        text.contains("@sin"),
        "g() must display the handle h, got: {text:?}"
    );
    assert!(!text.contains("Not enough input arguments"), "{text}");
    let err = engine.execute("g=@() h(); g()").unwrap_err();
    assert!(
        err.contains("Not enough input arguments"),
        "a parenthesized handle call is still a call, got: {err}"
    );
}

#[test]
fn eval_anonymous_zero_out_is_a_free_function() {
    let src = include_str!("../src/engine/handles.rs");
    let decl = src
        .lines()
        .find(|line| line.contains("fn eval_anonymous_zero_out"))
        .expect("eval_anonymous_zero_out");
    assert!(
        decl.starts_with("fn eval_anonymous_zero_out"),
        "must be a free function, not an impl Engine method: {decl}"
    );
    let mut engine = Engine::default();
    let exec = engine
        .execute(
            "ran = 0; g = @() zeroOut(); g();\nfunction zeroOut, assignin('base','ran',1); end",
        )
        .unwrap();
    assert!(
        !exec
            .output
            .iter()
            .any(|line| line.contains("provides at most 0 outputs")),
        "{:?}",
        exec.output
    );
    assert_eq!(scalar_num(&engine, "ran"), 1.0);
}

#[test]
fn uiresume_no_arg_only_resumes_gcf() {
    use matrust::engine::{GraphicsFrame, UiEvent, UiWidgetSnapshot};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct TestUiHost {
        target_label: Arc<Mutex<Option<String>>>,
        events: VecDeque<UiEvent>,
    }
    impl matrust::HostIo for TestUiHost {
        fn read_line(&mut self, _prompt: &str) -> Option<String> {
            None
        }
        fn supports_ui_wait(&self) -> bool {
            true
        }
        fn draw_frame(&mut self, frame: GraphicsFrame) {
            let mut label_guard = self.target_label.lock().unwrap();
            if let Some(label) = label_guard.as_deref()
                && let Some(node) = frame.ui.nodes.values().find(
                    |n| matches!(&n.widget, UiWidgetSnapshot::Button { text } if text == label),
                )
            {
                self.events.push_back(UiEvent::Press { key: node.key });
                *label_guard = None;
            }
        }
        fn poll_ui_event(&mut self, timeout: std::time::Duration) -> Option<UiEvent> {
            if let Some(ev) = self.events.pop_front() {
                return Some(ev);
            }
            std::thread::sleep(timeout);
            None
        }
        fn try_poll_ui_event(&mut self) -> Option<UiEvent> {
            self.events.pop_front()
        }
    }

    let target_label = Arc::new(Mutex::new(Some("BareBtn".to_string())));
    let mut engine = Engine::default();
    engine.set_host_io(Some(Box::new(TestUiHost {
        target_label: target_label.clone(),
        events: VecDeque::new(),
    })));

    // Classic figures: f1 waits, f2 is gcf (not waiting). Bare `uiresume` must do nothing
    // so uiwait(f1, 0.15) runs until timeout instead of resuming immediately.
    let start = std::time::Instant::now();
    engine
        .execute(
            "bareRan = 0; \
             f1 = figure; \
             uicontrol(f1, 'Style', 'pushbutton', 'String', 'BareBtn', ...
                 'Callback', @(~,~) evalin('base', 'bareRan = 1; uiresume;')); \
             f2 = figure; \
             uiwait(f1, 0.15);",
        )
        .unwrap();
    let elapsed_non_gcf = start.elapsed();
    assert_eq!(scalar_num(&engine, "bareRan"), 1.0);
    assert!(
        elapsed_non_gcf >= std::time::Duration::from_millis(120),
        "bare uiresume must not resume f1 when f2 is gcf (elapsed={elapsed_non_gcf:?})"
    );

    // Now make f1 be gcf: bare `uiresume` MUST resume f1 promptly (well before 2s timeout).
    *target_label.lock().unwrap() = Some("BareBtn".to_string());
    let start2 = std::time::Instant::now();
    engine
        .execute("bareRan = 0; figure(f1); uiwait(f1, 2.0);")
        .unwrap();
    let elapsed_gcf = start2.elapsed();
    assert_eq!(scalar_num(&engine, "bareRan"), 1.0);
    assert!(
        elapsed_gcf < std::time::Duration::from_millis(500),
        "bare uiresume must resume gcf promptly (elapsed={elapsed_gcf:?})"
    );

    // uifigures: u1 waits, u2 is last (not waiting). Bare `uiresume` must not resume u1.
    engine.execute("close all;").unwrap();
    *target_label.lock().unwrap() = Some("UiBareBtn".to_string());
    let start3 = std::time::Instant::now();
    engine
        .execute(
            "uBareRan = 0; \
             u1 = uifigure; \
             uibutton(u1, 'Text', 'UiBareBtn', ...
                 'ButtonPushedFcn', @(~,~) evalin('base', 'uBareRan = 1; uiresume;')); \
             u2 = uifigure; \
             uiwait(u1, 0.15);",
        )
        .unwrap();
    let elapsed_u1 = start3.elapsed();
    assert_eq!(scalar_num(&engine, "uBareRan"), 1.0);
    assert!(
        elapsed_u1 >= std::time::Duration::from_millis(120),
        "bare uiresume must not resume u1 when u2 is current (elapsed={elapsed_u1:?})"
    );
}

#[test]
fn classic_figure_without_uicontrols_uialert_uiconfirm_batch_and_snapshot() {
    use matrust::engine::{DialogReply, DialogRequest, GraphicsFrame, UiEvent};
    use std::sync::{Arc, Mutex};

    // 1. -batch behavior unchanged for plain classic figure.
    let mut batch = Engine::default();
    batch
        .execute(
            "closed = 0; \
             f = figure; \
             uialert(f, 'Batch msg', 'Batch title', 'CloseFcn', @(~,~) evalin('base', 'closed = 1;')); \
             ansChoice = uiconfirm(f, 'Batch q', 'Batch c', 'Options', {'Yes', 'No'}, 'DefaultOption', 2);",
        )
        .unwrap();
    assert_eq!(scalar_num(&batch, "closed"), 1.0);
    assert_eq!(char_var(&batch, "ansChoice"), "No");

    // 2. Interactive host: plain classic figure with pending uialert appears in UiSnapshot,
    //    and closing the figure clears the engine-side pending alert entry without running CloseFcn.
    struct SnapHost {
        saw_plain_window: Arc<Mutex<bool>>,
        alert_id: Arc<Mutex<Option<u32>>>,
    }
    impl matrust::HostIo for SnapHost {
        fn read_line(&mut self, _prompt: &str) -> Option<String> {
            None
        }
        fn supports_ui_wait(&self) -> bool {
            true
        }
        fn draw_frame(&mut self, frame: GraphicsFrame) {
            if frame
                .ui
                .windows
                .iter()
                .any(|w| w.is_classic && w.title.contains("PlainSnap"))
            {
                *self.saw_plain_window.lock().unwrap() = true;
            }
        }
        fn dialog(&mut self, request: DialogRequest) -> DialogReply {
            if let DialogRequest::UiAlert { id, .. } = request {
                *self.alert_id.lock().unwrap() = Some(id);
                return DialogReply::Done;
            }
            DialogReply::Unsupported
        }
    }

    let saw_plain = Arc::new(Mutex::new(false));
    let alert_id = Arc::new(Mutex::new(None));
    let mut engine = Engine::default();
    engine.set_host_io(Some(Box::new(SnapHost {
        saw_plain_window: saw_plain.clone(),
        alert_id: alert_id.clone(),
    })));
    engine
        .execute(
            "closed = 0; \
             f = figure('Name', 'PlainSnap'); \
             uialert(f, 'Msg', 'Title', 'CloseFcn', @(~,~) evalin('base', 'closed = 1;'));",
        )
        .unwrap();
    assert!(
        *saw_plain.lock().unwrap(),
        "plain classic figure with pending uialert must appear in UiSnapshot"
    );
    let id = alert_id.lock().unwrap().expect("captured alert id");
    engine.execute("close(f);").unwrap();
    let mut exec = matrust::Execution::default();
    matrust::engine::dispatch_ui_event(&mut engine, UiEvent::AlertClosed { id }, &mut exec)
        .unwrap();
    assert_eq!(
        scalar_num(&engine, "closed"),
        0.0,
        "closing the figure must clear its engine-side pending alert without running CloseFcn"
    );
}

#[test]
fn ui_components_findobj_and_findall_tree_uiaxes_and_handle_visibility() {
    let e = run(r#"
        % Deep container tree: uifigure -> uigridlayout -> uipanel -> uitabgroup -> uitab -> uibutton
        fig = uifigure('Visible', 'off', 'Tag', 'app_fig');
        fig_hv_default = fig.HandleVisibility;
        g = uigridlayout(fig, [2 1]);
        p = uipanel(g, 'Title', 'Controls');
        tg = uitabgroup(p);
        t1 = uitab(tg, 'Title', 'Main');
        btn = uibutton(t1, 'push', 'Text', 'Run', 'Tag', 'deep_run_btn');
        btn_hv_default = btn.HandleVisibility;

        % Tag lookup inside uigridlayout > uipanel > uitabgroup > uitab
        found_from_fig = findobj(fig, 'Tag', 'deep_run_btn');
        found_from_grid = findobj(g, 'Tag', 'deep_run_btn');
        found_from_panel = findobj(p, 'Tag', 'deep_run_btn');
        found_from_tab = findobj(t1, 'Tag', 'deep_run_btn');
        tag_ok = (numel(found_from_fig) == 1) && (found_from_fig == btn) ...
            && (found_from_grid == btn) && (found_from_panel == btn) && (found_from_tab == btn);

        % uiaxes with plotted line + text + patch
        ax = uiaxes(g);
        ln = plot(ax, 1:5, (1:5).^2, 'DisplayName', 'quad');
        hold(ax, 'on');
        txt = text(ax, 2, 4, 'pt');

        % findobj from uifigure, grid, and uiaxes reaches uiaxes graphics children
        ln_from_fig = findobj(fig, 'Type', 'line');
        ln_by_name = findobj(fig, 'Type', 'line', 'DisplayName', 'quad');
        ln_from_grid = findobj(g, 'Type', 'line');
        ln_from_ax = findobj(ax, 'Type', 'line');
        txt_from_fig = findobj(fig, 'Type', 'text');
        ax_from_fig = findobj(fig, 'Type', 'axes');
        uiaxes_ok = (numel(ln_from_fig) == 1) && (ln_from_fig == ln) ...
            && (numel(ln_by_name) == 1) && (ln_by_name == ln) ...
            && (ln_from_grid == ln) && (ln_from_ax == ln) ...
            && (numel(txt_from_fig) == 1) && (txt_from_fig == txt) ...
            && (numel(ax_from_fig) == 1) && (ax_from_fig == ax);

        % -depth on uiaxes: 0 checks uiaxes itself, 1 reaches its line
        d0_ax = findobj(ax, '-depth', 0, 'Type', 'axes');
        d0_ln = findobj(ax, '-depth', 0, 'Type', 'line');
        d1_ln = findobj(ax, '-depth', 1, 'Type', 'line');
        depth_ok = (numel(d0_ax) == 1) && (d0_ax == ax) && isempty(d0_ln) && (d1_ln == ln);

        % Default uifigure has HandleVisibility 'off': root findobj misses it, findall finds it
        root_findobj_fig = findobj(0, 'Type', 'figure');
        root_findobj_ln = findobj('Type', 'line');
        root_findall_fig = findall(0, 'Type', 'figure');
        root_findall_ln = findall('Type', 'line');
        root_findall_btn = findall(groot, 'Tag', 'deep_run_btn');
        root_hidden_ok = isempty(root_findobj_fig) && isempty(root_findobj_ln) ...
            && (numel(root_findall_fig) == 1) && (root_findall_fig == fig) ...
            && (numel(root_findall_ln) == 1) && (root_findall_ln == ln) ...
            && (numel(root_findall_btn) == 1) && (root_findall_btn == btn);

        % Setting uifigure HandleVisibility 'on' makes root findobj see the tree
        fig.HandleVisibility = 'on';
        root_findobj_fig_on = findobj(0, 'Type', 'figure');
        root_findobj_ln_on = findobj('Type', 'line');
        root_on_ok = (numel(root_findobj_fig_on) == 1) && (root_findobj_fig_on == fig) ...
            && (numel(root_findobj_ln_on) == 1) && (root_findobj_ln_on == ln);

        % Setting a child's HandleVisibility 'off': findobj skips it, findall finds it
        btn.HandleVisibility = 'off';
        child_findobj_off = findobj(fig, 'Tag', 'deep_run_btn');
        child_findall_off = findall(fig, 'Tag', 'deep_run_btn');
        child_findall_hv = findall(fig, 'Type', 'uibutton', 'HandleVisibility', 'off');
        child_hv_ok = isempty(child_findobj_off) ...
            && (numel(child_findall_off) == 1) && (child_findall_off == btn) ...
            && (numel(child_findall_hv) == 1) && (child_findall_hv == btn);

        % HandleVisibility 'callback': hidden from findobj outside callback, visible inside callback
        cb_found = 0;
        btn2 = uibutton(t1, 'push', 'Tag', 'cb_btn', 'HandleVisibility', 'callback');
        btn2.ButtonPushedFcn = @(~,~) evalin('base', 'cb_found = numel(findobj(fig, ''Tag'', ''cb_btn''));');
        outside_findobj = numel(findobj(fig, 'Tag', 'cb_btn'));
        outside_findall = numel(findall(fig, 'Tag', 'cb_btn'));
        mrUiFire(btn2, 'press');
        cb_hv_ok = (outside_findobj == 0) && (outside_findall == 1) && (cb_found == 1);

        % Classic figure findobj stays unchanged alongside a default (hidden) uifigure
        fig.HandleVisibility = 'off';
        cf = figure(4);
        cln = plot(1:3, [2 4 6], 'DisplayName', 'classic');
        classic_figs = findobj('Type', 'figure');
        classic_lines = findobj(cf, 'Type', 'line');
        all_figs = findall(0, 'Type', 'figure');
        classic_ok = (numel(classic_figs) == 1) && (classic_figs == cf) ...
            && (numel(classic_lines) == 1) && (classic_lines == cln) ...
            && (numel(all_figs) == 2);

        close(fig);
        close(cf);
    "#);

    assert_eq!(char_var(&e, "fig_hv_default"), "off");
    assert_eq!(char_var(&e, "btn_hv_default"), "on");
    assert!(scalar_bool(&e, "tag_ok"));
    assert!(scalar_bool(&e, "uiaxes_ok"));
    assert!(scalar_bool(&e, "depth_ok"));
    assert!(scalar_bool(&e, "root_hidden_ok"));
    assert!(scalar_bool(&e, "root_on_ok"));
    assert!(scalar_bool(&e, "child_hv_ok"));
    assert!(scalar_bool(&e, "cb_hv_ok"));
    assert!(scalar_bool(&e, "classic_ok"));
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

// le9a review item 1: xline/yline/rectangle/patch/text/scatter(ax,...) draw
// into a uiaxes; ax stays valid and its plot is kept.
#[test]
fn ui_components_uiaxes_shapes_draw_into_uiaxes_batch() {
    let out = le9a_batch(
        r#"
f = uifigure; ax = uiaxes(f); plot(ax, 1:3); hold(ax, 'on');
h1 = xline(ax, 2); h2 = yline(ax, 1);
h3 = rectangle(ax, 'Position', [0 0 1 1]);
h4 = patch(ax, [0 1 1], [0 0 1], [1 0 0]);
h5 = text(ax, 1, 1, 'a');
h6 = scatter(ax, 1:3, 1:3);
title(ax, 't');
ok = isvalid(ax) && all([isvalid(h1) isvalid(h2) isvalid(h3) isvalid(h4) isvalid(h5) isvalid(h6)]);
fprintf('SHAPES:%d:%d:%g:%g\n', ok, numel(ax.Children), h1.Value, h2.Value);
"#,
    );
    // plot line, patch, text and scatter are Children; constant lines and the
    // rectangle are axes extras.
    assert!(out.contains("SHAPES:1:4:2:1"), "{out}");
}

#[test]
fn ui_components_uiaxes_shapes_keep_ui_windows() {
    let e = run(
        "f = uifigure; ax = uiaxes(f); plot(ax, 1:3); xline(ax, 2); yline(ax, 1); \
                 rectangle(ax, 'Position', [0 0 1 1]); patch(ax, [0 1 1], [0 0 1], [1 0 0]);",
    );
    let snap = matrust::engine::capture_ui_snapshot(&e);
    assert_eq!(snap.windows.len(), 1);
    assert!(
        e.figure_ids().is_empty(),
        "no classic figure: {:?}",
        e.figure_ids()
    );
}

// le9a review item 2: anonymous callbacks run their body as a statement.
#[test]
fn ui_components_anonymous_callback_runs_as_statement_batch() {
    let out = le9a_batch(
        r#"
f = uifigure('CloseRequestFcn', @(src, event) delete(src)); close(f);
g = uifigure; b = uibutton(g, 'ButtonPushedFcn', @(s, e) delete(s)); mrUiFire(b, 'press');
k = uifigure; b2 = uibutton(k, 'ButtonPushedFcn', @(s, e) disp('CB-DISP')); mrUiFire(b2, 'press');
q = @() 3; x = q();
fprintf('ANON:%d:%d:%d\n', isvalid(f), isvalid(b), x);
"#,
    );
    assert!(out.contains("ANON:0:0:3"), "{out}");
    assert!(out.contains("CB-DISP"), "{out}");
    assert!(!out.contains("Warning"), "{out}");
}

// le9a review item 3: 'Parent' name-value works like the positional parent.
#[test]
fn ui_components_parent_name_value_batch() {
    let out = le9a_batch(
        r#"
f = uifigure; g = uigridlayout(f, [2 2]);
a = uibutton('Parent', g); b = uilabel('Parent', g);
ax = uiaxes('Parent', g); tg = uitabgroup('Parent', g);
t = uitab('Parent', tg); p = uipanel('Parent', t);
tb = uitable('Parent', p); g2 = uigridlayout([1 2], 'Parent', p);
fprintf('PARENT:%d%d:%d%d:%d%d:%d%d:%d:%d:%d\n', a.Layout.Row, a.Layout.Column, ...
    b.Layout.Row, b.Layout.Column, ax.Layout.Row, ax.Layout.Column, ...
    tg.Layout.Row, tg.Layout.Column, a.Parent == g, tb.Parent == p, g2.Parent == p);
"#,
    );
    assert!(out.contains("PARENT:11:12:21:22:1:1:1"), "{out}");
}

#[test]
fn ui_components_parent_name_value_makes_no_stray_figure() {
    let e = run(
        "f = uifigure; g = uigridlayout(f, [2 2]); a = uibutton('Parent', g); \
                 b = uilabel('Parent', g); ax = uiaxes('Parent', g); s = uislider('Parent', g);",
    );
    assert_eq!(matrust::engine::capture_ui_snapshot(&e).windows.len(), 1);
}

// line(ax, ...) and area(ax, ...) on a uiaxes draw into it instead of trying
// to select the uiaxes host as a classic figure number.
#[test]
fn ui_components_uiaxes_line_and_area_batch() {
    let out = le9a_batch(
        r#"
f = uifigure; ax = uiaxes(f); plot(ax, 1:3); hold(ax, 'on');
h1 = line(ax, [1 3], [3 1]);
h2 = area(ax, 1:3, [1 2 1]);
ok = isvalid(ax) && isvalid(h1) && isvalid(h2) && (h1.Parent == ax);
fprintf('LINEAREA:%d:%d:%d\n', ok, numel(ax.Children), numel(findall(0, 'Type', 'figure')));
"#,
    );
    // 4 children, same as a classic axes: the area baseline is listed too.
    assert!(out.contains("LINEAREA:1:4:1"), "{out}");
}

// Chart objects from plots_2d_more drawn on a uiaxes support property get/set
// and keep the classic gcf unchanged.
#[test]
fn ui_components_uiaxes_charts_properties_batch() {
    let out = le9a_batch(
        r#"
f0 = figure;
f = uifigure; ax = uiaxes(f); hold(ax, 'on');
ha = area(ax, 1:3, [1 2 1]); ha.FaceAlpha = 0.5;
hh = histogram(ax, 1:10, 4);
he = errorbar(ax, 1:3, [1 2 1], [0.1 0.2 0.1]);
hf = fill(ax, [0 1 1], [0 0 1], 'r');
p_area = (ha.Parent == ax);
fa = ha.FaceAlpha;
p_hist = (hh.Parent == ax);
nb = hh.NumBins;
p_err = (he.Parent == ax);
p_fill = (hf.Parent == ax);
hold(ax, 'off');
hp = pie(ax, [1 2 3]);
p_pie = (hp(1).Parent == ax) && (hp(3).Parent == ax) && (hp(5).Parent == ax);
gcf_ok = (gcf == f0);
fprintf('UICHARTS:%d:%d:%d:%d:%d:%.1f:%d:%d\n', p_area, p_hist, p_err, p_fill, p_pie, fa, nb, gcf_ok);
"#,
    );
    assert!(out.contains("UICHARTS:1:1:1:1:1:0.5:4:1"), "{out}");
}

// Drawing a chart into a uiaxes keeps the chart records of the classic
// current figure (pruning sees the held figure), and contour(ax, ...) draws
// into the uiaxes like area(ax, ...).
#[test]
fn ui_components_uiaxes_chart_keeps_classic_charts_and_contour_batch() {
    let out = le9a_batch(
        r#"
f0 = figure('Visible', 'off'); hc = histogram(randn(1, 50), 7);
f = uifigure; ax = uiaxes(f);
ha = area(ax, 1:3, [1 2 1]); area_ok = (ha.Parent == ax);
[c, hk] = contour(ax, peaks(10), 3);
ok = area_ok && isvalid(hc) && isvalid(hk) && (gcf == f0) && (hk.Parent == ax);
fprintf('KEEP:%d:%d:%d\n', ok, hc.NumBins, numel(hk.LevelList));
"#,
    );
    assert!(out.contains("KEEP:1:7:3"), "{out}");
}
