//! Rare-path clipping of the existing finite mapped SVG polyline.
//! Ordinary output remains in svg.rs. No sample storage or mapping rules change.
use super::{LinePattern, PlotKind, Series, SvgBuffer};
use std::fmt::Write;

// Keep ordinary output byte-identical, but avoid feeding very large paths to
// f32/fixed-point rasterizers. Canonical figure coordinates are at most 960x560.
pub(super) fn hazardous(point: [f64; 2]) -> bool {
    point.iter().any(|v| !v.is_finite() || v.abs() > 32_768.)
}

fn margin(series: &Series) -> f64 {
    // SVG's default miter limit is 4. Four full stroke widths plus four marker
    // radii is deliberately beyond every supported join/marker footprint.
    (4. * f64::from(series.stroke_width(2.)) + 4. * f64::from(series.marker_radius()) + 4.).max(16.)
}

pub(super) fn marker_visible(series: &Series, point: [f64; 2], rect: [f64; 4]) -> bool {
    let pad = margin(series);
    point[0] >= rect[0] - pad
        && point[0] <= rect[2] + pad
        && point[1] >= rect[1] - pad
        && point[1] <= rect[3] + pad
}

// Match the legacy SVG coordinate spelling before computing segment lengths.
// Binary multiplication/rounding is not equivalent at decimal halfway values.
// This rare path uses a fixed stack buffer, with no per-point heap allocation.
fn emitted_coordinate(value: f64) -> Option<f64> {
    struct Decimal {
        bytes: [u8; 64],
        len: usize,
    }
    impl Write for Decimal {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            let end = self.len.checked_add(text.len()).ok_or(std::fmt::Error)?;
            self.bytes
                .get_mut(self.len..end)
                .ok_or(std::fmt::Error)?
                .copy_from_slice(text.as_bytes());
            self.len = end;
            Ok(())
        }
    }
    let mut text = Decimal {
        bytes: [0; 64],
        len: 0,
    };
    write!(text, "{value:.2}").ok()?;
    let rounded: f64 = std::str::from_utf8(&text.bytes[..text.len])
        .ok()?
        .parse()
        .ok()?;
    rounded.is_finite().then_some(rounded)
}

#[derive(Debug)]
struct Segment {
    start: [f64; 2],
    end: [f64; 2],
    enter: f64,
    exit: f64,
}

fn clip(a: [f64; 2], b: [f64; 2], rect: [f64; 4]) -> Option<Segment> {
    // Mapping already clamps normalized values to +/-1e6. Subtraction and
    // hypot stay finite at the fixed SVG viewport sizes; no huge data product.
    if !a.into_iter().chain(b).all(f64::is_finite) {
        return None;
    }
    let delta = [b[0] - a[0], b[1] - a[1]];
    let (mut enter, mut exit) = (0_f64, 1_f64);
    for (p, q) in [
        (-delta[0], a[0] - rect[0]),
        (delta[0], rect[2] - a[0]),
        (-delta[1], a[1] - rect[1]),
        (delta[1], rect[3] - a[1]),
    ] {
        if p == 0. {
            if q < 0. {
                return None;
            }
        } else if p < 0. {
            enter = enter.max(q / p);
        } else {
            exit = exit.min(q / p);
        }
    }
    if enter > exit {
        return None;
    }
    let point = |t| {
        [
            libm::fma(t, delta[0], a[0]).clamp(rect[0], rect[2]),
            libm::fma(t, delta[1], a[1]).clamp(rect[1], rect[3]),
        ]
    };
    Some(Segment {
        start: if enter == 0. { a } else { point(enter) },
        end: if exit == 1. { b } else { point(exit) },
        enter,
        exit,
    })
}

struct Writer<'a, 'b> {
    svg: &'a mut SvgBuffer<'b>,
    color: &'a str,
    dash: &'a str,
    width: f32,
    period: f64,
    phase: f64,
    rect: [f64; 4],
    end: Option<[f64; 2]>,
}

impl Writer<'_, '_> {
    fn close(&mut self) {
        if self.end.take().is_some() {
            let _ = self.svg.write_str("\"/>");
        }
    }

    fn gap(&mut self) {
        self.close();
        self.phase = 0.;
    }

