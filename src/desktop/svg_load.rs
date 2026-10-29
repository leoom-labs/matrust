//! Static vector previews. XML, reference expansion and rendering are bounded
//! before publication; no resource resolver can open linked files or URLs.
use super::image_load::{MAX_PIXELS, MAX_PREVIEW_PIXELS, Preview, check, read_file};
use eframe::egui::ColorImage;
use matrust::Result;
use resvg::{tiny_skia, usvg};
use std::{collections::HashMap, path::Path, sync::atomic::AtomicBool};
use usvg::roxmltree::{Document, ParsingOptions};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_NODES: u32 = 50_000;
const MAX_DEPTH: usize = 64;
const MAX_EXPANDED_NODES: usize = 150_000;
const MAX_EXPANDED_BYTES: usize = 64 * 1024 * 1024;
const MAX_SEGMENTS: usize = 1_000_000;
const SVG_NS: &str = "http://www.w3.org/2000/svg";

#[derive(Clone, Copy, Default)]
struct Cost {
    nodes: usize,
    bytes: usize,
    height: usize,
    text: usize,
    geometry: usize,
}

fn reference(value: &str, ids: &HashMap<&str, usize>) -> Result<usize> {
    let id = value
        .trim()
        .strip_prefix('#')
        .ok_or("SVG preview accepts only local #id references")?;
    ids.get(id)
        .copied()
        .ok_or_else(|| format!("Unresolved SVG reference #{id}"))
}

fn expansion(
    index: usize,
    edges: &[Vec<usize>],
    weights: &[Cost],
    state: &mut [u8],
    memo: &mut [Cost],
    depth: usize,
    cancel: &AtomicBool,
) -> Result<Cost> {
    check(cancel)?;
    if depth > MAX_DEPTH || state[index] == 1 {
        return Err("SVG reference cycle or nesting exceeds 64 levels".into());
    }
    if state[index] == 2 {
        if depth + memo[index].height > MAX_DEPTH + 1 {
            return Err("Expanded SVG nesting exceeds 64 levels".into());
        }
        return Ok(memo[index]);
    }
    state[index] = 1;
    let mut cost = Cost {
        nodes: 1,
        height: 1,
        ..weights[index]
    };
    for &child in &edges[index] {
        let c = expansion(child, edges, weights, state, memo, depth + 1, cancel)?;
        cost.nodes += c.nodes;
        cost.bytes += c.bytes;
        cost.text += c.text;
        cost.geometry += c.geometry;
        cost.height = cost.height.max(c.height + 1);
        if cost.nodes > MAX_EXPANDED_NODES || cost.bytes > MAX_EXPANDED_BYTES {
            return Err("SVG reference expansion exceeds 150000 nodes or 64 MiB".into());
        }
    }
    if cost.text > 65_536 || cost.geometry > MAX_BYTES {
        return Err("Expanded SVG exceeds 65536 text bytes or 16 MiB path data".into());
    }
    state[index] = 2;
    memo[index] = cost;
    Ok(cost)
}

