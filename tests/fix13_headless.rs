use matrust::Engine;

#[test]
fn headless_dialog_lifecycle() {
    let mut e = Engine::default();
    e.execute("p=waitbar(0,'x','Name','Progress'); tagok=double(strcmp(get(p,'Tag'),'TMWWaitbar')); titleok=double(strcmp(get(p,'Name'),'Progress')); hidden=double(strcmp(get(p,'Visible'),'off')); delete(p);").unwrap();
    for name in ["tagok", "titleok", "hidden"] {
        assert_eq!(e.variables[name].data(), [1.]);
    }
    let result = e.execute("h=waitbar(0,'x'); for i=1:3, waitbar(i/3,h); end; uiwait(h); waitfor(h); close(h); valid=double(isvalid(h));").unwrap();
    assert!(result.output.is_empty());
    assert_eq!(e.variables["valid"].data(), [0.]);
    let result = e.execute("a=msgbox('x'); b=warndlg('x'); c=errordlg('x'); d=helpdlg('x'); alive=double(isvalid(a)); delete(a); close(b); close(c); close(d);").unwrap();
    assert!(result.output.is_empty());
    assert_eq!(e.variables["alive"].data(), [1.]);
    assert!(
        e.execute("inputdlg('x');")
            .unwrap_err()
            .contains("dialogs need the desktop app")
    );
}

#[test]
fn headless_dialogs_leave_the_current_plot_and_its_exports_intact() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("orphan = waitbar(0); close(orphan);").unwrap();
    assert!(e.figures().next().is_none());

    e.execute(
        "figure(7); plot(1:3); f = gcf; ax = gca;
         h = waitbar(0); m = msgbox('Done');
         same = double(isequal(gcf,f) && isequal(gca,ax));
         plot(4:6); saveas(gcf,'plot.svg'); ax = gca; close(h); close(m,'force');
         kept = double(isvalid(f) && isequal(gcf,f) && isequal(gca,ax));",
    )
    .unwrap();
    assert_eq!(e.variables["same"].data(), [1.]);
    assert_eq!(e.variables["kept"].data(), [1.]);
    assert_eq!(e.figure_ids(), [7]);
    let series = &e.figure.as_ref().unwrap().axes.series;
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].points.get(0), Some([1., 4.]));
    assert!(
        std::fs::read_to_string(dir.path().join("plot.svg"))
            .unwrap()
            .contains("<path")
    );

    let result = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .current_dir(dir.path())
        .args([
            "-batch",
            "plot(1:3); h=waitbar(0); m=msgbox('Done');",
            "--svg",
            "batch.svg",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        std::fs::read_to_string(dir.path().join("batch.svg"))
            .unwrap()
            .contains("<path")
    );
}

#[test]
fn headless_dialogs_do_not_consume_plot_figure_slots() {
    let mut e = Engine::default();
    e.execute(
        "for k=1:20, warndlg('x'); end
         for k=1:16, figure(k); end
         for k=1:20, warndlg('x'); end
         h = warndlg('x'); alive = double(isvalid(h));",
    )
    .unwrap();
    assert_eq!(e.figure_ids(), (1..=16).collect::<Vec<_>>());
    assert_eq!(e.variables["alive"].data(), [1.]);
    e.execute("close all hidden; dead = double(~isvalid(h));")
        .unwrap();
    assert_eq!(e.variables["dead"].data(), [1.]);
}

#[test]
fn headless_dialog_figure_properties_are_hidden_in_host_snapshots() {
    let mut e = Engine::default();
    e.execute(
        "h = waitbar(0,'x','Name','Progress'); m = msgbox('x','Done');
         waitbar_ok = double(strcmp(h.Visible,'off') && strcmp(h.Tag,'TMWWaitbar') && strcmp(h.Name,'Progress'));
         message_ok = double(strcmp(m.Visible,'off') && strcmp(m.Tag,'Msgbox_Done') && strcmp(m.Name,'Done'));
         get_ok = double(strcmp(get(h,'Visible'),h.Visible) && strcmp(get(h,'Tag'),h.Tag) && strcmp(get(h,'Name'),h.Name));
         typed = double(strcmp(h.Type,'figure') && isvalid(h) && isvalid(m));",
    )
    .unwrap();
    for name in ["waitbar_ok", "message_ok", "get_ok", "typed"] {
        assert_eq!(e.variables[name].data(), [1.], "{name}");
    }
    assert!(e.figure.is_none());
    assert!(e.figures().next().is_none());
}

#[test]
fn anonymous_varargin_makes_the_declared_input_count_negative() {
    let mut e = Engine::default();
    e.execute("a=nargin(@(varargin) 1); b=nargin(@(x,varargin) x); c=nargout(@(varargin) 1);")
        .unwrap();
    assert_eq!(e.variables["a"].data(), [-1.]);
    assert_eq!(e.variables["b"].data(), [-2.]);
    assert_eq!(e.variables["c"].data(), [1.]);
}

