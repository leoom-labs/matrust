use matrust::{Engine, HostIo};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

fn workspace() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("added")).unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.set_function_directory(dir.path()).unwrap();
    (e, dir)
}

fn run(e: &mut Engine, code: &str) -> String {
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"))
        .output
        .join("\n")
}

#[derive(Clone, Default)]
struct Editor(Arc<Mutex<Vec<Edit>>>);

#[derive(Debug, PartialEq)]
enum Edit {
    File(PathBuf),
    New(Option<String>),
    Copy(PathBuf),
}

impl HostIo for Editor {
    fn read_line(&mut self, _: &str) -> Option<String> {
        None
    }

    fn edit_file(&mut self, path: Option<&Path>, title: Option<&str>) -> bool {
        let edit = match path {
            Some(path) => Edit::File(path.to_owned()),
            None => Edit::New(title.map(str::to_owned)),
        };
        self.0.lock().unwrap().push(edit);
        true
    }

    fn edit_file_copy(&mut self, path: &Path) -> bool {
        self.0.lock().unwrap().push(Edit::Copy(path.to_owned()));
        true
    }
}

#[cfg(unix)]
#[test]
fn p1_program_and_method_symlinks_stay_inside_added_root() {
    use std::os::unix::fs::symlink;

    let (mut e, dir) = workspace();
    let root = dir.path();
    let added = root.join("added");
    let outside = root.join("sibling");
    fs::create_dir(&outside).unwrap();
    let source = outside.join("probe.m");
    fs::write(&source, "% Lookup fixture\n").unwrap();
    symlink(&source, added.join("probe.m")).unwrap();
    fs::create_dir(added.join("@Widget")).unwrap();
    symlink(&source, added.join("@Widget").join("inspectme.m")).unwrap();
    symlink(&outside, added.join("+pkg")).unwrap();
    symlink(&source, root.join("localprobe.m")).unwrap();
    fs::write(added.join("inside.m"), "% Contained fixture\n").unwrap();
    symlink("inside.m", added.join("alias.m")).unwrap();

    let editor = Editor::default();
    e.set_host_io(Some(Box::new(editor.clone())));
    let probes = [
        "disp(exist('probe.m','file'))",
        "which probe",
        "which probe -all",
        "help probe",
        "which pkg.probe",
        "help pkg.probe",
        "disp(exist('inspectme','file'))",
        "which inspectme",
        "which Widget.inspectme",
        "help Widget/inspectme",
    ];
    let observe = |engine: &mut Engine| probes.map(|code| engine.execute(code).map(|r| r.output));
    let misses = observe(&mut e);
    run(&mut e, "addpath added");
    assert_eq!(observe(&mut e), misses);
    run(&mut e, "edit probe");
    assert_eq!(
        *editor.0.lock().unwrap(),
        [Edit::New(Some("probe.m".into()))]
    );
    run(
        &mut e,
        "assert(exist('alias','file') == 2 && exist('localprobe','file') == 2);",
    );
    assert!(run(&mut e, "help alias").contains("Contained fixture"));
    assert!(run(&mut e, "help localprobe").contains("Lookup fixture"));
}

