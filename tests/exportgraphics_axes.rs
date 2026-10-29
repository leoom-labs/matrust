#![cfg(feature = "plot-export")]
use matrust::{Engine, export, plot::Position};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn engine(dir: &std::path::Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(dir).unwrap();
    e
}
fn root_size(svg: &str) -> [u32; 2] {
    let root = svg.split('>').next().unwrap();
    ["width", "height"].map(|name| {
        root.split(&format!("{name}=\""))
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    })
}

#[test]
fn ordinary_axes_export_excludes_figure_textbox_and_keeps_figure_export_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("f=figure(1);a=gca;h=plot(a,[0 1 2],[2 3 1],'Color',[1 0 0],'LineWidth',2);title(a,'TARGET TITLE');xlabel(a,'TARGET X');ylabel(a,'TARGET Y');annotation(f,'textbox',[.1 .9 .7 .05],'String','FIGURE ONLY','Interpreter','none','EdgeColor','none');").unwrap();
    let before = e.figure.clone().unwrap();
    e.execute("exportgraphics(a,'axes.svg');exportgraphics(f,'figure.svg');exportgraphics(1,'numeric.svg');").unwrap();
    let axes = std::fs::read_to_string(dir.path().join("axes.svg")).unwrap();
    let figure = std::fs::read_to_string(dir.path().join("figure.svg")).unwrap();
    assert!(
        axes.contains("TARGET TITLE") && axes.contains("TARGET X") && axes.contains("TARGET Y")
    );
    assert!(!axes.contains("FIGURE ONLY"));
    assert!(figure.contains("FIGURE ONLY"));
    assert_eq!(
        figure.as_bytes(),
        std::fs::read(dir.path().join("numeric.svg")).unwrap()
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    assert!((before.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    assert!(e.take_figure_file_events().is_empty());
}

#[test]
fn inactive_axes_export_keeps_negative_log_limits_and_ordered_legend_subset() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("figure(1);subplot(1,2,1);left=gca;plot(left,[1 2],[8 9]);title(left,'UNRELATED LEFT');subplot(1,2,2);right=gca;h=semilogy(right,[1 2],[-2 -4],[1 2],[-3 -6],[1 2],[-4 -7]);title(right,'SELECTED RIGHT');xlabel(right,'selected x');ylabel(right,'selected log y');set(right,'XLim',[0 3],'YLim',[-10 -1],'XGrid','off','YGrid','on');legend(right,[h(2) h(1)],{'SECOND KEY','FIRST KEY'},'Interpreter','none','AutoUpdate','off');sgtitle('SHARED ONLY');figure(2);plot([100 200]);title('UNRELATED FIGURE');").unwrap();
    let before = e.figures().map(|(n, f)| (n, f.clone())).collect::<Vec<_>>();
    e.execute("exportgraphics(right,'right.svg','ContentType','vector');")
        .unwrap();
    let svg = std::fs::read_to_string(dir.path().join("right.svg")).unwrap();
    assert!(svg.contains("SELECTED RIGHT") && svg.contains("selected log y"));
    assert!(svg.find("SECOND KEY").unwrap() < svg.find("FIRST KEY").unwrap());
    for excluded in ["UNRELATED LEFT", "UNRELATED FIGURE", "SHARED ONLY"] {
        assert!(!svg.contains(excluded));
    }
    assert_eq!(e.current_figure, 2);
    for ((number, old), (new_number, new)) in before.iter().zip(e.figures()) {
        assert_eq!(*number, new_number);
        assert!(old.same_snapshot(new));
        for ((_, a), (_, b)) in old.all_axes().zip(new.all_axes()) {
            assert!(a.identity.same_object(&b.identity));
            for (x, y) in a.series.iter().zip(&b.series) {
                assert!((x.points).same_storage(&y.points));
            }
        }
    }
    assert!(e.take_figure_file_events().is_empty());
}

#[test]
fn custom_position_and_outer_constraint_are_retained_and_raster_uses_cropped_axes_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("figure(1);a=axes('Position',[.2 .2 .5 .5]);h=plot(a,[0 1],[1 2]);title(a,'MANUAL');set(a,'OuterPosition',[.05 .05 .9 .9],'ActivePositionProperty','outerposition');legend(a,h,{'KEY'},'Interpreter','none','Location','eastoutside');").unwrap();
    let before = e.figure.clone().unwrap();
    assert!(before.axes.placement.is_some());
    e.execute("exportgraphics(a,'manual.svg');exportgraphics(a,'manual.png','Resolution',96);exportgraphics(a,'manual.pdf','ContentType','vector');").unwrap();
    let svg = std::fs::read_to_string(dir.path().join("manual.svg")).unwrap();
    assert!(svg.contains("MANUAL") && svg.contains("KEY"));
    let dimensions = root_size(&svg);
    assert!(
        dimensions[0] > 16 && dimensions[1] > 16 && dimensions[0] <= 960 && dimensions[1] <= 560
    );
    let image = image::open(dir.path().join("manual.png")).unwrap();
    assert_eq!([image.width(), image.height()], dimensions);
    assert!(
        std::fs::read(dir.path().join("manual.pdf"))
            .unwrap()
            .starts_with(b"%PDF-")
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    assert_eq!(
        before.axes.placement,
        e.figure.as_ref().unwrap().axes.placement
    );
}

#[test]
fn stale_foreign_array_and_non_axes_handles_preserve_existing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("figure(1);a=gca;h=plot(a,[1 2]);stale=a;clf;a=gca;h=plot(a,[3 4]);")
        .unwrap();
    let mut other = Engine::default();
    other.execute("plot([5 6]);foreign=gca;").unwrap();
    e.merge_workspace(BTreeMap::from([(
        "foreign".into(),
        other.array("foreign").unwrap(),
    )]))
    .unwrap();
    let before = e.figure.clone().unwrap();
    let path = dir.path().join("keep.svg");
    std::fs::write(&path, b"original").unwrap();
    for script in [
        "exportgraphics(stale,'keep.svg')",
        "exportgraphics(foreign,'keep.svg')",
        "exportgraphics(h,'keep.svg')",
        "exportgraphics([a a],'keep.svg')",
        "exportgraphics(a,'keep.svg','Width',400)",
        "exportgraphics(a,'keep.svg','ContentType','image')",
    ] {
        assert!(e.execute(script).is_err(), "{script}");
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    }
    assert!(e.take_figure_file_events().is_empty());
}

#[test]
fn empty_missing_and_cancelled_axes_never_replace_destination() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(dir.path());
    e.execute("figure(1);subplot(1,2,1);a=gca;plot(a,[1 2]);subplot(1,2,2);empty=gca;")
        .unwrap();
    let path = dir.path().join("keep.svg");
    std::fs::write(&path, b"original").unwrap();
    assert!(e.execute("exportgraphics(empty,'keep.svg')").is_err());
    assert!(
        export::export_axes_graphics_with_cancel(
            e.figure.as_ref().unwrap(),
            Position::from_grid(2, 2, &[4]).unwrap(),
            &path,
            "svg",
            export::GraphicsOptions::default(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(e.execute("exportgraphics(a,'keep.svg')").is_err());
    e.cancellation().store(false, Ordering::Relaxed);
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
