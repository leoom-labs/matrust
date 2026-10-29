// Real fresh-start fixtures intentionally do not use legacy_sample_desktop.
fn command_first_key(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2,
    key: egui::Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        frame_at(app, ctx, size, vec![egui::Event::Key {
            key, physical_key: Some(key), pressed, repeat: false, modifiers,
        }]);
    }
}
fn command_first_prompt_count(output: &egui::FullOutput) -> usize {
    output.shapes.iter().filter(|shape| text_position(&shape.shape, ">>").is_some()).count()
}

#[test]
fn command_first_fresh_real_fields_fit_both_themes_and_retain_one_command_input() {
    for theme in [session::DesktopTheme::Light, session::DesktopTheme::Dark] {
        for size in [egui::vec2(960.,650.), egui::vec2(1440.,900.)] {
            for scale in [75,100,150] {
                let ctx = egui::Context::default();
                let mut app = Workspace::with_context(ctx.clone(), true);
                app.layout.theme = Some(theme); app.apply_desktop_theme(&ctx);
                app.set_desktop_scale(&ctx, scale);
                let original = app.documents[0].id;
                assert!(app.documents[0].path.is_none() && app.documents[0].text.is_empty());
                assert!(!app.documents[0].dirty && app.central_command());
                assert!(app.workspace_on_left());
                for _ in 0..3 { frame_at(&mut app,&ctx,size,vec![]); }
                let output = frame_at(&mut app,&ctx,size,vec![]);
                assert_eq!(command_first_prompt_count(&output),1);
                let input = ctx.read_response(egui::Id::new("command_input")).unwrap();
                assert!(ctx.content_rect().contains_rect(input.rect), "scale={scale} size={size:?} input={:?}",input.rect);
                assert!(ctx.read_response(editor::id(original)).is_none());
                assert!(egui::containers::panel::PanelState::load(&ctx,egui::Id::new("console")).is_none());
                assert_eq!(ctx.global_style().visuals.dark_mode,theme==session::DesktopTheme::Dark);
                pointer_frame(&mut app,&ctx,size,input.rect.center(),true);
                pointer_frame(&mut app,&ctx,size,input.rect.center(),false);
                frame_at(&mut app,&ctx,size,vec![egui::Event::Text("fresh_value=17;".into())]);
                assert_eq!(app.command,"fresh_value=17;");
                command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::NONE);
                finish(&mut app);
                assert_eq!(app.variables["fresh_value"].as_numeric().unwrap().get(0,0),17.);
                assert!(app.central_command());
                assert_eq!(app.documents.len(),1); assert_eq!(app.documents[0].id,original);
                assert!(app.documents[0].text.is_empty());
                assert!(app.console.iter().any(|line|line==">> fresh_value=17;"));
                assert_eq!(command_first_prompt_count(&frame_at(&mut app,&ctx,size,vec![])),1);
            }
        }
    }
}

#[test]
fn command_first_explicit_new_shortcuts_typing_and_close_keep_real_document_intent() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let size=egui::vec2(960.,650.);
    app.layout.console_height=Some(155.);
    for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
    command_first_key(&mut app,&ctx,size,egui::Key::N,egui::Modifiers::CTRL);
    assert_eq!(app.documents.len(),1);assert!(!app.central_command());
    let document=app.documents[app.active].id;
    assert!(app.layout.center.unwrap().editor_revealed);
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL|egui::Modifiers::SHIFT);
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("keep_editor=23;".into())]);
    assert_eq!(app.documents[app.active].text,"keep_editor=23;");
    assert!(app.documents[app.active].dirty);
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL);
    assert!(app.central_command());
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("keep_command=29;".into())]);
    assert_eq!(app.command,"keep_command=29;");
    assert_eq!(app.documents[app.active].text,"keep_editor=23;");
    assert_eq!(app.layout.console_height,Some(155.));
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL|egui::Modifiers::SHIFT);
    assert!(ctx.memory(|m|m.has_focus(editor::id(document))));
    assert_eq!(command_first_prompt_count(&frame_at(&mut app,&ctx,size,vec![])),1);
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::CTRL);
    finish(&mut app);
    assert_eq!(app.variables["keep_editor"].as_numeric().unwrap().get(0,0),23.);
    assert!(!app.variables.contains_key("keep_command"));
    assert!(!app.central_command());
    app.show_command_center();
    app.request_script_close(app.active);
    assert!(app.file_action_pending(),"Hidden dirty script still participates in close");
    click_label_at(&mut app,&ctx,size,"Cancel");
    assert_eq!(app.documents[app.active].id,document);
    assert!(app.documents[app.active].dirty);
    assert!(app.central_command());
}

