use matrust::plot::{
    DEFAULT_ISOSURFACE_COLOR, RenderingStyle, colormap, render_volume, render_volume_with_step,
};
use matrust::{Array, Engine};

fn char_text(arr: &Array) -> String {
    match arr {
        Array::Character(c) => c.to_string(),
        other => panic!("expected Character array, got {other:?}"),
    }
}

#[test]
fn volume_styles_isosurface_sphere_disc_area_and_headlight() {
    let mut e = Engine::default();
    e.execute(
        "[x, y, z] = meshgrid(-16:16, -16:16, -16:16); \
         V = sqrt(x.^2 + y.^2 + z.^2); \
         v = volshow(V, RenderingStyle='Isosurface', IsosurfaceValue=10); \
         view(0, 90);",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    let vol = &fig.axes.extras.volumes[0];
    assert_eq!(vol.rendering_style, RenderingStyle::Isosurface);
    assert_eq!(vol.isosurface_color, DEFAULT_ISOSURFACE_COLOR);

    let (w, h, pixels) = render_volume(vol, fig.axes.view, None);
    assert_eq!((w, h), (33, 33));

    let hits = pixels.iter().filter(|px| px[3] > 0).count();
    let expected_area = std::f64::consts::PI * 100.0;
    let rel_err = (hits as f64 - expected_area).abs() / expected_area;
    assert!(
        rel_err <= 0.03,
        "expected hit area within 3% of pi*10^2 ({expected_area:.2}), got {hits} (rel_err={rel_err:.4})"
    );

    // Non-hit background pixels must be transparent.
    assert_eq!(pixels[0], [0, 0, 0, 0]);

    // Centre pixel (16, 16) has normal facing the camera and must be brightest.
    let centre = pixels[16 * 33 + 16];
    assert_eq!(centre[3], 255);
    let centre_sum: u16 = centre[0] as u16 + centre[1] as u16 + centre[2] as u16;
    assert!(centre_sum > 0);
    for (idx, px) in pixels.iter().enumerate() {
        if px[3] > 0 {
            let sum = px[0] as u16 + px[1] as u16 + px[2] as u16;
            assert!(
                centre_sum >= sum,
                "pixel {idx} ({px:?}) brighter than centre ({centre:?})"
            );
        }
    }
    let edge = pixels[16 * 33 + 25];
    assert!(edge[3] == 255);
    assert!(
        centre[0] > edge[0],
        "expected centre ({centre:?}) strictly brighter than off-centre ({edge:?})"
    );
}

#[test]
fn volume_styles_slice_planes_z8_exact_and_oblique() {
    let mut e = Engine::default();
    e.execute(
        "V = zeros(12, 14, 10);
         for k = 1:10
             for j = 1:14
                 for i = 1:12
                     V(i, j, k) = i + 10*j + 100*k;
                 end
             end
         end
         v = volshow(V, RenderingStyle='SlicePlanes', SlicePlaneValues=[0 0 1 -8], Colormap='gray');
         view(0, 90);",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    let vol = &fig.axes.extras.volumes[0];
    let (w, h, pixels) = render_volume(vol, fig.axes.view, None);
    assert_eq!((w, h), (14, 12));

    let gray = colormap::named("gray", 256).unwrap().unwrap();
    let [dmin, dmax] = vol.data_limits;
    for row in 0..12 {
        for col in 0..14 {
            let val = (row + 1) as f64 + 10.0 * (col + 1) as f64 + 100.0 * 8.0;
            let norm = ((val - dmin) / (dmax - dmin)).clamp(0.0, 1.0);
            let idx = (norm * 255.0).round() as usize;
            let expected_rgb = gray[idx].map(|c| (c * 255.0).round() as u8);
            assert_eq!(
                pixels[row * 14 + col],
                [expected_rgb[0], expected_rgb[1], expected_rgb[2], 255],
                "mismatch at row={row}, col={col}"
            );
        }
    }

    // Oblique plane x + y + z - 18 = 0 must produce opaque hits inside the volume.
    e.execute("v.SlicePlaneValues = [1 1 1 -18]; view(-37.5, 30);")
        .unwrap();
    let fig = e.figures().next().unwrap().1;
    let vol = &fig.axes.extras.volumes[0];
    let (_, _, oblique_pixels) = render_volume(vol, fig.axes.view, Some((32, 32)));
    let opaque = oblique_pixels.iter().filter(|px| px[3] == 255).count();
    let empty = oblique_pixels.iter().filter(|px| px[3] == 0).count();
    assert!(opaque > 20, "expected oblique plane hits, got {opaque}");
    assert!(
        empty > 20,
        "expected transparent rays outside oblique plane, got {empty}"
    );
}

#[test]
fn volume_styles_volume_rendering_step_halving() {
    let mut e = Engine::default();
    e.execute(
        "[x, y, z] = meshgrid(1:16, 1:16, 1:16); \
         V = 0.5 + 0.25*sin(x/3) + 0.25*cos(y/4) + 0.5*(z/16); \
         v = volshow(V, RenderingStyle='VolumeRendering', Colormap='parula'); \
         v.Alphamap = linspace(0.02, 0.25, 64); \
         view(-37.5, 30);",
    )
    .unwrap();
    let fig = e.figures().next().unwrap().1;
    let vol = &fig.axes.extras.volumes[0];
    let (w1, h1, p1) = render_volume_with_step(vol, fig.axes.view, Some((24, 24)), 0.5);
    let (w2, h2, p2) = render_volume_with_step(vol, fig.axes.view, Some((24, 24)), 0.25);
    assert_eq!((w1, h1), (w2, h2));
    for (i, (a, b)) in p1.iter().zip(p2.iter()).enumerate() {
        for ch in 0..4 {
            let diff = a[ch].abs_diff(b[ch]);
            assert!(
                diff <= 2,
                "pixel {i} channel {ch} differed by {diff} (> 2/255): {a:?} vs {b:?}"
            );
        }
    }
}

#[test]
fn volume_styles_properties_and_exportgraphics_all_styles() {
    let mut e = Engine::default();
    e.execute(
        "V = reshape(1:27, 3, 3, 3); \
         v = volshow(V); \
         ic0 = v.IsosurfaceColor; \
         sp0 = v.SlicePlaneValues; \
         v.RenderingStyle = 'Isosurface'; \
         rs = v.RenderingStyle; \
         v.IsosurfaceValue = 14; \
         iv = v.IsosurfaceValue; \
         v.IsosurfaceColor = [0.2 0.6 0.9]; \
         ic1 = v.IsosurfaceColor; \
         v.IsosurfaceColor = '#ff8000'; \
         ic2 = v.IsosurfaceColor; \
         v.SlicePlaneValues = [1 0 0 -2; 0 0 1 -2]; \
         sp1 = v.SlicePlaneValues;",
    )
    .unwrap();
    let ws = e.workspace();
    assert_eq!(char_text(ws.get("rs").unwrap()), "'Isosurface'");
    let Array::Numeric(ic0) = ws.get("ic0").unwrap() else {
        panic!("expected default IsosurfaceColor");
    };
    assert_eq!(ic0.data(), &DEFAULT_ISOSURFACE_COLOR);
    let Array::Numeric(sp0) = ws.get("sp0").unwrap() else {
        panic!("expected default SlicePlaneValues");
    };
    assert_eq!((sp0.rows(), sp0.cols()), (3, 4));
    assert_eq!(sp0.get(0, 3), -2.0);
    assert_eq!(sp0.get(1, 3), -2.0);
    assert_eq!(sp0.get(2, 3), -2.0);

    let Array::Numeric(iv) = ws.get("iv").unwrap() else {
        panic!("expected IsosurfaceValue");
    };
    assert_eq!(iv.as_scalar().unwrap(), 14.0);
    let Array::Numeric(ic1) = ws.get("ic1").unwrap() else {
        panic!("expected IsosurfaceColor");
    };
    assert_eq!(ic1.data(), &[0.2, 0.6, 0.9]);
    let Array::Numeric(ic2) = ws.get("ic2").unwrap() else {
        panic!("expected hex IsosurfaceColor");
    };
    assert!((ic2.data()[0] - 1.0).abs() < 1e-12);
    assert!((ic2.data()[1] - 128.0 / 255.0).abs() < 1e-12);
    assert_eq!(ic2.data()[2], 0.0);
    let Array::Numeric(sp1) = ws.get("sp1").unwrap() else {
        panic!("expected SlicePlaneValues");
    };
    assert_eq!((sp1.rows(), sp1.cols()), (2, 4));

    let dir = std::env::temp_dir().join(format!("matrust_volume_styles_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::default();
    e.set_file_directory(&dir).unwrap();
    for (idx, style_cmd) in [
        "volshow(V, RenderingStyle='Isosurface', IsosurfaceValue=10, IsosurfaceColor=[0.2 0.5 0.9]);",
        "volshow(V, RenderingStyle='SlicePlanes', Colormap='hot');",
        "volshow(V, RenderingStyle='VolumeRendering', Colormap='parula');",
    ]
    .iter()
    .enumerate()
    {
        let fname = format!("style_{idx}.png");
        let script = format!(
            "close all; \
             [x, y, z] = meshgrid(-8:8, -8:8, -8:8); \
             V = 12 - sqrt(x.^2 + y.^2 + z.^2); \
             {style_cmd} \
             exportgraphics(gcf, '{fname}');"
        );
        e.execute(&script).unwrap();
        let bytes = std::fs::read(dir.join(&fname)).unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        let (w, h) = img.dimensions();
        let mut non_white = 0usize;
        for py in (h / 4)..(3 * h / 4) {
            for px in (w / 4)..(3 * w / 4) {
                let p = img.get_pixel(px, py).0;
                if p[0] != 255 || p[1] != 255 || p[2] != 255 {
                    non_white += 1;
                }
            }
        }
        assert!(
            non_white > 0,
            "expected non-background centre pixels for style {idx}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
