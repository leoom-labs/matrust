//! histogram, histcounts options, errorbar, area, barh, pie, fill, polarplot,
//! fplot and heatmap. Data first, then SVG content.
//! Reference values: NumPy (tool/refs/plots_2d/histogram_refs.py) where
//! MATLAB's documented binning rule and numpy.histogram_bin_edges coincide;
//! the rest follow from the documented definitions (stated per test).
use matrust::{Array, Engine, plot::PlotKind};

fn num(e: &Engine, name: &str) -> Vec<f64> {
    match e.array(name) {
        Some(Array::Numeric(m)) => m.data().to_vec(),
        other => panic!("{name} is not numeric: {other:?}"),
    }
}
fn text(e: &Engine, name: &str) -> String {
    match e.array(name) {
        Some(Array::Character(c)) => c.to_utf8(&e.cancellation()).unwrap(),
        other => panic!("{name} is not text: {other:?}"),
    }
}
fn close(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() <= 1e-12 * y.abs().max(1.), "{a:?} vs {b:?}");
    }
}
fn run(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source)
        .unwrap_or_else(|err| panic!("{source}: {err}"));
    e
}
fn labels(e: &Engine) -> Vec<String> {
    e.figure
        .as_ref()
        .unwrap()
        .textboxes
        .iter()
        .map(|t| t.text.clone())
        .collect()
}
/// Area of a triangle list.
fn area(points: &[[f64; 2]]) -> f64 {
    points
        .chunks(3)
        .map(|c| {
            0.5 * ((c[1][0] - c[0][0]) * (c[2][1] - c[0][1])
                - (c[1][1] - c[0][1]) * (c[2][0] - c[0][0]))
                .abs()
        })
        .sum()
}

const X: &str = "x=[0.5 1.25 2 2 3.75 4 9.5 -1];";

#[test]
fn bin_edges_and_counts_match_numpy_where_the_rules_coincide() {
    // numpy.histogram_bin_edges(x, 5, range=(0, 4)) and numpy.histogram counts.
    let e = run(&format!(
        "{X}[n,ed]=histcounts(x,'BinLimits',[0 4],'NumBins',5);h=histogram(x,'BinLimits',[0 4],'NumBins',5);hv=get(h,'Values');he=get(h,'BinEdges');"
    ));
    close(&num(&e, "ed"), &[0., 0.8, 1.6, 2.4000000000000004, 3.2, 4.]);
    close(&num(&e, "n"), &[1., 1., 2., 0., 2.]);
    close(&num(&e, "hv"), &num(&e, "n"));
    close(&num(&e, "he"), &num(&e, "ed"));
}

#[test]
fn every_normalization_follows_its_definition() {
    // Explicit edges; numpy counts [1 2 3 2], numpy density [0.125 0.125 0.1875 0.041666...].
    let e = run(&format!(
        "{X}ed=[-1 0 2 4 10];h=histogram(x,ed);c=get(h,'BinCounts');v=get(h,'Values');\
         p=histcounts(x,ed,'Normalization','probability');d=histcounts(x,ed,'Normalization','pdf');\
         f=histcounts(x,ed,'Normalization','cdf');k=histcounts(x,ed,'Normalization','countdensity');\
         q=histcounts(x,ed,'Normalization','cumcount');set(h,'Normalization','pdf');hp=get(h,'Values');nm=get(h,'Normalization');"
    ));
    close(&num(&e, "c"), &[1., 2., 3., 2.]);
    close(&num(&e, "v"), &[1., 2., 3., 2.]);
    close(&num(&e, "p"), &[0.125, 0.25, 0.375, 0.25]);
    close(&num(&e, "d"), &[0.125, 0.125, 0.1875, 0.041666666666666664]);
    close(&num(&e, "f"), &[0.125, 0.375, 0.75, 1.]);
    close(&num(&e, "k"), &[1., 1., 1.5, 1. / 3.]);
    close(&num(&e, "q"), &[1., 3., 6., 8.]);
    close(&num(&e, "hp"), &num(&e, "d"));
    assert_eq!(text(&e, "nm"), "pdf");
}

#[test]
fn automatic_width_and_count_binning_rules() {
    // Integer data spanning at most 50: one bin per integer, centred on it.
    let e = run(
        "h=histogram([1 2 2 3 3 3]);e1=get(h,'BinEdges');v1=get(h,'Values');t=get(h,'Type');\
                 h2=histogram([0.3 1.2],'BinWidth',0.5);e2=get(h2,'BinEdges');w=get(h2,'BinWidth');\
                 h3=histogram([0.3 1.2 7],'BinLimits',[0 2],'BinWidth',0.75);e3=get(h3,'BinEdges');c3=get(h3,'BinCounts');",
    );
    close(&num(&e, "e1"), &[0.5, 1.5, 2.5, 3.5]);
    close(&num(&e, "v1"), &[1., 2., 3.]);
    assert_eq!(text(&e, "t"), "histogram");
    close(&num(&e, "e2"), &[0., 0.5, 1., 1.5]);
    close(&num(&e, "w"), &[0.5]);
    // Limits with a width: the last bin ends at the upper limit.
    close(&num(&e, "e3"), &[0., 0.75, 1.5, 2.]);
    close(&num(&e, "c3"), &[1., 1., 0.]);
}

#[test]
fn histogram_draws_one_face_per_bin_and_restyles_through_set() {
    let mut e = run("h=histogram([1 2 2 3 3 3]);");
    {
        let f = e.figure.as_ref().unwrap();
        assert_eq!(f.series.len(), 1);
        assert_eq!(f.series[0].kind, PlotKind::Patch);
        assert_eq!(f.series[0].points.len(), 12);
        let p: Vec<_> = f.series[0].points.iter().collect();
        assert_eq!(&p[4..8], &[[1.5, 0.], [1.5, 2.], [2.5, 2.], [2.5, 0.]]);
    }
    e.execute("set(h,'NumBins',2);n=get(h,'NumBins');set(h,'DisplayStyle','stairs');s=get(h,'DisplayStyle');").unwrap();
    assert_eq!(num(&e, "n"), vec![2.]);
    assert_eq!(text(&e, "s"), "stairs");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 1);
    assert_eq!(f.series[0].kind, PlotKind::Line);
    // A failed option leaves the chart unchanged.
    let before = f.clone();
    assert!(e.execute("set(h,'Normalization','percent');").is_err());
    assert!(
        e.execute("histogram([1 2],'Normalization','percent');")
            .is_err()
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    // Name-value histcounts and plain forms still agree.
    let e = run("a=histcounts([1 2 2 5],3);b=histcounts([1 2 2 5],'NumBins',3);");
    assert_eq!(num(&e, "a"), num(&e, "b"));
}

