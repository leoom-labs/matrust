// Panel-card redesign (warm light and dark) at the 1366x768 reference and the
// 1024x700 compact size. Real fresh-start fixtures, both themes.
const REDESIGN_SIZES: [egui::Vec2; 2] = [egui::vec2(1366., 768.), egui::vec2(1024., 700.)];
const REDESIGN_THEMES: [session::DesktopTheme; 2] =
    [session::DesktopTheme::Light, session::DesktopTheme::Dark];

fn redesign_app(theme: session::DesktopTheme) -> (egui::Context, Workspace) {
    let ctx = egui::Context::default();
    let mut app = Workspace::with_context(ctx.clone(), true);
    app.layout.theme = Some(theme);
    app.apply_desktop_theme(&ctx);
    (ctx, app)
}

fn redesign_frames(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
    for _ in 0..3 {
        frame_at(app, ctx, size, vec![]);
    }
    frame_at(app, ctx, size, vec![])
}

fn redesign_painted(output: &egui::FullOutput, label: &str) -> bool {
    output
        .shapes
        .iter()
        .any(|s| text_position(&s.shape, label).is_some())
}

/// Text shapes whose whole glyph box is inside their clip rectangle.
fn redesign_visible_texts(output: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) if clip.contains_rect(t.visual_bounding_rect()) => {
                out.push(t.galley.text().to_owned());
            }
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

fn redesign_rect_fills(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
    match shape {
        egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
        egui::Shape::Vec(v) => v.iter().for_each(|s| redesign_rect_fills(s, out)),
        _ => {}
    }
}

fn redesign_panel(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
    egui::containers::panel::PanelState::load(ctx, egui::Id::new(name)).map(|s| s.outer_rect)
}

#[test]
fn redesign_three_columns_with_a_figure_and_two_without() {
    for theme in REDESIGN_THEMES {
        for size in REDESIGN_SIZES {
            let (ctx, mut app) = redesign_app(theme);
            let output = redesign_frames(&mut app, &ctx, size);
            let canvas = tokens::palette(&ctx).canvas;
            let mut fills = Vec::new();
            output
                .shapes
                .iter()
                .for_each(|s| redesign_rect_fills(&s.shape, &mut fills));
            assert!(
                fills
                    .iter()
                    .any(|(r, f)| *f == canvas && r.contains_rect(ctx.content_rect())),
                "the window background is the canvas colour ({theme:?} {size:?})"
            );
            let input = ctx.read_response(egui::Id::new("command_input")).unwrap();
            let left = redesign_panel(&ctx, "workspace").expect("left column");
            assert!(redesign_panel(&ctx, "figure_column").is_none());
            assert!(left.right() <= input.rect.left());
            assert!(
                (input.rect.right() - size.x).abs() < 80.,
                "without a figure the Command Window takes the width: {:?}",
                input.rect
            );
            app.run("x = linspace(0,1,50); plot(x, sin(2*pi*x));".into());
            finish(&mut app);
            redesign_frames(&mut app, &ctx, size);
            let input = ctx.read_response(egui::Id::new("command_input")).unwrap();
            let figure = redesign_panel(&ctx, "figure_column").expect("figure column");
            assert!(left.right() <= input.rect.left() && input.rect.right() <= figure.left());
            assert!(figure.right() <= size.x + 0.5);
            if size.x >= 1366. {
                assert!(
                    (figure.width() - tokens::RIGHT_COLUMN).abs() <= 1.,
                    "{theme:?} figure column {figure:?}"
                );
                assert!(
                    (left.width() - tokens::LEFT_COLUMN).abs() <= 1.,
                    "{theme:?} left column {left:?}"
                );
            }
        }
    }
}

#[test]
fn redesign_workspace_shows_twelve_rows_of_42_variables() {
    let script = (1..=42)
        .map(|i| format!("v{i:02} = (1:{i}) * 0.5;"))
        .collect::<String>();
    for theme in REDESIGN_THEMES {
        let size = egui::vec2(1366., 768.);
        let (ctx, mut app) = redesign_app(theme);
        app.run(script.clone());
        finish(&mut app);
        assert_eq!(app.variables.len(), 42);
        let output = redesign_frames(&mut app, &ctx, size);
        let rows = redesign_visible_texts(&output)
            .into_iter()
            .filter(|t| t.len() == 3 && t.starts_with('v') && t[1..].parse::<u8>().is_ok())
            .count();
        assert!(rows >= 12, "{theme:?}: {rows} workspace rows visible");
        assert!(redesign_painted(&output, "42 variables"));
        // The minimum dock gives NAME and VALUE 96 px each; SIZE yields.
        assert!(!redesign_painted(&output, "1x1"));
        assert!(redesign_painted(&output, "[0.5000 1]"));
    }
}