#[test]
fn missing_function_counts_report_their_own_error() {
    let mut e = Engine::default();
    for call in [
        "nargin('missing_fix13_function')",
        "nargout('missing_fix13_function')",
    ] {
        e.execute(&format!(
            "caught=0; try; {call}; catch err;
             caught=double(strcmp(err.identifier,'MATLAB:narginout:functionDoesnotExist') && ...
                           strcmp(err.message,'Function missing_fix13_function does not exist.'));
             end"
        ))
        .unwrap();
        assert_eq!(e.variables["caught"].data(), [1.], "{call}");
    }
}

#[test]
fn declared_function_counts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("f.m"),
        "function [a,b]=f(x,y,z)\na=x; b=y;\nend\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("outer.m"), "function y=outer()\ny=nargin('localf')+nargin(@localf);\nend\nfunction z=localf(a,b)\nz=a+b;\nend\n").unwrap();
    std::fs::write(
        dir.path().join("v.m"),
        "function [a,varargout]=v(x,varargin)\na=x;\nend\n",
    )
    .unwrap();
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    e.execute("a=nargin('f'); b=nargout('f'); c=nargin('v'); d=nargout('v'); e=nargin(@(a,b) a+b); g=nargin(@f); local=outer();").unwrap();
    for (name, expected) in [
        ("a", 3.),
        ("b", 2.),
        ("c", -2.),
        ("d", -2.),
        ("e", 2.),
        ("g", 3.),
        ("local", 4.),
    ] {
        assert_eq!(e.variables[name].data(), [expected]);
    }
    assert!(
        e.execute("nargin('polyfit')")
            .unwrap_err()
            .contains("nargin is not known for builtin polyfit")
    );
}

#[test]
fn review_4_hidden_dialog_children() {
    let mut e = Engine::default();
    e.execute("h=waitbar(0); m=msgbox('x'); a=get(0,'Children'); set(0,'ShowHiddenHandles','on'); b=get(0,'Children'); offcount=double(isempty(a)); oncount=double(numel(b)==2 && isvalid(b(1)) && isvalid(b(2)));").unwrap();
    for n in ["offcount", "oncount"] {
        assert_eq!(e.variables[n].data(), [1.]);
    }
}
#[test]
fn review_5_close_all_hidden_dialogs() {
    let mut e = Engine::default();
    e.execute("h=waitbar(0); close all; kept=double(isvalid(h)); close all hidden; dead=double(~isvalid(h)); m=msgbox('x'); close all force; forced=double(~isvalid(m));").unwrap();
    for n in ["kept", "dead", "forced"] {
        assert_eq!(e.variables[n].data(), [1.]);
    }
}
#[test]
fn review_6_numeric_and_typed_figure_validity() {
    let mut e = Engine::default();
    e.execute("figure(1); a=double(ishghandle(1) && ishandle(1)); h=waitbar(0); b=double(ishghandle(h) && ishandle(h)); close(h); c=double(~ishghandle(h) && ~ishandle(h)); close(1); d=double(~ishghandle(1) && ~ishandle(1));").unwrap();
    for n in ["a", "b", "c", "d"] {
        assert_eq!(e.variables[n].data(), [1.]);
    }
}
#[test]
fn review_7_delete_indexed_waitbar() {
    let mut e = Engine::default();
    e.execute("h=waitbar(0); delete(h(1)); dead=double(~isvalid(h));")
        .unwrap();
    assert_eq!(e.variables["dead"].data(), [1.]);
}
#[test]
fn review_8_dialog_figure_properties() {
    let mut e = Engine::default();
    e.execute("set(0,'DefaultFigureColor',[.2 .3 .4]); h=waitbar(0); a=double(isequal(get(h,'Color'),[.2 .3 .4])); set(h,'Color',[1 0 0]); b=double(isequal(get(h,'Color'),[1 0 0])); m=msgbox('x'); c=double(isequal(get(m,'Color'),[.2 .3 .4])); w=waitbar(0,'x','Color',[0 1 0]); d=double(isequal(get(w,'Color'),[0 1 0]));").unwrap();
    for n in ["a", "b", "c", "d"] {
        assert_eq!(e.variables[n].data(), [1.]);
    }
}
#[test]
fn review_9_waitbar_rejects_message_handle() {
    let mut e = Engine::default();
    e.execute("m=msgbox('x','Original');").unwrap();
    let err = e.execute("waitbar(.5,m);").unwrap_err();
    assert!(err.contains("Invalid waitbar handle"), "{err}");
    e.execute("ok=double(strcmp(get(m,'Name'),'Original'));")
        .unwrap();
    assert_eq!(e.variables["ok"].data(), [1.]);
}
#[test]
fn review_10_listdlg_two_outputs() {
    let mut e = Engine::default();
    let err = e
        .execute("[i,ok]=listdlg('ListString',{'a'});")
        .unwrap_err();
    assert!(err.contains("dialogs need the desktop app"), "{err}");
}

