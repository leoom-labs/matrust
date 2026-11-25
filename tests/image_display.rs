//! image, imagesc, imshow and the axis modes they use (axis ij/xy, equal,
//! image, off). Expected behaviour from the MATLAB/Octave documentation:
//! image maps doubles onto colormap rows from 1 and integers from 0,
//! imagesc scales through CLim, imshow shows intensity images in gray over the
//! class range, and all three reverse the y direction on fresh axes.
use matrust::{Engine, plot::ImageMapping};

fn engine(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e
}
fn error(source: &str) -> String {
    Engine::default().execute(source).unwrap_err().to_string()
}
fn values(e: &mut Engine, expression: &str) -> Vec<f64> {
    e.execute(&format!("zz__ = double({expression});")).unwrap();
    e.variables["zz__"].data().to_vec()
}
fn text(e: &mut Engine, expression: &str) -> String {
    values(e, &format!("char({expression})"))
        .into_iter()
        .map(|c| char::from_u32(c as u32).unwrap())
        .collect()
}

#[test]
fn image_names_exist_as_builtins() {
    let mut e = Engine::default();
    for name in ["image", "imagesc", "imshow", "varargin"] {
        assert_eq!(values(&mut e, &format!("exist('{name}')")), [5.], "{name}");
    }
}

#[test]
fn imagesc_scales_through_data_range_and_reverses_y() {
    let e = engine("imagesc([1 2 3; 4 5 6]);");
    let axes = &e.figure.as_ref().unwrap().axes;
    assert_eq!(axes.extras.images.len(), 1);
    let image = &axes.extras.images[0];
    assert_eq!((image.rows, image.cols), (2, 3));
    assert_eq!(image.mapping, ImageMapping::Scaled);
    assert_eq!(axes.extras.clim, Some([1., 6.]));
    assert!(axes.extras.y_reversed);
    // Limits reach half a pixel beyond the first and last centres.
    assert_eq!(axes.bounds(), ([0.5, 3.5], [0.5, 2.5]));
    let mut e = engine("M = magic(3); imagesc(10:12, [5 7], M(1:2, :), [0 20]);");
    assert_eq!(values(&mut e, "clim"), [0., 20.]);
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!((image.x, image.y), ([10., 12.], [5., 7.]));
}

#[test]
fn image_direct_mapping_counts_doubles_from_one_and_integers_from_zero() {
    let map = [[0., 0., 0.], [0.5, 0.5, 0.5], [1., 1., 1.]];
    let e = engine("image([1 3]);");
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!(image.mapping, ImageMapping::Direct { zero_based: false });
    assert_eq!(image.rgb(&map, [0., 1.]), [0, 0, 0, 255, 255, 255]);
    let e = engine("image(uint8([0 2]));");
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!(image.rgb(&map, [0., 1.]), [0, 0, 0, 255, 255, 255]);
    // Out-of-range indices clamp to the first and last rows.
    let e = engine("image([-4 99]);");
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!(image.rgb(&map, [0., 1.]), [0, 0, 0, 255, 255, 255]);
    let mut e = engine("h = image(magic(4), 'CDataMapping', 'scaled');");
    assert_eq!(text(&mut e, "h.CDataMapping"), "scaled");
}

#[test]
fn truecolor_images_use_their_class_scale() {
    let e = engine("C = zeros(1, 2, 3); C(1, 1, 1) = 1; C(1, 2, 3) = 0.5; image(C);");
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!(image.rgb(&[], [0., 1.]), [255, 0, 0, 0, 0, 128]);
    let e = engine("C = uint8(zeros(1, 1, 3)); C(:) = [255 128 0]; imshow(C);");
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!(image.rgb(&[], [0., 1.]), [255, 128, 0]);
}

#[test]
fn imshow_uses_gray_over_the_class_range_and_hides_axes() {
    let e = engine("imshow(uint8([0 255; 128 64]));");
    let extras = &e.figure.as_ref().unwrap().axes.extras;
    assert_eq!(extras.clim, Some([0., 255.]));
    assert_eq!(extras.colormap.as_ref().unwrap().len(), 256);
    assert!(extras.equal_aspect && extras.axis_off && extras.y_reversed);
    let rgb = extras.images[0].rgb(&extras.effective_colormap(), extras.color_limits());
    assert_eq!(&rgb[..3], [0, 0, 0]);
    assert_eq!(&rgb[3..6], [255, 255, 255]);
    let mut e = engine("imshow([2 4; 6 8], []);");
    assert_eq!(values(&mut e, "clim"), [2., 8.]);
    let mut e = engine("imshow([2 4; 6 8], [0 10]);");
    assert_eq!(values(&mut e, "clim"), [0., 10.]);
    let mut e = engine("imshow(rand(3));");
    assert_eq!(values(&mut e, "clim"), [0., 1.]);
    let e = engine("imshow([1 2; 2 1], [1 0 0; 0 0 1]);");
    let extras = &e.figure.as_ref().unwrap().axes.extras;
    assert_eq!(
        **extras.colormap.as_ref().unwrap(),
        [[1., 0., 0.], [0., 0., 1.]]
    );
    assert_eq!(
        extras.images[0].rgb(&extras.effective_colormap(), extras.color_limits())[..6],
        [255, 0, 0, 0, 0, 255]
    );
}

