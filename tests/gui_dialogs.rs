//! Dialog builtins through a scripted dialog host: return shapes follow the
//! published MATLAB and Octave documentation (file name + folder with a
//! trailing separator + filter index, 0 on cancel; inputdlg cell column,
//! {} on cancel; questdlg button text, '' when closed; msgbox/waitbar
//! handles). No window opens.
use matrust::engine::{DialogReply, DialogRequest, MessageIcon};
use matrust::{Engine, HostIo};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Script {
    replies: Arc<Mutex<VecDeque<DialogReply>>>,
    requests: Arc<Mutex<Vec<DialogRequest>>>,
}
struct ScriptedHost(Script);
impl HostIo for ScriptedHost {
    fn read_line(&mut self, _prompt: &str) -> Option<String> {
        None
    }
    fn dialog(&mut self, request: DialogRequest) -> DialogReply {
        let blocks = request.blocks();
        self.0.requests.lock().unwrap().push(request);
        if !blocks {
            return DialogReply::Done;
        }
        self.0
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted reply")
    }
}

fn engine(replies: Vec<DialogReply>) -> (Engine, Script) {
    let script = Script::default();
    script.replies.lock().unwrap().extend(replies);
    let mut e = Engine::default();
    e.set_host_io(Some(Box::new(ScriptedHost(script.clone()))));
    (e, script)
}
fn check(e: &mut Engine, expr: &str) {
    e.execute(&format!("t__ = {expr}; ok__ = double(all(t__(:)));"))
        .unwrap_or_else(|err| panic!("{expr}: {err}"));
    assert_eq!(e.variables["ok__"].data(), [1.], "{expr}");
}
fn sep() -> &'static str {
    if cfg!(windows) { "\\" } else { "/" }
}
fn files(folder: &str, names: &[&str], filter: usize) -> DialogReply {
    DialogReply::Files {
        folder: folder.into(),
        names: names.iter().map(|n| n.to_string()).collect(),
        filter,
    }
}

#[test]
fn review_1111_unmatched_filter_index_is_zero_with_the_file() {
    // MathWorks uigetfile: filter index 0 means no filter was selected
    // (https://www.mathworks.com/help/matlab/ref/uigetfile.html). Reference
    // value: k == 0 while file is out.md and path is /tmp with a separator.
    let (mut e, _) = engine(vec![files("/tmp", &["out.md"], usize::MAX)]);
    e.execute("[f, p, k] = uiputfile({'*.csv','CSV';'*.txt','Text'},'Save','out.md');")
        .unwrap();
    check(&mut e, "strcmp(f, 'out.md')");
    check(&mut e, &format!("strcmp(p, '/tmp{}')", sep()));
    check(&mut e, "k == 0");
}

#[test]
fn uigetfile_returns_name_folder_and_filter_index() {
    let (mut e, script) = engine(vec![files("/data", &["a.csv"], 1)]);
    e.execute("[f, p, k] = uigetfile({'*.m', 'Scripts'; '*.csv', 'CSV files'}, 'Pick data');")
        .unwrap();
    check(&mut e, "strcmp(f, 'a.csv')");
    check(&mut e, &format!("strcmp(p, '/data{}')", sep()));
    check(&mut e, "k == 2");
    let requests = script.requests.lock().unwrap();
    let DialogRequest::OpenFile {
        title,
        filters,
        multiple,
        ..
    } = &requests[0]
    else {
        panic!("{requests:?}")
    };
    assert_eq!(title, "Pick data");
    assert_eq!(filters.len(), 2);
    assert_eq!(filters[1].patterns, ["*.csv"]);
    assert_eq!(filters[1].label, "CSV files");
    assert!(!multiple);
}