#[test]
fn redesign_welcome_cards_only_while_history_is_empty() {
    for theme in REDESIGN_THEMES {
        for size in REDESIGN_SIZES {
            let (ctx, mut app) = redesign_app(theme);
            let output = redesign_frames(&mut app, &ctx, size);
            for card in ["Open folder", "Try an example"] {
                assert!(redesign_painted(&output, card), "{card} {theme:?} {size:?}");
            }
            assert!(redesign_painted(&output, "A = [1 2; 3 4]; A * A"));
            click_label_at(&mut app, &ctx, size, "Try an example");
            finish(&mut app);
            assert_eq!(
                app.history.entries.last().map(String::as_str),
                Some(panels::EXAMPLE)
            );
            assert!(app.figure.is_some());
            let output = redesign_frames(&mut app, &ctx, size);
            for card in ["Open folder", "Try an example", "A = [1 2; 3 4]; A * A"] {
                assert!(!redesign_painted(&output, card), "{card} after a run");
            }
        }
    }
}

#[test]
fn redesign_top_bar_run_is_accent_filled_and_focus_ring_is_accent() {
    for theme in REDESIGN_THEMES {
        for size in REDESIGN_SIZES {
            let (ctx, mut app) = redesign_app(theme);
            app.documents[0].text = "answer = 42;".into();
            app.documents[0].dirty = true;
            let output = redesign_frames(&mut app, &ctx, size);
            let p = tokens::palette(&ctx);
            // The top bar owns the sole Run affordance.
            let run = output
                .shapes
                .iter()
                .filter_map(|s| text_position(&s.shape, "Run"))
                .min_by(|a, b| a.y.total_cmp(&b.y))
                .expect("top-bar Run");
            let mut fills = Vec::new();
            output
                .shapes
                .iter()
                .for_each(|s| redesign_rect_fills(&s.shape, &mut fills));
            assert!(
                fills
                    .iter()
                    .any(|(r, f)| *f == p.accent_fill && r.contains(run)),
                "Run sits on accent_fill ({theme:?} {size:?})"
            );
            let visuals = &ctx.global_style().visuals;
            let ring = visuals.widgets.active.bg_stroke;
            assert!(ring.width >= 2. && ring.color == p.accent, "{ring:?}");
            assert!(visuals.selection.stroke.width >= 2.);
            assert_eq!(visuals.window_shadow, egui::Shadow::NONE);
            click_label_at(&mut app, &ctx, size, "Run");
            finish(&mut app);
            assert!(app.variables.contains_key("answer"));
        }
    }
}

#[test]
fn redesign_toolstrip_overflows_into_more_below_1100_px() {
    for theme in REDESIGN_THEMES {
        let (ctx, mut app) = redesign_app(theme);
        let wide = redesign_frames(&mut app, &ctx, egui::vec2(1366., 768.));
        assert!(!redesign_painted(&wide, "More"));
        assert!(redesign_painted(&wide, "Layout"));
        // Nothing to save, plot or run yet: Save stays, the rest is hidden.
        assert!(redesign_painted(&wide, "Save"));
        assert!(!redesign_painted(&wide, "Plot"));
        let narrow = redesign_frames(&mut app, &ctx, egui::vec2(1024., 700.));
        assert!(redesign_painted(&narrow, "More"));
        assert!(!redesign_painted(&narrow, "Layout"));
        click_label_at(&mut app, &ctx, egui::vec2(1024., 700.), "More");
        let menu = redesign_frames(&mut app, &ctx, egui::vec2(1024., 700.));
        assert!(redesign_painted(&menu, "Layout"));
    }
}

#[test]
fn redesign_status_bar_reports_ready_on_the_canvas() {
    for theme in REDESIGN_THEMES {
        let (ctx, mut app) = redesign_app(theme);
        let output = redesign_frames(&mut app, &ctx, egui::vec2(1366., 768.));
        for label in ["Ready", "UTF-8", "No variables"] {
            assert!(redesign_painted(&output, label), "{label}");
        }
        let status = redesign_panel(&ctx, "status").unwrap();
        assert!((status.height() - tokens::STATUS_BAR).abs() <= 1.);
    }
}