#[test]
fn imshow_reads_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("imwrite(uint8([0 255; 255 0]), 'checker.png'); h = imshow('checker.png');")
        .unwrap();
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!((image.rows, image.cols), (2, 2));
    assert_eq!(values(&mut e, "h.CData"), [0., 255., 255., 0.]);
}

#[test]
fn image_handles_read_and_write_properties() {
    let mut e = engine("h = imagesc(magic(3));");
    assert_eq!(text(&mut e, "h.Type"), "image");
    assert_eq!(values(&mut e, "h.XData"), [1., 3.]);
    assert_eq!(values(&mut e, "size(h.CData)"), [3., 3.]);
    e.execute("h.CData = ones(2, 4); set(h, 'XData', [0 30]);")
        .unwrap();
    assert_eq!(values(&mut e, "size(h.CData)"), [2., 4.]);
    assert_eq!(values(&mut e, "get(h, 'XData')"), [0., 30.]);
    e.execute("set(h, 'CData', uint8(ones(2, 2, 3)));").unwrap();
    assert_eq!(values(&mut e, "size(h.CData)"), [2., 2., 3.]);
    e.execute("h.Visible = 'off';").unwrap();
    assert!(!e.figure.as_ref().unwrap().axes.extras.images[0].visible);
    assert!(error("h = image(1); set(h, 'AlphaData', 1);").contains("CData, XData"));
    assert!(error("h = image(1); h.AlphaData = 1;").contains("AlphaData"));
}

#[test]
fn hold_keeps_images_and_a_new_plot_replaces_them() {
    let e = engine("imagesc(magic(4)); hold on; plot([1 4], [1 4], 'r');");
    let axes = &e.figure.as_ref().unwrap().axes;
    assert_eq!((axes.extras.images.len(), axes.series.len()), (1, 1));
    assert!(axes.extras.y_reversed);
    let e = engine("imagesc(magic(4)); plot(1:3);");
    let axes = &e.figure.as_ref().unwrap().axes;
    assert!(axes.extras.images.is_empty() && !axes.extras.y_reversed);
    // Each subplot keeps its own image and colormap.
    let e = engine("subplot(1,2,1); imshow(rand(4)); subplot(1,2,2); imagesc(magic(4));");
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(
        figure
            .all_axes()
            .filter(|(_, a)| a.extras.images.len() == 1)
            .count(),
        2
    );
}

#[test]
fn axis_modes_set_and_clear_their_flags() {
    let mut e = engine("plot(1:3); axis equal; axis ij; axis off;");
    let extras = &e.figure.as_ref().unwrap().axes.extras;
    assert!(extras.equal_aspect && extras.y_reversed && extras.axis_off);
    e.execute("axis normal; axis xy; axis on;").unwrap();
    let extras = &e.figure.as_ref().unwrap().axes.extras;
    assert!(!extras.equal_aspect && !extras.y_reversed && !extras.axis_off);
    e.execute("axis([0 5 0 5]); axis image;").unwrap();
    let axes = &e.figure.as_ref().unwrap().axes;
    assert!(axes.extras.equal_aspect && axes.xlim.is_none());
    e.execute("axis('equal', 'tight')").unwrap();
    let e = engine("plot(1:3); axis square");
    assert!(e.figure.unwrap().axes.extras.square_plot_box);
}