#[test]
fn uigetfile_cancel_gives_zeros_and_multiselect_gives_a_cell() {
    let (mut e, script) = engine(vec![
        DialogReply::Cancelled,
        files("/d", &["x.txt", "y.txt"], 0),
        files("/d", &["only.txt"], 0),
    ]);
    e.execute("[f, p, k] = uigetfile('*.txt');").unwrap();
    check(&mut e, "isequal(f, 0) && isequal(p, 0) && isequal(k, 0)");
    e.execute("[f, p] = uigetfile('*.txt', 'Pick', 'MultiSelect', 'on');")
        .unwrap();
    check(&mut e, "iscell(f) && isequal(size(f), [1 2])");
    check(&mut e, "strcmp(f{2}, 'y.txt')");
    // One file picked with MultiSelect on: a char vector, as documented.
    e.execute("f = uigetfile('*.txt', 'Pick', 'MultiSelect', 'on');")
        .unwrap();
    check(&mut e, "ischar(f) && strcmp(f, 'only.txt')");
    let requests = script.requests.lock().unwrap();
    assert!(matches!(
        &requests[1],
        DialogRequest::OpenFile { multiple: true, .. }
    ));
    let DialogRequest::OpenFile { title, filters, .. } = &requests[0] else {
        panic!()
    };
    assert_eq!(title, "Select File to Open");
    assert_eq!(filters[0].patterns, ["*.txt"]);
}

#[test]
fn uigetfile_default_name_sets_folder_and_statement_sets_ans() {
    let (mut e, script) = engine(vec![files("/d", &["r.csv"], 0)]);
    // An absolute folder on every platform (C:\... on Windows).
    let start = std::env::temp_dir().join("start");
    e.execute(&format!(
        "uigetfile('*.m;*.csv', 'T', '{}/r.csv')",
        start.display()
    ))
    .unwrap();
    check(&mut e, "strcmp(ans, 'r.csv')");
    let requests = script.requests.lock().unwrap();
    let DialogRequest::OpenFile {
        folder,
        name,
        filters,
        ..
    } = &requests[0]
    else {
        panic!()
    };
    assert_eq!(folder.as_deref(), Some(start.as_path()));
    assert_eq!(name, "r.csv");
    assert_eq!(filters[0].patterns, ["*.m", "*.csv"]);
}

#[test]
fn uiputfile_and_uigetdir() {
    let (mut e, script) = engine(vec![
        files("/out", &["result.mat"], 0),
        DialogReply::Cancelled,
        DialogReply::Folder("/home/me/data".into()),
        DialogReply::Cancelled,
    ]);
    e.execute("[f, p, k] = uiputfile('*.mat', 'Save as', 'result.mat');")
        .unwrap();
    check(&mut e, "strcmp(f, 'result.mat') && k == 1");
    check(&mut e, &format!("strcmp(p, '/out{}')", sep()));
    e.execute("[f, p] = uiputfile();").unwrap();
    check(&mut e, "isequal(f, 0) && isequal(p, 0)");
    let home = std::env::temp_dir();
    e.execute(&format!(
        "d = uigetdir('{}', 'Pick a folder');",
        home.display()
    ))
    .unwrap();
    check(&mut e, "strcmp(d, '/home/me/data')");
    e.execute("d = uigetdir();").unwrap();
    check(&mut e, "isequal(d, 0)");
    let requests = script.requests.lock().unwrap();
    assert!(
        matches!(&requests[0], DialogRequest::SaveFile { title, name, .. } if title == "Save as" && name == "result.mat")
    );
    assert!(
        matches!(&requests[1], DialogRequest::SaveFile { title, .. } if title == "Select File to Write")
    );
    assert!(
        matches!(&requests[2], DialogRequest::Folder { title, folder: Some(f) } if title == "Pick a folder" && f == &home)
    );
}

