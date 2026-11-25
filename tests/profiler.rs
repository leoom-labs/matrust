//! Contract fixtures derived from MathWorks' public profile/profsave pages.
//! Counts follow the source below, times only use inequalities; no MATLAB oracle.
use matrust::{Array, Engine, StructArray};
use std::{fs, sync::atomic::AtomicBool};

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in files {
        fs::write(dir.path().join(name), source).unwrap();
    }
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    e.set_file_directory(dir.path()).unwrap();
    (dir, e)
}
fn structure(a: &Array) -> &StructArray {
    let Array::Struct(s) = a else {
        panic!("expected struct: {a:?}")
    };
    s
}
fn field<'a>(s: &'a StructArray, i: usize, name: &str) -> &'a Array {
    s.get(s.field_index(name).unwrap(), i).unwrap()
}
fn num(a: &Array) -> f64 {
    let Array::Numeric(m) = a else {
        panic!("expected double")
    };
    m.as_scalar().unwrap()
}
fn text(a: &Array) -> String {
    let Array::Character(c) = a else {
        panic!("expected char")
    };
    c.to_utf8(&AtomicBool::new(false)).unwrap()
}
fn snapshot(e: &mut Engine) -> StructArray {
    e.execute("p=profile('info');").unwrap();
    structure(&e.array("p").unwrap()).clone()
}
fn table(p: &StructArray) -> &StructArray {
    structure(field(p, 0, "FunctionTable"))
}
fn row(t: &StructArray, name: &str) -> usize {
    (0..t.len())
        .find(|&i| text(field(t, i, "FunctionName")) == name)
        .unwrap_or_else(|| panic!("missing {name}: {t:?}"))
}
fn numeric(a: &Array) -> &matrust::Matrix {
    let Array::Numeric(m) = a else {
        panic!("expected numeric matrix")
    };
    m
}

#[test]
fn function_graph_lines_and_builtin_shapes() {
    let (_dir, mut e) = fixture(&[(
        "main.m",
        "function y=main()\ny=0;\nfor k=1:3\ny=y+sub(k);\nend\nend\nfunction y=sub(x)\n% UTF-8 byte offset: café λ\na=x+1;\nb=a*2;\nc=b-1;\ny=sum([c x]);\nend\n",
    )]);
    e.execute("profile on -history; y=main(); profile off;")
        .unwrap();
    let p = snapshot(&mut e);
    let t = table(&p);
    let main = row(t, "main");
    let sub = row(t, "main>sub");
    let sum = row(t, "sum");
    assert_eq!(num(field(t, main, "NumCalls")), 1.);
    assert_eq!(num(field(t, sub, "NumCalls")), 3.);
    assert_eq!(num(field(t, sum, "NumCalls")), 3.);
    assert_eq!(text(field(t, sub, "Type")), "M-subfunction");
    let children = structure(field(t, main, "Children"));
    assert_eq!(
        num(field(children, 0, "FunctionTableIndex")),
        (sub + 1) as f64
    );
    assert_eq!(num(field(children, 0, "Index")), (sub + 1) as f64);
    assert_eq!(num(field(children, 0, "NumCalls")), 3.);
    let parents = structure(field(t, sub, "Parents"));
    assert_eq!(
        num(field(parents, 0, "FunctionTableIndex")),
        (main + 1) as f64
    );
    assert_eq!(num(field(parents, 0, "NumCalls")), 3.);
    let lines = numeric(field(t, sub, "ExecutedLines"));
    assert_eq!((lines.rows(), lines.cols()), (4, 3));
    assert_eq!(&lines.data()[..8], &[9., 10., 11., 12., 3., 3., 3., 3.]);
    let lines = numeric(field(t, sum, "ExecutedLines"));
    assert_eq!((lines.rows(), lines.cols()), (0, 3));
    let lines = numeric(field(t, main, "ExecutedLines"));
    assert_eq!(&lines.data()[..6], &[2., 3., 4., 1., 1., 3.]);
    let history = numeric(field(&p, 0, "FunctionHistory"));
    assert_eq!((history.rows(), history.cols()), (2, 14));
    let events: Vec<_> = history
        .data()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| p[0])
        .collect();
    assert_eq!(events.iter().filter(|&&n| n == 0.).count(), 7);
    assert_eq!(events.iter().filter(|&&n| n == 1.).count(), 7);
}