#[test]
fn command_first_save_and_close_do_not_target_startup_placeholder() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let size=egui::vec2(960.,650.);let original=app.documents[0].id;
    app.document_save.intercepted=Some(Vec::new());
    for key in [egui::Key::S,egui::Key::W] {
        command_first_key(&mut app,&ctx,size,key,egui::Modifiers::CTRL);
        assert!(!app.file_action_pending()&&!app.busy);
        assert_eq!(app.documents.len(),1);assert_eq!(app.documents[0].id,original);
        assert!(app.document_save.intercepted.as_ref().unwrap().is_empty());
    }
    command_first_key(&mut app,&ctx,size,egui::Key::N,egui::Modifiers::CTRL);
    let explicit_empty=app.documents[app.active].id;
    document_save_key(&mut app,&ctx,size,false);
    assert_eq!(app.document_save.intercepted.as_ref().unwrap(),&vec![document_save::Request::Script{document:explicit_empty,save_as:false}]);
    app.remove_document(app.active);
    assert!(app.central_command());assert!(!app.layout.center.unwrap().editor_revealed);
    assert_eq!(app.documents.len(),1);assert!(!app.documents[0].dirty);
}

#[test]
fn command_first_worker_open_and_failed_open_preserve_raw_text_and_surface() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("real_empty.m");
    std::fs::write(&path,"").unwrap();
    app.open_path(path.clone());finish_open_queue(&mut app,&ctx);
    assert!(!app.central_command());assert!(app.layout.center.unwrap().editor_revealed);
    assert_eq!(app.documents.len(),1,"First Open replaces only the explicit startup placeholder");
    assert!(app.documents[app.active].text.is_empty());
    assert_eq!(app.documents[app.active].path.as_deref(),Some(std::fs::canonicalize(&path).unwrap().as_path()));
    let document=app.documents[app.active].id;
    app.show_command_center();
    app.open_path(dir.path().join("missing.m"));finish_open_queue(&mut app,&ctx);
    assert!(app.central_command());assert_eq!(app.documents[app.active].id,document);
    let raw="% retained CR\ranswer=31;\r";
    let second=dir.path().join("logical_lines.m");std::fs::write(&second,raw).unwrap();
    app.open_path(second);finish_open_queue(&mut app,&ctx);
    assert!(!app.central_command());assert_eq!(app.documents[app.active].text,raw);
    assert!(!app.documents[app.active].dirty);
    app.run_document(None);finish(&mut app);
    assert_eq!(app.variables["answer"].as_numeric().unwrap().get(0,0),31.);
}

#[test]
fn command_first_changed_figure_reveals_but_numeric_replies_do_not_steal_center() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let size=egui::vec2(960.,650.);let doc=app.documents[0].id;
    app.run("x=[1 2 3];alias=x;plot(x,x);".into());finish(&mut app);
    assert!(app.central_command()&&app.figure_column(),"the figure takes the right column");
    assert!(!app.layout.center.unwrap().editor_revealed);
    let figure=app.figure.as_ref().unwrap().clone();let data=app.variables["x"].clone();
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL);
    app.run("numeric_only=7;".into());finish(&mut app);
    assert!(app.central_command());assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("next_command".into())]);
    assert_eq!(app.command,"next_command");
    app.run("title('Changed actual figure');".into());finish(&mut app);
    assert!(app.central_command()&&app.figure_column());assert_eq!(app.figure.as_ref().unwrap().title,"Changed actual figure");
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("_kept".into())]);
    assert_eq!(app.command,"next_command_kept","Publication does not steal command typing focus");
    assert!(data.same_snapshot(&app.variables["x"]));assert!(data.same_snapshot(&app.variables["alias"]));
    assert_eq!(app.documents[0].id,doc);assert!(app.documents[0].text.is_empty());
    assert_eq!(command_first_prompt_count(&frame_at(&mut app,&ctx,size,vec![])),1);
}

