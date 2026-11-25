use matrust::plot::colormap;
use matrust::{Array, Engine};

fn char_text(arr: &Array) -> String {
    match arr {
        Array::Character(c) => c.to_string(),
        other => panic!("expected Character array, got {other:?}"),
    }
}

fn build_v_script(ny: usize, nx: usize, nz: usize) -> String {
    format!(
        "V = zeros({ny}, {nx}, {nz});
         for k = 1:{nz}
             for j = 1:{nx}
                 for i = 1:{ny}
                     V(i, j, k) = i + 10*j + 100*k;
                 end
             end
         end"
    )
}

fn non_white_bbox(img: &image::RgbImage) -> (u32, u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0 != [255, 255, 255] {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    assert!(x1 > x0 && y1 > y0, "expected non-white image content");
    (x0, y0, x1 + 1, y1 + 1)
}

#[test]
fn volume_sliceviewer_default_slice_and_exported_pixels_match_colormap() {
    let (ny, nx, nz) = (12usize, 9usize, 7usize);
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(&format!(
        "{}
         s = sliceViewer(V);
         def_slice = s.SliceNumber;
         def_dir = s.SliceDirection;
         def_range = s.DisplayRange;
         def_scale = s.ScaleFactors;
         def_type = get(s, 'Type');
         ax = getAxesHandles(s);
         s.SliceNumber = 3;
         exportgraphics(gcf, 'slice_z3.png');",
        build_v_script(ny, nx, nz)
    ))
    .unwrap();

    // Default SliceNumber is round(nz/2) = 4, SliceDirection = [0 0 1].
    assert_eq!(e.variables["def_slice"].data(), &[4.0]);
    assert_eq!(e.variables["def_dir"].data(), &[0.0, 0.0, 1.0]);
    assert_eq!(e.variables["def_range"].data(), &[111.0, 790.0 + 12.0]);
    assert_eq!(e.variables["def_scale"].data(), &[1.0, 1.0, 1.0]);
    let ws = e.workspace();
    assert_eq!(char_text(ws.get("def_type").unwrap()), "'sliceviewer'");
    assert!(matches!(ws.get("ax"), Some(Array::Graphics(_))));

    let png = image::open(dir.path().join("slice_z3.png"))
        .unwrap()
        .to_rgb8();
    let (x0, y0, x1, y1) = non_white_bbox(&png);
    let bw = x1 - x0;
    let bh = y1 - y0;
    let gray = colormap::named("gray", 256).unwrap().unwrap();
    let dr = [111.0, 802.0];

    for row in 0..ny {
        for col in 0..nx {
            let px = x0 + (col as u32 * bw + bw / 2) / nx as u32;
            let py = y0 + (row as u32 * bh + bh / 2) / ny as u32;
            let actual = png.get_pixel(px, py).0;
            let val = (row + 1) as f64 + 10.0 * (col + 1) as f64 + 300.0;
            let expected = colormap::scaled_rgb(&gray, dr, val);
            assert_eq!(
                actual, expected,
                "pixel mismatch at row={row}, col={col} (px={px}, py={py})"
            );
        }
    }
}

#[test]
fn volume_sliceviewer_directions_x_and_y_orientation() {
    let (ny, nx, nz) = (12usize, 9usize, 7usize);
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(&format!(
        "{}
         s = sliceViewer(V, 'SliceDirection', 'X', 'SliceNumber', 4);
         dir_x = s.SliceDirection;
         exportgraphics(gcf, 'slice_x4.png');
         set(s, 'SliceDirection', 'Y');
         def_y = get(s, 'SliceNumber');
         s.SliceNumber = 5;
         exportgraphics(gcf, 'slice_y5.png');",
        build_v_script(ny, nx, nz)
    ))
    .unwrap();

    assert_eq!(e.variables["dir_x"].data(), &[1.0, 0.0, 0.0]);
    assert_eq!(e.variables["def_y"].data(), &[6.0]); // round(12/2) = 6

    let gray = colormap::named("gray", 256).unwrap().unwrap();
    let dr = [111.0, 802.0];

    // SliceDirection 'X' (k=4): squeeze(V(:,4,:)) -> rows=ny(12), cols=nz(7)
    let png_x = image::open(dir.path().join("slice_x4.png"))
        .unwrap()
        .to_rgb8();
    let (x0, y0, x1, y1) = non_white_bbox(&png_x);
    let (bw, bh) = (x1 - x0, y1 - y0);
    for row in 0..ny {
        for col in 0..nz {
            let px = x0 + (col as u32 * bw + bw / 2) / nz as u32;
            let py = y0 + (row as u32 * bh + bh / 2) / ny as u32;
            let val = (row + 1) as f64 + 10.0 * 4.0 + 100.0 * (col + 1) as f64;
            assert_eq!(
                png_x.get_pixel(px, py).0,
                colormap::scaled_rgb(&gray, dr, val),
                "X slice mismatch at row={row}, col={col}"
            );
        }
    }

    // SliceDirection 'Y' (k=5): squeeze(V(5,:,:))' -> rows=nz(7), cols=nx(9)
    let png_y = image::open(dir.path().join("slice_y5.png"))
        .unwrap()
        .to_rgb8();
    let (x0, y0, x1, y1) = non_white_bbox(&png_y);
    let (bw, bh) = (x1 - x0, y1 - y0);
    for row in 0..nz {
        for col in 0..nx {
            let px = x0 + (col as u32 * bw + bw / 2) / nx as u32;
            let py = y0 + (row as u32 * bh + bh / 2) / nz as u32;
            let val = 5.0 + 10.0 * (col + 1) as f64 + 100.0 * (row + 1) as f64;
            assert_eq!(
                png_y.get_pixel(px, py).0,
                colormap::scaled_rgb(&gray, dr, val),
                "Y slice mismatch at row={row}, col={col}"
            );
        }
    }
}

#[test]
fn volume_sliceviewer_errors_and_properties() {
    let mut e = Engine::default();
    e.execute("V = reshape(1:24, 2, 3, 4);").unwrap();

    // Out-of-range SliceNumber
    let err = e
        .execute("sliceViewer(V, 'SliceNumber', 0);")
        .unwrap_err()
        .to_string();
    assert!(err.contains("SliceNumber"), "{err}");

    let err = e
        .execute("sliceViewer(V, 'SliceNumber', 5);")
        .unwrap_err()
        .to_string();
    assert!(err.contains("SliceNumber"), "{err}");

    // Non-integer SliceNumber
    let err = e
        .execute("sliceViewer(V, 'SliceNumber', 2.5);")
        .unwrap_err()
        .to_string();
    assert!(err.contains("SliceNumber"), "{err}");

    // 4-D non-RGB input
    let err = e
        .execute("sliceViewer(zeros(2, 3, 4, 2));")
        .unwrap_err()
        .to_string();
    assert!(err.contains("2-D, 3-D, or 4-D RGB"), "{err}");

    // 4-D RGB input -> not supported yet
    let err = e
        .execute("sliceViewer(zeros(2, 3, 4, 3));")
        .unwrap_err()
        .to_string();
    assert!(err.contains("not supported yet"), "{err}");

    // 2-D input, Parent, Tag, Visible, DisplayRange=[], ScaleFactors, Colormap
    e.execute(
        "f = figure;
         s = sliceViewer([1 2; 3 4], 'Parent', f, 'Tag', 'sv1', 'Visible', 'off', 'ScaleFactors', [1 2 1], 'Colormap', 'hot');
         sn = s.SliceNumber;
         tg = s.Tag;
         vis = s.Visible;
         s.DisplayRange = [];
         dr = s.DisplayRange;",
    )
    .unwrap();
    assert_eq!(e.variables["sn"].data(), &[1.0]);
    let ws = e.workspace();
    assert_eq!(char_text(ws.get("tg").unwrap()), "'sv1'");
    assert_eq!(char_text(ws.get("vis").unwrap()), "'off'");
    assert_eq!(e.variables["dr"].data(), &[1.0, 4.0]);
}

#[test]
fn volume_orthosliceviewer_panes_and_crosshair() {
    let (ny, nx, nz) = (12usize, 9usize, 7usize);
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(&format!(
        "{}
         o = orthosliceViewer(V);
         def_sn = o.SliceNumbers;
         def_ch = o.CrosshairEnabled;
         o.SliceNumbers = [2 3 4];
         sn2 = get(o, 'SliceNumbers');
         [hXY, hYZ, hXZ] = getAxesHandles(o);
         exportgraphics(gcf, 'ortho_on.png');
         o.CrosshairEnabled = 'off';
         exportgraphics(gcf, 'ortho_off.png');",
        build_v_script(ny, nx, nz)
    ))
    .unwrap();

    assert_eq!(e.variables["def_sn"].data(), &[5.0, 6.0, 4.0]);
    assert_eq!(e.variables["sn2"].data(), &[2.0, 3.0, 4.0]);
    let ws = e.workspace();
    assert_eq!(char_text(ws.get("def_ch").unwrap()), "'on'");
    assert!(matches!(ws.get("hXY"), Some(Array::Graphics(_))));
    assert!(matches!(ws.get("hYZ"), Some(Array::Graphics(_))));
    assert!(matches!(ws.get("hXZ"), Some(Array::Graphics(_))));

    let png_on = image::open(dir.path().join("ortho_on.png"))
        .unwrap()
        .to_rgb8();
    let png_off = image::open(dir.path().join("ortho_off.png"))
        .unwrap()
        .to_rgb8();

    let (w, h) = (png_on.width(), png_on.height());
    let has_non_white = |img: &image::RgbImage, x0: u32, y0: u32, x1: u32, y1: u32| -> bool {
        (y0..y1).any(|y| (x0..x1).any(|x| img.get_pixel(x, y).0 != [255, 255, 255]))
    };
    // All three panes (top-left XY, top-right YZ, bottom-left XZ) have non-background pixels.
    assert!(has_non_white(&png_on, 0, 0, w / 2, h / 2), "XY pane empty");
    assert!(has_non_white(&png_on, w / 2, 0, w, h / 2), "YZ pane empty");
    assert!(has_non_white(&png_on, 0, h / 2, w / 2, h), "XZ pane empty");

    // Crosshair color [0, 210, 255] is present when 'on' and absent when 'off'.
    let has_crosshair = |img: &image::RgbImage| -> bool {
        img.pixels()
            .any(|p| p.0[2] > 200 && p.0[1] > 150 && p.0[0] < 60)
    };
    assert!(
        has_crosshair(&png_on),
        "expected crosshair pixels when CrosshairEnabled='on'"
    );
    assert!(
        !has_crosshair(&png_off),
        "expected no crosshair pixels when CrosshairEnabled='off'"
    );
}

fn pane_rgb(axes: &matrust::plot::Axes) -> Vec<u8> {
    let extras = &axes.extras;
    extras.images[0].rgb(&extras.effective_colormap(), extras.color_limits())
}

#[test]
fn held_children_survive_tag_and_slice_with_updated_pixels() {
    let mut e = Engine::default();
    e.execute(
        "s=sliceViewer(reshape(1:8,2,2,2)); a=getAxesHandles(s);
         hold(a,'on'); h=plot(a,1:2); im=image(a,[7 8;9 10]);
         set(a,'XLim',[-2 4],'YLim',[-3 5]); view(a,20,60);
         children=a.Children; assert(isgraphics(h));",
    )
    .unwrap();
    let expected = |values: [u8; 4]| values.into_iter().flat_map(|v| [v; 3]).collect::<Vec<_>>();
    assert_eq!(
        pane_rgb(&e.figure.as_ref().unwrap().axes),
        expected([0, 73, 36, 109])
    );
    let original_pixels = e.figure.as_ref().unwrap().axes.extras.images[0]
        .data
        .clone();
    for change in ["s.Tag='abc';", "s.SliceNumber=2;"] {
        e.execute(change).unwrap();
        e.execute(
            "assert(isgraphics(h)); assert(isequal(h.YData,[1 2]));
             assert(isequal(h.XData,[1 2])); assert(isequal(a.Children,children));
             assert(isequal(im.CData,[7 8;9 10])); assert(isequal(getAxesHandles(s),a));
             assert(isequal(a.View,[20 60])); assert(isequal(a.XLim,[-2 4]));
             assert(isequal(a.YLim,[-3 5]));",
        )
        .unwrap();
        if change.starts_with("s.Tag") {
            assert!(std::sync::Arc::ptr_eq(
                &original_pixels,
                &e.figure.as_ref().unwrap().axes.extras.images[0].data,
            ));
        }
    }
    assert_eq!(
        pane_rgb(&e.figure.as_ref().unwrap().axes),
        expected([146, 219, 182, 255])
    );
    e.execute("set(s,'DisplayRange',[5 8],'Colormap',[0 0 1;1 0 0]);")
        .unwrap();
    assert_eq!(
        pane_rgb(&e.figure.as_ref().unwrap().axes),
        [0, 0, 255, 255, 0, 0, 0, 0, 255, 255, 0, 0]
    );
    e.execute("s.DisplayRange=[]; assert(isequal(s.DisplayRange,[1 8]));")
        .unwrap();
    assert_eq!(
        pane_rgb(&e.figure.as_ref().unwrap().axes),
        [255, 0, 0].repeat(4)
    );
}

#[test]
fn ortho_refresh_keeps_held_children_and_moves_crosshairs() {
    let mut e = Engine::default();
    e.execute(
        "o=orthosliceViewer(reshape(1:8,2,2,2)); [xy,yz,xz]=getAxesHandles(o);
         cross=yz.Children; horizontal=cross(1); vertical=cross(2);
         hold(yz,'on'); h=plot(yz,[4 3]); im=image(yz,[10 9;8 7]); children=yz.Children;
         set(yz,'CLim',[0 8],'XLim',[-1 6],'YLim',[-2 8]); view(yz,30,70);
         o.Tag='yz'; set(o,'SliceNumbers',[2 2 2]);
         assert(isequal(yz.Children,children)); assert(isequal(h.YData,[4 3]));
         assert(isequal(im.CData,[10 9;8 7])); assert(isequal(yz.CLim,[0 8]));
         assert(isequal(yz.View,[30 70])); assert(isequal(yz.XLim,[-1 6]));
         assert(isequal(yz.YLim,[-2 8])); assert(isequal(vertical.XData,[2 2]));
         assert(isequal(horizontal.YData,[2 2])); o.CrosshairEnabled='off';
         assert(strcmp(vertical.Visible,'off')); assert(strcmp(h.Visible,'on'));
         o.CrosshairEnabled='on'; assert(strcmp(horizontal.Visible,'on'));
         assert(isequal(yz.Children,children));",
    )
    .unwrap();
    let f = e.figure.as_ref().unwrap();
    let yz_id = &f.orthoslice_viewers[0].axes_identities[1];
    let (_, yz) = f
        .all_axes()
        .find(|(_, a)| a.identity.same_object(yz_id))
        .unwrap();
    // Rows of squeeze(V(:,2,:)) are [3 7] and [4 8], scaled through [0 8].
    assert_eq!(
        pane_rgb(yz),
        [96, 96, 96, 224, 224, 224, 128, 128, 128, 255, 255, 255]
    );
}

#[test]
fn constant_viewers_render_with_finite_limits_and_validate_explicit_ranges() {
    for (constructor, data, constant) in [
        ("sliceViewer", "ones(2,2,2)", 1),
        ("orthosliceViewer", "false(2,2,2)", 0),
    ] {
        let mut e = Engine::default();
        e.execute(&format!(
            "s={constructor}({data}); a=getAxesHandles(s); assert(isgraphics(s));
             assert(all(isgraphics(a))); assert(isequal(s.DisplayRange,[{constant} {constant}]));"
        ))
        .unwrap();
        for (_, ax) in e.figure.as_ref().unwrap().all_axes() {
            let [lo, hi] = ax.extras.color_limits();
            assert!(lo.is_finite() && hi.is_finite() && lo < hi);
            assert_eq!(pane_rgb(ax), [128; 12]);
        }
        for invalid in ["[2 2]", "[3 2]", "[0 Inf]", "[NaN 1]"] {
            let error = e
                .execute(&format!("set(s,'Tag','invalid','DisplayRange',{invalid});"))
                .unwrap_err();
            assert!(error.contains("DisplayRange"), "{error}");
            e.execute("assert(isempty(s.Tag)); assert(all(isgraphics(a)));")
                .unwrap();
        }
        e.execute("s.DisplayRange=[0 2]; assert(isequal(s.DisplayRange,[0 2]));")
            .unwrap();
        for (_, ax) in e.figure.as_ref().unwrap().all_axes() {
            assert_eq!(pane_rgb(ax), [128 * constant as u8; 12]);
        }
        e.execute("s.DisplayRange=[];").unwrap();
        for (_, ax) in e.figure.as_ref().unwrap().all_axes() {
            assert_eq!(pane_rgb(ax), [128; 12]);
        }
    }
}

#[test]
fn deleting_single_pane_retires_viewer_and_stale_handles_are_atomic() {
    let mut e = Engine::default();
    e.execute(
        "f=figure; s=sliceViewer(reshape(1:8,2,2,2)); ax=getAxesHandles(s);
               g=figure; other=gca; h=plot(other,[9 7]); delete(ax);
               assert(~isgraphics(s)); assert(~isgraphics(ax)); assert(isgraphics(f));",
    )
    .unwrap();
    let unaffected = e.figure.clone().unwrap();
    for op in [
        "getAxesHandles(s);",
        "s.SliceNumber=2;",
        "s.Parent=g;",
        "delete([other ax]);",
    ] {
        assert!(e.execute(op).is_err(), "{op}");
        assert!(unaffected.same_snapshot(e.figure.as_ref().unwrap()));
    }
    e.execute("assert(isgraphics(other)); assert(isequal(h.YData,[9 7])); assert(isequal(gcf,g));")
        .unwrap();
}

#[test]
fn unrelated_tile_plot_keeps_ortho_viewer_and_line_through_slice_change() {
    for mode in ["off", "on"] {
        Engine::default().execute(&format!(
            "o=orthosliceViewer(reshape(1:8,2,2,2),'SliceNumbers',[1 1 1]); panes=getAxesHandles(o);
             assert(isgraphics(o)); nexttile(4); t=gca; hold(t,'{mode}'); h=plot(t,[6 5]);
             assert(isgraphics(o)); assert(all(isgraphics(panes))); assert(isgraphics(h));
             assert(isequal(panes,getAxesHandles(o))); o.SliceNumbers=[2 2 2];
             assert(isequal(o.SliceNumbers,[2 2 2])); assert(isequal(t.Children,h)); assert(isequal(h.YData,[6 5]));"
        )).unwrap();
    }
}

#[test]
fn deleting_any_ortho_pane_leaves_unrelated_axes_and_children() {
    for (deleted, surviving) in [("xy", "yz"), ("yz", "xz"), ("xz", "xy")] {
        let mut e = Engine::default();
        e.execute(&format!(
            "o=orthosliceViewer(reshape(1:8,2,2,2)); [xy,yz,xz]=getAxesHandles(o);
             hold({surviving},'on'); h=plot({surviving},[9 7]);
             nexttile(4); other=gca; hold(other,'on'); k=plot(other,[6 5]);
             set(gcf,'CurrentAxes',xy); assert(isgraphics(o)); delete({deleted});
             assert(~isgraphics(o)); assert(~isgraphics({deleted})); assert(isgraphics({surviving}));
             assert(isequal({surviving}.Children,h)); assert(isequal(h.YData,[9 7]));
             assert(isequal(other.Children,k)); assert(isequal(k.YData,[6 5]));"
        )).unwrap();
        assert!(e.execute("getAxesHandles(o);").is_err());
        assert!(e.execute("set(o,'SliceNumbers',[2 2 2]);").is_err());
        for (_, ax) in e.figure.as_ref().unwrap().all_axes() {
            assert!(ax.extras.images.is_empty());
        }
    }
    let mut e = Engine::default();
    e.execute(
        "o=orthosliceViewer(reshape(1:8,2,2,2)); panes=getAxesHandles(o); delete(panes);
               assert(~isgraphics(o)); assert(~any(isgraphics(panes)));",
    )
    .unwrap();
}

#[test]
fn set_parent_moves_the_same_slice_viewer_and_preserves_held_plots() {
    let mut e = Engine::default();
    e.execute(
        "old=figure(11); s=sliceViewer(reshape(1:8,2,2,2),'Tag','scan','SliceNumber',2);
         alias=s; a=getAxesHandles(s); hold(a,'on'); h=plot(a,[7 9]); view(a,30,60);
         dest=figure; b=gca; k=plot(b,[6 5]); closed=figure; close(closed);",
    )
    .unwrap();
    let before: Vec<_> = e.figures().map(|(number, f)| (number, f.clone())).collect();
    for op in [
        "s.Parent=b;",
        "set(alias,'SliceNumber',1,'Parent',closed);",
        "set(s,'Parent',dest,'DisplayRange',[2 1]);",
        "s.Parent=[old dest];",
        "s.Parent=7;",
    ] {
        assert!(e.execute(op).is_err(), "{op}");
        for ((number, figure), (saved_number, saved)) in e.figures().zip(&before) {
            assert_eq!(number, *saved_number);
            assert!(figure.same_snapshot(saved), "{op}");
        }
        e.execute("assert(isequal(s.Parent,old)); assert(isequal(getAxesHandles(alias),a));
                   assert(s.SliceNumber==2); assert(isequal(a.Children,h)); assert(isequal(b.Children,k));")
            .unwrap();
    }
    e.execute(
        "set(s,'Parent',dest,'DisplayRange',[1 8]); moved=getAxesHandles(alias);
         assert(isequal(s,alias)); assert(isequal(alias.Parent,dest)); assert(isequal(moved.Parent,dest));
         assert(~isequal(moved,a)); assert(isequal(a.Parent,old)); assert(isequal(moved.View,[30 60]));
         assert(isequal(h.Parent,a)); assert(isequal(h.YData,[7 9])); assert(isequal(k.Parent,b));
         assert(strcmp(s.Tag,'scan')); assert(s.SliceNumber==2); assert(isequal(s.DisplayRange,[1 8]));
         alias.Parent=dest; assert(isequal(getAxesHandles(s),moved));",
    ).unwrap();
    let (_, old_figure) = e.figures().find(|(number, _)| *number == 11).unwrap();
    assert!(old_figure.slice_viewers.is_empty());
    assert!(old_figure.axes.extras.images.is_empty());
    e.execute("close(old); assert(~isgraphics(old)); assert(~isgraphics(a)); assert(isgraphics(moved));
               assert(isgraphics(alias)); assert(isequal(getAxesHandles(s),moved)); assert(isgraphics(k));")
        .unwrap();
    assert_eq!(
        pane_rgb(&e.figure.as_ref().unwrap().axes),
        [146, 146, 146, 219, 219, 219, 182, 182, 182, 255, 255, 255]
    );
    e.execute("alias.SliceNumber=1; assert(s.SliceNumber==1);")
        .unwrap();
    assert_eq!(
        pane_rgb(&e.figure.as_ref().unwrap().axes),
        [0, 0, 0, 73, 73, 73, 36, 36, 36, 109, 109, 109]
    );
}

#[test]
fn dot_parent_moves_all_three_panes_and_retires_empty_old_axes() {
    let mut e = Engine::default();
    e.execute(
        "old=figure; o=orthosliceViewer(reshape(1:8,2,2,2),'SliceNumbers',[2 1 2], ...
                   'CrosshairEnabled','off','ScaleFactors',[2 3 4],'Tag','ortho');
         alias=o; before=getAxesHandles(o); dest=figure; o.Parent=dest;
         [xy,yz,xz]=getAxesHandles(alias); moved=[xy yz xz];
         assert(all(isgraphics(moved))); assert(~any(isgraphics(before))); assert(isgraphics(old));
         assert(isequal(xy.Parent,dest)); assert(isequal(yz.Parent,dest)); assert(isequal(xz.Parent,dest));
         assert(isequal(o.Parent,dest)); assert(isequal(o.SliceNumbers,[2 1 2]));
         assert(isequal(o.ScaleFactors,[2 3 4])); assert(strcmp(o.Tag,'ortho'));
         assert(strcmp(o.CrosshairEnabled,'off')); close(old);
         assert(~isgraphics(old)); assert(isgraphics(o)); assert(isequal(getAxesHandles(o),moved));",
    ).unwrap();
    let figure = e.figure.as_ref().unwrap();
    let ov = &figure.orthoslice_viewers[0];
    for (id, expected) in
        ov.axes_identities
            .iter()
            .zip([[5., 6., 7., 8.], [3., 4., 7., 8.], [1., 5., 3., 7.]])
    {
        let (_, a) = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(id))
            .unwrap();
        assert_eq!(&*a.extras.images[0].data, &expected);
    }
    assert!(e.execute("alias.Parent=old;").is_err());
    e.execute(
        "assert(isequal(getAxesHandles(o),moved)); set(alias,'SliceNumbers',[1 2 1]);
               assert(isequal(o.SliceNumbers,[1 2 1]));",
    )
    .unwrap();
}
