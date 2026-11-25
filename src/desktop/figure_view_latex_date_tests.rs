use crate::figure_geometry::Geometry;
use crate::figure_view::title_artwork_tests::{paint, title_shapes, visit_shapes};
use eframe::egui;
use matrust::plot::Figure;
use std::sync::atomic::AtomicBool;

const MATH: &str = r"$\frac{a}{b}+\sqrt{x}$";
const DATE: &str = "01-Jan-2026";
const SIZE: egui::Vec2 = egui::vec2(920., 560.);

fn figure(script: &str) -> Figure {
    let mut engine = matrust::Engine::default();
    engine.execute(script).unwrap();
    engine.figure.unwrap()
}

fn ordinary(figure: &Figure) -> (egui::FullOutput, egui::Rect) {
    ordinary_in_pane(figure, SIZE, true)
}

fn ordinary_in_pane(
    figure: &Figure,
    size: egui::Vec2,
    fitted: bool,
) -> (egui::FullOutput, egui::Rect) {
    let Geometry::Ordinary(geometry) =
        Geometry::prepare(1, figure, &AtomicBool::new(false)).unwrap()
    else {
        panic!("expected ordinary figure");
    };
    let ctx = egui::Context::default();
    for _ in 0..3 {
        paint(&ctx, figure, &geometry, fitted, size, vec![]);
    }
    let (output, _) = paint(&ctx, figure, &geometry, fitted, size, vec![]);
    let memory =
        egui_plot::PlotMemory::load(&ctx, egui::Id::new("title_scene")).expect("axes must render");
    (output, *memory.transform().frame())
}

fn layout(figure: &Figure) -> egui::FullOutput {
    let mut canvas = crate::subplot_view::Canvas::default();
    let ctx = egui::Context::default();
    let mut output = egui::FullOutput::default();
    for _ in 0..4 {
        output = crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
                ..Default::default()
            },
            |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    canvas.show(ui, 1, figure, false, None);
                });
            },
        );
    }
    output
}

fn ink(output: &egui::FullOutput, text: &str) -> Vec<egui::Rect> {
    let mut shapes = Vec::new();
    for shape in &output.shapes {
        title_shapes(&shape.shape, shape.clip_rect, text, &mut shapes);
    }
    shapes.into_iter().map(|(ink, ..)| ink).collect()
}

fn one(output: &egui::FullOutput, text: &str) -> egui::Rect {
    let shapes = ink(output, text);
    assert_eq!(shapes.len(), 1, "{text:?} must have one text shape");
    shapes[0]
}

// Rotate back to reading order when checking a vertical label's ink and rules.
fn reading_pos(pos: egui::Pos2, rotated: bool) -> egui::Pos2 {
    if rotated {
        egui::pos2(-pos.y, pos.x)
    } else {
        pos
    }
}

fn check_math(output: &egui::FullOutput, rotated: bool) -> egui::Rect {
    let in_reading_order = |text| {
        let rect = one(output, text);
        egui::Rect::from_two_pos(
            reading_pos(rect.min, rotated),
            reading_pos(rect.max, rotated),
        )
    };
    let a = in_reading_order("a");
    let b = in_reading_order("b");
    let x = in_reading_order("x");
    let radical = in_reading_order("√");
    assert!(a.bottom() < b.top(), "numerator {a:?}, denominator {b:?}");
    assert!(radical.center().x < x.center().x);
    let mut fraction_rule = false;
    let mut overline = false;
    for item in &output.shapes {
        visit_shapes(&item.shape, item.clip_rect, &mut |shape, clip| {
            if let egui::Shape::LineSegment { points, stroke } = shape {
                let [start, end] = points.map(|p| reading_pos(p, rotated));
                if (start.y - end.y).abs() > 0.1 || stroke.width <= 0. {
                    return;
                }
                let span = start.x.min(end.x)..=start.x.max(end.x);
                let y = (start.y + end.y) / 2.;
                if y > a.bottom()
                    && y < b.top()
                    && span.contains(&a.center().x)
                    && span.contains(&b.center().x)
                    && *span.end() < x.left()
                {
                    assert!(points.iter().all(|p| clip.contains(*p)));
                    fraction_rule = true;
                }
                if y < x.top()
                    && x.top() - y < 12.
                    && span.contains(&x.center().x)
                    && *span.start() > a.right()
                    && *span.end() - *span.start() < 2. * x.width() + 4.
                {
                    assert!(points.iter().all(|p| clip.contains(*p)));
                    overline = true;
                }
            }
        });
    }
    assert!(
        fraction_rule,
        "fraction rule must separate the numerator and denominator"
    );
    assert!(overline, "root rule must be over the radicand");
    one(output, "a")
        .union(one(output, "b"))
        .union(one(output, "x"))
}