#[test]
fn inclusive_times_recursion_and_error_unwind() {
    let (_dir, mut e) = fixture(&[
        (
            "parent.m",
            "function y=parent()\ny=child();\nend\nfunction y=child()\npause(0.05);\ny=1;\nend\n",
        ),
        (
            "fact.m",
            "function y=fact(n)\nif n<=1\ny=1;\nelse\ny=n*fact(n-1);\nend\nend\n",
        ),
        (
            "broken.m",
            "function broken()\nx=1;\nerror('fixture failure');\nend\n",
        ),
    ]);
    e.execute("profile on -history; y=parent(); z=fact(5); try, broken(); catch, caught=1; end;")
        .unwrap();
    let p = snapshot(&mut e);
    let t = table(&p);
    let parent = row(t, "parent");
    let child = row(t, "parent>child");
    assert!(num(field(t, child, "TotalTime")) >= 0.05);
    assert!(num(field(t, parent, "TotalTime")) >= num(field(t, child, "TotalTime")));
    let fact = row(t, "fact");
    assert_eq!(num(field(t, fact, "NumCalls")), 5.);
    assert_eq!(num(field(t, fact, "IsRecursive")), 1.);
    assert_eq!(num(field(t, row(t, "broken"), "NumCalls")), 1.);
    assert_eq!(e.variables["z"].as_scalar().unwrap(), 120.);
    for i in 0..t.len() {
        let time = num(field(t, i, "TotalTime"));
        assert!(time.is_finite() && time >= 0.);
        assert!(
            numeric(field(t, i, "ExecutedLines"))
                .data()
                .iter()
                .all(|n| n.is_finite() && *n >= 0.)
        );
    }
    let history = numeric(field(&p, 0, "FunctionHistory"));
    let mut stack = Vec::new();
    for event in history.data().as_chunks::<2>().0.iter() {
        if event[0] == 0. {
            stack.push(event[1]);
        } else {
            assert_eq!(stack.pop(), Some(event[1]));
        }
    }
    assert!(stack.is_empty());
}

#[test]
fn status_options_resume_clear_and_errors() {
    let (_dir, mut e) = fixture(&[]);
    e.execute("s=profile('status');").unwrap();
    let s = e.array("s").unwrap();
    let s = structure(&s);
    for (key, expected) in [
        ("ProfilerStatus", "off"),
        ("DetailLevel", "mmex"),
        ("Timer", "performance"),
        ("HistoryTracking", "off"),
    ] {
        assert_eq!(text(field(s, 0, key)), expected);
    }
    assert_eq!(num(field(s, 0, "HistorySize")), 1_000_000.);
    e.execute("profile on -history -historysize 3 -timer cpu; a=sum([1 2]); profile off; profile resume; b=sum([3 4]); s=profile('status');").unwrap();
    let s = e.array("s").unwrap();
    let s = structure(&s);
    assert_eq!(text(field(s, 0, "ProfilerStatus")), "on");
    assert_eq!(text(field(s, 0, "Timer")), "cpu");
    assert_eq!(text(field(s, 0, "HistoryTracking")), "on");
    let p = snapshot(&mut e);
    let t = table(&p);
    assert_eq!(num(field(t, row(t, "sum"), "NumCalls")), 2.);
    assert_eq!(numeric(field(&p, 0, "FunctionHistory")).cols(), 3);
    e.execute("profile('on','-nohistory','-timer','real'); c=sum([5 6]);")
        .unwrap();
    let p = snapshot(&mut e);
    let t = table(&p);
    assert_eq!(num(field(t, row(t, "sum"), "NumCalls")), 1.);
    assert_eq!(numeric(field(&p, 0, "FunctionHistory")).cols(), 0);
    e.execute("profile clear; a=sum([8 9]);").unwrap();
    let p = snapshot(&mut e);
    assert_eq!((table(&p).rows(), table(&p).cols()), (0, 1));
    for source in [
        "profile",
        "profile()",
        "profile nonsense",
        "profile off extra",
        "profile on -bad",
        "profile on -timer",
        "profile on -timer wrong",
        "profile('on','-historysize',-1)",
        "profile('on','-historysize',1.5)",
        "profile('on','-historysize',Inf)",
        "profile('on','-historysize',1000001)",
        "[a,b]=profile('status')",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
    }
    e.execute("profile -history; profile on;").unwrap();
    assert!(e.execute("profile -nohistory").is_err());
}