#[test]
fn command_first_restore_reset_and_explicit_empty_editor_keep_session_work() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    app.new_document("");let empty=app.documents[app.active].id;
    let saved=app.snapshot();
    let mut restored=Workspace::with_context(ctx.clone(),true);restored.restore_session(saved);
    assert!(!restored.central_command());assert!(restored.layout.center.unwrap().editor_revealed);
    assert_eq!(restored.documents[restored.active].id,empty);
    restored.documents[restored.active].text="recover=41;".into();restored.documents[restored.active].changed();
    restored.layout.theme=Some(session::DesktopTheme::Dark);
    let text=restored.documents[restored.active].text.clone();
    restored.reset_layout(&ctx);
    assert!(restored.central_command());assert!(restored.workspace_on_left());
    assert!(restored.layout.center.unwrap().editor_revealed);
    assert_eq!(restored.layout.theme,Some(session::DesktopTheme::Dark));
    assert_eq!(restored.documents[restored.active].text,text);assert!(restored.documents[restored.active].dirty);
    let mut saved=restored.snapshot();saved.layout.center=None;saved.layout.mode=layout::PaneMode::Split;
    let exact=saved.layout;restored.restore_session(saved);
    assert_eq!(restored.layout,exact);assert!(!restored.central_command());
    assert!(restored.documents[restored.active].dirty);
}

#[test]
fn command_first_two_stage_figure_open_requires_success_and_current_activation_intent() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let dir=tempfile::tempdir().unwrap();let invalid=dir.path().join("broken.mfig");
    std::fs::write(&invalid,b"not a figure").unwrap();
    app.open_path(invalid);finish_open_queue(&mut app,&ctx);
    assert!(app.central_command());assert!(app.figure.is_none());
    // Produce a genuine file through the ordinary worker, then use the real
    // two-stage Open preparation while explicitly suppressing selection.
    app.run("plot([1 2],[3 4]);title('Saved figure');".into());finish(&mut app);
    let path=dir.path().join("real.mfig");app.submit(Job::SaveFigure(path.clone(),None));finish(&mut app);
    let figure=app.figure.as_ref().unwrap().clone();
    app.show_command_center();
    app.open_path(path.clone());
    app.document_save.activate(document_save::Pane::Script);
    finish_open_queue(&mut app,&ctx);
    assert!(app.central_command());assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
    // An unchanged explicitly reopened figure still becomes visible when the
    // user has not changed the selection during preparation.
    app.open_path(path.clone());finish_open_queue(&mut app,&ctx);
    assert!(app.central_command()&&app.figure_column(),"the figure takes the right column");
    // Second-stage intent invalidation is independent of first-stage preflight.
    app.show_command_center();
    app.complete_open(Ok(open_files::Prepared::File(path,open_files::Kind::Figure)));
    assert!(app.busy);app.show_command_center();finish(&mut app);
    assert!(app.central_command());assert!(figure.same_snapshot(app.figure.as_ref().unwrap()));
}