#[test]
fn latex_xlabel_and_title_paint_stacked_artwork_in_their_bands() {
    for command in ["xlabel", "title"] {
        let figure = figure(&format!(
            "plot(1:5);{command}('{MATH}','Interpreter','latex');"
        ));
        let (output, frame) = ordinary(&figure);
        let math = check_math(&output, false);
        if command == "xlabel" {
            assert!(math.top() > frame.bottom());
        } else {
            assert!(math.bottom() < frame.top());
        }
    }
}

#[test]
fn latex_rotated_y_and_projected_z_labels_rotate_the_rules_with_the_text() {
    for script in [
        format!("plot(1:5);ylabel('{MATH}','Interpreter','latex');"),
        format!("plot3(1:5,1:5,1:5);zlabel('{MATH}','Interpreter','latex');"),
    ] {
        let (output, _) = ordinary(&figure(&script));
        check_math(&output, true);
    }
}

#[test]
fn latex_second_title_row_and_layout_titles_paint_the_same_boxes() {
    let figure_with_rows = figure(&format!(
        "plot(1:5);title({{'Heading';'{MATH}'}},'Interpreter','latex');"
    ));
    let (output, _) = ordinary(&figure_with_rows);
    let math = check_math(&output, false);
    assert!(one(&output, "Heading").bottom() < math.top());

    for script in [
        format!("subplot(1,2,1);plot(1:5);title('{MATH}','Interpreter','latex');"),
        format!("subplot(1,2,1);plot(1:5);sgtitle('{MATH}','Interpreter','latex');"),
        format!("t=tiledlayout(1,2);nexttile;plot(1:5);title(t,'{MATH}','Interpreter','latex');"),
        format!("t=tiledlayout(1,2);title(t,'{MATH}','Interpreter','latex');"),
    ] {
        check_math(&layout(&figure(&script)), false);
    }
    let tex = layout(&figure(r"subplot(1,2,1);plot(1:5);title('\alpha_1');"));
    one(&tex, "α1");
}

#[test]
fn export_latex_subtitle_paints_below_the_title_in_ordinary_and_tiled_views() {
    let ordinary_figure = figure("plot(1:5);[t,s]=title('Heading','Context');");
    let (output, frame) = ordinary(&ordinary_figure);
    let heading = one(&output, "Heading");
    let subtitle = one(&output, "Context");
    assert!(heading.bottom() < subtitle.top());
    assert!(subtitle.bottom() < frame.top());
    assert!(subtitle.height() < heading.height());

    for script in [
        "t=tiledlayout(1,2);nexttile;plot(1:5);title(t,'Heading','Context');",
        "t=tiledlayout(1,2);title(t,'Heading','Context');",
        "t=tiledlayout(1,2);nexttile;plot(1:5);title('Heading','Context');",
    ] {
        let output = layout(&figure(script));
        assert!(
            one(&output, "Heading").bottom() < one(&output, "Context").top(),
            "{script}"
        );
    }

    let rooted =
        figure(r"plot(1:5);title('Heading');subtitle('$\sqrt{x}$','Interpreter','latex');");
    let (output, frame) = ordinary(&rooted);
    let body = one(&output, "x");
    let heading = one(&output, "Heading");
    assert!(heading.bottom() < body.top() && body.bottom() < frame.top());
    assert!(output.shapes.iter().any(|item| {
        let mut found = false;
        visit_shapes(&item.shape, item.clip_rect, &mut |shape, clip| {
            if let egui::Shape::LineSegment {
                points: [start, end],
                ..
            } = shape
            {
                found |= start.y == end.y
                    && start.y < body.top()
                    && start.y > heading.bottom()
                    && start.x <= body.center().x
                    && end.x >= body.center().x
                    && clip.contains(*start)
                    && clip.contains(*end);
            }
        });
        found
    }));
}

