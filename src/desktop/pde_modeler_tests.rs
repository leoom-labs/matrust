// PDE Modeler desktop tests (8c1): sample buttons, model validation, headless
// Host solve with bitwise NodalSolution match, naming rules, and launch hooks.
mod pde_modeler_tests {
    use super::*;
    use apps::{App, AppKind, Host, Snapshot, pde_modeler::PdeModeler};
    use matrust::engine::{
        pde_app_model::{BoundaryCondition, GeometricOrder, PdeAppModel, PdeType, Shape},
        pde_modeler_app_hooks::{self, Launch},
    };
    use std::sync::{Arc, Mutex};

    fn frame_sized(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        let mut action = None;
        let output = crate::headless::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| apps::pde_modeler::show(h, ui, ws, false, &mut action),
        );
        (output, action)
    }

    fn frame(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        frame_sized(h, ws, ctx, egui::vec2(1400., 900.), events)
    }

    fn frame_tall(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<toolkits::Action>) {
        frame_sized(h, ws, ctx, egui::vec2(1400., 2200.), events)
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        name: &str,
    ) -> Option<toolkits::Action> {
        let rect = *h
            .app
            .controls
            .get(name)
            .unwrap_or_else(|| panic!("no control {name}"));
        let pos = rect.center();
        frame(
            h,
            ws,
            ctx,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        let action = frame(h, ws, ctx, vec![pointer(pos, false)]).1;
        frame(h, ws, ctx, vec![]);
        action
    }

    fn wait(h: &mut Host<PdeModeler>, ws: &Snapshot, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            frame(h, ws, ctx, vec![]);
            if !h.running() && h.app.nodal_solution.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "PDE Modeler timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        frame(h, ws, ctx, vec![]);
    }

    #[test]
    fn pde_modeler_sample_buttons_fill_model_and_solve_bitwise() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());

        // Initial empty state reports no shape drawn.
        frame(&mut h, &ws, &ctx, vec![]);
        assert!(h.app.model.shapes.is_empty());
        assert_eq!(h.status().1, toolkits::AppStatus::Error);
        assert!(h.status().0.to_lowercase().contains("no shape"));

        // Click "Plate with a hole" sample button -> fills elliptic model & solves.
        click(&mut h, &ws, &ctx, "Plate with a hole");
        assert_eq!(h.app.model.shapes.len(), 2);
        assert_eq!(h.app.model.set_formula, "R1-C1");
        assert_eq!(h.app.model.pde_type, PdeType::Elliptic);
        assert_eq!(h.app.model.boundary_conditions.len(), 8);

        wait(&mut h, &ws, &ctx);
        assert_eq!(h.status().1, toolkits::AppStatus::Success);
        let received = h
            .app
            .nodal_solution
            .clone()
            .expect("host should receive NodalSolution");

        // Run the exact same script in a fresh Engine and compare bitwise with isequal.
        let script = h.app.script(&ws).unwrap();
        let mut fresh = matrust::Engine::default();
        fresh.execute(&script).unwrap();
        fresh.execute("fresh_u = results.NodalSolution;").unwrap();
        let fresh_u = fresh.array("fresh_u").unwrap();

        let mut cmp = matrust::Engine::default();
        cmp.insert_array("u_app", received).unwrap();
        cmp.insert_array("u_fresh", fresh_u).unwrap();
        cmp.execute("eq = isequal(u_app, u_fresh);").unwrap();
        let matrust::Array::Logical(flag) = cmp.array("eq").unwrap() else {
            panic!("expected logical from isequal");
        };
        assert_eq!(
            flag.data()[0],
            1,
            "received NodalSolution must match fresh Engine bitwise"
        );

        // Exported names and results list model/results.
        assert_eq!(h.app.exported(&ws), vec!["model", "results"]);
        let res_labels: Vec<_> = h.app.results().into_iter().map(|(k, _)| k).collect();
        assert!(res_labels.contains(&"Model"));
        assert!(res_labels.contains(&"Results"));

        // Click "Heat in a plate" sample button -> fills parabolic model.
        click(&mut h, &ws, &ctx, "Heat in a plate");
        assert_eq!(h.app.model.pde_type, PdeType::Parabolic);
        assert_eq!(h.app.model.set_formula, "R1-C1");
        assert_eq!(h.app.model.time_range.steps, 21);
        assert_eq!(h.app.model.coeffs.d, "1");
    }

    #[test]
    fn pde_modeler_validate_messages() {
        // 1. No shape drawn.
        let empty = PdeAppModel::default();
        let err = empty.validate().unwrap_err();
        assert!(err.to_lowercase().contains("no shape"), "{err}");

        // 2. Bad set formula ('R1+X9') gives one clear line naming X9.
        let mut bad_sf = PdeAppModel::plate_with_hole();
        bad_sf.set_formula = "R1+X9".into();
        let err = bad_sf.validate().unwrap_err();
        assert!(!err.contains('\n'), "expected one line: {err}");
        assert!(err.contains("X9"), "should name bad token X9: {err}");

        // 3. Missing boundary condition lists the missing edge IDs.
        let mut missing_bc = PdeAppModel::plate_with_hole();
        missing_bc.boundary_conditions.remove(&3);
        missing_bc.boundary_conditions.remove(&7);
        let err = missing_bc.validate().unwrap_err();
        assert!(err.contains('3') && err.contains('7'), "{err}");
    }

    #[test]
    fn pde_modeler_unique_and_default_name_rules() {
        let mut m = PdeAppModel::default();
        assert_eq!(m.add_rectangle(0.0, 0.0, 1.0, 1.0), "R1");
        assert_eq!(m.add_rectangle(1.0, 0.0, 1.0, 1.0), "R2");
        assert_eq!(m.add_circle(0.5, 0.5, 0.2), "C1");
        assert_eq!(m.add_ellipse(0.0, 0.0, 1.0, 0.5, 0.0), "E1");
        assert_eq!(m.add_polygon(vec![[0., 0.], [1., 0.], [0., 1.]]), "P1");
        assert_eq!(m.set_formula, "R1+R2+C1+E1+P1");

        // Renaming R1 to Plate frees R1 as the next default rectangle name.
        m.rename_shape(0, "Plate").unwrap();
        assert_eq!(m.add_rectangle(2.0, 0.0, 1.0, 1.0), "R1");

        // Reject invalid identifiers and duplicates.
        assert!(m.rename_shape(0, "1bad").is_err());
        assert!(m.rename_shape(0, "bad name").is_err());
        assert!(m.rename_shape(0, "end").is_err());
        assert!(m.rename_shape(0, "").is_err());
        assert!(m.rename_shape(0, "R2").is_err());
    }

    #[test]
    fn pde_modeler_launch_hook_and_registration() {
        let mut e = matrust::Engine::default();
        let err = e.execute("pdeModeler").unwrap_err();
        assert!(err.contains("pdeModeler requires the desktop"), "{err}");

        let slot = Arc::new(Mutex::new(None));
        let dest = slot.clone();
        let _hook = pde_modeler_app_hooks::present(move |launch| {
            *dest.lock().unwrap() = Some(launch);
            Ok(())
        });
        e.execute("pdeModeler").unwrap();
        assert_eq!(slot.lock().unwrap().take(), Some(Launch::Open));
        e.execute("pdeModeler close").unwrap();
        assert_eq!(slot.lock().unwrap().take(), Some(Launch::Close));
        assert!(e.execute("x = pdeModeler").is_err());
        assert!(e.execute("pdeModeler('bogus')").is_err());

        // Desktop worker opens and closes AppKind::PdeModeler.
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx, true);
        app.run("pdeModeler".into());
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.busy {
            app.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.toolkits.apps.is_open(AppKind::PdeModeler));

        app.run("pdeModeler close".into());
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.busy {
            app.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.toolkits.apps.is_open(AppKind::PdeModeler));

        assert!(AppKind::ALL.contains(&AppKind::PdeModeler));
        assert!(AppKind::PdeModeler.is_built());
        assert_eq!(
            AppKind::PdeModeler.tab(),
            matrust::session::ToolTab::PdeModeler
        );
        assert_eq!(
            AppKind::from_tab(matrust::session::ToolTab::PdeModeler),
            Some(AppKind::PdeModeler)
        );
    }

    fn click_tall(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        name: &str,
    ) -> Option<toolkits::Action> {
        let rect = *h
            .app
            .controls
            .get(name)
            .unwrap_or_else(|| panic!("no control {name}"));
        let pos = rect.center();
        frame_tall(
            h,
            ws,
            ctx,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        let action = frame_tall(h, ws, ctx, vec![pointer(pos, false)]).1;
        frame_tall(h, ws, ctx, vec![]);
        action
    }

    fn click_pos(h: &mut Host<PdeModeler>, ws: &Snapshot, ctx: &egui::Context, pos: egui::Pos2) {
        frame_tall(
            h,
            ws,
            ctx,
            vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
        );
        frame_tall(h, ws, ctx, vec![pointer(pos, false)]);
        frame_tall(h, ws, ctx, vec![]);
    }

    fn drag_pos(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        start: egui::Pos2,
        end: egui::Pos2,
    ) {
        let mid = egui::pos2(0.5 * (start.x + end.x), 0.5 * (start.y + end.y));
        frame_tall(
            h,
            ws,
            ctx,
            vec![egui::Event::PointerMoved(start), pointer(start, true)],
        );
        frame_tall(h, ws, ctx, vec![egui::Event::PointerMoved(mid)]);
        frame_tall(
            h,
            ws,
            ctx,
            vec![egui::Event::PointerMoved(end), pointer(end, false)],
        );
        frame_tall(h, ws, ctx, vec![]);
    }

    fn press_key(h: &mut Host<PdeModeler>, ws: &Snapshot, ctx: &egui::Context, key: egui::Key) {
        frame_tall(
            h,
            ws,
            ctx,
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame_tall(h, ws, ctx, vec![]);
    }

    fn type_chars(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        control: &str,
        text: &str,
    ) {
        click_tall(h, ws, ctx, control);
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        };
        let del = egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame_tall(h, ws, ctx, vec![select_all, del]);
        for ch in text.chars() {
            frame_tall(h, ws, ctx, vec![egui::Event::Text(ch.to_string())]);
        }
        frame_tall(h, ws, ctx, vec![]);
    }

    fn type_into(
        h: &mut Host<PdeModeler>,
        ws: &Snapshot,
        ctx: &egui::Context,
        control: &str,
        text: &str,
    ) {
        click_tall(h, ws, ctx, control);
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        };
        let del = egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame_tall(
            h,
            ws,
            ctx,
            vec![select_all, del, egui::Event::Text(text.to_owned())],
        );
        frame_tall(h, ws, ctx, vec![]);
    }

    fn wait_decompose(h: &mut Host<PdeModeler>, ws: &Snapshot, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            frame_tall(h, ws, ctx, vec![]);
            if !h.app.decompose_runner.running() && !h.app.canvas.decomposed_edges.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Decompose timed out: err={:?}",
                h.app.decompose_error
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        frame_tall(h, ws, ctx, vec![]);
    }

    #[test]
    fn pde_modeler_canvas_draw_select_move_delete_and_refusals() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Transform round-trip <= 1e-9 in f64.
        let sample_pt = [0.372819, -0.841293];
        let scr = h.app.canvas.view.data_to_screen_f64(sample_pt);
        let back = h.app.canvas.view.screen_to_data_f64(scr);
        assert!((back[0] - sample_pt[0]).abs() <= 1e-9);
        assert!((back[1] - sample_pt[1]).abs() <= 1e-9);

        // Turn Snap OFF and drag Rectangle -> R1 corners match screen_to_data <= 1e-9.
        assert!(h.app.canvas.snap);
        click_tall(&mut h, &ws, &ctx, "Canvas:Snap");
        assert!(!h.app.canvas.snap);

        let p0 = h.app.canvas.view.data_to_screen([-0.73, -0.41]);
        let p1 = h.app.canvas.view.data_to_screen([0.62, 0.54]);
        let expected_a = h.app.canvas.view.screen_to_data(p0);
        let expected_b = h.app.canvas.view.screen_to_data(p1);
        drag_pos(&mut h, &ws, &ctx, p0, p1);

        assert_eq!(h.app.model.shapes.len(), 1);
        let Shape::Rectangle {
            ref name,
            x0,
            y0,
            w,
            h: rh,
        } = h.app.model.shapes[0]
        else {
            panic!("expected Rectangle R1");
        };
        assert_eq!(name, "R1");
        assert!((x0 - expected_a[0].min(expected_b[0])).abs() <= 1e-9);
        assert!((y0 - expected_a[1].min(expected_b[1])).abs() <= 1e-9);
        assert!((w - (expected_b[0] - expected_a[0]).abs()).abs() <= 1e-9);
        assert!((rh - (expected_b[1] - expected_a[1]).abs()).abs() <= 1e-9);

        // Turn Snap ON and drag Rectangle -> R2 has exact 0.1 grid values.
        click_tall(&mut h, &ws, &ctx, "Canvas:Snap");
        assert!(h.app.canvas.snap);
        let s0 = h.app.canvas.view.data_to_screen([-0.52, -0.28]);
        let s1 = h.app.canvas.view.data_to_screen([0.41, 0.32]);
        drag_pos(&mut h, &ws, &ctx, s0, s1);
        assert_eq!(h.app.model.shapes.len(), 2);
        let Shape::Rectangle {
            ref name,
            x0,
            y0,
            w,
            h: rh,
        } = h.app.model.shapes[1]
        else {
            panic!("expected Rectangle R2");
        };
        assert_eq!(name, "R2");
        assert_eq!(x0, -0.5);
        assert_eq!(y0, -0.3);
        assert!((w - 0.9).abs() <= 1e-12);
        assert!((rh - 0.6).abs() <= 1e-12);

        // Zero-size drag is refused with an info-line message.
        let z = h.app.canvas.view.data_to_screen([0.0, 0.0]);
        click_pos(&mut h, &ws, &ctx, z);
        assert_eq!(h.app.model.shapes.len(), 2);
        assert!(
            h.app
                .canvas
                .info_message
                .as_deref()
                .is_some_and(|m| m.to_lowercase().contains("refused")),
            "expected refusal message, got {:?}",
            h.app.canvas.info_message
        );

        // Circle tool drag gives expected radius.
        click_tall(&mut h, &ws, &ctx, "Tool:Circle");
        let c_center = h.app.canvas.view.data_to_screen([0.0, 0.0]);
        let c_edge = h.app.canvas.view.data_to_screen([0.5, 0.0]);
        drag_pos(&mut h, &ws, &ctx, c_center, c_edge);
        assert_eq!(h.app.model.shapes.len(), 3);
        let Shape::Circle {
            ref name,
            cx,
            cy,
            r,
        } = h.app.model.shapes[2]
        else {
            panic!("expected Circle C1");
        };
        assert_eq!(name, "C1");
        assert_eq!(cx, 0.0);
        assert_eq!(cy, 0.0);
        assert!((r - 0.5).abs() <= 1e-12);

        // Polygon tool: 4 clicks + closing click on first vertex -> P1 with 4 vertices.
        click_tall(&mut h, &ws, &ctx, "Tool:Polygon");
        let poly_pts = [[-1.0, -1.0], [-0.5, -1.0], [-0.5, -0.5], [-1.0, -0.5]];
        for pt in poly_pts {
            let pos = h.app.canvas.view.data_to_screen(pt);
            click_pos(&mut h, &ws, &ctx, pos);
        }
        let close_pos = h.app.canvas.view.data_to_screen(poly_pts[0]);
        click_pos(&mut h, &ws, &ctx, close_pos);
        assert_eq!(h.app.model.shapes.len(), 4);
        let Shape::Polygon { ref name, ref pts } = h.app.model.shapes[3] else {
            panic!("expected Polygon P1");
        };
        assert_eq!(name, "P1");
        assert_eq!(pts.len(), 4);

        // Polygon tool: self-intersecting bowtie polygon is refused with info message.
        let bowtie = [[0.2, 0.2], [0.8, 0.8], [0.8, 0.2], [0.2, 0.8]];
        for pt in bowtie {
            let pos = h.app.canvas.view.data_to_screen(pt);
            click_pos(&mut h, &ws, &ctx, pos);
        }
        let close_bow = h.app.canvas.view.data_to_screen(bowtie[0]);
        click_pos(&mut h, &ws, &ctx, close_bow);
        assert_eq!(h.app.model.shapes.len(), 4);
        assert!(
            h.app
                .canvas
                .info_message
                .as_deref()
                .is_some_and(|m| m.to_lowercase().contains("self-intersect")),
            "expected self-intersecting refusal, got {:?}",
            h.app.canvas.info_message
        );

        // Polygon tool: Esc cancels an in-progress polygon.
        let ep0 = h.app.canvas.view.data_to_screen([0.8, 0.8]);
        click_pos(&mut h, &ws, &ctx, ep0);
        let ep1 = h.app.canvas.view.data_to_screen([1.0, 0.8]);
        click_pos(&mut h, &ws, &ctx, ep1);
        assert_eq!(h.app.canvas.poly_draft.len(), 2);
        press_key(&mut h, &ws, &ctx, egui::Key::Escape);
        assert!(h.app.canvas.poly_draft.is_empty());
        assert_eq!(h.app.model.shapes.len(), 4);

        // Select + click picks C1; drag moves it by the drag delta.
        click_tall(&mut h, &ws, &ctx, "Tool:Select");
        let move_from = h.app.canvas.view.data_to_screen([0.0, 0.0]);
        click_pos(&mut h, &ws, &ctx, move_from);
        assert_eq!(h.app.canvas.selected_shape, Some(2), "click should select C1");
        let move_to = h.app.canvas.view.data_to_screen([0.3, -0.2]);
        drag_pos(&mut h, &ws, &ctx, move_from, move_to);
        let Shape::Circle { cx, cy, .. } = h.app.model.shapes[2] else {
            panic!("expected Circle C1");
        };
        assert!((cx - 0.3).abs() <= 1e-12);
        assert!((cy - (-0.2)).abs() <= 1e-12);

        // Delete removes the selected shape.
        press_key(&mut h, &ws, &ctx, egui::Key::Delete);
        assert_eq!(h.app.model.shapes.len(), 3);
        assert!(h.app.model.shapes.iter().all(|s| s.name() != "C1"));
    }

    #[test]
    fn pde_modeler_shape_fields_and_rename_validation() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.model.add_rectangle(0.0, 0.0, 1.0, 1.0);
        h.app.model.add_rectangle(2.0, 0.0, 1.0, 1.0);
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Editing numeric field x0 moves R1.
        type_into(&mut h, &ws, &ctx, "Shape:0:x0", "0.5");
        let Shape::Rectangle { x0, .. } = h.app.model.shapes[0] else {
            panic!("expected Rectangle");
        };
        assert!((x0 - 0.5).abs() <= 1e-12);

        // Renaming R1 to duplicate R2 is refused.
        type_into(&mut h, &ws, &ctx, "Shape:0:name", "R2");
        assert_eq!(h.app.model.shapes[0].name(), "R1");
        assert!(
            h.app
                .shape_name_error
                .as_deref()
                .is_some_and(|e| e.to_lowercase().contains("duplicate")),
            "expected duplicate error, got {:?}",
            h.app.shape_name_error
        );

        // Renaming R1 to invalid identifier '1bad' is refused.
        type_into(&mut h, &ws, &ctx, "Shape:0:name", "1bad");
        assert_eq!(h.app.model.shapes[0].name(), "R1");
        assert!(
            h.app
                .shape_name_error
                .as_deref()
                .is_some_and(|e| e.to_lowercase().contains("invalid")),
            "expected invalid error, got {:?}",
            h.app.shape_name_error
        );

        // Typing '1' '.' '5' as separate events keeps the decimal point.
        type_chars(&mut h, &ws, &ctx, "Shape:0:w", "1.5");
        let Shape::Rectangle { w, .. } = h.app.model.shapes[0] else {
            panic!("expected Rectangle");
        };
        assert!((w - 1.5).abs() <= 1e-12, "w = {w}, expected 1.5");

        // Typing '-0.5' char by char into x0 keeps the sign and the point.
        type_chars(&mut h, &ws, &ctx, "Shape:0:x0", "-0.5");
        let Shape::Rectangle { x0, w, .. } = h.app.model.shapes[0] else {
            panic!("expected Rectangle");
        };
        assert!((x0 - (-0.5)).abs() <= 1e-12, "x0 = {x0}, expected -0.5");
        assert!((w - 1.5).abs() <= 1e-12);

        // A non-positive width is refused under the shape list and not applied.
        type_into(&mut h, &ws, &ctx, "Shape:0:w", "0");
        let Shape::Rectangle { w, .. } = h.app.model.shapes[0] else {
            panic!("expected Rectangle");
        };
        assert!((w - 1.5).abs() <= 1e-12, "rejected width was applied: {w}");
        assert_eq!(
            h.app.shape_geom_error.as_deref(),
            Some("Refused degenerate rectangle: width and height must be positive.")
        );
        assert!(h.app.controls.contains_key("Shape geometry error"));

        // Loading a sample marks the formula edited; a rename rewrites that token.
        h.app.load_plate_with_hole();
        frame_tall(&mut h, &ws, &ctx, vec![]);
        assert!(h.app.formula_edited);
        assert_eq!(h.app.model.set_formula, "R1-C1");
        type_into(&mut h, &ws, &ctx, "Shape:0:name", "Plate");
        assert_eq!(h.app.model.shapes[0].name(), "Plate");
        assert_eq!(h.app.model.shapes[1].name(), "C1");
        assert_eq!(h.app.model.set_formula, "Plate-C1");
        assert!(
            h.app.model.validate_set_formula().is_ok(),
            "renamed formula should still validate: {:?}",
            h.app.model.validate_set_formula()
        );
        assert_eq!(
            apps::pde_modeler::replace_formula_token("R1+R10-R1", "R1", "Plate"),
            "Plate+R10-Plate"
        );
    }

    #[test]
    fn pde_modeler_formula_auto_union_manual_edit_and_error() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Draw R1 then C1 -> formula auto-updates to union.
        let p0 = h.app.canvas.view.data_to_screen([-0.5, -0.5]);
        let p1 = h.app.canvas.view.data_to_screen([0.5, 0.5]);
        drag_pos(&mut h, &ws, &ctx, p0, p1);
        assert_eq!(h.app.model.set_formula, "R1");

        click_tall(&mut h, &ws, &ctx, "Tool:Circle");
        let c0 = h.app.canvas.view.data_to_screen([0.0, 0.0]);
        let c1 = h.app.canvas.view.data_to_screen([0.2, 0.0]);
        drag_pos(&mut h, &ws, &ctx, c0, c1);
        assert_eq!(h.app.model.set_formula, "R1+C1");

        // Manual edit to 'R1-C1' stops auto-updating on further shape additions.
        type_into(&mut h, &ws, &ctx, "Set formula", "R1-C1");
        assert!(h.app.formula_edited);
        assert_eq!(h.app.model.set_formula, "R1-C1");

        let c2 = h.app.canvas.view.data_to_screen([0.8, 0.8]);
        let c3 = h.app.canvas.view.data_to_screen([1.0, 0.8]);
        drag_pos(&mut h, &ws, &ctx, c2, c3);
        assert_eq!(h.app.model.shapes.len(), 3);
        assert_eq!(h.app.model.set_formula, "R1-C1");

        // Invalid formula 'R1-Q7' is shown under the field and names Q7.
        type_into(&mut h, &ws, &ctx, "Set formula", "R1-Q7");
        let shown = h.app.formula_error_shown.clone().unwrap_or_default();
        assert!(
            shown.contains("Q7"),
            "expected Q7 under the field, got {shown:?}"
        );
        assert!(
            h.app.controls.contains_key("Set formula error"),
            "formula error label was not recorded"
        );
    }

    #[test]
    fn pde_modeler_bcs_decompose_assign_script_and_stale() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        // Clear BCs & edges to test building them from scratch via Decompose + panel.
        h.app.model.edge_ids.clear();
        h.app.model.boundary_conditions.clear();
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Click Decompose and wait for edges E1..E8.
        click_tall(&mut h, &ws, &ctx, "Decompose");
        wait_decompose(&mut h, &ws, &ctx);
        assert_eq!(h.app.model.edge_ids, (1..=8).collect::<Vec<_>>());
        assert_eq!(h.app.canvas.decomposed_edges.len(), 8);

        // Clicking an edge on the canvas (Select tool) selects its row.
        click_tall(&mut h, &ws, &ctx, "Tool:Select");
        let e2_pos = h
            .app
            .canvas
            .view
            .data_to_screen(h.app.canvas.decomposed_edges[1].label_pos);
        click_pos(&mut h, &ws, &ctx, e2_pos);
        assert_eq!(h.app.canvas.selected_edge, Some(2));

        // Set outer edges 1..=4 Dirichlet 0 via panel buttons, hole edges 5..=8 Neumann 0
        // (setting E5 Neumann 0 and using "Apply to all unassigned" for E6..E8).
        for edge in 1..=4 {
            click_tall(&mut h, &ws, &ctx, &format!("Edge:{edge}:Dirichlet"));
        }
        click_tall(&mut h, &ws, &ctx, "Edge:5:Neumann");
        click_tall(&mut h, &ws, &ctx, "Apply to all unassigned");

        for edge in 1..=4 {
            assert_eq!(
                h.app.model.boundary_conditions.get(&edge),
                Some(&BoundaryCondition::Dirichlet { u: "0".into() })
            );
        }
        for edge in 5..=8 {
            assert_eq!(
                h.app.model.boundary_conditions.get(&edge),
                Some(&BoundaryCondition::Neumann {
                    g: "0".into(),
                    q: "0".into()
                })
            );
        }

        // Generated script equals the 8c1 plate_with_hole sample's script.
        let generated = h.app.model.script().unwrap();
        let expected = PdeAppModel::plate_with_hole().script().unwrap();
        assert_eq!(generated, expected);
        assert!(!h.app.bcs_stale);

        // Adding a shape afterwards marks BCs stale.
        click_tall(&mut h, &ws, &ctx, "Tool:Circle");
        let p0 = h.app.canvas.view.data_to_screen([0.6, 0.6]);
        let p1 = h.app.canvas.view.data_to_screen([0.8, 0.6]);
        drag_pos(&mut h, &ws, &ctx, p0, p1);
        assert!(h.app.bcs_stale);
        assert!(h.app.bc_notice.is_some());
    }

    #[test]
    fn pde_modeler_pde_panel_eigen_and_mesh_panel() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Elliptic shows f and hides range.
        assert!(h.app.controls.contains_key("PDE:f"));
        assert!(!h.app.controls.contains_key("PDE:range"));

        // Switching to Eigen hides f and shows range; generated script uses solvepdeeig.
        click_tall(&mut h, &ws, &ctx, "PDE:Eigen");
        assert_eq!(h.app.model.pde_type, PdeType::Eigen);
        assert!(!h.app.controls.contains_key("PDE:f"));
        assert!(h.app.controls.contains_key("PDE:range"));
        let script = h.app.model.script().unwrap();
        assert!(
            script.contains("solvepdeeig(model,"),
            "expected solvepdeeig in script:\n{script}"
        );

        // Mesh panel: switch order to Linear and edit Hmax.
        click_tall(&mut h, &ws, &ctx, "Mesh:Linear");
        assert_eq!(h.app.model.mesh.order, GeometricOrder::Linear);
        type_into(&mut h, &ws, &ctx, "Mesh:Hmax", "0.5");
        assert_eq!(h.app.model.mesh.hmax, "0.5");
    }

    #[test]
    fn pde_modeler_panel_polygon_refusal_and_concave_fill() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app
            .model
            .add_polygon(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        frame_tall(&mut h, &ws, &ctx, vec![]);
        let before = h.app.model.shapes[0].clone();
        // Moving the third vertex across the left edge self-intersects.
        type_into(&mut h, &ws, &ctx, "Shape:0:x2", "-0.5");
        assert_eq!(h.app.model.shapes[0], before);
        assert_eq!(
            h.app.shape_geom_error.as_deref(),
            Some("Refused degenerate polygon: edges must not self-intersect.")
        );

        let l = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let tris = apps::pde_modeler::canvas::triangulate_polygon(&l);
        assert_eq!(tris.len(), 12, "L-shape should be four triangles");
        let area: f64 = tris
            .chunks(3)
            .map(|c| {
                0.5 * ((c[1][0] - c[0][0]) * (c[2][1] - c[0][1])
                    - (c[1][1] - c[0][1]) * (c[2][0] - c[0][0]))
                    .abs()
            })
            .sum();
        assert!(
            (area - 3.0).abs() < 1e-9,
            "concave fill area {area}, convex hull would be 3.5"
        );
    }

    #[test]
    fn pde_modeler_decompose_discards_stale_and_sample_load_stops() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        frame_tall(&mut h, &ws, &ctx, vec![]);
        assert!(h.app.canvas.decomposed_edges.is_empty());

        h.app.trigger_decompose(&ctx);
        assert!(h.app.decompose_runner.key().is_some());
        assert!(
            h.app.canvas.decomposed_edges.is_empty(),
            "result applied before the geometry could change"
        );

        if let Shape::Rectangle { w, .. } = &mut h.app.model.shapes[0] {
            *w = 2.5;
        }
        h.app.mark_geometry_changed(false);
        assert!(h.app.bcs_stale);

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            frame_tall(&mut h, &ws, &ctx, vec![]);
            if !h.app.decompose_runner.running() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Decompose timed out: err={:?}",
                h.app.decompose_error
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        frame_tall(&mut h, &ws, &ctx, vec![]);
        assert!(
            h.app.bcs_stale,
            "stale decompose cleared bcs_stale"
        );
        assert!(
            h.app.canvas.decomposed_edges.is_empty(),
            "stale decompose wrote edges"
        );
        assert_eq!(h.app.model.boundary_conditions.len(), 8);

        h.app.trigger_decompose(&ctx);
        assert!(h.app.decompose_runner.key().is_some());
        h.app.load_plate_heat();
        assert!(h.app.decompose_runner.key().is_none());
        assert!(h.app.decompose_runner.result().is_none());
        assert!(h.app.canvas.decomposed_edges.is_empty());
        assert!(!h.app.bcs_stale);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            frame_tall(&mut h, &ws, &ctx, vec![]);
            if !h.app.decompose_runner.running() {
                break;
            }
            assert!(Instant::now() < deadline, "stopped decompose kept running");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            h.app.canvas.decomposed_edges.is_empty(),
            "sample load let a late decompose overwrite edges"
        );
        assert!(!h.app.bcs_stale);
        assert_eq!(h.app.model.pde_type, PdeType::Parabolic);
    }

    fn wait_idle(h: &mut Host<PdeModeler>, ws: &Snapshot, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            frame(h, ws, ctx, vec![]);
            if !h.running()
                && !h.app.decompose_runner.running()
                && !h.app.plot_runner.running()
            {
                break;
            }
            assert!(Instant::now() < deadline, "PDE Modeler runner timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        frame(h, ws, ctx, vec![]);
    }

    fn solution_patch_cdata(h: &Host<PdeModeler>) -> matrust::Matrix {
        let view = h
            .app
            .solution_view
            .as_ref()
            .expect("solution_view should exist");
        view.figure
            .series
            .iter()
            .find_map(|s| s.indexed_patch.as_ref().and_then(|ip| ip.colors.clone()))
            .expect("solution figure should contain a patch with CData")
    }

    #[test]
    fn pde_modeler_plate_and_heat_solution_cdata_and_slider_no_resolve() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame(&mut h, &ws, &ctx, vec![]);

        // Plate sample one click -> Solution tab with patch whose CData equals results.NodalSolution.
        click(&mut h, &ws, &ctx, "Plate with a hole");
        wait_idle(&mut h, &ws, &ctx);
        assert_eq!(h.app.active_tab, apps::pde_modeler::views::MainTab::Solution);
        assert!(h.app.geometry_view.is_some());
        assert!(h.app.mesh_view.is_some());
        assert!(h.app.solution_view.is_some());

        let patch_c = solution_patch_cdata(&h);
        let matrust::Array::Numeric(ref nodal) = h.app.nodal_solution.clone().unwrap() else {
            panic!("expected numeric NodalSolution");
        };
        assert_eq!(patch_c.data(), nodal.data());

        // Toggling Contour, Mesh overlay, and 3-D surface replots without re-solving.
        let solves_before_opts = h.app.solve_count;
        click(&mut h, &ws, &ctx, "Solution:Contour");
        click(&mut h, &ws, &ctx, "Solution:Mesh");
        click(&mut h, &ws, &ctx, "Solution:3D");
        wait_idle(&mut h, &ws, &ctx);
        assert_eq!(h.app.solve_count, solves_before_opts);
        assert!(h.app.solution_view.is_some());

        // Turn 3-D off before checking 2-D heat sample.
        click(&mut h, &ws, &ctx, "Solution:3D");
        wait_idle(&mut h, &ws, &ctx);

        // Heat sample one click -> 21 positions; moving slider to last changes CData to column 21
        // without a new solve (solve_count unchanged).
        click(&mut h, &ws, &ctx, "Heat in a plate");
        wait_idle(&mut h, &ws, &ctx);
        assert_eq!(h.app.active_tab, apps::pde_modeler::views::MainTab::Solution);
        let tlist = h.app.tlist.clone().expect("parabolic tlist");
        assert_eq!(tlist.len(), 21);
        assert!(h.app.controls.contains_key("Solution:TimeSlider"));
        assert!(h.app.controls.contains_key("Solution:PlayPause"));

        let solves_after_heat = h.app.solve_count;
        h.app.set_time_index(20, &ctx);
        wait_idle(&mut h, &ws, &ctx);
        assert_eq!(
            h.app.solve_count, solves_after_heat,
            "moving time slider must not re-solve"
        );
        assert_eq!(h.app.time_index, 20);

        let patch_last = solution_patch_cdata(&h);
        let matrust::Array::Numeric(ref nodal_heat) = h.app.nodal_solution.clone().unwrap() else {
            panic!("expected numeric NodalSolution for heat");
        };
        assert_eq!(nodal_heat.cols(), 21);
        let col21: Vec<f64> = (0..nodal_heat.rows())
            .map(|r| nodal_heat.get(r, 20))
            .collect();
        assert_eq!(
            patch_last.data(),
            col21.as_slice(),
            "patch CData must equal column 21 of NodalSolution"
        );
    }

    #[test]
    fn pde_modeler_eigen_unit_square_first_lambda_and_mode_picker() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.model.add_rectangle(0.0, 0.0, 1.0, 1.0);
        h.app.set_pde_type(PdeType::Eigen);
        h.app.model.edge_ids = vec![1, 2, 3, 4];
        for e in 1..=4 {
            h.app
                .model
                .boundary_conditions
                .insert(e, BoundaryCondition::Dirichlet { u: "0".into() });
        }
        frame(&mut h, &ws, &ctx, vec![]);

        click(&mut h, &ws, &ctx, "Solve");
        wait_idle(&mut h, &ws, &ctx);

        let eigs = h.app.eigenvalues.clone().expect("eigenvalues");
        assert!(!eigs.is_empty(), "expected at least one eigenvalue");
        let target = 2.0 * std::f64::consts::PI * std::f64::consts::PI;
        assert!(
            (eigs[0] - target).abs() < 1e-2,
            "first lambda = {}, expected within 1e-2 of 2*pi^2 = {target}",
            eigs[0]
        );
        assert!(h.app.controls.contains_key("Solution:ModePicker"));

        // Switching to mode 2 updates patch CData without re-solving.
        if eigs.len() >= 2 {
            let solves_before = h.app.solve_count;
            h.app.set_eigen_index(1, &ctx);
            wait_idle(&mut h, &ws, &ctx);
            assert_eq!(h.app.solve_count, solves_before);
            let patch_c = solution_patch_cdata(&h);
            let matrust::Array::Numeric(ref vecs) = h.app.nodal_solution.clone().unwrap() else {
                panic!("expected numeric Eigenvectors");
            };
            let col2: Vec<f64> = (0..vecs.rows()).map(|r| vecs.get(r, 1)).collect();
            assert_eq!(patch_c.data(), col2.as_slice());
        }
    }

    #[test]
    fn pde_modeler_export_workspace_isequal_and_custom_names() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame(&mut h, &ws, &ctx, vec![]);

        click(&mut h, &ws, &ctx, "Plate with a hole");
        wait_idle(&mut h, &ws, &ctx);

        // Export with default names (model, results) and run in Engine.
        let action = click(&mut h, &ws, &ctx, "Export to workspace");
        let Some(toolkits::Action::ExportScript(export_code)) = action else {
            panic!("expected ExportScript action");
        };
        let mut eng = matrust::Engine::default();
        eng.execute(&export_code).expect("export script runs cleanly");
        eng.execute("ws_u = results.NodalSolution;").unwrap();
        let ws_u = eng.array("ws_u").unwrap();
        let app_u = h.app.nodal_solution.clone().unwrap();

        let mut cmp = matrust::Engine::default();
        cmp.insert_array("u_app", app_u.clone()).unwrap();
        cmp.insert_array("u_ws", ws_u).unwrap();
        cmp.execute("ok = isequal(u_app, u_ws);").unwrap();
        let matrust::Array::Logical(flag) = cmp.array("ok").unwrap() else {
            panic!("expected logical");
        };
        assert_eq!(flag.data()[0], 1, "exported NodalSolution must equal app's");
        assert!(eng.array("model").is_some(), "model must be in workspace");
        assert!(eng.array("results").is_some(), "results must be in workspace");

        // Custom export names also work.
        h.app.export_names = ["my_pde".into(), "my_res".into()];
        frame(&mut h, &ws, &ctx, vec![]);
        let custom_action = click(&mut h, &ws, &ctx, "Export to workspace");
        let Some(toolkits::Action::ExportScript(custom_code)) = custom_action else {
            panic!("expected custom ExportScript action");
        };
        let mut eng2 = matrust::Engine::default();
        eng2.execute(&custom_code).expect("custom export runs");
        assert!(eng2.array("my_pde").is_some());
        assert!(eng2.array("my_res").is_some());

        // Generate script emits SaveAppScript containing createpde workflow.
        let gen_action = click(&mut h, &ws, &ctx, "Generate script");
        let Some(toolkits::Action::SaveAppScript(AppKind::PdeModeler, saved)) = gen_action else {
            panic!("expected SaveAppScript action");
        };
        assert!(saved.contains("my_pde = createpde();"), "{saved}");
        assert!(saved.contains("my_res = solvepde(my_pde);"), "{saved}");
    }

    #[test]
    fn pde_modeler_ugly_no_shape_drawn() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Solve");
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.contains('\n'), "expected one line: {msg}");
        assert!(msg.to_lowercase().contains("no shape"), "{msg}");
        assert!(h.app.solution_view.is_none());
        assert_eq!(h.app.solve_count, 0);
    }

    #[test]
    fn pde_modeler_ugly_invalid_set_formula() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        h.app.model.set_formula = "R1+Z9".into();
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Solve");
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.contains('\n'), "expected one line: {msg}");
        assert!(msg.contains("Z9"), "{msg}");
        assert!(h.app.solution_view.is_none());
        assert_eq!(h.app.solve_count, 0);
    }

    #[test]
    fn pde_modeler_ugly_decsg_empty_region() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.model.add_rectangle(0.0, 0.0, 1.0, 1.0);
        h.app.model.set_formula = "R1-R1".into();
        frame_tall(&mut h, &ws, &ctx, vec![]);
        click_tall(&mut h, &ws, &ctx, "Decompose");
        wait_idle(&mut h, &ws, &ctx);
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.is_empty() && !msg.contains('\n'), "expected one-line decsg error: {msg:?}");
        assert!(h.app.info_is_error);
        assert!(h.app.geometry_view.is_none());
        assert!(h.app.solution_view.is_none());
    }

    #[test]
    fn pde_modeler_ugly_mesh_fails() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        // The real engine failure: generateMesh refuses a too-fine Hmax.
        h.app.model.mesh.hmax = "0.0001".into();
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Mesh");
        assert_eq!(h.app.mesh_count, 1, "Hmax 0.0001 must reach the mesher");
        wait_idle(&mut h, &ws, &ctx);
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.contains('\n'), "expected one line: {msg}");
        assert!(h.app.info_is_error);
        assert!(
            msg.contains("generateMesh") && msg.contains("too small"),
            "expected the generateMesh 'too small' line, got {msg:?}"
        );
        assert!(h.app.mesh_view.is_none());
        assert!(h.app.node_count.is_none());
    }

    #[test]
    fn pde_modeler_ugly_mesh_negative_hmax_refused_before_mesher() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        h.app.model.mesh.hmax = "-0.5".into();
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Mesh");
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.contains('\n'), "expected one line: {msg}");
        assert!(msg.to_lowercase().contains("hmax"), "{msg}");
        assert_eq!(h.app.mesh_count, 0, "validation must refuse before the mesher");
        assert!(h.app.mesh_view.is_none());
    }

    #[test]
    fn pde_modeler_ugly_missing_boundary_condition() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        h.app.model.boundary_conditions.remove(&2);
        h.app.model.boundary_conditions.remove(&6);
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Solve");
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.contains('\n'), "expected one line: {msg}");
        assert!(msg.contains('2') && msg.contains('6'), "{msg}");
        assert!(h.app.solution_view.is_none());
        assert_eq!(h.app.solve_count, 0);
    }

    #[test]
    fn pde_modeler_ugly_coefficient_expression_error() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        h.app.model.coeffs.c = "unknown_coeff_symbol_xyz".into();
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Solve");
        wait_idle(&mut h, &ws, &ctx);
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.is_empty() && !msg.contains('\n'), "expected one-line coeff error: {msg:?}");
        assert!(h.app.info_is_error);
        assert!(h.app.solution_view.is_none());
    }

    #[test]
    fn pde_modeler_ugly_solve_error() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        // Eigen with range [0 0.1] contains no eigenvalues -> solve error.
        h.app.set_pde_type(PdeType::Eigen);
        h.app.model.eigen_range = "[0 0.1]".into();
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Solve");
        wait_idle(&mut h, &ws, &ctx);
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(!msg.is_empty() && !msg.contains('\n'), "expected one-line solve error: {msg:?}");
        assert!(h.app.info_is_error);
        assert!(h.app.solution_view.is_none());
    }

    #[test]
    fn pde_modeler_ugly_dof_guard_no_engine_call() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_heat();
        h.app.model.mesh.hmax = "0.05".into();
        frame(&mut h, &ws, &ctx, vec![]);
        // Only the real node count refuses: mesh first, then Solve.
        click(&mut h, &ws, &ctx, "Mesh");
        wait_idle(&mut h, &ws, &ctx);
        let nodes = h.app.node_count.expect("mesh node count");
        assert!(nodes > 4096, "Hmax 0.05 quadratic plate should exceed 4096 nodes: {nodes}");
        click(&mut h, &ws, &ctx, "Solve");

        assert_eq!(h.app.solve_count, 0, "DOF guard must not call the engine");
        assert!(!h.running());
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert_eq!(
            msg,
            format!(
                "Mesh has {nodes} nodes; time-dependent solves are limited to 4096. Increase Hmax."
            )
        );
        assert!(h.app.solution_view.is_none());
    }

    #[test]
    fn pde_modeler_off_thread_frame_and_cancel() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        h.app.test_solve_delay_ms = 5000;
        frame(&mut h, &ws, &ctx, vec![]);

        let t0 = Instant::now();
        click(&mut h, &ws, &ctx, "Solve");
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "UI frame must return immediately while slow solve runs off-thread (took {elapsed:?})"
        );
        assert!(h.running(), "slow solve should still be running");
        assert!(h.app.controls.contains_key("Cancel"), "Cancel button must be shown");
        assert!(h.app.controls.contains_key("Loading"), "Loading state must be shown");

        click(&mut h, &ws, &ctx, "Cancel");
        let cancel_t0 = Instant::now();
        wait_idle(&mut h, &ws, &ctx);
        assert!(
            cancel_t0.elapsed() < Duration::from_secs(2),
            "Cancel must end the 5s slow solve promptly"
        );
        assert!(!h.running());
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(
            msg.to_lowercase().contains("cancelled"),
            "expected cancelled message, got {msg:?}"
        );
        assert!(h.app.solution_view.is_none(), "nothing half-drawn after cancel");
    }

    #[test]
    fn pde_modeler_narrow_width_stacks_and_many_shapes_scroll() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        for k in 0..20 {
            h.app.model.add_rectangle(k as f64 * 0.2, 0.0, 0.15, 0.15);
        }
        // Narrow window: inputs panel stacks above main view, no clipped controls.
        let narrow = egui::vec2(420.0, 700.0);
        frame_sized(&mut h, &ws, &ctx, narrow, vec![]);
        let sample_rect = h.app.controls["Plate with a hole"];
        let tab_rect = h.app.controls["Tab:Draw"];
        assert!(
            sample_rect.bottom() < tab_rect.top(),
            "at narrow width, inputs ({sample_rect:?}) must stack above main view ({tab_rect:?})"
        );
        assert!(
            sample_rect.right() <= narrow.x,
            "inputs button must not clip past narrow width: {sample_rect:?}"
        );
        assert!(
            h.app.controls["Solve"].right() <= narrow.x,
            "Solve button must not clip past narrow width"
        );
    }

    #[test]
    fn pde_modeler_7b_shape_name_error_clears_on_fix_or_geom_change() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.model.add_rectangle(0.0, 0.0, 1.0, 1.0);
        h.app.model.add_rectangle(2.0, 0.0, 1.0, 1.0);
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Trigger duplicate name error.
        type_into(&mut h, &ws, &ctx, "Shape:0:name", "R2");
        assert!(h.app.shape_name_error.is_some());

        // Fixing the name clears the error.
        type_into(&mut h, &ws, &ctx, "Shape:0:name", "Plate");
        assert!(h.app.shape_name_error.is_none());

        // Trigger again, then fix geometry by deleting R2 -> error clears.
        type_into(&mut h, &ws, &ctx, "Shape:0:name", "R2");
        assert!(h.app.shape_name_error.is_some());
        click_tall(&mut h, &ws, &ctx, "Shape:1:delete");
        assert!(
            h.app.shape_name_error.is_none(),
            "deleting conflicting shape must clear shape_name_error"
        );
    }

    #[test]
    fn pde_modeler_7b_stale_bcs_block_mesh_solve_and_script() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Modify geometry after edges/BCs exist -> BCs become stale.
        if let Shape::Rectangle { w, .. } = &mut h.app.model.shapes[0] {
            *w = 2.4;
        }
        h.app.mark_geometry_changed(false);
        assert!(h.app.bcs_stale);

        let expected = "Boundary conditions are out of date; press Decompose";
        assert_eq!(h.app.script(&ws).unwrap_err(), expected);

        click_tall(&mut h, &ws, &ctx, "Mesh");
        assert_eq!(h.app.mesh_count, 0, "stale BCs must block Mesh");
        assert_eq!(h.app.info_message.as_deref(), Some(expected));

        click_tall(&mut h, &ws, &ctx, "Solve");
        assert_eq!(h.app.solve_count, 0, "stale BCs must block Solve");
        assert_eq!(h.app.info_message.as_deref(), Some(expected));
    }

    #[test]
    fn pde_modeler_7b_edge_click_selects_and_scrolls_with_any_tool() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        frame_tall(&mut h, &ws, &ctx, vec![]);

        click_tall(&mut h, &ws, &ctx, "Decompose");
        wait_decompose(&mut h, &ws, &ctx);
        assert_eq!(h.app.canvas.decomposed_edges.len(), 8);

        // Keep Rectangle tool active (NOT Select) and click edge E3 on the canvas.
        assert_eq!(
            h.app.canvas.tool,
            apps::pde_modeler::canvas::DrawTool::Rectangle
        );
        let e3_pos = h
            .app
            .canvas
            .view
            .data_to_screen(h.app.canvas.decomposed_edges[2].label_pos);
        click_pos(&mut h, &ws, &ctx, e3_pos);
        assert_eq!(
            h.app.canvas.selected_edge,
            Some(3),
            "clicking edge with Rectangle tool must select E3"
        );
        assert_eq!(
            h.app.last_scrolled_edge,
            Some(3),
            "clicking edge must scroll its BC row into view"
        );
    }

    #[test]
    fn pde_modeler_7b_canvas_refits_when_shape_added_outside_view() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame(&mut h, &ws, &ctx, vec![]);

        let before = h.app.canvas.view.data_bounds();
        assert!(before[1] < 10.0, "initial view should not include x=20");

        // Add a shape far outside the current view.
        h.app.model.add_rectangle(20.0, 15.0, 4.0, 3.0);
        frame(&mut h, &ws, &ctx, vec![]);

        let after = h.app.canvas.view.data_bounds();
        assert!(
            after[0] <= 20.0 && after[1] >= 24.0 && after[2] <= 15.0 && after[3] >= 18.0,
            "canvas must refit to include newly added out-of-view shape: {after:?}"
        );
    }

    #[test]
    fn pde_modeler_8c4_heat_fixed_clim_and_flat_fallback() {
        // Flat solution (max == min) falls back to [min - 1, min + 1].
        let flat = matrust::Array::Numeric(
            matrust::Matrix::from_column_vec(4, 2, vec![5.0; 8]).unwrap(),
        );
        assert_eq!(
            apps::pde_modeler::views::global_solution_clim(&flat),
            Some([4.0, 6.0])
        );

        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame(&mut h, &ws, &ctx, vec![]);

        click(&mut h, &ws, &ctx, "Heat in a plate");
        wait_idle(&mut h, &ws, &ctx);

        let matrust::Array::Numeric(ref nodal) = h.app.nodal_solution.clone().unwrap() else {
            panic!("expected numeric NodalSolution");
        };
        assert_eq!(nodal.cols(), 21);
        let expected_min = nodal
            .data()
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f64::INFINITY, f64::min);
        let expected_max = nodal
            .data()
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(expected_max > expected_min);

        // At t index 1 (0-based 0), CLim equals [min(U(:)), max(U(:))].
        assert_eq!(h.app.time_index, 0);
        let clim_t1 = h
            .app
            .solution_view
            .as_ref()
            .expect("solution_view at t1")
            .figure
            .axes
            .extras
            .color_limits();
        assert_eq!(clim_t1, [expected_min, expected_max]);

        // At t index 21 (0-based 20), CLim still equals [min(U(:)), max(U(:))].
        h.app.set_time_index(20, &ctx);
        wait_idle(&mut h, &ws, &ctx);
        assert_eq!(h.app.time_index, 20);
        let clim_t21 = h
            .app
            .solution_view
            .as_ref()
            .expect("solution_view at t21")
            .figure
            .axes
            .extras
            .color_limits();
        assert_eq!(clim_t21, [expected_min, expected_max]);
    }

    #[test]
    fn pde_modeler_8c4_info_line_notice_vs_error_colors() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let p = tokens::palette(&ctx);
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Notice: stale BCs uses secondary text colour (not error colour).
        if let Shape::Rectangle { w, .. } = &mut h.app.model.shapes[0] {
            *w = 2.4;
        }
        h.app.mark_geometry_changed(false);
        frame_tall(&mut h, &ws, &ctx, vec![]);
        let (notice_text, notice_is_err) = h.app.info_line_state();
        assert!(notice_text.contains("Boundary conditions are out of date"));
        assert!(!notice_is_err);
        assert_eq!(h.app.last_info_color, Some(p.text_muted));

        // Clicking Solve while BCs are stale keeps notice colour (not error).
        click_tall(&mut h, &ws, &ctx, "Solve");
        let (_, after_click_err) = h.app.info_line_state();
        assert!(!after_click_err);
        assert_eq!(h.app.last_info_color, Some(p.text_muted));

        // Error: invalid Hmax on Mesh keeps error colour.
        h.app.load_plate_with_hole();
        h.app.model.mesh.hmax = "-1".into();
        frame_tall(&mut h, &ws, &ctx, vec![]);
        click_tall(&mut h, &ws, &ctx, "Mesh");
        let (err_text, is_err) = h.app.info_line_state();
        assert!(err_text.to_lowercase().contains("hmax"), "{err_text}");
        assert!(is_err);
        assert_eq!(h.app.last_info_color, Some(p.error));
    }

    #[test]
    fn pde_modeler_8c4_canvas_labels_no_crosshair_overlap_and_edge_hover() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        frame(&mut h, &ws, &ctx, vec![]);

        // Plate with a hole: R1 and C1 are both centered at (0,0), yet their
        // labels must be placed at top-left inside corners, never on origin,
        // and must not overlap each other.
        let origin = h.app.canvas.view.data_to_screen([0.0, 0.0]);
        assert_eq!(h.app.canvas.last_shape_labels.len(), 2);
        let r1_rect = h.app.canvas.last_shape_labels[0].1;
        let c1_rect = h.app.canvas.last_shape_labels[1].1;
        assert!(!r1_rect.contains(origin), "R1 label hit origin: {r1_rect:?}");
        assert!(!c1_rect.contains(origin), "C1 label hit origin: {c1_rect:?}");
        assert!(
            !r1_rect.intersects(c1_rect),
            "R1 ({r1_rect:?}) and C1 ({c1_rect:?}) labels must not overlap"
        );

        // Two identical overlapping rectangles: second label nudged down by height.
        h.app.model.shapes.clear();
        h.app.model.add_rectangle(-1.0, -1.0, 2.0, 2.0);
        h.app.model.add_rectangle(-1.0, -1.0, 2.0, 2.0);
        frame(&mut h, &ws, &ctx, vec![]);
        assert_eq!(h.app.canvas.last_shape_labels.len(), 2);
        let first = h.app.canvas.last_shape_labels[0].1;
        let second = h.app.canvas.last_shape_labels[1].1;
        assert!(
            !first.intersects(second) && second.top() >= first.bottom() - 0.5,
            "second overlapping label must nudge down: first={first:?}, second={second:?}"
        );

        // Decomposed-edge labels closer than 12 px: skip second, show on hover.
        let e1_pt = [0.0, 0.5];
        let e2_pt = [0.005, 0.5]; // ~1 px apart in screen space
        h.app.canvas.decomposed_edges = vec![
            apps::pde_modeler::canvas::DecomposedEdge {
                id: 1,
                points: vec![[-0.5, 0.5], [0.5, 0.5]],
                label_pos: e1_pt,
                boundary: true,
            },
            apps::pde_modeler::canvas::DecomposedEdge {
                id: 2,
                points: vec![[0.005, 0.2], [0.005, 0.8]],
                label_pos: e2_pt,
                boundary: true,
            },
        ];
        frame(&mut h, &ws, &ctx, vec![]);
        assert_eq!(
            h.app.canvas.last_drawn_edge_labels,
            vec![1],
            "overlapping edge label E2 (< 12 px from E1) must be skipped"
        );

        // Hovering near edge E2 reveals its label.
        let hover_e2 = h.app.canvas.view.data_to_screen([0.005, 0.25]);
        frame(&mut h, &ws, &ctx, vec![egui::Event::PointerMoved(hover_e2)]);
        assert!(
            h.app.canvas.last_drawn_edge_labels.contains(&2),
            "hovering skipped edge E2 must show its label: {:?}",
            h.app.canvas.last_drawn_edge_labels
        );
    }

    #[test]
    fn pde_modeler_8c4_inputs_order_20_shapes_decompose_above_list() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        for k in 0..20 {
            h.app.model.add_rectangle(k as f64 * 0.2, 0.0, 0.15, 0.15);
        }
        frame(&mut h, &ws, &ctx, vec![]);

        let set_formula = h.app.controls["Set formula"];
        let decompose = h.app.controls["Decompose"];
        let shape_list = h.app.controls["ShapeList"];
        let x0_field = h.app.controls["Shape:0:x0"];

        assert!(
            set_formula.bottom() <= decompose.top() + 1.0,
            "Set formula ({set_formula:?}) must be above Decompose ({decompose:?})"
        );
        assert!(
            decompose.top() < shape_list.bottom(),
            "Decompose ({decompose:?}) top must be above ShapeList ({shape_list:?}) bottom"
        );
        assert!(
            shape_list.height() <= 245.0,
            "20-shape list must scroll inside ~240 px max height, got {}",
            shape_list.height()
        );
        assert!(
            (x0_field.height() - tokens::CONTROL).abs() <= 1.0,
            "shape numeric field height must match tokens::CONTROL ({}), got {}",
            tokens::CONTROL,
            x0_field.height()
        );
    }

    #[test]
    fn pde_modeler_8c5_heat_t1_vertex_colors_match_colormap_first_entry() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame(&mut h, &ws, &ctx, vec![]);

        click(&mut h, &ws, &ctx, "Heat in a plate");
        wait_idle(&mut h, &ws, &ctx);
        assert_eq!(h.app.time_index, 0);

        let check_bottom_color = |h: &Host<PdeModeler>| {
            let view = h
                .app
                .solution_view
                .as_ref()
                .expect("solution_view should exist");
            let map = view.figure.axes.extras.effective_colormap();
            let bottom_rgb: [u8; 3] = map[0].map(|c| (c * 255.0).round() as u8);
            let face_colors = view
                .figure
                .series
                .iter()
                .find(|s| s.indexed_patch.is_some())
                .and_then(|s| s.face_colors.as_ref())
                .expect("indexed patch should have mapped face_colors");
            assert!(!face_colors.is_empty());
            for (i, &rgb) in face_colors.iter().enumerate() {
                assert_eq!(
                    rgb, bottom_rgb,
                    "frame 1 face {i} color {rgb:?} should equal colormap first entry {bottom_rgb:?}"
                );
            }
        };

        check_bottom_color(&h);

        // Moving away and back to t index 1 via replot must also map to colormap's first entry.
        h.app.set_time_index(20, &ctx);
        wait_idle(&mut h, &ws, &ctx);
        h.app.set_time_index(0, &ctx);
        wait_idle(&mut h, &ws, &ctx);
        check_bottom_color(&h);
    }

    #[test]
    fn pde_modeler_8c5_engine_errors_strip_line_column_prefix() {
        assert_eq!(
            apps::pde_modeler::views::first_line(
                "Line 42, column 1: Mesh has 5804 nodes; time-dependent solves are limited to 4096. Increase Hmax."
            ),
            "Mesh has 5804 nodes; time-dependent solves are limited to 4096. Increase Hmax."
        );

        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_heat();
        h.app.model.mesh.hmax = "0.08".into();
        frame(&mut h, &ws, &ctx, vec![]);

        click(&mut h, &ws, &ctx, "Solve");
        wait_idle(&mut h, &ws, &ctx);

        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(h.app.info_is_error, "DOF guard error should be marked as error");
        assert!(
            msg.starts_with("Mesh has "),
            "expected message starting with 'Mesh has ', got {msg:?}"
        );
        assert!(
            !msg.contains("Line "),
            "GUI error must not contain script-location prefix 'Line ': {msg:?}"
        );
    }

    #[test]
    fn pde_modeler_8c5_clearing_rules_and_solving_state() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        h.app.model.boundary_conditions.remove(&2);
        frame_tall(&mut h, &ws, &ctx, vec![]);

        // Trigger missing BC error on Solve.
        click_tall(&mut h, &ws, &ctx, "Solve");
        assert!(h.app.info_is_error);
        assert!(
            h.app
                .info_message
                .as_deref()
                .is_some_and(|m| m.contains("Missing boundary condition")),
            "expected missing BC error, got {:?}",
            h.app.info_message
        );

        // Assigning the missing BC clears the error immediately (no second Solve).
        click_tall(&mut h, &ws, &ctx, "Edge:2:Dirichlet");
        assert!(
            !h.app.info_is_error && h.app.info_message.is_none(),
            "assigning missing BC must clear stale error, got {:?}",
            h.app.info_message
        );

        // Trigger invalid set formula error on Decompose, then fix formula -> error clears.
        type_into(&mut h, &ws, &ctx, "Set formula", "R1+Z9");
        click_tall(&mut h, &ws, &ctx, "Decompose");
        assert!(h.app.info_is_error);
        assert!(
            h.app
                .info_message
                .as_deref()
                .is_some_and(|m| m.contains("Z9")),
            "expected formula error naming Z9, got {:?}",
            h.app.info_message
        );
        type_into(&mut h, &ws, &ctx, "Set formula", "R1-C1");
        assert!(
            !h.app.info_is_error && h.app.info_message.is_none(),
            "fixing set formula must clear stale error, got {:?}",
            h.app.info_message
        );

        // Solve, export -> "Exported..." notice appears, then any model change clears it.
        h.app.load_plate_with_hole();
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Solve");
        wait_idle(&mut h, &ws, &ctx);
        let _ = click(&mut h, &ws, &ctx, "Export to workspace");
        assert!(
            h.message.starts_with("Exported "),
            "expected Exported notice, got {:?}",
            h.message
        );
        let (info_text, info_is_error) = h.app.info_line_state();
        assert!(
            !info_is_error && info_text.starts_with("Exported "),
            "info line must show the export notice, got {info_text:?} error={info_is_error}"
        );

        // Editing Hmax (a model change) clears the old Exported notice.
        h.app.model.mesh.hmax = "0.4".into();
        frame(&mut h, &ws, &ctx, vec![]);
        assert!(
            h.message.is_empty(),
            "old Exported notice must clear on model change, got {:?}",
            h.message
        );
        let (info_after, _) = h.app.info_line_state();
        assert!(
            !info_after.starts_with("Exported "),
            "info line must drop the Exported notice after a model change, got {info_after:?}"
        );

        // While solving, info_line_state must not say "Ready".
        h.app.load_plate_with_hole();
        h.app.test_solve_delay_ms = 5000;
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Solve");
        assert!(h.running());
        let (info_text, _) = h.app.info_line_state();
        assert_ne!(
            info_text, "Ready",
            "info line must not say Ready while solving"
        );
        assert!(h.app.controls.contains_key("Loading"));
        click(&mut h, &ws, &ctx, "Cancel");
        wait_idle(&mut h, &ws, &ctx);
    }

    #[test]
    fn pde_modeler_8c5_eigen_range_matches_drawn_mode_colorbar() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.model.add_rectangle(0.0, 0.0, 1.0, 1.0);
        h.app.set_pde_type(PdeType::Eigen);
        h.app.model.edge_ids = vec![1, 2, 3, 4];
        for e in 1..=4 {
            h.app
                .model
                .boundary_conditions
                .insert(e, BoundaryCondition::Dirichlet { u: "0".into() });
        }
        // Check shape-card numeric field width at 1200 px window.
        frame_sized(&mut h, &ws, &ctx, egui::vec2(1200.0, 800.0), vec![]);
        let x0_rect = h.app.controls["Shape:0:x0"];
        assert!(
            x0_rect.width() >= 70.0,
            "shape-card numeric field should be wide enough for 4 sig figs, got {}",
            x0_rect.width()
        );

        click(&mut h, &ws, &ctx, "Solve");
        wait_idle(&mut h, &ws, &ctx);

        let range_row = |h: &Host<PdeModeler>| -> String {
            h.app
                .results()
                .into_iter()
                .find(|(k, _)| *k == "Range")
                .map(|(_, v)| v)
                .expect("Range row should be present")
        };

        let clim0 = h
            .app
            .solution_view
            .as_ref()
            .expect("mode 0 view")
            .figure
            .axes
            .extras
            .color_limits();
        let range0 = range_row(&h);
        assert_eq!(
            range0,
            format!("{:.4} .. {:.4}", clim0[0], clim0[1]),
            "mode 1 Range must match its colourbar limits"
        );

        let eigs = h.app.eigenvalues.clone().expect("eigenvalues");
        assert!(eigs.len() >= 2, "expected at least 2 eigenmodes");
        h.app.set_eigen_index(1, &ctx);
        wait_idle(&mut h, &ws, &ctx);

        let clim1 = h
            .app
            .solution_view
            .as_ref()
            .expect("mode 1 view")
            .figure
            .axes
            .extras
            .color_limits();
        let range1 = range_row(&h);
        assert_eq!(
            range1,
            format!("{:.4} .. {:.4}", clim1[0], clim1[1]),
            "mode 2 Range must match its colourbar limits"
        );
        assert_ne!(
            range0, range1,
            "mode 1 and mode 2 should have distinct Range strings"
        );
    }

    // ---------------------------------------------------------------------
    // 8c whole-step review must-fix round

    fn decompose_tall(h: &mut Host<PdeModeler>, ws: &Snapshot, ctx: &egui::Context) {
        frame_tall(h, ws, ctx, vec![]);
        click_tall(h, ws, ctx, "Decompose");
        wait_decompose(h, ws, ctx);
    }

    fn wait_idle_tall(h: &mut Host<PdeModeler>, ws: &Snapshot, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            frame_tall(h, ws, ctx, vec![]);
            if !h.running()
                && !h.app.decompose_runner.running()
                && !h.app.plot_runner.running()
            {
                break;
            }
            assert!(Instant::now() < deadline, "PDE Modeler runner timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        frame_tall(h, ws, ctx, vec![]);
    }

    #[test]
    fn pde_modeler_review_stale_solve_dropped_when_f_edited() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        h.app.test_solve_delay_ms = 1500;
        frame_tall(&mut h, &ws, &ctx, vec![]);
        click_tall(&mut h, &ws, &ctx, "Solve");
        assert!(h.running(), "slow solve should still be running");
        // Type f = 5 while "Solving...".
        type_into(&mut h, &ws, &ctx, "PDE:f", "5");
        assert_eq!(h.app.model.coeffs.f, "5");
        wait_idle_tall(&mut h, &ws, &ctx);
        assert!(h.app.nodal_solution.is_none(), "f = 1 result must not land on f = 5");
        assert!(h.app.solution_view.is_none());
        assert!(h.app.max_u.is_none());
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(
            msg.contains("discarded because the model changed"),
            "info line must say why: {msg:?}"
        );
        assert!(!h.app.info_is_error);

        // Solving again gives the f = 5 answer (report: 0.8866).
        h.app.test_solve_delay_ms = 0;
        frame_tall(&mut h, &ws, &ctx, vec![]);
        click_tall(&mut h, &ws, &ctx, "Solve");
        wait_idle_tall(&mut h, &ws, &ctx);
        let max_u = h.app.max_u.expect("max u after re-solve");
        assert!((max_u - 0.8866).abs() < 1e-3, "max u {max_u}");
    }

    #[test]
    fn pde_modeler_review_stale_mesh_dropped_when_formula_changes() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.model.add_rectangle(-1.0, -1.0, 2.0, 2.0);
        h.app.model.add_circle(0.0, 0.0, 0.4);
        h.app.model.set_formula = "R1".into();
        h.app.formula_edited = true;
        h.app.test_mesh_delay_ms = 1500;
        frame_tall(&mut h, &ws, &ctx, vec![]);
        click_tall(&mut h, &ws, &ctx, "Mesh");
        assert!(h.running(), "slow mesh should still be running");
        // R1 -> R1-C1 before the mesh lands.
        type_into(&mut h, &ws, &ctx, "Set formula", "R1-C1");
        assert_eq!(h.app.model.set_formula, "R1-C1");
        wait_idle_tall(&mut h, &ws, &ctx);
        assert!(h.app.mesh_view.is_none(), "old R1 mesh must not be shown");
        assert!(h.app.node_count.is_none());
        assert!(h.app.model.edge_ids.is_empty(), "old R1 edges must not be adopted");
        assert!(h.app.canvas.decomposed_edges.is_empty());
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(msg.contains("Mesh result discarded because the model changed"), "{msg:?}");
        h.app.model.boundary_conditions.clear();
        assert!(h.app.model.validate().is_err(), "no BCs assigned for R1-C1 yet");
    }

    #[test]
    fn pde_modeler_review_interior_edges_take_no_condition() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.model.add_rectangle(-1.0, -1.0, 2.0, 2.0);
        h.app.model.add_circle(0.0, 0.0, 0.4);
        assert_eq!(h.app.model.set_formula, "R1+C1");
        decompose_tall(&mut h, &ws, &ctx);
        assert_eq!(h.app.canvas.decomposed_edges.len(), 8);
        let interior: Vec<usize> = h
            .app
            .canvas
            .decomposed_edges
            .iter()
            .filter(|e| !e.boundary)
            .map(|e| e.id)
            .collect();
        assert_eq!(interior, vec![5, 6, 7, 8]);
        assert_eq!(h.app.model.edge_ids, vec![1, 2, 3, 4]);
        assert!(!h.app.controls.contains_key("Edge:5:Dirichlet"), "interior edge listed");

        // Interior edges are not clickable on the canvas.
        click_tall(&mut h, &ws, &ctx, "Tool:Select");
        let e5 = h.app.canvas.decomposed_edges[4].label_pos;
        let pos = h.app.canvas.view.data_to_screen(e5);
        click_pos(&mut h, &ws, &ctx, pos);
        assert_ne!(h.app.canvas.selected_edge, Some(5));

        // u = 0 on E1-E4 only; "Apply to all unassigned" leaves the circle free.
        h.app.canvas.selected_edge = None;
        click_tall(&mut h, &ws, &ctx, "Apply to all unassigned");
        let ids: Vec<usize> = h.app.model.boundary_conditions.keys().copied().collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        h.app.model.validate().expect("BCs on outer edges are enough");

        click_tall(&mut h, &ws, &ctx, "Solve");
        wait_idle_tall(&mut h, &ws, &ctx);
        assert!(!h.app.info_is_error, "{:?}", h.app.info_message);
        let max_u = h.app.max_u.expect("max u");
        assert!((max_u - 0.2947).abs() < 1e-3, "max u {max_u}, expected 0.2947");
    }

    #[test]
    fn pde_modeler_review_time_fields_refuse_and_snap_back() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_heat();
        frame_tall(&mut h, &ws, &ctx, vec![]);

        for (control, text, needle) in [
            ("PDE:steps", "1", "Steps"),
            ("PDE:steps", "0", "Steps"),
            ("PDE:steps", "2.5", "Steps"),
            ("PDE:t1", "0,5", "t1"),
            ("PDE:t1", "", "t1"),
            ("PDE:t1", "Inf", "t1"),
            ("PDE:t1", "NaN", "t1"),
            ("PDE:t1", "-1", "t1 (-1) must exceed t0"),
            ("PDE:t0", "1", "must exceed t0 (1)"),
        ] {
            type_into(&mut h, &ws, &ctx, control, text);
            let err = h.app.time_field_error.clone().unwrap_or_default();
            assert!(err.contains(needle), "{control} {text:?}: {err:?}");
            let msg = h.app.info_message.clone().unwrap_or_default();
            assert_eq!(msg, err, "{control} {text:?}: info line");
            assert!(h.app.info_is_error);
            assert_eq!(h.app.model.time_range.t0, 0.0);
            assert_eq!(h.app.model.time_range.t1, 1.0);
            assert_eq!(h.app.model.time_range.steps, 21);
            // While the refused text is shown, Solve and Export refuse.
            assert!(h.app.script(&ws).is_err());
            apps::pde_modeler::start_solve(&mut h, &ctx, &ws);
            assert_eq!(h.app.solve_count, 0, "{control} {text:?}: solve ran");
            assert!(h.app.info_is_error);
            // ... and the fields show the model values again.
            assert_eq!(h.app.time_t0_draft, "0");
            assert_eq!(h.app.time_t1_draft, "1");
            assert_eq!(h.app.time_steps_draft, "21");
        }

        // Report case: Steps 1, then t1 0,5, then Solve. Leaving a field
        // snaps it back, so Solve runs with exactly what the panel shows.
        type_into(&mut h, &ws, &ctx, "PDE:steps", "1");
        assert!(h.app.time_field_error.is_some());
        type_into(&mut h, &ws, &ctx, "PDE:t1", "0,5");
        assert_eq!(h.app.time_steps_draft, "21", "Steps snaps back on leaving it");
        assert!(h.app.time_field_error.is_some());
        click_tall(&mut h, &ws, &ctx, "Solve");
        assert_eq!(h.app.time_t1_draft, "1", "t1 snaps back on leaving it");
        wait_idle_tall(&mut h, &ws, &ctx);
        let shown_t1: f64 = h.app.time_t1_draft.parse().unwrap();
        let shown_steps: usize = h.app.time_steps_draft.parse().unwrap();
        let tlist = h.app.tlist.clone().expect("tlist");
        assert_eq!(tlist.len(), shown_steps);
        assert_eq!(*tlist.last().unwrap(), shown_t1);

        // A valid entry is applied.
        type_into(&mut h, &ws, &ctx, "PDE:t1", "0.5");
        assert!(h.app.time_field_error.is_none());
        assert_eq!(h.app.model.time_range.t1, 0.5);
    }

    #[test]
    fn pde_modeler_review_field_text_must_be_one_expression() {
        use matrust::engine::pde_app_model::check_expression;
        // Report injection input.
        let mut model = PdeAppModel::plate_with_hole();
        model.coeffs.f = "1); rev_injected = 42; rev_x = (1".into();
        let err = model.script().unwrap_err();
        assert!(err.starts_with("Invalid f:"), "{err}");
        for bad in ["1; 2", "1, 2", "1\n2", "1) + (2", "1 % note", "x = 1", "(1", "1 ..."] {
            assert!(check_expression("f", bad).is_err(), "{bad:?} accepted");
        }
        for good in ["@(l,s) 1+l.x.^2", "[1;2]", "[-Inf 100]", "2*pi", "", " 0 ", "'abc'"] {
            check_expression("f", good).unwrap_or_else(|e| panic!("{good:?}: {e}"));
        }
        let mut ok = PdeAppModel::plate_with_hole();
        ok.coeffs.c = "@(l,s) 1+l.x.^2".into();
        assert!(ok.script().unwrap().contains("'c', @(l,s) 1+l.x.^2,"));

        // BC, u0 and eigen-range text is checked too.
        let mut bc = PdeAppModel::plate_with_hole();
        bc.boundary_conditions
            .insert(1, BoundaryCondition::Dirichlet { u: "0); rev_bc = 1; (0".into() });
        assert!(bc.script().unwrap_err().contains("u on edge E1"));
        let mut heat = PdeAppModel::plate_heat();
        heat.u0 = "0; rev_u0 = 1".into();
        assert!(heat.script().unwrap_err().starts_with("Invalid u0:"));
        let mut eig = PdeAppModel::plate_with_hole();
        eig.pde_type = PdeType::Eigen;
        eig.eigen_range = "[0 10]); rev_eig = 1; ([0 10]".into();
        assert!(eig.script().unwrap_err().starts_with("Invalid Range:"));

        // In the app: clear message, no solve, nothing in the workspace.
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        h.app.load_plate_with_hole();
        frame_tall(&mut h, &ws, &ctx, vec![]);
        type_into(&mut h, &ws, &ctx, "PDE:f", "1); rev_injected = 42; rev_x = (1");
        assert!(h.app.controls.contains_key("PDE:expression_error"));
        click_tall(&mut h, &ws, &ctx, "Solve");
        assert_eq!(h.app.solve_count, 0);
        let msg = h.app.info_message.clone().unwrap_or_default();
        assert!(msg.starts_with("Invalid f: enter one MATLAB expression"), "{msg}");
        assert!(h.app.script(&ws).is_err());
    }

    #[test]
    fn pde_modeler_review_dof_estimate_does_not_refuse_meshes_that_fit() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        // (shapes, formula, Hmax, real node count on the graded (Hgrad) mesher)
        let cases: [(&str, f64, usize); 2] = [("C1-C2", 0.05, 3490), ("R1*C1", 0.1, 569)];
        for (formula, hmax, expected_nodes) in cases {
            let mut h = Host::new(PdeModeler::default());
            if formula == "C1-C2" {
                h.app.model.add_circle(0.0, 0.0, 1.0);
                h.app.model.add_circle(0.0, 0.0, 0.9);
            } else {
                h.app.model.add_rectangle(-10.0, -10.0, 20.0, 20.0);
                h.app.model.add_circle(0.0, 0.0, 0.4);
            }
            h.app.model.set_formula = formula.into();
            h.app.formula_edited = true;
            h.app.set_pde_type(PdeType::Parabolic);
            h.app.model.mesh.hmax = hmax.to_string();
            h.app.model.time_range.steps = 3;
            decompose_tall(&mut h, &ws, &ctx);
            h.app.apply_bc_to_all_unassigned();
            frame_tall(&mut h, &ws, &ctx, vec![]);
            // Solve passes the pre-check (the old area estimate refused it).
            click_tall(&mut h, &ws, &ctx, "Solve");
            assert_eq!(h.app.solve_count, 1, "{formula}: {:?}", h.app.info_message);
            assert!(h.running(), "{formula}: {:?}", h.app.info_message);
            apps::pde_modeler::cancel_running(&mut h);
            wait_idle_tall(&mut h, &ws, &ctx);
            // The real mesh fits under the limit.
            click_tall(&mut h, &ws, &ctx, "Mesh");
            wait_idle_tall(&mut h, &ws, &ctx);
            assert!(!h.app.info_is_error, "{formula}: {:?}", h.app.info_message);
            let nodes = h.app.node_count.expect("node count after Mesh");
            #[cfg(not(windows))]
            assert_eq!(nodes, expected_nodes, "{formula}");
            // Windows libm can move the graded circle mesh by a few nodes.
            #[cfg(windows)]
            assert!(nodes.abs_diff(expected_nodes) * 50 <= expected_nodes, "{formula}: {nodes}");
            assert!(h.app.mesh_view.is_some(), "{formula}");
            // After Mesh, Solve is not refused by the real-count pre-check either.
            click_tall(&mut h, &ws, &ctx, "Solve");
            assert_eq!(h.app.solve_count, 2, "{formula}: {:?}", h.app.info_message);
            apps::pde_modeler::cancel_running(&mut h);
            wait_idle_tall(&mut h, &ws, &ctx);
        }
    }

    #[test]
    fn pde_modeler_review_export_keeps_user_gd_sf_ns_tlist() {
        let ctx = egui::Context::default();
        let ws = Snapshot::default();
        let mut h = Host::new(PdeModeler::default());
        frame(&mut h, &ws, &ctx, vec![]);
        click(&mut h, &ws, &ctx, "Heat in a plate");
        wait_idle(&mut h, &ws, &ctx);
        let Some(toolkits::Action::ExportScript(code)) =
            click(&mut h, &ws, &ctx, "Export to workspace")
        else {
            panic!("expected ExportScript action");
        };
        let mut eng = matrust::Engine::default();
        eng.execute("gd = 11; sf = 'mine'; ns = [1 2 3]; tlist = 44;").unwrap();
        eng.execute(&code).expect("export runs");
        eng.execute(
            "kept = isequal(gd, 11) + isequal(sf, 'mine') + isequal(ns, [1 2 3]) + isequal(tlist, 44);\n\
             leftovers = exist('pdem_gd', 'var') + exist('pdem_sf', 'var') + exist('pdem_ns', 'var') + exist('pdem_tlist', 'var');\n\
             ok_model = isa(model, 'pde.PDEModel'); ok_res = size(results.NodalSolution, 2);",
        )
        .unwrap();
        let num = |eng: &matrust::Engine, n: &str| match eng.array(n).unwrap() {
            matrust::Array::Numeric(m) => m.data()[0],
            matrust::Array::Logical(m) => f64::from(m.data()[0]),
            other => panic!("{n}: {other:?}"),
        };
        assert_eq!(num(&eng, "kept"), 4.0, "all four user variables must survive Export");
        assert_eq!(num(&eng, "leftovers"), 0.0, "pdem_ helpers must not stay behind");
        assert_eq!(num(&eng, "ok_res"), 21.0);

        // Names the script uses internally are refused as export names; `gd` is fine now.
        h.app.export_names = ["pdem_gd".into(), "results".into()];
        assert!(h.app.validate_export_names().is_err());
        h.app.export_names = ["model".into(), "pdem_tlist".into()];
        assert!(h.app.validate_export_names().is_err());
        h.app.export_names = ["gd".into(), "tlist".into()];
        h.app.validate_export_names().unwrap();
        let script = h.app.script(&ws).unwrap();
        let mut eng2 = matrust::Engine::default();
        eng2.execute(&script).expect("script with model named gd runs");
        assert!(eng2.array("gd").is_some() && eng2.array("tlist").is_some());
    }

    #[test]
    fn scrolled_off_solution_does_not_paint_colorbar_over_inputs() {
        use apps::pde_modeler::views;
        let mut engine = matrust::Engine::default();
        engine
            .execute("plot([0 1],[0 1]); colorbar; clim([0 0.05]);")
            .unwrap();
        let mut view = views::prepare_figure(engine.figure.unwrap(), &false.into(), 1).unwrap();
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            ctx.set_theme(theme);
            for top in [80., 600., 700.] {
                let body = egui::Rect::from_min_size(egui::pos2(16., top), egui::vec2(868., 380.));
                let out = crate::headless::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900., 650.),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let mut plot = ui.new_child(egui::UiBuilder::new().max_rect(body));
                        views::show_prepared_figure(
                            &mut plot,
                            &mut view,
                            egui::Id::new("pde-test"),
                        );
                    },
                );
                let mut ticks = 0;
                for s in &out.shapes {
                    if let egui::Shape::Text(t) = &s.shape
                        && t.galley.text() == "0.05"
                    {
                        ticks += 1;
                        assert!(body.contains_rect(s.clip_rect));
                    }
                }
                if top == 80. {
                    assert!(ticks > 0);
                }
                if top >= 650. {
                    assert_eq!(ticks, 0);
                }
            }
        }
    }
}
