//! Release-only SVG outline baseline, including real fill/stroke intersections.
//! No native/GPU/renderer, exact-predicate or MATLAB speed claim.
use matrust::{
    Matrix,
    plot::{Figure, LinePattern, PlotKind, Series, clip_face_edge, clip_patch_face},
};
use std::{sync::atomic::AtomicBool, time::Instant};

#[derive(Clone, Copy, Debug)]
enum Workload {
    VisibleBar,
    VisibleQuad,
    ClippedTriangle,
    ExtremeClippedTriangle,
}
impl Workload {
    fn kind(self) -> PlotKind {
        if matches!(self, Self::VisibleBar) {
            PlotKind::Bar
        } else {
            PlotKind::Patch
        }
    }
    fn clipped(self) -> bool {
        matches!(self, Self::ClippedTriangle | Self::ExtremeClippedTriangle)
    }
}

fn figure(workload: Workload, count: usize, style: u8) -> Figure {
    let kind = workload.kind();
    let mut series = match workload {
        Workload::VisibleBar => Series::new(
            (0..count)
                .map(|i| [i as f64 + 1., (i % 7 + 1) as f64])
                .collect(),
            kind,
        ),
        Workload::VisibleQuad => {
            let x = Matrix::from_column_vec(
                4,
                count,
                (0..count)
                    .flat_map(|i| {
                        let x = i as f64 + 1.;
                        [x - 0.4, x + 0.4, x + 0.4, x - 0.4]
                    })
                    .collect(),
            )
            .unwrap();
            let y = Matrix::from_column_vec(
                4,
                count,
                (0..count)
                    .flat_map(|i| {
                        let y = (i % 7 + 1) as f64;
                        [0., 0., y, y]
                    })
                    .collect(),
            )
            .unwrap();
            Series::patch_from_matrices(&x, &y, [17, 129, 241], &AtomicBool::new(false)).unwrap()
        }
        Workload::ClippedTriangle | Workload::ExtremeClippedTriangle => {
            let extent = if matches!(workload, Workload::ClippedTriangle) {
                2.
            } else {
                f64::MAX
            };
            let x = Matrix::from_column_vec(
                3,
                count,
                (0..count).flat_map(|_| [-extent, extent, extent]).collect(),
            )
            .unwrap();
            let y = Matrix::from_column_vec(
                3,
                count,
                (0..count)
                    .flat_map(|i| {
                        let sign = if i.is_multiple_of(2) { 1. } else { -1. };
                        [-sign * extent, sign * extent, -sign * extent]
                    })
                    .collect(),
            )
            .unwrap();
            Series::patch_from_matrices(&x, &y, [17, 129, 241], &AtomicBool::new(false)).unwrap()
        }
    };
    series.color = [17, 129, 241];
    series.explicit_color = true;
    if style != 0 {
        series.face_edge_color = Some([231, 33, 87]);
        series.line_width = Some(0.5);
        series.pattern = if style == 1 {
            LinePattern::Solid
        } else {
            LinePattern::Dashed
        };
    }
    let mut figure = Figure::default();
    figure.add(vec![series]).unwrap();
    figure.legend = false;
    if workload.clipped() {
        figure.xlim = Some([-1., 1.]);
        figure.ylim = Some([-1., 1.]);
    } else {
        figure.xlim = Some([0., count as f64 + 1.]);
        figure.ylim = Some([0., 8.]);
    }
    figure.validate().unwrap();
    figure
}

fn verify_clipped_geometry(figure: &Figure, count: usize) {
    let points = &figure.series[0].points;
    assert_eq!(points.len(), count * 3);
    for index in 0..count {
        let storage = points.face(index, 3).unwrap();
        let face = storage.as_slice();
        let sign = if index.is_multiple_of(2) { 1. } else { -1. };
        let expected = [[-1., -sign], [1., sign], [1., -sign]];
        let clipped = clip_patch_face(face, [-1., 1., -1., 1.]);
        assert_eq!(clipped.points().len(), 3);
        for point in expected {
            assert!(clipped.points().contains(&point));
        }
        // Only the source diagonal reaches the viewport. The two synthetic
        // fill boundaries must not acquire outlines.
        assert_eq!(
            clip_face_edge(face[0], face[1], [-1., 1., -1., 1.]),
            Some([expected[0], expected[1]])
        );
        assert!(clip_face_edge(face[1], face[2], [-1., 1., -1., 1.]).is_none());
        assert!(clip_face_edge(face[2], face[0], [-1., 1., -1., 1.]).is_none());
    }
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("Run this harness in release mode");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    let reverse = match args.as_slice() {
        [] => false,
        [flag] if flag == "--reverse" => true,
        _ => panic!("Only --reverse is supported"),
    };
    let mut groups = Vec::new();
    for workers in [1, 8] {
        for workload in [
            Workload::VisibleBar,
            Workload::VisibleQuad,
            Workload::ClippedTriangle,
            Workload::ExtremeClippedTriangle,
        ] {
            for count in [10, 1000, 10000] {
                for style in 0..3 {
                    groups.push((workers, workload, count, style));
                }
            }
        }
    }
    assert_eq!(groups.len(), 72);
    if reverse {
        groups.reverse();
    }
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let many = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    println!("workers,workload,kind,faces,style,points,logical_bytes,rep,elapsed_ns,svg_bytes");
    for (workers, workload, count, style) in groups {
        let figure = figure(workload, count, style);
        let snapshot = figure.clone();
        let samples = figure.series[0].points.clone();
        let expected = figure.svg();
        assert!(!expected.is_empty());
        if workload.clipped() {
            verify_clipped_geometry(&figure, count);
            assert_eq!(expected.matches("<polygon fill=\"#1181f1\"").count(), count);
            assert_eq!(
                expected
                    .matches("<path fill=\"none\" stroke=\"#e72157\"")
                    .count(),
                if style == 0 { 0 } else { count }
            );
            // The finite and extreme sources have the same analytically
            // proved clipped geometry. Require exact complete SVG parity.
            let control = self::figure(Workload::ClippedTriangle, count, style);
            assert_eq!(expected, control.svg());
        }
        let pool = if workers == 1 { &one } else { &many };
        let kind = workload.kind();
        for rep in 0..9 {
            let started = Instant::now();
            let svg = pool.install(|| figure.svg());
            let elapsed = started.elapsed().as_nanos();
            assert_eq!(svg, expected);
            assert!(figure.same_snapshot(&snapshot));
            assert!((samples).same_storage(&figure.series[0].points));
            if rep >= 2 {
                println!(
                    "{workers},{workload:?},{kind:?},{count},{style},{},{},{},{elapsed},{}",
                    samples.len(),
                    samples.len() * 16,
                    rep - 2,
                    svg.len()
                );
            }
            std::hint::black_box(&svg);
            drop(svg);
        }
    }
}