#[test]
fn inputdlg_returns_a_cell_column_or_an_empty_cell() {
    let (mut e, script) = engine(vec![
        DialogReply::Answers(vec!["5".into(), "Ann".into()]),
        DialogReply::Cancelled,
        DialogReply::Answers(vec!["line one\nline 2".into()]),
    ]);
    e.execute("a = inputdlg({'Count:', 'Name:'}, 'Setup', [1 30], {'3', 'Bob'});")
        .unwrap();
    check(&mut e, "iscell(a) && isequal(size(a), [2 1])");
    check(&mut e, "strcmp(a{1}, '5') && strcmp(a{2}, 'Ann')");
    check(&mut e, "str2double(a{1}) == 5");
    e.execute("a = inputdlg('Value:');").unwrap();
    check(&mut e, "iscell(a) && isempty(a)");
    // A multi-line field answers a char matrix, one padded row per line.
    e.execute("a = inputdlg('Notes:', 'T', 3);").unwrap();
    check(&mut e, "isequal(size(a{1}), [2 8])");
    check(&mut e, "strcmp(a{1}(2,:), 'line 2  ')");
    let requests = script.requests.lock().unwrap();
    assert_eq!(
        requests[0],
        DialogRequest::Input {
            title: "Setup".into(),
            prompts: vec!["Count:".into(), "Name:".into()],
            defaults: vec!["3".into(), "Bob".into()],
            lines: vec![1, 1],
        }
    );
    assert!(matches!(&requests[2], DialogRequest::Input { lines, .. } if lines == &[3]));
}

#[test]
fn inputdlg_rejects_mismatched_defaults() {
    let (mut e, _) = engine(Vec::new());
    let error = e
        .execute("a = inputdlg({'A', 'B'}, 'T', 1, {'x'});")
        .unwrap_err();
    assert!(error.contains("one entry per prompt"), "{error}");
}

#[test]
fn questdlg_buttons_defaults_and_close() {
    let (mut e, script) = engine(vec![
        DialogReply::Button("No".into()),
        DialogReply::Button("Blue".into()),
        DialogReply::Cancelled,
    ]);
    e.execute("b = questdlg('Continue?');").unwrap();
    check(&mut e, "strcmp(b, 'No')");
    e.execute("b = questdlg('Colour?', 'Pick', 'Red', 'Blue', 'Green', 'Blue');")
        .unwrap();
    check(&mut e, "strcmp(b, 'Blue')");
    e.execute("b = questdlg('Save?', 'T', 'Save', 'Discard', 'Save');")
        .unwrap();
    check(&mut e, "ischar(b) && isempty(b)");
    let requests = script.requests.lock().unwrap();
    assert_eq!(
        requests[0],
        DialogRequest::Question {
            title: String::new(),
            text: vec!["Continue?".into()],
            buttons: vec!["Yes".into(), "No".into(), "Cancel".into()],
            default: "Yes".into(),
        }
    );
    assert!(
        matches!(&requests[1], DialogRequest::Question { buttons, default, .. } if buttons.len() == 3 && default == "Blue")
    );
    assert!(
        matches!(&requests[2], DialogRequest::Question { buttons, .. } if buttons == &["Save", "Discard"])
    );
}

#[test]
fn msgbox_family_returns_handles_and_close_closes_them() {
    let (mut e, script) = engine(vec![DialogReply::Done]);
    e.execute("h = msgbox({'Done.', 'All files saved.'}, 'Report', 'help');")
        .unwrap();
    check(&mut e, "isscalar(h) && h > 1000000");
    e.execute("e1 = errordlg('Bad input', 'Oops'); w = warndlg('Careful');")
        .unwrap();
    check(&mut e, "e1 ~= h && w ~= e1");
    e.execute("uiwait(h); close(e1); delete(w);").unwrap();
    // A plain message box statement shows no ans.
    e.execute("clear ans; msgbox('hi')").unwrap();
    check(&mut e, "~exist('ans', 'var')");
    let requests = script.requests.lock().unwrap();
    assert_eq!(
        requests[0],
        DialogRequest::Message {
            id: 1_000_001,
            title: "Report".into(),
            text: vec!["Done.".into(), "All files saved.".into()],
            icon: MessageIcon::Help,
        }
    );
    assert!(
        matches!(&requests[1], DialogRequest::Message { icon: MessageIcon::Error, title, .. } if title == "Oops")
    );
    assert!(
        matches!(&requests[2], DialogRequest::Message { icon: MessageIcon::Warn, title, .. } if title == "Warning Dialog")
    );
    assert!(matches!(
        requests[3],
        DialogRequest::WaitClosed {
            id: 1_000_001,
            timeout: None
        }
    ));
    assert!(matches!(
        requests[4],
        DialogRequest::Close { id: 1_000_002 }
    ));
    assert!(matches!(
        requests[5],
        DialogRequest::Close { id: 1_000_003 }
    ));
}