#[test]
fn datetime_secondary_text_fits_between_time_ticks_and_axis_label_on_both_axes() {
    let horizontal = figure(
        "t=datetime(2026,1,1)+hours(0:48);plot(t,1:49);xlim([t(1) t(9)]);xlabel('Time');xtickangle(30);",
    );
    assert!(horizontal.svg().contains(DATE));
    let (output, _) = ordinary(&horizontal);
    let date = one(&output, DATE);
    assert!(one(&output, "04:00").bottom() < date.top());
    assert!(date.bottom() < one(&output, "Time").top());

    let vertical = figure("plot(1:9,datetime(2026,1,1)+hours(0:8));ylabel('Time');");
    let (output, _) = ordinary(&vertical);
    let date = one(&output, DATE);
    assert!(one(&output, "Time").right() < date.left());
    assert!(date.right() < one(&output, "04:00").left());
}

#[test]
fn plain_ylabel_keeps_its_native_x_anchor_with_a_date_band() {
    let ylabel_x = |script| {
        let (output, _) = ordinary(&figure(script));
        let mut anchors = Vec::new();
        for item in &output.shapes {
            visit_shapes(&item.shape, item.clip_rect, &mut |shape, _| {
                if let egui::Shape::Text(text) = shape
                    && text.galley.text() == "Time"
                {
                    anchors.push(text.pos.x);
                }
            });
        }
        assert_eq!(anchors.len(), 1);
        anchors[0]
    };
    let native = ylabel_x("plot(1:9,1:9);ylabel('Time');");
    let with_date = ylabel_x("plot(1:9,datetime(2026,1,1)+hours(0:8));ylabel('Time');");
    assert_eq!(with_date, native);
}

#[test]
fn offscreen_rotated_tick_does_not_shrink_data_frame() {
    let height = |ticks| {
        let figure = figure(&format!(
            r"plot(1:2);xlim([1 2]);{ticks}xtickangle(-90);xlabel('$\sqrt{{x}}$','Interpreter','latex');"
        ));
        ordinary(&figure).1.height()
    };
    assert_eq!(
        height("xticks([1 2 99]);xticklabels({'A','B',repmat('W',1,512)});"),
        height("xticks([1 2]);xticklabels({'A','B'});")
    );
}

#[test]
fn custom_tick_markup_is_laid_out_once_for_each_visible_tick() {
    let figure =
        figure(r"plot(1:2);xticks(1:1024);xlim([1 2]);xlabel('$\sqrt{x}$','Interpreter','latex');");
    let Geometry::Ordinary(geometry) =
        Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap()
    else {
        panic!("expected ordinary figure");
    };
    let ctx = egui::Context::default();
    let id = egui::Id::new("title_scene").with(crate::figure_view::CUSTOM_TICK_LAYOUT_COUNT);
    for _ in 0..4 {
        paint(&ctx, &figure, &geometry, true, SIZE, vec![]);
        assert_eq!(ctx.data_mut(|data| data.remove_temp::<usize>(id)), Some(2));
    }
}

#[test]
fn short_pane_drops_the_date_before_flattening_the_xlabel() {
    let figure = figure(&format!(
        "plot(datetime(2026,1,1)+hours(0:8),1:9);title('Heading');xlabel('{MATH}','Interpreter','latex');"
    ));
    for fitted in [true, false] {
        for (height, stacked) in [(154., true), (128., false)] {
            let (output, frame) = ordinary_in_pane(&figure, egui::vec2(SIZE.x, height), fitted);
            assert!(frame.is_positive());
            assert!(ink(&output, DATE).is_empty());
            let tick = one(&output, "04:00");
            let label = if stacked {
                check_math(&output, false)
            } else {
                one(&output, "a/b+√(x)")
            };
            assert!(frame.bottom() <= tick.top() && tick.bottom() < label.top());
        }
    }
}