#[test]
fn errorbar_forms_build_bars_and_caps() {
    let e = run(
        "h=errorbar([1 2 3],[2 4 3],[0.5 1 0.5]);t=get(h,'Type');yn=get(h,'YNegativeDelta');cs=get(h,'CapSize');",
    );
    assert_eq!(text(&e, "t"), "errorbar");
    assert_eq!(num(&e, "yn"), vec![0.5, 1., 0.5]);
    assert_eq!(num(&e, "cs"), vec![6.]);
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 2);
    let line: Vec<_> = f.series[0].points.iter().collect();
    assert_eq!(line, vec![[1., 2.], [2., 4.], [3., 3.]]);
    let bars: Vec<_> = f.series[1].points.iter().collect();
    // Per point: bar, lower cap, upper cap, each followed by a gap.
    assert_eq!(bars.len(), 3 * 9);
    assert_eq!(&bars[..2], &[[1., 1.5], [1., 2.5]]);
    assert_eq!((bars[3][1], bars[4][1]), (1.5, 1.5));
    assert!(bars[3][0] < 1. && bars[4][0] > 1.);
    // Caps are 6 points wide: about 8 px of a 2-unit span over 850 px.
    let cap = bars[4][0] - bars[3][0];
    assert!((cap - 8. * 2. / 850.).abs() < 1e-12, "{cap}");
    assert!(f.series[1].label.is_empty(), "helpers stay out of legends");
    assert_eq!(f.series[0].color, f.series[1].color);
    // neg/pos, horizontal and both.
    let e = run(
        "h=errorbar(1:2,[5 6],[1 2],[3 4]);a=get(h,'YPositiveDelta');\
                 g=errorbar(1:2,[5 6],[1 1],'horizontal');b=get(g,'XNegativeDelta');c=get(g,'YNegativeDelta');\
                 k=errorbar(1:2,[5 6],[1 1],'both','CapSize',0);",
    );
    assert_eq!(num(&e, "a"), vec![3., 4.]);
    assert_eq!(num(&e, "b"), vec![1., 1.]);
    assert!(num(&e, "c").is_empty());
    let f = e.figure.as_ref().unwrap();
    // Two points, a vertical and a horizontal bar each, no caps.
    assert_eq!(f.series[1].points.len(), 2 * 2 * 3);
    assert!(Engine::default().execute("errorbar(1:3,[1 2]);").is_err());
}

#[test]
fn stacked_area_layers_sum_the_columns() {
    let e = run("h=area([1 2 3],[1 2;3 4;5 6]);y2=get(h(2),'YData');t=get(h(1),'Type');");
    assert_eq!(text(&e, "t"), "area");
    assert_eq!(num(&e, "y2"), vec![2., 4., 6.]);
    let f = e.figure.as_ref().unwrap();
    let patches: Vec<_> = f
        .series
        .iter()
        .filter(|s| s.kind == PlotKind::Patch)
        .collect();
    assert_eq!(patches.len(), 2);
    // Trapezoid areas: layer 1 = (1+3)/2+(3+5)/2 = 6, layer 2 = (2+4)/2+(4+6)/2 = 8.
    let a1 = area(&patches[0].points.iter().collect::<Vec<_>>());
    let a2 = area(&patches[1].points.iter().collect::<Vec<_>>());
    assert!(
        (a1 - 6.).abs() < 1e-12 && (a2 - 8.).abs() < 1e-12,
        "{a1} {a2}"
    );
    // The top of the last layer is the row sum.
    let top = f.series[3]
        .points
        .iter()
        .take(3)
        .map(|p| p[1])
        .collect::<Vec<_>>();
    assert_eq!(top, vec![3., 7., 11.]);
    // A crossing of the base splits the band: |area| of y = [-1 1] over [0 1] is 1/2.
    let e = run("area([0 1],[-1 1]);");
    let f = e.figure.as_ref().unwrap();
    let a = area(&f.series[0].points.iter().collect::<Vec<_>>());
    assert!((a - 0.5).abs() < 1e-12, "{a}");
    let e = run("h=area([1 2],[3 3],'BaseValue',1);b=get(h,'BaseValue');");
    assert_eq!(num(&e, "b"), vec![1.]);
    let a = area(
        &e.figure.as_ref().unwrap().series[0]
            .points
            .iter()
            .collect::<Vec<_>>(),
    );
    assert!((a - 2.).abs() < 1e-12);
}

#[test]
fn barh_rotates_single_grouped_and_stacked_bars() {
    let e = run("h=barh([3 5 2]);t=get(h,'Type');o=get(h,'Horizontal');");
    assert_eq!(text(&e, "t"), "bar");
    assert_eq!(text(&e, "o"), "on");
    let f = e.figure.as_ref().unwrap();
    let p: Vec<_> = f.series[0].points.iter().collect();
    // Bar 1 spans x 0..3 at y = 1 +- 0.4.
    assert_eq!(&p[..4], &[[0., 0.6], [3., 0.6], [3., 1.4], [0., 1.4]]);
    let e = run("h=barh([1 2;3 4],'stacked');");
    let f = e.figure.as_ref().unwrap();
    let second: Vec<_> = f.series[1].points.iter().collect();
    assert_eq!((second[0][0], second[1][0]), (1., 3.));
    let e = run("h=barh([1 2;3 4]);");
    let f = e.figure.as_ref().unwrap();
    let (a, b) = (
        f.series[0].points.get(0).unwrap(),
        f.series[1].points.get(0).unwrap(),
    );
    // Grouped: two thinner bars side by side within the 0.8*(2/3.5) group.
    assert!(a[1] < b[1]);
    assert!(f.series[0].color != f.series[1].color);
}

#[test]
fn pie_percentages_partial_pies_and_labels() {
    let e = run("h=pie([1 2 3 4]);n=numel(h);t1=get(h(1),'Type');");
    assert_eq!(num(&e, "n"), vec![8.]);
    assert_eq!(text(&e, "t1"), "patch");
    assert_eq!(labels(&e), vec!["10%", "20%", "30%", "40%"]);
    let f = e.figure.as_ref().unwrap();
    let slices: Vec<_> = f
        .series
        .iter()
        .filter(|s| s.kind == PlotKind::Patch)
        .collect();
    assert_eq!(slices.len(), 4);
    // Slice areas are the fractions of the unit disc (2-degree polygon steps).
    let total: f64 = slices
        .iter()
        .map(|s| area(&s.points.iter().collect::<Vec<_>>()))
        .sum();
    let polygon = 90. * (std::f64::consts::PI / 90.).sin();
    assert!((total - polygon).abs() < 0.02, "{total}");
    let a4 = area(&slices[3].points.iter().collect::<Vec<_>>());
    assert!((a4 / total - 0.4).abs() < 0.01);
    // Sum below one: a partial pie; small slices label outside.
    let e = run("pie([0.1 0.2 0.004]);");
    assert_eq!(labels(&e), vec!["10%", "20%", "< 1%"]);
    let e = run("pie([2 2],[0 1],{'left','right'});");
    assert_eq!(labels(&e), vec!["left", "right"]);
    // A new plot removes the chart's labels.
    let mut e = run("pie([1 1]);");
    e.execute("plot(1:3);").unwrap();
    assert!(labels(&e).is_empty());
    assert!(Engine::default().execute("pie([1 -1]);").is_err());
}

#[test]
fn fill_triangulates_polygons_with_constant_colour() {
    let e = run("h=fill([0 2 2 1 1 0],[0 0 1 1 2 2],'r');t=get(h,'Type');");
    assert_eq!(text(&e, "t"), "patch");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series[0].color, [255, 0, 0]);
    let a = area(&f.series[0].points.iter().collect::<Vec<_>>());
    assert!((a - 3.).abs() < 1e-12);
    // Outline helper closes the polygon.
    let outline: Vec<_> = f.series[1].points.iter().collect();
    assert_eq!(outline.first(), outline.last());
    let e = run("fill([0 1 0],[0 0 1],[0 0.5 1],[2 3 2],[0 0 1],'g');");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series[0].color, [0, 128, 255]);
    assert_eq!(f.series[2].color, [0, 255, 0]);
}

