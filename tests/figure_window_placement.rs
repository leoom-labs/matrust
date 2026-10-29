use matrust::session::figure_window::{
    ChildMetrics, FigureWindow, MonitorArea, NormalWindow, PhysicalRect, WindowRecovery,
    resolve_window,
};
use serde_json::json;

fn normal() -> NormalWindow {
    NormalWindow {
        inner_size_dip: [800., 600.],
        outer_rect_px: [100., 100., 900., 700.],
        work_area_px: [0., 0., 1920., 1040.],
        monitor_device: Some("display-1".into()),
    }
}
fn monitor(edges: [f64; 4], name: &str, primary: bool) -> MonitorArea {
    MonitorArea {
        work: PhysicalRect::new(edges).unwrap(),
        device: Some(name.into()),
        primary,
    }
}
fn metrics(native: f64, zoom: f64) -> ChildMetrics {
    ChildMetrics {
        native_pixels_per_point: native,
        zoom,
        decoration_px: [0., 0.],
    }
}

#[test]
fn optional_versioned_schema_roundtrips_and_resets_only_invalid_metadata() {
    let window = FigureWindow::new(true, Some(normal()), true).unwrap();
    let accepted = WindowRecovery::admitted(window.clone());
    let json = serde_json::to_vec(&accepted).unwrap();
    assert!(json.len() <= 1024);
    let decoded: WindowRecovery = serde_json::from_slice(&json).unwrap();
    assert_eq!(decoded.as_ref(), Some(&window));
    assert!(window.detached() && window.maximized());
    assert_eq!(window.normal().unwrap(), &normal());
    assert!(!decoded.was_reset());
    let empty: WindowRecovery = serde_json::from_str("null").unwrap();
    assert!(empty.is_absent() && !empty.was_reset());
    let valid = serde_json::to_value(&window).unwrap();
    let mut unknown_version = valid.clone();
    unknown_version["version"] = json!(2);
    let mut unknown_field = valid.clone();
    unknown_field["future"] = json!(true);
    let mut invalid_rect = valid.clone();
    invalid_rect["normal"]["outer_rect_px"] = json!([0, 0, 0, 30]);
    let mut invalid_name = valid.clone();
    invalid_name["normal"]["monitor_device"] = json!("x".repeat(33));
    let mut sequence_normal = valid.clone();
    sequence_normal["normal"] = json!([[800, 600], [100, 100, 900, 700], [0, 0, 1920, 1040], null]);
    let mut deep = json!(0);
    for _ in 0..20 {
        deep = json!([deep]);
    }
    for value in [
        unknown_version,
        unknown_field,
        invalid_rect,
        invalid_name,
        sequence_normal,
        json!([1, true, null, false]),
        deep,
        json!(vec![0; 129]),
        json!("x".repeat(4096)),
        json!(false),
    ] {
        let got: WindowRecovery = serde_json::from_value(value).unwrap();
        assert!(got.is_absent() && got.was_reset());
    }
    // A valid device with 32 UTF-16 units can exceed 32 UTF-8 bytes.
    let mut unicode = normal();
    unicode.monitor_device = Some("\u{1f5a5}".repeat(16));
    assert!(FigureWindow::new(true, Some(unicode.clone()), false).is_ok());
    unicode.monitor_device.as_mut().unwrap().push('x');
    assert!(FigureWindow::new(true, Some(unicode), false).is_err());
}

#[test]
fn negative_origin_and_monitor_reordering_preserve_actual_geometry() {
    let monitors = [
        monitor([0., 0., 2560., 1400.], "right", true),
        monitor([-1920., 0., 0., 1040.], "left", false),
    ];
    let mut saved = normal();
    saved.monitor_device = Some("left".into());
    saved.outer_rect_px = [-1360., 220., -560., 820.];
    saved.work_area_px = [-1920., 0., 0., 1040.];
    let first = resolve_window(Some(&saved), &monitors, None, metrics(1., 1.), 0).unwrap();
    assert_eq!(first.monitor_index, 1);
    assert_eq!(first.outer.edges(), saved.outer_rect_px);
    assert_eq!(first.inner_size_points, [800., 600.]);
    let reversed = [monitors[1].clone(), monitors[0].clone()];
    let second = resolve_window(Some(&saved), &reversed, None, metrics(1., 1.), 0).unwrap();
    assert_eq!(second.monitor_index, 0);
    assert_eq!(second.outer, first.outer);
}

