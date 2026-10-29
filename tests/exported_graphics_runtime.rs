use matrust::{Array, Engine, files, plot::PlotKind};
use std::sync::atomic::AtomicBool;
fn text(e: &Engine, name: &str) -> String {
    let Array::Character(c) = e.array(name).unwrap() else {
        panic!()
    };
    c.to_utf8(&AtomicBool::new(false)).unwrap()
}

// Exercise the public producer and the same bounded reader used by script Open.
// No text rewriting or exporter-specific execution path is permitted here.
fn replay_saved_script(figure: &matrust::plot::Figure) -> (String, Engine) {
    let source_snapshot = figure.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved_reconstruction.m");
    files::write_matlab_plot_script_with_cancel(&path, figure, &AtomicBool::new(false)).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let document = matrust::document::Document::open(&path).unwrap();
    assert!(!document.dirty);
    assert!(matrust::document::same_path(
        document.path.as_ref().unwrap(),
        &path
    ));
    assert_eq!(document.text.as_bytes(), bytes.as_slice());
    let mut replay = Engine::default();
    replay.execute(&document.text).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(figure.same_snapshot(&source_snapshot));
    for ((_, before), (_, after)) in source_snapshot.all_axes().zip(figure.all_axes()) {
        for (before, after) in before.series.iter().zip(&after.series) {
            assert!((before.points).same_storage(&after.points));
        }
    }
    (document.text, replay)
}

#[test]
fn figure_appearance_construction_queries_and_existing_handle_updates_are_real() {
    let mut e = Engine::default();
    e.execute("f=figure('Name','literal figure','Color',[.2 .4 .6]);a=gca;h=plot(a,[0 1],[2 3]);figure(9);set(f,'Name','renamed','Color','#204060');name=get(f,'Name');rgb=get(f,'Color');").unwrap();
    assert_eq!(e.current_figure, 9);
    assert_eq!(text(&e, "name"), "renamed");
    let f = e.figures().find(|(id, _)| *id != 9).unwrap().1;
    assert_eq!(f.name, "renamed");
    assert_eq!(f.color, [32, 64, 96]);
    assert_eq!(
        e.variables["rgb"].data(),
        &[32. / 255., 64. / 255., 96. / 255.]
    );
    let before = f.clone();
    let ids = e.figure_ids();
    for script in [
        "figure('Name','must not publish','Color',[2 0 0]);",
        "set(f,'Name','must not publish','NumberTitle','off');",
        "set(f,'Color',[1 0 0],'Name',1);",
    ] {
        assert!(e.execute(script).is_err());
        assert_eq!(e.figure_ids(), ids);
        assert_eq!(e.current_figure, 9);
        assert!(before.same_snapshot(e.figures().find(|(id, _)| *id != 9).unwrap().1));
    }
}

