use matrust::{Engine, files};
use std::sync::atomic::{AtomicBool, Ordering};

fn engine(path: &std::path::Path) -> Engine {
    let mut engine = Engine::default();
    engine.set_file_directory(path).unwrap();
    engine
}

#[test]
fn script_figure_files_roundtrip_formats_handles_and_number_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("f=figure;plot([1 3 2]);title('original');n=gcf;savefig(f,'first');savefig native.mfig;close all;g=openfig('first.fig');old_live=isgraphics(f);new_live=isgraphics(g);number=get(g,'Number');").unwrap();
    let handle = |name: &str| {
        let matrust::Array::Graphics(h) = e.array(name).unwrap() else {
            panic!("Expected figure handle");
        };
        assert_eq!(
            h.scalar_handle().unwrap().kind(),
            matrust::GraphicsKind::Figure
        );
        h
    };
    assert!(
        handle("f")
            .scalar_handle()
            .unwrap()
            .same_object(handle("n").scalar_handle().unwrap())
    );
    assert!(
        !handle("f")
            .scalar_handle()
            .unwrap()
            .same_object(handle("g").scalar_handle().unwrap())
    );
    assert_eq!(e.current_figure, 1);
    assert_eq!(e.logical_variables["old_live"].data(), &[0]);
    assert_eq!(e.logical_variables["new_live"].data(), &[1]);
    assert_eq!(e.variables["number"].get(0, 0), 1.);
    assert_eq!(e.figure.as_ref().unwrap().title, "original");
    let events = e.take_figure_file_events();
    assert_eq!(events.len(), 1);
    assert!(events[0].opened && events[0].matches(e.figure.as_ref().unwrap()));
    assert_eq!(
        files::figure_version::inspect(&events[0].path, &AtomicBool::new(false)).unwrap(),
        files::figure_version::DiskVersion::Present(events[0].version)
    );
    e.execute("openfig native.mfig;figure(1);title('changed');savefig(2,'second.mfig');")
        .unwrap();
    assert_eq!(e.current_figure, 1);
    let (saved, _) =
        files::figure_version::load(&dir.path().join("second.mfig"), &AtomicBool::new(false))
            .unwrap();
    assert_eq!(saved.title, "original");
    assert_eq!(e.figure.as_ref().unwrap().title, "changed");
}

#[test]
fn script_figure_files_events_survive_later_errors_but_do_not_retain_replaced_samples() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("plot(1:1000);").unwrap();
    let old = (e.figure.as_ref().unwrap().series[0].points).downgrade();
    assert!(
        e.execute("savefig original.mfig;plot(1:2000);unknown_function();")
            .is_err()
    );
    assert!(old.upgrade().is_none());
    let event = e.take_figure_file_events().pop().unwrap();
    assert!(!event.matches(e.figure.as_ref().unwrap()));
    let (saved, _) = files::figure_version::load(&event.path, &AtomicBool::new(false)).unwrap();
    assert_eq!(saved.series[0].points.len(), 1000);
    e.execute("savefig current.mfig;title('later');").unwrap();
    assert!(!e.take_figure_file_events()[0].matches(e.figure.as_ref().unwrap()));
    e.execute("savefig current.mfig;close;figure(1);plot([9 8]);")
        .unwrap();
    assert!(e.take_figure_file_events().is_empty());
}

#[test]
fn script_figure_files_reject_options_bad_files_outputs_and_cancel_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("plot([1 2]);savefig kept.mfig;").unwrap();
    let original = e.figure.clone().unwrap();
    let bytes = std::fs::read(dir.path().join("kept.mfig")).unwrap();
    for script in [
        "savefig('kept.mfig','-v7.3')",
        "savefig(999,'kept.mfig')",
        "savefig('bad.png')",
        "result=savefig('new.mfig')",
        "A(1)=savefig('new.mfig')",
        "openfig('kept.mfig','reuse')",
        "openfig('kept.mfig','invisible')",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert!(e.figure.as_ref().unwrap().same_snapshot(&original));
        assert_eq!(std::fs::read(dir.path().join("kept.mfig")).unwrap(), bytes);
        assert!(!dir.path().join("new.mfig").exists());
        assert!(e.take_figure_file_events().is_empty());
    }
    std::fs::write(dir.path().join("broken.fig"), b"broken").unwrap();
    assert!(e.execute("openfig broken.fig").is_err());
    assert!(e.figure.as_ref().unwrap().same_snapshot(&original));
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("savefig cancelled.mfig").is_err());
    assert!(!dir.path().join("cancelled.mfig").exists());
}

#[test]
fn script_figure_files_defaults_quoted_paths_scope_shadowing_and_limits() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("f=figure;plot([2 4]);savefig;savefig '%quote'' name.mfig';close;openfig '%quote'' name.mfig';").unwrap();
    assert!(dir.path().join("Untitled.fig").exists());
    e.execute("call_helper();function call_helper();savefig helper.mfig;end")
        .unwrap();
    assert!(dir.path().join("helper.mfig").exists());
    e.execute("gcf=17;savefig=2;openfig=3;x=gcf;savefig + openfig;")
        .unwrap();
    assert_eq!(e.variables["ans"].get(0, 0), 5.);
    e.execute("clear;for i=1:16;figure(i);end;").unwrap();
    assert_eq!(e.figure_ids().len(), 16);
    assert!(
        e.execute("openfig helper.mfig")
            .unwrap_err()
            .contains("16 figures")
    );
    assert_eq!(e.figure_ids().len(), 16);
}
