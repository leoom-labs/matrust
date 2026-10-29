use super::*;
use eframe::egui::{Color32, ColorImage};
use std::{
    fs,
    sync::{Arc, atomic::Ordering},
};
fn preview(path: PathBuf, source: &Path) -> Input {
    Input::Preview(image_export::Request {
        path,
        source: source.into(),
        pixels: Arc::new(ColorImage::filled([64, 32], Color32::from_rgb(17, 83, 149))),
    })
}
fn figure(path: PathBuf) -> Input {
    let mut engine = matrust::Engine::default();
    engine
        .execute("plot([0 1 2],[1 3 2]);title('Captured');")
        .unwrap();
    Input::Figure(Box::new(ExportRequest {
        path,
        figure: engine.figure.unwrap(),
        width: 128,
        height: 96,
        vector: None,
    }))
}
fn entries(folder: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|p| p.unwrap().file_name())
        .collect();
    names.sort();
    names
}
#[test]
fn all_single_preview_formats_stage_without_overwrite_and_detect_equal_length_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("original.svg");
    fs::write(&source, b"original source").unwrap();
    let cancel = AtomicBool::new(false);
    for ext in ["png", "jpg", "bmp", "tiff"] {
        let path = dir.path().join(format!("preview.{ext}"));
        fs::write(&path, b"old!").unwrap();
        let baseline = entries(dir.path());
        let plan = prepare(preview(path.clone(), &source), BaseTheme::Dark, &cancel).unwrap();
        assert!(plan.replaces());
        assert_eq!(fs::read(&path).unwrap(), b"old!");
        fs::write(&path, b"new!").unwrap();
        assert!(
            commit(plan, &cancel)
                .unwrap_err()
                .contains("changed after export preparation")
        );
        assert_eq!(fs::read(&path).unwrap(), b"new!");
        assert_eq!(entries(dir.path()), baseline);
        let plan = prepare(preview(path.clone(), &source), BaseTheme::Light, &cancel).unwrap();
        commit(plan, &cancel).unwrap();
        let decoded = image::open(&path).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (64, 32));
        for (a, b) in decoded.get_pixel(20, 10).0.into_iter().zip([17, 83, 149]) {
            assert!(a.abs_diff(b) <= if ext == "jpg" { 3 } else { 0 });
        }
        fs::remove_file(path).unwrap();
    }
    assert_eq!(fs::read(source).unwrap(), b"original source");
}
#[test]
fn real_codecs_keep_prepared_figure_snapshot_and_existing_destination_until_review() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for ext in ["png", "jpg", "bmp", "tiff", "svg", "pdf", "eps", "csv", "m"] {
        let path = dir.path().join(format!("figure_1.{ext}"));
        fs::write(&path, b"existing").unwrap();
        let input = figure(path.clone());
        let Input::Figure(request) = &input else {
            unreachable!()
        };
        let before = request.figure.clone();
        let alias = request.figure.series[0].points.clone();
        let plan = prepare(input, BaseTheme::Light, &cancel).unwrap();
        assert!(plan.replaces());
        assert_eq!(fs::read(&path).unwrap(), b"existing");
        assert!(before.series[0].points.same_storage(&alias));
        commit(plan, &cancel).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_ne!(bytes, b"existing");
        if ext == "svg" {
            assert!(String::from_utf8(bytes).unwrap().contains("Captured"));
        }
        if ext == "m" {
            let doc = matrust::document::Document::open(&path).unwrap();
            let mut replay = matrust::Engine::default();
            replay
                .execute_document(&doc.text, doc.path.as_ref().unwrap())
                .unwrap();
            assert_eq!(replay.figure.as_ref().unwrap().title, "Captured");
            assert_eq!(
                replay.figure.unwrap().series[0]
                    .points
                    .iter()
                    .collect::<Vec<_>>(),
                before.series[0].points.iter().collect::<Vec<_>>()
            );
        }
        fs::remove_file(&path).unwrap();
    }
    assert!(entries(dir.path()).is_empty());
}
#[test]
fn changes_during_encoding_new_collisions_cancellation_and_aliases_preserve_disk() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let source = dir.path().join("source.png");
    fs::write(&source, b"source").unwrap();
    let path = dir.path().join("output.png");
    fs::write(&path, b"aaaa").unwrap();
    let baseline = entries(dir.path());
    let error = prepare_checked(
        preview(path.clone(), &source),
        BaseTheme::Light,
        &cancel,
        || fs::write(&path, b"bbbb").unwrap(),
    )
    .err()
    .unwrap();
    assert!(error.contains("changed after export preparation"));
    assert_eq!(fs::read(&path).unwrap(), b"bbbb");
    assert_eq!(entries(dir.path()), baseline);
    let error = prepare_checked(figure(path.clone()), BaseTheme::Light, &cancel, || {
        cancel.store(true, Ordering::Relaxed)
    })
    .err()
    .unwrap();
    assert!(error.to_ascii_lowercase().contains("cancel"));
    assert_eq!(entries(dir.path()), baseline);
    cancel.store(false, Ordering::Relaxed);
    fs::remove_file(&path).unwrap();
    let plan = prepare(preview(path.clone(), &source), BaseTheme::Light, &cancel).unwrap();
    assert!(!plan.replaces());
    fs::write(&path, b"other creator").unwrap();
    assert!(commit(plan, &cancel).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"other creator");
    let plan = prepare(figure(path.clone()), BaseTheme::Light, &cancel).unwrap();
    cancel.store(true, Ordering::Relaxed);
    assert!(commit(plan, &cancel).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"other creator");
    cancel.store(false, Ordering::Relaxed);
    assert!(prepare(preview(source.clone(), &source), BaseTheme::Light, &cancel).is_err());
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(entries(dir.path()), baseline);
}
#[test]
fn prepared_script_companion_is_owned_until_commit_and_drop_does_not_remove_old_files() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let path = dir.path().join("large_plot.m");
    fs::write(&path, b"old script").unwrap();
    let mut f = matrust::plot::Figure::default();
    f.add(vec![matrust::plot::Series::new(
        (0..30_000).map(|i| [i as f64, i as f64 / 3.]).collect(),
        matrust::plot::PlotKind::Line,
    )])
    .unwrap();
    let source = f.clone();
    let input = || {
        Input::Figure(Box::new(ExportRequest {
            path: path.clone(),
            figure: f.clone(),
            width: 128,
            height: 96,
            vector: None,
        }))
    };
    let baseline = entries(dir.path());
    let plan = prepare(input(), BaseTheme::Light, &cancel).unwrap();
    assert!(entries(dir.path()).len() > baseline.len());
    drop(plan);
    assert_eq!(entries(dir.path()), baseline);
    let plan = prepare(input(), BaseTheme::Light, &cancel).unwrap();
    fs::write(&path, b"new script").unwrap();
    assert!(commit(plan, &cancel).is_err());
    assert_eq!(entries(dir.path()), baseline);
    assert_eq!(fs::read(&path).unwrap(), b"new script");
    let plan = prepare(input(), BaseTheme::Light, &cancel).unwrap();
    commit(plan, &cancel).unwrap();
    let doc = matrust::document::Document::open(&path).unwrap();
    let mut replay = matrust::Engine::default();
    replay
        .execute_document(&doc.text, doc.path.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        replay.figure.unwrap().series[0]
            .points
            .iter()
            .collect::<Vec<_>>(),
        source.series[0].points.iter().collect::<Vec<_>>()
    );
    assert!(f.same_snapshot(&source));
    assert!(f.series[0].points.same_storage(&source.series[0].points));
}