#[test]
fn literal_axes_text_box_and_stem_baseline_are_supported_without_discarding_values() {
    let mut e = Engine::default();
    e.execute("f=figure;a=gca;h=stem(a,[1 2],[3 4],'Color',[0 0 1],'Marker','o','MarkerFaceColor',[0 0 1],'BaseValue',-2);title(a,'literal _ ^ \\alpha','Interpreter','none');xlabel(a,'x','Interpreter','none');ylabel(a,'y','Interpreter','none');set(a,'Box','on');b=get(h,'BaseValue');kind=get(h,'Type');").unwrap();
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series[0].base_value, Some(-2.));
    assert_eq!(f.bounds().1[0], -2.);
    assert_eq!(e.variables["b"].data(), &[-2.]);
    assert_eq!(text(&e, "kind"), "stem");
    assert!(f.title.contains("_ ^"));
    let before = f.clone();
    for script in [
        "title(a,'lost','Interpreter','tex');",
        "set(a,'Box','off');",
        "set(h,'BaseValue',Inf);",
        "set(h,'MarkerFaceColor',[1 0 0]);",
    ] {
        assert!(e.execute(script).is_err());
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    }
    e.execute("set(h,'BaseValue',1,'MarkerFaceColor','none');")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series[0].base_value, Some(1.));
    assert!(!e.figure.as_ref().unwrap().series[0].marker_filled);
    assert!((before.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
}

#[test]
fn marker_fill_never_uses_a_different_automatic_or_independent_color() {
    let mut e = Engine::default();
    e.execute("plot([0 1],[2 3]);hold on;").unwrap();
    let before = e.figure.clone().unwrap();
    assert!(
        e.execute("plot([0 1],[4 5],'Marker','o','MarkerFaceColor',[1 0 0]);")
            .is_err()
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute("h=plot([0 1],[4 5],'Marker','o','Color',[1 0 0],'MarkerFaceColor',[1 0 0]);")
        .unwrap();
    let s = &e.figure.as_ref().unwrap().series[1];
    assert!(s.marker_filled && s.explicit_color);
    assert_eq!(s.color, [255, 0, 0]);
    e.execute("set(h,'Color',[0 1 0],'MarkerFaceColor',[0 1 0]);")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series[1].color, [0, 255, 0]);
}

#[test]
fn unchanged_full_matlab_export_executes_all_plot_families_and_literal_annotation() {
    let mut source = Engine::default();
    source.execute("f=figure('Name','all families','Color',[.2 .4 .6]);a=gca;hold(a,'on');h1=plot(a,[-0 1],[2 3],'Color',[1 0 0],'Marker','o','MarkerFaceColor',[1 0 0]);h2=scatter(a,[0 1],[3 4],25,'Marker','s','MarkerEdgeColor',[0 1 0],'MarkerFaceColor','none');h3=stairs(a,[0 1],[4 5],'Color',[0 0 1],'Marker','none','MarkerFaceColor','none');h4=stem(a,[0 1],[5 6],'Color',[1 0 1],'Marker','o','MarkerFaceColor',[1 0 1],'BaseValue',-1);bar([0 2],[1 3]);p=patch('Parent',a,'XData',[0 2;1 3;0 2],'YData',[2 2;2 2;3 4],'FaceColor',[.25 .5 .75],'EdgeColor','none');set(p,'DisplayName','patch literal','Visible','off');set(a,'XGrid','off','YGrid','on');title(a,'literal < & title','Interpreter','none');legend(a,[h4 h1],{'stem entry','line entry'},'Interpreter','none','AutoUpdate','off');t=annotation(f,'textbox',[.1 .9 .8 .08],'String','literal annotation','Interpreter','none','EdgeColor','none','HorizontalAlignment','center');").unwrap();
    let before = source.figure.as_ref().unwrap();
    let (_, target) = replay_saved_script(before);
    let after = target.figure.as_ref().unwrap();
    after.validate().unwrap();
    assert!(
        before.layout.is_none() && after.layout.is_none(),
        "An ordinary figure must not gain subplot controls after script replay"
    );
    assert_eq!(after.name, before.name);
    assert_eq!(after.color, before.color);
    assert_eq!(after.title, before.title);
    assert_eq!((after.xgrid, after.ygrid), (false, true));
    assert_eq!(before.series.len(), 6);
    assert_eq!(after.series.len(), 6);
    assert_eq!(after.textboxes, before.textboxes);
    for (a, b) in before.series.iter().zip(&after.series).take(4) {
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.color, b.color);
        assert_eq!(a.label, b.label);
        assert_eq!(a.marker_is_filled(), b.marker_is_filled());
        assert_eq!(a.points.len(), b.points.len());
        assert_eq!(a.pattern, b.pattern);
        assert_eq!(a.visible, b.visible);
        assert_eq!(a.effective_marker(), b.effective_marker());
        assert_eq!(a.marker_size_points(), b.marker_size_points());
        assert_eq!(a.line_width.unwrap_or(1.5), b.line_width.unwrap_or(1.5));
        for (p, q) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
        }
    }
    assert_eq!(after.series[3].base_value, Some(-1.));
    assert_eq!(after.series[4].kind, PlotKind::Bar);
    let bar = &before.series[4];
    let restored_bar = &after.series[4];
    assert_eq!(restored_bar.points.len(), bar.points.len());
    assert_eq!(restored_bar.bar_width.to_bits(), bar.bar_width.to_bits());
    assert_eq!(restored_bar.bar_offset.to_bits(), bar.bar_offset.to_bits());
    assert_eq!(
        restored_bar.color,
        before
            .resolve_appearance(matrust::plot::BaseTheme::Light)
            .series_color(bar)
    );
    // Genuine Bar construction admits slot 4 before the exported FaceColor set.
    // The manual override retains that latent provenance but resolves fixed RGB.
    assert!(restored_bar.explicit_color);
    assert_eq!(
        bar.auto_palette,
        Some(matrust::plot::AutoPalette {
            version: matrust::plot::PaletteVersion::MatRustModern1,
            slot: 4,
        })
    );
    assert_eq!(restored_bar.auto_palette, bar.auto_palette);
    for host in [
        matrust::plot::BaseTheme::Light,
        matrust::plot::BaseTheme::Dark,
    ] {
        assert_eq!(
            after.resolve_appearance(host).series_color(restored_bar),
            restored_bar.color
        );
    }
    assert_eq!(restored_bar.visible, bar.visible);
    assert_eq!(restored_bar.label, bar.label);
    for (point, actual) in bar.points.iter().zip(restored_bar.points.iter()) {
        assert_eq!(point.map(f64::to_bits), actual.map(f64::to_bits));
        let edges = |p: [f64; 2], s: &matrust::plot::Series| {
            [
                p[0] + s.bar_offset - s.bar_width / 2.,
                p[0] + s.bar_offset + s.bar_width / 2.,
            ]
        };
        assert_eq!(
            edges(point, bar).map(f64::to_bits),
            edges(actual, restored_bar).map(f64::to_bits)
        );
    }
    // The genuine Patch remains independent of the reconstructed Bar above.
    let original_patch = &before.series[5];
    let replayed_patch = &after.series[5];
    assert_eq!(original_patch.kind, PlotKind::Patch);
    assert_eq!(replayed_patch.kind, PlotKind::Patch);
    assert_eq!(original_patch.patch_face_count(), 2);
    assert_eq!(replayed_patch.patch_face_count(), 2);
    assert_eq!(original_patch.points.len(), 6);
    assert_eq!(replayed_patch.points.len(), original_patch.points.len());
    assert_eq!(replayed_patch.color, original_patch.color);
    assert_eq!(replayed_patch.visible, original_patch.visible);
    assert_eq!(replayed_patch.label, original_patch.label);
    for (actual, expected) in replayed_patch
        .points
        .iter()
        .zip(original_patch.points.iter())
    {
        assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
    }
    assert_eq!(
        after.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
        [3, 0]
    );
    assert!(!after.legend_state.as_ref().unwrap().auto_update);
    assert!(source.figure.as_ref().unwrap().same_snapshot(before));
}

