//! Clean-room comparisons: Python difflib fixtures, published LCS edit counts,
//! engine-written MAT roundtrips, and exact CLI output. All I/O uses tempdir.
use matrust::{Array, Engine, engine::compare::*};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

fn cancel() -> AtomicBool {
    AtomicBool::new(false)
}
fn diff(a: &str, b: &str) -> TextDiff {
    text_diff(a, b, TextDiffOptions::default()).unwrap()
}
fn rebuilt(d: &TextDiff) -> String {
    let mut out = String::new();
    let (mut l, mut r) = (1, 1);
    for h in &d.hunks {
        assert_eq!(h.left.start, l);
        assert_eq!(h.right.start, r);
        match h.kind {
            HunkKind::Equal => {
                assert_eq!(h.left.len(), h.right.len());
                for (li, ri) in h.left.clone().zip(h.right.clone()) {
                    assert_eq!(d.left_lines[li - 1], d.right_lines[ri - 1]);
                    out.push_str(&d.left_lines[li - 1]);
                }
            }
            HunkKind::Insert | HunkKind::Replace => {
                for j in h.right.clone() {
                    out.push_str(&d.right_lines[j - 1]);
                }
            }
            HunkKind::Delete => assert!(h.right.is_empty()),
        }
        l = h.left.end;
        r = h.right.end;
    }
    assert_eq!(l, d.left_lines.len() + 1);
    assert_eq!(r, d.right_lines.len() + 1);
    out
}

#[test]
fn text_python_difflib_hunks_and_exact_unified_reports() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/visdiff/text.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let (a, b) = (c["left"].as_str().unwrap(), c["right"].as_str().unwrap());
        let d = diff(a, b);
        assert_eq!(
            d.deleted_lines,
            c["deleted"].as_u64().unwrap() as usize,
            "{}",
            c["name"]
        );
        assert_eq!(
            d.inserted_lines,
            c["inserted"].as_u64().unwrap() as usize,
            "{}",
            c["name"]
        );
        assert_eq!(rebuilt(&d), b);
        assert_eq!(
            render_text_report(&d, "left", "right", &cancel()).unwrap(),
            c["unified"].as_str().unwrap(),
            "{}",
            c["name"]
        );
    }
    let d = diff("a\nold\nc\n", "a\nnew\nc\n");
    assert_eq!(
        d.hunks
            .iter()
            .map(|h| (h.kind, h.left.clone(), h.right.clone()))
            .collect::<Vec<_>>(),
        vec![
            (HunkKind::Equal, 1..2, 1..2),
            (HunkKind::Replace, 2..3, 2..3),
            (HunkKind::Equal, 3..4, 3..4)
        ]
    );
    assert_eq!(diff("a\n", "new\na\n").hunks[0].left, 1..1);
    assert_eq!(diff("a\nold\n", "a\n").hunks[1].right, 2..2);
}

#[test]
fn myers_linear_space_paths_are_minimal_and_rebuild_repeated_lines() {
    let cases: Vec<(String, String, usize)> =
        serde_json::from_str(include_str!("fixtures/visdiff/minimal.json")).unwrap();
    for (a, b, count) in cases {
        let lines = |s: &str| s.chars().map(|c| format!("{c}\n")).collect::<String>();
        let (a, b) = (lines(&a), lines(&b));
        let d = diff(&a, &b);
        assert_eq!(d.deleted_lines + d.inserted_lines, count, "{a:?} -> {b:?}");
        assert_eq!(rebuilt(&d), b);
    }
}

#[test]
fn text_options_unicode_highlights_and_missing_final_newline() {
    for (a, b, opts) in [
        (
            "a\r\nb\r\n",
            "a\nb\n",
            TextDiffOptions {
                ignore_crlf: true,
                ..Default::default()
            },
        ),
        (
            " \t a  b \n",
            "a b\n",
            TextDiffOptions {
                ignore_whitespace: true,
                ..Default::default()
            },
        ),
        (
            "AbC\nÄ\n",
            "abc\nä\n",
            TextDiffOptions {
                ignore_case: true,
                ..Default::default()
            },
        ),
        (
            " \t Ä  B \r\n",
            "ä b\n",
            TextDiffOptions {
                ignore_crlf: true,
                ignore_whitespace: true,
                ignore_case: true,
                ..Default::default()
            },
        ),
    ] {
        assert!(diff(a, b).deleted_lines > 0);
        let d = text_diff(a, b, opts).unwrap();
        assert_eq!((d.deleted_lines, d.inserted_lines), (0, 0));
        assert!(
            render_text_report(&d, "left", "right", &cancel())
                .unwrap()
                .is_empty()
        );
    }
    let d = diff("α red β blue\n", "α tan β cyan\n");
    assert_eq!(d.hunks[0].inline[0].left, vec![2..5, 8..12]);
    assert_eq!(d.hunks[0].inline[0].right, vec![2..5, 8..12]);
    let d = diff("a", "a\n");
    assert_eq!(rebuilt(&d), "a\n");
    assert_eq!(
        render_text_report(&d, "left", "right", &cancel()).unwrap(),
        "--- left\n+++ right\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+a\n"
    );
}