#[test]
fn command_first_switch_with_discard_keeps_one_real_input_and_completion_cursor() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let size=egui::vec2(960.,650.);app.new_document("");
    app.command="sin".into();app.history.push("earlier=1;".into());
    for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
    // Layout > Command Window only: no tab row, the switch is a menu choice.
    click_label_at(&mut app,&ctx,size,"Menu");
    click_label_at(&mut app,&ctx,size,"Layout");
    for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
    let point=frame_at(&mut app,&ctx,size,vec![]).shapes.iter().rev()
        .find_map(|s|text_position(&s.shape,"Command Window only")).unwrap();
    pointer_frame(&mut app,&ctx,size,point,true);
    let mut passes=0;
    let output=crate::headless::run(&ctx,egui::RawInput{
        screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,size)),
        events:vec![egui::Event::PointerMoved(point),egui::Event::PointerButton{
            pos:point,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}],
        ..Default::default()
    },|root|{passes+=1;app.ui(root);if passes==1{root.ctx().request_discard("center transition retry");}});
    assert!(passes>=2);assert!(app.central_command());assert_eq!(command_first_prompt_count(&output),1);
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL);
    focus_command(&app,&ctx);
    command_first_key(&mut app,&ctx,size,egui::Key::Tab,egui::Modifiers::NONE);
    assert!(app.command.starts_with("sin"));
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    assert_eq!(app.command,"earlier=1;");
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL|egui::Modifiers::SHIFT);
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL);
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("% kept".into())]);
    assert_eq!(app.command,"earlier=1;% kept");
    assert!(app.documents[app.active].text.is_empty());
}

fn flowing_command_visible_text(output: &egui::FullOutput, label: &str) -> egui::Rect {
    fn visit(shape:&egui::epaint::Shape,label:&str,clip:egui::Rect,result:&mut Vec<egui::Rect>) {
        match shape {
            egui::epaint::Shape::Text(t) if t.galley.text()==label => {
                let ink=shape.visual_bounding_rect();
                if clip.contains_rect(ink) {result.push(ink);}
            }
            egui::epaint::Shape::Vec(shapes)=>for shape in shapes {visit(shape,label,clip,result);},
            _=>{}
        }
    }
    let mut found=Vec::new();
    for shape in &output.shapes {visit(&shape.shape,label,shape.clip_rect,&mut found);}
    assert_eq!(found.len(),1,"Expected one completely visible {label}");found[0]
}

#[test]
fn command_prompt_flows_after_short_output_in_both_center_and_legacy_panels() {
    for theme in [session::DesktopTheme::Light,session::DesktopTheme::Dark] {
        for size in [egui::vec2(960.,650.),egui::vec2(1440.,900.)] {
            for legacy in [false,true] {
                let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
                if legacy {app.layout.center=None;app.layout.console_height=Some(200.);}
                app.layout.theme=Some(theme);app.apply_desktop_theme(&ctx);
                app.console.clear();
                // A command has run before: no welcome block above the prompt.
                app.history.push("earlier=1;".into());
                for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
                let output=frame_at(&mut app,&ctx,size,vec![]);
                // The header's Clear link; the input sits at the bottom of the card.
                let header=flowing_command_visible_text(&output,"Clear");
                let prompt=flowing_command_visible_text(&output,">>");
                assert!(prompt.top()>header.top(),"header={header:?} prompt={prompt:?}");
                let empty_y=ctx.read_response(egui::Id::new("command_input")).unwrap().rect.top();
                app.console.push("first short output".into());app.console.push("second short output".into());
                for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
                let output=frame_at(&mut app,&ctx,size,vec![]);
                let last=flowing_command_visible_text(&output,"second short output");
                let prompt=flowing_command_visible_text(&output,">>");
                let input=ctx.read_response(egui::Id::new("command_input")).unwrap();
                assert_eq!(input.rect.top(),empty_y,"the input stays at the card bottom");
                assert!(prompt.top()>last.top() && last.bottom()<=input.rect.top());
                assert!(ctx.content_rect().contains_rect(input.rect));
                assert_eq!(command_first_prompt_count(&output),1);
                assert_eq!(ctx.global_style().visuals.dark_mode,theme==session::DesktopTheme::Dark);
            }
        }
    }
}