#[test]
fn unchanged_export_executes_empty_series_and_nonfinite_samples_without_erasing_bits() {
    for input in ["plot([],[]);", "plot([0 NaN Inf -Inf],[1 2 3 4]);"] {
        let mut source = Engine::default();
        source.execute(input).unwrap();
        let before = source.figure.as_ref().unwrap();
        let (_, target) = replay_saved_script(before);
        let after = target.figure.as_ref().unwrap();
        assert_eq!(before.series.len(), after.series.len());
        for (a, b) in before.series.iter().zip(&after.series) {
            assert_eq!(a.points.len(), b.points.len());
            for (p, q) in a.points.iter().zip(b.points.iter()) {
                assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
            }
        }
    }
}

#[test]
fn unchanged_export_reconstructs_two_axes_and_actual_shared_title() {
    let mut source = Engine::default();
    source.execute("f=figure('Name','two axes','Color',[.8 .9 1]);subplot(1,2,1);a=gca;h1=plot(a,[1 2],[3 4],'Color',[1 0 0],'LineWidth',2);set(a,'XGrid','off','YGrid','on');title(a,'left literal','Interpreter','none');legend(a,h1,{'left'},'Interpreter','none','AutoUpdate','off');subplot(1,2,2);b=gca;h2=semilogy(b,[1 2],[-2 -4],'Color',[0 0 1],'LineStyle','--','LineWidth',1);set(b,'XGrid','on','YGrid','off','YLim',[-8 -1]);title(b,'right literal','Interpreter','none');legend(b,h2,{'right'},'Interpreter','none','Visible','off');sgtitle('shared literal _ <&>');").unwrap();
    let before = source.figure.as_ref().unwrap();
    let (_, target) = replay_saved_script(before);
    let after = target.figure.as_ref().unwrap();
    after.validate().unwrap();
    assert_eq!(before.all_axes().count(), 2);
    assert_eq!(after.all_axes().count(), 2);
    assert_eq!((&before.name, before.color), (&after.name, after.color));
    assert_eq!(after.axes.title, before.axes.title);
    assert_eq!(after.textboxes, before.textboxes);
    assert_eq!(after.layout.as_ref().unwrap().title, "shared literal _ <&>");
    for ((_, a), (_, b)) in before.all_axes().zip(after.all_axes()) {
        assert_eq!(
            (&a.title, &a.xlabel, &a.ylabel),
            (&b.title, &b.xlabel, &b.ylabel)
        );
        assert_eq!(
            (a.xgrid, a.ygrid, a.xscale, a.yscale, a.legend),
            (b.xgrid, b.ygrid, b.xscale, b.yscale, b.legend)
        );
        assert_eq!(a.bounds(), b.bounds());
        assert_eq!(a.series.len(), b.series.len());
        assert_eq!(
            a.legend_members().map(|(i, _)| i).collect::<Vec<_>>(),
            b.legend_members().map(|(i, _)| i).collect::<Vec<_>>()
        );
        assert_eq!(
            a.legend_state.as_ref().unwrap().auto_update,
            b.legend_state.as_ref().unwrap().auto_update
        );
        for (x, y) in a.series.iter().zip(&b.series) {
            assert_eq!(
                (x.kind, x.color, x.pattern, x.line_width, &x.label),
                (y.kind, y.color, y.pattern, y.line_width, &y.label)
            );
            assert_eq!(x.points.len(), y.points.len());
            for (p, q) in x.points.iter().zip(y.points.iter()) {
                assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
            }
        }
    }
    for ((_, expected), (_, actual)) in before
        .normalized_axes_positions()
        .into_iter()
        .zip(after.normalized_axes_positions())
    {
        for (expected, actual) in expected.into_iter().zip(actual) {
            assert!((expected - actual).abs() <= 1e-14);
        }
    }
}