#[test]
fn waitbar_creates_updates_and_closes() {
    let (mut e, script) = engine(Vec::new());
    e.execute(
        "h = waitbar(0, 'Working...', 'Name', 'Progress');\n\
         for k = 1:4, waitbar(k/4, h); end\n\
         waitbar(1.5, h, 'Finishing');\n\
         close(h);",
    )
    .unwrap();
    check(&mut e, "h > 1000000");
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 7, "{requests:?}");
    assert_eq!(
        requests[0],
        DialogRequest::Progress {
            id: 1_000_001,
            title: "Progress".into(),
            text: vec!["Working...".into()],
            fraction: 0.,
        }
    );
    // Updates keep the title and message.
    assert_eq!(
        requests[2],
        DialogRequest::Progress {
            id: 1_000_001,
            title: "Progress".into(),
            text: vec!["Working...".into()],
            fraction: 0.5,
        }
    );
    assert!(
        matches!(&requests[5], DialogRequest::Progress { fraction, text, .. } if *fraction == 1. && text == &["Finishing"])
    );
    assert!(matches!(
        requests[6],
        DialogRequest::Close { id: 1_000_001 }
    ));
    drop(requests);
    // A closed waitbar handle is no longer valid.
    let error = e.execute("waitbar(0.5, h);").unwrap_err();
    assert!(error.contains("Invalid waitbar handle"), "{error}");
}

#[test]
fn without_a_dialog_host_blocking_dialogs_fail_at_once() {
    let mut e = Engine::default();
    for code in [
        "f = uigetfile('*.csv');",
        "f = uiputfile();",
        "d = uigetdir();",
        "a = inputdlg('x');",
        "b = questdlg('q');",
    ] {
        let error = e.execute(code).unwrap_err();
        assert!(
            error.contains("dialogs need the desktop app"),
            "{code}: {error}"
        );
    }
    // A failed msgbox leaves no handle behind.
    assert!(e.execute("uiwait(1000001);").is_err());
}

#[test]
fn dialog_names_exist_as_builtins() {
    let mut e = Engine::default();
    for name in [
        "uigetfile",
        "uiputfile",
        "uigetdir",
        "inputdlg",
        "questdlg",
        "msgbox",
        "waitbar",
    ] {
        e.execute(&format!("x__ = exist('{name}');")).unwrap();
        assert!(e.variables["x__"].data()[0] > 0., "{name}");
    }
}

