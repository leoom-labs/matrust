/// Painted text shapes: (galley text, visual rect, clip rect).
fn value_column_texts(output: &egui::FullOutput) -> Vec<(String, egui::Rect, egui::Rect)> {
    fn walk(
        shape: &egui::Shape,
        clip: egui::Rect,
        out: &mut Vec<(String, egui::Rect, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Text(t) => {
                out.push((t.galley.text().into(), t.visual_bounding_rect(), clip))
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &output.shapes {
        walk(&shape.shape, shape.clip_rect, &mut out);
    }
    out
}

#[test]
fn workspace_keeps_value_column_and_full_name_tooltip_at_800_and_1366() {
    let preview = "[0.0855 0.2625 0.5000 0.7500]";
    for size in [egui::vec2(800., 600.), egui::vec2(1366., 768.)] {
        for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
            for dock_position in [
                session::WorkspaceDock::BelowFiles,
                session::WorkspaceDock::Right,
            ] {
                let ctx = egui::Context::default();
                let mut app = Workspace::with_context(ctx.clone(), true);
                app.layout.workspace_dock = Some(dock_position);
                app.layout.theme = Some(theme);
                app.apply_desktop_theme(&ctx);
                app.run("v=[0.0855 0.2625 0.5 0.75];".into());
                finish(&mut app);
                let shared = app.variables["v"].clone();
                app.variables.clear();
                let names: Vec<String> = (0..40)
                    .map(|i| format!("var_longitude_signal_name_{i:02}"))
                    .collect();
                for name in &names {
                    app.variables.insert(name.clone(), shared.clone());
                }
                let mut output = frame_at(&mut app, &ctx, size, vec![]);
                for _ in 0..4 {
                    output = frame_at(&mut app, &ctx, size, vec![]);
                }
                let texts = value_column_texts(&output);
                let shown = |text: &(String, egui::Rect, egui::Rect)| {
                    text.2.intersect(text.1).width() > 0.5
                };
                let header = texts
                    .iter()
                    .find(|t| t.0 == "VALUE" && shown(t))
                    .unwrap_or_else(|| panic!("VALUE header hidden at {size:?}"));
                assert!(
                    header.2.width() >= 95.5,
                    "VALUE column below 96 px: {header:?}"
                );
                let name_header = texts.iter().find(|t| t.0 == "NAME" && shown(t)).unwrap();
                assert!(
                    name_header.2.width() >= 95.5,
                    "NAME column below 96 px: {name_header:?}"
                );
                let dock = workspace_dock_rect(
                    &ctx,
                    if app.workspace_on_left() {
                        "workspace_variables_left"
                    } else {
                        "workspace_variables"
                    },
                );
                let minimum = workspace_tools::minimum_dock_width(&ctx);
                assert!(dock.width() >= minimum - 0.5);
                if size.x == 800. || !app.workspace_on_left() {
                    assert!((dock.width() - minimum).abs() < 2.);
                }
                let values: Vec<_> = texts
                    .iter()
                    .filter(|t| t.0 == preview && shown(t))
                    .collect();
                assert!(!values.is_empty(), "No value preview painted at {size:?}");
                for value in &values {
                    let visible = value.2.intersect(value.1);
                    assert!(
                        visible.width() >= 40.,
                        "Value squeezed to {visible:?} at {size:?}"
                    );
                    assert!(
                        (value.1.left() - header.1.left()).abs() < 3.,
                        "{size:?} {value:?} {header:?}"
                    );
                }
                // Long names keep their distinguishing suffix around one ellipsis.
                let (row_name, rect) = names
                    .iter()
                    .find_map(|name| {
                        texts.iter().find_map(|t| {
                            let (head, tail) = t.0.split_once('…')?;
                            (shown(t)
                                && !tail.is_empty()
                                && name.starts_with(head)
                                && name.ends_with(tail)
                                && tail.ends_with(&name[name.len() - 2..]))
                            .then_some((name.clone(), t.1))
                        })
                    })
                    .unwrap_or_else(|| panic!("No middle-truncated name at {size:?}"));
                // Hovering the row shows the full name, class, size and preview.
                frame_at(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::PointerMoved(rect.center())],
                );
                let tooltip = (0..90).find_map(|_| {
                    let output = frame_at(&mut app, &ctx, size, vec![]);
                    value_column_texts(&output)
                        .into_iter()
                        .find(|t| t.0.contains(&row_name) && t.0.contains('\n'))
                });
                let (tooltip, _, _) =
                    tooltip.unwrap_or_else(|| panic!("No row tooltip at {size:?}"));
                assert!(tooltip.contains("double"), "{tooltip}");
                assert!(tooltip.contains("1 x 4"), "{tooltip}");
                assert!(tooltip.contains(preview), "{tooltip}");
            }
        }
    }
}