#[test]
fn positional_scatter_size_is_one_series_and_invalid_sizes_are_atomic() {
    let mut e = Engine::default();
    e.execute("f=figure;a=gca;hold(a,'on');h=scatter(a,[0 1],[2 3],25,'Marker','s');set(h,'LineWidth',2);kind=get(h,'Type');").unwrap();
    assert_eq!(text(&e, "kind"), "scatter");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 1);
    assert_eq!(
        f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
        &[[0., 2.], [1., 3.]]
    );
    assert_eq!(f.series[0].marker_size_points(), 5.);
    assert_eq!(f.series[0].line_width, Some(2.));
    let before = f.clone();
    for script in [
        "scatter(a,[0 1],[2 3],-1);",
        "scatter(a,[0 1],[2 3],Inf);",
        "scatter(a,[0 1],[2 3],f);",
        "scatter(a,[0 1],[2 3],1i);",
        "scatter(a,[0 1],[2 3],[4 9]);",
        "scatter(a,[0 1],[2 3],25,[1 0 0]);",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    }
    e.execute("h2=scatter(a,[0 1],[2 3],25,'SizeData',36);set(h2,'Visible','off');")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series.len(), 2);
    assert_eq!(
        e.figure.as_ref().unwrap().series[1].marker_size_points(),
        6.
    );
    e.execute("h3=scatter(a,[0 1],[2 3],'r');set(h3,'Visible','off');")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series.len(), 3);
    assert_eq!(e.figure.as_ref().unwrap().series[2].color, [255, 0, 0]);
    // A logical scatter size is admitted as a numeric scalar and
    // applies to all points.
    e.execute("h4=scatter(a,[0 1],[2 3],true);set(h4,'LineWidth',3);logical_kind=get(h4,'Type');")
        .unwrap();
    assert_eq!(text(&e, "logical_kind"), "scatter");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 4);
    assert_eq!(
        f.series[3].points.iter().collect::<Vec<_>>().as_slice(),
        &[[0., 2.], [1., 3.]]
    );
    assert_eq!(f.series[3].marker_size_points(), 1.);
    assert_eq!(f.series[3].line_width, Some(3.));
}

