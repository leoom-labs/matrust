#[test]
fn complex_cell_text_is_single_row_bounded_and_hover_retains_full_value() {
    let ctx=egui::Context::default();
    ctx.all_styles_mut(|s|s.interaction.tooltip_delay=0.);
    let values=[matrust::Scalar::Complex(matrust::Complex64::new(f64::MAX,-f64::MAX)),
        matrust::Scalar::Complex(matrust::Complex64::new(f64::from_bits(1),-0.)),
        matrust::Scalar::Real(3.)];
    let mut rectangles=Vec::new();
    let mut pointer=None;
    let mut output=None;
    for pass in 0..6 {
        rectangles.clear();
        output=Some(crate::headless::run(&ctx,egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(960.,650.))),
            time:Some(pass as f64),events:pointer.map(egui::Event::PointerMoved).into_iter().collect(),
            ..Default::default()
        },|root| {
            egui::CentralPanel::default().show(root,|ui| {
                ui.horizontal(|ui|for &value in &values {rectangles.push(scalar_cell_button(ui,value,false).rect);});
            });
        }));
        assert_eq!(rectangles.len(),3);
        for rectangle in &rectangles {assert!(rectangle.width()<=90.1 && rectangle.height()<=22.1,"{rectangle:?}");}
        for pair in rectangles.windows(2) {
            assert!(pair[0].right()<=pair[1].left());
            assert!((pair[0].top()-pair[1].top()).abs()<0.1);
        }
        pointer=Some(rectangles[0].center());
    }
    let full=values[0].to_string();assert!(full.len()>256);
    assert!(output.unwrap().shapes.iter().any(|s|text_position(&s.shape,&full).is_some()),"Full tooltip text missing");
}

#[test]
fn scalar_editor_literal_round_trips_extreme_components_inside_byte_limit() {
    for value in [
        matrust::Scalar::Real(f64::MAX),matrust::Scalar::Real(f64::from_bits(1)),
        matrust::Scalar::Complex(matrust::Complex64::new(f64::MAX,-f64::MAX)),
        matrust::Scalar::Complex(matrust::Complex64::new(f64::from_bits(1),-0.)),
        matrust::Scalar::Complex(matrust::Complex64::new(-0.,-f64::from_bits(1))),
    ] {
        let literal=scalar_edit_literal(value);assert!(literal.len()<=256);
        let parsed=matrust::Scalar::parse_cell(&literal,value.is_complex(),false).unwrap().into_complex();
        let original=value.into_complex();
        assert_eq!((parsed.re.to_bits(),parsed.im.to_bits()),(original.re.to_bits(),original.im.to_bits()));
    }
}

#[test]
fn complex_inspector_real_cell_click_and_copy_keep_full_precision_and_aliases() {
    for size in [egui::vec2(1440.,900.),egui::vec2(960.,650.)] {
        let ctx=egui::Context::default();
        let mut app=legacy_sample_desktop(ctx.clone(),true);
        app.run("Z=complex([1 1.7976931348623157e308],[2 -1.7976931348623157e308]);Alias=Z;".into());
        finish(&mut app);
        let original=app.variables["Z"].clone();let alias=app.variables["Alias"].clone();
        app.inspect_variable("Z".into());
        let value=original.scalar(0,1).unwrap();
        let label=format_scalar_cell(value);
        let mut position=None;
        for _ in 0..3 {position=frame_at(&mut app,&ctx,size,vec![]).shapes.iter().rev().find_map(|s|text_position(&s.shape,&label));}
        let position=position.expect("Second complex cell visible");
        for pressed in [true,false] {pointer_frame(&mut app,&ctx,size,position,pressed);}
        assert_eq!((app.edit_row,app.edit_col),(1,2));
        assert_eq!(app.edit_value,matrust::variable_edit::scalar_literal(value));assert!(app.edit_value.len()<=256);
        let parsed=matrust::Scalar::parse_cell(&app.edit_value,true,false).unwrap().into_complex();
        assert_eq!(parsed,value.into_complex());
        let mut copy=None;
        for _ in 0..3 {copy=frame_at(&mut app,&ctx,size,vec![]).shapes.iter().rev().find_map(|s|text_position(&s.shape,"Copy cell"));}
        let copy=copy.expect("Copy cell action visible");pointer_frame(&mut app,&ctx,size,copy,true);
        let output=frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(copy),egui::Event::PointerButton {
            pos:copy,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE,
        }]);
        assert!(!output.platform_output.commands.iter().any(|command|matches!(command,egui::OutputCommand::CopyText(_))),"Copy is dispatched to the worker");
        finish(&mut app);
        let output=frame_at(&mut app,&ctx,size,vec![]);
        let copied=output.platform_output.commands.into_iter().find_map(|command|match command {
            egui::OutputCommand::CopyText(text)=>Some(text),_=>None,
        }).expect("Actual Copy cell command");
        assert_eq!(copied,matrust::variable_edit::scalar_literal(value));assert!(copied.len()<=256);
        let roundtrip=matrust::Scalar::parse_cell(&copied,true,false).unwrap().into_complex();
        let expected=value.into_complex();
        assert_eq!((roundtrip.re.to_bits(),roundtrip.im.to_bits()),(expected.re.to_bits(),expected.im.to_bits()));
        assert!(original.same_snapshot(&app.variables["Z"]));
        assert!(alias.same_snapshot(&app.variables["Alias"]));
        assert!(original.same_snapshot(&alias));assert!(!app.busy);
        // Apply the automatically populated bounded literal through the actual
        // worker action, rather than merely testing its parser in isolation.
        let mut apply=None;
        for _ in 0..3 {apply=frame_at(&mut app,&ctx,size,vec![]).shapes.iter().rev().find_map(|s|text_position(&s.shape,"Apply cell"));}
        let apply=apply.expect("Apply cell action visible");
        for pressed in [true,false] {pointer_frame(&mut app,&ctx,size,apply,pressed);}
        assert!(app.busy,"Actual Apply cell action did not submit its worker job");
        finish(&mut app);
        assert!(app.cell_error.is_none(),"{:?}",app.cell_error);
        let updated=app.variables["Z"].scalar(0,1).unwrap().into_complex();
        let expected=value.into_complex();
        assert_eq!((updated.re.to_bits(),updated.im.to_bits()),(expected.re.to_bits(),expected.im.to_bits()));
        assert!(alias.same_snapshot(&app.variables["Alias"]));
        assert_eq!(app.variables["Z"].scalar(0,0).unwrap(),original.scalar(0,0).unwrap());
    }
}


