use matrust::plot::{RenderingStyle, colormap, render_volume};
use matrust::{Array, Engine};
use std::sync::Arc;

fn build_test_volume(ny: usize, nx: usize, nz: usize) -> Vec<f64> {
    let mut data = Vec::with_capacity(ny * nx * nz);
    for z in 0..nz {
        for x in 0..nx {
            for y in 0..ny {
                let v = ((y * 7 + x * 13 + z * 19) % 101) as f64 / 100.0
                    + 0.25 * ((y as f64 * 0.2).sin() + (x as f64 * 0.3).cos());
                data.push(v);
            }
        }
    }
    data
}

fn char_text(arr: &Array) -> String {
    match arr {
        Array::Character(c) => c.to_string(),
        other => panic!("expected Character array, got {other:?}"),
    }
}

const BUILD_V_SCRIPT: &str = "
V = zeros(32, 24, 16);
for z = 0:15
    for x = 0:23
        for y = 0:31
            V(y+1, x+1, z+1) = mod(y*7 + x*13 + z*19, 101) / 100 + 0.25 * (sin(y*0.2) + cos(x*0.3));
        end
    end
end
";

#[test]
fn volume_show_mip_axis_aligned_matches_max_projection_pixel_for_pixel() {
    let (ny, nx, nz) = (32, 24, 16);
    let data = build_test_volume(ny, nx, nz);
    let mut e = Engine::default();
    let script = format!(
        "{BUILD_V_SCRIPT}\nv = volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray'); view(0, 90);"
    );
    e.execute(&script).unwrap();
    let fig = e.figures().next().unwrap().1;
    let vol = &fig.axes.extras.volumes[0];
    assert_eq!(vol.dims, [ny, nx, nz]);
    let (w, h, pixels) = render_volume(vol, fig.axes.view, None);
    assert_eq!((w, h), (nx, ny));

    let gray = colormap::named("gray", 256).unwrap().unwrap();
    let [dmin, dmax] = vol.data_limits;
    for row in 0..ny {
        for col in 0..nx {
            let mut max_v = f64::NEG_INFINITY;
            for z in 0..nz {
                max_v = max_v.max(data[row + ny * (col + nx * z)]);
            }
            let norm = ((max_v - dmin) / (dmax - dmin)).clamp(0.0, 1.0);
            let idx = (norm * 255.0).round() as usize;
            let expected_rgb = gray[idx].map(|c| (c * 255.0).round() as u8);
            assert_eq!(
                pixels[row * nx + col],
                [expected_rgb[0], expected_rgb[1], expected_rgb[2], 255],
                "mismatch at row={row}, col={col}"
            );
        }
    }
}

#[test]
fn volume_show_minip_axis_aligned_matches_min_projection_pixel_for_pixel() {
    let (ny, nx, nz) = (32, 24, 16);
    let data = build_test_volume(ny, nx, nz);
    let mut e = Engine::default();
    let script = format!(
        "{BUILD_V_SCRIPT}\nv = volshow(V, 'RenderingStyle', 'MinimumIntensityProjection', 'Colormap', 'gray'); view(0, 90);"
    );
    e.execute(&script).unwrap();
    let fig = e.figures().next().unwrap().1;
    let vol = &fig.axes.extras.volumes[0];
    let (w, h, pixels) = render_volume(vol, fig.axes.view, None);
    assert_eq!((w, h), (nx, ny));

    let gray = colormap::named("gray", 256).unwrap().unwrap();
    let [dmin, dmax] = vol.data_limits;
    for row in 0..ny {
        for col in 0..nx {
            let mut min_v = f64::INFINITY;
            for z in 0..nz {
                min_v = min_v.min(data[row + ny * (col + nx * z)]);
            }
            let norm = ((min_v - dmin) / (dmax - dmin)).clamp(0.0, 1.0);
            let idx = (norm * 255.0).round() as usize;
            let expected_rgb = gray[idx].map(|c| (c * 255.0).round() as u8);
            assert_eq!(
                pixels[row * nx + col],
                [expected_rgb[0], expected_rgb[1], expected_rgb[2], 255],
                "mismatch at row={row}, col={col}"
            );
        }
    }
}

#[cfg(feature = "plot-export")]
fn svg_mip(group: &resvg::usvg::Group) -> Option<&resvg::usvg::Image> {
    use resvg::usvg::Node;
    group.children().iter().find_map(|node| match node {
        Node::Image(image) => Some(image.as_ref()),
        Node::Group(group) => svg_mip(group),
        _ => None,
    })
}