#[test]
fn exported_inner_positions_do_not_hold_a_transient_full_canvas_outer_box() {
    let mut original = Engine::default();
    original
        .execute("f=figure;subplot('Position',[.08 .15 .35 .7]);plot([0 1],[2 3]);subplot('Position',[.58 .15 .35 .7]);plot([0 1],[4 5]);")
        .unwrap();
    let f = original.figure.as_ref().unwrap();
    let (script, mut replay) = replay_saved_script(f);
    assert_eq!(
        script
            .matches("'ActivePositionProperty','position','NextPlot'")
            .count(),
        2
    );
    let actual = replay.figure.as_ref().unwrap();
    actual.validate().unwrap();
    assert_eq!(actual.all_axes().count(), 2);
    for ((_, expected), (_, observed)) in f
        .normalized_axes_positions()
        .into_iter()
        .zip(actual.normalized_axes_positions())
    {
        assert_eq!(expected, observed);
    }
    let before = actual.clone();
    assert!(
        replay
            .execute(
                "set(mr_ax2,'Position',get(mr_ax1,'Position'),'ActivePositionProperty','position');"
            )
            .is_err()
    );
    assert!(before.same_snapshot(replay.figure.as_ref().unwrap()));
}
#[test]
fn ordinary_export_reopen_retains_annotation_coordinates_and_explicit_layouts_stay_explicit() {
    for prefix in ["", "subplot(1,1,1);"] {
        for (scale, limits, values) in [
            ("linear", "[-3 8]", "[3 6]"),
            ("log", "[.5 8]", "[2 6]"),
            ("log", "[-8 -.5]", "[-2 -6]"),
        ] {
            let mut source = Engine::default();
            source.execute(&format!("{prefix}f=gcf;a=gca;h=plot(a,[1 2],{values});set(a,'YScale','{scale}','YLim',{limits});legend(a,h,{{'line'}},'Interpreter','none','Location','eastoutside');annotation(f,'textbox',[.04 .91 .92 .07],'String','literal <&> \\alpha','Interpreter','none','EdgeColor','none');")).unwrap();
            let before = source.figure.as_ref().unwrap();
            let (_, replay) = replay_saved_script(before);
            let after = replay.figure.as_ref().unwrap();
            assert_eq!(after.layout.is_some(), before.layout.is_some());
            assert_eq!(
                after.textboxes, before.textboxes,
                "No automatic repositioning of annotation/title overlaps"
            );
            assert_eq!(
                after.textboxes[0].position.map(f64::to_bits),
                [0.04_f64, 0.91, 0.92, 0.07].map(f64::to_bits)
            );
            assert_eq!(after.ylim, before.ylim);
            assert_eq!(after.series[0].points, before.series[0].points);
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("replayed.mfig");
            files::save_figure(&path, after).unwrap();
            let reopened = files::load_figure(&path).unwrap();
            assert_eq!(reopened.layout.is_some(), before.layout.is_some());
            assert_eq!(reopened.textboxes, before.textboxes);
        }
    }
    assert!(files::validate_matlab_plot_script_path(std::path::Path::new("gca.m")).is_err());
}

