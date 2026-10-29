//! Real background jobs while a different viewport owns the shared Context.
use super::*;
use eframe::egui::{self, ViewportId};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const WAIT: Duration = Duration::from_secs(10);

fn settled_context() -> egui::Context {
    let context = egui::Context::default();
    // Consume initial ROOT demand before observing any background job.
    for _ in 0..3 {
        let _ = context.run_logic(&egui::RawInput::default(), |_| {});
    }
    assert!(!context.has_requested_repaint_for(&ViewportId::ROOT));
    context
}

fn child_input(child: ViewportId) -> egui::RawInput {
    egui::RawInput {
        viewport_id: child,
        viewports: [
            (ViewportId::ROOT, egui::ViewportInfo::default()),
            (
                child,
                egui::ViewportInfo {
                    parent: Some(ViewportId::ROOT),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    }
}

#[test]
fn worker_completion_wakes_root_while_child_viewport_is_active() {
    let context = settled_context();
    let mut worker = Worker::new(context.clone());
    let owner = worker.thread.as_ref().unwrap().thread().id();
    let (send, wakes) = mpsc::channel();
    // This fresh headless Context has no native backend callback to replace.
    context.set_request_repaint_callback(move |request| {
        if std::thread::current().id() == owner {
            let _ = send.send(request.viewport_id);
        }
    });
    let child = ViewportId::from_hash_of("completion-child");
    let mut submitted = false;
    let _ = context.run_ui(child_input(child), |ui| {
        if submitted {
            return;
        }
        submitted = true;
        assert_eq!(ui.ctx().viewport_id(), child);
        assert!(!ui.ctx().has_requested_repaint_for(&ViewportId::ROOT));
        worker
            .send
            .send(Job::Script("A=[1 2;3 4];Alias=A;".into()).into())
            .unwrap();
        let reply = worker.receive.recv_timeout(WAIT).unwrap();
        assert_eq!(wakes.recv_timeout(WAIT).unwrap(), ViewportId::ROOT);
        assert!(reply.result.is_ok());
        assert_eq!(reply.vars["A"].as_numeric().unwrap().get(1, 0), 3.0);
        assert!(reply.vars["A"].same_snapshot(&reply.vars["Alias"]));
        assert!(worker.receive.try_recv().is_err());
    });
    assert!(submitted);
    worker.shutdown();
}

#[test]
fn worker_export_progress_wakes_root_before_reply_and_retains_stop() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine
        .execute("figure(1);plot([1 2]);figure(2);plot([3 4]);")
        .unwrap();
    let plan = super::super::export_batch::prepare(
        super::super::export_batch::Prepare {
            folder: directory.path().into(),
            figures: engine
                .figures()
                .map(|(id, figure)| (id, figure.clone()))
                .collect(),
            settings: matrust::session::ExportSettings {
                format: matrust::session::ExportFormat::Csv,
                ..Default::default()
            },
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(plan.items.len(), 2);
    let first = plan.items[0].path.clone();
    let second = plan.items[1].path.clone();
    let progress = Arc::new(AtomicUsize::new(0));
    let observed_progress = progress.clone();
    let context = settled_context();
    let mut worker = Worker::new(context.clone());
    let owner = worker.thread.as_ref().unwrap().thread().id();
    let (send, wakes) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    let first_wake = AtomicBool::new(true);
    context.set_request_repaint_callback(move |request| {
        if std::thread::current().id() == owner && first_wake.swap(false, Ordering::Relaxed) {
            let _ = send.send((
                request.viewport_id,
                observed_progress.load(Ordering::Relaxed),
            ));
            // Never touch Context while this callback holds its internal lock.
            // Bound the gate even if the test panics before releasing it.
            let _ = gate.lock().unwrap().recv_timeout(WAIT);
        }
    });
    let child = ViewportId::from_hash_of("export-child");
    let mut request = Some(super::super::export_batch::Run {
        plan,
        completed: 0,
        replace: false,
        progress: progress.clone(),
    });
    let _ = context.run_ui(child_input(child), |ui| {
        let Some(request) = request.take() else {
            return;
        };
        assert_eq!(ui.ctx().viewport_id(), child);
        assert!(!ui.ctx().has_requested_repaint_for(&ViewportId::ROOT));
        worker
            .send
            .send(Job::ExportBatch(Box::new(request)).into())
            .unwrap();
        let wake = wakes.recv_timeout(WAIT);
        let reply_pending = matches!(worker.receive.try_recv(), Err(mpsc::TryRecvError::Empty));
        let first_published = first.is_file();
        let second_unpublished = !second.exists();
        worker.cancel.store(true, Ordering::Relaxed);
        let _ = release.send(());
        // Assertions follow release so a failed assertion cannot strand the worker.
        assert_eq!(wake.unwrap(), (ViewportId::ROOT, 1));
        assert!(reply_pending && first_published && second_unpublished);
        let reply = worker.receive.recv_timeout(WAIT).unwrap();
        assert!(reply.result.is_err());
        let outcome = reply.export_outcome.unwrap();
        assert_eq!(outcome.completed, 1);
        assert!(outcome.error.unwrap().contains("Cancelled"));
        assert_eq!(progress.load(Ordering::Relaxed), 1);
        assert!(first.is_file() && !second.exists());
    });
    assert!(request.is_none());
    worker.shutdown();
}
