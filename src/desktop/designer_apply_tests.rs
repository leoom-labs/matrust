use super::*;
use matrust::dsp::{Band, WindowKind};
use std::sync::atomic::Ordering;

fn fir() -> Design {
    Design::Fir(crate::fir_designer::Design {
        order: 4,
        band: Band::Lowpass,
        sample_rate: 1000.,
        cutoff: 100.,
        upper: 200.,
        window: WindowKind::Hamming,
        scale: true,
    })
}
fn iir(family: crate::iir_designer::Family) -> Design {
    Design::Iir(crate::iir_designer::Design {
        family,
        ripple_db: 1.,
        order: 3,
        band: Band::Lowpass,
        sample_rate: 1000.,
        cutoff: 100.,
        upper: 200.,
    })
}
fn request(design: Design, name: &str) -> Request {
    Request {
        design,
        input: name.into(),
        folder: std::env::current_dir().unwrap(),
        view: None,
    }
}
fn reference(design: Design, signal: &Matrix) -> (Matrix, Matrix) {
    let mut reference = Engine::default();
    reference
        .merge_workspace(BTreeMap::from([("signal".into(), signal.clone().into())]))
        .unwrap();
    let (script, coeff, expression) = match design {
        Design::Fir(d) => (d.script().unwrap(), "b", "filter(b,1,signal)"),
        Design::Iir(d) => (d.script().unwrap(), "sos", "sosfilt(sos,signal)"),
    };
    reference
        .execute(&format!("{script}\nexpected={expression};"))
        .unwrap();
    (
        reference.variables[coeff].clone(),
        reference.variables["expected"].clone(),
    )
}
fn unchanged(before: &Engine, after: &Engine) {
    let a = before.workspace();
    let b = after.workspace();
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (name, value) in a {
        assert!(value.same_snapshot(&b[&name]), "{name}");
    }
    assert_eq!(before.current_figure, after.current_figure);
    let a = before.figures().collect::<Vec<_>>();
    let b = after.figures().collect::<Vec<_>>();
    assert_eq!(a.len(), b.len());
    for ((id, figure), (other, current)) in a.into_iter().zip(b) {
        assert_eq!(id, other);
        assert!(figure.same_snapshot(current));
    }
}

#[test]
fn worker_apply_reads_colliding_and_ordinary_names_before_output_publication() {
    for design in [
        fir(),
        iir(crate::iir_designer::Family::Butterworth),
        iir(crate::iir_designer::Family::Chebyshev1),
    ] {
        for name in ["b", "sos", "y", "x"] {
            let mut worker = crate::worker::Worker::new(eframe::egui::Context::default());
            worker
                .send
                .send(
                    crate::worker::Job::Script(format!(
                        "{name}=[1 -2 4 0 3 -1]; alias={name}; kept=[7 8];"
                    ))
                    .into(),
                )
                .unwrap();
            let initial = worker
                .receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            initial.result.unwrap();
            let original = initial.vars[name].clone();
            let (coeff, expected) = reference(design, original.as_numeric().unwrap());
            worker
                .send
                .send(crate::worker::Job::ApplyFilter(Box::new(request(design, name))).into())
                .unwrap();
            let reply = worker
                .receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            reply.result.unwrap();
            let coeff_name = if matches!(design, Design::Fir(_)) {
                "b"
            } else {
                "sos"
            };
            assert_eq!(
                reply.vars[coeff_name].as_numeric().unwrap().data(),
                coeff.data()
            );
            let y = reply.vars["y"].as_numeric().unwrap();
            assert_eq!((y.rows(), y.cols()), (expected.rows(), expected.cols()));
            assert_eq!(y.data(), expected.data());
            assert!(reply.vars["alias"].same_snapshot(&original));
            assert!(reply.vars["kept"].same_snapshot(&initial.vars["kept"]));
            if name != coeff_name && name != "y" {
                assert!(reply.vars[name].same_snapshot(&original));
            }
            let mut names = initial
                .vars
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            names.extend([coeff_name.into(), "y".into()]);
            assert_eq!(names, reply.vars.keys().cloned().collect());
            let figure = reply.figure.unwrap();
            assert_eq!(figure.title, "Filtered signal");
            assert_eq!(figure.series.len(), 1);
            for (point, value) in figure.series[0].points.iter().zip(expected.data()) {
                assert_eq!(point[1], *value);
            }
            worker.shutdown();
        }
    }
}