#[test]
fn polarplot_maps_angles_and_follows_axes_properties() {
    let mut e = run(
        "h=polarplot([0 pi/2],[1 2]);t=get(h,'Type');r=get(gca,'RLim');z=get(gca,'ThetaZeroLocation');td=get(h,'ThetaData');",
    );
    assert_eq!(text(&e, "t"), "line");
    assert_eq!(num(&e, "r"), vec![0., 2.]);
    assert_eq!(text(&e, "z"), "right");
    close(&num(&e, "td"), &[0., std::f64::consts::FRAC_PI_2]);
    {
        let f = e.figure.as_ref().unwrap();
        // Grid first (behind the data), with an empty legend label.
        assert!(f.series[0].label.is_empty());
        let p: Vec<_> = f.series[1].points.iter().collect();
        assert!((p[0][0] - 1.).abs() < 1e-15 && p[0][1].abs() < 1e-15);
        assert!(p[1][0].abs() < 1e-15 && (p[1][1] - 2.).abs() < 1e-15);
        let l = labels(&e);
        assert!(l.contains(&"90°".to_string()) && l.contains(&"330°".to_string()));
    }
    e.execute(
        "set(gca,'ThetaZeroLocation','top','ThetaDir','clockwise','RLim',[0 4]);r=get(gca,'RLim');",
    )
    .unwrap();
    assert_eq!(num(&e, "r"), vec![0., 4.]);
    let f = e.figure.as_ref().unwrap();
    let p: Vec<_> = f.series[1].points.iter().collect();
    // theta 0 points up; theta pi/2 clockwise points right.
    assert!(p[0][0].abs() < 1e-15 && (p[0][1] - 1.).abs() < 1e-15);
    assert!((p[1][0] - 2.).abs() < 1e-15 && p[1][1].abs() < 1e-15);
    assert!(labels(&e).contains(&"4".to_string()));
    // Held polar axes take more lines and rescale; Cartesian axes refuse them.
    let e = run("polarplot([0 1],[1 1]);hold on;polarplot([0 1],[3 3],'r--');r=get(gca,'RLim');");
    assert_eq!(num(&e, "r"), vec![0., 3.]);
    assert!(
        Engine::default()
            .execute("plot(1:3);hold on;polarplot([0 1],[1 1]);")
            .is_err()
    );
}

#[test]
fn fplot_samples_include_the_limits_and_refine_curvature() {
    let e = run(
        "h=fplot(@(x) x.^2,[-1 2]);x=get(h,'XData');t=get(h,'Type');r=get(h,'XRange');f=get(h,'Function');",
    );
    let x = num(&e, "x");
    assert_eq!((x[0], *x.last().unwrap()), (-1., 2.));
    // 23 initial points, 22 tested midpoints, then further bisection where it bends.
    assert!(x.len() > 45 && x.windows(2).all(|w| w[1] > w[0]));
    assert_eq!(text(&e, "t"), "functionline");
    assert_eq!(num(&e, "r"), vec![-1., 2.]);
    assert!(text(&e, "f").contains("x.^2"));
    // A straight line stops after the first midpoint test; the default interval is [-5 5].
    let e = run("h=fplot(@(x) 2*x+1);x=get(h,'XData');");
    let x = num(&e, "x");
    assert_eq!(x.len(), 45);
    assert_eq!((x[0], x[44]), (-5., 5.));
    // Non-vectorized handles are evaluated point by point; set XRange resamples.
    let e = run("h=fplot(@(x) max(x,0),[0 1]);set(h,'XRange',[2 3]);x=get(h,'XData');");
    let x = num(&e, "x");
    assert_eq!((x[0], *x.last().unwrap()), (2., 3.));
}

#[test]
fn heatmap_cells_carry_their_values_and_labels() {
    let e =
        run("h=heatmap([1 2;3 4]);c=get(h,'ColorData');t=get(h,'Type');l=get(h,'ColorLimits');");
    assert_eq!(num(&e, "c"), vec![1., 3., 2., 4.]);
    assert_eq!(text(&e, "t"), "heatmap");
    assert_eq!(num(&e, "l"), vec![1., 4.]);
    let f = e.figure.as_ref().unwrap();
    let find = |s: &str| {
        f.textboxes
            .iter()
            .find(|t| t.text == s && t.font_size == 9.)
            .map(|t| t.position)
    };
    let labels: Vec<_> = f.textboxes.iter().map(|t| t.text.as_str()).collect();
    // Column labels 1 2, row labels 1 2, colour-scale limits 1 4, cell values 1 2 3 4.
    assert_eq!(labels.len(), 2 + 2 + 2 + 4);
    // The cell of row 1 (value 2 in column 2) sits above row 2's value 4.
    let cells: Vec<_> = f.textboxes.iter().rev().take(4).collect();
    let pos = |v: &str| cells.iter().find(|t| t.text == v).unwrap().position;
    assert!(pos("2")[1] > pos("4")[1] && pos("1")[0] < pos("2")[0]);
    assert!(find("4").is_some());
    // Colour levels: low values light, high values dark.
    let patches: Vec<_> = f
        .series
        .iter()
        .filter(|s| s.kind == PlotKind::Patch)
        .collect();
    let lum = |c: [u8; 3]| c.iter().map(|&v| f64::from(v)).sum::<f64>();
    assert!(lum(patches[0].color) > lum(patches.last().unwrap().color));
    let e = run(
        "heatmap({'a','b','c'},{'r1','r2'},[1 2 3;4 5 NaN],'CellLabelColor','none','Title','T');",
    );
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.axes.title, "T");
    let labels: Vec<_> = f.textboxes.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(labels, vec!["a", "b", "c", "r1", "r2", "1", "5"]);
}

#[test]
fn svg_exports_contain_the_chart_elements() {
    let e = run("histogram([1 2 2 3 3 3]);");
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("font-size=\"12\""));
    assert_eq!(svg.matches("<polygon").count(), 3, "{svg}");
    let e = run("pie([1 2 3 4]);");
    let svg = e.figure.as_ref().unwrap().svg();
    for label in ["10%", "20%", "30%", "40%"] {
        assert!(svg.contains(&format!(">{label}</text>")), "{label}");
    }
    // Pie, polar and heatmap charts hide the Cartesian tick labels.
    assert!(!svg.contains("font-size=\"12\""));
    assert!(e.figure.as_ref().unwrap().axes.ticks_hidden);
    let mut e = e;
    e.execute("plot(1:3);").unwrap();
    assert!(!e.figure.as_ref().unwrap().axes.ticks_hidden);
    let e = run("polarplot(linspace(0,2*pi,50),ones(1,50));");
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains(">90°</text>") && svg.contains(">180°</text>"));
    let e = run("heatmap([1 2;3 4]);");
    let svg = e.figure.as_ref().unwrap().svg();
    // Four cells and sixteen colour-scale steps.
    assert_eq!(svg.matches("<polygon").count(), 4 + 16);
    let e = run("errorbar([1 2 3],[2 4 3],[0.5 1 0.5]);");
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.matches("<path").count() >= 2);
    let e = run("barh([3 5 2]);");
    let svg = e.figure.as_ref().unwrap().svg();
    assert_eq!(svg.matches("<polygon").count(), 3);
}

// ------------------------------------------------------------ fix round 1