// Note 1: the mockup has a top bar, one toolstrip card, three columns of cards
// and a status bar. No menu row, rails, tab row or bottom toggle.
#[test]
fn redesign_chrome_has_no_menu_row_rails_or_document_tab_row() {
    for theme in REDESIGN_THEMES {
        for size in REDESIGN_SIZES {
            let (ctx, mut app) = redesign_app(theme);
            app.run("x = 1:3; plot(x, x);".into());
            finish(&mut app);
            let output = redesign_frames(&mut app, &ctx, size);
            for label in [
                "Menu",
                "Help",
                "Files",
                "Workspace",
                "Command Window",
                "Figure 1",
            ] {
                assert!(
                    redesign_painted(&output, label),
                    "{label} {theme:?} {size:?}"
                );
            }
            // Old second row, rail captions and the Command | Figure tab row.
            for gone in [
                "Current Folder",
                "Hide tools",
                "Show tools",
                "Command",
                "Figure",
                "Editor",
            ] {
                assert!(
                    !redesign_painted(&output, gone),
                    "{gone} {theme:?} {size:?}"
                );
            }
            let command_windows = redesign_visible_texts(&output)
                .iter()
                .filter(|t| *t == "Command Window")
                .count();
            assert_eq!(command_windows, 1, "no bottom Command Window toggle");
            assert!(
                ctx.read_response(egui::Id::new(("desktop_rail", "files")))
                    .is_none()
            );
            assert!(redesign_panel(&ctx, "desktop_left_rail").is_none());
            assert!(redesign_panel(&ctx, "desktop_bottom_rail").is_none());
            assert!(redesign_panel(&ctx, "desktop_center_tabs").is_none());
        }
    }
}

#[test]
fn redesign_files_card_shows_eight_rows_with_two_variables() {
    let size = egui::vec2(1366., 768.);
    for theme in REDESIGN_THEMES {
        let (ctx, mut app) = redesign_app(theme);
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("file_{i:02}.m")), "x=1;").unwrap();
        }
        assert!(app.change_folder(dir.path().to_path_buf()));
        finish_folder(&mut app);
        app.run("x = 1; y = 2;".into());
        finish(&mut app);
        finish_folder(&mut app);
        let output = redesign_frames(&mut app, &ctx, size);
        let rows = redesign_visible_texts(&output)
            .into_iter()
            .filter(|t| t.starts_with("file_") && t.ends_with(".m"))
            .count();
        assert!(rows >= 8, "{theme:?}: {rows} file rows");
        // The Workspace card is as tall as its rows: header, column row, 3 rows.
        let card = redesign_panel(&ctx, "workspace_variables_left").unwrap();
        assert!(card.height() < 190., "{card:?}");
    }
}

#[test]
fn redesign_figure_card_is_one_header_with_export_and_an_overflow_menu() {
    let size = egui::vec2(1366., 768.);
    for theme in REDESIGN_THEMES {
        let (ctx, mut app) = redesign_app(theme);
        app.run("figure(1); plot([1 2 3]); title('One');".into());
        finish(&mut app);
        let output = redesign_frames(&mut app, &ctx, size);
        for label in ["Figure 1", "Export", "Actions"] {
            assert!(redesign_painted(&output, label), "{label}");
        }
        for gone in [
            "Single",
            "2-up",
            "4-up",
            "Reset view",
            "Save current figure",
            "All figures",
        ] {
            assert!(
                !redesign_painted(&output, gone),
                "{gone} in a one-figure header"
            );
        }
        // With more than one figure a compact picker joins the header.
        app.run("figure(2); plot([3 2 1]);".into());
        finish(&mut app);
        let output = redesign_frames(&mut app, &ctx, size);
        assert!(redesign_painted(&output, "Figure 2"));
        assert!(redesign_painted(&output, "All figures"));
        // The overflow holds the view modes, edit history, reset and save.
        click_label_at(&mut app, &ctx, size, "Actions");
        let menu = redesign_frames(&mut app, &ctx, size);
        for label in [
            "Single",
            "2-up",
            "4-up",
            "Expand",
            "Undo edit",
            "Redo edit",
            "Reset view",
            "View options",
            "Save figure",
        ] {
            assert!(
                redesign_painted(&menu, label),
                "{label} in the overflow menu"
            );
        }
    }
}