#[test]
fn changed_work_area_maps_fraction_and_removed_monitor_uses_root_then_primary() {
    let mut saved = normal();
    saved.outer_rect_px = [560., 220., 1360., 820.]; // Half of each old travel range.
    let current = [monitor([100., 40., 1700., 940.], "display-1", true)];
    let result = resolve_window(Some(&saved), &current, None, metrics(1., 1.), 0).unwrap();
    assert_eq!(result.outer.edges(), [500., 190., 1300., 790.]);
    saved.monitor_device = Some("removed".into());
    saved.outer_rect_px = [3000., 100., 3800., 700.];
    saved.work_area_px = [2560., 0., 4480., 1040.];
    let current = [
        monitor([0., 0., 1920., 1040.], "primary", true),
        monitor([-1600., 0., 0., 900.], "root", false),
    ];
    let root = PhysicalRect::new([-1500., 50., -500., 700.]).unwrap();
    let result = resolve_window(Some(&saved), &current, Some(root), metrics(1., 1.), 0).unwrap();
    assert_eq!(result.monitor_index, 1);
    assert!(current[1].work.contains(result.outer));
    assert_eq!(
        resolve_window(Some(&saved), &current, None, metrics(1., 1.), 0)
            .unwrap()
            .monitor_index,
        0
    );
}

#[test]
fn child_dpi_and_ui_zoom_are_separate_and_decorations_fit() {
    let mut saved = normal();
    saved.outer_rect_px = [100., 100., 1716., 1340.];
    saved.work_area_px = [0., 0., 4000., 2400.];
    let monitors = [monitor(saved.work_area_px, "display-1", true)];
    let mut child = metrics(2., 1.5);
    child.decoration_px = [16., 40.];
    let result = resolve_window(Some(&saved), &monitors, None, child, 0).unwrap();
    for (actual, expected) in result.outer.edges().into_iter().zip(saved.outer_rect_px) {
        assert!((actual - expected).abs() < 1e-9);
    }
    assert_eq!(result.inner_size_points, [1600. / 3., 400.]);
    for value in result.outer_position_points {
        assert!((value - 100. / 3.).abs() < 1e-9);
    }
    child.zoom = 0.8;
    let other = resolve_window(Some(&saved), &monitors, None, child, 0).unwrap();
    assert_eq!(other.outer, result.outer); // Same DIP size despite UI zoom.
    assert_eq!(other.inner_size_points, [1000., 750.]);
    for native in [1., 1.5, 2.] {
        let monitors = [monitor([0., 0., 4000., 2400.], "display-1", true)];
        let r = resolve_window(Some(&normal()), &monitors, None, metrics(native, 1.), 0).unwrap();
        assert_eq!(r.inner_size_points, [800., 600.]);
        assert_eq!(r.monitor_index, 0);
        assert!(monitors[0].work.contains(r.outer));
        // Physical size is reconstructed by subtracting independently rounded
        // edges. Bound that rounding using their coordinate scale; the logical
        // content size and work-area containment above remain exact checks.
        let coordinate_scale = r.outer.edges().into_iter().map(f64::abs).fold(1., f64::max);
        for (actual, expected) in r
            .outer
            .size()
            .into_iter()
            .zip([800. * native, 600. * native])
        {
            assert!((actual - expected).abs() <= 4. * f64::EPSILON * coordinate_scale);
        }
    }
}

#[test]
fn small_work_area_reduces_minimum_and_invalid_topology_fails_closed() {
    let monitors = [monitor([-200., 50., 100., 250.], "small", true)];
    let mut child = metrics(2., 1.5);
    child.decoration_px = [16., 40.];
    let r = resolve_window(None, &monitors, None, child, 15).unwrap();
    assert_eq!(r.outer, monitors[0].work);
    assert_eq!(r.inner_size_points, [284. / 3., 160. / 3.]);
    assert!(resolve_window(None, &[], None, child, 0).is_err());
    assert!(resolve_window(None, &vec![monitors[0].clone(); 33], None, child, 0).is_err());
    assert!(resolve_window(None, &monitors, None, child, 16).is_err());
    child.native_pixels_per_point = f64::NAN;
    assert!(resolve_window(None, &monitors, None, child, 0).is_err());
    let too_short = [monitor([0., 0., 300., 40.], "small", true)];
    child.native_pixels_per_point = 1.;
    assert!(resolve_window(None, &too_short, None, child, 0).is_err());
    for rect in [
        [0., 0., 0., 1.],
        [0., 0., f64::INFINITY, 1.],
        [-1_000_001., 0., 0., 1.],
        [0., 0., 65537., 1.],
    ] {
        assert!(PhysicalRect::new(rect).is_err());
    }
    let mut saved = normal();
    saved.inner_size_dip[0] = f64::NAN;
    assert!(FigureWindow::new(true, Some(saved), false).is_err());
}
