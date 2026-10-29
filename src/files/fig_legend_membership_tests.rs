use super::*;

fn props_mut<'a, 'b>(node: &'b mut Fields<'a>) -> &'b mut Fields<'a> {
    let Value::Struct(items) = node.get_mut("properties").unwrap() else {
        panic!("properties")
    };
    &mut items[0]
}
fn root_children_mut<'a, 'b>(value: &'b mut Value<'a>) -> &'b mut Vec<Fields<'a>> {
    let Value::Struct(root) = value else {
        panic!("root")
    };
    let Value::Struct(children) = root[0].get_mut("children").unwrap() else {
        panic!("children")
    };
    children
}
fn check(value: &Value<'_>) -> Result<()> {
    let root = object(value)?;
    let all = children(root)?;
    let axes: Vec<_> = all
        .iter()
        .filter(|n| string(n, "type", "").unwrap() == "axes")
        .collect();
    let legend = all
        .iter()
        .find(|n| string(n, "type", "").unwrap() == "scribe.legend")
        .unwrap();
    validate_legend_membership(
        object(field(legend, "properties")?)?,
        &legend_series_handles(&axes)?,
        0,
    )
    .map(|_| ())
}
fn numeric(bytes: &[u8]) -> Value<'_> {
    Value::Number {
        kind: 9,
        endian: mat::Endian::Little,
        bytes,
        count: bytes.len() / 8,
    }
}
fn replace_selected<'a>(value: &mut Value<'a>, selected: Value<'a>) {
    let legend = root_children_mut(value)
        .iter_mut()
        .find(|n| n["type"].text() == Some("scribe.legend"))
        .unwrap();
    props_mut(legend).insert(
        "ApplicationData".into(),
        Value::Struct(vec![BTreeMap::from([("PlotChildren".into(), selected)])]),
    );
}
fn authentic() -> mat::Expanded {
    mat::expand(&std::fs::read("tests/fixtures/fig/matlab-lines-v6.fig").unwrap()).unwrap()
}

#[test]
fn actual_handles_are_scalar_finite_and_signed_zero_equality_is_consistent() {
    let bytes = authentic();
    let negative_zero = (-0_f64).to_le_bytes();
    let positive_zero = 0_f64.to_le_bytes();
    let mut value = bytes.hierarchy().unwrap();
    let second = {
        let axis = root_children_mut(&mut value)
            .iter_mut()
            .find(|n| n["type"].text() == Some("axes"))
            .unwrap();
        let Value::Struct(series) = axis.get_mut("children").unwrap() else {
            panic!("series")
        };
        let mut lines = series
            .iter_mut()
            .filter(|n| n["type"].text() == Some("graph2d.lineseries"));
        lines
            .next()
            .unwrap()
            .insert("handle".into(), numeric(&negative_zero));
        scalar(lines.next().unwrap(), "handle", f64::NAN).unwrap()
    };
    let selected: Vec<u8> = [0., second].iter().flat_map(|v| v.to_le_bytes()).collect();
    replace_selected(&mut value, numeric(&selected));
    assert!(
        check(&value).is_ok(),
        "-0 actual handle associates with +0 metadata"
    );
    let axis = root_children_mut(&mut value)
        .iter_mut()
        .find(|n| n["type"].text() == Some("axes"))
        .unwrap();
    let Value::Struct(series) = axis.get_mut("children").unwrap() else {
        panic!("series")
    };
    series
        .iter_mut()
        .filter(|n| n["type"].text() == Some("graph2d.lineseries"))
        .nth(1)
        .unwrap()
        .insert("handle".into(), numeric(&positive_zero));
    assert!(
        check(&value).unwrap_err().contains("unique"),
        "+0 and -0 are duplicate actual handles"
    );
    for malformed in [vec![f64::NAN], vec![f64::INFINITY], vec![1., 2.], vec![]] {
        let data: Vec<u8> = malformed.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut root = bytes.hierarchy().unwrap();
        let axis = root_children_mut(&mut root)
            .iter_mut()
            .find(|n| n["type"].text() == Some("axes"))
            .unwrap();
        let Value::Struct(series) = axis.get_mut("children").unwrap() else {
            panic!("series")
        };
        series
            .iter_mut()
            .find(|n| n["type"].text() == Some("graph2d.lineseries"))
            .unwrap()
            .insert("handle".into(), numeric(&data));
        assert!(check(&root).is_err(), "{malformed:?}");
    }
}

#[test]
fn authentic_legend_handles_preserve_five_fixtures_including_hidden_series() {
    for name in [
        "matlab-lines-v6.fig",
        "matlab-lines.fig",
        "matlab-log-scatter.fig",
        "matlab-bars.fig",
        "matlab-hidden.fig",
    ] {
        let bytes = std::fs::read(format!("tests/fixtures/fig/{name}")).unwrap();
        let result = decode_matlab_figure(&bytes).unwrap();
        assert!(result.legend);
        assert_eq!(result.series.len(), 2);
        if name == "matlab-hidden.fig" {
            assert!(result.series[0].visible && !result.series[1].visible);
            assert_eq!(result.series[1].label, "Hidden");
        }
    }
}