fn preflight(doc: &Document<'_>, cancel: &AtomicBool) -> Result<()> {
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NS) {
        return Err("Expected an SVG root with the SVG namespace".into());
    }
    let nodes: Vec<_> = doc.descendants().collect();
    let indices: HashMap<_, _> = nodes.iter().enumerate().map(|(i, n)| (n.id(), i)).collect();
    let mut ids = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        check(cancel)?;
        if n.ancestors().take(MAX_DEPTH + 2).count() > MAX_DEPTH {
            return Err("SVG nesting exceeds 64 levels".into());
        }
        if n.is_pi() {
            return Err(
                "SVG processing instructions and external stylesheets are unsupported".into(),
            );
        }
        if !n.is_element() {
            continue;
        }
        let tag = n.tag_name().name();
        if n.tag_name().namespace() != Some(SVG_NS)
            || !matches!(
                tag,
                "svg"
                    | "g"
                    | "defs"
                    | "path"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "text"
                    | "tspan"
                    | "textPath"
                    | "clipPath"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "use"
                    | "symbol"
                    | "a"
                    | "title"
                    | "desc"
            )
        {
            return Err(format!(
                "SVG preview does not yet support <{tag}>; use a raster export for this file"
            ));
        }
        if let Some(id) = n.attribute("id")
            && ids.insert(id, i).is_some()
        {
            return Err(format!("Duplicate SVG id: {id}"));
        }
    }
    let mut edges = vec![Vec::new(); nodes.len()];
    let mut weights = vec![Cost::default(); nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        edges[i].extend(n.children().map(|child| indices[&child.id()]));
        weights[i].bytes = if n.is_text() {
            n.text().map_or(0, str::len)
        } else {
            0
        };
        if n.is_text()
            && n.ancestors()
                .any(|p| matches!(p.tag_name().name(), "text" | "tspan" | "textPath"))
        {
            weights[i].text = weights[i].bytes;
        }
        for attr in n.attributes() {
            weights[i].bytes += attr.value().len() + attr.name().len();
            if matches!(attr.name(), "d" | "points") {
                weights[i].geometry += attr.value().len();
            }
            if attr.name().starts_with("on") {
                return Err("SVG event handlers are unsupported".into());
            }
            let unsupported_property = |name: &str| {
                matches!(
                    name.trim(),
                    "filter" | "mask" | "marker" | "marker-start" | "marker-mid" | "marker-end"
                )
            };
            if unsupported_property(attr.name())
                || (attr.name() == "style"
                    && attr.value().split(';').any(|p| {
                        p.split_once(':').is_some_and(|(name, _)| {
                            unsupported_property(&name.to_ascii_lowercase())
                        })
                    }))
            {
                return Err(
                    "SVG filters, masks and markers are not yet supported in previews".into(),
                );
            }
            if attr.name() == "href" {
                edges[i].push(reference(attr.value(), &ids)?);
            }
            let value = attr.value();
            // CSS escapes/comments could conceal resource names from this pass.
            if value.contains('\\') || (attr.name() == "style" && value.contains("/*")) {
                return Err("Escaped/commented SVG inline CSS is unsupported".into());
            }
            let lower = value.to_ascii_lowercase();
            let mut rest = value;
            let mut remaining_lower = lower.as_str();
            while let Some(at) = remaining_lower.find("url(") {
                let tail = &rest[at + 4..];
                let end = tail.find(')').ok_or("Unclosed SVG URL reference")?;
                let value = tail[..end].trim().trim_matches(['\'', '"']);
                edges[i].push(reference(value, &ids)?);
                rest = &tail[end + 1..];
                remaining_lower = &remaining_lower[at + 4 + end + 1..];
            }
        }
    }
    expansion(
        0,
        &edges,
        &weights,
        &mut vec![0; nodes.len()],
        &mut vec![Cost::default(); nodes.len()],
        1,
        cancel,
    )?;
    Ok(())
}

struct RenderBudget {
    nodes: usize,
    segments: usize,
    layer_bytes: f64,
    painted_pixels: f64,
    dashes: f64,
    canvas: [u32; 2],
}

