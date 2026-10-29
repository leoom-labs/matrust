//! Release I/O timings; shared snapshots include no numeric-array copying.
use matrust::{
    document::Document,
    plot::{Figure, Series},
    session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, FigureView, restore_figures},
    },
    simulation::Model,
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
fn state(count: usize) -> Session {
    let mut figures = Vec::new();
    for id in 1..=count {
        let points: Vec<_> = (0..1_000_000)
            .map(|i| [i as f64, ((i % 1024) as f64 - 512.) + id as f64])
            .collect();
        let mut figure = Figure::default();
        figure.series.push(Series {
            identity: matrust::ObjectIdentity::default(),
            baseline_identity: matrust::ObjectIdentity::default(),
            bar_baseline_visible: false,
            points: Arc::new(points).into(),
            patch: None,
            base_value: None,
            face_edge_color: None,
            label: "Recording".into(),
            kind: matrust::plot::PlotKind::Line,
            color: [0, 114, 189],
            explicit_color: false,
            auto_palette: None,
            pattern: matrust::plot::LinePattern::Solid,
            line_width: None,
            marker_shape: None,
            marker_size: None,
            markers: false,
            marker_filled: false,
            visible: true,
            bar_width: 0.8,
            bar_offset: 0.,
        });
        figure.title = format!("Recording {id}");
        figures.push(FigureDocument {
            window: None,
            id: id as u32,
            figure,
            path: None,
            source_version: None,
            dirty: true,
            view: Some(FigureView {
                bounds: [0., 999999., -1000., 1000.],
                auto: [true, true],
                dirty: false,
            }),
        });
    }
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: None,
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
        layout: Default::default(),
        figure_manifest: None,
        figures: Some(FigureSession {
            current: 1,
            figures,
        }),
    }
}
fn report(case: &str, count: usize, mut samples: Vec<f64>) {
    samples.sort_by(f64::total_cmp);
    println!(
        "{case},{count},1000000,{},{:.6},{:.6},{:.6}",
        samples.len(),
        samples[samples.len() / 2],
        samples[0],
        samples[samples.len() - 1]
    );
}
fn main() -> Result<(), String> {
    let max = std::env::args().any(|a| a == "--max");
    let count = if max { 16 } else { 1 };
    let reps = if max { 1 } else { 15 };
    std::fs::create_dir_all("target/evidence/performance/data").map_err(|e| e.to_string())?;
    let temp =
        tempfile::tempdir_in("target/evidence/performance/data").map_err(|e| e.to_string())?;
    let path = temp.path().join("session.json");
    let mut snapshot = state(count);
    let mut store = SessionStore::new(&path);
    println!("case,figures,points_per_figure,repetitions,median_ms,min_ms,max_ms");
    let start = Instant::now();
    store.save(snapshot.clone())?;
    report(
        "initial_checkpoint",
        count,
        vec![start.elapsed().as_secs_f64() * 1000.],
    );
    for case in [
        "unchanged_checkpoint",
        "view_checkpoint",
        "metadata_checkpoint",
    ] {
        if max && case == "metadata_checkpoint" {
            continue;
        }
        let mut samples = vec![];
        for index in 0..reps + 2 {
            let first = &mut snapshot.figures.as_mut().unwrap().figures[0];
            match case {
                "view_checkpoint" => {
                    let view = first.view.as_mut().unwrap();
                    view.auto[0] = false;
                    view.bounds[1] = 900000. + index as f64;
                    view.dirty = true;
                }
                "metadata_checkpoint" => first.figure.title = format!("Edited title {index}"),
                _ => {}
            }
            let start = Instant::now();
            store.save(snapshot.clone())?;
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            if index >= 2 {
                samples.push(elapsed);
            }
        }
        report(case, count, samples);
    }
    let manifest = Session::load(&path)?.figure_manifest.unwrap();
    let mut times = vec![];
    for _ in 0..reps {
        let start = Instant::now();
        let result = restore_figures(&path, &manifest, &AtomicBool::new(false));
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if !result.warnings.is_empty() {
            return Err(result.warnings.join("; "));
        }
        assert_eq!(result.figures.figures.len(), count);
        for (a, b) in snapshot
            .figures
            .as_ref()
            .unwrap()
            .figures
            .iter()
            .zip(&result.figures.figures)
        {
            assert_eq!(a.id, b.id);
            assert_eq!(
                a.figure.series[0].points.len(),
                b.figure.series[0].points.len()
            );
            assert!(
                a.figure.series[0]
                    .points
                    .iter()
                    .zip(b.figure.series[0].points.iter())
                    .all(|(a, b)| a[0].to_bits() == b[0].to_bits()
                        && a[1].to_bits() == b[1].to_bits())
            );
        }
        times.push(elapsed);
    }
    report("restore", count, times);
    Ok(())
}