#[test]
fn explicit_membership_rejects_duplicate_stale_nonfinite_and_empty_handles() {
    let bytes = authentic();
    let root = bytes.hierarchy().unwrap();
    let all = children(object(&root).unwrap()).unwrap();
    let legend = all
        .iter()
        .find(|n| n["type"].text() == Some("scribe.legend"))
        .unwrap();
    let p = object(field(legend, "properties").unwrap()).unwrap();
    let app = object(field(p, "ApplicationData").unwrap()).unwrap();
    let v = &app["PlotChildren"];
    let a = v.number(0).unwrap();
    for values in [vec![a, a], vec![a, 999.], vec![a, f64::NAN], vec![]] {
        let encoded: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut value = bytes.hierarchy().unwrap();
        replace_selected(&mut value, numeric(&encoded));
        assert!(check(&value).is_err(), "{values:?}");
    }
}

#[test]
fn duplicate_labels_retain_explicit_order_and_absent_metadata_keeps_legacy_fallback() {
    let bytes = authentic();
    let mut value = bytes.hierarchy().unwrap();
    for node in root_children_mut(&mut value) {
        match node["type"].text().unwrap() {
            "axes" => {
                let Value::Struct(series) = node.get_mut("children").unwrap() else {
                    panic!("series")
                };
                for s in series
                    .iter_mut()
                    .filter(|s| s["type"].text() == Some("graph2d.lineseries"))
                {
                    props_mut(s).insert("DisplayName".into(), Value::Text("same".into()));
                }
            }
            "scribe.legend" => {
                props_mut(node).insert(
                    "String".into(),
                    Value::Cell(vec![Value::Text("same".into()), Value::Text("same".into())]),
                );
            }
            _ => {}
        }
    }
    assert!(check(&value).is_ok());
    let all = children(object(&value).unwrap()).unwrap();
    let legend = all
        .iter()
        .find(|n| n["type"].text() == Some("scribe.legend"))
        .unwrap();
    let p = object(field(legend, "properties").unwrap()).unwrap();
    let app = object(field(p, "ApplicationData").unwrap()).unwrap();
    let handles = &app["PlotChildren"];
    let reversed: Vec<u8> = [handles.number(1).unwrap(), handles.number(0).unwrap()]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    replace_selected(&mut value, numeric(&reversed));
    let all = children(object(&value).unwrap()).unwrap();
    let axes: Vec<_> = all
        .iter()
        .filter(|n| n["type"].text() == Some("axes"))
        .collect();
    let legend = all
        .iter()
        .find(|n| n["type"].text() == Some("scribe.legend"))
        .unwrap();
    assert_eq!(
        validate_legend_membership(
            object(field(legend, "properties").unwrap()).unwrap(),
            &legend_series_handles(&axes).unwrap(),
            0
        )
        .unwrap(),
        Some(vec![1, 0])
    );
    let legend = root_children_mut(&mut value)
        .iter_mut()
        .find(|n| n["type"].text() == Some("scribe.legend"))
        .unwrap();
    props_mut(legend).insert("ApplicationData".into(), Value::Opaque);
    assert!(
        check(&value).is_ok(),
        "absent metadata retains explicitly ambiguous legacy fallback"
    );
}

#[test]
fn actual_serialized_membership_mutation_is_rejected_before_plot_publication() {
    let original = std::fs::read("tests/fixtures/fig/matlab-lines-v6.fig").unwrap();
    let mut expanded = mat::expand(&original).unwrap();
    let offset = {
        let value = expanded.hierarchy().unwrap();
        let all = children(object(&value).unwrap()).unwrap();
        let legend = all
            .iter()
            .find(|n| n["type"].text() == Some("scribe.legend"))
            .unwrap();
        let p = object(field(legend, "properties").unwrap()).unwrap();
        let app = object(field(p, "ApplicationData").unwrap()).unwrap();
        let Value::Number {
            kind: 9,
            bytes,
            count: 2,
            ..
        } = &app["PlotChildren"]
        else {
            panic!("real MATLAB double handles")
        };
        bytes.as_ptr() as usize - expanded.bytes.as_ptr() as usize
    };
    expanded.bytes[offset..offset + 8].copy_from_slice(&999_f64.to_le_bytes());
    let mut changed = original[..128].to_vec();
    changed.extend_from_slice(&expanded.bytes);
    assert!(
        decode_matlab_figure(&changed)
            .unwrap_err()
            .contains("PlotChildren")
    );
}

#[test]
fn foreign_and_duplicate_series_handles_cannot_bind_an_explicit_legend() {
    let bytes = authentic();
    let replacement = 999_f64.to_le_bytes();
    let mut value = bytes.hierarchy().unwrap();
    let axis = root_children_mut(&mut value)
        .iter_mut()
        .find(|n| n["type"].text() == Some("axes"))
        .unwrap();
    let Value::Struct(series) = axis.get_mut("children").unwrap() else {
        panic!("series")
    };
    let line = series
        .iter_mut()
        .find(|n| n["type"].text() == Some("graph2d.lineseries"))
        .unwrap();
    line.insert("handle".into(), numeric(&replacement));
    assert!(check(&value).is_err());
    let axis_handle = {
        let axis = children(object(&value).unwrap())
            .unwrap()
            .iter()
            .find(|n| n["type"].text() == Some("axes"))
            .unwrap();
        scalar(axis, "handle", f64::NAN).unwrap().to_le_bytes()
    };
    let axis = root_children_mut(&mut value)
        .iter_mut()
        .find(|n| n["type"].text() == Some("axes"))
        .unwrap();
    let Value::Struct(series) = axis.get_mut("children").unwrap() else {
        panic!("series")
    };
    let line = series
        .iter_mut()
        .find(|n| n["type"].text() == Some("graph2d.lineseries"))
        .unwrap();
    line.insert("handle".into(), numeric(&axis_handle));
    assert!(check(&value).unwrap_err().contains("unique"));
}