fn dash_bound(path: &tiny_skia::Path, dash: &[f32]) -> f64 {
    use tiny_skia::PathSegment;
    let mut point = tiny_skia::Point::from_xy(0., 0.);
    let mut start = point;
    let mut length = 0.;
    let distance = |a: tiny_skia::Point, b: tiny_skia::Point| {
        (f64::from(a.x) - f64::from(b.x)).hypot(f64::from(a.y) - f64::from(b.y))
    };
    // Bezier control-polygon length bounds arc length without subdivision.
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                point = p;
                start = p;
            }
            PathSegment::LineTo(p) => {
                length += distance(point, p);
                point = p;
            }
            PathSegment::QuadTo(a, b) => {
                length += distance(point, a) + distance(a, b);
                point = b;
            }
            PathSegment::CubicTo(a, b, c) => {
                length += distance(point, a) + distance(a, b) + distance(b, c);
                point = c;
            }
            PathSegment::Close => {
                length += distance(point, start);
                point = start;
            }
        }
    }
    let interval: f64 = dash.iter().map(|&v| f64::from(v)).sum();
    length / interval * dash.len() as f64
}
fn render_budget(
    group: &usvg::Group,
    scale: f32,
    budget: &mut RenderBudget,
    depth: usize,
    cancel: &AtomicBool,
) -> Result<()> {
    check(cancel)?;
    if depth > MAX_DEPTH {
        return Err("Rendered SVG exceeds 64 levels".into());
    }
    if group.should_isolate() {
        let bounds = group.abs_layer_bounding_box();
        // Conservative sum of all layer payloads, including clip-mask scratch.
        // Unlike the renderer's clipping cap this also bounds nested allocations.
        budget.layer_bytes += (f64::from(bounds.width()) * f64::from(scale) + 8.).ceil()
            * (f64::from(bounds.height()) * f64::from(scale) + 8.).ceil()
            * 8.;
        if !budget.layer_bytes.is_finite() || budget.layer_bytes > 256. * 1024. * 1024. {
            return Err("SVG compositing exceeds the 256 MiB preview layer budget".into());
        }
    }
    for node in group.children() {
        budget.nodes += 1;
        if budget.nodes > MAX_EXPANDED_NODES {
            return Err("Rendered SVG has too many nodes".into());
        }
        match node {
            usvg::Node::Group(g) => render_budget(g, scale, budget, depth + 1, cancel)?,
            usvg::Node::Path(p) => {
                budget.segments += p.data().segments().count();
                if budget.segments > MAX_SEGMENTS {
                    return Err("SVG exceeds one million path segments".into());
                }
                if let Some(dash) = p.stroke().and_then(|stroke| stroke.dasharray()) {
                    budget.dashes += dash_bound(p.data(), dash);
                    if !budget.dashes.is_finite() || budget.dashes > 1_000_000. {
                        return Err("SVG exceeds one million estimated stroke dashes".into());
                    }
                }
                let rect = p.abs_stroke_bounding_box();
                let scale = f64::from(scale);
                let left =
                    (f64::from(rect.x()) * scale - 2.).clamp(0., f64::from(budget.canvas[0]));
                let top = (f64::from(rect.y()) * scale - 2.).clamp(0., f64::from(budget.canvas[1]));
                let right =
                    (f64::from(rect.right()) * scale + 2.).clamp(0., f64::from(budget.canvas[0]));
                let bottom =
                    (f64::from(rect.bottom()) * scale + 2.).clamp(0., f64::from(budget.canvas[1]));
                budget.painted_pixels += (right - left).max(0.) * (bottom - top).max(0.);
                if !budget.painted_pixels.is_finite() || budget.painted_pixels > 500_000_000. {
                    return Err("SVG exceeds the preview painting work budget".into());
                }
            }
            usvg::Node::Image(_) => return Err("Embedded SVG images are unsupported".into()),
            usvg::Node::Text(_) => {} // subroots visits flattened glyph paths.
        }
        let mut result = Ok(());
        node.subroots(|root| {
            if result.is_ok() {
                result = render_budget(root, scale, budget, depth + 1, cancel);
            }
        });
        result?;
    }
    Ok(())
}