#[test]
fn text_twenty_thousand_lines_fifty_changes_under_one_second() {
    let a = (0..20000)
        .map(|i| format!("line {i}\n"))
        .collect::<String>();
    let b = (0..20000)
        .map(|i| {
            if i % 400 == 0 {
                format!("changed {i}\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect::<String>();
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for repetition in 1..=5 {
            let (d, elapsed) = pool.install(|| {
                let start = Instant::now();
                let d = diff(&a, &b);
                (d, start.elapsed())
            });
            eprintln!(
                "visdiff 20000 lines / 50 replacements: {elapsed:?}; workers={workers}; repetition={repetition}"
            );
            assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
            assert_eq!((d.deleted_lines, d.inserted_lines), (50, 50));
            assert_eq!(rebuilt(&d), b);
        }
    }
}

#[test]
fn text_cancels_during_work_and_rejects_excessive_work() {
    let a = (0..10000)
        .map(|i| format!("left {i}\n"))
        .collect::<String>();
    let b = (0..10000)
        .map(|i| format!("right {i}\n"))
        .collect::<String>();
    let c = std::sync::Arc::new(cancel());
    let flag = c.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(5));
        flag.store(true, Ordering::Relaxed);
    });
    let result = text_diff(
        &a,
        &b,
        TextDiffOptions {
            cancel: Some(&c),
            ..Default::default()
        },
    );
    thread.join().unwrap();
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(
        text_diff(&a, &b, TextDiffOptions::default())
            .unwrap_err()
            .contains("work limit")
    );
    assert!(
        text_diff(&"a\n".repeat(100001), "", TextDiffOptions::default())
            .unwrap_err()
            .contains("lines")
    );
    assert!(
        text_diff(&"a".repeat(65537), "", TextDiffOptions::default())
            .unwrap_err()
            .contains("characters per line")
    );
}

#[test]
fn text_and_files_observe_cancellation_and_limits() {
    let c = cancel();
    c.store(true, Ordering::Relaxed);
    assert!(
        text_diff(
            "a",
            "b",
            TextDiffOptions {
                cancel: Some(&c),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert!(
        mat_diff(Path::new("absent"), Path::new("absent"), &c)
            .unwrap_err()
            .contains("cancelled")
    );
    let too_large = "x".repeat(16 * 1024 * 1024 + 1);
    assert!(
        text_diff(&too_large, "", TextDiffOptions::default())
            .unwrap_err()
            .contains("input limit")
    );
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a"), "a").unwrap();
    assert!(
        folder_diff_with_cancel(temp.path(), temp.path(), &c)
            .unwrap_err()
            .contains("cancelled")
    );
}

fn save_pair(version: &str) {
    let temp = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    e.execute(&format!("same=[1 NaN 3]; leftonly=7; classed=1; sized=[1 2]; value=[1 2;3 4]; cellv={{1,2}}; chars='left'; save('left.mat','{version}'); clear;")).unwrap();
    e.execute(&format!("same=[1 NaN 3]; rightonly=8; classed=single(1); sized=[1 2 3]; value=[1 2;3 9]; cellv={{1,5}}; chars='rift'; save('right.mat','{version}');")).unwrap();
    let c = cancel();
    let d = mat_diff(
        &temp.path().join("left.mat"),
        &temp.path().join("right.mat"),
        &c,
    )
    .unwrap();
    let names = d
        .variables
        .iter()
        .map(|v| v.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "cellv",
            "chars",
            "classed",
            "leftonly",
            "rightonly",
            "same",
            "sized",
            "value"
        ]
    );
    let var = |name: &str| d.variables.iter().find(|v| v.name == name).unwrap();
    assert_eq!(var("same").status, DiffStatus::Same);
    assert_eq!(var("leftonly").status, DiffStatus::OnlyLeft);
    assert_eq!(var("rightonly").status, DiffStatus::OnlyRight);
    assert_eq!(
        var("classed").reasons,
        vec![DifferenceReason::Class {
            left: "double".into(),
            right: "single".into()
        }]
    );
    assert_eq!(
        var("sized").reasons,
        vec![DifferenceReason::Size {
            left: vec![1, 2],
            right: vec![1, 3]
        }]
    );
    assert_eq!(
        var("value").reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 1,
            first_index: 4,
            max_abs_difference: Some(5.)
        })]
    );
    assert_eq!(var("cellv").children[0].status, DiffStatus::Same);
    assert_eq!(
        var("cellv").children[1].reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 1,
            first_index: 1,
            max_abs_difference: Some(3.)
        })]
    );
    assert_eq!(var("chars").status, DiffStatus::Differs);
    assert_eq!(
        var("chars").reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 2,
            first_index: 1,
            max_abs_difference: None,
        })]
    );
    let report = render_report(&Comparison::Mat(d), "left", "right", &c).unwrap();
    assert!(report.starts_with("Variable\tStatus\tDetails\n"));
    assert!(
        report
            .contains("value\tDiffers\tvalues: 1 differing, first index 4, max abs difference 5\n")
    );
    assert!(report.contains("cellv.{2}\tDiffers"));
    e.execute("visdiff('left.mat','right.mat')").unwrap();
    let same = mat_diff(
        &temp.path().join("right.mat"),
        &temp.path().join("right.mat"),
        &c,
    )
    .unwrap();
    assert!(same.variables.iter().all(|v| v.status == DiffStatus::Same));
}

