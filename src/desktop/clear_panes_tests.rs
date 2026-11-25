// Clear panes (step ui-windows-clarity, check 2): every pane has a text
// title, panes are separated by a 1 px `pane_border` of >= 3:1 against both
// surfaces, and toolstrip and pane-header buttons show text.

const CLEAR_PANES_SIZES: [egui::Vec2; 2] = [egui::vec2(1366., 768.), egui::vec2(1024., 640.)];

/// A text shape of a frame: text, layout box, glyph box and whether it is
/// the invisible 1 px name tests use to find buttons (it has no glyphs).
struct ClearPanesText {
    text: String,
    rect: egui::Rect,
    glyphs: egui::Rect,
    clip: egui::Rect,
    marker: bool,
}

fn clear_panes_texts(output: &egui::FullOutput) -> Vec<ClearPanesText> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<ClearPanesText>) {
        match shape {
            // The layout box: a 1 px marker has no glyph box.
            egui::Shape::Text(t) => out.push(ClearPanesText {
                text: t.galley.text().to_owned(),
                rect: egui::Rect::from_min_size(t.pos, t.galley.size()),
                glyphs: t.visual_bounding_rect(),
                clip,
                marker: t
                    .galley
                    .job
                    .sections
                    .iter()
                    .all(|s| s.format.font_id.size <= 1.5),
            }),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &output.shapes {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

/// The whole desktop with Files (a folder of scripts), Workspace (two
/// variables), the Command Window, the Editor and Figure 1. A compact window
/// shows Workspace and the figure preview in its Inspector sheet.
fn clear_panes_desktop(
    theme: session::DesktopTheme,
    size: egui::Vec2,
) -> (
    egui::Context,
    Workspace,
    tempfile::TempDir,
    egui::FullOutput,
) {
    let (ctx, mut app) = redesign_app(theme);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("data")).unwrap();
    for i in 0..3 {
        std::fs::write(dir.path().join(format!("script_{i}.m")), "x = 1;").unwrap();
    }
    assert!(app.change_folder(dir.path().to_path_buf()));
    finish_folder(&mut app);
    app.run("a = 1; b = magic(3);".into());
    finish(&mut app);
    app.new_document("x = 1:10;\nplot(x, x.^2);\n");
    app.reveal_editor();
    app.run("plot(1:10, (1:10).^2);".into());
    finish(&mut app);
    app.select_document_layout(layout::PaneMode::Split);
    redesign_frames(&mut app, &ctx, size);
    if workspace_dock::inspector::uses_sheet(&app, &ctx) {
        app.navigation.inspector_open = true;
    }
    let output = redesign_frames(&mut app, &ctx, size);
    (ctx, app, dir, output)
}

/// Icon buttons without visible text inside the toolstrip and every pane
/// header: (button name, area) pairs.
fn clear_panes_icon_only(ctx: &egui::Context, output: &egui::FullOutput) -> Vec<(String, String)> {
    let texts = clear_panes_texts(output);
    let mut areas: Vec<(String, egui::Rect)> = ["toolbar", "action_row"]
        .into_iter()
        .filter_map(|name| redesign_panel(ctx, name).map(|r| (name.to_owned(), r)))
        .collect();
    areas.extend(panels::painted_headers(ctx));
    texts
        .iter()
        .filter(|t| t.marker && !t.text.is_empty())
        .filter(|m| {
            !texts.iter().any(|t| {
                !t.marker
                    // Icons are Phosphor glyphs (private-use text): a label
                    // needs a letter or digit.
                    && t.text.chars().any(char::is_alphanumeric)
                    && t.clip.contains(m.rect.center())
                    && t.glyphs.expand(2.).contains(m.rect.center())
            })
        })
        .filter_map(|m| {
            areas
                .iter()
                .find(|(_, area)| area.contains(m.rect.center()))
                .map(|(area, _)| (m.text.clone(), area.clone()))
        })
        .collect()
}

/// Before this step (counted with this test on the old chrome): 12 icon-only
/// buttons at 1366x768 (Files 7, Workspace 2, Editor 2, Figure 1) and 11 at
/// 1024x640 (Files 6, Workspace 2, Editor 2, Figure 1) in both themes; the
/// toolstrip had none at these widths. Header tools also stay whole inside
/// their header: only the title may truncate.
#[test]
fn clear_panes_toolstrip_and_pane_headers_have_no_icon_only_buttons() {
    for theme in REDESIGN_THEMES {
        for size in CLEAR_PANES_SIZES {
            let (ctx, _app, _dir, output) = clear_panes_desktop(theme, size);
            let icons = clear_panes_icon_only(&ctx, &output);
            assert!(icons.is_empty(), "{theme:?} {size:?}: {icons:?}");
            let headers = panels::painted_headers(&ctx);
            let titles: Vec<&str> = headers.iter().map(|(t, _)| t.as_str()).collect();
            for title in ["Files", "Workspace", "Command Window", "Editor", "Figure 1"] {
                assert!(
                    titles.contains(&title),
                    "{theme:?} {size:?}: {title} header in {titles:?}"
                );
            }
            let texts = clear_panes_texts(&output);
            for (title, header) in &headers {
                // The header row of a pane sits inside the window.
                assert!(header.right() <= size.x + 0.5, "{title} header {header:?}");
                let row: Vec<&ClearPanesText> = texts
                    .iter()
                    .filter(|t| t.text.chars().any(char::is_alphanumeric))
                    .filter(|t| header.contains(t.rect.center()))
                    .collect();
                for t in &row {
                    let inside = if t.marker { t.rect } else { t.glyphs };
                    assert!(
                        header.expand(1.).contains_rect(inside)
                            && t.clip.expand(1.).contains_rect(inside),
                        "{theme:?} {size:?}: {:?} leaves the {title} header {header:?}: {inside:?} clip {:?}",
                        t.text,
                        t.clip
                    );
                }
            }
            // The Files tools are words: Up, Browse and More at least.
            let (_, files) = headers.iter().find(|(t, _)| t == "Files").unwrap();
            for word in ["Up", "Browse", "More"] {
                assert!(
                    texts
                        .iter()
                        .any(|t| !t.marker && t.text == word && files.contains(t.rect.center())),
                    "{theme:?} {size:?}: {word} in the Files header"
                );
            }
        }
    }
}

fn clear_panes_contrast(a: egui::Color32, b: egui::Color32) -> f64 {
    // WCAG 2 relative luminance of sRGB colours.
    let luminance = |c: egui::Color32| {
        let channel = |v: u8| {
            let v = f64::from(v) / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn clear_panes_pane_border_has_three_to_one_on_every_surface() {
    assert!((clear_panes_contrast(egui::Color32::BLACK, egui::Color32::WHITE) - 21.).abs() < 1e-9);
    for (name, p) in [("light", &tokens::LIGHT), ("dark", &tokens::DARK)] {
        for (surface, color) in [
            ("canvas", p.canvas),
            ("surface", p.surface),
            ("surface_sunken", p.surface_sunken),
        ] {
            let ratio = clear_panes_contrast(p.pane_border, color);
            assert!(ratio >= 3.0, "{name} pane_border on {surface}: {ratio:.2}");
        }
        // The hairline stays the faint in-pane divider.
        assert!(
            clear_panes_contrast(p.hairline, p.surface) < 3.0,
            "{name} hairline"
        );
    }
}

/// 1 px lines of `color`: (from, to) of every line segment.
fn clear_panes_lines(
    output: &egui::FullOutput,
    color: egui::Color32,
) -> Vec<(egui::Pos2, egui::Pos2)> {
    fn walk(shape: &egui::Shape, color: egui::Color32, out: &mut Vec<(egui::Pos2, egui::Pos2)>) {
        match shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == color && stroke.width >= 1. =>
            {
                out.push((points[0], points[1]));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, color, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &output.shapes {
        walk(&s.shape, color, &mut out);
    }
    out
}

#[test]
fn clear_panes_every_pane_edge_is_a_pane_border_line() {
    for theme in REDESIGN_THEMES {
        let size = egui::vec2(1366., 768.);
        let (ctx, _app, _dir, output) = clear_panes_desktop(theme, size);
        let p = tokens::palette(&ctx);
        let lines = clear_panes_lines(&output, p.pane_border);
        let vertical = |x: f32, name: &str| {
            assert!(
                lines.iter().any(|(a, b)| (a.x - x).abs() <= 1.
                    && (b.x - x).abs() <= 1.
                    && (a.y - b.y).abs() > 100.),
                "{theme:?}: no pane_border at x = {x} ({name}); {lines:?}"
            );
        };
        let horizontal = |y: f32, name: &str| {
            assert!(
                lines.iter().any(|(a, b)| (a.y - y).abs() <= 1.
                    && (b.y - y).abs() <= 1.
                    && (a.x - b.x).abs() > 100.),
                "{theme:?}: no pane_border at y = {y} ({name}); {lines:?}"
            );
        };
        let files = redesign_panel(&ctx, "workspace").unwrap();
        vertical(files.right(), "Files | work area");
        let right = redesign_panel(&ctx, "workspace_variables").unwrap();
        vertical(right.left(), "work area | Workspace");
        let figure = redesign_panel(&ctx, "figure_column").unwrap();
        horizontal(figure.top(), "Workspace / Figure");
        let console = redesign_panel(&ctx, "console").unwrap();
        horizontal(console.top(), "Editor / Command Window");
        let row = redesign_panel(&ctx, "action_row").unwrap();
        horizontal(row.bottom(), "toolstrip / panes");
        let status = redesign_panel(&ctx, "status").unwrap();
        horizontal(status.top(), "panes / status bar");
        // In-pane dividers (header rules) stay hairlines.
        assert!(!clear_panes_lines(&output, p.hairline).is_empty());
    }
}

#[test]
fn clear_panes_every_visible_pane_and_window_paints_its_title() {
    for theme in REDESIGN_THEMES {
        for size in CLEAR_PANES_SIZES {
            let (ctx, mut app, _dir, output) = clear_panes_desktop(theme, size);
            let texts = clear_panes_texts(&output);
            let painted = |texts: &[ClearPanesText], title: &str| {
                texts
                    .iter()
                    .any(|t| !t.marker && t.text == title && t.clip.contains_rect(t.glyphs))
            };
            // Pane headers use the uppercase caption style.
            for title in ["FILES", "WORKSPACE", "COMMAND WINDOW", "EDITOR", "FIGURE 1"] {
                assert!(painted(&texts, title), "{theme:?} {size:?}: {title}");
            }
            if size.x < 1100. {
                // The compact Inspector sheet holds Workspace and the figure.
                assert!(
                    painted(&texts, "INSPECTOR"),
                    "{theme:?} {size:?}: sheet title"
                );
            }
            // The Variable Editor window names itself and its variable.
            app.editing_variable = Some("b".into());
            let output = redesign_frames(&mut app, &ctx, size);
            assert!(
                painted(&clear_panes_texts(&output), "Variable Editor — b"),
                "{theme:?} {size:?}: Variable Editor title"
            );
            app.editing_variable = None;
            // Each app names itself above its work area; a wide app also
            // titles its Inspector pane.
            for (tool, name) in [
                (toolkits::Tool::Designer, "Filter Designer"),
                (toolkits::Tool::Model, "Block Simulation"),
            ] {
                app.open_app(tool);
                let output = redesign_frames(&mut app, &ctx, size);
                let texts = clear_panes_texts(&output);
                assert!(painted(&texts, name), "{theme:?} {size:?}: {name} title");
                // The designer's areas are SPECS | ANALYSIS | OUTPUT.
                let pane = if tool == toolkits::Tool::Designer {
                    "SPECS"
                } else {
                    "INSPECTOR"
                };
                if size.x >= 1100. {
                    assert!(
                        painted(&texts, pane),
                        "{theme:?} {size:?}: {name} {pane} title"
                    );
                }
            }
        }
    }
}

/// An open filename filter keeps the FILES title whole: the field, "Clear"
/// and More fit beside it, and Browse yields only when they would not.
#[test]
fn clear_panes_files_title_stays_whole_with_an_open_filter() {
    for theme in REDESIGN_THEMES {
        for size in CLEAR_PANES_SIZES {
            let (ctx, mut app, _dir, _) = clear_panes_desktop(theme, size);
            app.folder_browser.filter_open = true;
            let output = redesign_frames(&mut app, &ctx, size);
            let texts = clear_panes_texts(&output);
            let (_, files) = panels::painted_headers(&ctx)
                .into_iter()
                .find(|(t, _)| t == "Files")
                .unwrap();
            let title = texts
                .iter()
                .find(|t| !t.marker && t.text == "FILES" && files.contains(t.rect.center()))
                .unwrap_or_else(|| panic!("{theme:?} {size:?}: whole FILES title"));
            assert!(
                title.clip.contains_rect(title.glyphs),
                "{theme:?} {size:?}: FILES cut"
            );
            for name in ["Clear filename filter", "Folder actions"] {
                let tool = texts
                    .iter()
                    .find(|t| t.text == name && files.contains(t.rect.center()))
                    .unwrap_or_else(|| panic!("{theme:?} {size:?}: {name} in the Files header"));
                assert!(
                    tool.rect.left() > title.glyphs.right(),
                    "{name} overlaps the title"
                );
            }
            let field = ctx.read_response(egui::Id::new("folder_filter")).unwrap();
            assert!(
                files.contains_rect(field.rect) && field.rect.left() > title.glyphs.right(),
                "{theme:?} {size:?}: filter field {:?} in {files:?}",
                field.rect
            );
        }
    }
}