#[test]
fn a_plot_that_replaces_polar_axes_ends_polar_mode() {
    // plot reuses the axes identity: the polar record must not survive it.
    let mut e = run("polarplot([0 pi/2],[1 2]);plot(1:3);t=get(gca,'Type');");
    assert_eq!(text(&e, "t"), "axes");
    // RLim no longer applies, and no polar grid comes back.
    assert!(e.execute("set(gca,'RLim',[0 4]);").is_err());
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 1);
    assert!(!f.axes.ticks_hidden);
    assert!(labels(&e).is_empty(), "polar labels are gone");
    // Same through the chart path (histogram) and for a later held polarplot.
    let e = run("polarplot([0 1],[1 1]);histogram([1 2 2]);t=get(gca,'Type');");
    assert_eq!(text(&e, "t"), "axes");
    assert!(
        Engine::default()
            .execute("polarplot([0 1],[1 1]);plot(1:3);hold on;polarplot([0 1],[1 1]);")
            .is_err()
    );
    // A new polarplot without hold makes the axes polar again; held polar stays polar.
    let e = run(
        "polarplot([0 1],[1 1]);plot(1:3);polarplot([0 1],[2 2]);hold on;polarplot([0 1],[3 3]);t=get(gca,'Type');r=get(gca,'RLim');",
    );
    assert_eq!(text(&e, "t"), "polaraxes");
    assert_eq!(num(&e, "r"), vec![0., 3.]);
}

#[test]
fn errorbar_orientation_applies_to_every_form() {
    // (form, expected [yneg, ypos, xneg, xpos]) for points (1,5) and (2,6).
    let none: Vec<f64> = Vec::new();
    let cases: [(&str, [Vec<f64>; 4]); 9] = [
        (
            "[5 6],[1 2],'vertical'",
            [vec![1., 2.], vec![1., 2.], none.clone(), none.clone()],
        ),
        (
            "[5 6],[1 2],'horizontal'",
            [none.clone(), none.clone(), vec![1., 2.], vec![1., 2.]],
        ),
        (
            "[5 6],[1 2],'both'",
            [vec![1., 2.], vec![1., 2.], vec![1., 2.], vec![1., 2.]],
        ),
        (
            "1:2,[5 6],[1 2],'both'",
            [vec![1., 2.], vec![1., 2.], vec![1., 2.], vec![1., 2.]],
        ),
        (
            "1:2,[5 6],[1 2],[3 4],'both'",
            [vec![1., 2.], vec![3., 4.], vec![1., 2.], vec![3., 4.]],
        ),
        (
            "1:2,[5 6],[1 2],[3 4],[5 6],[7 8]",
            [vec![1., 2.], vec![3., 4.], vec![5., 6.], vec![7., 8.]],
        ),
        (
            "1:2,[5 6],[1 2],[3 4],[5 6],[7 8],'both'",
            [vec![1., 2.], vec![3., 4.], vec![5., 6.], vec![7., 8.]],
        ),
        (
            "1:2,[5 6],[1 2],[3 4],[5 6],[7 8],'vertical'",
            [vec![1., 2.], vec![3., 4.], none.clone(), none.clone()],
        ),
        (
            "1:2,[5 6],[1 2],[3 4],[5 6],[7 8],'horizontal'",
            [none.clone(), none.clone(), vec![5., 6.], vec![7., 8.]],
        ),
    ];
    for (form, [yn, yp, xn, xp]) in cases {
        let e = run(&format!(
            "h=errorbar({form},'CapSize',0);x=get(h,'XData');yn=get(h,'YNegativeDelta');yp=get(h,'YPositiveDelta');xn=get(h,'XNegativeDelta');xp=get(h,'XPositiveDelta');"
        ));
        assert_eq!(num(&e, "x"), vec![1., 2.], "{form}");
        assert_eq!(
            [num(&e, "yn"), num(&e, "yp"), num(&e, "xn"), num(&e, "xp")],
            [yn.clone(), yp, xn.clone(), xp],
            "{form}"
        );
        // One segment (bar, gap) per drawn direction and point; no caps.
        let directions = usize::from(!yn.is_empty()) + usize::from(!xn.is_empty());
        let f = e.figure.as_ref().unwrap();
        assert_eq!(f.series[1].points.len(), 2 * directions * 3, "{form}");
    }
    for bad in [
        "errorbar(1:2,[5 6],[1 2],'both','vertical');",
        "errorbar(1:2,[5 6],[1 2],[3 4],[5 6]);",
    ] {
        assert!(Engine::default().execute(bad).is_err(), "{bad}");
    }
}

#[test]
fn fplot_falls_back_to_points_when_the_vector_call_errors() {
    // x^2 on a row vector is an error (non-square matrix power).
    let e = run("h=fplot(@(x) x^2,[-1 2]);x=get(h,'XData');y=get(h,'YData');m=lasterr;");
    let (x, y) = (num(&e, "x"), num(&e, "y"));
    assert_eq!((x[0], *x.last().unwrap()), (-1., 2.));
    assert!(x.len() > 45, "refined like the vectorized x.^2");
    close(&y, &x.iter().map(|v| v * v).collect::<Vec<_>>());
    assert_eq!(
        text(&e, "m"),
        "",
        "the swallowed vector error leaves no lasterr"
    );
    // An error on scalars too is still reported.
    assert!(
        Engine::default()
            .execute("fplot(@(x) [x x] * [1 2 3]);")
            .is_err()
    );
}

#[test]
fn histogram_face_alpha_tints_from_the_stored_face() {
    let mut e = run("h=histogram([1 2 2 3 3 3],'FaceColor',[1 0 0]);");
    let drawn = |e: &Engine| e.figure.as_ref().unwrap().series[0].color;
    // tint = face*alpha + 255*(1-alpha); default FaceAlpha 0.6.
    assert_eq!(drawn(&e), [255, 102, 102]);
    for (alpha, rgb) in [
        ("1", [255, 0, 0]),
        ("0", [255, 255, 255]),
        ("0.6", [255, 102, 102]),
        ("0", [255, 255, 255]),
        ("1", [255, 0, 0]),
    ] {
        e.execute(&format!("set(h,'FaceAlpha',{alpha});c=get(h,'FaceColor');"))
            .unwrap();
        assert_eq!(drawn(&e), rgb, "FaceAlpha {alpha}");
        assert_eq!(num(&e, "c"), vec![1., 0., 0.], "FaceAlpha {alpha}");
    }
    // A new FaceColor set at alpha 0 shows once the alpha rises.
    e.execute("set(h,'FaceAlpha',0);set(h,'FaceColor',[0 0 1]);set(h,'FaceAlpha',1);c=get(h,'FaceColor');")
        .unwrap();
    assert_eq!(drawn(&e), [0, 0, 255]);
    assert_eq!(num(&e, "c"), vec![0., 0., 1.]);
    // Rebinning keeps the face too.
    e.execute("set(h,'NumBins',2);").unwrap();
    assert_eq!(drawn(&e), [0, 0, 255]);
}

#[test]
fn errorbar_xdata_and_ydata_are_settable() {
    let mut e = run(
        "h=errorbar([1 2 3],[2 4 3],[0.5 1 0.5]);set(h,'YData',[1 1 1]);set(h,'XData',[4 5 6]);x=get(h,'XData');y=get(h,'YData');",
    );
    assert_eq!(num(&e, "x"), vec![4., 5., 6.]);
    assert_eq!(num(&e, "y"), vec![1., 1., 1.]);
    {
        let f = e.figure.as_ref().unwrap();
        let line: Vec<_> = f.series[0].points.iter().collect();
        assert_eq!(line, vec![[4., 1.], [5., 1.], [6., 1.]]);
        let bars: Vec<_> = f.series[1].points.iter().collect();
        assert_eq!(&bars[..2], &[[4., 0.5], [4., 1.5]]);
    }
    // A new point count needs matching deltas in the same call; a lone
    // mismatched XData fails and leaves the chart unchanged.
    let before = e.figure.clone().unwrap();
    assert!(e.execute("set(h,'XData',1:2);").is_err());
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute(
        "set(h,'XData',1:2,'YData',[3 4],'YNegativeDelta',[1 1],'YPositiveDelta',[2 2]);y=get(h,'YData');",
    )
    .unwrap();
    assert_eq!(num(&e, "y"), vec![3., 4.]);
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series[0].points.len(), 2);
    let bars: Vec<_> = f.series[1].points.iter().collect();
    assert_eq!(&bars[..2], &[[1., 2.], [1., 5.]]);
}

