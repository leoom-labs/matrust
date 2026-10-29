//! Future parent-owned combined graph/export verification. Fresh output only.
#[cfg(not(feature = "plot-export"))]
fn main() {
    panic!("Build with --features desktop");
}

#[cfg(feature = "plot-export")]
fn main() {
    use matrust::{
        Engine, export, files,
        plot::{Canvas, Figure, PlotKind},
    };
    use std::{path::PathBuf, sync::atomic::AtomicBool};
    fn same_model(a: &Figure, b: &Figure) {
        a.validate().unwrap();
        b.validate().unwrap();
        assert_eq!(
            a.layout.is_some(),
            b.layout.is_some(),
            "Ordinary figure topology must survive script execution/reopen"
        );
        assert_eq!(a.all_axes().count(), b.all_axes().count());
        assert_eq!(a.active_position(), b.active_position());
        assert_eq!((&a.name, a.color), (&b.name, b.color));
        assert_eq!(
            (&a.title, &a.xlabel, &a.ylabel),
            (&b.title, &b.xlabel, &b.ylabel)
        );
        assert_eq!(
            (a.xgrid, a.ygrid, a.xscale, a.yscale),
            (b.xgrid, b.ygrid, b.xscale, b.yscale)
        );
        assert_eq!((a.xlim, a.ylim), (b.xlim, b.ylim));
        assert_eq!(a.textboxes, b.textboxes);
        assert_eq!((a.legend, a.legend_location), (b.legend, b.legend_location));
        assert_eq!(
            a.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
            b.legend_members().map(|(i, _)| i).collect::<Vec<_>>()
        );
        assert_eq!(
            a.legend_state.as_ref().unwrap().auto_update,
            b.legend_state.as_ref().unwrap().auto_update
        );
        assert_eq!(a.series.len(), b.series.len());
        for (x, y) in a.series.iter().zip(&b.series) {
            assert_eq!(
                (x.kind, x.color, x.visible, &x.label, x.pattern),
                (y.kind, y.color, y.visible, &y.label, y.pattern)
            );
            assert_eq!(
                (x.marker_is_filled(), x.marker_size, x.line_width),
                (y.marker_is_filled(), y.marker_size, y.line_width)
            );
            assert_eq!(
                (x.has_marker_style(), x.effective_marker()),
                (y.has_marker_style(), y.effective_marker())
            );
            assert_eq!(
                x.base_value.map(f64::to_bits),
                y.base_value.map(f64::to_bits)
            );
            assert_eq!(x.patch_face_count(), y.patch_face_count());
            assert_eq!(x.points.len(), y.points.len());
            for (p, q) in x.points.iter().zip(y.points.iter()) {
                assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
            }
        }
    }
    let output = PathBuf::from(std::env::args().nth(1).expect("fresh output directory"));
    assert_eq!(std::env::args().count(), 2);
    std::fs::create_dir(&output).unwrap();
    let stop = AtomicBool::new(false);
    let mut manifest = Vec::new();
    let mut checks = Vec::new();
    for (name, color, yscale, limits, y1, y2, line, patch, b1, b2, xgrid, ygrid) in [
        (
            "linear",
            "[.8 .9 1]",
            "linear",
            "[-3 8]",
            "[3 6 4]",
            "[5 2 7]",
            "[1 4 2]",
            "[-1 -1;2 3;-1 -1]",
            "-2",
            "1",
            "off",
            "on",
        ),
        (
            "positive-log",
            "[1 .88 .8]",
            "log",
            "[.5 8]",
            "[2 6 3]",
            "[5 2 7]",
            "[1 4 2]",
            "[1 1;3 4;1 1]",
            ".75",
            "4",
            "on",
            "off",
        ),
        (
            "negative-log",
            "[.9 .85 1]",
            "log",
            "[-8 -.5]",
            "[-2 -6 -3]",
            "[-5 -2 -7]",
            "[-1 -4 -2]",
            "[-1 -1;-3 -4;-1 -1]",
            "-.75",
            "-4",
            "off",
            "on",
        ),
    ] {
        let source = format!(
            "f=figure('Name','{name}: named canvas','Color',{color});a=gca;hold(a,'on');\n\
            h1=stem(a,[1 3 5],{y1},'Color',[.8 0 0],'Marker','o','MarkerSize',6,'LineWidth',1.5,'MarkerFaceColor',[.8 0 0],'BaseValue',{b1});\n\
            h2=stem(a,[2 4 6],{y2},'Color',[0 0 .8],'Marker','s','MarkerSize',6,'LineWidth',1.5,'MarkerFaceColor','none','BaseValue',{b2});\n\
            omitted=plot(a,[1 3 6],{line},'Color',[0 .5 0],'LineWidth',2);\n\
            p=patch('Parent',a,'XData',[.5 4.5;1.5 5.5;2.5 6.5],'YData',{patch},'FaceColor',[.7 .5 .1],'EdgeColor','none');\n\
            set(a,'XLim',[0 7],'YLim',{limits},'YScale','{yscale}','XGrid','{xgrid}','YGrid','{ygrid}','Box','on');\n\
            title(a,'Two distinct baselines; genuine triangle faces','Interpreter','none');xlabel(a,'x literal _ <&>','Interpreter','none');ylabel(a,'y literal \\alpha','Interpreter','none');\n\
            lg=legend(a,[h2 p h1],{{'second stem','two triangles','first stem'}},'Interpreter','none','Location','northeastoutside','AutoUpdate','off');\n\
            t=annotation(f,'textbox',[.04 .91 .92 .07],'String','Literal <&> \\alpha: green line excluded from legend','Interpreter','none','EdgeColor','none','HorizontalAlignment','center','FontWeight','bold','FontSize',12);\n"
        );
        let mut initial = Engine::default();
        initial.execute(&source).unwrap();
        let original = initial.figure.as_ref().unwrap().clone();
        original.validate().unwrap();
        assert_eq!(original.name, format!("{name}: named canvas"));
        assert_eq!(
            original.color,
            match name {
                "linear" => [204, 230, 255],
                "positive-log" => [255, 224, 204],
                _ => [230, 217, 255],
            }
        );
        assert_eq!(
            (original.xgrid, original.ygrid),
            (xgrid == "on", ygrid == "on")
        );
        assert_eq!(
            original.series[0].base_value.map(f64::to_bits),
            Some(b1.parse::<f64>().unwrap().to_bits())
        );
        assert_eq!(
            original.series[1].base_value.map(f64::to_bits),
            Some(b2.parse::<f64>().unwrap().to_bits())
        );
        assert_eq!(original.series.len(), 4);
        assert_eq!(original.series[3].kind, PlotKind::Patch);
        assert_eq!(original.series[3].patch_face_count(), 2);
        assert_eq!(
            original
                .legend_members()
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            [1, 3, 0]
        );
        assert_ne!(original.series[0].base_value, original.series[1].base_value);
        assert_ne!(original.xgrid, original.ygrid);
        let script = files::matlab_plot_script(&original, &stop).unwrap();
        let mut executed = Engine::default();
        executed.execute(&script).unwrap();
        let rerun = executed.figure.as_ref().unwrap().clone();
        same_model(&original, &rerun);
        checks.push(serde_json::json!({"scene":name,"unchanged_export_executed":true,"model_checked":true,"layout_topology_checked":true,"legend_indices":[1,3,0],"series":4,"patch_faces":2,"textboxes":1,"native_review_pending":true}));
        for (variant, figure) in [("original", original), ("executed", rerun)] {
            let key = format!("{name}-{variant}");
            let dir = output.join(&key);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("scene_source.m"), &source).unwrap();
            // Both variants retain the exact script that was executed, not a rewritten surrogate.
            std::fs::write(dir.join("graphics_fixture.m"), &script).unwrap();
            files::save_figure(&dir.join("figure.mfig"), &figure).unwrap();
            let reopened = files::load_figure(&dir.join("figure.mfig")).unwrap();
            same_model(&figure, &reopened);
            files::save_figure(&dir.join("reopened.mfig"), &reopened).unwrap();
            assert_eq!(
                std::fs::read(dir.join("figure.mfig")).unwrap(),
                std::fs::read(dir.join("reopened.mfig")).unwrap()
            );
            for (size, width, height) in [("wide", 1440, 900), ("compact", 640, 480)] {
                let canvas = Canvas {
                    width: width as f32,
                    height: height as f32,
                };
                assert_eq!(
                    figure.svg_on_canvas(canvas).unwrap(),
                    reopened.svg_on_canvas(canvas).unwrap()
                );
                export::export_figure(&reopened, &dir.join(format!("{size}.png")), width, height)
                    .unwrap();
                for ext in ["svg", "pdf"] {
                    export::export_vector_with_cancel(
                        &reopened,
                        &dir.join(format!("{size}.{ext}")),
                        canvas,
                        &stop,
                    )
                    .unwrap();
                }
            }
            let mut paths: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            paths.sort();
            let artifacts:Vec<_>=paths.iter().map(|p|serde_json::json!({"file":p.file_name().unwrap().to_str().unwrap(),"bytes":std::fs::metadata(p).unwrap().len()})).collect();
            manifest.push(serde_json::json!({"case":key,"scene":name,"variant":variant,"polygons":2,"textboxes":1,"files":artifacts,"roundtrip":"Exact MFIG reopen bytes and per-size SVG; unchanged exported script executed and semantic graph checked"}));
        }
    }
    std::fs::write(
        output.join("fixtures.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("comparisons.json"),
        serde_json::to_vec_pretty(&checks).unwrap(),
    )
    .unwrap();
}