#[test]
fn narrow_pane_paints_the_axes_with_a_flat_rotated_ylabel() {
    let figure = figure(&format!(
        "plot(1:9,datetime(2026,1,1)+hours(0:8));ylabel('{MATH}','Interpreter','latex');"
    ));
    let (output, frame) = ordinary_in_pane(&figure, egui::vec2(120., SIZE.y), true);
    assert!(frame.is_positive());
    assert!(ink(&output, DATE).is_empty());
    let label = one(&output, "a/b+√(x)");
    let tick = one(&output, "04:00");
    assert!(label.height() > label.width());
    assert!(label.right() < tick.left() && tick.right() <= frame.left());
}

#[test]
fn short_pane_paints_a_flat_title_when_only_one_row_fits() {
    let figure = figure(&format!("plot(1:9);title('{MATH}','Interpreter','latex');"));
    let (output, frame) = ordinary_in_pane(&figure, egui::vec2(SIZE.x, 128.), true);
    assert!(frame.is_positive());
    assert!(one(&output, "a/b+√(x)").bottom() < frame.top());
}

#[test]
fn wider_datetime_ranges_custom_format_and_duration_do_not_add_a_date_row() {
    let prefix = "t=datetime(2026,1,1)+hours(0:48);plot(t,1:49);xlabel('Time');";
    let wide = figure(prefix);
    let (output, frame) = ordinary(&wide);
    let labels = wide.effective_tick_labels(0);
    let date_tick = one(&output, &labels[1]);
    assert!(date_tick.bottom() < one(&output, "Time").top());
    assert!(
        ink(&output, DATE)
            .iter()
            .all(|date| date.top() - frame.bottom() < 6.),
        "the full range must have only its primary tick row"
    );

    for script in [
        format!("{prefix}xlim([t(1) t(9)]);xtickformat('HH:mm');"),
        "plot(hours(0:8),1:9);xlabel('Time');".into(),
    ] {
        assert!(ink(&ordinary(&figure(&script)).0, DATE).is_empty());
    }
}

#[test]
fn zooming_the_48_hour_plot_adds_the_date_when_the_visible_range_is_subday() {
    let figure = figure("plot(datetime(2026,1,1)+hours(0:48),1:49);xlabel('Time');");
    let Geometry::Ordinary(geometry) =
        Geometry::prepare(1, &figure, &AtomicBool::new(false)).unwrap()
    else {
        panic!("expected ordinary figure");
    };
    let ctx = egui::Context::default();
    for _ in 0..3 {
        paint(&ctx, &figure, &geometry, true, SIZE, vec![]);
    }
    let id = egui::Id::new("title_scene");
    let before = egui_plot::PlotMemory::load(&ctx, id).unwrap();
    let frame = *before.transform().frame();
    let pointer = egui::pos2(frame.left() + frame.width() / 4., frame.center().y);
    paint(
        &ctx,
        &figure,
        &geometry,
        true,
        SIZE,
        vec![egui::Event::PointerMoved(pointer)],
    );
    paint(
        &ctx,
        &figure,
        &geometry,
        true,
        SIZE,
        vec![egui::Event::Zoom(6.)],
    );
    paint(
        &ctx,
        &figure,
        &geometry,
        true,
        SIZE,
        vec![egui::Event::PointerGone],
    );
    let (output, _) = paint(&ctx, &figure, &geometry, true, SIZE, vec![]);
    let after = egui_plot::PlotMemory::load(&ctx, id).unwrap();
    assert!(after.bounds().width() < 1.);
    let frame = *after.transform().frame();
    let date = one(&output, DATE);
    assert!(date.top() - frame.bottom() > 10.);
    assert!(date.bottom() < one(&output, "Time").top());
}

#[test]
fn plain_xlabel_stays_one_text_shape() {
    let (output, frame) = ordinary(&figure("plot(1:5);xlabel('abc');"));
    assert!(one(&output, "abc").top() > frame.bottom());
}