#[test]
fn help_says_pie_polar_and_heatmap_hide_the_tick_labels() {
    for (name, source) in [
        ("pie", "pie([1 2 3]);"),
        ("polarplot", "polarplot([0 1],[1 2]);"),
        ("heatmap", "heatmap([1 2;3 4]);"),
    ] {
        let mut e = run(source);
        assert!(e.figure.as_ref().unwrap().axes.ticks_hidden, "{name}");
        let page = e
            .execute(&format!("help {name}"))
            .unwrap()
            .output
            .join("\n");
        assert!(
            page.contains("with the tick labels hidden"),
            "{name}: {page}"
        );
        assert!(!page.contains("stay visible"), "{name}");
    }
}

// ------------------------------------------------- legend membership of helpers
// MATLAB (errorbar / legend reference pages): an errorbar is one object with one
// legend entry; helper graphics carry IconDisplayStyle 'off' and are never
// legend members.

fn legend_texts(e: &Engine) -> Vec<String> {
    e.figure
        .as_ref()
        .unwrap()
        .axes
        .legend_entries()
        .map(|(_, s)| s.label.clone())
        .collect()
}

#[test]
fn errorbar_takes_one_legend_label() {
    let e = run("errorbar(1:3,[2 3 4],[.1 .2 .1]); lg=legend('measured'); s=get(lg,'String');");
    assert_eq!(legend_texts(&e), ["measured"]);
    match e.array("s") {
        Some(Array::Cell(c)) => assert_eq!(c.len(), 1, "String lists only the errorbar"),
        Some(Array::Character(c)) => {
            assert_eq!(c.to_utf8(&e.cancellation()).unwrap(), "measured")
        }
        other => panic!("legend String: {other:?}"),
    }
    let f = e.figure.as_ref().unwrap();
    assert!(f.series.len() > 1, "errorbar draws helper series");
    assert!(f.series[1..].iter().all(|s| s.legend_hidden));
    assert!(!f.series[0].legend_hidden);
}

#[test]
fn polarplot_legend_names_the_data_not_the_grid() {
    let e = run("t=0:.1:2*pi; polarplot(t, 1+0*t); legend('a');");
    assert_eq!(legend_texts(&e), ["a"]);
    let f = e.figure.as_ref().unwrap();
    let (_, member) = f.axes.legend_members().next().unwrap();
    assert!(!member.legend_hidden);
    assert_eq!(f.axes.legend_members().count(), 1);
}

#[test]
fn automatic_labels_count_charts_not_helpers() {
    let e =
        run("errorbar(1:3,[2 3 4],[.1 .2 .1]); hold on; errorbar(1:3,[3 4 5],[.2 .1 .2]); legend;");
    assert_eq!(legend_texts(&e), ["data1", "data2"]);
    let e = run(
        "errorbar(1:3,[2 3 4],[.1 .2 .1]); hold on; errorbar(1:3,[3 4 5],[.2 .1 .2]); legend('a','b');",
    );
    assert_eq!(legend_texts(&e), ["a", "b"]);
    // Auto-update: a chart drawn after the legend joins it without its helpers.
    let e = run(
        "errorbar(1:3,[2 3 4],[.1 .2 .1]); legend('a'); hold on; errorbar(1:3,[3 4 5],[.2 .1 .2]);",
    );
    assert_eq!(legend_texts(&e), ["a", "data2"]);
}

#[test]
fn plain_plot_legends_are_unchanged() {
    let e = run("plot(1:3,[1 2 3]); hold on; plot(1:3,[3 2 1]); legend;");
    assert_eq!(legend_texts(&e), ["data1", "data2"]);
    let e = run("plot(1:3,[1 2 3],1:3,[3 2 1]); legend('up','down');");
    assert_eq!(legend_texts(&e), ["up", "down"]);
    assert!(
        e.figure
            .as_ref()
            .unwrap()
            .series
            .iter()
            .all(|s| !s.legend_hidden)
    );
}

#[test]
fn saved_figures_keep_helpers_out_of_the_legend() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("errorbar(1:3,[2 3 4],[.1 .2 .1]); savefig('eb.mrfig');")
        .unwrap();
    let before: Vec<bool> = e
        .figure
        .as_ref()
        .unwrap()
        .series
        .iter()
        .map(|s| s.legend_hidden)
        .collect();
    assert!(before.iter().skip(1).all(|&h| h) && !before[0]);
    let bytes = std::fs::read(dir.path().join("eb.mrfig")).unwrap();
    assert!(
        bytes
            .windows(br#""icon_display_style":"off""#.len())
            .any(|w| w == br#""icon_display_style":"off""#)
    );
    let loaded = matrust::files::load_figure(&dir.path().join("eb.mrfig")).unwrap();
    let after: Vec<bool> = loaded.series.iter().map(|s| s.legend_hidden).collect();
    assert_eq!(before, after);
    assert_eq!(loaded.axes.legend_members().count(), 1);
    // A plain plot writes no flag, so its file is byte-identical to before.
    e.execute("figure; plot(1:3); savefig('p.mrfig');").unwrap();
    let plain = std::fs::read(dir.path().join("p.mrfig")).unwrap();
    assert!(
        !plain
            .windows(b"icon_display_style".len())
            .any(|w| w == b"icon_display_style")
    );
    // MATLAB FIG export carries the flag too.
    e.execute("figure; errorbar(1:3,[2 3 4],[.1 .2 .1]); savefig('eb.fig');")
        .unwrap();
    let fig = matrust::files::load_figure(&dir.path().join("eb.fig")).unwrap();
    let flags: Vec<bool> = fig.series.iter().map(|s| s.legend_hidden).collect();
    assert_eq!(flags, before);
    assert_eq!(fig.axes.legend_members().count(), 1);
}

fn read_with_magic(
    dir: &std::path::Path,
    from: &str,
    magic: &[u8; 8],
    to: &str,
) -> std::path::PathBuf {
    let mut bytes = std::fs::read(dir.join(from)).unwrap();
    bytes[..8].copy_from_slice(magic);
    let path = dir.join(to);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn legend_helper_flag_has_its_own_file_magic() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("errorbar(1:3,[2 3 4],[.1 .2 .1]); savefig('eb.mrfig');")
        .unwrap();
    let hidden = e.figure.as_ref().unwrap().clone();
    let eb = std::fs::read(dir.path().join("eb.mrfig")).unwrap();
    assert_eq!(&eb[..8], b"MATRFIGZ");
    let loaded = matrust::files::load_figure(&dir.path().join("eb.mrfig")).unwrap();
    let flags =
        |f: &matrust::plot::Figure| f.series.iter().map(|s| s.legend_hidden).collect::<Vec<_>>();
    assert_eq!(flags(&loaded), flags(&hidden));

    // A plain plot has no helper: it keeps the magic it had before this flag
    // (V, because its automatic palette slot is saved).
    e.execute("figure; plot(1:3); savefig('p.mrfig');").unwrap();
    let plain = std::fs::read(dir.path().join("p.mrfig")).unwrap();
    assert_eq!(&plain[..8], b"MATRFIGV");
    assert!(matrust::files::load_figure(&dir.path().join("p.mrfig")).is_ok());
    // The helper flag alone selects Z: the same errorbar figure without the
    // flag is written under the magic the older chain picked.
    let mut unflagged = hidden.clone();
    for s in &mut unflagged.axes.series {
        s.legend_hidden = false;
    }
    let path = dir.path().join("unflagged.mrfig");
    matrust::files::save_figure(&path, &unflagged).unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[..8], b"MATRFIGV");

    // Older magics must not carry the flag: each one is rejected by name.
    for (i, magic) in [b"MATRFIG4", b"MATRFIG9", b"MATRFIGV", b"MATRFIGX"]
        .into_iter()
        .enumerate()
    {
        let path = read_with_magic(dir.path(), "eb.mrfig", magic, &format!("old{i}.mrfig"));
        let error = matrust::files::load_figure(&path).unwrap_err();
        assert!(
            error.contains("Legend helper metadata requires MFIGZ"),
            "{}: {error}",
            String::from_utf8_lossy(magic)
        );
    }
}