#[test]
fn review_cases_filters_file_names_folders_dims_and_options() {
    let tmp = std::env::temp_dir();
    let tmp_text = tmp.display().to_string();
    let tmp_text = tmp_text.trim_end_matches(['/', '\\']);
    let (mut e, script) = engine(vec![
        DialogReply::Cancelled,
        DialogReply::Cancelled,
        DialogReply::Cancelled,
        DialogReply::Cancelled,
        DialogReply::Answers(vec!["a".into(), "b".into(), "c".into()]),
        DialogReply::Button("No".into()),
    ]);
    // A 1-by-2 cell is one pattern with its label.
    e.execute("uigetfile({'*.csv', 'CSV files'});").unwrap();
    // A file name as filter: prefilled, its extension filters, its folder starts.
    e.execute(&format!("uiputfile('{tmp_text}/result.csv');"))
        .unwrap();
    // No folder given: the engine's current folder.
    e.execute(&format!("cd('{tmp_text}'); uigetfile('*.csv');"))
        .unwrap();
    // 'MultiSelect' as the third positional argument is a file name.
    e.execute("uigetfile('*.csv', 'Pick', 'MultiSelect');")
        .unwrap();
    e.execute("a = inputdlg({'A', 'B', 'C'}, 'T', [1 2 3]);")
        .unwrap();
    e.execute(
        "o = struct('Default', 'No', 'Interpreter', 'none'); b = questdlg('Proceed?', 'T', o);",
    )
    .unwrap();
    check(&mut e, "strcmp(b, 'No')");
    let requests = script.requests.lock().unwrap();
    let DialogRequest::OpenFile { filters, .. } = &requests[0] else {
        panic!("{requests:?}")
    };
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0].patterns, ["*.csv"]);
    assert_eq!(filters[0].label, "CSV files");
    let DialogRequest::SaveFile {
        filters,
        folder,
        name,
        ..
    } = &requests[1]
    else {
        panic!("{requests:?}")
    };
    assert_eq!(name, "result.csv");
    assert_eq!(filters[0].patterns, ["*.csv"]);
    assert_eq!(folder.as_deref(), Some(tmp.as_path()));
    let DialogRequest::OpenFile { folder, .. } = &requests[2] else {
        panic!("{requests:?}")
    };
    let current = folder.as_ref().expect("current folder");
    assert_eq!(current.canonicalize().unwrap(), tmp.canonicalize().unwrap());
    assert!(
        matches!(&requests[3], DialogRequest::OpenFile { name, multiple: false, .. } if name == "MultiSelect")
    );
    assert!(matches!(&requests[4], DialogRequest::Input { lines, .. } if lines == &[1, 2, 3]));
    assert!(
        matches!(&requests[5], DialogRequest::Question { default, buttons, .. } if default == "No" && buttons.len() == 3)
    );
}

#[test]
fn review_desktop_dialog_handle_contract() {
    let (mut e, _) = engine(Vec::new());
    e.execute("h=waitbar(0); alive=double(ishghandle(h) && ishandle(h)); delete(h(1)); dead=double(~ishghandle(h) && ~ishandle(h)); m=msgbox('x','Original');").unwrap();
    for n in ["alive", "dead"] {
        assert_eq!(e.variables[n].data(), [1.]);
    }
    let err = e.execute("waitbar(.5,m);").unwrap_err();
    assert!(err.contains("Invalid waitbar handle"), "{err}");
    check(&mut e, "strcmp(get(m,'Name'),'Original')");
    e.execute("set(0,'DefaultFigureColor',[.2 .3 .4]); w=waitbar(0,'x','Color',[1 0 0]); inherited=msgbox('x');").unwrap();
    check(
        &mut e,
        "isequal(get(w,'Color'),[1 0 0]) && isequal(get(inherited,'Color'),[.2 .3 .4])",
    );
    e.execute("set(0,'ShowHiddenHandles','on'); kids=get(0,'Children');")
        .unwrap();
    check(&mut e, "numel(kids)==3 && all(isvalid(kids))");
    e.execute(
        "close all; kept=double(ishghandle(w)); close all hidden; closed=double(~ishghandle(w));",
    )
    .unwrap();
    for n in ["kept", "closed"] {
        assert_eq!(e.variables[n].data(), [1.]);
    }
    let (mut e, _) = engine(vec![DialogReply::Selection(vec![0])]);
    e.execute("[i,ok]=listdlg('ListString',{'a'});").unwrap();
    check(&mut e, "i == 1 && ok == 1");
}

