fn workspace_height_texts(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<(String, egui::Rect, egui::Rect)>) {
    match shape {
        egui::Shape::Text(t) => out.push((t.galley.text().into(), t.visual_bounding_rect(), clip)),
        egui::Shape::Vec(shapes) => for shape in shapes { workspace_height_texts(shape, clip, out); },
        _ => {}
    }
}
fn workspace_height_labels(output: &egui::FullOutput) -> Vec<(String, egui::Rect, egui::Rect)> {
    let mut out=Vec::new();
    for shape in &output.shapes { workspace_height_texts(&shape.shape, shape.clip_rect, &mut out); }
    out
}
fn workspace_height_actions(output: &egui::FullOutput, pane: egui::Rect) {
    let labels=workspace_height_labels(output);
    // The variable actions are in the card header's overflow menu.
    let label = "Workspace actions";
    let (_,rect,clip)=labels.iter().find(|(text,_,_)| text==label).unwrap_or_else(||panic!("Missing action {label}"));
    assert!(clip.contains_rect(*rect) && pane.contains_rect(*rect), "Clipped {label}: {rect:?} in {clip:?}, pane {pane:?}");
}
fn workspace_height_fill(app: &mut Workspace) -> matrust::Array {
    app.run("payload=[1 2;3 4];".into());finish(app);
    let shared=app.variables["payload"].clone();
    app.variables.clear();
    for i in 0..1000 { app.variables.insert(format!("value{i:04}"),shared.clone()); }
    shared
}

#[test]
fn workspace_list_uses_tall_pane_and_keeps_actions_visible_in_both_docks_themes_and_selection_states() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.)] {
        for theme in [session::DesktopTheme::Light,session::DesktopTheme::Dark] {
            for dock in ["Right","Below Files"] {
                let ctx=egui::Context::default();let mut app=legacy_sample_desktop(ctx.clone(),true);
                app.layout.theme=Some(theme);app.layout.workspace_width=Some(180.);app.layout.sidebar_width=Some(230.);
                let shared=workspace_height_fill(&mut app);
                choose_workspace_position(&mut app,&ctx,size,dock);
                for selected in [false,true] {
                    app.selected=selected.then(||"value0000".into());
                    for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
                    let output=frame_at(&mut app,&ctx,size,vec![]);
                    let pane=workspace_dock_rect(&ctx,if dock=="Right" {"workspace_variables"}else{"workspace_variables_left"});
                    workspace_height_actions(&output,pane);
                    assert!(app.workspace_tools.list_metrics().0>0. && app.workspace_tools.list_metrics().0<pane.height());
                    if dock=="Right" { assert!(app.workspace_tools.list_metrics().0>140.,"Tall pane should use its available list area"); }
                    assert!(output.shapes.len()<7000,"Visible metadata must remain virtualized");
                    assert!(app.variables["value0000"].same_snapshot(&shared));
                    assert!(app.variables["value0999"].same_snapshot(&shared));
                }
            }
        }
    }
}