#[test]
fn block_simulation_diagram_takes_sixty_percent_of_the_tab_at_1024x700() {
    let size = egui::vec2(1024., 700.);
    for theme in REDESIGN_THEMES {
        let (ctx, mut app) = redesign_app(theme);
        app.show_model_document();
        for _ in 0..6 {
            frame_at(&mut app, &ctx, size, vec![]);
        }
        let (tab, _) = app.toolkits.model_document_region().expect("model tab");
        // The diagram canvas lies between the guidance line and the status line.
        let output = frame_at(&mut app, &ctx, size, vec![]);
        let line = |text: &str| {
            let mut found = None;
            for s in &output.shapes {
                fn walk(shape: &egui::Shape, text: &str, found: &mut Option<egui::Rect>) {
                    match shape {
                        egui::Shape::Text(t) if t.galley.text().starts_with(text) => {
                            *found = Some(t.visual_bounding_rect());
                        }
                        egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, found)),
                        _ => {}
                    }
                }
                walk(&s.shape, text, &mut found);
            }
            found.unwrap_or_else(|| panic!("Missing {text}"))
        };
        let top = line("Select a block to edit").bottom();
        let bottom = line("Model valid").top();
        let share = (bottom - top) / tab.height();
        assert!(
            share >= 0.6,
            "{theme:?}: diagram {top}..{bottom} of tab {tab:?} = {share:.3}"
        );
        assert!(
            tab.bottom() <= bottom + 40.,
            "the status line stays inside the tab"
        );
    }
}

#[test]
fn workspace_value_preview_shows_a_matrix_as_size_and_class() {
    let size = egui::vec2(1366., 768.);
    let (ctx, mut app) = redesign_app(session::DesktopTheme::Light);
    app.run("A = [1 2; 3 4];".into());
    finish(&mut app);
    let output = redesign_frames(&mut app, &ctx, size);
    assert!(redesign_painted(&output, "2x2 double"));
    assert!(!redesign_painted(&output, "[1 3 ...]"));
}

#[test]
fn welcome_cards_stay_hidden_while_a_tool_tab_is_selected() {
    for size in REDESIGN_SIZES {
        let (ctx, mut app) = redesign_app(session::DesktopTheme::Light);
        let output = redesign_frames(&mut app, &ctx, size);
        assert!(redesign_painted(&output, "Try an example"));
        app.show_model_document();
        let output = redesign_frames(&mut app, &ctx, size);
        for card in ["New script", "Open folder", "Try an example"] {
            // "New script" is also a toolstrip tile at 1366; the cards' detail text is unique.
            let detail = match card {
                "New script" => "Ctrl N",
                "Open folder" => "Your .m files and data",
                _ => "plot(x, sin(x))",
            };
            assert!(
                !redesign_painted(&output, detail),
                "{card} card under a tool tab ({size:?})"
            );
        }
        assert!(redesign_painted(&output, "Command Window"));
    }
}

#[test]
fn image_preview_without_a_figure_keeps_the_right_column_hidden() {
    for theme in REDESIGN_THEMES {
        for size in REDESIGN_SIZES {
            let (ctx, mut app) = redesign_app(theme);
            app.pending_image = Some(image_load::Preview {
                original: None,
                path: std::env::temp_dir().join("redesign-preview.png"),
                original_size: [64, 64],
                format: "PNG",
                file_bytes: 123,
                pixels: egui::ColorImage::filled([64, 64], egui::Color32::RED),
            });
            let output = redesign_frames(&mut app, &ctx, size);
            assert!(app.image_view.is_some() && app.figure.is_none());
            assert!(!app.figure_column(), "{theme:?} {size:?}");
            assert!(redesign_panel(&ctx, "figure_column").is_none());
            assert!(
                !redesign_painted(&output, chrome::NO_FIGURE),
                "no empty figure card beside a preview ({theme:?} {size:?})"
            );
            let input = ctx.read_response(egui::Id::new("command_input")).unwrap();
            assert!(
                (input.rect.right() - size.x).abs() < 80.,
                "the Command Window keeps the width: {:?}",
                input.rect
            );
        }
    }
}

