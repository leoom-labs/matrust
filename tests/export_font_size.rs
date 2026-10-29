#![cfg(feature = "plot-export")]

use matrust::{Engine, export};
use resvg::usvg::{Group, Node, Tree};

fn parsed_text(svg: &str) -> Vec<(String, Vec<f32>)> {
    fn visit(group: &Group, result: &mut Vec<(String, Vec<f32>)>) {
        for node in group.children() {
            match node {
                Node::Group(group) => visit(group, result),
                Node::Text(text) => {
                    for chunk in text.chunks() {
                        if !chunk.text().is_empty() {
                            result.push((
                                chunk.text().to_owned(),
                                chunk.spans().iter().map(|s| s.font_size().get()).collect(),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let tree = Tree::from_str(svg, &export::svg_render_options()).unwrap();
    let mut result = Vec::new();
    visit(tree.root(), &mut result);
    result
}

fn assert_axes_sizes(svg: &str) {
    let text = parsed_text(svg);
    for (label, size) in [
        ("HorizontalLabel", 16.),
        ("VerticalLabel", 16.),
        ("AxesTitle", 20.),
    ] {
        let found: Vec<_> = text.iter().filter(|(s, _)| s == label).collect();
        assert_eq!(
            found.len(),
            1,
            "Expected exactly one parsed {label}: {text:?}"
        );
        assert!(
            !found[0].1.is_empty(),
            "Font spans must actually survive parsing"
        );
        assert!(
            found[0].1.iter().all(|&actual| actual == size),
            "{label}: {found:?}"
        );
    }
    let ticks: Vec<_> = text
        .iter()
        .filter(|(s, _)| {
            !matches!(
                s.as_str(),
                "HorizontalLabel" | "VerticalLabel" | "AxesTitle"
            )
        })
        .collect();
    assert!(
        ticks.len() >= 4,
        "Both axes must retain actual parsed tick text: {text:?}"
    );
    for (label, spans) in ticks {
        assert!(!spans.is_empty());
        assert!(
            spans.iter().all(|&size| size == 12.),
            "Tick {label}: {spans:?}"
        );
    }
}

#[test]
fn generated_axes_font_sizes_are_explicit_in_standalone_and_positioned_text() {
    for prefix in ["", "subplot('Position',[.2 .2 .6 .6]);", "subplot(1,2,1);"] {
        let mut engine = Engine::default();
        engine.execute(&format!(
            "{prefix}plot([0 1 2],[0 1 2]);xlim([0 2]);ylim([0 2]);title('AxesTitle');xlabel('HorizontalLabel');ylabel('VerticalLabel');"
        )).unwrap();
        let figure = engine.figure.as_ref().unwrap();
        assert_axes_sizes(&figure.svg());
        // Public standalone Axes export shares the same positioned emitter.
        assert_axes_sizes(&figure.axes.svg());
    }
}

#[test]
fn external_svg_without_font_size_retains_the_existing_parser_default() {
    assert_eq!(export::svg_render_options().font_size, 12.);
    let text = parsed_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="50"><text x="10" y="30" font-family="Arial">ExternalDefault</text></svg>"#,
    );
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].0, "ExternalDefault");
    assert!(!text[0].1.is_empty());
    assert!(text[0].1.iter().all(|&size| size == 12.));
}