// Explicit focus/history uses egui's configured scroll animation. Advance its
// actual input clock through the maximum duration, then paint the updated offset.
// This is not a wall-clock sleep or a change to production scrolling behavior.
fn settle_flowing_command_scroll(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) {
    let duration = ctx.global_style().scroll_animation.duration.max;
    assert!(duration.is_finite() && (0.0..=1.0).contains(&duration));
    let started = ctx.input(|input| input.time);
    for _ in 0..120 {
        frame_at(app, ctx, size, vec![]);
        if ctx.input(|input| input.time) - started >= f64::from(duration) {
            // ScrollArea creates this frame's content rect before advancing its
            // animated offset; retain two final paints for the settled geometry.
            for _ in 0..2 { frame_at(app, ctx, size, vec![]); }
            return;
        }
    }
    panic!("Headless input clock did not advance through the bounded scroll animation");
}

#[test]
fn flowing_command_preserves_manual_scroll_and_explicit_focus_history_completion_execution() {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("flowing_command_scroll_diagnostic"), true));
    let size=egui::vec2(960.,650.);let document=app.documents[0].id;
    app.console=(0..160).map(|i|format!("previous output {i}")).collect();
    app.history.push("flowing_answer=61;".into());
    for _ in 0..8 {frame_at(&mut app,&ctx,size,vec![]);}
    let bottom=ctx.read_response(egui::Id::new("command_input")).unwrap().rect;
    assert!(ctx.content_rect().contains_rect(bottom));
    pointer_frame(&mut app,&ctx,size,bottom.center(),true);
    pointer_frame(&mut app,&ctx,size,bottom.center(),false);
    let point=bottom.center()-egui::vec2(0.,80.);
    frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(point),egui::Event::MouseWheel {
        unit:egui::MouseWheelUnit::Point,delta:egui::vec2(0.,100000.),modifiers:egui::Modifiers::NONE,phase:egui::TouchPhase::Move,
    }]);
    for _ in 0..16 {frame_at(&mut app,&ctx,size,vec![]);}
    let scrolled=ctx.read_response(egui::Id::new("command_input")).unwrap().rect;
    assert_eq!(scrolled,bottom,"the prompt stays at the card bottom while the history scrolls");
    // Finish the manual gesture before testing output retention and explicit focus.
    frame_at(&mut app,&ctx,size,vec![egui::Event::MouseWheel {
        unit:egui::MouseWheelUnit::Point,delta:egui::Vec2::ZERO,modifiers:egui::Modifiers::NONE,phase:egui::TouchPhase::End,
    }]);
    app.console.push("new output while reading history".into());
    for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
    assert!(ctx.content_rect().contains_rect(ctx.read_response(egui::Id::new("command_input")).unwrap().rect));
    // Explicit Ctrl+0 focuses the prompt even after a manual scroll.
    pane_focus_key(&mut app,&ctx,size,egui::Modifiers::CTRL);
    settle_flowing_command_scroll(&mut app,&ctx,size);
    assert!(ctx.content_rect().contains_rect(ctx.read_response(egui::Id::new("command_input")).unwrap().rect));
    // History/completion retain their existing keyboard and cursor behavior.
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    settle_flowing_command_scroll(&mut app,&ctx,size);
    assert_eq!(app.command,"flowing_answer=61;");
    assert!(ctx.content_rect().contains_rect(ctx.read_response(egui::Id::new("command_input")).unwrap().rect));
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::NONE);finish(&mut app);
    assert_eq!(app.variables["flowing_answer"].as_numeric().unwrap().data(),&[61.]);
    assert_eq!(app.documents[0].id,document);assert!(app.documents[0].text.is_empty());
    app.command="flowing_ans".into();
    focus_command(&app,&ctx);
    command_first_key(&mut app,&ctx,size,egui::Key::Tab,egui::Modifiers::NONE);
    assert_eq!(app.command,"flowing_answer");
    assert!(ctx.memory(|m|m.has_focus(egui::Id::new("command_input"))));
    app.stop_work();
    assert_eq!(app.command,"flowing_answer");assert_eq!(app.documents[0].id,document);
}