// Exact pre-optimization commit reference; only name and publisher API are adapted.
fn commit_before_one_stamp(plan: Plan, cancel: &AtomicBool) -> Result<PathBuf> {
    plan.verify(cancel)?;
    let Plan {
        path,
        expected,
        source: _,
        encoded,
    } = plan;
    match encoded {
        Encoded::File(temporary) => super::super::export_batch::publish_checked(
            &path,
            &expected,
            temporary,
            cancel,
            "Destination changed after review; choose the folder again before replacing it",
        )?,
        Encoded::Script(script) => {
            script.publish(expected.is_some(), cancel, || {
                if stamp(&path, cancel)? != expected {
                    return Err("Destination changed after export preparation; choose the destination again".into());
                }
                Ok(())
            })?;
        }
    }
    Ok(path)
}

#[test]
fn one_stamp_commit_retains_exact_conflict_errors_cancel_and_publication_outcomes() {
    for old in [true, false] {
        for case in [
            "unchanged",
            "changed",
            "removed",
            "appeared",
            "cancel",
            "directory",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("source.svg");
            fs::write(&source, b"source unchanged").unwrap();
            let path = dir.path().join("output.png");
            if case != "appeared" {
                fs::write(&path, b"before").unwrap();
            }
            let cancel = AtomicBool::new(false);
            let plan = prepare(preview(path.clone(), &source), BaseTheme::Light, &cancel).unwrap();
            match case {
                "changed" | "appeared" => fs::write(&path, b"after!").unwrap(),
                "removed" => fs::remove_file(&path).unwrap(),
                "cancel" => cancel.store(true, Ordering::Relaxed),
                "directory" => {
                    fs::remove_file(&path).unwrap();
                    fs::create_dir(&path).unwrap();
                }
                _ => {}
            }
            let result = if old {
                commit_before_one_stamp(plan, &cancel)
            } else {
                commit(plan, &cancel)
            };
            match case {
                "unchanged" => {
                    assert_eq!(result.unwrap(), path.canonicalize().unwrap());
                    assert_eq!(
                        image::open(&path).unwrap().to_rgb8().get_pixel(20, 10).0,
                        [17, 83, 149]
                    );
                }
                "cancel" => {
                    assert_eq!(result.unwrap_err(), "Cancelled");
                    assert_eq!(fs::read(&path).unwrap(), b"before");
                }
                "directory" => {
                    assert_eq!(
                        result.unwrap_err(),
                        "Destination must be a regular file, not a directory or symbolic link"
                    );
                    assert!(path.is_dir());
                }
                _ => {
                    assert_eq!(
                        result.unwrap_err(),
                        "Destination changed after export preparation; choose the destination again"
                    );
                    if case == "removed" {
                        assert!(!path.exists());
                    } else {
                        assert_eq!(fs::read(&path).unwrap(), b"after!");
                    }
                }
            }
            assert_eq!(fs::read(&source).unwrap(), b"source unchanged");
            assert_eq!(
                entries(dir.path()).len(),
                if case == "removed" { 1 } else { 2 }
            );
        }
    }
}