#[test]
fn legend_helper_magic_keeps_older_metadata_and_subplots() {
    use matrust::plot::{LegendColors, PaletteCycle, PaletteVersion};
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    // One figure with helpers, print preference, legend RGB and palette metadata.
    e.execute("errorbar(1:3,[2 3 4],[.1 .2 .1]);").unwrap();
    let mut figure = e.figure.as_ref().unwrap().clone();
    figure.invert_hardcopy = Some(true);
    figure
        .axes
        .colors
        .get_or_insert_with(Default::default)
        .legend = Some(LegendColors {
        text: Some([0.1, 0.2, 0.3]),
        ..Default::default()
    });
    figure.axes.palette_cycle = Some(PaletteCycle::new(PaletteVersion::MatRustModern1));
    let path = dir.path().join("all.mrfig");
    matrust::files::save_figure(&path, &figure).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"MATRFIGZ"));
    let loaded = matrust::files::load_figure(&path).unwrap();
    assert_eq!(loaded.invert_hardcopy, Some(true));
    assert_eq!(
        loaded
            .axes
            .colors
            .and_then(|c| c.legend)
            .and_then(|l| l.text),
        Some([0.1, 0.2, 0.3])
    );
    assert_eq!(loaded.axes.palette_cycle, figure.axes.palette_cycle);
    let flags =
        |f: &matrust::plot::Figure| f.series.iter().map(|s| s.legend_hidden).collect::<Vec<_>>();
    assert_eq!(flags(&loaded), flags(&figure));

    // Subplots: helper axes sit under the newest container, other axes stay plain.
    e.execute(
        "figure; subplot(1,2,1); errorbar(1:3,[2 3 4],[.1 .2 .1]); \
         subplot(1,2,2); plot(1:3); savefig('sub.mrfig');",
    )
    .unwrap();
    let before: Vec<Vec<bool>> = e
        .figure
        .as_ref()
        .unwrap()
        .all_axes()
        .map(|(_, a)| a.series.iter().map(|s| s.legend_hidden).collect())
        .collect();
    let sub = std::fs::read(dir.path().join("sub.mrfig")).unwrap();
    assert_eq!(&sub[..8], b"MATRFIGY");
    let loaded = matrust::files::load_figure(&dir.path().join("sub.mrfig")).unwrap();
    let after: Vec<Vec<bool>> = loaded
        .all_axes()
        .map(|(_, a)| a.series.iter().map(|s| s.legend_hidden).collect())
        .collect();
    assert_eq!(before, after);
    assert!(after.iter().any(|a| a.contains(&true)));

    // Helper children need the newest container and the helper magic.
    let path = read_with_magic(dir.path(), "sub.mrfig", b"MATRFIGW", "sub-w.mrfig");
    let error = matrust::files::load_figure(&path).unwrap_err();
    assert!(error.contains("requires MFIGY"), "{error}");
    let size = u32::from_le_bytes(sub[8..12].try_into().unwrap()) as usize;
    let mut old_child = sub.clone();
    old_child[12 + size + 4..12 + size + 12].copy_from_slice(b"MATRFIGX");
    let path = dir.path().join("sub-x.mrfig");
    std::fs::write(&path, old_child).unwrap();
    let error = matrust::files::load_figure(&path).unwrap_err();
    assert!(
        error.contains("Legend helper metadata requires MFIGZ"),
        "{error}"
    );
}

#[test]
fn corpus_scalar_fill_maps_cdata_through_axes_clim_and_colormap_and_preserves_controls() {
    let mut e = run("h=fill([0;2;2;0],[0;0;1;1],1); \
         t=get(h,'Type'); fc=get(h,'FaceColor'); cd=get(h,'CData'); \
         cm=get(h,'CDataMapping'); ec=get(h,'EdgeColor');");
    assert_eq!(text(&e, "t"), "patch");
    assert_eq!(text(&e, "fc"), "flat");
    assert_eq!(num(&e, "cd"), vec![1.0]);
    assert_eq!(text(&e, "cm"), "scaled");
    assert_eq!(num(&e, "ec"), vec![0.0, 0.0, 0.0]);

    // Explicit limits and two distinct colours: CData=1 maps to row 1 (blue #0000ff).
    e.execute("colormap([1 0 0; 0 0 1]); clim([0 2]);").unwrap();
    let svg_blue = e.figure.as_ref().unwrap().svg();
    assert!(
        svg_blue.contains("fill=\"#0000ff\""),
        "expected blue mapped interior in SVG: {svg_blue}"
    );

    // Changing the palette recolours the existing handle.
    e.execute("colormap([0 1 0; 1 1 0]); fc2=get(h,'FaceColor'); cd2=get(h,'CData');")
        .unwrap();
    assert_eq!(text(&e, "fc2"), "flat");
    assert_eq!(num(&e, "cd2"), vec![1.0]);
    let svg_yellow = e.figure.as_ref().unwrap().svg();
    assert!(
        svg_yellow.contains("fill=\"#ffff00\""),
        "expected yellow mapped interior after colormap change in SVG: {svg_yellow}"
    );
    assert!(!svg_yellow.contains("fill=\"#0000ff\""));

    // Setting RGB FaceColor and EdgeColor fixes both and retains scalar CData.
    e.execute(
        "set(h,'EdgeColor',[.7 .7 .7],'FaceColor',[.7 .7 .7]); \
         fc3=get(h,'FaceColor'); ec3=get(h,'EdgeColor'); cd3=get(h,'CData');",
    )
    .unwrap();
    assert_eq!(num(&e, "cd3"), vec![1.0]);
    close(&num(&e, "fc3"), &[179. / 255., 179. / 255., 179. / 255.]);
    close(&num(&e, "ec3"), &[179. / 255., 179. / 255., 179. / 255.]);

    // Changing the palette after fixing RGB must NOT recolour the polygon.
    e.execute("colormap([1 0 1; 0 1 1]); cd4=get(h,'CData');")
        .unwrap();
    assert_eq!(num(&e, "cd4"), vec![1.0]);
    let svg_fixed = e.figure.as_ref().unwrap().svg();
    assert!(
        svg_fixed.contains("fill=\"#b3b3b3\""),
        "expected fixed #b3b3b3 interior after colormap change in SVG: {svg_fixed}"
    );
    assert!(svg_fixed.contains("stroke=\"#b3b3b3\""));
    assert!(!svg_fixed.contains("fill=\"#00ffff\""));
    assert!(!svg_fixed.contains("fill=\"#ff00ff\""));

    // Failed set call is atomic: leaves FaceColor and EdgeColor unchanged.
    assert!(
        e.execute("set(h,'FaceColor',[0 0 1],'EdgeColor','bogus');")
            .is_err()
    );
    e.execute("fc_after=get(h,'FaceColor'); ec_after=get(h,'EdgeColor');")
        .unwrap();
    close(
        &num(&e, "fc_after"),
        &[179. / 255., 179. / 255., 179. / 255.],
    );
    close(
        &num(&e, "ec_after"),
        &[179. / 255., 179. / 255., 179. / 255.],
    );

    // Non-finite scalar CData is rejected cleanly.
    assert!(
        Engine::default()
            .execute("fill([0;2;2;0],[0;0;1;1],NaN);")
            .is_err()
    );

    // Control: existing triangle row-RGB case and named-colour concave fill are unaffected by colormap.
    let mut ctrl = run("h1=fill([0 1 0],[0 0 1],[0 0.5 1],[2 3 2],[0 0 1],'g'); \
         colormap([1 0 0; 0 0 1]); \
         c1=get(h1(1),'FaceColor'); c2=get(h1(2),'FaceColor');");
    close(&num(&ctrl, "c1"), &[0., 128. / 255., 1.]);
    close(&num(&ctrl, "c2"), &[0., 1., 0.]);
    let ctrl_svg = ctrl.figure.as_ref().unwrap().svg();
    assert!(ctrl_svg.contains("fill=\"#0080ff\""));
    assert!(ctrl_svg.contains("fill=\"#00ff00\""));
    ctrl.execute("figure; hc=fill([0 2 2 1 1 0],[0 0 1 1 2 2],'r');")
        .unwrap();
    let f = ctrl.figure.as_ref().unwrap();
    assert_eq!(f.series[0].color, [255, 0, 0]);
    assert!((area(&f.series[0].points.iter().collect::<Vec<_>>()) - 3.).abs() < 1e-12);
}