pub fn load(path: &Path, texture_side: usize, cancel: &AtomicBool) -> Result<Preview> {
    let bytes = read_file(path, MAX_BYTES, cancel)?;
    let source = std::str::from_utf8(&bytes)
        .map_err(|_| "SVG preview requires uncompressed UTF-8 XML; SVGZ is unsupported")?;
    let doc = Document::parse_with_options(
        source,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
        },
    )
    .map_err(|e| format!("Invalid SVG XML: {e}"))?;
    preflight(&doc, cancel)?;
    let mut options = matrust::export::svg_render_options();
    options.image_href_resolver = usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    };
    check(cancel)?;
    let tree = usvg::Tree::from_xmltree(&doc, &options).map_err(|e| e.to_string())?;
    check(cancel)?;
    let width = f64::from(tree.size().width());
    let height = f64::from(tree.size().height());
    if width > 8192. || height > 8192. || width * height > MAX_PIXELS as f64 {
        return Err("SVG exceeds 8192 pixels per side or 32 million pixels".into());
    }
    let side = texture_side.clamp(1, 4096) as f64;
    let scale = (side / width)
        .min(side / height)
        .min((MAX_PREVIEW_PIXELS / (width * height)).sqrt())
        .min(1.);
    let size = [
        (width * scale).ceil().max(1.) as u32,
        (height * scale).ceil().max(1.) as u32,
    ];
    // Rounding up both sides can exceed the pixel budget; shrink to floor if needed.
    let size = if u64::from(size[0]) * u64::from(size[1]) > MAX_PREVIEW_PIXELS as u64 {
        [
            (width * scale).floor().max(1.) as u32,
            (height * scale).floor().max(1.) as u32,
        ]
    } else {
        size
    };
    let scale = (size[0] as f32 / width as f32).min(size[1] as f32 / height as f32);
    let mut budget = RenderBudget {
        nodes: 0,
        segments: 0,
        layer_bytes: f64::from(size[0]) * f64::from(size[1]) * 8.,
        painted_pixels: 0.,
        dashes: 0.,
        canvas: size,
    };
    render_budget(tree.root(), scale, &mut budget, 1, cancel)?;
    let mut pixmap =
        tiny_skia::Pixmap::new(size[0], size[1]).ok_or("Could not allocate SVG preview")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    check(cancel)?;
    let pixels = ColorImage::from_rgba_premultiplied(size.map(|s| s as usize), pixmap.data());
    check(cancel)?;
    Ok(Preview {
        original: Some(super::image_load::Original::identity(&bytes, cancel)?),
        path: path.to_owned(),
        original_size: [width.ceil() as u32, height.ceil() as u32],
        format: "SVG (rendered preview)",
        file_bytes: bytes.len(),
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(body: &str) -> String {
        format!(r#"<svg xmlns="{SVG_NS}" width="64" height="32">{body}</svg>"#)
    }
    fn preview(source: &str, side: usize) -> Result<Preview> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.svg");
        std::fs::write(&path, source).unwrap();
        load(&path, side, &AtomicBool::new(false))
    }
    #[test]
    fn svg_preview_renders_shapes_alpha_clips_gradients_and_references() {
        let source = document(
            r##"<defs><rect id="box" width="64" height="32"/>
            <clipPath id="clip"><rect width="32" height="32"/></clipPath></defs>
            <g clip-path="url(#clip)"><use href="#box" fill="red" opacity="0.5"/></g>"##,
        );
        let image = preview(&source, 4096).unwrap();
        assert_eq!(image.original_size, [64, 32]);
        assert_eq!(image.pixels.size, [64, 32]);
        assert_eq!(
            image.pixels[(10, 10)].to_srgba_unmultiplied(),
            [255, 0, 0, 128]
        );
        assert_eq!(image.pixels[(40, 10)].to_array(), [0; 4]);
        assert_eq!(preview(&source, 16).unwrap().pixels.size, [16, 8]);
        let gradient = preview(
            &document(
                r##"<defs><linearGradient id="grad"><stop stop-color="red"/>
            <stop offset="1" stop-color="blue"/></linearGradient></defs>
            <rect width="64" height="32" style="fill:url('#grad')"/>"##,
            ),
            4096,
        )
        .unwrap();
        assert!(gradient.pixels[(2, 10)].r() > 240);
        assert!(gradient.pixels[(61, 10)].b() > 240);
        let viewbox = preview(&format!(r#"<svg xmlns="{SVG_NS}" width="64" height="32" viewBox="0 0 2 1"><rect width="2" height="1" fill="lime"/></svg>"#), 32).unwrap();
        assert!(
            viewbox
                .pixels
                .pixels
                .iter()
                .all(|p| p.to_array() == [0, 255, 0, 255])
        );
    }
    #[test]
    fn svg_preview_matches_exported_plot_pixels_and_layouts() {
        let mut engine = matrust::Engine::default();
        let dir = tempfile::tempdir().unwrap();
        for script in [
            "plot([1 3 2]);title('Unicode α <test>');xlabel('Time');ylabel('Value');",
            "clf;subplot(2,1,1);stairs([1 3 2]);subplot(2,1,2);stem([2 1 4]);",
            "clf;tiledlayout(1,2);nexttile;plot([1 2]);nexttile;bar([2 3]);sgtitle('Shared title');",
            "clf;subplot('Position',[0.15 0.2 0.7 0.6]);plot([1 4 2]);",
        ] {
            engine.execute(script).unwrap();
            let figure = engine.figure.as_ref().unwrap();
            let path = dir.path().join("reference.png");
            matrust::export::export_figure(figure, &path, 960, 560).unwrap();
            let expected =
                super::super::image_load::load(&path, 4096, &AtomicBool::new(false)).unwrap();
            let actual = preview(&figure.svg(), 4096).unwrap();
            assert_eq!(actual.pixels, expected.pixels);
        }
    }
    #[test]
    fn svg_preview_rejects_unsupported_resources_cycles_and_complexity() {
        for body in [
            r#"<image href="file:///private.png"/>"#,
            r#"<script>anything</script>"#,
            "<foreignObject/>",
            "<filter/>",
            "<pattern/>",
            "<style>rect { fill: red; }</style>",
            "<animate/>",
            r#"<rect onload="anything"/>"#,
            r#"<rect style="filter:blur(2px)"/>"#,
            r#"<use href="https://example.invalid/a.svg#item"/>"#,
            r##"<use id="a" href="#a"/>"##,
            r##"<g id="a"><use href="#b"/></g><g id="b"><use href="#a"/></g>"##,
            r##"<use href="#missing"/>"##,
            r##"<rect id="a"/><rect id="a"/>"##,
            r#"<rect fill="URL(https://example.invalid/a.svg#item)"/>"#,
        ] {
            assert!(preview(&document(body), 4096).is_err(), "{body}");
        }
        assert!(preview("<!DOCTYPE svg [<!ENTITY a 'test'>]><svg/>", 4096).is_err());
        assert!(preview("<svg/>", 4096).is_err());
        assert!(
            preview(
                &document(&format!("{}{}", "<g>".repeat(65), "</g>".repeat(65))),
                4096
            )
            .is_err()
        );
        assert!(preview(&document(&"<rect/>".repeat(MAX_NODES as usize)), 4096).is_err());
        let mut expansion = String::from(r#"<defs><path id="p0" d="M0 0L1 1"/>"#);
        for i in 1..20 {
            expansion.push_str(&format!(
                r##"<g id="p{i}"><use href="#p{}"/><use href="#p{}"/></g>"##,
                i - 1,
                i - 1
            ));
        }
        expansion.push_str(r##"</defs><use href="#p19"/>"##);
        assert!(matches!(preview(&document(&expansion), 4096), Err(e) if e.contains("expansion")));
    }
    #[test]
    fn svg_preview_enforces_file_pixel_layer_and_cancel_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.svg");
        assert!(matches!(load(&path,4096,&AtomicBool::new(true)), Err(e) if e == "Cancelled"));
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_BYTES as u64 + 1)
            .unwrap();
        assert!(matches!(load(&path,4096,&AtomicBool::new(false)), Err(e) if e.contains("16 MiB")));
        for dimensions in [
            "width=\"8193\" height=\"1\"",
            "width=\"8192\" height=\"8192\"",
        ] {
            assert!(preview(&format!(r#"<svg xmlns="{SVG_NS}" {dimensions}/>"#), 4096).is_err());
        }
        let large = format!(
            r#"<svg xmlns="{SVG_NS}" width="8000" height="4000"><rect width="8000" height="4000" fill="blue"/></svg>"#
        );
        let image = preview(&large, 4096).unwrap();
        assert_eq!(image.pixels.size, [4000, 2000]);
        assert!(
            image
                .pixels
                .pixels
                .iter()
                .all(|p| p.to_array() == [0, 0, 255, 255])
        );
        let layers = document(r#"<g opacity="0.5"><rect width="100000" height="100000"/></g>"#);
        assert!(matches!(preview(&layers, 4096), Err(e) if e.contains("layer budget")));
        let text = document(&format!("<text>{}</text>", "x".repeat(65_537)));
        assert!(matches!(preview(&text, 4096), Err(e) if e.contains("text bytes")));
        let dashes =
            document(r#"<path d="M0 0L64 32" stroke="red" stroke-dasharray="0.000001 0.000001"/>"#);
        assert!(matches!(preview(&dashes, 4096), Err(e) if e.contains("stroke dashes")));
        let overpaint = format!(
            r#"<svg xmlns="{SVG_NS}" width="960" height="560">{}</svg>"#,
            r#"<rect width="960" height="560"/>"#.repeat(1000)
        );
        assert!(matches!(preview(&overpaint, 4096), Err(e) if e.contains("painting work")));
    }
}