#[test]
fn short_workspace_splitter_keeps_footer_visible_and_real_scroll_keyboard_can_select_last_shared_row() {
    for theme in [session::DesktopTheme::Light,session::DesktopTheme::Dark] {
        let size=egui::vec2(960.,650.);let ctx=egui::Context::default();
        let mut app=legacy_sample_desktop(ctx.clone(),true);app.layout.theme=Some(theme);
        let shared=workspace_height_fill(&mut app);
        choose_workspace_position(&mut app,&ctx,size,"Below Files");
        let column=workspace_dock_rect(&ctx,"workspace");
        let before=workspace_dock_rect(&ctx,"workspace_variables_left");
        let from=ctx.read_response(egui::Id::new("workspace_variables_left").with("__resize")).unwrap().rect.center();
        let to=egui::pos2(from.x,column.bottom()-1.);
        frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(from)]);
        pointer_frame(&mut app,&ctx,size,from,true);
        frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(to)]);
        pointer_frame(&mut app,&ctx,size,to,false);
        for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
        let pane=workspace_dock_rect(&ctx,"workspace_variables_left");
        assert!(pane.height()<before.height()-30.);
        let mut found=None;
        for _ in 0..32 {
            let output=frame_at(&mut app,&ctx,size,vec![]);workspace_height_actions(&output,pane);
            found=workspace_height_labels(&output).into_iter().find(|(s,r,c)|s.starts_with("value")&&c.contains_rect(*r)&&pane.contains_rect(*r)).map(|(_,r,_)|r.center());
            if found.is_some(){break;}
            // Scroll the bounded metadata body, above the fixed action footer.
            let pos=egui::pos2(pane.center().x,pane.top()+20.);
            frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(pos),egui::Event::MouseWheel {
                unit:egui::MouseWheelUnit::Point,delta:egui::vec2(0.,-12.),modifiers:egui::Modifiers::NONE,phase:egui::TouchPhase::Move,
            }]);
        }
        let pos=found.expect("A real virtual row must remain reachable in the short pane");
        pointer_frame(&mut app,&ctx,size,pos,true);pointer_frame(&mut app,&ctx,size,pos,false);
        frame_at(&mut app,&ctx,size,vec![]);
        assert!(app.selected.as_ref().is_some_and(|name|name.starts_with("value")));
        frame_at(&mut app,&ctx,size,vec![egui::Event::Key {key:egui::Key::End,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]);
        let output=frame_at(&mut app,&ctx,size,vec![]);workspace_height_actions(&output,pane);
        assert_eq!(app.selected.as_deref(),Some("value0999"));
        assert!(app.workspace_tools.list_metrics().1>0.);
        let painted = workspace_height_labels(&output);
        assert!(painted.iter().any(|(s,r,c)|s=="value0999"&&c.contains_rect(*r)&&pane.contains_rect(*r)),
            "Last row must actually be visible after End: row={:?}, pane={pane:?}, list(height,offset)={:?}",
            painted.iter().filter(|(s,_,_)|s=="value0999").collect::<Vec<_>>(), app.workspace_tools.list_metrics());
        // Both ends must stay reachable after selection introduced preview metadata.
        // Keep real focused keyboard dispatch and exact painted-ink containment.
        for (key, name) in [(egui::Key::Home, "value0000"), (egui::Key::End, "value0999")] {
            frame_at(&mut app,&ctx,size,vec![egui::Event::Key {key,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]);
            let output=frame_at(&mut app,&ctx,size,vec![]);
            workspace_height_actions(&output,pane);
            assert_eq!(app.selected.as_deref(),Some(name));
            let painted=workspace_height_labels(&output);
            assert!(painted.iter().any(|(s,r,c)|s==name&&c.contains_rect(*r)&&pane.contains_rect(*r)),
                "Keyboard target {name} ink must be visible: row={:?}, pane={pane:?}, list(height,offset)={:?}",
                painted.iter().filter(|(s,_,_)|s==name).collect::<Vec<_>>(),app.workspace_tools.list_metrics());
        }
        assert!(app.variables["value0999"].same_snapshot(&shared));
        assert!(app.variables["value0000"].same_snapshot(&shared));
        assert!(app.layout.left_workspace_fraction.is_some());
    }
}

#[test]
fn selected_value_label_and_arrow_each_toggle_once_without_changing_shared_values() {
    fn arrow(shape:&egui::Shape,clip:egui::Rect,label:egui::Rect)->Option<egui::Pos2> {
        match shape {
            egui::Shape::Path(path) if path.closed && path.points.len()==3 => {
                let rect=path.visual_bounding_rect();
                (clip.contains_rect(rect) && rect.right()<label.left()
                    && label.left()-rect.right()<32. && (rect.center().y-label.center().y).abs()<8.)
                    .then_some(rect.center())
            }
            egui::Shape::Vec(shapes)=>shapes.iter().find_map(|shape|arrow(shape,clip,label)),
            _=>None,
        }
    }
    for theme in [session::DesktopTheme::Light,session::DesktopTheme::Dark] {
        let ctx=egui::Context::default();let size=egui::vec2(1440.,900.);
        let mut app=legacy_sample_desktop(ctx.clone(),true);app.layout.theme=Some(theme);
        app.run("value=complex([1 2],[3 4]);alias=value;".into());finish(&mut app);
        app.selected=Some("value".into());choose_workspace_position(&mut app,&ctx,size,"Right");
        let before=app.variables.clone();let detail="complex double | 32 bytes";
        for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
        // The preview starts folded: the card is as tall as its rows.
        let mut shown=false;
        for use_arrow in [false,true,false,true] {
            let output=frame_at(&mut app,&ctx,size,vec![]);
            let labels=workspace_height_labels(&output);
            assert_eq!(labels.iter().any(|(s,r,c)|s==detail && c.contains_rect(*r)),shown);
            let (_,label,clip)=labels.iter().find(|(s,_,_)|s=="Selected value").unwrap();
            assert!(clip.contains_rect(*label));
            let pos=if use_arrow {
                output.shapes.iter().find_map(|s|arrow(&s.shape,s.clip_rect,*label)).expect("Actual painted collapse arrow")
            } else {label.center()};
            for pressed in [true,false] {pointer_frame(&mut app,&ctx,size,pos,pressed);}
            shown = !shown;
            for _ in 0..4 {frame_at(&mut app,&ctx,size,vec![]);}
            let after=frame_at(&mut app,&ctx,size,vec![]);
            assert_eq!(workspace_height_labels(&after).iter().any(|(s,r,c)|s==detail && c.contains_rect(*r)),shown,
                "Each label/arrow click must toggle exactly once, arrow={use_arrow}");
            assert!(!app.busy);
            assert_eq!(before.len(),app.variables.len());
            for (name,value) in &before {assert!(value.same_snapshot(&app.variables[name]));}
            assert!(app.variables["value"].same_snapshot(&app.variables["alias"]));
        }
    }
}