/// Commit-only comparison; preparation/encoding and fixture writes are excluded.
#[test]
#[ignore = "release measurement; root-owned exact-test invocation only"]
#[allow(
    clippy::assertions_on_constants,
    reason = "This ignored measurement must compile in debug and reject execution there; release artifacts are separately admitted"
)]
fn single_export_commit_release_measurements() {
    use std::{hint::black_box, time::Instant};
    assert!(
        !cfg!(debug_assertions),
        "Use the accepted release test artifact"
    );
    let order = std::env::var("MATRUST_SINGLE_EXPORT_ORDER").unwrap_or_else(|_| "normal".into());
    assert!(matches!(order.as_str(), "normal" | "reverse"));
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("source.svg");
    fs::write(&source, b"immutable preview source").unwrap();
    let path = root.join("output.png");
    // The existing destination is an opaque regular file; review does not decode it.
    // One bounded 256 KiB chunk avoids allocating a destination-sized buffer.
    let chunk = vec![0x5a_u8; 256 * 1024];
    let pools: Vec<_> = [1, 8]
        .into_iter()
        .map(|workers| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| assert_eq!(rayon::current_num_threads(), workers));
            (workers, pool)
        })
        .collect();
    let mut groups = Vec::new();
    for existing_bytes in [
        None,
        Some(4096_usize),
        Some(4 * 1024 * 1024),
        Some(64 * 1024 * 1024),
    ] {
        for pool in 0..pools.len() {
            groups.push((existing_bytes, pool));
        }
    }
    if order == "reverse" {
        groups.reverse();
    }
    println!(
        "SINGLE_EXPORT_COMMIT\texisting_bytes\tworkers\troute\torder\tphase\tsample\telapsed_ns"
    );
    for (existing_bytes, pool_index) in groups {
        let (workers, pool) = &pools[pool_index];
        for repetition in 0_usize..9 {
            let routes = if (repetition.is_multiple_of(2)) == (order == "normal") {
                [true, false]
            } else {
                [false, true]
            };
            for old in routes {
                assert!(!path.exists());
                if let Some(bytes) = existing_bytes {
                    let mut file = fs::File::create(&path).unwrap();
                    let mut remaining = bytes;
                    while remaining != 0 {
                        let count = remaining.min(chunk.len());
                        file.write_all(&chunk[..count]).unwrap();
                        remaining -= count;
                    }
                    file.sync_all().unwrap();
                }
                let elapsed = pool.install(|| {
                    assert_eq!(rayon::current_num_threads(), *workers);
                    let plan =
                        prepare(preview(path.clone(), &source), BaseTheme::Light, &cancel).unwrap();
                    assert_eq!(plan.replaces(), existing_bytes.is_some());
                    assert_eq!(
                        fs::metadata(&path).ok().map(|m| m.len()),
                        existing_bytes.map(|n| n as u64)
                    );
                    let started = Instant::now();
                    let result = if old {
                        commit_before_one_stamp(black_box(plan), &cancel)
                    } else {
                        commit(black_box(plan), &cancel)
                    };
                    let elapsed = started.elapsed();
                    assert_eq!(result.unwrap(), path);
                    elapsed
                });
                let decoded = image::open(&path).unwrap().to_rgb8();
                assert_eq!(decoded.dimensions(), (64, 32));
                assert!(decoded.pixels().all(|pixel| pixel.0 == [17, 83, 149]));
                fs::remove_file(&path).unwrap();
                assert_eq!(fs::read(&source).unwrap(), b"immutable preview source");
                assert_eq!(entries(&root), vec![source.file_name().unwrap().to_owned()]);
                println!(
                    "SINGLE_EXPORT_COMMIT\t{}\t{workers}\t{}\t{order}\t{}\t{}\t{}",
                    existing_bytes.map_or_else(|| "missing".into(), |n| n.to_string()),
                    if old {
                        "before_two_stamps"
                    } else {
                        "after_one_stamp"
                    },
                    if repetition < 2 { "warmup" } else { "retained" },
                    if repetition < 2 {
                        repetition
                    } else {
                        repetition - 2
                    },
                    elapsed.as_nanos()
                );
            }
        }
    }
}
