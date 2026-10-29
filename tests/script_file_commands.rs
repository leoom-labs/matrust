use matrust::{Array, Engine, Matrix, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn engine(directory: &Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(directory).unwrap();
    e
}
fn scalar(e: &Engine, name: &str) -> f64 {
    e.array(name)
        .unwrap()
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}

#[test]
fn script_file_commands_roundtrip_all_versions_and_typed_subset_merge() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    for option in ["'-v6'", "'-v7'", "'-v7','-nocompression'", "'-v7.3'"] {
        e.execute("clear;A=[1 2 3;4 5 6];mask=logical([0 1 0]);keep=77;")
            .unwrap();
        let original = e.workspace();
        e.execute(&format!("save('typed','A','mask',{option});"))
            .unwrap();
        for (name, value) in &original {
            assert!(value.same_snapshot(&e.array(name).unwrap()));
        }
        e.execute("A=9;mask=0;load('typed','A','mask');").unwrap();
        assert_eq!(
            e.array("A")
                .unwrap()
                .scalar(1, 2)
                .unwrap()
                .into_real()
                .unwrap(),
            6.
        );
        assert!(e.array("mask").unwrap().is_logical());
        assert_eq!(scalar(&e, "keep"), 77.);
        let catalog =
            files::inspect_workspace(&dir.path().join("typed.mat"), &AtomicBool::new(false))
                .unwrap();
        assert_eq!(
            catalog
                .variables()
                .map(|v| v.name.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["A", "mask"])
        );
    }
}

#[test]
fn script_file_commands_command_quotes_continuations_defaults_and_statement_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("A=12;mask=logical([0 1]);save 'quote'' \u{5e9}\u{6570}%.mat' A ... % continuation\n mask -v7.3; A=9; load 'quote'' \u{5e9}\u{6570}%.mat' A, after=4;").unwrap();
    assert_eq!(scalar(&e, "A"), 12.);
    assert_eq!(scalar(&e, "after"), 4.);
    e.execute("save -v7.3;clear;load;loaded=1;").unwrap();
    assert_eq!(scalar(&e, "A"), 12.);
    assert!(e.array("mask").unwrap().is_logical());
    e.execute("save();clear;load();").unwrap();
    assert_eq!(scalar(&e, "loaded"), 1.);
    e.execute("if 1, save block.mat A;end; A=7;for i=1:2;load block.mat A;end")
        .unwrap();
    assert_eq!(scalar(&e, "A"), 12.);
    assert!(dir.path().join("quote' \u{5e9}\u{6570}%.mat").is_file());
    for filename in ["%leading.mat", ";leading.mat", ",leading.mat"] {
        e.execute(&format!("save '{filename}' A;A=0;load '{filename}' A;"))
            .unwrap();
        assert_eq!(scalar(&e, "A"), 12.);
        assert!(dir.path().join(filename).is_file());
    }
}

#[test]
fn script_file_commands_reject_side_effecting_output_forms_options_and_missing_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("A=7;save existing.mat A -v7.3;").unwrap();
    let path = dir.path().join("existing.mat");
    let before = std::fs::read(&path).unwrap();
    for script in [
        "save existing.mat missing",
        "save existing.mat A -append",
        "save existing.mat A -v7.3 -nocompression",
        "save existing.mat A -v7 -v6",
        "save existing.mat A*",
        "result=save('new.mat')",
        "A(1)=save('new.mat')",
        "x=sin(save('new.mat'))",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join("new.mat").exists());
        assert_eq!(scalar(&e, "A"), 7.);
    }
    e.execute("A=99;").unwrap();
    for script in [
        "result=load('existing.mat')",
        "result=sin(load('existing.mat'))",
        "load existing.mat A missing",
        "load existing.mat -regexp A",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(scalar(&e, "A"), 99.);
    }
}

#[test]
fn script_file_commands_preserve_caller_scope_and_name_shadowing() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("A=100;answer=helper();function y=helper();A=8;save local.mat A -v7.3;A=2;load local.mat A;y=A;end").unwrap();
    assert_eq!(scalar(&e, "A"), 100.);
    assert_eq!(scalar(&e, "answer"), 8.);
    e.execute("r=load(5);function y=load(x);y=x+3;end").unwrap();
    assert_eq!(scalar(&e, "r"), 8.);
    e.execute("save=9;load=[6 7];save + 1; indexed=load(2);both=[save load];")
        .unwrap();
    assert_eq!(scalar(&e, "ans"), 10.);
    assert_eq!(scalar(&e, "indexed"), 7.);
    for (expression, expected) in [
        ("save .* 2", 18.),
        ("save ./ 3", 3.),
        ("save .^ 2", 81.),
        ("save ~= 3", 1.),
    ] {
        e.execute(expression).unwrap();
        assert_eq!(scalar(&e, "ans"), expected);
    }
    for expression in ["load '", "load .'", "load ';", "load ' % comment"] {
        e.execute(expression).unwrap();
        let value = e.array("ans").unwrap();
        assert_eq!((value.rows(), value.cols()), (2, 1));
        assert_eq!(value.scalar(1, 0).unwrap().into_real().unwrap(), 7.);
    }
    e.execute("load ' + 1").unwrap();
    let value = e.array("ans").unwrap();
    assert_eq!((value.rows(), value.cols()), (2, 1));
    assert_eq!(value.scalar(1, 0).unwrap().into_real().unwrap(), 8.);
    assert!(!dir.path().join("matlab.mat").exists());
}

