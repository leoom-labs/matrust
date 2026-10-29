use super::*;
use matrust::{Engine, session::ExportFormat as Format};

fn figures() -> Vec<(u32, Figure)> {
    let mut engine = Engine::default();
    engine.execute("figure(1);plot([0 1 2],[0 3 0]);title('First');figure(2);plot([0 1 2],[2 0 2]);title('Second');").unwrap();
    engine.figures().map(|(id, f)| (id, f.clone())).collect()
}
fn plan(folder: &Path, format: Format, cancel: &AtomicBool) -> Plan {
    prepare(
        Prepare {
            folder: folder.into(),
            figures: figures(),
            settings: ExportSettings {
                format,
                width: 400,
                height: 300,
                ..Default::default()
            },
        },
        cancel,
    )
    .unwrap()
}
fn request(plan: Plan, completed: usize, replace: bool) -> Run {
    Run {
        plan,
        completed,
        replace,
        progress: Arc::new(AtomicUsize::new(completed)),
    }
}
#[test]
fn batch_export_all_formats_match_single_exports_and_share_sample_storage() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for format in [
        Format::Png,
        Format::Jpeg,
        Format::Bmp,
        Format::Tiff,
        Format::Svg,
        Format::Pdf,
        Format::Eps,
        Format::Csv,
        Format::MatlabScript,
    ] {
        let plan = plan(dir.path(), format, &cancel);
        let run_request = request(plan.clone(), 0, false);
        assert!(
            (plan.items[0].figure.series[0].points)
                .same_storage(&run_request.plan.items[0].figure.series[0].points)
        );
        let outcome = run(run_request, &cancel, || {});
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.completed, 2);
        for item in &plan.items {
            let reference = dir
                .path()
                .join(format!("reference.{}", format.extensions()[0]));
            encode(
                &item.figure,
                &reference,
                plan.settings,
                matrust::plot::BaseTheme::Light,
                &cancel,
            )
            .unwrap();
            assert_eq!(
                fs::read(&item.path).unwrap(),
                fs::read(&reference).unwrap(),
                "{}",
                format.label()
            );
            if format.raster() {
                assert_eq!(
                    image::open(&item.path).unwrap().to_rgb8().dimensions(),
                    (400, 300)
                );
            }
        }
    }
    assert!(!fs::read_dir(dir.path()).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".matrust-export")
    }));
}
#[test]
fn batch_export_reviews_replacement_and_rejects_new_or_changed_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let path = dir.path().join("figure-1.csv");
    fs::write(&path, b"original").unwrap();
    let reviewed = plan(dir.path(), Format::Csv, &cancel);
    assert!(reviewed.items[0].replaces());
    let denied = run(request(reviewed.clone(), 0, false), &cancel, || {});
    assert_eq!(denied.completed, 0);
    assert!(denied.error.unwrap().contains("Confirm replacement"));
    fs::write(&path, b"modified").unwrap(); // same byte length, different digest
    let stale = run(request(reviewed, 0, true), &cancel, || {});
    assert_eq!(stale.completed, 0);
    assert!(stale.error.unwrap().contains("changed after review"));
    assert_eq!(fs::read(&path).unwrap(), b"modified");
    fs::remove_file(&path).unwrap();
    let missing = plan(dir.path(), Format::Csv, &cancel);
    fs::write(&path, b"created elsewhere").unwrap();
    assert_eq!(run(request(missing, 0, true), &cancel, || {}).completed, 0);
    assert_eq!(fs::read(&path).unwrap(), b"created elsewhere");
    let current = plan(dir.path(), Format::Csv, &cancel);
    let result = run(request(current, 0, true), &cancel, || {});
    assert_eq!(result.completed, 2);
    assert!(result.error.is_none());
}
#[test]
fn batch_export_cancellation_and_retry_only_publish_remaining_files() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let plan = plan(dir.path(), Format::Csv, &cancel);
    let result = run(request(plan.clone(), 0, false), &cancel, || {
        cancel.store(true, Ordering::Relaxed)
    });
    assert_eq!(result.completed, 1);
    assert!(result.error.unwrap().contains("Cancelled"));
    assert!(!plan.items[1].path.exists());
    fs::write(&plan.items[0].path, b"completed output changed later").unwrap();
    cancel.store(false, Ordering::Relaxed);
    let retry = request(plan.clone(), 1, false);
    let progress = retry.progress.clone();
    let result = run(retry, &cancel, || {});
    assert_eq!(result.error, None);
    assert_eq!(result.completed, 2);
    assert_eq!(progress.load(Ordering::Relaxed), 2);
    assert_eq!(
        fs::read(&plan.items[0].path).unwrap(),
        b"completed output changed later"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn batch_export_rechecks_after_encoding_and_cleans_failed_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let plan = plan(dir.path(), Format::Csv, &cancel);
    let temp = tempfile::Builder::new()
        .suffix(".csv")
        .tempfile_in(dir.path())
        .unwrap()
        .into_temp_path();
    let temp_path = temp.to_path_buf();
    encode(
        &plan.items[0].figure,
        &temp,
        plan.settings,
        matrust::plot::BaseTheme::Light,
        &cancel,
    )
    .unwrap();
    fs::write(&plan.items[0].path, b"concurrent writer").unwrap();
    assert!(publish(&plan.items[0], temp, &cancel).is_err());
    assert!(!temp_path.exists());
    assert_eq!(fs::read(&plan.items[0].path).unwrap(), b"concurrent writer");
    fs::remove_file(&plan.items[0].path).unwrap();
    fs::create_dir(&plan.items[1].path).unwrap();
    let outcome = run(request(plan.clone(), 0, false), &cancel, || {});
    assert_eq!(outcome.completed, 1);
    assert!(outcome.error.unwrap().contains("regular file"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn batch_export_rejects_invalid_inputs_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let make = |figures, settings| Prepare {
        folder: dir.path().into(),
        figures,
        settings,
    };
    assert!(prepare(make(vec![], ExportSettings::default()), &cancel).is_err());
    let f = figures();
    assert!(
        prepare(
            make(vec![f[0].clone(); 17], ExportSettings::default()),
            &cancel
        )
        .is_err()
    );
    assert!(
        prepare(
            make(vec![f[0].clone(); 2], ExportSettings::default()),
            &cancel
        )
        .is_err()
    );
    assert!(
        prepare(
            make(
                f.clone(),
                ExportSettings {
                    width: 8192,
                    height: 8192,
                    ..Default::default()
                }
            ),
            &cancel
        )
        .is_err()
    );
    cancel.store(true, Ordering::Relaxed);
    assert!(prepare(make(f, ExportSettings::default()), &cancel).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    cancel.store(false, Ordering::Relaxed);
    let oversized = fs::File::create(dir.path().join("figure-1.csv")).unwrap();
    oversized.set_len(MAX_EXISTING_BYTES + 1).unwrap();
    assert!(
        prepare(
            make(
                figures(),
                ExportSettings {
                    format: Format::Csv,
                    ..Default::default()
                }
            ),
            &cancel
        )
        .err()
        .unwrap()
        .contains("1 GiB")
    );
}

#[test]
fn eps_batch_retains_conflict_checks_and_stop_retry_without_rewriting_completed_file() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let target = dir.path().join("figure-1.eps");
    fs::write(&target, b"before").unwrap();
    let reviewed = plan(dir.path(), Format::Eps, &cancel);
    assert!(
        run(request(reviewed.clone(), 0, false), &cancel, || {})
            .error
            .unwrap()
            .contains("Confirm replacement")
    );
    fs::write(&target, b"newer!").unwrap();
    let stale = run(request(reviewed, 0, true), &cancel, || {});
    assert_eq!(stale.completed, 0);
    assert!(stale.error.unwrap().contains("changed after review"));
    assert_eq!(fs::read(&target).unwrap(), b"newer!");
    let current = plan(dir.path(), Format::Eps, &cancel);
    let first = run(request(current.clone(), 0, true), &cancel, || {
        cancel.store(true, Ordering::Relaxed)
    });
    assert_eq!(first.completed, 1);
    assert!(first.error.unwrap().contains("Cancelled"));
    let completed = fs::read(&target).unwrap();
    assert!(completed.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
    assert!(!dir.path().join("figure-2.eps").exists());
    fs::write(&target, b"completed file subsequently changed").unwrap();
    cancel.store(false, Ordering::Relaxed);
    let result = run(request(current, 1, true), &cancel, || {});
    assert_eq!(result.error, None);
    assert_eq!(result.completed, 2);
    assert_eq!(
        fs::read(&target).unwrap(),
        b"completed file subsequently changed"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn matlab_batch_owns_each_companion_until_commit_and_retries_only_remaining_items() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let mut figure = Figure::default();
    figure
        .add(vec![matrust::plot::Series::new(
            (0..30_000).map(|i| [i as f64, i as f64 / 3.]).collect(),
            matrust::plot::PlotKind::Line,
        )])
        .unwrap();
    let alias = figure.clone();
    let settings = ExportSettings {
        format: Format::MatlabScript,
        ..Default::default()
    };
    let make = || {
        prepare(
            Prepare {
                folder: dir.path().to_owned(),
                figures: vec![(1, figure.clone()), (2, figure.clone())],
                settings,
            },
            &cancel,
        )
        .unwrap()
    };
    let pending = make();
    fs::write(&pending.items[0].path, b"other writer").unwrap();
    let before = fs::read_dir(dir.path()).unwrap().count();
    let stale = run(request(pending, 0, true), &cancel, || {});
    assert_eq!(stale.completed, 0);
    assert!(stale.error.unwrap().contains("changed after review"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), before);
    let reviewed = make();
    let first = run(request(reviewed.clone(), 0, true), &cancel, || {
        cancel.store(true, Ordering::Relaxed)
    });
    assert_eq!(first.completed, 1);
    assert!(first.error.is_some());
    assert!(!reviewed.items[1].path.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2); // one script + one owned committed generation
    let saved = fs::read(&reviewed.items[0].path).unwrap();
    cancel.store(false, Ordering::Relaxed);
    let second = run(request(reviewed.clone(), 1, true), &cancel, || {});
    assert_eq!(second.completed, 2);
    assert!(second.error.is_none());
    assert_eq!(fs::read(&reviewed.items[0].path).unwrap(), saved);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
    for item in reviewed.items {
        let doc = matrust::document::Document::open(&item.path).unwrap();
        let mut replay = Engine::default();
        replay
            .execute_document(&doc.text, doc.path.as_ref().unwrap())
            .unwrap();
        assert_eq!(
            replay.figure.unwrap().series[0]
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
    }
    assert!((figure.series[0].points).same_storage(&alias.series[0].points));
}
