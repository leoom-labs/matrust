use matrust::{Engine, document::Document, syntax};

#[test]
fn syntax_errors_have_unicode_aware_source_locations() {
    for (source, line, column, token) in [
        ("% αβ\na=1;\nb=(2+);", 3, 6, ")"),
        ("a=1;\nb=sin(2", 2, 8, ""),
        ("% α\ntitle('λ'); @", 2, 13, "@"),
        ("a=1;\ntitle('unfinished", 2, 7, "'"),
        ("a=1;\nb=2 ... comment\n+ @", 3, 3, "@"),
    ] {
        let error = syntax::parse_detailed(source).unwrap_err();
        assert_eq!(
            (error.line, error.column),
            (line, column),
            "{source}: {error}"
        );
        assert!(source[error.byte..].starts_with(token));
    }
    let source = "% α\na=1;\nb=2 ... rest\n+ 3;";
    let statements = syntax::parse_detailed(source).unwrap();
    assert_eq!(&source[statements[0].span.clone()], "a=1");
    assert_eq!(&source[statements[1].span.clone()], "b=2 ... rest\n+ 3");
}

#[test]
fn runtime_error_preserves_completed_output_and_points_at_statement() {
    let mut engine = Engine::default();
    let source = "disp('old'); clc;\na=7\n  b=unknown(2);\nc=9;";
    assert!(engine.execute(source).is_err());
    let error = engine.diagnostic.as_ref().unwrap();
    assert_eq!((error.line, error.column), (3, 3));
    assert!(source[error.byte..].starts_with("b=unknown"));
    assert!(engine.failed_output.clear_console);
    assert_eq!(engine.failed_output.output.len(), 1);
    assert!(engine.failed_output.output[0].starts_with("\na ="));
    assert_eq!(engine.variables["a"].data(), &[7.]);
    assert!(!engine.variables.contains_key("c"));
    engine.execute("c=9;").unwrap();
    assert!(engine.diagnostic.is_none());
    assert!(engine.failed_output.output.is_empty());
}

#[test]
fn recovered_documents_receive_new_unique_runtime_identities() {
    let a = Document::new();
    let b = Document::new();
    assert_ne!(a.id, b.id);
    let serialized = serde_json::to_string(&a).unwrap();
    assert!(!serialized.contains("\"id\""));
    let recovered: Document = serde_json::from_str(&serialized).unwrap();
    assert_ne!(a.id, recovered.id);
    assert_eq!(a.id, a.clone().id);
}

#[test]
fn logical_lines_preserve_comments_continuations_blocks_and_matrix_rows() {
    for newline in ["\n", "\r\n", "\r"] {
        let source = [
            "% header",
            "a=1",
            "b=2 ... ignored",
            "+3",
            "m=[1 2",
            "3 4];",
            "for k=1:2",
            "a=a+1; % tail",
            "end",
        ]
        .join(newline);
        let mut engine = Engine::default();
        engine.execute(&source).unwrap();
        assert_eq!(engine.variables["a"].data(), &[3.]);
        assert_eq!(engine.variables["b"].data(), &[5.]);
        assert_eq!(engine.variables["m"].data(), &[1., 3., 2., 4.]);
        let statements = syntax::parse_detailed(&source).unwrap();
        assert_eq!(&source[statements[0].span.clone()], "a=1");
        assert_eq!(
            &source[statements[1].span.clone()],
            format!("b=2 ... ignored{newline}+3")
        );
    }
}

#[test]
fn command_mode_and_selection_obey_raw_logical_line_boundaries() {
    for newline in ["\n", "\r\n", "\r"] {
        let mut engine = Engine::default();
        engine
            .execute(&format!(
                "A=1; B=2;{newline}clear A{newline}C=7{newline}clear ... tail{newline}B{newline}D=9"
            ))
            .unwrap();
        assert!(!engine.variables.contains_key("A"));
        assert!(!engine.variables.contains_key("B"));
        assert_eq!(engine.variables["C"].data(), &[7.]);
        assert_eq!(engine.variables["D"].data(), &[9.]);
        let source = [
            "% Unicode \u{3bb}",
            "skip=missing;",
            "R=twice(4);",
            "function y=twice(x)",
            "y=x*2;",
            "end",
        ]
        .join(newline);
        let start = source.find("R=").unwrap();
        engine
            .execute_selection(&source, start..start + 11)
            .unwrap();
        assert_eq!(engine.variables["R"].data(), &[8.]);
        assert!(!engine.variables.contains_key("skip"));
        assert!(syntax::parse_detailed(&format!("load 'bad{newline}name'")).is_err());
        let error = syntax::parse_detailed(&format!("load file 'bad{newline}name'")).unwrap_err();
        assert!(error.message.contains("cannot cross a newline"));
    }
}