#[test]
fn mat_v5_engine_save_comparison() {
    save_pair("-v7");
    save_pair("-v6");
}
#[cfg(feature = "mat-v73")]
#[test]
fn mat_v73_engine_save_comparison() {
    // HDF5 descriptors lack close-on-exec in the existing codecs. Keep them
    // out of the suite process, where unrelated tests concurrently spawn CLI
    // children (see the grouping rules in tool/suites-assign.py).
    const CHILD: &str = "MATRUST_VISDIFF_V73_CHILD";
    if std::env::var_os(CHILD).is_some() {
        save_pair("-v7.3");
        empty_mat_comparison("-v7.3");
        return;
    }
    let result = Command::new(std::env::current_exe().unwrap())
        .env(CHILD, "1")
        .args([
            "--exact",
            "visdiff::mat_v73_engine_save_comparison",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn arrays_nested_struct_fields_cells_strings_and_numeric_metadata() {
    let mut e = Engine::default();
    e.execute("a=struct('nested',struct('value',1),'removed',2); b=struct('nested',struct('value',9),'added',3);").unwrap();
    let c = cancel();
    let d = array_diff("s", &e.array("a").unwrap(), &e.array("b").unwrap(), &c).unwrap();
    assert_eq!(d.status, DiffStatus::Differs);
    assert_eq!(
        d.children
            .iter()
            .map(|x| (x.name.as_str(), x.status))
            .collect::<Vec<_>>(),
        vec![
            ("added", DiffStatus::OnlyRight),
            ("nested", DiffStatus::Differs),
            ("removed", DiffStatus::OnlyLeft)
        ]
    );
    assert_eq!(
        d.children[1].children[0].children[0].children[0].reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 1,
            first_index: 1,
            max_abs_difference: Some(8.)
        })]
    );
    e.execute("a=[1+2i 3+NaN*i]; b=[4+6i 3+NaN*i];").unwrap();
    let d = array_diff("z", &e.array("a").unwrap(), &e.array("b").unwrap(), &c).unwrap();
    assert_eq!(
        d.reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 1,
            first_index: 1,
            max_abs_difference: Some(5.)
        })]
    );
    e.execute("a=[1 2]; b=complex(a); s=sparse(a); strings1=string({'a','b'}); strings2=string({'a','c'});").unwrap();
    assert_eq!(
        array_diff("x", &e.array("a").unwrap(), &e.array("b").unwrap(), &c)
            .unwrap()
            .reasons,
        vec![DifferenceReason::Complexity]
    );
    assert_eq!(
        array_diff("x", &e.array("a").unwrap(), &e.array("s").unwrap(), &c)
            .unwrap()
            .reasons,
        vec![DifferenceReason::Sparsity]
    );
    assert_eq!(
        array_diff(
            "x",
            &e.array("strings1").unwrap(),
            &e.array("strings2").unwrap(),
            &c
        )
        .unwrap()
        .reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 1,
            first_index: 2,
            max_abs_difference: None
        })]
    );
    let temp = tempfile::tempdir().unwrap();
    e.set_current_folder(temp.path()).unwrap();
    e.execute("st=struct('nested',struct('x',1)); save('left.mat','st'); st.nested.x=2; save('right.mat','st');").unwrap();
    let changed = mat_diff(
        &temp.path().join("left.mat"),
        &temp.path().join("right.mat"),
        &c,
    )
    .unwrap();
    assert_eq!(changed.variables[0].status, DiffStatus::Differs);
    let nested = &changed.variables[0].children[0];
    assert_eq!(nested.name, "nested");
    let field = &nested.children[0].children[0];
    assert_eq!(field.name, "x");
    assert_eq!(field.children[0].status, DiffStatus::Differs);
    let report = render_report(&Comparison::Mat(changed), "left.mat", "right.mat", &c).unwrap();
    assert!(report.contains("st.nested.(1).x.(1)"), "{report}");
}