#[test]
fn invalid_input_design_budget_and_plot_failures_leave_all_outputs_unchanged() {
    for setup in ["x=[1 2 3];", "x=[1+i 2];", "x=true;", "x='text';", "x={1};"] {
        let mut engine = Engine::default();
        engine
            .execute(&format!("{setup} b=99; sos=88; y=77; plot([3 4]);"))
            .unwrap();
        let before = engine.clone();
        let input = if setup == "x=[1 2 3];" {
            "missing"
        } else {
            "x"
        };
        assert!(perform(&mut engine, request(fir(), input)).is_err());
        unchanged(&before, &engine);
    }
    let mut engine = Engine::default();
    engine
        .execute("x=[1 2 3]; b=99; y=77; plot([3 4]);")
        .unwrap();
    let before = engine.clone();
    let Design::Fir(mut invalid) = fir() else {
        unreachable!()
    };
    invalid.sample_rate = 0.;
    assert!(perform(&mut engine, request(Design::Fir(invalid), "x")).is_err());
    unchanged(&before, &engine);
    // A real ordinary title-dispatch failure occurs after candidate plot succeeds.
    engine.execute("title=7;").unwrap();
    let before = engine.clone();
    assert!(perform(&mut engine, request(fir(), "x")).is_err());
    unchanged(&before, &engine);
    let mut engine = Engine::default();
    let shared = Matrix::scalar(1.);
    let mut full = BTreeMap::from([(
        "x".into(),
        Matrix::from_column_vec(1, 3, vec![1., 2., 3.])
            .unwrap()
            .into(),
    )]);
    for n in 0..9999 {
        full.insert(format!("value{n}"), shared.clone().into());
    }
    engine.merge_workspace(full).unwrap();
    let before = engine.clone();
    assert!(
        perform(&mut engine, request(fir(), "x"))
            .unwrap_err()
            .contains("10000")
    );
    unchanged(&before, &engine);
}

#[test]
fn cancellation_before_work_and_after_staged_plot_never_publishes_then_retry_succeeds() {
    let mut engine = Engine::default();
    engine
        .execute("b=[1 3 2 4]; alias=b; y=42; plot([8 9]);")
        .unwrap();
    let before = engine.clone();
    let cancel = engine.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        perform(&mut engine, request(fir(), "b")).unwrap_err(),
        "Calculation cancelled"
    );
    unchanged(&before, &engine);
    cancel.store(false, Ordering::Relaxed);
    assert_eq!(
        before_publish(&mut engine, request(fir(), "b"), || cancel
            .store(true, Ordering::Relaxed))
        .unwrap_err(),
        "Calculation cancelled"
    );
    unchanged(&before, &engine);
    cancel.store(false, Ordering::Relaxed);
    perform(&mut engine, request(fir(), "b")).unwrap();
    assert!(std::sync::Arc::ptr_eq(&cancel, &engine.cancellation()));
    assert!(
        engine
            .array("alias")
            .unwrap()
            .same_snapshot(&before.array("b").unwrap())
    );
}

#[test]
fn held_tiled_axes_and_manual_view_match_existing_plot_dispatch() {
    let mut engine = Engine::default();
    engine
        .execute("x=[1 2 3 4]; subplot(1,2,1);plot([5 6]);subplot(1,2,2);plot([8 9]);hold on;")
        .unwrap();
    let original = engine.figure.clone().unwrap();
    let mut reference = engine.clone();
    let view = ([0., 10., -5., 20.], [false, true]);
    let mut figure =
        crate::worker::with_history_view(reference.figure.as_ref().unwrap(), Some(view)).unwrap();
    figure.ylim = None;
    reference.figure = Some(figure);
    let Design::Fir(d) = fir() else {
        unreachable!()
    };
    reference
        .execute(&format!(
            "{} y=filter(b,1,x);plot(y);title('Filtered signal');",
            d.script().unwrap()
        ))
        .unwrap();
    let mut req = request(fir(), "x");
    req.view = Some((engine.current_figure, view));
    perform(&mut engine, req).unwrap();
    let actual = engine.figure.as_ref().unwrap();
    let expected = reference.figure.as_ref().unwrap();
    assert_eq!(actual.svg(), expected.svg());
    assert_eq!(actual.xlim, Some([0., 10.]));
    assert!(actual.ylim.is_none());
    assert_eq!(actual.series.len(), 2);
    assert!(
        actual.series[0]
            .points
            .same_storage(&original.series[0].points)
    );
    let before = engine.clone();
    let mut invalid = request(fir(), "x");
    invalid.view = Some((99, view));
    assert!(perform(&mut engine, invalid).is_err());
    unchanged(&before, &engine);
}

