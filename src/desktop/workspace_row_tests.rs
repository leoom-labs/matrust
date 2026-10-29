use super::*;

fn table_frame(
    app: &mut Workspace,
    ctx: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width + 16., 400.),
            )),
            events,
            ..Default::default()
        },
        |root| {
            egui::CentralPanel::default().show(root, |ui| {
                ui.set_width(width);
                app.workspace_variables_ui(ui);
            });
        },
    )
}
fn texts(
    shape: &egui::Shape,
    clip: egui::Rect,
    found: &mut Vec<(egui::epaint::TextShape, egui::Rect)>,
) {
    match shape {
        egui::Shape::Text(t) => found.push((t.clone(), clip)),
        egui::Shape::Vec(v) => {
            for s in v {
                texts(s, clip, found);
            }
        }
        _ => {}
    }
}
fn all_text(output: &egui::FullOutput) -> Vec<(egui::epaint::TextShape, egui::Rect)> {
    let mut result = Vec::new();
    for s in &output.shapes {
        texts(&s.shape, s.clip_rect, &mut result);
    }
    result
}
fn pointer(app: &mut Workspace, ctx: &egui::Context, width: f32, pos: egui::Pos2, pressed: bool) {
    table_frame(
        app,
        ctx,
        width,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
}
fn key(app: &mut Workspace, ctx: &egui::Context, width: f32, key: egui::Key) {
    table_frame(
        app,
        ctx,
        width,
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
}

#[test]
fn narrow_and_wide_workspace_types_are_complete_for_typed_empty_and_nonempty_values() {
    let mut engine = matrust::Engine::default();
    engine.execute("R=[1 2];L=logical([0 1]);Z=complex([1 2],[3 4]);Zero=complex([1 2]);Empty=complex(zeros(0,3));Text=char([]);Chars='hello';EmptyCell=cell(0,2);Huge=zeros(0,16777216);C={R,'label'};H=figure;").unwrap();
    let values = engine.workspace();
    let long_name = "long_variable_name_".to_owned() + &"x".repeat(44);
    assert_eq!(long_name.len(), 63);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        for width in [150., 180., 220., 480.] {
            for value in values.values() {
                let ctx = egui::Context::default();
                ctx.set_theme(theme);
                let mut app = Workspace::with_context(ctx.clone(), true);
                app.variables = BTreeMap::from([(long_name.clone(), value.clone())]);
                let mut output = table_frame(&mut app, &ctx, width, vec![]);
                for _ in 0..3 {
                    output = table_frame(&mut app, &ctx, width, vec![]);
                }
                let rendered = all_text(&output);
                // One line per variable: NAME | VALUE | SIZE chip. When the
                // pane is short the SIZE chip goes first; NAME and VALUE stay.
                // The type and size stay in the row's hover text.
                let size = format!("{}x{}", value.rows(), value.cols());
                let dimensions = rendered.iter().find(|(t, _)| t.galley.text() == size);
                // The long name keeps its start and end around one ellipsis.
                let (name, _) = rendered
                    .iter()
                    .find(|(t, _)| {
                        let text = t.galley.text();
                        text == long_name
                            || text.split_once('…').is_some_and(|(head, tail)| {
                                long_name.starts_with(head)
                                    && long_name.ends_with(tail)
                                    && !tail.is_empty()
                            })
                    })
                    .unwrap_or_else(|| panic!("Missing name at {width}"));
                let preview = row::value_preview(value);
                let (shown, _) = rendered
                    .iter()
                    .find(|(t, _)| t.galley.text() == preview)
                    .unwrap_or_else(|| panic!("Missing {preview} at {width}"));
                assert!((shown.pos.y - name.pos.y).abs() < 2.);
                assert!(
                    !shown
                        .visual_bounding_rect()
                        .intersects(name.visual_bounding_rect())
                );
                if width >= 300. {
                    let (dimensions, size_clip) =
                        dimensions.unwrap_or_else(|| panic!("Missing {size} at {width}"));
                    assert!(
                        !dimensions.galley.elided,
                        "Dimensions were truncated at {width}"
                    );
                    assert!(
                        size_clip.contains_rect(dimensions.visual_bounding_rect()),
                        "Dimension glyph clipping: theme={theme:?} width={width} class={} size={size} clip={size_clip:?} glyph={:?}",
                        value.class(),
                        dimensions.visual_bounding_rect(),
                    );
                    assert!(
                        ctx.content_rect()
                            .contains_rect(dimensions.visual_bounding_rect())
                    );
                    for text in [name, shown] {
                        assert!(
                            !text
                                .visual_bounding_rect()
                                .intersects(dimensions.visual_bounding_rect())
                        );
                    }
                } else {
                    assert!(dimensions.is_none(), "SIZE kept at {width}");
                }
                assert!(app.variables[&long_name].same_snapshot(value));
                let pos = name.pos + name.galley.size() * 0.5;
                pointer(&mut app, &ctx, width, pos, true);
                pointer(&mut app, &ctx, width, pos, false);
                assert_eq!(app.selected.as_deref(), Some(long_name.as_str()));
                key(&mut app, &ctx, width, egui::Key::Enter);
                assert_eq!(app.editing_variable.as_deref(), Some(long_name.as_str()));
                assert!(app.variables[&long_name].same_snapshot(value));
            }
        }
    }
}

#[test]
fn workspace_type_mode_resize_keeps_virtual_row_anchor_selection_and_shared_payload() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let shared = matrust::Array::from(matrust::Matrix::scalar(1.));
    app.variables = (0..10000)
        .map(|i| (format!("v{i:05}"), shared.clone()))
        .collect();
    let mut output = table_frame(&mut app, &ctx, 180., vec![]);
    for _ in 0..3 {
        output = table_frame(&mut app, &ctx, 180., vec![]);
    }
    let text = all_text(&output)
        .into_iter()
        .find(|(t, _)| t.galley.text() == "v00000")
        .unwrap()
        .0;
    let pos = text.pos + text.galley.size() * 0.5;
    pointer(&mut app, &ctx, 180., pos, true);
    pointer(&mut app, &ctx, 180., pos, false);
    for _ in 0..12 {
        key(&mut app, &ctx, 180., egui::Key::PageDown);
    }
    for _ in 0..3 {
        table_frame(&mut app, &ctx, 180., vec![]);
    }
    let selected = app.selected.clone();
    let narrow_stride = app.workspace_tools.row_stride;
    let top = app.workspace_tools.scroll_offset / narrow_stride;
    assert!(
        top > 2. && top < 9990.,
        "Require an interior scrolled anchor"
    );
    for width in [480., 180., 480., 180.] {
        for _ in 0..3 {
            output = table_frame(&mut app, &ctx, width, vec![]);
        }
        assert!(output.shapes.len() < 1500, "Virtualization lost");
        assert!(
            (app.workspace_tools.scroll_offset / app.workspace_tools.row_stride - top).abs() < 0.1
        );
        assert_eq!(app.selected, selected);
        // Rows are one line at every width.
        assert_eq!(app.workspace_tools.row_stride, narrow_stride);
        assert!(
            app.variables
                .values()
                .all(|value| value.same_snapshot(&shared))
        );
    }
    key(&mut app, &ctx, 180., egui::Key::End);
    assert_eq!(app.selected.as_deref(), Some("v09999"));
    app.documents[app.active].text = "unsaved=1;".into();
    app.documents[app.active].changed();
    app.request_script_close(app.active);
    assert!(app.file_action_pending());
    key(&mut app, &ctx, 180., egui::Key::Home);
    assert_eq!(app.selected.as_deref(), Some("v09999"));
}

