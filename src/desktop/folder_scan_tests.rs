use super::*;
#[test]
fn folder_scan_large_listing_filter_sort_and_bounds() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..2505 {
        std::fs::write(dir.path().join(format!("sample-{n:04}.m")), b"").unwrap();
    }
    std::fs::create_dir(dir.path().join("Z_directory")).unwrap();
    std::fs::write(dir.path().join("Alpha.M"), b"").unwrap();
    let cancel = AtomicBool::new(false);
    let full = scan(dir.path(), "", &cancel).unwrap();
    assert_eq!(
        (full.entries.len(), full.visited, full.truncated),
        (2507, 2507, false)
    );
    assert_eq!(full.entries[0].name, "Z_directory");
    assert_eq!(full.entries[1].name, "Alpha.M");
    let filtered = scan(dir.path(), "ALPHA.m", &cancel).unwrap();
    assert_eq!(filtered.entries.len(), 1);
    assert_eq!(filtered.entries[0].name, "Alpha.M");
    assert_eq!(filtered.visited, 2507);
    let limited = scan_limited(dir.path(), "", &cancel, 20, MAX_VISITED, MAX_BYTES).unwrap();
    assert_eq!(limited.entries.len(), 20);
    assert!(limited.truncated);
    let visited =
        scan_limited(dir.path(), "no-match", &cancel, MAX_ENTRIES, 10, MAX_BYTES).unwrap();
    assert_eq!(visited.visited, 10);
    assert!(visited.entries.is_empty() && visited.truncated);
    let bytes = scan_limited(dir.path(), "", &cancel, MAX_ENTRIES, MAX_VISITED, 1).unwrap();
    assert!(bytes.entries.is_empty() && bytes.truncated);
}
#[test]
fn folder_scan_cancel_and_invalid_inputs_do_not_publish_partial_results() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(true);
    assert_eq!(scan(dir.path(), "", &cancel).err().unwrap(), "Cancelled");
    cancel.store(false, Ordering::Relaxed);
    assert!(scan(&dir.path().join("absent"), "", &cancel).is_err());
    assert!(scan(dir.path(), &"a".repeat(1025), &cancel).is_err());
    assert!(
        scan(dir.path(), "empty", &cancel)
            .unwrap()
            .entries
            .is_empty()
    );
}
#[test]
fn folder_scan_latest_request_wins_and_queue_remains_bounded() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("last.m"), b"").unwrap();
    let mut scanner = Scanner::new(eframe::egui::Context::default());
    for n in 0..200 {
        scanner.request(dir.path().into(), format!("obsolete-{n}"));
    }
    let generation = scanner.request(dir.path().into(), "last".into());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(reply) = scanner.take() {
            assert_eq!(reply.generation, generation);
            assert_eq!(reply.result.unwrap().entries[0].name, "last.m");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    scanner.request(dir.path().into(), "last".into());
    scanner.cancel();
    assert!(scanner.take().is_none());
    assert!(scanner.state.0.lock().unwrap().pending.is_none());
}

#[test]
fn folder_completion_wakes_root_while_child_viewport_is_active() {
    use eframe::egui::{self, ViewportId};
    use std::sync::mpsc;
    use std::time::Duration;

    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("ready.m"), b"A=1;").unwrap();
    let context = egui::Context::default();
    for _ in 0..3 {
        let _ = context.run_logic(&egui::RawInput::default(), |_| {});
    }
    assert!(!context.has_requested_repaint_for(&ViewportId::ROOT));
    let mut scanner = Scanner::new(context.clone());
    let owner = scanner.thread.as_ref().unwrap().thread().id();
    let (send, wakes) = mpsc::channel();
    context.set_request_repaint_callback(move |request| {
        if std::thread::current().id() == owner {
            let _ = send.send(request.viewport_id);
        }
    });
    let child = ViewportId::from_hash_of("folder-child");
    let input = egui::RawInput {
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
    };
    let mut submitted = false;
    let _ = context.run_ui(input, |ui| {
        if submitted {
            return;
        }
        submitted = true;
        assert_eq!(ui.ctx().viewport_id(), child);
        assert!(!ui.ctx().has_requested_repaint_for(&ViewportId::ROOT));
        let generation = scanner.request(directory.path().into(), "ready".into());
        assert_eq!(
            wakes.recv_timeout(Duration::from_secs(10)).unwrap(),
            ViewportId::ROOT
        );
        let reply = scanner.take().unwrap();
        assert_eq!(reply.generation, generation);
        assert!(reply.directory.is_none());
        let listing = reply.result.unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "ready.m");
        assert_eq!(listing.entries[0].path, directory.path().join("ready.m"));
        assert!(!listing.truncated);
        assert!(scanner.take().is_none());
    });
    assert!(submitted);
    let thread = scanner.thread.take().unwrap();
    drop(scanner);
    thread.join().unwrap();
}