#[test]
fn complex_display_attribute_is_visible_for_empty_zero_imaginary_and_nonempty_values() {
    fn fully_visible(shape:&egui::Shape,label:&str,clip:egui::Rect)->bool {
        match shape {
            egui::Shape::Text(text) if text.galley.text()==label => clip.contains_rect(text.visual_bounding_rect()),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape|fully_visible(shape,label,clip)),
            _ => false,
        }
    }
    for right in [false,true] {
    for size in [egui::vec2(1440.,900.), egui::vec2(960.,650.)] {
        let ctx=egui::Context::default();
        let mut app=legacy_sample_desktop(ctx.clone(),true);
        if right {app.layout.workspace_dock=None;}
        assert_eq!(app.workspace_on_left(),!right);
        assert!(app.console.starts_with(&chrome::banner()));
        app.run("Empty=complex(zeros(0,3));Zero=complex([1 2]);Z=complex([1 2],[3 -4]);R=1;L=logical(1);".into());
        finish(&mut app);
        let before=app.variables.clone();
        for name in ["Empty","Zero","Z"] {
            app.inspect_variable(name.into());
            let mut output=None;
            for _ in 0..3 {output=Some(frame_at(&mut app,&ctx,size,vec![]));}
            let output=output.unwrap();
            let value=&app.variables[name];
            assert_eq!(value.class(),"double","MATLAB class remains double");
            let header=format!("{} rows \u{00d7} {} columns \u{00b7} complex double",value.rows(),value.cols());
            assert!(output.shapes.iter().any(|s|text_position(&s.shape,&header).is_some()),"{name} at {size:?}");
            if value.is_empty() {
                assert!(output.shapes.iter().any(|s|text_position(&s.shape,"Empty matrix").is_some()));
            }
            let detail=format!("complex double | {} bytes",value.bytes());
            app.editing_variable=None;
            // The responsive preview can start collapsed. First try real scrolling;
            // if its detail is absent, activate the actual Selected value label.
            let panel=if right {"workspace_variables"} else {"workspace_variables_left"};
            let mut visible=false;
            for attempt in 0..2 {
                for _ in 0..16 {
                    let sidebar=frame_at(&mut app,&ctx,size,vec![]);
                    visible=sidebar.shapes.iter().any(|s|fully_visible(&s.shape,&detail,s.clip_rect.intersect(egui::Rect::from_min_size(egui::Pos2::ZERO,size))));
                    if visible {break;}
                    let pane=egui::containers::panel::PanelState::load(&ctx,egui::Id::new(panel)).unwrap().outer_rect;
                    // Wheel over the middle of the card.
                    let pos=pane.center();
                    frame_at(&mut app,&ctx,size,vec![egui::Event::PointerMoved(pos),egui::Event::MouseWheel {
                        unit:egui::MouseWheelUnit::Point,delta:egui::vec2(0.,-60.),
                        modifiers:egui::Modifiers::NONE,phase:egui::TouchPhase::Move,
                    }]);
                }
                if visible {break;}
                if attempt==0 {
                    let sidebar=frame_at(&mut app,&ctx,size,vec![]);
                    let labels=workspace_height_labels(&sidebar);
                    let (_,label,clip)=labels.iter().find(|(s,_,_)|s=="Selected value").expect("Actual selected-value header");
                    assert!(clip.contains_rect(*label));
                    for pressed in [true,false] {pointer_frame(&mut app,&ctx,size,label.center(),pressed);}
                }
            }
            assert!(visible,"Selected complex attribute must be reachable: {name}, {size:?}, right={right}");
        }
        assert_eq!(workspace_type_label(&app.variables["R"]),"double");
        assert_eq!(workspace_type_label(&app.variables["L"]),"logical");
        for (name,value) in &before {assert!(value.same_snapshot(&app.variables[name]));}
    }
    }
}