// Per-vertex fill CData (FaceColor 'interp'). Property shapes follow GNU
// Octave 10.3 (column CData -> 'interp' with Nx1 CData; a 1x3 row stays a
// fixed RGB FaceColor with empty CData). Colours follow the CDataMapping
// 'scaled' row rule floor(m (c - cmin) / (cmax - cmin)) applied to the
// linearly interpolated scalar. The 256-row ramp palettes make row k exactly
// k/255, so expected channels are integers: red->blue row k = [255-k 0 k],
// green->yellow row k = [k 255 0].
const VF_RED_BLUE: &str =
    "colormap([linspace(1,0,256)' zeros(256,1) linspace(0,1,256)']); clim([0 1]);";
const VF_GREEN_YELLOW: &str = "colormap([linspace(0,1,256)' ones(256,1) zeros(256,1)]);";
/// Left corners C=0, right corners C=1 on the rectangle [0 2]x[0 1].
const VF_FIXTURE: &str = "h=fill([0;2;2;0],[0;0;1;1],[0;1;1;0],'EdgeColor','none');";
/// Two interior points of the first rendered triangle (0,1),(0,0),(2,0).
const VF_SAMPLES: [[f64; 2]; 2] = [[0.3, 0.2], [1.3, 0.2]];

fn vf_row(c: f64) -> u8 {
    ((256. * c).floor() as usize).min(255) as u8
}
fn vf_inside(tri: &[[f64; 2]], p: [f64; 2]) -> bool {
    let cross =
        |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let d = [
        cross(tri[0], tri[1]),
        cross(tri[1], tri[2]),
        cross(tri[2], tri[0]),
    ];
    d.iter().all(|&v| v > 0.) || d.iter().all(|&v| v < 0.)
}
/// Exported raster pixels at data points of the [0 2]x[0 1] patch, located
/// from the bounding box of pixels matching `patch` (the red->blue fill).
fn vf_raster(
    figure: &matrust::plot::Figure,
    bbox: &mut Option<[f64; 4]>,
    at: &[[f64; 2]],
) -> Vec<[u8; 3]> {
    let image = matrust::export::render_figure_with_cancel(
        figure,
        800,
        600,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let b = *bbox.get_or_insert_with(|| {
        let mut b = [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for (x, y, p) in image.enumerate_pixels() {
            if p[1] < 20 && u16::from(p[0]) + u16::from(p[2]) > 230 {
                b = [
                    b[0].min(x as f64),
                    b[1].max(x as f64 + 1.),
                    b[2].min(y as f64),
                    b[3].max(y as f64 + 1.),
                ];
            }
        }
        assert!(b[1] - b[0] > 200. && b[3] - b[2] > 100., "patch box {b:?}");
        b
    });
    at.iter()
        .map(|&[x, y]| {
            let px = b[0] + x / 2. * (b[1] - b[0]);
            let py = b[3] - y * (b[3] - b[2]);
            let p = image.get_pixel(px as u32, py as u32);
            [p[0], p[1], p[2]]
        })
        .collect()
}
fn vf_close(actual: &[[u8; 3]], expected: &[[u8; 3]], tol: u8) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            a.iter().zip(e).all(|(x, y)| x.abs_diff(*y) <= tol),
            "samples {actual:?} expected {expected:?} (tolerance {tol})"
        );
    }
}

#[test]
fn corpus_vertex_fill_column_cdata_is_interp_and_row_rgb_stays_fixed() {
    // Single triangle: column -> 'interp' with 3x1 CData; row -> RGB, empty CData.
    let e = run(
        "h=fill([0;1;0],[0;0;1],[0;.5;1]); fc=get(h,'FaceColor'); cd=get(h,'CData'); \
         figure; g=fill([0;1;0],[0;0;1],[0 .5 1]); gc=get(g,'FaceColor'); gd=get(g,'CData');",
    );
    assert_eq!(text(&e, "fc"), "interp");
    assert_eq!(num(&e, "cd"), vec![0., 0.5, 1.]);
    match e.array("cd") {
        Some(Array::Numeric(m)) => assert_eq!((m.rows(), m.cols()), (3, 1)),
        other => panic!("{other:?}"),
    }
    close(&num(&e, "gc"), &[0., 128. / 255., 1.]);
    assert!(num(&e, "gd").is_empty());

    // Rectangle columns with four values keep 4x1 CData; a closing vertex is
    // retained in CData and its rendered corner maps to the first vertex.
    let e = run(
        "h=fill([0;2;2;0],[0;0;1;1],[1;2;3;4]); cd=get(h,'CData'); fc=get(h,'FaceColor'); \
         figure; k=fill([0;2;2;0;0],[0;0;1;1;0],[0;1;1;0;0]); kd=get(k,'CData');",
    );
    assert_eq!(num(&e, "cd"), vec![1., 2., 3., 4.]);
    assert_eq!(text(&e, "fc"), "interp");
    assert_eq!(num(&e, "kd"), vec![0., 1., 1., 0., 0.]);
    let s = &e.figure.as_ref().unwrap().series[0];
    let vertex = s.patch_vertex_cdata.as_ref().unwrap();
    let xy = [[0., 0.], [2., 0.], [2., 1.], [0., 1.], [0., 0.]];
    let points: Vec<[f64; 2]> = s.points.iter().collect();
    assert_eq!(points.len(), 6);
    assert_eq!(vertex.corners.len(), points.len());
    for (p, &i) in points.iter().zip(vertex.corners.iter()) {
        assert_eq!(*p, xy[i as usize]);
        assert!(i < 4, "closing vertex maps to its first occurrence");
    }

    // A mismatched vertex count rejects and leaves no extra object.
    let mut e = run("fill([0;1;0],[0;0;1],'r');");
    let before = e.figure.as_ref().unwrap().series.len();
    let err = e
        .execute("hold on; fill([0;2;2;0],[0;0;1;1],[1;2;3;4;5]);")
        .unwrap_err();
    assert!(err.to_string().contains("5 values for 4 vertices"), "{err}");
    assert_eq!(e.figure.as_ref().unwrap().series.len(), before);
}