#[test]
fn redesign_files_header_holds_the_folder_navigation_icons() {
    // (text, glyph box, clip) of every text shape, invisible test labels included.
    fn all_texts(output: &egui::FullOutput) -> Vec<(String, egui::Rect, egui::Rect)> {
        fn walk(
            shape: &egui::Shape,
            clip: egui::Rect,
            out: &mut Vec<(String, egui::Rect, egui::Rect)>,
        ) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_owned(), t.visual_bounding_rect(), clip))
                }
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
    let icons = [
        "Back",
        "Forward",
        "Up",
        "Refresh",
        "Browse",
        "Search text",
        "Filter filenames",
    ];
    for theme in REDESIGN_THEMES {
        for size in REDESIGN_SIZES {
            let (ctx, mut app) = redesign_app(theme);
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir(dir.path().join("sub")).unwrap();
            for i in 0..5 {
                std::fs::write(dir.path().join(format!("file_{i}.m")), "x=1;").unwrap();
            }
            assert!(app.change_folder(dir.path().to_path_buf()));
            finish_folder(&mut app);
            let output = redesign_frames(&mut app, &ctx, size);
            let texts = all_texts(&output);
            let left = redesign_panel(&ctx, "workspace").unwrap();
            let find = |label: &str| {
                texts
                    .iter()
                    .find(|(t, rect, _)| t == label && left.contains_rect(*rect))
                    .unwrap_or_else(|| panic!("{label} in the left column ({theme:?} {size:?})"))
            };
            // The title is whole: painted untruncated and inside its clip.
            let (_, title, clip) = find("Files");
            assert!(
                clip.contains_rect(*title),
                "Files title is cut ({theme:?} {size:?})"
            );
            let header = panels::header_height(&ctx);
            for icon in icons {
                let (_, rect, _) = find(icon);
                assert!(
                    (rect.center().y - title.center().y).abs() < header / 2.
                        && rect.left() > title.right(),
                    "{icon} sits in the Files header row ({theme:?} {size:?}): {rect:?} vs {title:?}"
                );
            }
            // No second row: the first file row follows the header directly.
            let first = texts
                .iter()
                .filter(|(t, rect, _)| {
                    (t == "sub/" || t.starts_with("file_")) && left.contains_rect(*rect)
                })
                .map(|(_, rect, _)| rect.center().y)
                .fold(f32::INFINITY, f32::min);
            assert!(
                first - title.center().y < header + tokens::XS,
                "first file row at {first}, title at {} ({theme:?} {size:?})",
                title.center().y
            );
            // Ctrl+L: the address field takes the tools' place in the header.
            frame_at(
                &mut app,
                &ctx,
                size,
                vec![egui::Event::Key {
                    key: egui::Key::L,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                }],
            );
            let output = redesign_frames(&mut app, &ctx, size);
            let field = ctx.read_response(folder_navigation::address_id()).unwrap();
            assert!(ctx.memory(|m| m.has_focus(folder_navigation::address_id())));
            let texts_open = all_texts(&output);
            let title_open = texts_open
                .iter()
                .find(|(t, rect, _)| t == "Files" && left.contains_rect(*rect))
                .unwrap()
                .1;
            assert!((field.rect.center().y - title_open.center().y).abs() < header / 2.);
            assert!(field.rect.left() > title_open.right());
            assert!(!texts_open.iter().any(|(t, _, _)| t == "Back"));
        }
    }
    // A column dragged to its minimum keeps the title whole and Up.
    let (ctx, mut app) = redesign_app(session::DesktopTheme::Light);
    // Files alone can still use the narrower minimum; a shared Workspace
    // column reserves space for NAME and VALUE.
    app.layout.workspace_dock = None;
    app.layout.sidebar_width = Some(170.);
    let output = redesign_frames(&mut app, &ctx, egui::vec2(1366., 768.));
    let texts = all_texts(&output);
    let left = redesign_panel(&ctx, "workspace").unwrap();
    assert!(left.width() < 180., "{left:?}");
    let (_, title, clip) = texts
        .iter()
        .find(|(t, rect, _)| t == "Files" && left.contains_rect(*rect))
        .expect("whole Files title in a narrow column");
    assert!(clip.contains_rect(*title));
    for icon in ["Up", "Search text", "Filter filenames"] {
        assert!(
            texts
                .iter()
                .any(|(t, rect, _)| t == icon && left.contains_rect(*rect)),
            "{icon}"
        );
    }
}

#[test]
fn redesign_round3_narrow_chrome_keeps_tabs_and_actions_visible() {
    for theme in REDESIGN_THEMES {
        for width in [800., 899., 900., 1024., 1099., 1100., 1366.] {
            let size = egui::vec2(width, 600.);
            let (ctx, mut app) = redesign_app(theme);
            app.folder = PathBuf::from("/parent/signals");
            app.new_document("a = 1;");
            app.documents[app.active].path = Some(PathBuf::from("signal.m"));
            let output = redesign_frames(&mut app, &ctx, size);
            let texts = redesign_visible_texts(&output);
            for tab in ["Home", "Plots", "Apps"] {
                assert!(texts.iter().any(|s| s == tab), "{tab} {width}");
                assert_eq!(painted_font(&output, tab).size, tokens::BODY);
            }
            assert_eq!(texts.iter().filter(|s| *s == "Run").count(), 1);
            for action in ["New script", "Open", "Editor", "signal.m *", "Find / replace", "Editor actions"] {
                assert!(texts.iter().any(|s| s == action), "{action}: {width} {texts:?}");
            }
            if width < 1100. {
                assert!(texts.iter().any(|s| s == "More"), "{width}");
            }
            let point = |text: &str| output.shapes.iter().find_map(|s| text_position(&s.shape, text)).unwrap();
            for action in ["Find / replace", "Editor actions"] {
                assert!((point("Editor").y - point(action).y).abs() < tokens::S, "header wraps at {width}");
            }
            for gone in ["Side by side", "Undock Editor", "Restore split"] {
                assert!(!redesign_painted(&output, gone));
            }
            if width < 900. {
                assert!(texts.iter().any(|s| s == "Search"));
                assert!(!redesign_painted(&output, "Search functions and actions"));
                // The status bar still has a folder label; the crumb must be absent from the top bar.
                for shape in &output.shapes {
                    for path in ["signals", "/parent/signals"] {
                        if let Some(p) = text_position(&shape.shape, path) {
                            assert!(p.y > tokens::TOP_BAR, "crumb at {width}: {p:?}");
                        }
                    }
                }
                click_label_at(&mut app, &ctx, size, "Search");
                assert!(app.global_search.is_open());
            } else {
                let crumb = if width < 1100. { "signals" } else { "/parent/signals" };
                assert!(point(crumb).y < tokens::TOP_BAR);
            }
        }
    }
}

