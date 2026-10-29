//! Parent-owned visual fixture preparation, only after source integration.
//! Requires a fresh output directory; successful encoding is not visual review.
#[cfg(not(feature = "plot-export"))]
fn main() {
    eprintln!("Build with --features plot-export (or desktop)");
    std::process::exit(2);
}

#[cfg(feature = "plot-export")]
fn main() {
    use matrust::{
        Engine, export, files,
        plot::{Canvas, Figure, LegendLocation, LinePattern, PlotKind, Series},
    };
    use std::{path::PathBuf, sync::atomic::AtomicBool};
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("one fresh output-directory argument required"),
    );
    assert_eq!(std::env::args().count(), 2);
    std::fs::create_dir(&output).expect("output directory must not already exist");
    let cancel = AtomicBool::new(false);
    let scripted = |script: &str| {
        let mut e = Engine::default();
        e.execute(script).unwrap();
        e.figure.unwrap()
    };
    let mut cases = vec![
        (
            "short-east",
            scripted(
                "plot([0 1 2],[0 1 0],'o-',[0 1 2],[1 0 1],'LineStyle','none','Marker','square');legend('Line','Markers','Location','eastoutside');",
            ),
        ),
        (
            "short-south",
            scripted("plot([0 1 2],[0 1 0]);legend('Signal','Location','southoutside');"),
        ),
        (
            "custom-pair",
            scripted(
                "subplot('Position',[0.1 0.2 0.3 0.6]);plot([0 1],[0 1]);legend('Left','Location','westoutside');subplot('Position',[0.6 0.2 0.3 0.6]);plot([0 1],[1 0]);legend('Right','Location','eastoutside');",
            ),
        ),
        (
            "grid-title",
            scripted(
                "subplot(2,2,1);plot([0 1],[0 1]);legend('North','Location','northoutside');subplot(2,2,2);plot([0 1],[1 0]);legend('East','Location','eastoutside');subplot(2,2,3);plot([0 1],[1 0]);legend('West','Location','westoutside');subplot(2,2,4);plot([0 1],[0 1]);legend('South','Location','southoutside');sgtitle('Outside legend grid');",
            ),
        ),
        (
            "best-gaps",
            scripted(
                "plot([0 .5 NaN .8 1],[.9 .9 NaN .1 .1]);legend('Gapped line','Location','best');xlim([0 1]);ylim([0 1]);",
            ),
        ),
    ];
    cases[0].1.series[1].visible = false;
    for (name, count) in [("long-unicode", 4), ("dense64", 64)] {
        let mut f = Figure::default();
        for i in 0..count {
            let mut s = Series::new(
                vec![[0., i as f64], [1., i as f64 + 0.5], [2., i as f64]],
                if i % 3 == 0 {
                    PlotKind::Stem
                } else {
                    PlotKind::Line
                },
            );
            s.label = if count == 4 && i < 2 {
                "Fréquence Δ: duplicate label <&> with a long descriptive suffix".into()
            } else {
                format!("Series {i}: Fréquence Δ <&> {}", "long label ".repeat(8))
            };
            s.visible = i != 2 && i != 7;
            s.markers = i % 3 == 1;
            if i == 1 {
                s.pattern = LinePattern::None;
                s.marker_size = Some(100.);
                s.line_width = Some(50.);
            }
            f.add(vec![s]).unwrap();
        }
        f.legend = true;
        f.legend_location = LegendLocation::Eastoutside;
        cases.push((name, f));
    }
    let mut manifest = Vec::new();
    for (name, figure) in cases {
        figure.validate().unwrap();
        let folder = output.join(name);
        std::fs::create_dir(&folder).unwrap();
        files::save_figure(&folder.join("figure.mfig"), &figure).unwrap();
        files::write_matlab_plot_script_with_cancel(
            &folder.join("legend_fixture.m"),
            &figure,
            &cancel,
        )
        .unwrap();
        for (label, width, height) in [("compact", 640, 480), ("wide", 1440, 900)] {
            export::export_figure(&figure, &folder.join(format!("{label}.png")), width, height)
                .unwrap();
            for extension in ["svg", "pdf"] {
                export::export_vector_with_cancel(
                    &figure,
                    &folder.join(format!("{label}.{extension}")),
                    Canvas {
                        width: width as f32,
                        height: height as f32,
                    },
                    &cancel,
                )
                .unwrap();
            }
        }
        let mut paths: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        let files: Vec<_> = paths.into_iter().map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            serde_json::json!({"file":path.file_name().unwrap().to_str().unwrap(),"bytes":bytes.len(),"blake3":blake3::hash(&bytes).to_hex().to_string()})
        }).collect();
        manifest.push(serde_json::json!({"case":name,"files":files,"visual_review":"pending","labels":"Complete editable/SVG title text; visible exports may abbreviate and scale. PNG/PDF do not expose tooltips."}));
    }
    std::fs::write(
        output.join("fixtures.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}