#[test]
fn anonymous_scripts_html_and_viewer() {
    let (dir, mut e) = fixture(&[("work.m", "x=1 < 2;\nh=@(v) sum(v);\ny=h([1 2]);\n")]);
    e.execute("profile on; work; profile off;").unwrap();
    let p = snapshot(&mut e);
    let t = table(&p);
    assert_eq!(text(field(t, row(t, "work"), "Type")), "M-script");
    assert!((0..t.len()).any(|i| text(field(t, i, "Type")) == "M-anonymous-function"));
    assert_eq!(t.len(), 3);
    e.execute("profsave(p,'export'); profsave(p); profsave;")
        .unwrap();
    let summary = fs::read_to_string(dir.path().join("export/file0.html")).unwrap();
    assert!(summary.contains("Self time"));
    let detail = fs::read_to_string(
        dir.path()
            .join(format!("export/file{}.html", row(t, "work") + 1)),
    )
    .unwrap();
    assert!(detail.contains("x=1 &lt; 2;"));
    assert!(!detail.contains("<script"));
    for i in 0..=t.len() {
        assert!(
            dir.path()
                .join(format!("profile_results/file{i}.html"))
                .exists()
        );
    }
    let output = e.execute("profile viewer").unwrap().output.join("\n");
    assert!(output.contains("Function\tCalls\tTotal time\tSelf time"));
    assert!(output.contains("work"));
    assert!(e.execute("profsave(1)").is_err());
    assert!(e.execute("profsave(p,'export','extra')").is_err());
}

#[test]
fn cancellation_closes_frames_and_history() {
    let (_dir, mut e) = fixture(&[("waiting.m", "function waiting()\npause(10);\nend\n")]);
    e.execute("profile on -history;").unwrap();
    let cancel = e.cancellation();
    let thread = std::thread::spawn(move || {
        let error = e.execute("waiting();").unwrap_err();
        (e, error)
    });
    // pause(10) keeps waiting() open, so a long delay costs nothing; 100 ms let the flag land before
    // the call started on a loaded gate machine (Windows gate 2026-11-22: "missing waiting").
    std::thread::sleep(std::time::Duration::from_millis(1000));
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let (mut e, error) = thread.join().unwrap();
    assert!(error.contains("cancel"), "{error}");
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let p = snapshot(&mut e);
    let t = table(&p);
    assert_eq!(num(field(t, row(t, "waiting"), "NumCalls")), 1.);
    let history = numeric(field(&p, 0, "FunctionHistory"));
    assert_eq!(history.cols(), 4);
    assert_eq!(
        history
            .data()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| c[0])
            .collect::<Vec<_>>(),
        [0., 0., 1., 1.]
    );
}

#[test]
fn stopping_and_clearing_inside_active_frames_is_balanced() {
    let (_dir, mut e) = fixture(&[(
        "switcher.m",
        "function switcher()\nprofile off;\nprofile clear;\nprofile on;\nx=sum([1 2]);\nend\n",
    )]);
    e.execute("profile on -history; switcher();").unwrap();
    let p = snapshot(&mut e);
    let t = table(&p);
    assert_eq!(t.len(), 1);
    assert_eq!(text(field(t, 0, "FunctionName")), "sum");
    assert_eq!(numeric(field(&p, 0, "FunctionHistory")).cols(), 2);
}

#[test]
fn help_examples_and_string_scalar_options() {
    for name in ["profile", "profsave"] {
        let page = matrust::help::page(name).unwrap();
        for example in page
            .lines()
            .filter_map(|line| line.strip_prefix("Example: "))
        {
            let (_dir, mut e) = fixture(&[]);
            e.execute(example).unwrap();
        }
        assert!(matrust::session::FUNCTIONS.contains(&name));
    }
    let (_dir, mut e) = fixture(&[]);
    e.execute("option=string('on'); profile(option, string('-timer'), string('performance')); x=sum([1 2]); p=profile(string('info')); profsave(p,string('saved'));").unwrap();
    let t = e.profiler_info().unwrap();
    let t = table(structure(&t));
    assert_eq!(num(field(t, row(t, "sum"), "NumCalls")), 1.);
}

