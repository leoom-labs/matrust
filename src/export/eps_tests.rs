use super::*;
use std::sync::atomic::AtomicBool;

fn tree(body: &str) -> usvg::Tree {
    usvg::Tree::from_str(
        &format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">{body}</svg>"##
        ),
        &usvg::Options::default(),
    )
    .unwrap()
}
fn text(tree: &usvg::Tree) -> String {
    String::from_utf8(encode(tree, [240, 250, 255], &AtomicBool::new(false)).unwrap()).unwrap()
}
#[test]
fn eps_envelope_ascii_exact_size_stroke_fill_and_bounded_lines() {
    let t = tree(
        r##"<path d="M10 10L90 10L50 80Z" fill="#123456" stroke="#abcdef" stroke-width="3" stroke-linecap="round" stroke-linejoin="bevel" stroke-miterlimit="8" stroke-dasharray="2 4" stroke-dashoffset="1" paint-order="stroke fill"/>"##,
    );
    let s = text(&t);
    assert!(s.starts_with("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 160 100\n"));
    assert!(s.ends_with("grestore\nshowpage\n%%Trailer\n%%EOF\n"));
    assert!(s.lines().all(|l| l.is_ascii() && l.len() <= 255));
    assert!(
        s.contains("3.000000000e0 setlinewidth")
            && s.contains("1 setlinecap")
            && s.contains("2 setlinejoin")
    );
    assert!(s.contains("2.000000000e0\n4.000000000e0\n] 1.000000000e0 setdash"));
    assert!(s.find("\nstroke\n").unwrap() < s.find("\nfill\n").unwrap());
    for forbidden in [
        "colorimage",
        "imagemask",
        "setpagedevice",
        "initgraphics",
        "initmatrix",
        "initclip",
        "erasepage",
        "findfont",
    ] {
        assert!(
            !s.split_whitespace().any(|token| token == forbidden),
            "{forbidden}"
        );
    }
}
#[test]
fn quadratic_conversion_matches_independent_bernstein_evaluation() {
    let p = [-13., 7.];
    let q = [19., -31.];
    let end = [43., 11.];
    let (a, b) = quadratic(p, q, end);
    for i in 0..=32 {
        let t = i as f64 / 32.;
        let u = 1. - t;
        for axis in 0..2 {
            let expected = u * u * p[axis] + 2. * u * t * q[axis] + t * t * end[axis];
            let actual = u * u * u * p[axis]
                + 3. * u * u * t * a[axis]
                + 3. * u * t * t * b[axis]
                + t * t * t * end[axis];
            assert!((actual - expected).abs() < 2e-14);
        }
    }
    let s = text(&tree(
        r##"<path d="M0 0Q3 6 9 0" fill="none" stroke="black"/>"##,
    ));
    assert!(s.contains("2.000000000e0 4.000000000e0 5.000000000e0 4.000000000e0 9.000000000e0 0.000000000e0 curveto"));
}
#[test]
fn nested_transform_and_clip_restore_ctm_without_double_absolute_transform() {
    let s = text(&tree(
        r##"<defs><clipPath id="c" transform="translate(2 3)"><path clip-rule="evenodd" d="M0 0H60V60H0Z M10 10V20H20V10Z"/></clipPath></defs><g transform="translate(13 17)"><g transform="scale(2)" clip-path="url(#c)"><path fill="none" stroke="black" d="M1 2L4 5"/></g></g>"##,
    ));
    assert!(s.contains("matrix currentmatrix\n"));
    assert!(s.contains("eoclip newpath\nsetmatrix\n"));
    assert!(s.contains("1.000000000e0 2.000000000e0 moveto\n4.000000000e0 5.000000000e0 lineto"));
    assert_eq!(s.matches("1.300000000e1 1.700000000e1] concat").count(), 1);
    assert_eq!(s.matches("2.000000000e0 3.000000000e0] concat").count(), 1);
}
#[test]
fn unsupported_effects_reject_without_raster_fallback() {
    let bodies = [
        r##"<path d="M1 1H80V80Z" fill="red" opacity=".5"/>"##,
        r##"<g opacity=".5"><path d="M1 1H80V80Z"/></g>"##,
        r##"<defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><path d="M1 1H80V80Z" fill="url(#g)"/>"##,
        r##"<defs><clipPath id="c"><rect width="30" height="30"/><rect x="40" width="30" height="30"/></clipPath></defs><path d="M1 1H80V80Z" clip-path="url(#c)"/>"##,
        r##"<defs><mask id="m"><rect width="160" height="100" fill="white"/></mask></defs><path d="M1 1H80V80Z" mask="url(#m)"/>"##,
    ];
    for body in bodies {
        assert!(
            encode(&tree(body), [255; 3], &AtomicBool::new(false)).is_err(),
            "{body}"
        );
    }
    assert!(transform(Transform::from_scale(0., 1.)).is_err());
    assert!(number(f64::NAN).is_err());
    assert!(number(1e13).is_err());
}
#[test]
fn output_tree_depth_and_segment_limits_are_exact_and_cancel_is_cooperative() {
    let t = tree(r##"<g transform="translate(1 1)"><path d="M0 0H20V20H0Z"/></g>"##);
    let normal = encode(&t, [255; 3], &AtomicBool::new(false)).unwrap();
    let exact = Limits {
        bytes: normal.len(),
        ..Limits::default()
    };
    assert_eq!(
        encode_checked(&t, [255; 3], exact, &mut || Ok(())).unwrap(),
        normal
    );
    for limits in [
        Limits {
            bytes: normal.len() - 1,
            ..exact
        },
        Limits { nodes: 1, ..exact },
        Limits {
            segments: 1,
            ..exact
        },
        Limits { depth: 0, ..exact },
    ] {
        assert!(encode_checked(&t, [255; 3], limits, &mut || Ok(())).is_err());
    }
    let mut data = String::from("M0 0");
    for i in 0..5000 {
        use std::fmt::Write;
        write!(&mut data, "L{} {}", i % 100, (i / 100) % 80).unwrap();
    }
    let t = tree(&format!(
        r##"<path fill="none" stroke="black" d="{data}"/>"##
    ));
    let mut calls = 0;
    let result = encode_checked(&t, [255; 3], Limits::default(), &mut || {
        calls += 1;
        if calls == 6 {
            Err("deterministic cancellation".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(result.unwrap_err(), "deterministic cancellation");
    assert_eq!(calls, 6);
    assert!(encode(&t, [255; 3], &AtomicBool::new(false)).is_ok());
}
#[test]
fn outline_text_and_five_ordinary_plot_families_use_real_vector_content() {
    let mut engine = crate::Engine::default();
    engine.execute("plot([1 2 3],[2 3 2],'o-');hold on;stairs([1 2 3],[1 2 1]);stem([1 2 3],[3 4 3]);scatter([1 2 3],[4 3 4]);bar([1 2 3],[1 1 1]);title('Outlined text');xlabel('X axis');ylabel('Y axis');legend('line','stairs','stem','scatter','bar');").unwrap();
    let f = engine.figure.as_ref().unwrap();
    let before = f.clone();
    let svg = f
        .svg_on_canvas_with_cancel(Canvas::default(), &AtomicBool::new(false))
        .unwrap();
    let tree = usvg::Tree::from_str(&svg, &super::super::svg_render_options()).unwrap();
    let s = text(&tree);
    assert!(s.matches("curveto").count() > 5);
    assert!(s.matches("fill\n").count() > 10);
    assert!(!s.contains("Outlined text"));
    assert!(f.same_snapshot(&before));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vector.eps");
    export_eps_with_cancel(
        f,
        &path,
        Canvas {
            width: 640.,
            height: 480.,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let saved = std::fs::read_to_string(path).unwrap();
    assert!(saved.contains("%%BoundingBox: 0 0 640 480"));
}
#[test]
fn public_export_cancellation_keeps_destination_and_legacy_svg_canvas_bytes() {
    let mut engine = crate::Engine::default();
    engine.execute("plot([1 2],[3 4]);").unwrap();
    let f = engine.figure.as_ref().unwrap();
    for canvas in [
        Canvas::default(),
        Canvas {
            width: 640.,
            height: 480.,
        },
        Canvas {
            width: 1440.5,
            height: 900.25,
        },
    ] {
        assert_eq!(
            f.svg_on_canvas(canvas).unwrap(),
            f.svg_on_canvas_with_cancel(canvas, &AtomicBool::new(false))
                .unwrap()
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.eps");
    std::fs::write(&path, b"sentinel").unwrap();
    assert!(export_eps_with_cancel(f, &path, Canvas::default(), &AtomicBool::new(true)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn actual_generated_patch_textbox_log_legend_and_tiled_clips_encode() {
    let scripts = [
        "f=figure('Color',[.2 .4 .6]);a=gca;h=plot(a,[1 2],[1 100]);set(a,'YScale','log','YLim',[1 100]);hold(a,'on');patch('Parent',a,'XData',[1;1.5;2],'YData',[2;20;2],'FaceColor',[1 0 0],'EdgeColor','none');legend(a,h,{'line'},'Interpreter','none','Location','eastoutside');annotation(f,'textbox',[.1 .8 .8 .1],'String','Literal <&> text','Interpreter','none','EdgeColor','none');",
        "subplot(2,2,1);plot([1 2],[3 4]);subplot(2,2,2);plot([1 2],[-2 -20]);set(gca,'YScale','log','YLim',[-20 -2]);subplot(2,2,3);scatter([1 2],[5 6]);subplot(2,2,4);stem([1 2],[3 4],'BaseValue',2);",
    ];
    for script in scripts {
        let mut engine = crate::Engine::default();
        engine.execute(script).unwrap();
        let f = engine.figure.as_ref().unwrap();
        let svg = f
            .svg_on_canvas_with_cancel(
                Canvas {
                    width: 640.,
                    height: 480.,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        let tree = usvg::Tree::from_str(&svg, &super::super::svg_render_options()).unwrap();
        let encoded = text(&tree);
        assert!(encoded.matches("clip newpath").count() >= 2);
        assert!(encoded.matches("setmatrix").count() >= 1);
    }
}

#[test]
fn image_nodes_and_long_ascii_lines_fail_explicitly() {
    let t = tree(
        r##"<image width="80" height="80" href="data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PHJlY3Qgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIi8+PC9zdmc+"/>"##,
    );
    assert!(
        encode(&t, [255; 3], &AtomicBool::new(false))
            .unwrap_err()
            .contains("image")
    );
    let mut output = Output {
        bytes: Vec::new(),
        length: 0,
        line: 0,
        emit: true,
        limit: 1024,
    };
    output.write_all(&[b'a'; 255]).unwrap();
    assert!(output.write_all(b"b").is_err());
}

#[test]
fn linked_transformed_clips_intersect_then_restore_the_referring_ctm() {
    let s = text(&tree(
        r##"<defs><clipPath id="outer" transform="translate(7 11)"><rect width="80" height="60"/></clipPath><clipPath id="inner" clip-path="url(#outer)" transform="translate(2 3)"><rect width="60" height="40"/></clipPath></defs><g transform="translate(13 17)"><g transform="scale(2)" clip-path="url(#inner)"><path fill="none" stroke="black" d="M1 2L4 5"/></g></g>"##,
    ));
    assert_eq!(s.matches("matrix currentmatrix\n").count(), 2);
    assert_eq!(s.matches("clip newpath\nsetmatrix\n").count(), 2);
    let outer = s.find("7.000000000e0 1.100000000e1] concat\n").unwrap();
    let first_restore = s[outer..].find("setmatrix\n").unwrap() + outer;
    let inner = s.find("2.000000000e0 3.000000000e0] concat\n").unwrap();
    let second_restore = s[inner..].find("setmatrix\n").unwrap() + inner;
    assert!(outer < first_restore && first_restore < inner && inner < second_restore);
    assert!(s[first_restore..inner].starts_with("setmatrix\nmatrix currentmatrix\n"));
    assert!(
        s[second_restore..]
            .contains("1.000000000e0 2.000000000e0 moveto\n4.000000000e0 5.000000000e0 lineto\n")
    );
    assert_eq!(
        s.matches("1.300000000e1 1.700000000e1] concat\n").count(),
        1
    );
}

#[test]
fn isolated_text_is_resolved_as_outlines_without_any_marker_geometry() {
    fn outlines(group: &usvg::Group) -> usize {
        group
            .children()
            .iter()
            .map(|node| match node {
                usvg::Node::Path(_) => 1,
                usvg::Node::Group(group) => outlines(group),
                usvg::Node::Text(text) => outlines(text.flattened()),
                usvg::Node::Image(_) => panic!("Text-only fixture must not use a raster glyph"),
            })
            .sum()
    }
    let options = super::super::svg_render_options();
    let empty = usvg::Tree::from_str(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100"/>"##,
        &options,
    )
    .unwrap();
    let letters=usvg::Tree::from_str(r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100"><text x="10" y="60" font-family="sans-serif" font-size="32">Outlined</text></svg>"##,&options).unwrap();
    assert_eq!(outlines(empty.root()), 0);
    assert!(
        outlines(letters.root()) > 0,
        "System font resolution must create real outline paths"
    );
    let baseline = text(&empty);
    let rendered = text(&letters);
    assert!(!baseline.contains("curveto"));
    assert!(rendered.contains("curveto"));
    assert!(rendered.matches("\nfill\n").count() > baseline.matches("\nfill\n").count());
    assert!(!rendered.contains("Outlined"));
    assert!(
        !rendered
            .split_whitespace()
            .any(|token| matches!(token, "show" | "findfont" | "colorimage"))
    );
}