#[test]
fn shadowed_title_failure_keeps_original_workspace_and_figure() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("title.m"),
        "function out=title(value)\nout=missing_title_value;\nend\n",
    )
    .unwrap();
    let mut engine = Engine::default();
    engine
        .execute("x=[1 2 3];alias=x;b=99;y=77;plot([4 5]);")
        .unwrap();
    let before = engine.clone();
    let mut req = request(fir(), "x");
    req.folder = directory.path().to_owned();
    let error = perform(&mut engine, req).unwrap_err();
    assert!(error.contains("missing_title_value"), "{error}");
    unchanged(&before, &engine);
    // Removing the override allows the same intended operation to succeed.
    std::fs::remove_file(directory.path().join("title.m")).unwrap();
    let mut req = request(fir(), "x");
    req.folder = directory.path().to_owned();
    perform(&mut engine, req).unwrap();
    assert_eq!(engine.figure.as_ref().unwrap().title, "Filtered signal");
}

#[test]
fn worker_apply_error_does_not_reuse_prior_script_diagnostic_or_partial_output() {
    let mut worker = crate::worker::Worker::new(eframe::egui::Context::default());
    worker
        .send
        .send(
            crate::worker::Job::Script(
                "x=[1 2 3];alias=x;plot([8 9]);disp('prior output');prior_missing_variable;".into(),
            )
            .into(),
        )
        .unwrap();
    let before = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(before.result.is_err());
    assert!(before.diagnostic.is_some());
    assert!(!before.partial_output.output.is_empty());
    worker
        .send
        .send(crate::worker::Job::ApplyFilter(Box::new(request(fir(), "missing_input"))).into())
        .unwrap();
    let reply = worker
        .receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let error = reply.result.unwrap_err();
    assert!(error.contains("missing_input") && !error.contains("prior_missing_variable"));
    assert!(reply.diagnostic.is_none() && reply.file_diagnostic.is_none());
    assert!(reply.partial_output.output.is_empty());
    assert_eq!(
        before.vars.keys().collect::<Vec<_>>(),
        reply.vars.keys().collect::<Vec<_>>()
    );
    for (name, value) in before.vars {
        assert!(value.same_snapshot(&reply.vars[&name]));
    }
    assert!(
        before
            .figure
            .unwrap()
            .same_snapshot(reply.figure.as_ref().unwrap())
    );
    assert_eq!(before.figure_documents, reply.figure_documents);
    worker.shutdown();
}