#[test]
fn ordinary_document_with_explicit_placement_keeps_explicit_export_path() {
    let mut engine = Engine::default();
    engine.execute("plot([1 2],[3 4]);").unwrap();
    let mut figure = engine.figure.unwrap();
    figure.axes.placement = Some(figure.initial_axes_placement().unwrap());
    assert!(figure.layout.is_none());
    figure.validate().unwrap();
    let (script, replay) = replay_saved_script(&figure);
    assert!(script.contains("mr_ax1 = axes('Parent',mr_figure"));
    assert!(!script.contains("mr_ax1 = gca;"));
    assert!(script.contains("'ActivePositionProperty'"));
    let after = replay.figure.as_ref().unwrap();
    assert!(after.placement.is_some());
    assert_eq!(
        after.placement.unwrap().requested_outer,
        figure.placement.unwrap().requested_outer
    );
    assert_eq!(after.series[0].points, figure.series[0].points);
}

#[test]
fn shared_title_replay_preserves_manual_and_outside_geometry_without_second_shrink() {
    for manual in [false, true] {
        let mut source = Engine::default();
        source.execute("f=figure(1);subplot(1,2,1);a=gca;h=plot([1 2],[3 4]);title('left');legend(a,h,{'left'},'Location','eastoutside','Interpreter','none');subplot(1,2,2);b=gca;k=semilogy([1 2],[-2 -5]);title('right');set(b,'YLim',[-8 -1]);sgtitle('Shared _ <&>');annotation(f,'textbox',[.7 .95 .2 .04],'String','separate','Interpreter','none','EdgeColor','none');").unwrap();
        if manual {
            source.execute("set(a,'OuterPosition',[0 0 .48 1],'ActivePositionProperty','outerposition');set(b,'OuterPosition',[.52 0 .48 1],'ActivePositionProperty','outerposition');").unwrap();
        }
        let before = source.figure.as_ref().unwrap();
        let (script, replay) = replay_saved_script(before);
        assert!(script.contains("sgtitle(mr_figure,"));
        let after = replay.figure.as_ref().unwrap();
        assert_eq!(
            after.layout.as_ref().unwrap().title,
            before.layout.as_ref().unwrap().title
        );
        assert_eq!(after.textboxes, before.textboxes);
        let a = before.resolved_axes_geometry().unwrap();
        let b = after.resolved_axes_geometry().unwrap();
        assert_eq!(a.len(), b.len());
        for ((_, a), (_, b)) in a.iter().zip(&b) {
            for (a, b) in a.data.values().into_iter().zip(b.data.values()) {
                assert!((a - b).abs() < 1e-12, "{a} != {b}");
            }
            assert_eq!(a.legend.is_some(), b.legend.is_some());
            if let (Some(a), Some(b)) = (a.legend, b.legend) {
                for (a, b) in a.values().into_iter().zip(b.values()) {
                    assert!((a - b).abs() < 1e-12, "legend {a} != {b}");
                }
            }
        }
        for ((_, a), (_, b)) in before.all_axes().zip(after.all_axes()) {
            assert_eq!(a.title, b.title);
            assert_eq!(a.xscale, b.xscale);
            assert_eq!(a.yscale, b.yscale);
            assert_eq!(a.series.len(), b.series.len());
            for (a, b) in a.series.iter().zip(&b.series) {
                assert_eq!(a.points, b.points);
                assert_eq!(
                    before
                        .resolve_appearance(matrust::plot::BaseTheme::Light)
                        .series_color(a),
                    b.color
                );
                assert!(b.explicit_color && b.auto_palette.is_none());
                assert_eq!(a.pattern, b.pattern);
            }
        }
    }
}