#[test]
fn redesign_round3_editor_menu_and_find_remain_functional() {
    for theme in REDESIGN_THEMES {
        for size in [egui::vec2(800., 600.), egui::vec2(1024., 700.)] {
            let (ctx, mut app) = redesign_app(theme);
            app.new_document("first = 1;");
            app.new_document("second = 2;");
            redesign_frames(&mut app, &ctx, size);
            click_label_at(&mut app, &ctx, size, "Editor actions");
            let output = redesign_frames(&mut app, &ctx, size);
            for label in ["Side by side", "Undock Editor", "Restore split"] {
                assert!(redesign_painted(&output, label), "{label}");
            }
            click_label_at(&mut app, &ctx, size, "Side by side");
            assert!(app.editor_columns.other.is_some());
            click_label_at(&mut app, &ctx, size, "Find / replace");
            let output = redesign_frames(&mut app, &ctx, size);
            assert!(redesign_painted(&output, "Replace"));
        }
    }
}

#[test]
fn redesign_round3_figure_overflow_has_only_view_edit_and_save_actions() {
    let size = egui::vec2(1366., 768.);
    for theme in REDESIGN_THEMES {
        let (ctx, mut app) = redesign_app(theme);
        app.run("plot([1 2]);".into());
        finish(&mut app);
        redesign_frames(&mut app, &ctx, size);
        click_label_at(&mut app, &ctx, size, "Actions");
        let output = redesign_frames(&mut app, &ctx, size);
        for gone in ["New", "Open…", "Open separate disk copy...", "Close", "Save as…", "Save all figures", "Close all figures", "New layout", "Reload from disk...", "Properties…", "Export…", "Undock figure", "Ctrl+click a plot to inspect its nearest visible original sample."] {
            assert!(!redesign_painted(&output, gone), "{gone}");
        }
        egui::Popup::close_all(&ctx);
        click_label_at(&mut app, &ctx, size, "Menu");
        click_label_at(&mut app, &ctx, size, "File");
        click_label_at(&mut app, &ctx, size, "Figures");
        let output = redesign_frames(&mut app, &ctx, size);
        for label in ["New", "Open…", "Open separate disk copy...", "Close", "Save as…", "Save all figures", "Close all figures", "New layout", "Reload from disk...", "Properties…"] {
            assert!(redesign_painted(&output, label), "{label}");
        }
    }
}

#[test]
fn redesign_round3_workspace_preview_uses_each_variables_data() {
    let size = egui::vec2(1366., 768.);
    let (ctx, mut app) = redesign_app(session::DesktopTheme::Light);
    app.run("a = [1 2]; b = [5 6]; c = [9; 10; 11];".into());
    finish(&mut app);
    let output = redesign_frames(&mut app, &ctx, size);
    for (name, preview) in [("a", "[1 2]"), ("b", "[5 6]"), ("c", "[9;10;11]")] {
        let point = |text| output.shapes.iter().find_map(|s| text_position(&s.shape, text)).unwrap();
        assert!((point(name).y - point(preview).y).abs() < 1., "{name}");
    }
}

#[test]
fn redesign_round3_plot_inspection_hint_appears_only_on_hover() {
    let size = egui::vec2(1366., 768.);
    let (ctx, mut app) = redesign_app(session::DesktopTheme::Light);
    app.run("plot([1 3 2]);".into());
    finish(&mut app);
    let hint = layout::PLOT_INSPECTION_HINT;
    let output = redesign_frames(&mut app, &ctx, size);
    assert!(!redesign_painted(&output, hint));
    let plot = egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).unwrap();
    let point = plot.transform().frame().center();
    frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(point)]);
    let mut seen = false;
    for _ in 0..90 {
        let output = frame_at(&mut app, &ctx, size, vec![]);
        seen |= redesign_painted(&output, hint);
    }
    assert!(seen, "inspection help must be reachable by hovering the plot");
}