#[test]
#[ignore = "parent-owned release Apply measurements; requires matching accepted source and quiet lane"]
fn designer_apply_release_measurements() {
    use std::{hint::black_box, time::Instant};
    if cfg!(debug_assertions) {
        panic!("Release measurements only");
    }
    let Design::Fir(mut fir_design) = fir() else {
        unreachable!()
    };
    fir_design.order = 64;
    let Design::Iir(mut butter_design) = iir(crate::iir_designer::Family::Butterworth) else {
        unreachable!()
    };
    butter_design.order = 6;
    let mut cheby_design = butter_design;
    cheby_design.family = crate::iir_designer::Family::Chebyshev1;
    let designs = [
        ("fir", Design::Fir(fir_design)),
        ("butterworth", Design::Iir(butter_design)),
        ("chebyshev1", Design::Iir(cheby_design)),
    ];
    let sizes = [64usize, 8192, 65536];
    let folder = std::env::current_dir().unwrap();
    let pools = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
    });
    let mut fixtures = Vec::new();
    for (family, design) in designs {
        for size in sizes {
            let mut engine = Engine::default();
            let signal = Matrix::from_column_vec(
                1,
                size,
                (0..size).map(|n| (n % 17) as f64 / 8. - 1.).collect(),
            )
            .unwrap();
            engine
                .merge_workspace(BTreeMap::from([
                    ("x".into(), signal.clone().into()),
                    ("alias".into(), signal.into()),
                    ("b".into(), Matrix::scalar(42.).into()),
                    ("sos".into(), Matrix::scalar(43.).into()),
                    ("y".into(), Matrix::scalar(44.).into()),
                ]))
                .unwrap();
            engine.execute("plot([1 2]);").unwrap();
            let script = match design {
                Design::Fir(d) => format!(
                    "{}\ny = filter(b,1,x); plot(y); title('Filtered signal');",
                    d.script().unwrap()
                ),
                Design::Iir(d) => format!(
                    "{}\ny = sosfilt(sos,x); plot(y); title('Filtered signal');",
                    d.script().unwrap()
                ),
            };
            let mut expected = engine.clone();
            expected.set_function_directory(&folder).unwrap();
            expected.set_file_directory(&folder).unwrap();
            expected.execute(&script).unwrap();
            fixtures.push((family, design, size, engine, script, expected));
        }
    }
    let mut schedule = Vec::new();
    for index in 0..fixtures.len() {
        for workers in [1, 8] {
            for variant in ["legacy_script", "transactional_native"] {
                schedule.push((index, workers, variant));
            }
        }
    }
    let reverse = std::env::var_os("MATRUST_DESIGNER_BENCH_REVERSE").is_some();
    if reverse {
        schedule.reverse();
    }
    println!("family\tsamples\tworkers\tvariant\torder\tphase\trepeat\telapsed_ns");
    for (index, workers, variant) in schedule {
        let (family, design, size, initial, script, expected) = &fixtures[index];
        for repetition in 0..9 {
            // Common fixture clone/request construction and pool entry are untimed.
            // The extra transactional clone, kernels, plot, admission and commit
            // belong to the native action and are inside its timer.
            let mut engine = initial.clone();
            let req = Request {
                design: *design,
                input: "x".into(),
                folder: folder.clone(),
                view: None,
            };
            let elapsed = pools[usize::from(workers == 8)].install(|| {
                assert_eq!(rayon::current_num_threads(), workers, "observed pool size");
                let start = Instant::now();
                let output = if variant == "legacy_script" {
                    engine.set_function_directory(&folder).unwrap();
                    engine.set_file_directory(&folder).unwrap();
                    engine.execute(black_box(script)).unwrap()
                } else {
                    perform(black_box(&mut engine), req).unwrap()
                };
                let elapsed = start.elapsed().as_nanos();
                black_box(output);
                elapsed
            });
            let actual = engine.workspace();
            let reference = expected.workspace();
            assert_eq!(
                actual.keys().collect::<Vec<_>>(),
                reference.keys().collect::<Vec<_>>()
            );
            for (name, value) in &reference {
                let a = actual[name].as_numeric().unwrap();
                let b = value.as_numeric().unwrap();
                assert_eq!((a.rows(), a.cols()), (b.rows(), b.cols()));
                assert!(
                    a.data()
                        .iter()
                        .zip(b.data())
                        .all(|(x, y)| x.to_bits() == y.to_bits())
                );
            }
            assert!(actual["alias"].same_snapshot(&initial.array("x").unwrap()));
            assert_eq!(
                engine.figure.as_ref().unwrap().svg(),
                expected.figure.as_ref().unwrap().svg()
            );
            let phase = if repetition < 2 { "warmup" } else { "retained" };
            let repeat = if repetition < 2 {
                repetition
            } else {
                repetition - 2
            };
            println!(
                "{family}\t{size}\t{workers}\t{variant}\t{}\t{phase}\t{repeat}\t{elapsed}",
                if reverse { "reverse" } else { "normal" }
            );
            // Result-engine destruction is untimed for both routes.
            drop(engine);
        }
    }
}