#[test]
fn grid_spans_tiled_and_mixed_axes_replay_exact_geometry_with_and_without_shared_title() {
    let scenes = [
        "subplot(3,4,[1 2 5 6]);plot([1 2],[2 3]);title('span left');legend('line','Location','eastoutside');subplot(3,4,[3 4 7 8]);plot([1 2],[4 5]);title('span right');subplot(3,4,[9 10 11 12]);plot([1 2],[6 7]);title('bottom');",
        "tiledlayout(2,3);nexttile(1,[2 1]);plot([1 2],[2 3]);title('tall');legend('line','Location','westoutside');nexttile(2,[1 2]);plot([1 2],[4 5]);title('wide upper');nexttile(5,[1 2]);plot([1 2],[6 7]);title('wide lower');",
        "subplot(1,2,1);plot([1 2],[2 3]);title('grid');legend('line','Location','eastoutside');axes('Position',[.65 .2 .3 .6]);plot([1 2],[4 5]);title('custom');",
    ];
    for scene in scenes {
        for shared in [false, true] {
            let mut source = Engine::default();
            source.execute(scene).unwrap();
            if shared {
                source.execute("sgtitle('Shared <&>');").unwrap();
            }
            let before = source.figure.as_ref().unwrap();
            let original = before.clone();
            let (script, mut replay) = replay_saved_script(before);
            assert!(script.contains("subplot("));
            assert!(script.ends_with("set(mr_figure,'CurrentAxes',mr_ax1);\n"));
            let after = replay.figure.as_ref().unwrap();
            assert_eq!(after.active_position(), before.active_position());
            assert_eq!(after.axes.title, before.axes.title);
            assert!(before.same_snapshot(&original));
            assert_eq!(before.all_axes().count(), after.all_axes().count());
            for ((pa, a), (pb, b)) in before.all_axes().zip(after.all_axes()) {
                assert_eq!(pa, pb, "Grid/custom position identity changed: {scene}");
                if pa.custom.is_none() {
                    assert!(a.placement.is_none() && b.placement.is_none());
                }
                assert_eq!(a.title, b.title);
                assert_eq!(a.legend_location, b.legend_location);
                assert_eq!(a.series.len(), b.series.len());
                for (a, b) in a.series.iter().zip(&b.series) {
                    assert_eq!(a.kind, b.kind);
                    assert_eq!(a.points.len(), b.points.len());
                    for (a, b) in a.points.iter().zip(b.points.iter()) {
                        assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
                    }
                }
            }
            let expected = before.resolved_axes_geometry().unwrap();
            let actual = after.resolved_axes_geometry().unwrap();
            for ((pa, a), (pb, b)) in expected.iter().zip(&actual) {
                assert_eq!(pa, pb);
                for (a, b) in a.data.values().into_iter().zip(b.data.values()) {
                    assert!((a - b).abs() < 1e-12, "data {a} != {b}: {scene}");
                }
                assert_eq!(a.legend.is_some(), b.legend.is_some());
                if let (Some(a), Some(b)) = (a.legend, b.legend) {
                    for (a, b) in a.values().into_iter().zip(b.values()) {
                        assert!((a - b).abs() < 1e-12, "legend {a} != {b}: {scene}");
                    }
                }
            }
            let stable = after.clone();
            assert!(replay.execute("set(mr_ax2,'Position',get(mr_ax1,'Position'),'ActivePositionProperty','position');").is_err());
            assert!(stable.same_snapshot(replay.figure.as_ref().unwrap()));
        }
    }
    assert!(files::validate_matlab_plot_script_path(std::path::Path::new("subplot.m")).is_err());
}