#[test]
fn numeric_exact_uint64_sparse_implicit_zeros_and_nan() {
    use matrust::{Matrix, NumericClass, TypedArray};
    let c = cancel();
    let make = |v| {
        Array::Typed(
            TypedArray::from_ints(NumericClass::Uint64, 1, 1, std::iter::once(i128::from(v)))
                .unwrap(),
        )
    };
    let d = array_diff("x", &make(u64::MAX), &make(u64::MAX - 1), &c).unwrap();
    assert_eq!(
        d.reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 1,
            first_index: 1,
            max_abs_difference: Some(1.)
        })]
    );
    let a = matrust::sparse::SparseMatrix::from_dense(
        &Matrix::from_column_vec(2, 2, vec![0., f64::NAN, 0., 4.]).unwrap(),
        &c,
    )
    .unwrap();
    let b = matrust::sparse::SparseMatrix::from_dense(
        &Matrix::from_column_vec(2, 2, vec![3., f64::NAN, 0., 0.]).unwrap(),
        &c,
    )
    .unwrap();
    let d = array_diff("x", &Array::Sparse(a), &Array::Sparse(b), &c).unwrap();
    assert_eq!(
        d.reasons,
        vec![DifferenceReason::Values(ValueDifference {
            count: 2,
            first_index: 1,
            max_abs_difference: Some(4.)
        })]
    );
}

#[test]
fn folders_binary_and_file_kind_errors() {
    let temp = tempfile::tempdir().unwrap();
    let (a, b) = (temp.path().join("left"), temp.path().join("right"));
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    for (dir, name, bytes) in [
        (&a, "left", b"l".as_slice()),
        (&b, "right", b"r"),
        (&a, "same", b"s"),
        (&b, "same", b"s"),
        (&a, "different", b"123"),
        (&b, "different", b"12x"),
    ] {
        fs::write(dir.join(name), bytes).unwrap();
    }
    fs::create_dir(a.join("ignored")).unwrap();
    fs::write(a.join("ignored/nested"), "ignore").unwrap();
    let d = folder_diff(&a, &b).unwrap();
    assert_eq!(
        d.files
            .iter()
            .map(|f| (f.name.to_str().unwrap(), f.status))
            .collect::<Vec<_>>(),
        vec![
            ("different", DiffStatus::Differs),
            ("left", DiffStatus::OnlyLeft),
            ("right", DiffStatus::OnlyRight),
            ("same", DiffStatus::Same)
        ]
    );
    assert_eq!(
        render_report(&Comparison::Folder(d), "a", "b", &cancel()).unwrap(),
        "Differs\tdifferent\nOnlyLeft\tleft\nOnlyRight\tright\nSame\tsame\n"
    );
    assert_eq!(
        binary_diff(&a.join("different"), &b.join("different"), &cancel())
            .unwrap()
            .first_offset,
        Some(2)
    );
    fs::write(b.join("different"), b"1234").unwrap();
    assert_eq!(
        binary_diff(&a.join("different"), &b.join("different"), &cancel())
            .unwrap()
            .first_offset,
        Some(3)
    );
    fs::write(a.join("different"), b"1234").unwrap();
    assert_eq!(
        binary_diff(&a.join("different"), &b.join("different"), &cancel())
            .unwrap()
            .first_offset,
        None
    );
    assert!(
        compare_paths(&a, &b.join("same"), ComparisonType::Auto, &cancel())
            .unwrap_err()
            .contains("file with a folder")
    );
}