fn multiline_fixture() -> (egui::Context, Workspace, egui::Vec2) {
    let ctx=egui::Context::default();let mut app=Workspace::with_context(ctx.clone(),true);
    let size=egui::vec2(960.,650.);
    for _ in 0..3 {frame_at(&mut app,&ctx,size,vec![]);}
    focus_command(&app,&ctx);
    frame_at(&mut app,&ctx,size,vec![]);
    (ctx,app,size)
}
fn multiline_type(app:&mut Workspace,ctx:&egui::Context,size:egui::Vec2,text:&str) {
    frame_at(app,ctx,size,vec![egui::Event::Text(text.into())]);
}

#[test]
fn multiline_incomplete_enter_inserts_newline_and_complete_block_runs_once() {
    let (ctx,mut app,size)=multiline_fixture();
    let base=app.console.len();
    multiline_type(&mut app,&ctx,size,"if 1");
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::NONE);
    assert_eq!(app.command,"if 1\n");assert!(!app.busy);assert_eq!(app.console.len(),base);
    multiline_type(&mut app,&ctx,size,"disp(5)");
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::NONE);
    assert_eq!(app.command,"if 1\ndisp(5)\n");assert_eq!(app.console.len(),base);
    multiline_type(&mut app,&ctx,size,"end");
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::NONE);
    finish(&mut app);
    assert!(app.command.is_empty());
    assert_eq!(app.console.iter().filter(|l|l.trim()=="5").count(),1,"{:?}",app.console);
    assert_eq!(app.console[base..base+3],[">> if 1".to_string(),"disp(5)".to_string(),"end".to_string()]);
    assert_eq!(app.history.entries.last().unwrap(),"if 1\ndisp(5)\nend");
    assert_eq!(command_first_prompt_count(&frame_at(&mut app,&ctx,size,vec![])),1);
}

#[test]
fn multiline_shift_enter_always_inserts_newline() {
    let (ctx,mut app,size)=multiline_fixture();
    let base=app.console.len();
    multiline_type(&mut app,&ctx,size,"x=1;");
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::SHIFT);
    assert_eq!(app.command,"x=1;\n");assert!(!app.variables.contains_key("x"));assert_eq!(app.console.len(),base);
}

#[test]
fn multiline_arrows_recall_history_only_on_first_and_last_line() {
    let (ctx,mut app,size)=multiline_fixture();
    app.history.push("a=1;".into());
    app.history.push("for k=1:2\nb=k;\nend".into());
    // Up on a single (first) line recalls the whole block.
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    assert_eq!(app.command,"for k=1:2\nb=k;\nend");
    // Cursor is now on the last line: Up moves the cursor, history stays.
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    assert_eq!(app.command,"for k=1:2\nb=k;\nend");
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    assert_eq!(app.command,"for k=1:2\nb=k;\nend");
    // Now on the first line: Up recalls the previous entry.
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    assert_eq!(app.command,"a=1;");
    // Down on its last line returns to the block, then to the empty draft.
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowDown,egui::Modifiers::NONE);
    assert_eq!(app.command,"for k=1:2\nb=k;\nend");
    // Cursor at end (last line): Down leaves the block for the draft.
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowDown,egui::Modifiers::NONE);
    assert_eq!(app.command,"");
}

#[test]
fn multiline_down_moves_cursor_when_not_on_last_line() {
    let (ctx,mut app,size)=multiline_fixture();
    app.history.push("later=1;".into());
    app.command="p=1;\nq=2;".into();
    let mut state=egui::text_edit::TextEditState::default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(2))));
    state.store(&ctx,egui::Id::new("command_input"));
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowDown,egui::Modifiers::NONE);
    assert_eq!(app.command,"p=1;\nq=2;");
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    assert_eq!(app.command,"p=1;\nq=2;");
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::NONE);
    assert_eq!(app.command,"later=1;");
}

#[test]
fn multiline_block_in_history_recalls_whole_and_session_keeps_it() {
    let block="if 1\ndisp(7)\nend".to_string();
    let mut history=History::default();
    history.push(block.clone());
    assert_eq!(history.previous("").unwrap(),block);
    let json=serde_json::to_string(&history.entries).unwrap();
    let restored=History::from_entries(serde_json::from_str(&json).unwrap());
    assert_eq!(restored.entries,vec![block]);
}

