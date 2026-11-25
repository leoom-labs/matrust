use matrust::{Array, Engine};
use tempfile::tempdir;

fn run(script: &str) -> Engine {
    let mut engine = Engine::default();
    engine.execute(script).unwrap();
    engine
}

fn text_value(engine: &Engine, name: &str) -> String {
    match engine.workspace().get(name).unwrap() {
        Array::Character(ch) => String::from_utf16_lossy(ch.units()),
        other => panic!("expected character array for {name}, got {other:?}"),
    }
}

fn logical_scalar(engine: &Engine, name: &str) -> bool {
    match engine.workspace().get(name).unwrap() {
        Array::Logical(l) => l.data()[0] != 0,
        other => panic!("expected logical scalar for {name}, got {other:?}"),
    }
}

fn cell_strings(engine: &Engine, name: &str) -> Vec<String> {
    match engine.workspace().get(name).unwrap() {
        Array::Cell(cell) => (0..cell.len())
            .map(|i| match cell.get_linear(i).unwrap().as_ref() {
                Array::Character(ch) => String::from_utf16_lossy(ch.units()),
                other => panic!("expected char cell, got {other:?}"),
            })
            .collect(),
        other => panic!("expected cell for {name}, got {other:?}"),
    }
}

#[test]
fn plot_time_axes_datetime_xlim_and_ticks() {
    let e = run("d = datetime(2024, 1, 1:3); \
         plot(d, 1:3); \
         xl = get(gca, 'XLim'); \
         c = class(xl); \
         ok = xl(1) <= datetime(2024, 1, 1); \
         xt = xticks; \
         whole = all(hour(xt) == 0 & minute(xt) == 0 & second(xt) == 0); \
         lbls = xticklabels; \
         r = gca().XAxis; \
         rc = class(r);");
    assert_eq!(text_value(&e, "c"), "datetime");
    assert!(logical_scalar(&e, "ok"));
    assert!(logical_scalar(&e, "whole"));
    let lbls = cell_strings(&e, "lbls");
    assert!(!lbls.is_empty());
    assert!(
        lbls.iter().all(|s| s.contains("Jan")),
        "expected 'Jan' in labels, got {lbls:?}"
    );
    assert_eq!(
        text_value(&e, "rc"),
        "matlab.graphics.axis.decorator.DatetimeRuler"
    );
}

#[test]
fn plot_time_axes_duration_and_xtickformat() {
    let e = run("plot(hours(1:3), 1:3); \
         xl = get(gca, 'XLim'); \
         c = class(xl); \
         rc = class(gca().XAxis); \
         xtickformat('hh:mm'); \
         fmt = xtickformat; \
         lbls = xticklabels;");
    assert_eq!(text_value(&e, "c"), "duration");
    assert_eq!(
        text_value(&e, "rc"),
        "matlab.graphics.axis.decorator.DurationRuler"
    );
    assert_eq!(text_value(&e, "fmt"), "hh:mm");
    let lbls = cell_strings(&e, "lbls");
    assert!(!lbls.is_empty());
    assert!(lbls.iter().any(|s| s.contains(':')), "got {lbls:?}");
}

#[test]
fn plot_time_axes_categorical_bar_order() {
    let e = run("c = categorical({'b', 'a', 'c'}); \
         bar(c, [1 2 3]); \
         xt = get(gca, 'XTick'); \
         xc = class(xt); \
         xlc = class(get(gca, 'XLim')); \
         rc = class(gca().XAxis); \
         cats = gca().XAxis.Categories; \
         lbls = xticklabels;");
    assert_eq!(text_value(&e, "xc"), "categorical");
    assert_eq!(text_value(&e, "xlc"), "categorical");
    assert_eq!(
        text_value(&e, "rc"),
        "matlab.graphics.axis.decorator.CategoricalRuler"
    );
    assert_eq!(cell_strings(&e, "cats"), vec!["a", "b", "c"]);
    assert_eq!(cell_strings(&e, "lbls"), vec!["a", "b", "c"]);
}

#[test]
fn plot_time_axes_scatter_area_line_stairs_stem() {
    let e = run("d = datetime(2024, 1, 1:4); \
         scatter(d, [1 2 3 4]); \
         c1 = class(get(gca, 'XLim')); \
         area(d, [1 2 3 4]); \
         c2 = class(get(gca, 'XLim')); \
         stairs(d, [1 2 3 4]); \
         stem(d, [1 2 3 4]); \
         line(d, [4 3 2 1]); \
         xlim([datetime(2024,1,1) datetime(2024,1,5)]); \
         xl = xlim; \
         ok = xl(2) == datetime(2024,1,5);");
    assert_eq!(text_value(&e, "c1"), "datetime");
    assert_eq!(text_value(&e, "c2"), "datetime");
    assert!(logical_scalar(&e, "ok"));
}

#[test]
fn plot_time_axes_hold_mismatch_errors() {
    let mut e = Engine::default();
    e.execute("plot(datetime(2024, 1, 1:3), 1:3); hold on;")
        .unwrap();
    let err = e.execute("plot(1:3, 1:3);").unwrap_err();
    assert!(
        err.contains("datetime"),
        "expected datetime mismatch error, got: {err}"
    );
}

#[test]
fn plot_time_axes_subday_secondary_date_label_and_nat() {
    let e = run(
        "d = [datetime(2024, 1, 1, 10, 0, 0), NaT, datetime(2024, 1, 1, 14, 0, 0)]; \
         plot(d, [1 2 3]); \
         xtickangle(30);",
    );
    let svg = e.figure.unwrap().svg();
    assert!(
        svg.contains("01-Jan-2024"),
        "expected secondary date label in SVG"
    );
}

#[test]
fn plot_time_axes_exportgraphics_png_and_save_load() {
    let dir = tempdir().unwrap();
    let png_path = dir.path().join("dt.png");
    let fig_path = dir.path().join("dt.mrfig");
    let mut e = Engine::default();
    e.execute(&format!(
        "plot(datetime(2024, 1, 1:3), 1:3); \
         exportgraphics(gcf, '{}'); \
         savefig('{}'); \
         close all; \
         openfig('{}'); \
         c = class(get(gca, 'XLim'));",
        png_path.display(),
        fig_path.display(),
        fig_path.display(),
    ))
    .unwrap();
    assert_eq!(text_value(&e, "c"), "datetime");

    let img = image::open(&png_path).unwrap().to_rgba8();
    let first = img.get_pixel(0, 0);
    let has_non_bg = img.pixels().any(|p| p != first);
    assert!(has_non_bg, "exported PNG must have non-background pixels");
}

#[test]
fn plot_time_axes_hour_ticks_do_not_truncate_to_59() {
    let e = run("plot(datetime(2026,1,1)+hours(0:8),1:9); lbls = xticklabels;");
    let lbls = cell_strings(&e, "lbls");
    assert_eq!(
        lbls,
        [
            "00:00", "01:00", "02:00", "03:00", "04:00", "05:00", "06:00", "07:00", "08:00"
        ]
    );
}