#[test]
fn corpus_vertex_fill_export_interpolates_within_one_triangle_and_follows_palette() {
    let mut e = run(&format!("{VF_FIXTURE}{VF_RED_BLUE}"));
    // Both samples lie in the same rendered triangle.
    let s = &e.figure.as_ref().unwrap().series[0];
    let tri: Vec<[f64; 2]> = s.points.iter().take(3).collect();
    assert_eq!(tri, vec![[0., 1.], [0., 0.], [2., 0.]]);
    assert!(VF_SAMPLES.iter().all(|&p| vf_inside(&tri, p)));
    // c = x/2 inside that triangle: 0.15 -> row 38, 0.65 -> row 166.
    let rows = VF_SAMPLES.map(|[x, _]| vf_row(x / 2.));
    assert_eq!(rows, [38, 166]);
    // A 2-px band per row at this size: +-4 per channel covers the row
    // boundary and the anti-aliased box location.
    let mut bbox = None;
    let red_blue = vf_raster(e.figure.as_ref().unwrap(), &mut bbox, &VF_SAMPLES);
    eprintln!("export red->blue samples {red_blue:?}");
    vf_close(&red_blue, &rows.map(|k| [255 - k, 0, k]), 4);
    assert!(red_blue[0] != red_blue[1]);

    e.execute(VF_GREEN_YELLOW).unwrap();
    let green_yellow = vf_raster(e.figure.as_ref().unwrap(), &mut bbox, &VF_SAMPLES);
    eprintln!("export green->yellow samples {green_yellow:?}");
    vf_close(&green_yellow, &rows.map(|k| [k, 255, 0]), 4);

    // A fixed row RGB fill does not follow the palette.
    let mut f = run(&format!(
        "{VF_RED_BLUE}h=fill([0;2;2;0],[0;0;1;1],[1 0 .5],'EdgeColor','none');"
    ));
    let svg = f.figure.as_ref().unwrap().svg();
    assert!(svg.contains("fill=\"#ff0080\""));
    f.execute(VF_GREEN_YELLOW).unwrap();
    let svg = f.figure.as_ref().unwrap().svg();
    assert!(svg.contains("fill=\"#ff0080\""));
    assert_eq!(f.figure.as_ref().unwrap().series[0].color, [255, 0, 128]);
}

#[test]
fn corpus_vertex_fill_native_round_trip_keeps_interp_and_rejects_bad_corners() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "h=fill([0;2;2;0],[0;0;1;1],[1;2;3;4],'EdgeColor','none'); \
         fc_before=get(h,'FaceColor'); cd_before=get(h,'CData'); \
         savefig('vf.mrfig');",
    )
    .unwrap();
    let before = e.figure.as_ref().unwrap().series[0]
        .patch_vertex_cdata
        .clone()
        .unwrap();

    let path = dir.path().join("vf.mrfig");
    let loaded = matrust::files::load_figure(&path).unwrap();
    assert_eq!(
        loaded.series[0].face_color_mode,
        matrust::plot::FaceColorMode::Flat
    );
    assert_eq!(loaded.series[0].patch_vertex_cdata.as_ref(), Some(&before));

    e.execute(
        "close all; openfig('vf.mrfig'); ch=get(gca,'Children'); \
         for i=1:numel(ch), if strcmp(get(ch(i),'Type'),'patch'), p=ch(i); end, end; \
         fc_after=get(p,'FaceColor'); cd_after=get(p,'CData');",
    )
    .unwrap();
    assert_eq!(text(&e, "fc_before"), "interp");
    assert_eq!(text(&e, "fc_after"), "interp");
    assert_eq!(num(&e, "cd_after"), num(&e, "cd_before"));
    assert_eq!(num(&e, "cd_after"), vec![1., 2., 3., 4.]);

    // Tampered file with a corner index out of range is rejected. Corners
    // travel in the binary payload as little-endian u32 values.
    let mut bad = std::fs::read(&path).unwrap();
    let json_len = u32::from_le_bytes(bad[8..12].try_into().unwrap()) as usize;
    assert!(
        !std::str::from_utf8(&bad[12..12 + json_len])
            .unwrap()
            .contains("patch_vertex_corners")
    );
    // A corner count near usize::MAX / 4 (a multiple of 3) must fail cleanly
    // instead of overflowing the payload extent sum.
    let json = std::str::from_utf8(&bad[12..12 + json_len]).unwrap();
    let key = "\"patch_vertex_corner_count\":";
    let start = json.find(key).unwrap() + key.len();
    let end = start + json[start..].find(|c: char| !c.is_ascii_digit()).unwrap();
    let huge = format!("{}{}{}", &json[..start], usize::MAX / 12 * 3, &json[end..]);
    let mut over = bad[..8].to_vec();
    over.extend((huge.len() as u32).to_le_bytes());
    over.extend(huge.as_bytes());
    over.extend(&bad[12 + json_len..]);
    let over_path = dir.path().join("vf-over.mrfig");
    std::fs::write(&over_path, over).unwrap();
    assert!(matrust::files::load_figure(&over_path).is_err());

    let corner_bytes: Vec<u8> = before
        .corners
        .iter()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    let at = 12
        + json_len
        + bad[12 + json_len..]
            .windows(corner_bytes.len())
            .position(|w| w == corner_bytes.as_slice())
            .unwrap();
    bad[at..at + 4].copy_from_slice(&(before.source.len() as u32).to_le_bytes());
    let bad_path = dir.path().join("vf-bad.mrfig");
    std::fs::write(&bad_path, bad).unwrap();
    let err = matrust::files::load_figure(&bad_path).unwrap_err();
    assert!(
        err.contains("Invalid native Patch vertex CData"),
        "unexpected error: {err}"
    );
}

#[test]
fn corpus_vertex_fill_many_large_interp_fills_round_trip() {
    // Twelve 3000-vertex interp fills: their per-vertex CData would exceed the
    // 1 MiB axes metadata cap as JSON; the binary payload carries it instead.
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "t=linspace(0,2*pi,3001)'; t(end)=[]; hold on; \
         for k=1:12, fill(cos(t)+3*k,sin(t),t+k/7,'EdgeColor','none'); end; \
         savefig('many.mrfig');",
    )
    .unwrap();
    let before: Vec<_> = e
        .figure
        .as_ref()
        .unwrap()
        .series
        .iter()
        .filter_map(|s| s.patch_vertex_cdata.clone())
        .collect();
    assert_eq!(before.len(), 12);
    assert!(before.iter().all(|v| v.source.len() == 3000));
    let loaded = matrust::files::load_figure(&dir.path().join("many.mrfig")).unwrap();
    let after: Vec<_> = loaded
        .series
        .iter()
        .filter_map(|s| s.patch_vertex_cdata.clone())
        .collect();
    assert_eq!(after, before);
    e.execute(
        "close all; openfig('many.mrfig'); ch=get(gca,'Children'); n=0; ok=true; \
         for i=1:numel(ch), if strcmp(get(ch(i),'Type'),'patch'), n=n+1; \
         ok=ok && strcmp(get(ch(i),'FaceColor'),'interp') && numel(get(ch(i),'CData'))==3000; end, end; ok=double(ok);",
    )
    .unwrap();
    assert_eq!(num(&e, "n"), vec![12.]);
    assert_eq!(num(&e, "ok"), vec![1.]);
}