#[test]
fn fix15_listdlg_validates_and_limits_expanded_rows_before_opening() {
    let mut e = Engine::default();
    let valid = e
        .execute(r"[i,ok]=listdlg('ListString',{sprintf('a\nb')},'InitialValue',2);")
        .unwrap_err();
    assert!(valid.contains("dialogs need the desktop app"), "{valid}");

    let invalid = e
        .execute(r"[i,ok]=listdlg('ListString',{sprintf('a\nb')},'InitialValue',3);")
        .unwrap_err();
    assert!(invalid.contains("integers from 1 to 2"), "{invalid}");

    let too_many = e
        .execute(r"[i,ok]=listdlg('ListString',{repmat(sprintf('a\n'),1,10000)});")
        .unwrap_err();
    assert!(
        too_many.contains("ListString exceeds 10000 items"),
        "{too_many}"
    );
}

#[test]
fn review_batch_dialog_contracts() {
    for code in [
        "h=waitbar(0); m=msgbox('x'); assert(isempty(get(0,'Children'))); set(0,'ShowHiddenHandles','on'); assert(numel(get(0,'Children'))==2);",
        "h=waitbar(0); close all; assert(isvalid(h)); close all hidden; assert(~isvalid(h)); m=msgbox('x'); close all force; assert(~isvalid(m));",
        "figure(1); assert(ishghandle(1) && ishandle(1)); close(1); assert(~ishghandle(1));",
        "h=waitbar(0); delete(h(1)); assert(~isvalid(h));",
        "set(0,'DefaultFigureColor',[.2 .3 .4]); h=waitbar(0); assert(isequal(get(h,'Color'),[.2 .3 .4])); set(h,'Color',[1 0 0]); assert(isequal(get(h,'Color'),[1 0 0])); w=waitbar(0,'x','Color',[0 1 0]); assert(isequal(get(w,'Color'),[0 1 0]));",
        "m=msgbox('x','Original'); try; waitbar(.5,m); catch; end; assert(strcmp(get(m,'Name'),'Original'));",
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
            .args(["-batch", code])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{code}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(["-batch", "[i,ok]=listdlg('ListString',{'a'});"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("dialogs need the desktop app"));
}

fn round_1_batch(code: &str) -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .args(["-batch", code])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{code}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn round_1_waitbar_accepts_extra_figure_options() {
    round_1_batch(
        "h=waitbar(0,'x','CreateCancelBtn','disp(1)','WindowStyle','modal', ...
                   'Position',[10 20 300 80],'Units','pixels','Name','A');
         assert(strcmp(get(h,'Name'),'A'));
         waitbar(.5,h,'y','Units','normalized','Name','B');
         assert(isvalid(h) && strcmp(get(h,'Name'),'B'));
         rejected=0;
         try; set(h,'NoSuchDialogProperty',1); catch; rejected=1; end;
         assert(rejected==1);",
    );
}

#[test]
fn round_1_dialog_properties_accept_scalar_text() {
    round_1_batch(
        r#"h=waitbar(0,'x',"Name","Initial","Color",[0 1 0]);
           p={'Name'};
           assert(strcmp(get(h,"Name"),'Initial'));
           assert(strcmp(get(h,p{1}),'Initial'));
           assert(isequal(get(h,'Color'),[0 1 0]));
           set(h,"Color",[1 0 0],"Name","Updated","Theme","dark");
           assert(isequal(get(h,"Color"),[1 0 0]));
           assert(strcmp(get(h,"Name"),'Updated') && strcmp(get(h,'Theme'),'dark'));"#,
    );
}

#[test]
fn round_1_invalid_waitbar_creation_leaves_no_child() {
    round_1_batch(
        "set(0,'ShowHiddenHandles','on');
         rejected=0;
         try; h=waitbar(0,'x','Color','nocolor'); catch; rejected=1; end;
         assert(rejected==1);
         assert(numel(get(0,'Children'))==0);",
    );
}

#[test]
fn round_1_waitbar_update_applies_color() {
    round_1_batch(
        "h=waitbar(0,'x','Color',[0 1 0]);
         assert(isequal(get(h,'Color'),[0 1 0]));
         again=waitbar(.5,h,'y','Color',[1 0 0]);
         assert(isequal(again,h) && isequal(get(h,'Color'),[1 0 0]));",
    );
}

#[test]
fn round_1_waitbar_update_preserves_name() {
    round_1_batch(
        "h=waitbar(0,'x','Name','A');
         set(h,'Name','B');
         waitbar(.5,h);
         assert(strcmp(get(h,'Name'),'B'));
         waitbar(.7,h,'y');
         assert(strcmp(get(h,'Name'),'B'));
         waitbar(.8,h,'Name','C');
         assert(strcmp(get(h,'Name'),'C'));",
    );
}

#[test]
fn round_1_headless_dialogs_are_invisible() {
    round_1_batch(
        "h=waitbar(0); m=msgbox('x');
         assert(strcmp(get(h,'Visible'),'off') && strcmp(get(m,'Visible'),'off'));",
    );
}

#[test]
fn round_1_indexed_close_and_delete_evaluate_once() {
    let stdout = round_1_batch(
        r"figure(5); figure(6); x=[5 6];
          close(x(fprintf('E\n')));
          assert(ishghandle(5) && ~ishghandle(6));
          h=plot(1:3); delete(h(fprintf('D')));
          assert(~ishghandle(h)); fprintf('\n');",
    );
    assert_eq!(stdout.lines().collect::<Vec<_>>(), ["E", "D"]);
}