#[test]
fn large_size_chip_yields_before_name_or_value_falls_below_minimum() {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    let mut engine = matrust::Engine::default();
    engine
        .execute("long_variable_name_12=zeros(0,16777216);")
        .unwrap();
    app.variables = engine.workspace();
    let mut output = table_frame(&mut app, &ctx, 304., vec![]);
    for _ in 0..3 {
        output = table_frame(&mut app, &ctx, 304., vec![]);
    }
    let texts = all_text(&output);
    assert!(!texts.iter().any(|(t, _)| t.galley.text() == "0x16777216"));
    let preview = row::value_preview(&app.variables["long_variable_name_12"]);
    for (text, clip) in &texts {
        if text.galley.text() == preview || text.galley.text().contains('…') {
            assert!(clip.width() >= 95.5, "column squeezed to {clip:?}");
        }
    }
    assert!(texts.iter().any(|(t, _)| t.galley.text() == preview));
}

#[test]
fn value_column_shows_scalars_short_vectors_and_sizes_like_matlab() {
    let mut engine = matrust::Engine::default();
    engine
        .execute(
            "s=16; p=0.8147; big=123456.7; r=[1 2 3]; h=[0.5 1.25]'; l=1:30; m=magic(4); \
             z=[1+2i 3]; zs=1+2i; c=['ab';'cd';'ef']; e=zeros(0,3); \
             ts=single(0.8147); ti=int8(5); tv=int8([1 2 3]); tm=int16(magic(4));",
        )
        .unwrap();
    let vars = engine.workspace();
    for (name, text) in [
        ("s", "16"),
        ("p", "0.8147"),
        ("big", "1.2346e+05"),
        ("r", "[1 2 3]"),
        ("h", "[0.5000;1.2500]"),
        ("l", "1x30 double"),
        ("m", "4x4 double"),
        ("z", "1x2 double (complex)"),
        ("zs", "1+2i"),
        ("c", "3x2 char"),
        ("e", "[]"),
        ("ts", "0.8147"),
        ("ti", "5"),
        ("tv", "[1 2 3]"),
        ("tm", "4x4 int16"),
    ] {
        assert_eq!(row::value_preview(&vars[name]), text, "{name}");
    }
}