#[test]
fn logical_line_diagnostics_keep_original_unicode_byte_offsets() {
    for newline in ["\n", "\r\n", "\r"] {
        let source = format!("% \u{3bb}{newline}a=1{newline}  b=(2+);");
        let error = syntax::parse_detailed(&source).unwrap_err();
        assert_eq!((error.line, error.column), (3, 8));
        assert_eq!(&source[error.byte..error.byte + 1], ")");
    }
    let source = "% \u{3bb}\r\na=1\rb=2\n  @";
    let error = syntax::parse_detailed(source).unwrap_err();
    assert_eq!((error.line, error.column), (4, 3));
    assert_eq!(&source[error.byte..], "@");
}

#[test]
fn character_literal_line_ending_payloads_are_not_normalized() {
    for newline in ["\n", "\r\n", "\r"] {
        let text = format!("a{newline}b");
        let mut engine = Engine::default();
        engine.execute(&format!("s='{text}';")).unwrap();
        let matrust::Array::Character(value) = engine.array("s").unwrap() else {
            panic!("character result")
        };
        assert_eq!(value.units(), text.encode_utf16().collect::<Vec<_>>());
    }
}

#[test]
fn disk_cr_bom_run_save_recovery_and_conflicts_preserve_raw_body() {
    use matrust::document::SaveOutcome;
    let dir = tempfile::tempdir().unwrap();
    for (i, newline) in ["\n", "\r\n", "\r"].into_iter().enumerate() {
        let path = dir.path().join(format!("source_{i}.m"));
        let body = format!("% header{newline}A=7;{newline}");
        std::fs::write(&path, format!("\u{feff}{body}")).unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.text, body);
        assert!(!doc.dirty);
        let mut engine = Engine::default();
        engine
            .execute_document(&doc.text, doc.path.as_ref().unwrap())
            .unwrap();
        assert_eq!(engine.variables["A"].data(), &[7.]);
        assert!(doc.external_change().unwrap().is_none());
        doc.text.push_str(&format!("B=9;{newline}"));
        doc.changed();
        doc.save(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), doc.text.as_bytes());
        assert!(doc.external_change().unwrap().is_none());
        let mut recovered: Document =
            serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        recovered.validate_recovery().unwrap();
        assert_eq!(recovered.text, doc.text);
        let other = dir.path().join(format!("copy_{i}.m"));
        recovered.save(&other).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), doc.text.as_bytes());
        std::fs::write(&other, format!("outside=4;{newline}")).unwrap();
        recovered.text.push_str("C=3;");
        recovered.changed();
        assert!(matches!(
            recovered.try_save(&other, None).unwrap(),
            SaveOutcome::Conflict(_)
        ));
        assert!(recovered.dirty);
        assert_eq!(
            std::fs::read_to_string(&other).unwrap(),
            format!("outside=4;{newline}")
        );
    }
}

#[test]
fn display_mapping_preserves_all_source_indices_and_comment_boundaries() {
    use matrust::editor::{Style, highlights, layout_matches, layout_text};
    let source = "% \u{3bb}\ra=1;\r\nb=2;\nc=3;";
    let display = layout_text(source);
    assert_eq!(display, "% \u{3bb}\na=1;\r\nb=2;\nc=3;");
    assert_eq!(display.len(), source.len());
    assert_eq!(display.chars().count(), source.chars().count());
    assert!(layout_matches(&display, source));
    assert!(!layout_matches(&display, &format!("{source}x")));
    for (a, b) in source.char_indices().zip(display.char_indices()) {
        assert_eq!(a.0, b.0);
    }
    let comment = highlights(source)
        .into_iter()
        .find(|(_, s)| *s == Style::Comment)
        .unwrap()
        .0;
    assert_eq!(&source[comment], "% \u{3bb}");
    assert!(matches!(
        layout_text("a\r\nb\n"),
        std::borrow::Cow::Borrowed(_)
    ));
}