fn multiline_enter(modifiers:egui::Modifiers) -> egui::Event {
    egui::Event::Key{key:egui::Key::Enter,physical_key:Some(egui::Key::Enter),pressed:true,repeat:false,modifiers}
}
fn multiline_select(ctx:&egui::Context,from:usize,to:usize) {
    let mut state=egui::text_edit::TextEditState::default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(from),egui::text::CCursor::new(to))));
    state.store(ctx,egui::Id::new("command_input"));
}

#[test]
fn multiline_text_and_enter_in_one_frame_apply_in_order() {
    let (ctx,mut app,size)=multiline_fixture();
    let base=app.console.len();
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("if 1".into()),multiline_enter(egui::Modifiers::NONE)]);
    assert_eq!(app.command,"if 1\n");assert!(!app.busy);assert_eq!(app.console.len(),base);
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("disp(5)".into()),multiline_enter(egui::Modifiers::NONE)]);
    assert_eq!(app.command,"if 1\ndisp(5)\n");assert!(!app.busy);assert_eq!(app.console.len(),base);
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("end".into()),multiline_enter(egui::Modifiers::NONE)]);
    finish(&mut app);
    assert!(app.command.is_empty());
    assert_eq!(app.console.iter().filter(|l|l.trim()=="5").count(),1,"{:?}",app.console);
    assert_eq!(app.history.entries.last().unwrap(),"if 1\ndisp(5)\nend");
}

#[test]
fn multiline_keys_after_enter_in_one_frame_land_after_the_newline() {
    let (ctx,mut app,size)=multiline_fixture();
    frame_at(&mut app,&ctx,size,vec![egui::Event::Text("a".into()),multiline_enter(egui::Modifiers::SHIFT),egui::Event::Text("b".into())]);
    frame_at(&mut app,&ctx,size,vec![]);
    assert_eq!(app.command,"a\nb");
}

#[test]
fn multiline_unique_completion_on_earlier_line_leaves_cursor_after_it() {
    let (ctx,mut app,size)=multiline_fixture();
    app.command="if 1\nlinsp\nend".into();
    multiline_select(&ctx,10,10);
    command_first_key(&mut app,&ctx,size,egui::Key::Tab,egui::Modifiers::NONE);
    assert_eq!(app.command,"if 1\nlinspace\nend");
    multiline_type(&mut app,&ctx,size,"(");
    assert_eq!(app.command,"if 1\nlinspace(\nend");
}

#[test]
fn multiline_newline_replaces_the_selection() {
    let (ctx,mut app,size)=multiline_fixture();
    app.command="a x b".into();
    multiline_select(&ctx,2,3);
    command_first_key(&mut app,&ctx,size,egui::Key::Enter,egui::Modifiers::SHIFT);
    assert_eq!(app.command,"a \n b");
    multiline_type(&mut app,&ctx,size,"y");
    assert_eq!(app.command,"a \ny b");
}

#[test]
fn multiline_down_on_last_line_keeps_typed_block_when_not_browsing_history() {
    let (ctx,mut app,size)=multiline_fixture();
    app.history.push("old=1;".into());
    multiline_type(&mut app,&ctx,size,"if 1\nx");
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowDown,egui::Modifiers::NONE);
    assert_eq!(app.command,"if 1\nx");
}

#[test]
fn multiline_shift_arrows_on_edge_lines_do_not_recall_history() {
    let (ctx,mut app,size)=multiline_fixture();
    app.history.push("old=1;".into());
    app.command="a\nb".into();
    multiline_select(&ctx,1,1);
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowUp,egui::Modifiers::SHIFT);
    assert_eq!(app.command,"a\nb");
    multiline_select(&ctx,3,3);
    command_first_key(&mut app,&ctx,size,egui::Key::ArrowDown,egui::Modifiers::SHIFT);
    assert_eq!(app.command,"a\nb");
}
