use matrust::simulation::{Block, Model};

#[test]
fn parameterless_blocks_preserve_unit_api_and_exact_legacy_wire_form() {
    for (name, block) in [
        ("Sum", Block::Sum),
        ("Abs", Block::Abs),
        ("UnaryMinus", Block::UnaryMinus),
        ("Scope", Block::Scope),
    ] {
        let expected = format!(r#"{{"type":"{name}"}}"#);
        assert_eq!(serde_json::to_string(&block).unwrap(), expected);
        let restored: Block = serde_json::from_str(&expected).unwrap();
        assert_eq!(serde_json::to_string(&restored).unwrap(), expected);
        assert_eq!(restored.label(), block.label());
    }
    let model = Model::default();
    let saved = serde_json::to_vec(&model).unwrap();
    let reopened: Model = serde_json::from_slice(&saved).unwrap();
    assert!(model.same_storage(&reopened));
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), saved);
}

#[test]
fn parameterless_blocks_reject_unknown_parameters_in_every_tag_order() {
    for name in ["Sum", "Abs", "UnaryMinus", "Scope"] {
        for value in [
            "null",
            "false",
            "1",
            r#""single""#,
            "[]",
            "{}",
            r#"{"nested":1}"#,
        ] {
            for text in [
                format!(r#"{{"type":"{name}","type_name":{value}}}"#),
                format!(r#"{{"type_name":{value},"type":"{name}"}}"#),
            ] {
                let error = serde_json::from_str::<Block>(&text).unwrap_err();
                assert!(
                    error.to_string().contains("unknown field"),
                    "{text}: {error}"
                );
            }
        }
        assert!(
            serde_json::from_str::<Block>(&format!(r#"{{"type":"{name}","type":"{name}"}}"#))
                .is_err()
        );
    }
}

#[test]
fn strict_block_loading_rejects_nested_model_parameters_without_changing_existing_model() {
    let original = Model::default();
    let before = serde_json::to_vec(&original).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let sum = json["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["block"]["type"] == "Sum")
        .unwrap();
    sum["block"]["type_name"] = serde_json::Value::String("single".into());
    assert!(serde_json::from_value::<Model>(json).is_err());
    assert_eq!(serde_json::to_vec(&original).unwrap(), before);
    // Existing structured variants retain their missing/default/unknown policy.
    assert!(serde_json::from_str::<Block>(r#"{"type":"Gain"}"#).is_err());
    assert!(serde_json::from_str::<Block>(r#"{"type":"Gain","gain":1,"extra":0}"#).is_err());
    assert!(serde_json::from_str::<Block>(r#"{"type":"Product"}"#).is_ok());
    assert!(serde_json::from_str::<Block>(r#"{"type":"Switch"}"#).is_ok());
}