#[test]
fn specialized_output_and_stopwatch_calls_are_recorded_once() {
    let (_dir, mut e) = fixture(&[]);
    e.execute("profile on -history; tic; t=toc; toc; [v,i]=min([3 1]); [s,j]=sort([2 1]); [a,b]=meshgrid([1 2]); profile off;").unwrap();
    let p = snapshot(&mut e);
    let t = table(&p);
    for (name, count) in [
        ("tic", 1.),
        ("toc", 2.),
        ("min", 1.),
        ("sort", 1.),
        ("meshgrid", 1.),
    ] {
        assert_eq!(num(field(t, row(t, name), "NumCalls")), count, "{name}");
    }
    assert_eq!(t.len(), 5);
}

#[test]
fn eval_text_preserves_caller_lines_and_call_graph() {
    for expression in [
        "eval('x=1; probe();');",
        "evalin('base','x=1; probe();');",
        "evalin('caller','x=1; probe();');",
        "eval('eval(''x=1; probe();'');');",
        "eval('error(''fixture'');','x=1; probe();');",
    ] {
        let source = format!("function driver()\n{expression}\ny=2;\nend\n");
        let (_dir, mut e) = fixture(&[
            ("driver.m", &source),
            ("probe.m", "function probe()\nz=3;\nend\n"),
        ]);
        e.execute("profile on; driver(); profile off;").unwrap();
        let p = snapshot(&mut e);
        let t = table(&p);
        let driver = row(t, "driver");
        let probe = row(t, "probe");
        let lines = numeric(field(t, driver, "ExecutedLines"));
        assert_eq!(lines.rows(), 2, "{expression}: {:?}", lines.data());
        assert_eq!(&lines.data()[..4], &[2., 3., 1., 1.], "{expression}");
        let lines = numeric(field(t, probe, "ExecutedLines"));
        assert_eq!(&lines.data()[..2], &[2., 1.]);
        let children = structure(field(t, driver, "Children"));
        let edge = (0..children.len())
            .find(|&i| num(field(children, i, "FunctionTableIndex")) == (probe + 1) as f64)
            .unwrap();
        assert_eq!(num(field(children, edge, "NumCalls")), 1.);
        let parents = structure(field(t, probe, "Parents"));
        assert_eq!(parents.len(), 1);
        assert_eq!(
            num(field(parents, 0, "FunctionTableIndex")),
            (driver + 1) as f64
        );
        assert_eq!(num(field(parents, 0, "NumCalls")), 1.);
    }
}

#[test]
fn recursive_child_intervals_do_not_overlap() {
    let (_dir, mut e) = fixture(&[
        (
            "fact.m",
            "function y=fact(n)\npause(0.01);\nif n<=1\ny=1;\nelse\ny=n*fact(n-1);\nend\nend\n",
        ),
        (
            "mutual.m",
            "function mutual(n)\npause(0.01);\nif n>1\nother(n-1);\nend\nend\nfunction other(n)\npause(0.01);\nmutual(n);\nend\n",
        ),
    ]);
    e.execute("profile on; y=fact(5); mutual(3); profile off;")
        .unwrap();
    assert_eq!(e.variables["y"].as_scalar().unwrap(), 120.);
    let p = snapshot(&mut e);
    let t = table(&p);
    let fact = row(t, "fact");
    assert_eq!(num(field(t, fact, "NumCalls")), 5.);
    assert_eq!(num(field(t, fact, "IsRecursive")), 1.);
    let children = structure(field(t, fact, "Children"));
    let recursive = (0..children.len())
        .find(|&i| num(field(children, i, "FunctionTableIndex")) == (fact + 1) as f64)
        .unwrap();
    assert_eq!(num(field(children, recursive, "NumCalls")), 4.);
    assert!(num(field(children, recursive, "TotalTime")) >= 0.04);
    for i in 0..t.len() {
        let total = num(field(t, i, "TotalTime"));
        let children = structure(field(t, i, "Children"));
        let child_total: f64 = (0..children.len())
            .map(|j| num(field(children, j, "TotalTime")))
            .sum();
        assert!(
            total - child_total >= 0.,
            "{}: total={total}, children={child_total}",
            text(field(t, i, "FunctionName"))
        );
    }
}