#[test]
fn redesign_plot_inspection_hint_and_crosshair_leave_with_the_pointer() {
    let size = egui::vec2(1366., 768.);
    for theme in REDESIGN_THEMES {
        let (ctx, mut app) = redesign_app(theme);
        app.run("plot([1 3 2]);".into());
        finish(&mut app);
        redesign_frames(&mut app, &ctx, size);
        let plot = egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).unwrap();
        let frame = *plot.transform().frame();
        let coordinates = |output: &egui::FullOutput| {
            fn walk(shape: &egui::Shape) -> bool {
                match shape {
                    // egui_plot's crosshair label from `label_formatter`.
                    egui::Shape::Text(t) => {
                        t.galley.text().contains("\nx = ") && t.galley.text().contains("\ny = ")
                    }
                    egui::Shape::Vec(v) => v.iter().any(walk),
                    _ => false,
                }
            }
            output.shapes.iter().any(|s| walk(&s.shape))
        };
        frame_at(&mut app, &ctx, size, vec![egui::Event::PointerMoved(frame.center())]);
        let shown = (0..90).find_map(|_| {
            let output = frame_at(&mut app, &ctx, size, vec![]);
            output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, layout::PLOT_INSPECTION_HINT))
                .map(|hint| (hint, coordinates(&output)))
        });
        let (hint, crosshair) = shown.expect("hint must show while resting over the axes");
        assert!(crosshair, "the crosshair label must show over the axes");
        // Caption stays inside the axes, never over the figure header.
        assert!(frame.y_range().contains(hint.y), "{hint:?} outside {frame:?}");
        // Leave the axes upwards, over the figure title and card header.
        let outside = egui::pos2(frame.center().x, frame.top() - 12.);
        // Gone at once, and it stays gone while the pointer rests there.
        let mut events = vec![egui::Event::PointerMoved(outside)];
        for _ in 0..90 {
            let output = frame_at(&mut app, &ctx, size, std::mem::take(&mut events));
            assert!(!redesign_painted(&output, layout::PLOT_INSPECTION_HINT));
            assert!(!coordinates(&output), "crosshair label left behind");
            assert_ne!(output.platform_output.cursor_icon, egui::CursorIcon::Crosshair);
        }
    }
}

#[test]
fn redesign_plot_caption_avoids_coordinate_popup_and_moves_to_the_right() {
    for size in [egui::vec2(800., 600.), egui::vec2(1366., 768.)] {
        for theme in REDESIGN_THEMES {
            let (ctx, mut app) = redesign_app(theme);
            app.run("plot([1 3 2]);".into());
            finish(&mut app);
            redesign_frames(&mut app, &ctx, size);
            let plot = egui_plot::PlotMemory::load(&ctx, egui::Id::new("workspace_figure")).unwrap();
            let frame = *plot.transform().frame();
            for near_caption in [false, true] {
                let pointer = if near_caption {
                    frame.left_bottom() + egui::vec2(12., -12.)
                } else {
                    frame.center()
                };
                let mut events = vec![egui::Event::PointerMoved(pointer)];
                let mut coordinates_seen = false;
                for _ in 0..3 {
                    let mut hovered = None;
                    let output = crate::headless::run(&ctx, egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events: std::mem::take(&mut events), ..Default::default()
                    }, |root| {
                        app.ui(root);
                        hovered = figure_view::hovered_plot(&ctx);
                    });
                    let texts = value_column_texts(&output);
                    let (_, caption, clip) = texts.iter().find(|t| t.0 == layout::PLOT_INSPECTION_HINT)
                        .expect("caption must be painted inside hovered axes");
                    let visible_caption = caption.intersect(*clip);
                    assert!(frame.contains_rect(visible_caption), "{visible_caption:?} outside {frame:?}");
                    // egui may use the first hover pass only to measure the
                    // coordinate popup. Check every pass where it paints.
                    if let Some((_, coordinates, _)) = texts.iter().find(|t| t.0.contains("\nx = ") && t.0.contains("\ny = ")) {
                        coordinates_seen = true;
                        assert!(!visible_caption.intersects(*coordinates), "caption {visible_caption:?} overlaps {coordinates:?}");
                        let popup = hovered.unwrap().coordinates.expect("coordinate popup bounds");
                        assert!(!visible_caption.intersects(popup), "caption {visible_caption:?} overlaps popup {popup:?}");
                    }
                    if near_caption {
                        assert!((visible_caption.right() - (frame.right() - 8.)).abs() < 3., "caption did not move right: {visible_caption:?}");
                    } else {
                        assert!((visible_caption.left() - (frame.left() + 8.)).abs() < 3.);
                    }
                }
                assert!(coordinates_seen, "egui_plot coordinates must remain visible at {size:?}, pointer={pointer:?}");
            }
        }
    }
}

