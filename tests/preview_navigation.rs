use matrust::session::{PreviewFormat, PreviewNavigation, PreviewState, PreviewWindow};

#[test]
fn preview_navigation_normalizes_bad_values_without_discarding_file_reference() {
    let mut state = PreviewState {
        navigation: PreviewNavigation {
            scroll: [12., f32::NAN],
            window: Some(PreviewWindow {
                position: [f32::INFINITY, 10.],
                size: [640., 480.],
            }),
        },
        path: std::env::current_dir().unwrap().join("image.png"),
        page: 0,
        fit: false,
        zoom: 2.,
        format: PreviewFormat::Png,
    };
    state.navigation.normalize();
    assert_eq!(state.navigation.scroll, [12., 0.]);
    assert!(state.navigation.window.is_none());
    let mut value = serde_json::to_value(&state).unwrap();
    for bad in [
        serde_json::json!("broken"),
        serde_json::json!({"window":{"position":[0,0],"size":[1,2]},"scroll":[-1,40000]}),
    ] {
        value["navigation"] = bad;
        let decoded: PreviewState = serde_json::from_value(value.clone()).unwrap();
        assert!(decoded.valid());
        assert_eq!(decoded.navigation, PreviewNavigation::default());
        assert_eq!(decoded.path, state.path);
    }
    value.as_object_mut().unwrap().remove("navigation");
    let decoded: PreviewState = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.navigation, PreviewNavigation::default());
}