#[test]
fn script_file_commands_isolate_directories_and_force_mat_without_host_cwd_changes() {
    let cwd = std::env::current_dir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut first = engine(&a);
    let mut second = engine(&b);
    first.execute("A=11;save same.mat A -v7.3").unwrap();
    second.execute("A=22;save same.mat A").unwrap();
    first.execute("clear;load same.mat").unwrap();
    second.execute("clear;load same.mat").unwrap();
    assert_eq!(scalar(&first, "A"), 11.);
    assert_eq!(scalar(&second, "A"), 22.);
    assert!(
        first
            .set_file_directory(&dir.path().join("missing"))
            .is_err()
    );
    first.execute("save special.dat A -v7.3;A=6;").unwrap();
    assert!(
        first
            .execute("load special.dat")
            .unwrap_err()
            .contains("ASCII")
    );
    assert_eq!(scalar(&first, "A"), 6.);
    first.execute("load special.dat -mat A;").unwrap();
    assert_eq!(scalar(&first, "A"), 11.);
    assert!(a.join("special.dat").exists() && !b.join("special.dat").exists());
    first
        .execute("save trailing. A;A=0;load trailing.;")
        .unwrap();
    assert_eq!(scalar(&first, "A"), 11.);
    assert!(a.join("trailing..mat").exists());
    first
        .execute("save ./relative.mat A;A=0;load ./relative.mat;")
        .unwrap();
    assert_eq!(scalar(&first, "A"), 11.);
    assert_eq!(std::env::current_dir().unwrap(), cwd);
}

#[test]
fn script_file_commands_corrupt_or_cancelled_loads_leave_workspace_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    std::fs::copy(
        "tests/fixtures/mat/bad-checksum-v73.mat",
        dir.path().join("broken.mat"),
    )
    .unwrap();
    e.execute("good=99;keep=7;").unwrap();
    let before = e.workspace();
    assert!(e.execute("load broken.mat").is_err());
    for (name, value) in &before {
        assert!(value.same_snapshot(&e.array(name).unwrap()));
    }
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("save cancelled.mat").is_err());
    assert!(!dir.path().join("cancelled.mat").exists());
    e.cancellation().store(false, Ordering::Relaxed);
    e.execute("clear;save empty.mat -v7.3;keep=3;load empty.mat;")
        .unwrap();
    assert_eq!(e.workspace().len(), 1);
}

#[test]
fn script_file_commands_validate_merged_budget_before_loading_payload() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let m = Matrix::from_column_vec(1024 * 1024, 12, vec![1.; 12 * 1024 * 1024]).unwrap();
    let values = BTreeMap::from([("incoming".into(), Array::from(m.clone()))]);
    files::save_array_workspace_v73(
        &dir.path().join("large.mat"),
        &values,
        &AtomicBool::new(false),
    )
    .unwrap();
    e.variables.insert("first".into(), m.clone());
    e.variables.insert("second".into(), m);
    assert!(
        e.execute("load large.mat")
            .unwrap_err()
            .contains("Resulting workspace exceeds")
    );
    assert!(!e.variables.contains_key("incoming"));
    assert_eq!(e.variables.len(), 2);
}

#[test]
fn script_file_commands_source_spans_and_malformed_quotes_fail_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    let text = "A=3;\nsave chosen.mat A   ;\nsave skipped.mat A;";
    e.execute("A=9;").unwrap();
    let start = text.find("save chosen").unwrap();
    let end = start + "save chosen.mat A".len();
    e.execute_selection(text, start..end).unwrap();
    assert!(dir.path().join("chosen.mat").exists() && !dir.path().join("skipped.mat").exists());
    assert_eq!(scalar(&e, "A"), 9.);
    for source in [
        "save would.mat;\nload 'unterminated",
        "save would.mat;\nload 'bad'joined",
        "save would.mat;\nload \"broken\nquote\"",
    ] {
        assert!(e.execute(source).is_err());
        assert_eq!(e.diagnostic.as_ref().unwrap().line, 2);
        assert!(!dir.path().join("would.mat").exists());
    }
    let many = format!("save {}", vec!["x"; 256].join(" "));
    assert!(e.execute(&many).unwrap_err().contains("255 arguments"));
}

#[test]
fn script_file_commands_cli_uses_script_directory_for_relative_files() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("script folder");
    std::fs::create_dir(&sub).unwrap();
    let script = sub.join("exchange.m");
    std::fs::write(
        &script,
        "A=42;save output.mat A -v7.3;clear;load output.mat;disp(A);",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg(&script)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("42"));
    assert!(sub.join("output.mat").exists() && !dir.path().join("output.mat").exists());
}

#[test]
fn cli_saved_bundle_uses_real_script_path_from_another_working_directory() {
    use matrust::plot::{Figure, PlotKind, Series};
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("moved ' \u{03bb}");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("reconstruct.m");
    let mut figure = Figure::default();
    figure
        .add(vec![Series::new(
            (0..30_000).map(|i| [i as f64, i as f64 / 7.]).collect(),
            PlotKind::Line,
        )])
        .unwrap();
    let bundle = matrust::files::write_matlab_plot_reconstruction_with_cancel(
        &path,
        &figure,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let mut source = std::fs::read_to_string(&path).unwrap();
    source.push_str("\ndisp(mfilename());savefig replayed.mfig;\n");
    std::fs::write(&path, source).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
        .arg(&path)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("reconstruct"));
    let loaded = matrust::files::load_figure(&folder.join("replayed.mfig")).unwrap();
    assert_eq!(
        loaded.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice(),
        figure.series[0]
            .points
            .iter()
            .collect::<Vec<_>>()
            .as_slice()
    );
    assert!(!root.path().join("replayed.mfig").exists());
    assert!(bundle.data.is_file());
}