#[cfg(feature = "plot-export")]
#[test]
fn exported_png_shows_row_one_at_the_top() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "imagesc([0 1; 1 0]); colormap(gray(2)); axis off; saveas(gcf, 'ij.png'); \
         axis xy; saveas(gcf, 'xy.png');",
    )
    .unwrap();
    let sample = |file: &str| {
        let png = image::open(dir.path().join(file)).unwrap().to_rgb8();
        // The picture is the only pure black or white area once axes are off.
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for (x, y, p) in png.enumerate_pixels() {
            if p.0 == [0; 3] || p.0 == [255; 3] {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        assert!(x1 > x0 + 20 && y1 > y0 + 20, "{file}: no image area");
        let at = |fx: f64, fy: f64| {
            let x = x0 + ((x1 - x0) as f64 * fx) as u32;
            let y = y0 + ((y1 - y0) as f64 * fy) as u32;
            png.get_pixel(x, y).0
        };
        [at(0.25, 0.25), at(0.75, 0.25), at(0.25, 0.75)]
    };
    assert_eq!(sample("ij.png"), [[0; 3], [255; 3], [255; 3]]);
    assert_eq!(sample("xy.png"), [[255; 3], [0; 3], [0; 3]]);
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("data:image/png;base64,iVBORw0KGgo"));
}

#[test]
fn bad_image_arguments_report_their_form() {
    assert!(error("image()").contains("Use image(C)"));
    assert!(error("imagesc(1, 2, 3, 4, 5)").contains("Use imagesc(C)"));
    assert!(error("imshow(rand(3), [1 0])").contains("display range"));
    assert!(error("image(1, 'Foo', 2)").contains("unsupported property 'foo'"));
    assert!(error("image(ones(2, 2, 2))").contains("M-by-N-by-3"));
}

#[test]
fn review_fixes_scaled_limits_direct_mapping_and_property_snapshots() {
    // image(..., 'CDataMapping', 'scaled') takes automatic limits from the data.
    let mut e = engine("image([10 20], 'CDataMapping', 'scaled'); colormap(gray(2));");
    assert_eq!(values(&mut e, "clim"), [10., 20.]);
    // Setting 'direct' again keeps integer CData zero-based.
    let e = engine("h = image(uint8([0 1])); set(h, 'CDataMapping', 'direct');");
    let image = &e.figure.as_ref().unwrap().axes.extras.images[0];
    assert_eq!(image.mapping, ImageMapping::Direct { zero_based: true });
    // An XData edit changes the axes snapshot, so the desktop redraws it.
    let mut e = engine("h = image(ones(2));");
    let before = e.figure.as_ref().unwrap().axes.extras.clone();
    e.execute("set(h, 'XData', [100 101]);").unwrap();
    assert!(before != e.figure.as_ref().unwrap().axes.extras);
}

#[test]
fn review_1111_imshow_int64_uint64_class_range() {
    // imshow with no DisplayRange uses [intmin intmax] of the class.
    // As doubles, intmax('uint64') is 2^64 and intmax('int64') is 2^63;
    // intmin('int64') is -2^63. A ramp of 0..1000 stays black on that scale.
    // https://www.mathworks.com/help/matlab/ref/imshow.html
    // https://www.mathworks.com/help/images/ref/getrangefromclass.html
    let e = engine("imshow(uint64(0:1000));");
    let extras = &e.figure.as_ref().unwrap().axes.extras;
    assert_eq!(extras.clim, Some([0., 18_446_744_073_709_551_616.]));
    let rgb = extras.images[0].rgb(&extras.effective_colormap(), extras.color_limits());
    assert_eq!(&rgb[..3], &[0, 0, 0]);
    assert_eq!(&rgb[rgb.len() - 3..], &[0, 0, 0]);
    let e = engine("imshow(int64([0 1000]));");
    let extras = &e.figure.as_ref().unwrap().axes.extras;
    assert_eq!(
        extras.clim,
        Some([-9_223_372_036_854_775_808., 9_223_372_036_854_775_808.])
    );
    let rgb = extras.images[0].rgb(&extras.effective_colormap(), extras.color_limits());
    assert_ne!(&rgb[3..6], &[255, 255, 255]);
}