    fn segment(&mut self, a: [f64; 2], b: [f64; 2]) {
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        if let Some(segment) = clip(a, b, self.rect) {
            if self.end != Some(segment.start) || segment.enter != 0. {
                self.close();
                let _ = write!(
                    self.svg,
                    "<path fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-dasharray=\"{}\"",
                    self.color, self.width, self.dash
                );
                if self.period > 0. {
                    // Reset each emitted SVG subpath to its original source
                    // phase. Keep only a bounded remainder, never huge offsets.
                    let offset =
                        (self.phase + (length * segment.enter) % self.period) % self.period;
                    let _ = write!(self.svg, " stroke-dashoffset=\"{offset:.9}\"");
                }
                let _ = write!(
                    self.svg,
                    " d=\"M{:.2},{:.2} ",
                    segment.start[0], segment.start[1]
                );
            }
            let _ = write!(self.svg, "L{:.2},{:.2} ", segment.end[0], segment.end[1]);
            self.end = Some(segment.end);
            if segment.exit != 1. {
                self.close();
            }
        } else {
            self.close();
        }
        if self.period > 0. {
            self.phase = (self.phase + length % self.period) % self.period;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    svg: &mut SvgBuffer<'_>,
    series: &Series,
    color: &str,
    dash: &str,
    rect: [f64; 4],
    baseline: f64,
    valid: impl Fn(f64, f64) -> bool,
    mapped: impl Fn(f64, f64) -> [f64; 2],
) {
    let pad = margin(series);
    let rect = [rect[0] - pad, rect[1] - pad, rect[2] + pad, rect[3] + pad];
    let mut writer = Writer {
        svg,
        color,
        dash,
        width: series.stroke_width(2.),
        period: match series.pattern {
            LinePattern::Dashed => 13.,
            LinePattern::Dotted => 6.,
            LinePattern::DashDot => 18.,
            _ => 0.,
        },
        phase: 0.,
        rect,
        end: None,
    };
    let mut previous: Option<[f64; 2]> = None;
    let baseline = emitted_coordinate(baseline);
    for (index, [x, y]) in series.points.iter().enumerate() {
        if writer.svg.stopped(index) {
            return;
        }
        if !valid(x, y) {
            writer.gap();
            previous = None;
            continue;
        }
        let raw = mapped(x, y);
        let (Some(px), Some(py)) = (emitted_coordinate(raw[0]), emitted_coordinate(raw[1])) else {
            writer.gap();
            previous = None;
            continue;
        };
        let point = [px, py];
        if series.kind == PlotKind::Stem {
            writer.gap();
            if let Some(baseline) = baseline {
                writer.segment([point[0], baseline], point);
            }
            writer.gap();
        } else {
            if let Some(prior) = previous {
                if series.kind == PlotKind::Stairs {
                    let corner = [point[0], prior[1]];
                    writer.segment(prior, corner);
                    writer.segment(corner, point);
                } else {
                    writer.segment(prior, point);
                }
            }
            previous = Some(point);
        }
    }
    writer.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(points: Vec<[f64; 2]>, kind: PlotKind, pattern: LinePattern) -> String {
        let mut series = Series::new(points, kind);
        series.pattern = pattern;
        let mut svg = SvgBuffer::new(String::new(), None);
        let dash = match pattern {
            LinePattern::Dashed => "8 5",
            LinePattern::Dotted => "2 4",
            LinePattern::DashDot => "8 4 2 4",
            _ => "",
        };
        draw(
            &mut svg,
            &series,
            "#ffffff",
            dash,
            [0., 0., 100., 100.],
            0.,
            |x, y| x.is_finite() && y.is_finite(),
            |x, y| [x, y],
        );
        svg.finish()
    }

    #[test]
    fn huge_segments_clip_without_endpoint_clamping_distorting_the_slope() {
        let segment = clip([-1e9, -1e9], [1e9, 1e9], [0., 0., 100., 100.]).unwrap();
        for (actual, expected) in segment
            .start
            .into_iter()
            .zip([0., 0.])
            .chain(segment.end.into_iter().zip([100., 100.]))
        {
            assert!((actual - expected).abs() < 1e-6);
        }
        assert!(clip([-1e9, -100.], [1e9, -100.], [0., 0., 100., 100.]).is_none());
        assert!(clip([50., 50.], [50., 50.], [0., 0., 100., 100.]).is_some());
    }

    #[test]
    fn clipping_preserves_contiguous_joins_but_breaks_gaps_and_stems() {
        let line = render(
            vec![[-1e9, 50.], [50., 50.], [1e9, 50.]],
            PlotKind::Line,
            LinePattern::Solid,
        );
        assert_eq!(line.matches("<path").count(), 1);
        assert!(line.contains("L50.00,50.00"));
        assert!(!line.contains("1000000000"));
        let gaps = render(
            vec![
                [-1e9, 50.],
                [50., 50.],
                [f64::NAN, 0.],
                [50., 60.],
                [1e9, 60.],
            ],
            PlotKind::Line,
            LinePattern::Solid,
        );
        assert_eq!(gaps.matches("<path").count(), 2);
        let stems = render(
            vec![[25., 1e9], [75., 1e9]],
            PlotKind::Stem,
            LinePattern::Solid,
        );
        assert_eq!(stems.matches("<path").count(), 2);
        assert!(!stems.contains("L75.00,0.00"));
    }

    #[test]
    fn stairs_keep_corners_and_dash_phase_is_bounded_across_hidden_segments() {
        let stairs = render(
            vec![[-1e9, 20.], [50., 40.], [1e9, 60.]],
            PlotKind::Stairs,
            LinePattern::Solid,
        );
        assert!(stairs.contains("L50.00,20.00 L50.00,40.00"));
        for (pattern, period) in [
            (LinePattern::Dashed, 13.),
            (LinePattern::Dotted, 6.),
            (LinePattern::DashDot, 18.),
        ] {
            let line = render(
                vec![[-40000., 50.], [50., 50.], [40000., 50.]],
                PlotKind::Line,
                pattern,
            );
            let text = line
                .split("stroke-dashoffset=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let offset: f64 = text.parse().unwrap();
            let series = Series::new(Vec::new(), PlotKind::Line);
            let expected = (40000. - margin(&series)) % period;
            assert!((offset - expected).abs() < 1e-6);
            assert!((0. ..period).contains(&offset));
        }
    }

    #[test]
    fn hidden_fractional_segments_use_legacy_emitted_coordinates_for_dash_phase() {
        let mut points: Vec<[f64; 2]> = (0..=1000)
            .map(|i| [-40000.001, if i % 2 == 0 { 50.004 } else { 50.006 }])
            .collect();
        points.push([50., 50.004]);
        let line = render(points, PlotKind::Line, LinePattern::Dashed);
        let offset: f64 = line
            .split("stroke-dashoffset=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let pad = margin(&Series::new(Vec::new(), PlotKind::Line));
        // The old textual path alternates 50.00/50.01, hence ten hidden units,
        // not the two units obtained from the unrounded source coordinates.
        let expected = (1000. * (50.01_f64 - 50.) + 40000. - pad) % 13.;
        assert!((offset - expected).abs() < 1e-6, "{offset} != {expected}");
        for value in [50.004, 50.006, -0.005, 1.125, 32768.005, -849999930.125] {
            let expected: f64 = format!("{value:.2}").parse().unwrap();
            assert_eq!(
                emitted_coordinate(value).unwrap().to_bits(),
                expected.to_bits()
            );
        }
    }

    #[test]
    fn nonfinite_mapping_breaks_the_path_without_poisoning_later_dash_phase() {
        let mut series = Series::new(
            vec![[0., 0.], [1., 0.], [2., 0.], [3., 0.], [4., 0.]],
            PlotKind::Line,
        );
        series.pattern = LinePattern::Dashed;
        let mut svg = SvgBuffer::new(String::new(), None);
        draw(
            &mut svg,
            &series,
            "#fff",
            "8 5",
            [0., 0., 100., 100.],
            f64::INFINITY,
            |_, _| true,
            |x, _| {
                if x == 2. {
                    [f64::NAN, 50.]
                } else {
                    [x * 10., 50.]
                }
            },
        );
        let output = svg.finish();
        assert_eq!(output.matches("<path").count(), 2);
        assert!(!output.contains("NaN") && !output.contains("inf"));
        assert_eq!(
            output.matches("stroke-dashoffset=\"0.000000000\"").count(),
            2
        );
        series.kind = PlotKind::Stem;
        let mut svg = SvgBuffer::new(String::new(), None);
        draw(
            &mut svg,
            &series,
            "#fff",
            "8 5",
            [0., 0., 100., 100.],
            f64::INFINITY,
            |_, _| true,
            |x, y| [x, y],
        );
        assert!(svg.finish().is_empty());
    }

    #[cfg(feature = "plot-export")]
    #[test]
    fn rendered_clipped_dash_phase_matches_an_unclipped_short_reference() {
        fn pixels(path: &str) -> Vec<u8> {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\" viewBox=\"0 0 100 100\">{path}</svg>"
            );
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
            let mut pixmap = resvg::tiny_skia::Pixmap::new(100, 100).unwrap();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::identity(),
                &mut pixmap.as_mut(),
            );
            pixmap.data().to_vec()
        }
        let clipped = render(
            vec![[-40000., 50.], [40000., 50.]],
            PlotKind::Line,
            LinePattern::Dashed,
        );
        // 40000 = 3076 complete 13-unit periods + 12. A path starting at -12
        // is therefore an independent short-geometry reference in this viewport.
        // No offset property appears in the reference; the renderer decides ink.
        let expected = pixels(
            "<path d=\"M-12 50H112\" fill=\"none\" stroke=\"#ffffff\" stroke-width=\"2\" stroke-dasharray=\"8 5\"/>",
        );
        let actual = pixels(&clipped);
        assert!(
            actual
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        assert_eq!(actual[(50 * 100 + 3) * 4 + 3], 255);
        assert_eq!(actual[(50 * 100 + 11) * 4 + 3], 0);
    }

    #[test]
    fn guard_includes_maximum_marker_and_stroke_footprints() {
        let mut series = Series::new(Vec::new(), PlotKind::Line);
        series.marker_size = Some(100.);
        series.line_width = Some(50.);
        assert!(
            margin(&series)
                > f64::from(series.marker_radius()) + 2. * f64::from(series.stroke_width(1.))
        );
        assert!(marker_visible(
            &series,
            [-f64::from(series.marker_radius()), 50.],
            [0., 0., 100., 100.]
        ));
        assert!(!marker_visible(&series, [1e9, 50.], [0., 0., 100., 100.]));
    }
}
