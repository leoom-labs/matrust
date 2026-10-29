use super::*;
use crate::plot::MarkerShape;

fn first_properties(bytes: &[u8]) -> Fields<'_> {
    let value = mat::hierarchy(&bytes[128..], mat::Endian::Little).unwrap();
    let mut root = match value {
        Value::Struct(mut roots) => roots.remove(0),
        _ => panic!(),
    };
    let Value::Struct(mut axes) = root.remove("children").unwrap() else {
        panic!()
    };
    let Value::Struct(mut series) = axes[0].remove("children").unwrap() else {
        panic!()
    };
    let Value::Struct(mut props) = series[0].remove("properties").unwrap() else {
        panic!()
    };
    props.remove(0)
}

#[test]
fn filled_fig_markers_use_explicit_public_rgb_and_reopen_with_exact_style() {
    for kind in [PlotKind::Line, PlotKind::Scatter] {
        let mut figure = Figure::default();
        let mut series = Series::new(vec![[-0., 2.], [1., 3.]], kind);
        series.color = [17, 129, 241];
        series.explicit_color = true;
        series.markers = true;
        series.marker_shape = Some(MarkerShape::Square);
        series.marker_size = Some(9.);
        series.marker_filled = true;
        series.line_width = Some(2.);
        series.label = "literal filled marker".into();
        figure.add(vec![series]).unwrap();
        let bytes = encode_matlab_figure(&figure).unwrap();
        let props = first_properties(&bytes);
        let face = field(&props, "MarkerFaceColor").unwrap();
        assert!(
            face.text().is_none(),
            "MATLAB face auto is not series Color"
        );
        assert_eq!(face.len(), 3);
        for (i, c) in [17, 129, 241].into_iter().enumerate() {
            assert_eq!(face.number(i), Some(f64::from(c) / 255.));
        }
        let reopened = decode_matlab_figure(&bytes).unwrap();
        let actual = &reopened.series[0];
        assert_eq!(actual.kind, kind);
        assert_eq!(actual.color, [17, 129, 241]);
        assert!(actual.marker_is_filled());
        assert_eq!(actual.marker_shape, Some(MarkerShape::Square));
        assert_eq!(actual.marker_size, Some(9.));
        assert_eq!(actual.line_width, Some(2.));
        assert_eq!(actual.label, "literal filled marker");
        for (got, expected) in actual.points.iter().zip(figure.series[0].points.iter()) {
            assert_eq!(got.map(f64::to_bits), expected.map(f64::to_bits));
        }
        assert_eq!(encode_matlab_figure(&reopened).unwrap(), bytes);
    }
}

#[test]
fn unfilled_fig_marker_face_remains_none() {
    for kind in [PlotKind::Line, PlotKind::Scatter] {
        let mut figure = Figure::default();
        let mut series = Series::new(vec![[0., 2.], [1., 3.]], kind);
        series.markers = true;
        series.marker_shape = Some(MarkerShape::Square);
        series.marker_filled = false;
        figure.add(vec![series]).unwrap();
        let bytes = encode_matlab_figure(&figure).unwrap();
        assert_eq!(
            string(&first_properties(&bytes), "MarkerFaceColor", "missing").unwrap(),
            "none"
        );
        assert!(!decode_matlab_figure(&bytes).unwrap().series[0].marker_is_filled());
    }
}

fn scatter_file(face: mat::Encoded<'static>) -> Vec<u8> {
    // Synthetic supported hierarchy: not a newly generated MATLAB fixture.
    let series = node(
        "specgraph.scattergroup",
        3.,
        vec![
            ("XData", num(&[1., 2.])),
            ("YData", num(&[3., 4.])),
            ("CData", num(&[1., 0., 0.])),
            ("Marker", mat::text("s")),
            ("MarkerEdgeColor", mat::text("flat")),
            ("MarkerFaceColor", face),
            ("SizeData", num(&[64.])),
        ],
        vec![],
        &[],
    );
    mat::file(mat::structure(
        "hgS_070000",
        node(
            "figure",
            1.,
            vec![],
            vec![node("axes", 2., vec![], vec![series], &[0.; 4])],
            &[],
        ),
    ))
    .unwrap()
}

#[test]
fn scatter_face_rgb_matching_cdata_is_supported_and_invalid_modes_reject() {
    for face in [mat::text("flat"), num(&[1., 0., 0.])] {
        let f = decode_matlab_figure(&scatter_file(face)).unwrap();
        assert_eq!(f.series.len(), 1);
        assert_eq!(f.series[0].kind, PlotKind::Scatter);
        assert_eq!(f.series[0].color, [255, 0, 0]);
        assert!(f.series[0].marker_is_filled());
        assert_eq!(f.series[0].marker_size_points(), 8.);
        assert_eq!(
            f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 3.], [2., 4.]]
        );
    }
    assert!(
        !decode_matlab_figure(&scatter_file(mat::text("none")))
            .unwrap()
            .series[0]
            .marker_is_filled()
    );
    for face in [
        num(&[0., 1., 0.]),
        num(&[1., 0.]),
        num(&[f64::NAN, 0., 0.]),
        num(&[1., f64::INFINITY, 0.]),
        mat::text("auto"),
        mat::text("bogus"),
    ] {
        assert!(decode_matlab_figure(&scatter_file(face)).is_err());
    }
}

#[test]
fn authentic_uniform_flat_scatter_fixture_retains_samples_and_fill() {
    // Historical reference fixture, unchanged. It has CData=[1,0,0],
    // MarkerFaceColor=flat and MarkerEdgeColor=none; this is import evidence.
    let figure = decode_matlab_figure(include_bytes!(
        "../../tests/fixtures/fig/matlab-log-scatter.fig"
    ))
    .unwrap();
    let series = figure
        .series
        .iter()
        .find(|s| s.kind == PlotKind::Scatter)
        .unwrap();
    assert_eq!(series.color, [255, 0, 0]);
    assert!(series.marker_is_filled());
    assert_eq!(series.marker_size_points(), 6.);
    assert_eq!(
        series.points.iter().collect::<Vec<_>>().as_slice(),
        &[[1., 2.], [10., 3.], [100., 5.]]
    );
}