#[test]
fn fix15_listdlg_single_multiple_cancel_and_options() {
    let (mut e, script) = engine(vec![
        DialogReply::Selection(vec![1]),
        DialogReply::Selection(vec![0, 2]),
        DialogReply::Cancelled,
    ]);
    e.execute(
        "[i1, tf1] = listdlg('ListString', {'Red','Green','Blue'}, 'SelectionMode', 'single', 'InitialValue', 2, 'Name', 'Pick');
         [i2, tf2] = listdlg('ListString', {'Red','Green','Blue'}, 'InitialValue', [1 3], 'PromptString', {'Choose', 'colors'}, 'OKString', 'Choose', 'CancelString', 'Abort', 'ListSize', [200 240]);
         [i3, tf3] = listdlg('ListString', {'Red','Green'});",
    )
    .unwrap();
    check(&mut e, "isequal(i1, 2) && tf1 == 1");
    check(&mut e, "isequal(i2, [1 3]) && tf2 == 1");
    check(&mut e, "isempty(i3) && tf3 == 0");

    let requests = script.requests.lock().unwrap();
    assert_eq!(
        requests[0],
        DialogRequest::List {
            title: "Pick".into(),
            prompts: Vec::new(),
            items: vec!["Red".into(), "Green".into(), "Blue".into()],
            multiple: false,
            initial: vec![1],
            ok_label: "OK".into(),
            cancel_label: "Cancel".into(),
            list_size: [160., 300.],
        }
    );
    assert_eq!(
        requests[1],
        DialogRequest::List {
            title: String::new(),
            prompts: vec!["Choose".into(), "colors".into()],
            items: vec!["Red".into(), "Green".into(), "Blue".into()],
            multiple: true,
            initial: vec![0, 2],
            ok_label: "Choose".into(),
            cancel_label: "Abort".into(),
            list_size: [200., 240.],
        }
    );
    assert!(matches!(
        &requests[2],
        DialogRequest::List { multiple: true, initial, .. } if initial == &[0]
    ));
}

#[test]
fn fix15_listdlg_newlines_create_selectable_rows() {
    let (mut e, script) = engine(vec![DialogReply::Selection(vec![1])]);
    e.execute(
        r"[idx,tf]=listdlg('ListString',{sprintf('a\nb'),'c'},'InitialValue',2);
           assert(isequal(idx,2) && tf==1);",
    )
    .unwrap();
    let requests = script.requests.lock().unwrap();
    let DialogRequest::List { items, initial, .. } = &requests[0] else {
        panic!("expected a list request, got {:?}", requests[0]);
    };
    assert_eq!(items, &["a", "b", "c"]);
    assert_eq!(initial, &[1]);
}

#[test]
fn fix15_listdlg_host_distinguishes_empty_confirmation_and_cancel() {
    let (mut e, _) = engine(vec![
        DialogReply::Selection(Vec::new()),
        DialogReply::Cancelled,
    ]);
    e.execute(
        "[selected,accepted]=listdlg('ListString',{'a'},'InitialValue',[]);
         [cancelled,rejected]=listdlg('ListString',{'a'},'InitialValue',[]);
         assert(isequal(selected,[]) && accepted==1);
         assert(isequal(cancelled,[]) && rejected==0);",
    )
    .unwrap();
}

#[test]
fn round_1_waitbar_accepts_extra_figure_options() {
    let (mut e, _) = engine(Vec::new());
    e.execute(
        "h=waitbar(0,'x','CreateCancelBtn','disp(1)','WindowStyle','modal', ...
                   'Position',[10 20 300 80],'Units','pixels','Name','A');
         waitbar(.5,h,'y','Units','normalized','Name','B');",
    )
    .unwrap();
    check(&mut e, "ishghandle(h) && strcmp(get(h,'Name'),'B')");
    let error = e.execute("set(h,'NoSuchDialogProperty',1);").unwrap_err();
    assert!(error.contains("Unsupported dialog property"), "{error}");
}