#[test]
fn p2_edit_added_script_has_contents_and_no_save_target() {
    let (mut e, dir) = workspace();
    let editor = Editor::default();
    e.set_host_io(Some(Box::new(editor.clone())));
    let source = include_str!("fixtures/classdef/octave/Plain.m");
    fs::write(dir.path().join("added").join("probe.m"), source).unwrap();
    run(&mut e, "addpath added");
    run(&mut e, "edit probe");
    run(&mut e, "edit('probe.m')");
    fs::write(dir.path().join("probe.m"), "% Local\n").unwrap();
    run(&mut e, "edit probe");
    let local = dir.path().join("probe.m").canonicalize().unwrap();
    let added = dir
        .path()
        .join("added")
        .join("probe.m")
        .canonicalize()
        .unwrap();
    assert_eq!(
        *editor.0.lock().unwrap(),
        [
            Edit::Copy(added.clone()),
            Edit::Copy(added.clone()),
            Edit::File(local),
        ]
    );

    // Exercise the document policy used by the desktop preflight and Save picker.
    let mut document = matrust::document::Document::open(&added).unwrap();
    document.require_save_as();
    assert_eq!(document.text, source);
    // Restoring a clean tab refreshes its source without granting a Save target.
    let session_file = dir.path().join("session.json");
    let snapshot = serde_json::json!({
        "version": 1, "documents": [document], "active": 0, "folder": dir.path(),
        "recent": [], "history": [], "model": matrust::simulation::Model::default(),
    });
    fs::write(&session_file, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let mut document = matrust::session::Session::load(&session_file)
        .unwrap()
        .documents
        .remove(0);
    assert_eq!(document.text, source);
    document.text.push_str("% edited\n");
    document.changed();
    assert_eq!(document.default_save_path(), None);
    // F4: the Save dialog proposes the source name (.m first) in the user's
    // folder, never the added-folder path.
    let user_folder = dir.path().join("work");
    assert_eq!(
        document.suggested_save_path(&user_folder),
        Some(user_folder.join("probe.m"))
    );
    assert_eq!(
        matrust::document::Document::new().suggested_save_path(&user_folder),
        None
    );
    let chosen = dir.path().canonicalize().unwrap().join("chosen.m");
    document.save(&chosen).unwrap();
    assert_eq!(document.default_save_path(), Some(chosen.as_path()));
    assert_eq!(fs::read_to_string(chosen).unwrap(), document.text);
    assert_eq!(fs::read_to_string(added).unwrap(), source);
}

#[test]
fn p2_edit_refuses_non_script_kinds_from_added_folder() {
    let (mut e, dir) = workspace();
    let editor = Editor::default();
    e.set_host_io(Some(Box::new(editor.clone())));
    let added = dir.path().join("added");
    for name in ["m.mrsim", "f.mrfig", "d.csv"] {
        fs::write(added.join(name), "x").unwrap();
    }
    run(&mut e, "addpath added");
    for name in ["m.mrsim", "f.mrfig", "d.csv"] {
        let err = e.execute(&format!("edit {name}")).unwrap_err().to_string();
        assert!(err.contains("added path folder"), "{name}: {err}");
    }
    assert!(editor.0.lock().unwrap().is_empty());
    // A local file of the same kind still opens as an ordinary file.
    fs::write(dir.path().join("m.mrsim"), "x").unwrap();
    run(&mut e, "edit m.mrsim");
    let local = dir.path().join("m.mrsim");
    match editor.0.lock().unwrap().as_slice() {
        [Edit::File(path)] => {
            assert_eq!(path.canonicalize().unwrap(), local.canonicalize().unwrap())
        }
        other => panic!("{other:?}"),
    }
}

#[cfg(windows)]
#[test]
fn p3_fopen_rejects_drive_relative_before_opening() {
    let (mut e, _dir) = workspace();
    let expected = e.execute("load('C:x.txt')").unwrap_err();
    assert!(expected.contains("Drive-relative filenames"), "{expected}");
    for mode in ["r", "r+"] {
        let error = e
            .execute(&format!("fopen('C:x.txt','{mode}')"))
            .unwrap_err();
        assert!(error.contains("Drive-relative filenames"), "{error}");
    }
}

#[test]
fn p4_folder_components_do_not_gain_a_file_suffix() {
    let (mut e, dir) = workspace();
    let probes = [
        "load('..')",
        "load('.')",
        "imread('..','png')",
        "imread('.','png')",
    ];
    let local = probes.map(|code| e.execute(code).unwrap_err());
    run(&mut e, "addpath added");
    for folder in [dir.path().join("added"), dir.path().to_owned()] {
        for name in ["..", "."] {
            fs::write(
                folder.join(format!("{name}.mat")),
                include_bytes!("fixtures/mat/scipy-v6.mat"),
            )
            .unwrap();
            fs::write(
                folder.join(format!("{name}.png")),
                include_bytes!("fixtures/preview/tiff-preview/reference-1.png"),
            )
            .unwrap();
        }
        for (code, expected) in probes.iter().zip(&local) {
            assert_eq!(&e.execute(code).unwrap_err(), expected, "{code}");
        }
    }
}

#[cfg(windows)]
#[test]
fn p5_which_reopens_the_selected_verbatim_filename() {
    let (mut e, dir) = workspace();
    let added = dir.path().join("added").canonicalize().unwrap();
    fs::write(added.join("x"), b"ordinary").unwrap();
    fs::write(added.join("x."), b"literal dot").unwrap();
    run(&mut e, "addpath added");
    for (name, expected) in [("x.", "literal dot"), ("x", "ordinary")] {
        run(&mut e, &format!("resolved = which('{name}');"));
        let Some(matrust::Array::Character(path)) = e.array("resolved") else {
            panic!("which('{name}') must return a filename");
        };
        let path = path.to_utf8(&Default::default()).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
    }
}

#[cfg(windows)]
#[test]
fn p6_junction_case_and_prefixed_search_names() {
    use matrust::files::read_fallback::{contained_file, name_is_eligible};
    use std::process::Command;

    let (mut e, dir) = workspace();
    let root = dir.path();
    let added = root.join("added");
    let sibling = root.join("added-other");
    fs::create_dir(&sibling).unwrap();
    fs::write(sibling.join("inspectme.m"), "% Junction fixture\n").unwrap();
    fs::write(sibling.join("Widget.m"), "% Junction class\n").unwrap();
    // Constant relative arguments keep the temporary directory out of cmd's syntax.
    let link = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J", "added\\@Widget", "added-other"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(link.status.success(), "{link:?}");
    assert_eq!(
        fs::read(added.join("@Widget").join("inspectme.m")).unwrap(),
        b"% Junction fixture\n"
    );
    fs::write(added.join("CaseProbe.txt"), "case fixture").unwrap();
    run(&mut e, "addpath added");
    run(
        &mut e,
        "assert(isempty(which('Widget')) && isempty(which('Widget.inspectme')));",
    );
    run(
        &mut e,
        "assert(exist('Widget','file') == 0 && exist('inspectme','file') == 0);",
    );
    run(&mut e, "assert(exist('@Widget/inspectme.m','file') == 0);");
    assert!(e.execute("help Widget/inspectme").is_err());
    let editor = Editor::default();
    e.set_host_io(Some(Box::new(editor.clone())));
    run(&mut e, "edit Widget");
    assert_eq!(
        *editor.0.lock().unwrap(),
        [Edit::New(Some("Widget.m".into()))]
    );
    run(
        &mut e,
        "assert(strcmp(fileread('CaseProbe.txt'),'case fixture'));",
    );
    if added.join("caseprobe.TXT").is_file() {
        run(
            &mut e,
            "assert(strcmp(fileread('caseprobe.TXT'),'case fixture'));",
        );
    } else {
        run(&mut e, "assert(exist('caseprobe.TXT','file') == 0);");
    }
    let added = added.canonicalize().unwrap();
    assert!(contained_file(&added, Path::new(r"@Widget\inspectme.m")).is_none());
    for name in [r"\\server\share\x", r"\\?\C:\x", r"\\?\UNC\server\share\x"] {
        assert!(!name_is_eligible(Path::new(name)), "{name}");
        assert!(contained_file(&added, Path::new(name)).is_none(), "{name}");
    }
}

#[cfg(unix)]
#[test]
fn f5_outward_symlink_function_is_not_called() {
    // Plan decision 2026-11-25 (A-F5): the call path refuses what which/exist refuse.
    let (mut e, dir) = workspace();
    let added = dir.path().join("added");
    let outside = dir.path().join("sibling");
    fs::create_dir(&outside).unwrap();
    fs::write(
        outside.join("probe.m"),
        "function probe\ndisp('ran')\nend\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.join("probe.m"), added.join("probe.m")).unwrap();
    fs::write(added.join("inside.m"), "function inside\ndisp('in')\nend\n").unwrap();
    run(&mut e, "addpath added");
    // Same error as a name that does not exist at all.
    let missing = e.execute("nosuchprobe").unwrap_err();
    let err = e.execute("probe").unwrap_err();
    assert_eq!(err, missing.replace("nosuchprobe", "probe"));
    assert_eq!(run(&mut e, "inside"), "in");
}