#[cfg(feature = "plot-export")]
fn render_direct_map_pixels(code: &str, file: &str) -> [[u8; 3]; 2] {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(&format!(
        "{code}; colormap([1 0 0; 0 1 0]); axis off; saveas(gcf, '{file}');"
    ))
    .unwrap();
    let png = image::open(dir.path().join(file)).unwrap().to_rgb8();
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in png.enumerate_pixels() {
        if p.0 == [255, 0, 0] || p.0 == [0, 255, 0] {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    assert!(x1 > x0 + 20 && y1 > y0 + 20, "{file}: no image area");
    let y = (y0 + y1) / 2;
    let left = png.get_pixel(x0 + (x1 - x0) / 4, y).0;
    let right = png.get_pixel(x0 + (x1 - x0) * 3 / 4, y).0;
    [left, right]
}

#[cfg(feature = "plot-export")]
#[test]
fn fix15_image_direct_mapping_renders_expected_pixels() {
    assert_eq!(
        render_direct_map_pixels("image(uint8([0 1]))", "uint8.png"),
        [[255, 0, 0], [0, 255, 0]]
    );
    assert_eq!(
        render_direct_map_pixels("image([1 2])", "double.png"),
        [[255, 0, 0], [0, 255, 0]]
    );
    assert_eq!(
        render_direct_map_pixels(
            "set(groot, 'DefaultImageCDataMapping', 'direct'); image(uint8([0 1]))",
            "default_direct.png"
        ),
        [[255, 0, 0], [0, 255, 0]]
    );
}

#[cfg(feature = "plot-export")]
#[test]
fn fix15_image_switch_to_direct_uses_cdata_class() {
    assert_eq!(
        render_direct_map_pixels(
            "h = imagesc(uint8([0 1])); set(h, 'CDataMapping', 'direct')",
            "uint8_scaled_to_direct.png",
        ),
        [[255, 0, 0], [0, 255, 0]]
    );
    assert_eq!(
        render_direct_map_pixels(
            "h = imagesc(uint8([0 1])); set(h, 'CData', single([1 2])); set(h, 'CDataMapping', 'direct')",
            "single_scaled_to_direct.png",
        ),
        [[255, 0, 0], [0, 255, 0]]
    );
}

#[test]
fn fix15_direct_mapping_ind2rgb_imshow_and_clamping_agree() {
    let mut e = engine(
        "map = [1 0 0; 0 1 0; 0 0 1];
         rgb_u8 = ind2rgb(uint8([0 1 2]), map);
         imshow(uint8([0 1 2]), map);",
    );
    let extras = &e.figure.as_ref().unwrap().axes.extras;
    let shown = extras.images[0].rgb(&extras.effective_colormap(), extras.color_limits());
    assert_eq!(shown, [255, 0, 0, 0, 255, 0, 0, 0, 255]);
    // ind2rgb returns HxWx3 double in [0, 1]; column 1 is red, column 2 is green, column 3 is blue.
    assert_eq!(
        values(&mut e, "reshape(round(rgb_u8 * 255), 1, [])"),
        [255., 0., 0., 0., 255., 0., 0., 0., 255.]
    );
    // uint16, int8, single and out-of-range clamping.
    for (expr, expected) in [
        (
            "image(uint16([0 1 99]));",
            &[255, 0, 0, 0, 255, 0, 0, 0, 255][..],
        ),
        (
            "image(int8([-5 0 1 99]));",
            &[255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255][..],
        ),
        (
            "image(single([-2 1 2 99]));",
            &[255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255][..],
        ),
        (
            "image([-4 0 1 2 99]);",
            &[255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255][..],
        ),
    ] {
        let mut e = engine(expr);
        e.execute("colormap([1 0 0; 0 1 0; 0 0 1]);").unwrap();
        let extras = &e.figure.as_ref().unwrap().axes.extras;
        let rgb = extras.images[0].rgb(&extras.effective_colormap(), extras.color_limits());
        assert_eq!(rgb.as_slice(), expected, "{expr}");
    }
    // ind2rgb out-of-range clamping on uint8 and double.
    let mut e = engine(
        "map = [1 0 0; 0 1 0; 0 0 1];
         a = ind2rgb(uint8([0 99]), map);
         b = ind2rgb([-4 99], map);",
    );
    assert_eq!(values(&mut e, "a(:)"), [1., 0., 0., 0., 0., 1.]);
    assert_eq!(values(&mut e, "b(:)"), [1., 0., 0., 0., 0., 1.]);
}

#[test]
fn fix15c_image_cdata_round_trip_keeps_class_and_direct_pixels() {
    let mut e = Engine::default();
    e.execute(
        "colormap([1 0 0; 0 1 0]);
         h = image(uint8([0 1]));
         assert(strcmp(class(h.CData), 'uint8'));
         assert(strcmp(class(get(h, 'CData')), 'uint8'));
         h.CData = h.CData;
         assert(strcmp(class(h.CData), 'uint8'));
         assert(isequal(h.CData, uint8([0 1])));
         set(h, 'CData', get(h, 'CData'));
         assert(strcmp(class(get(h, 'CData')), 'uint8'));",
    )
    .unwrap();
    let img = &e.figure.as_ref().unwrap().axes.extras.images[0];
    let map = e.figure.as_ref().unwrap().axes.extras.effective_colormap();
    assert_eq!(img.mapping, ImageMapping::Direct { zero_based: true });
    assert_eq!(img.rgb(&map, [0., 1.]), [255, 0, 0, 0, 255, 0]);
}