#[test]
fn redesign_command_card_keeps_its_height_and_a_block_grows_the_input_upward() {
    for theme in REDESIGN_THEMES {
        for size in REDESIGN_SIZES {
            // An open script docks the Command Window card below the Editor.
            let ctx = egui::Context::default();
            let mut app = legacy_sample_desktop(ctx.clone(), true);
            app.layout.theme = Some(theme);
            app.apply_desktop_theme(&ctx);
            redesign_frames(&mut app, &ctx, size);
            let card = redesign_panel(&ctx, "console").unwrap();
            let line = ctx.read_response(egui::Id::new("command_input")).unwrap().rect;
            // Repainting never grows the card: the input stays inside it.
            redesign_frames(&mut app, &ctx, size);
            assert_eq!(redesign_panel(&ctx, "console").unwrap(), card, "{size:?}");
            assert!(card.contains_rect(line), "{line:?} outside {card:?}");
            app.command = "if 1\n  x = 1;\nend".into();
            let output = redesign_frames(&mut app, &ctx, size);
            assert_eq!(redesign_panel(&ctx, "console").unwrap(), card, "{size:?}");
            let block = ctx.read_response(egui::Id::new("command_input")).unwrap();
            assert!(card.contains_rect(block.interact_rect), "{size:?}");
            if size != REDESIGN_SIZES[0] {
                // The compact card's minimum height scrolls a long block in the field.
                continue;
            }
            // All three rows show without scrolling; the field grew upward.
            assert!(block.rect.height() > line.height() * 2.5, "{:?}", block.rect);
            assert!(block.interact_rect.height() > block.rect.height() - 0.5);
            assert!((block.rect.bottom() - line.bottom()).abs() < 1.);
            // `>>` stays beside the first row.
            let prompt = output
                .shapes
                .iter()
                .rev()
                .find_map(|s| text_position(&s.shape, ">>"))
                .unwrap();
            let first_row = block.rect.top() + (line.center().y - line.top());
            assert!(
                (prompt.y - first_row).abs() < 2.,
                "prompt {prompt:?}, block {:?}",
                block.rect
            );
        }
    }
}

#[test]
fn small_window_with_editor_and_figure_keeps_six_console_lines_and_marks_the_shown_layout() {
    let size = egui::vec2(1024., 640.);
    let (ctx, mut app) = redesign_app(session::DesktopTheme::Light);
    app.new_document("x = 1:10;\nplot(x, x.^2);\n");
    app.reveal_editor();
    app.run("plot(1:10, (1:10).^2);".into());
    finish(&mut app);
    app.select_document_layout(layout::PaneMode::Split);
    redesign_frames(&mut app, &ctx, size);
    assert!(app.figure.is_some() && !app.central_command(), "editor and figure are open");
    let console = egui::containers::panel::PanelState::load(&ctx, egui::Id::new("console"))
        .unwrap()
        .outer_rect;
    assert!(console.height() + 0.5 >= console_six_lines(&ctx), "{console:?}");
    let lines = (console.height() - console_six_lines(&ctx)) / (tokens::CONSOLE * tokens::CONSOLE_LINE);
    assert!(lines > -0.05, "fewer than six output lines: {console:?}");
    // The Layout menu marks the panes actually shown: editor and figure.
    assert_eq!(app.current_layout_preset(), Some(Some(layout::PaneMode::Split)));
    // Editor mode with a figure open shows the editor and the figure column.
    app.select_document_layout(layout::PaneMode::Editor);
    assert!(app.figure_column());
    assert_eq!(app.current_layout_preset(), Some(Some(layout::PaneMode::Split)));
    // A narrow Split shows one pane at a time.
    app.select_document_layout(layout::PaneMode::Split);
    app.document_save.set_compact(true);
    app.compact_figure = false;
    assert_eq!(app.current_layout_preset(), Some(Some(layout::PaneMode::Editor)));
    app.compact_figure = true;
    assert_eq!(app.current_layout_preset(), Some(Some(layout::PaneMode::Figure)));
    app.document_save.set_compact(false);
    app.show_command_center();
    app.layout.center.as_mut().unwrap().editor_revealed = false;
    app.figure = None;
    redesign_frames(&mut app, &ctx, size);
    assert_eq!(app.current_layout_preset(), Some(None));
    app.show_model_document();
    assert_eq!(app.current_layout_preset(), None, "a tool tab matches no preset");
}