#[cfg(feature = "plot-export")]
#[test]
fn volume_show_svg_voxel_and_marker_share_data_coordinates() {
    use resvg::usvg::{ImageKind, Options, Tree, roxmltree};

    for (ny, nx, row, col) in [(3, 3, 1, 2), (2, 4, 2, 1)] {
        let (x_max, y_max) = (nx as f64 + 0.5, ny as f64 + 0.5);
        let mut e = Engine::default();
        e.execute(&format!(
            "V=zeros({ny},{nx},2); V({row},{col},1)=1; \
             volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray'); \
             view(0,90); hold on; plot({col},{row},'ro'); \
             axis([.5 {x_max} .5 {y_max}]); axis off;"
        ))
        .unwrap();
        for (direction, axis_command) in [("normal", "axis xy;"), ("reverse", "axis ij;")] {
            e.execute(axis_command).unwrap();
            let svg = e.figures().next().unwrap().1.svg();
            let tree = Tree::from_str(&svg, &Options::default()).unwrap();
            let image = svg_mip(tree.root()).expect("embedded MIP");
            let ImageKind::PNG(png) = image.kind() else {
                panic!("expected an embedded PNG");
            };
            let pixels = image::load_from_memory(png).unwrap().to_rgba8();
            assert_eq!(pixels.dimensions(), (nx * 4, ny * 4));
            let (mut x_sum, mut y_sum, mut weight) = (0., 0., 0.);
            for (x, y, p) in pixels.enumerate_pixels() {
                let light = f32::from(p[0]);
                x_sum += (x as f32 + 0.5) * light;
                y_sum += (y as f32 + 0.5) * light;
                weight += light;
            }
            assert!(weight > 0.);
            let transform = image.abs_transform();
            let mut voxel = resvg::tiny_skia::Point::from_xy(x_sum / weight, y_sum / weight);
            transform.map_point(&mut voxel);

            let xml = roxmltree::Document::parse(&svg).unwrap();
            let marker = xml
                .descendants()
                .find(|node| {
                    node.has_tag_name("circle") && node.attribute("stroke") == Some("#ff0000")
                })
                .expect("red circle marker");
            let coordinate = |name| marker.attribute(name).unwrap().parse::<f32>().unwrap();
            let marker = [coordinate("cx"), coordinate("cy")];
            eprintln!(
                "SVG {ny}x{nx} {direction}: voxel=({:.2}, {:.2}), marker={marker:.2?}",
                voxel.x, voxel.y
            );
            assert!((voxel.x - marker[0]).abs() <= transform.sx.abs());
            assert!((voxel.y - marker[1]).abs() <= transform.sy.abs());
        }
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn volume_show_svg_mip_and_line_stack_in_child_order_after_reopen() {
    use resvg::tiny_skia::{Pixmap, Point, Transform};
    use resvg::usvg::{Options, Tree};

    let directory = tempfile::tempdir().unwrap();
    for volume_on_top in [true, false] {
        let mut e = Engine::default();
        e.set_file_directory(directory.path()).unwrap();
        let line = "h=plot([-1 5],[2 2],'r-','LineWidth',3);";
        let volume = "v=volshow(zeros(3,3,2), RenderingStyle='MaximumIntensityProjection', Colormap='gray');";
        let (first, last, children) = if volume_on_top {
            (line, volume, "[v;h]")
        } else {
            (volume, line, "[h;v]")
        };
        e.execute(&format!(
            "subplot(1,2,1); plot([0 1],[0 1],'b-'); \
             ax=subplot(1,2,2); hold on; {first} {last} \
             view(0,90); axis([-2 6 .5 3.5]); axis off; \
             assert(isequal(ax.Children,{children})); savefig('stack.mrfig');"
        ))
        .unwrap();
        for reopened in [false, true] {
            if reopened {
                e.execute(&format!(
                    "close all; openfig('stack.mrfig'); ax=subplot(1,2,2); \
                     h=findobj(ax,'Type','line'); v=findobj(ax,'Type','volume'); \
                     assert(isequal(ax.Children,{children}));"
                ))
                .unwrap();
            }
            let svg = e.figures().next().unwrap().1.svg();
            let tree = Tree::from_str(&svg, &Options::default()).unwrap();
            let image = svg_mip(tree.root()).expect("embedded MIP");
            let transform = image.abs_transform();
            let [w, h] = [image.size().width(), image.size().height()];
            let size = tree.size().to_int_size();
            let mut pixmap = Pixmap::new(size.width(), size.height()).unwrap();
            resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
            for (fraction, red) in [(-0.25, true), (0.5, !volume_on_top), (1.25, true)] {
                let mut position = Point::from_xy(w * fraction, h * 0.5);
                transform.map_point(&mut position);
                let pixel = pixmap.pixel(position.x as u32, position.y as u32).unwrap();
                eprintln!(
                    "SVG volume_on_top={volume_on_top}, reopened={reopened}: ({:.2}, {:.2}) red={} alpha={}",
                    position.x,
                    position.y,
                    pixel.red(),
                    pixel.alpha()
                );
                assert_eq!(
                    [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()],
                    [if red { 255 } else { 0 }, 0, 0, 255]
                );
            }
        }
    }
}

#[test]
fn volume_show_properties_set_get_and_dot_syntax() {
    let mut e = Engine::default();
    e.execute(
        "V = reshape(1:24, 2, 3, 4); \
         v = volshow(V); \
         rs0 = v.RenderingStyle; \
         v.RenderingStyle = 'Isosurface'; \
         rs1 = get(v, 'RenderingStyle'); \
         set(v, 'RenderingStyle', 'SlicePlanes'); \
         rs2 = v.RenderingStyle; \
         v.Colormap = hot(16); \
         cm = v.Colormap; \
         v.DataLimits = [2 20]; \
         dl = get(v, 'DataLimits'); \
         v.IsosurfaceValue = 12; \
         iv = v.IsosurfaceValue; \
         v.SlicePlaneValues = [1 0 0 -1; 0 1 0 -2]; \
         sp = v.SlicePlaneValues; \
         v.Transformation = [1.5 2 3]; \
         tr = v.Transformation;",
    )
    .unwrap();
    let ws = e.workspace();
    assert_eq!(char_text(ws.get("rs0").unwrap()), "'VolumeRendering'");
    assert_eq!(char_text(ws.get("rs1").unwrap()), "'Isosurface'");
    assert_eq!(char_text(ws.get("rs2").unwrap()), "'SlicePlanes'");
    let Array::Numeric(cm) = ws.get("cm").unwrap() else {
        panic!("expected Colormap matrix");
    };
    assert_eq!((cm.rows(), cm.cols()), (16, 3));
    let Array::Numeric(dl) = ws.get("dl").unwrap() else {
        panic!("expected DataLimits matrix");
    };
    assert_eq!(dl.data(), &[2.0, 20.0]);
    let Array::Numeric(iv) = ws.get("iv").unwrap() else {
        panic!("expected scalar IsosurfaceValue");
    };
    assert_eq!(iv.as_scalar().unwrap(), 12.0);
    let Array::Numeric(sp) = ws.get("sp").unwrap() else {
        panic!("expected SlicePlaneValues matrix");
    };
    assert_eq!((sp.rows(), sp.cols()), (2, 4));
    let Array::Numeric(tr) = ws.get("tr").unwrap() else {
        panic!("expected Transformation matrix");
    };
    assert_eq!(tr.data(), &[1.5, 2.0, 3.0]);
}

#[test]
fn volume_show_children_are_in_stacking_order_and_delete_removes_the_handle() {
    let mut e = Engine::default();
    e.execute(
        "V=reshape(1:24,2,3,4); h=plot(1:3); ax=gca; hold on;
         v1=volshow(V,Parent=ax); k=plot(3:-1:1); v2=volshow(V,Parent=ax,Visible='off');
         expected=[v2;k;v1;h];
         assert(isequal(ax.Children,expected));
         assert(isequal(get(ax,'Children'),expected));
         assert(isequal(allchild(ax),expected));
         assert(isequal(findobj(ax,'-depth',1,'Type','volume'),[v2;v1]));
         assert(isequal(findobj(v1,'-depth',0,'Type','volume'),v1));
         delete(v2); assert(~isgraphics(v2));
         assert(isequal(ax.Children,[k;v1;h]));
         assert(isequal(allchild(ax),[k;v1;h]));
         assert(isequal(findobj(ax,'Type','volume'),v1));",
    )
    .unwrap();
}

#[test]
fn volume_show_parent_moves_the_same_object_and_rejects_invalid_targets() {
    let mut e = Engine::default();
    e.execute(
        "f1=figure(1); subplot(1,2,1); ax=gca; subplot(1,2,2); bx=gca;
         V=reshape(1:24,2,3,4); v=volshow(V,Parent=ax); alias=v;
         other=volshow(V,Parent=bx); axes(ax);
         v.Parent=bx;
         assert(isequal(alias.Parent,bx) && isequal(get(v,'Parent'),bx));
         assert(isempty(ax.Children) && isequal(bx.Children,[v;other]));
         assert(isequal(gca,ax));
         assert(isempty(findobj(ax,'Type','volume')));
         assert(isequal(findobj(bx,'Type','volume'),[v;other]));
         f2=figure(2); cx=gca; view(3);
         set(v,'Parent',cx,'RenderingStyle','MaximumIntensityProjection');
         assert(isequal(get(alias,'Parent'),cx) && isequal(cx.Children,v));
         assert(isequal(findobj(f2,'Type','volume'),alias));
         assert(isequal(bx.Children,other)); assert(isequal(gcf,f2));",
    )
    .unwrap();
    let f2 = e.figures().find(|(n, _)| *n == 2).unwrap().1;
    assert_eq!(f2.axes.extras.volumes.len(), 1);
    assert!(f2.svg().contains("data:image/png;base64,"));
    let f1 = e.figures().find(|(n, _)| *n == 1).unwrap().1;
    assert!(f1.all_axes().any(|(_, a)| a.extras.volumes.is_empty()));

    e.execute("f3=figure(3); stale=gca; close(f3);").unwrap();
    for bad in [
        "v.Parent=f2;",
        "set(v,'Visible','off','Parent',stale);",
        "v.Parent=[ax bx];",
        "set(v,'Parent',0);",
    ] {
        let err = e.execute(bad).unwrap_err();
        assert!(
            err.contains("Parent must be a scalar live Axes handle"),
            "{bad}: {err}"
        );
        e.execute("assert(isequal(v.Parent,cx) && isequal(cx.Children,alias)); assert(strcmp(v.Visible,'on'));").unwrap();
    }
    e.execute("close(f1); assert(isgraphics(alias)); delete(alias); assert(isempty(cx.Children) && ~isgraphics(v));").unwrap();
}

#[test]
fn volume_show_alphamap_and_volume_alphamap_are_independent() {
    let mut e = Engine::default();
    e.execute(
        "V = reshape(1:27, 3, 3, 3); \
         v = volshow(V); \
         va0 = v.Alphamap; \
         alphamap('rampdown'); \
         fa = alphamap; \
         va1 = v.Alphamap; \
         v.Alphamap = [0 0.5 1]; \
         va2 = v.Alphamap; \
         fa2 = alphamap;",
    )
    .unwrap();
    let ws = e.workspace();
    let Array::Numeric(fa) = ws.get("fa").unwrap() else {
        panic!("expected figure alphamap matrix");
    };
    assert_eq!(fa.len(), 64);
    assert!((fa.data()[0] - 1.0).abs() < 1e-12);
    assert!(fa.data()[63].abs() < 1e-12);

    let Array::Numeric(va0) = ws.get("va0").unwrap() else {
        panic!("expected volume alphamap");
    };
    let Array::Numeric(va1) = ws.get("va1").unwrap() else {
        panic!("expected volume alphamap");
    };
    assert_eq!(va0.data(), va1.data());

    let Array::Numeric(va2) = ws.get("va2").unwrap() else {
        panic!("expected updated volume alphamap");
    };
    assert_eq!(va2.data(), &[0.0, 0.5, 1.0]);

    let Array::Numeric(fa2) = ws.get("fa2").unwrap() else {
        panic!("expected figure alphamap unchanged");
    };
    assert_eq!(fa2.len(), 64);
    assert!((fa2.data()[0] - 1.0).abs() < 1e-12);
}

#[test]
fn volume_show_preserves_wide_integer_low_bits() {
    let mut e = Engine::default();
    e.execute(
        "u=bitset(bitshift(uint64(1),53),1); V=repmat(u,2,2,2); v=volshow(V); W=v.Data;
         input_bit=bitget(V(1),1); output_bit=bitget(W(1),1);",
    )
    .unwrap();
    for name in ["input_bit", "output_bit"] {
        let Array::Typed(bit) = e.array(name).unwrap() else {
            panic!("expected a typed bit result for {name}");
        };
        assert_eq!(bit.int(0), 1, "{name}");
    }

    e.execute(
        "G=get(v,'Data');
         U=V; U(2,1,1)=intmax('uint64')-uint64(2); U(1,2,1)=bitset(bitshift(uint64(1),63),1);
         v.Data=U; Q=get(v,'Data');
         s=-bitset(bitshift(int64(1),53),1); S=repmat(s,2,3,2);
         S(2,1,1)=intmin('int64'); S(1,2,1)=intmin('int64')+int64(1);
         S(2,2,1)=intmax('int64')-int64(2);
         hold on; vs=volshow(S); C=vs.Data;
         v.Data=S; D=v.Data; E=get(v,'Data');
         set(v,'Data',U); RU=v.Data; set(v,'Data',S); RS=v.Data;
         assert(all(bitxor(U(:),RU(:)) == uint64(0)) && all(bitxor(S(:),RS(:)) == int64(0)));
         assert(all(bitxor(V(:),W(:)) == uint64(0)));
         assert(all(bitxor(V(:),G(:)) == uint64(0)));
         assert(all(bitxor(U(:),Q(:)) == uint64(0)));
         assert(all(bitxor(S(:),C(:)) == int64(0)));
         assert(all(bitxor(S(:),D(:)) == int64(0)));
         assert(all(bitxor(S(:),E(:)) == int64(0)));
         A=uint8(reshape(1:12,2,3,2)); v.Data=A;
         assert(strcmp(class(v.Data),'uint8') && isequal(v.Data,A));",
    )
    .unwrap();

    for (input, output, class, dims) in [
        ("V", "W", "uint64", [2, 2, 2]),
        ("V", "G", "uint64", [2, 2, 2]),
        ("U", "Q", "uint64", [2, 2, 2]),
        ("S", "C", "int64", [2, 3, 2]),
        ("S", "D", "int64", [2, 3, 2]),
        ("S", "E", "int64", [2, 3, 2]),
        ("U", "RU", "uint64", [2, 2, 2]),
        ("S", "RS", "int64", [2, 3, 2]),
    ] {
        let (Array::Typed(input), Array::Typed(readback)) =
            (e.array(input).unwrap(), e.array(output).unwrap())
        else {
            panic!("expected typed volume data for {input} and {output}");
        };
        assert_eq!(readback.class().name(), class, "{output}");
        assert_eq!(readback.dims().as_ref(), &dims, "{output}");
        assert!(readback.same_storage(&input), "{output}");
    }
}

#[test]
fn volume_show_accepts_typed_and_logical_and_rejects_non_3d() {
    let mut e = Engine::default();
    e.execute(
        "V1 = uint8(reshape(1:24, 2, 3, 4)); v1 = volshow(V1); d1 = v1.Data; \
         V2 = single(reshape(1:24, 2, 3, 4)); v2 = volshow(V2); d2 = v2.Data; \
         V3 = logical(reshape(mod(1:24, 2), 2, 3, 4)); v3 = volshow(V3); d3 = v3.Data;
         V4 = reshape(1:24, 2, 3, 4); v4 = volshow(V4); d4 = v4.Data;
         assert(isequal(d1,V1) && isequal(d2,V2) && isequal(d3,V3) && isequal(d4,V4));",
    )
    .unwrap();
    for (name, class) in [
        ("d1", "uint8"),
        ("d2", "single"),
        ("d3", "logical"),
        ("d4", "double"),
    ] {
        let data = e.array(name).unwrap();
        assert_eq!(data.class(), class);
        assert_eq!(data.dims().as_ref(), &[2, 3, 4]);
    }

    assert!(e.execute("volshow(ones(4, 4));").is_err());
    assert!(e.execute("volshow(ones(2, 2, 2, 2));").is_err());
}

#[test]
fn volume_show_exportgraphics_writes_png_with_non_background_center() {
    let dir = std::env::temp_dir().join("matrust_volume_show_export_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let png_path = dir.join("vol.png");
    let mut e = Engine::default();
    e.set_file_directory(&dir).unwrap();
    e.execute(
        "V = zeros(16, 16, 16); V(5:12, 5:12, 5:12) = 1; \
         volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='hot'); \
         exportgraphics(gcf, 'vol.png');",
    )
    .unwrap();
    let bytes = std::fs::read(&png_path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
    let (w, h) = img.dimensions();
    assert!(w > 10 && h > 10);
    let mut non_white = 0usize;
    for y in (h / 4)..(3 * h / 4) {
        for x in (w / 4)..(3 * w / 4) {
            let p = img.get_pixel(x, y).0;
            if p[0] != 255 || p[1] != 255 || p[2] != 255 {
                non_white += 1;
            }
        }
    }
    assert!(non_white > 0, "expected non-background pixels in centre");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn volume_show_native_opaque_logical_overlay_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "mask = true(2,2,2); \
         v = volshow(zeros(2,2,2), RenderingStyle='MaximumIntensityProjection', Colormap='gray', \
             OverlayData=mask, OverlayRenderingStyle='LabelOverlay', \
             OverlayColormap=[1 0 0], OverlayAlphamap=1); \
         view(0,90); n_before = numel(v.OverlayData); \
         cmap = v.OverlayColormap; alpha = v.OverlayAlphamap;",
    )
    .unwrap();
    let pixels = |e: &Engine| {
        let fig = e.figures().next().unwrap().1;
        render_volume(&fig.axes.extras.volumes[0], fig.axes.view, None)
    };
    let before = pixels(&e);
    assert_eq!(before, (2, 2, vec![[255, 0, 0, 255]; 4]));
    e.execute(
        "savefig('opaque.mrfig'); close all; f = openfig('opaque.mrfig'); \
         ax = f.Children; children = ax.Children; reopened = children(1); \
         n_after = numel(reopened.OverlayData);",
    )
    .unwrap();
    let count = |name| match e.workspace().get(name).unwrap() {
        Array::Numeric(m) => m.data()[0],
        other => panic!("expected numeric count, got {other:?}"),
    };
    assert_eq!((count("n_before"), count("n_after")), (8., 8.));
    e.execute(
        "assert(strcmp(class(reopened.OverlayData),'logical')); \
         assert(isequal(size(reopened.OverlayData),[2 2 2])); \
         assert(isequal(reopened.OverlayData,mask)); \
         assert(strcmp(reopened.OverlayRenderingStyle,'LabelOverlay')); \
         assert(isequal(reopened.OverlayColormap,cmap)); \
         assert(isequal(reopened.OverlayAlphamap,alpha));",
    )
    .unwrap();
    assert_eq!(pixels(&e), before);
}

#[test]
fn volume_show_native_asymmetric_typed_overlay_keeps_samples_settings_and_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "mask = reshape(uint8([1 0 0 1 0 1 0 0]),2,2,2); \
         v = volshow(zeros(2,2,2), RenderingStyle='MaximumIntensityProjection', Colormap='gray', \
             OverlayData=mask, OverlayRenderingStyle='VolumeOverlay', \
             OverlayColormap=[0 0 0; 1 0 0], OverlayAlphamap=[0 0.5], OverlayDisplayRange=[0.25 0.75]); \
         view(0,90); cmap = v.OverlayColormap; alpha = v.OverlayAlphamap;",
    )
    .unwrap();
    let pixels = |e: &Engine| {
        let fig = e.figures().next().unwrap().1;
        render_volume(&fig.axes.extras.volumes[0], fig.axes.view, None)
    };
    let before = pixels(&e);
    assert_eq!(before.0, 2);
    assert_eq!(before.1, 2);
    assert_eq!(before.2[1], [0, 0, 0, 255]);
    for i in [0, 2, 3] {
        assert!(before.2[i][0] > 0 && before.2[i][0] < 255);
    }
    e.execute(
        "savefig('mask.mrfig'); close all; f = openfig('mask.mrfig'); \
         ax = f.Children; children = ax.Children; reopened = children(1); \
         assert(strcmp(class(reopened.OverlayData),'uint8')); \
         assert(isequal(size(reopened.OverlayData),[2 2 2])); \
         assert(isequal(reopened.OverlayData,mask)); \
         assert(strcmp(reopened.OverlayRenderingStyle,'VolumeOverlay')); \
         assert(isequal(reopened.OverlayColormap,cmap)); \
         assert(isequal(reopened.OverlayAlphamap,alpha)); \
         assert(isequal(reopened.OverlayDisplayRange,[0.25 0.75]));",
    )
    .unwrap();
    let vol = &e.figures().next().unwrap().1.axes.extras.volumes[0];
    assert!(vol.overlay_display_range_manual);
    assert_eq!(
        vol.overlay_data.as_ref().unwrap().data.as_ref(),
        [1., 0., 0., 1., 0., 1., 0., 0.]
    );
    assert_eq!(pixels(&e), before);
}

#[test]
fn volume_show_native_exact_signed_and_unsigned_voxels() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "U = repmat(bitset(bitshift(uint64(1),53),1),2,2,2); \
         U(2,1,1) = intmax('uint64')-uint64(2); \
         U(1,2,2) = bitset(bitshift(uint64(1),63),1); \
         S = repmat(-bitset(bitshift(int64(1),53),1),2,2,2); \
         S(1,2,1) = intmin('int64')+int64(1); \
         S(2,1,2) = intmax('int64')-int64(2); \
         volshow(U); hold on; volshow(S); \
         savefig('exact.mrfig'); close all; f = openfig('exact.mrfig'); \
         ax = f.Children; children = ax.Children; si = children(1); ui = children(2); \
         R = si.Data; Q = ui.Data; \
         assert(strcmp(class(R),'int64') && strcmp(class(Q),'uint64')); \
         assert(isequal(size(R),[2 2 2]) && isequal(size(Q),[2 2 2])); \
         assert(all(bitxor(S(:),R(:)) == int64(0))); \
         assert(all(bitxor(U(:),Q(:)) == uint64(0)));",
    )
    .unwrap();
}