fn cli(dir: &Path, source: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_matrust"))
        .current_dir(dir)
        .args(["-batch", source])
        .output()
        .unwrap()
}

#[test]
fn builtin_exact_cli_report_missing_file_and_output_errors() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("left.txt"), "a\nold\nc\n").unwrap();
    fs::write(temp.path().join("right.txt"), "a\nnew\nc\n").unwrap();
    for source in [
        "visdiff('left.txt','right.txt')",
        "visdiff('left.txt','right.txt','text');",
        "visdiff(\"left.txt\",\"right.txt\",\"text\");",
    ] {
        let result = cli(temp.path(), source);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            "--- left.txt\n+++ right.txt\n@@ -1,3 +1,3 @@\n a\n-old\n+new\n c\n"
        );
        assert!(result.stderr.is_empty());
    }
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    let missing = cli(temp.path(), "visdiff('absent.txt','right.txt')");
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("absent.txt"));
    let error = e.execute("visdiff('absent.txt','right.txt')").unwrap_err();
    assert!(error.contains("Could not open file '") && error.contains("absent.txt"));
    for source in [
        "x=visdiff('left.txt','right.txt')",
        "[x,y]=visdiff('left.txt','right.txt')",
    ] {
        assert!(
            e.execute(source)
                .unwrap_err()
                .contains("Too many output arguments.")
        );
    }
    assert!(
        e.execute("visdiff('left.txt','right.txt','xml')")
            .unwrap_err()
            .contains("Unsupported visdiff")
    );
    assert!(
        e.execute("visdiff('left.txt','right.txt','binary')")
            .unwrap()
            .output
            .join("\n")
            .contains("byte offset 2")
    );
    assert!(
        e.execute("visdiff('left.txt','left.txt')")
            .unwrap()
            .output
            .is_empty()
    );
    assert!(matrust::session::FUNCTIONS.contains(&"visdiff"));
    assert!(
        !matrust::engine::compare_builtin::open_comparison_tool(
            &mut e,
            temp.path(),
            temp.path(),
            &Comparison::Binary(BinaryDiff {
                left_bytes: 0,
                right_bytes: 0,
                first_offset: None
            })
        )
        .unwrap()
    );
}

fn empty_mat_comparison(version: &str) {
    let temp = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(temp.path()).unwrap();
    e.execute(&format!(
        "save('empty.mat','{version}'); x=1; save('full.mat','{version}');"
    ))
    .unwrap();
    let empty = temp.path().join("empty.mat");
    let full = temp.path().join("full.mat");
    assert!(
        mat_diff(&empty, &empty, &cancel())
            .unwrap()
            .variables
            .is_empty()
    );
    for (left, right, expected) in [
        (&empty, &full, DiffStatus::OnlyRight),
        (&full, &empty, DiffStatus::OnlyLeft),
    ] {
        let d = mat_diff(left, right, &cancel()).unwrap();
        assert_eq!(d.variables.len(), 1);
        assert_eq!(d.variables[0].name, "x");
        assert_eq!(d.variables[0].status, expected);
    }
}

#[test]
fn mat_empty_variable_maps() {
    empty_mat_comparison("-v6");
    empty_mat_comparison("-v7");
}

#[test]
fn builtin_report_uses_diary_and_pending_prefix() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("left.txt"), "old\n").unwrap();
    fs::write(temp.path().join("right.txt"), "new\n").unwrap();
    let result = cli(
        temp.path(),
        "diary('session.txt'); fprintf('prefix'); visdiff('left.txt','right.txt'); diary off;",
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected = "prefix--- left.txt\n+++ right.txt\n@@ -1 +1 @@\n-old\n+new\n";
    assert_eq!(String::from_utf8(result.stdout).unwrap(), expected);
    assert_eq!(
        fs::read_to_string(temp.path().join("session.txt")).unwrap(),
        expected
    );
}

#[test]
fn builtin_report_obeys_output_budgets() {
    use matrust::engine::Limits;
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("left.txt"), "old\n").unwrap();
    fs::write(temp.path().join("right.txt"), "new\n").unwrap();
    for limits in [
        Limits {
            output_bytes: Some(1),
            ..Limits::local()
        },
        Limits {
            output_entries: Some(1),
            ..Limits::local()
        },
    ] {
        let mut e = Engine::with_limits(limits);
        e.set_current_folder(temp.path()).unwrap();
        let error = e
            .execute("visdiff('left.txt','right.txt'); visdiff('left.txt','right.txt');")
            .unwrap_err();
        assert!(error.contains("output"), "{error}");
    }
}