#[test]
fn round_1_dialog_properties_accept_scalar_text() {
    let (mut e, _) = engine(Vec::new());
    e.execute(r#"h=waitbar(0,'x',"Name","Initial","Color",[0 1 0]); p={'Name'};"#)
        .unwrap();
    check(
        &mut e,
        r#"strcmp(get(h,"Name"),'Initial') && strcmp(get(h,p{1}),'Initial') && isequal(get(h,'Color'),[0 1 0])"#,
    );
    e.execute(r#"set(h,"Color",[1 0 0],"Name","Updated","Theme","dark");"#)
        .unwrap();
    check(
        &mut e,
        r#"isequal(get(h,"Color"),[1 0 0]) && strcmp(get(h,"Name"),'Updated') && strcmp(get(h,'Theme'),'dark')"#,
    );
}

#[test]
fn round_1_invalid_waitbar_creation_leaves_no_child_or_window() {
    let (mut e, script) = engine(Vec::new());
    e.execute("set(0,'ShowHiddenHandles','on');").unwrap();
    let error = e
        .execute("h=waitbar(0,'x','Color','nocolor');")
        .unwrap_err();
    assert!(error.contains("color"), "{error}");
    check(&mut e, "numel(get(0,'Children'))==0");
    assert!(script.requests.lock().unwrap().is_empty());
}

#[test]
fn round_1_waitbar_update_applies_color() {
    let (mut e, _) = engine(Vec::new());
    e.execute("h=waitbar(0,'x','Color',[0 1 0]);").unwrap();
    check(&mut e, "isequal(get(h,'Color'),[0 1 0])");
    e.execute("again=waitbar(.5,h,'y','Color',[1 0 0]);")
        .unwrap();
    check(&mut e, "again==h && isequal(get(h,'Color'),[1 0 0])");
}

#[test]
fn round_1_waitbar_update_preserves_name() {
    let (mut e, script) = engine(Vec::new());
    e.execute("h=waitbar(0,'x','Name','A'); set(h,'Name','B'); waitbar(.5,h);")
        .unwrap();
    check(&mut e, "strcmp(get(h,'Name'),'B')");
    e.execute("waitbar(.7,h,'y');").unwrap();
    check(&mut e, "strcmp(get(h,'Name'),'B')");
    e.execute("waitbar(.8,h,'Name','C');").unwrap();
    check(&mut e, "strcmp(get(h,'Name'),'C')");

    let requests = script.requests.lock().unwrap();
    for (request, (expected_title, expected_text)) in
        requests
            .iter()
            .zip([("A", "x"), ("B", "x"), ("B", "y"), ("C", "y")])
    {
        assert!(
            matches!(request, DialogRequest::Progress { title, text, .. }
                if title == expected_title && text == &[expected_text]),
            "{request:?}"
        );
    }
    assert_eq!(requests.len(), 4);
}

#[test]
fn round_1_desktop_dialogs_are_visible() {
    let (mut e, _) = engine(Vec::new());
    e.execute(
        "h=waitbar(0); m=msgbox('x');
         set(0,'ShowHiddenHandles','on'); kids=get(0,'Children'); kid=kids(1);",
    )
    .unwrap();
    check(
        &mut e,
        "strcmp(get(h,'Visible'),'on') && strcmp(get(m,'Visible'),'on') && strcmp(kid.Visible,'on')",
    );
}

#[test]
fn round_1_indexed_close_and_delete_evaluate_once() {
    let (mut e, _) = engine(Vec::new());
    let result = e
        .execute(
            r"figure(5); figure(6); x=[5 6];
              close(x(fprintf('E\n')));
              h=plot(1:3); delete(h(fprintf('D')));
              fprintf('\n');",
        )
        .unwrap();
    assert_eq!(result.output, ["E", "D"]);
    check(&mut e, "ishghandle(5) && ~ishghandle(6) && ~ishghandle(h)");
}