#[test]
fn volume_show_save_and_load_figure_keeps_rendering_style() {
    let dir = std::env::temp_dir().join("matrust_volume_show_fig_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::default();
    e.set_file_directory(&dir).unwrap();
    e.execute(
        "V = reshape(1:27, 3, 3, 3); \
         v = volshow(V, RenderingStyle='MinimumIntensityProjection'); \
         savefig('vol.mrfig'); \
         close all; \
         f = openfig('vol.mrfig');",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    assert_eq!(fig.axes.extras.volumes.len(), 1);
    assert_eq!(
        fig.axes.extras.volumes[0].rendering_style,
        RenderingStyle::MinimumIntensityProjection
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn volume_show_display_camera_and_native_spacing() {
    let mut e = Engine::default();
    e.execute("V = reshape(1:24, 2, 3, 4); volshow(V, RenderingStyle='MaximumIntensityProjection', Transformation=[2 3 1]); view(3);").unwrap();
    let fig = e.figures().next().unwrap().1;
    let display = matrust::plot::view3d::display_view(&fig.axes);
    let vol = &display.extras.volumes[0];
    assert_eq!(vol.display_view, Some(fig.axes.view));
    let a = render_volume(vol, vol.display_view.unwrap(), Some((32, 32)));
    let b = render_volume(vol, [0., 90.], Some((32, 32)));
    assert_ne!(a.2, b.2);
    assert_eq!(
        {
            let (w, h, _) = render_volume(vol, [0., 90.], None);
            (w, h)
        },
        (3, 2)
    );
    let default_svg = fig.svg();
    e.execute("view(0,90);").unwrap();
    let top_svg = e.figures().next().unwrap().1.svg();
    let raster = |svg: &str| {
        svg.split("data:image/png;base64,")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_owned()
    };
    assert_ne!(raster(&default_svg), raster(&top_svg));
}

#[test]
fn volume_show_limits_and_alphamap_outputs() {
    let mut e = Engine::default();
    e.execute("v = volshow(reshape(1:24,2,3,4)); v.Data = reshape(31:54,2,3,4); auto = v.DataLimits; v.DataLimits = [0 100]; v.Data = reshape(61:84,2,3,4); manual = v.DataLimits; before = alphamap; a = alphamap('rampdown'); b = alphamap('increase', 0.1); after = alphamap;").unwrap();
    let ws = e.workspace();
    let numeric = |name| match ws.get(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        _ => panic!("expected numeric"),
    };
    assert_eq!(numeric("auto"), [31., 54.]);
    assert_eq!(numeric("manual"), [0., 100.]);
    assert_eq!(numeric("before"), numeric("after"));
    assert_eq!(numeric("a")[0], 1.);
    assert!((numeric("b")[0] - 0.1).abs() < 1e-12);
}

#[test]
fn volume_show_extreme_spacing_march_terminates() {
    let mut e = Engine::default();
    e.execute("volshow(reshape(1:24,2,3,4), RenderingStyle='MaximumIntensityProjection', Transformation=[1 1 1e30]);").unwrap();
    let fig = e.figures().next().unwrap().1;
    let (w, h, _) = render_volume(&fig.axes.extras.volumes[0], [-37.5, 30.], Some((4, 4)));
    assert_eq!((w, h), (4, 4));
}

#[test]
fn volume_show_subplot_roundtrip_and_matlab_fig_rejection() {
    let dir = std::env::temp_dir().join(format!("matrust_volume_subplot_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::default();
    e.set_file_directory(&dir).unwrap();
    e.execute("subplot(1,2,1); volshow(reshape(1:24,2,3,4)); subplot(1,2,2); plot(1:3); savefig('sub.mrfig'); close all; openfig('sub.mrfig');").unwrap();
    assert_eq!(
        e.figures()
            .next()
            .unwrap()
            .1
            .all_axes()
            .map(|(_, a)| a.extras.volumes.len())
            .sum::<usize>(),
        1
    );
    let err = e.execute("savefig('sub.fig');").unwrap_err();
    assert!(
        err.contains("volshow objects are saved only in .mrfig"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn volume_show_oversized_native_save_rejected() {
    use matrust::files::{encode_native_figure, save_figure_with_limit};

    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("plot(1:2); savefig('prior.mrfig'); close all;")
        .unwrap();
    let prior_path = dir.path().join("prior.mrfig");
    let prior = std::fs::read(&prior_path).unwrap();
    for script in [
        "volshow(zeros(2,2,2),OverlayData=true(2,2,2));",
        "subplot(1,2,1); volshow(zeros(2,2,2),OverlayData=true(2,2,2)); \
         subplot(1,2,2); volshow(zeros(2,2,2),OverlayData=true(2,2,2));",
    ] {
        e.execute(script).unwrap();
        let fig = e.figures().next().unwrap().1;
        let size = encode_native_figure(fig).unwrap().len();
        // This budget fits either pane in isolation but misses the full record
        // by one eight-byte voxel.
        let limit = size - 8;
        for name in ["prior.mrfig", "missing.mrfig"] {
            let err = save_figure_with_limit(&dir.path().join(name), fig, limit).unwrap_err();
            assert!(err.contains(&format!("{limit} byte save limit")), "{err}");
        }
        let mut without_overlay = fig.clone();
        without_overlay.axes.extras.volumes[0].overlay_data = None;
        assert!(encode_native_figure(&without_overlay).unwrap().len() <= limit);
        assert_eq!(std::fs::read(&prior_path).unwrap(), prior);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        e.execute("close all;").unwrap();
    }
}

#[test]
fn volume_show_overlay_properties_defaults_roundtrip_and_display() {
    let mut e = Engine::default();
    let out = e
        .execute(
            "V = reshape(1:24, 2, 3, 4); \
             v = volshow(V); \
             od0 = v.OverlayData; \
             ors0 = v.OverlayRenderingStyle; \
             ocm0 = v.OverlayColormap; \
             oam0 = v.OverlayAlphamap; \
             odr0 = v.OverlayDisplayRange; \
             disp(v); \
             L = zeros(2, 3, 4); L(1, 1, 1) = 1; \
             v.OverlayData = L; \
             od1 = get(v, 'OverlayData'); \
             set(v, 'OverlayRenderingStyle', 'VolumeOverlay', 'OverlayAlphamap', 0.25, 'OverlayDisplayRange', [0 5]); \
             ors1 = v.OverlayRenderingStyle; \
             oam1 = get(v, 'OverlayAlphamap'); \
             odr1 = v.OverlayDisplayRange; \
             v.OverlayColormap = [1 0 0; 0 1 0]; \
             ocm1 = v.OverlayColormap; \
             v.OverlayData = []; \
             od2 = v.OverlayData;",
        )
        .unwrap();
    let ws = e.workspace();
    assert!(ws.get("od0").unwrap().is_empty());
    assert!(ws.get("od2").unwrap().is_empty());
    assert_eq!(char_text(ws.get("ors0").unwrap()), "'LabelOverlay'");
    assert_eq!(char_text(ws.get("ors1").unwrap()), "'VolumeOverlay'");
    let Array::Numeric(ocm0) = ws.get("ocm0").unwrap() else {
        panic!("expected OverlayColormap matrix");
    };
    assert_eq!((ocm0.rows(), ocm0.cols()), (256, 3));
    let Array::Numeric(oam0) = ws.get("oam0").unwrap() else {
        panic!("expected OverlayAlphamap scalar");
    };
    assert_eq!(oam0.as_scalar().unwrap(), 0.5);
    let Array::Numeric(odr0) = ws.get("odr0").unwrap() else {
        panic!("expected OverlayDisplayRange");
    };
    assert_eq!(odr0.data(), &[0.0, 1.0]);
    let Array::Numeric(od1) = ws.get("od1").unwrap() else {
        panic!("expected OverlayData matrix");
    };
    assert_eq!(od1.dims().as_ref(), &[2, 3, 4]);
    assert_eq!(od1.data()[0], 1.0);
    let Array::Numeric(oam1) = ws.get("oam1").unwrap() else {
        panic!("expected OverlayAlphamap");
    };
    assert_eq!(oam1.as_scalar().unwrap(), 0.25);
    let Array::Numeric(odr1) = ws.get("odr1").unwrap() else {
        panic!("expected OverlayDisplayRange");
    };
    assert_eq!(odr1.data(), &[0.0, 5.0]);
    let Array::Numeric(ocm1) = ws.get("ocm1").unwrap() else {
        panic!("expected OverlayColormap");
    };
    assert_eq!((ocm1.rows(), ocm1.cols()), (2, 3));
    let shown = out.output.join("\n");
    assert!(shown.contains("OverlayData:"), "{shown}");
    assert!(shown.contains("OverlayRenderingStyle:"), "{shown}");
    assert!(shown.contains("OverlayColormap:"), "{shown}");
    assert!(shown.contains("OverlayAlphamap:"), "{shown}");
    assert!(shown.contains("OverlayDisplayRange:"), "{shown}");
}

#[test]
fn volume_show_overlay_size_mismatch_and_invalid_style_error() {
    let mut e = Engine::default();
    let err = e
        .execute("V = zeros(4, 5, 6); volshow(V, OverlayData=zeros(4, 5, 7));")
        .unwrap_err();
    assert!(!err.contains('\n'), "expected one error line: {err}");
    assert!(err.contains("4x5x7") && err.contains("4x5x6"), "{err}");

    let err_style = e
        .execute("V = zeros(4, 5, 6); volshow(V, OverlayRenderingStyle='BadStyle');")
        .unwrap_err();
    assert!(err_style.contains("OverlayRenderingStyle"), "{err_style}");
}

#[test]
fn volume_show_data_replacement_preserves_overlay_shape_and_state() {
    let mut e = Engine::default();
    e.execute(
        "V=reshape(1:12,2,3,2); L=zeros(2,3,2,'uint8'); L(1,2,1)=1;
         v=volshow(V,OverlayData=L,RenderingStyle='MaximumIntensityProjection');",
    )
    .unwrap();
    let before = e.figures().next().unwrap().1.axes.extras.volumes[0].clone();
    let overlay = before.overlay_data.as_ref().unwrap();
    for (command, dims) in [
        ("v.Data=reshape(V+20,3,2,2);", "3x2x2"),
        (
            "set(v,'Visible','off','Data',reshape(V+20,2,2,3));",
            "2x2x3",
        ),
        ("set(v,'Data',zeros(3,3,2));", "3x3x2"),
        ("v.Data=zeros(2,4,2);", "2x4x2"),
        ("v.Data=zeros(2,3,3);", "2x3x3"),
    ] {
        let err = e.execute(command).unwrap_err();
        assert!(
            err.contains(&format!(
                "Data size [{dims}] must match attached OverlayData size [2x3x2]"
            )),
            "{command}: {err}"
        );
        let after = &e.figures().next().unwrap().1.axes.extras.volumes[0];
        assert_eq!(after.dims, before.dims);
        assert!(Arc::ptr_eq(&after.data, &before.data));
        assert!(Arc::ptr_eq(
            &after.overlay_data.as_ref().unwrap().data,
            &overlay.data
        ));
        assert!(after.data_revision().same_object(before.data_revision()));
        assert_eq!(after.data_class, before.data_class);
        assert_eq!(after.data_limits, before.data_limits);
        assert_eq!(after.rendering_style, before.rendering_style);
        assert_eq!(after.visible, before.visible);
    }

    e.execute(
        "R=V+20; set(v,'Data',R);
         assert(isequal(v.Data,R));
         assert(strcmp(class(v.OverlayData),'uint8') && isequal(v.OverlayData,L));",
    )
    .unwrap();
    let after = &e.figures().next().unwrap().1.axes.extras.volumes[0];
    assert!(Arc::ptr_eq(
        &after.overlay_data.as_ref().unwrap().data,
        &overlay.data
    ));
    assert!(!after.data_revision().same_object(before.data_revision()));

    e.execute(
        "v.OverlayData=[]; R=reshape(R,3,2,2); v.Data=R;
         assert(isequal(v.Data,R) && isempty(v.OverlayData));",
    )
    .unwrap();
}

#[test]
fn volume_show_empty_overlay_renders_byte_identical() {
    let mut e = Engine::default();
    e.execute(
        "V = reshape(1:64, 4, 4, 4); \
         v1 = volshow(V); hold on; \
         v2 = volshow(V, OverlayData=[]);",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    let v1 = &fig.axes.extras.volumes[0];
    let v2 = &fig.axes.extras.volumes[1];
    for style in [
        RenderingStyle::VolumeRendering,
        RenderingStyle::MaximumIntensityProjection,
        RenderingStyle::MinimumIntensityProjection,
        RenderingStyle::Isosurface,
        RenderingStyle::SlicePlanes,
    ] {
        let mut a = v1.clone();
        let mut b = v2.clone();
        a.rendering_style = style;
        b.rendering_style = style;
        let ra = render_volume(&a, [-37.5, 30.0], Some((24, 24)));
        let rb = render_volume(&b, [-37.5, 30.0], Some((24, 24)));
        assert_eq!(ra, rb, "mismatch for {style:?}");
    }
}

#[test]
fn volume_show_label_overlay_cube_mip_az0_el90_and_alpha_monotonicity() {
    let mut e = Engine::default();
    e.execute(
        "V = zeros(32, 32, 32); \
         L = zeros(32, 32, 32); \
         L(13:20, 13:20, 13:20) = 1; \
         v0 = volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray'); \
         hold on; \
         v_zero = volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray', OverlayData=L, OverlayAlphamap=0); \
         v_02 = volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray', OverlayData=L, OverlayAlphamap=0.2); \
         v_05 = volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray', OverlayData=L, OverlayAlphamap=0.5); \
         v_10 = volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray', OverlayData=L, OverlayAlphamap=1.0); \
         view(0, 90);",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    let vols = &fig.axes.extras.volumes;
    let base = render_volume(&vols[0], [0.0, 90.0], None);
    let zero = render_volume(&vols[1], [0.0, 90.0], None);
    let r02 = render_volume(&vols[2], [0.0, 90.0], None);
    let r05 = render_volume(&vols[3], [0.0, 90.0], None);
    let r10 = render_volume(&vols[4], [0.0, 90.0], None);
    assert_eq!((base.0, base.1), (32, 32));
    assert_eq!(base, zero, "OverlayAlphamap 0 must equal no-overlay render");

    // Rays outside the 8x8x8 cube (indices 12..20) must equal base exactly.
    for row in 0..32 {
        for col in 0..32 {
            if !(12..20).contains(&row) || !(12..20).contains(&col) {
                let idx = row * 32 + col;
                assert_eq!(
                    r05.2[idx], base.2[idx],
                    "outside pixel ({row},{col}) changed"
                );
            }
        }
    }

    // Centre pixel (16, 16) differs and moves monotonically toward OverlayColormap(1,:).
    let centre = 16 * 32 + 16;
    let p_base = base.2[centre];
    let p02 = r02.2[centre];
    let p05 = r05.2[centre];
    let p10 = r10.2[centre];
    assert_ne!(p02, p_base);
    let target = vols[4].overlay_colormap[0].map(|c| (c * 255.0).round() as u8);
    assert_eq!([p10[0], p10[1], p10[2]], target);
    let dist = |p: [u8; 4]| -> i32 { (0..3).map(|c| (p[c] as i32 - target[c] as i32).abs()).sum() };
    assert!(dist(p02) < dist(p_base), "p02={p02:?}, p_base={p_base:?}");
    assert!(dist(p05) < dist(p02), "p05={p05:?}, p02={p02:?}");
    assert_eq!(dist(p10), 0);
}

#[test]
fn volume_show_label_overlay_two_labels_and_categorical() {
    let mut e = Engine::default();
    e.execute(
        "V = zeros(8, 8, 8); \
         L = zeros(8, 8, 8); \
         L(2:3, 2:3, 2:3) = 1; \
         L(6:7, 6:7, 6:7) = 2; \
         cmap = [1 0 0; 0 0 1]; \
         v = volshow(V, RenderingStyle='MaximumIntensityProjection', OverlayData=L, OverlayColormap=cmap, OverlayAlphamap=1); \
         hold on; \
         C = categorical(L, [1 2], {'one', 'two'}); \
         vc = volshow(V, RenderingStyle='MaximumIntensityProjection', OverlayData=C, OverlayColormap=cmap, OverlayAlphamap=1); \
         view(0, 90);",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    let v = &fig.axes.extras.volumes[0];
    let vc = &fig.axes.extras.volumes[1];
    let (_, _, px) = render_volume(v, [0.0, 90.0], None);
    let (_, _, pxc) = render_volume(vc, [0.0, 90.0], None);
    assert_eq!(px[9], [255, 0, 0, 255]);
    assert_eq!(px[5 * 8 + 5], [0, 0, 255, 255]);
    assert_eq!(px[0], [0, 0, 0, 255]);
    assert_eq!(
        px, pxc,
        "categorical overlay should match numeric label overlay"
    );
}

#[test]
fn volume_show_volume_overlay_smoke_and_display_range_clipping() {
    let mut e = Engine::default();
    e.execute(
        "V = zeros(8, 8, 8); \
         O = zeros(8, 8, 8); \
         O(2:3, 2:3, 2:3) = 10; \
         O(6:7, 6:7, 6:7) = 100; \
         cmap = [0 0 0; 0 1 0]; \
         v = volshow(V, RenderingStyle='VolumeRendering', Alphamap=0, \
             OverlayData=O, OverlayRenderingStyle='VolumeOverlay', \
             OverlayColormap=cmap, OverlayAlphamap=[0 1], OverlayDisplayRange=[20 50]);",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    let v = &fig.axes.extras.volumes[0];
    let (_, _, px) = render_volume(v, [0.0, 90.0], None);
    // Value 10 <= 20 clips to norm 0 -> alpha 0 -> transparent.
    assert_eq!(px[9], [0, 0, 0, 0]);
    // Value 100 >= 50 clips to norm 1 -> alpha 1, green [0 255 0].
    assert_eq!(px[5 * 8 + 5], [0, 255, 0, 255]);
}

#[test]
#[ignore = "release-mode timing; run with --release -- --ignored"]
fn volume_show_overlay_96_benchmark() {
    use matrust::plot::{OverlayDataClass, OverlayRenderingStyle, OverlayVolume};
    use std::sync::Arc;
    use std::time::Instant;

    let n = 96usize;
    let data: Arc<[f64]> = Arc::from(build_test_volume(n, n, n));
    let mut labels = vec![0.0; n * n * n];
    for z in 24..72 {
        for x in 24..72 {
            for y in 24..72 {
                labels[y + n * (x + n * z)] = if z < 48 { 1.0 } else { 2.0 };
            }
        }
    }
    let overlay = OverlayVolume {
        data: Arc::from(labels),
        class: OverlayDataClass::Double,
    };
    let mut base =
        matrust::plot::VolumeObject::new([n, n, n], data, matrust::plot::ImageDataClass::Double);
    let view = [-37.5, 30.0];
    for style in [
        RenderingStyle::VolumeRendering,
        RenderingStyle::MaximumIntensityProjection,
        RenderingStyle::MinimumIntensityProjection,
        RenderingStyle::Isosurface,
        RenderingStyle::SlicePlanes,
    ] {
        base.rendering_style = style;
        base.overlay_data = None;
        let t0 = Instant::now();
        let _ = render_volume(&base, view, Some((160, 160)));
        let dt_base = t0.elapsed().as_secs_f64() * 1e3;

        base.overlay_data = Some(overlay.clone());
        base.overlay_rendering_style = OverlayRenderingStyle::LabelOverlay;
        let t1 = Instant::now();
        let _ = render_volume(&base, view, Some((160, 160)));
        let dt_lbl = t1.elapsed().as_secs_f64() * 1e3;

        base.overlay_rendering_style = OverlayRenderingStyle::VolumeOverlay;
        let t2 = Instant::now();
        let _ = render_volume(&base, view, Some((160, 160)));
        let dt_vol = t2.elapsed().as_secs_f64() * 1e3;

        eprintln!("{style:?}: base={dt_base:.2}ms label={dt_lbl:.2}ms volume={dt_vol:.2}ms");
    }
}

#[test]
fn volume_show_mip_minip_non_unit_spacing_exact_voxel_centers() {
    // Regression test for single-column 1x1x3 MIP at non-unit z spacing [1 1 0.7]:
    // middle voxel is 0.5 on [0 1] with a 2-entry [black; white] colormap -> white (255).
    let mut e = Engine::default();
    let render_one = |e: &mut Engine, script: &str| -> (usize, usize, Vec<[u8; 4]>) {
        e.execute(script).unwrap();
        let fig = e.figures().next().unwrap().1;
        render_volume(&fig.axes.extras.volumes[0], fig.axes.view, None)
    };
    let (wu, hu, px_unit) = render_one(
        &mut e,
        "V = reshape([0 0.5 0], 1, 1, 3); \
         volshow(V, RenderingStyle='MaximumIntensityProjection', DataLimits=[0 1], Colormap=[0 0 0; 1 1 1], Transformation=[1 1 1]); \
         view(0, 90);",
    );
    let (ws, hs, px_scaled) = render_one(
        &mut e,
        "volshow(V, RenderingStyle='MaximumIntensityProjection', DataLimits=[0 1], Colormap=[0 0 0; 1 1 1], Transformation=[1 1 0.7]); \
         view(0, 90);",
    );
    assert_eq!((wu, hu), (1, 1));
    assert_eq!((ws, hs), (1, 1));
    assert_eq!(px_unit, vec![[255, 255, 255, 255]]);
    assert_eq!(px_scaled, vec![[255, 255, 255, 255]]);

    // Asymmetric 2x3x3 volume with non-unit [0.7 1.3 0.7] spacing and NaN handling,
    // verified against independent min/max reductions and unit-spacing control.
    e.execute(
        "V = zeros(2, 3, 3); \
         V(:,:,1) = [0.0 0.4 0.2; 0.8 NaN 0.1]; \
         V(:,:,2) = [0.5 0.1 0.9; 0.3 0.5 0.6]; \
         V(:,:,3) = [0.2 0.3 0.4; 0.7 0.2 NaN]; \
         cmap = [0 0 0; 1 0 0; 0 1 0; 1 1 1];",
    )
    .unwrap();
    let (w_mip_u, h_mip_u, mip_u) = render_one(
        &mut e,
        "volshow(V, RenderingStyle='MaximumIntensityProjection', DataLimits=[0 1], Colormap=cmap, Transformation=[1 1 1]); view(0, 90);",
    );
    let (w_mip_s, h_mip_s, mip_s) = render_one(
        &mut e,
        "volshow(V, RenderingStyle='MaximumIntensityProjection', DataLimits=[0 1], Colormap=cmap, Transformation=[0.7 1.3 0.7]); view(0, 90);",
    );
    let (w_min_u, h_min_u, min_u) = render_one(
        &mut e,
        "volshow(V, RenderingStyle='MinimumIntensityProjection', DataLimits=[0 1], Colormap=cmap, Transformation=[1 1 1]); view(0, 90);",
    );
    let (w_min_s, h_min_s, min_s) = render_one(
        &mut e,
        "volshow(V, RenderingStyle='MinimumIntensityProjection', DataLimits=[0 1], Colormap=cmap, Transformation=[0.7 1.3 0.7]); view(0, 90);",
    );
    assert_eq!((w_mip_u, h_mip_u), (3, 2));
    assert_eq!((w_mip_s, h_mip_s), (3, 2));
    assert_eq!((w_min_u, h_min_u), (3, 2));
    assert_eq!((w_min_s, h_min_s), (3, 2));

    // Independent max(V, [], 3) -> [0.5 0.4 0.9; 0.8 0.5 0.6] -> round(v * 3) indices [2, 1, 3; 2, 2, 2]
    let expected_mip = vec![
        [0, 255, 0, 255],
        [255, 0, 0, 255],
        [255, 255, 255, 255],
        [0, 255, 0, 255],
        [0, 255, 0, 255],
        [0, 255, 0, 255],
    ];
    assert_eq!(mip_u, expected_mip);
    assert_eq!(mip_s, expected_mip);

    // Independent min(V, [], 3) -> [0.0 0.1 0.2; 0.3 0.2 0.1] -> round(v * 3) indices [0, 0, 1; 1, 1, 0]
    let expected_min = vec![
        [0, 0, 0, 255],
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [255, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 0, 0, 255],
    ];
    assert_eq!(min_u, expected_min);
    assert_eq!(min_s, expected_min);
}

#[test]
fn volume_show_mip_near_axis_oblique_crosses_both_rows() {
    // View [0 89.99999999999], spacing [1 1e-13 1], 2-by-1-by-3 rows [0; 1].
    // The center ray is oblique and crosses both row centers. The independent
    // maximum is 1, white 255 on a gray map. Treating the ray as axial samples
    // the row midpoint 0.5 and returns gray 128.
    let mut e = Engine::default();
    let render_center = |e: &mut Engine, style: &str| -> (usize, usize, Vec<[u8; 4]>) {
        e.execute(&format!(
            "V = reshape([0 1 0 1 0 1], 2, 1, 3); \
             volshow(V, RenderingStyle='{style}', DataLimits=[0 1], Colormap='gray', Transformation=[1 1e-13 1]); \
             view(0, 89.99999999999);"
        ))
        .unwrap();
        let fig = e.figures().next().unwrap().1;
        let vol = &fig.axes.extras.volumes[0];
        assert_eq!(fig.axes.view, [0.0, 89.99999999999]);
        assert_eq!(vol.transformation, [1.0, 1e-13, 1.0]);
        assert_eq!(vol.dims, [2, 1, 3]);
        render_volume(vol, fig.axes.view, Some((1, 1)))
    };
    let (w, h, mip) = render_center(&mut e, "MaximumIntensityProjection");
    assert_eq!((w, h), (1, 1));
    assert_eq!(mip, vec![[255, 255, 255, 255]]);
    let (_, _, minip) = render_center(&mut e, "MinimumIntensityProjection");
    assert_eq!(minip, vec![[0, 0, 0, 255]]);
}
