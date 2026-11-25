use matrust::{
    Engine, files,
    plot::{AxisScale, LinePattern, PlotKind},
};
use std::path::Path;

#[test]
fn matlab_generated_figures_import_data_labels_scales_and_styles() {
    for name in ["matlab-lines.fig", "matlab-lines-v6.fig"] {
        let f = files::load_figure(&Path::new("tests/fixtures/fig").join(name)).unwrap();
        assert_eq!(f.series.len(), 2);
        assert_eq!(f.point_count(), 6);
        assert_eq!(
            (&*f.title, &*f.xlabel, &*f.ylabel),
            ("Shared figure", "Time", "Amplitude")
        );
        assert!(f.legend && f.xgrid && f.ygrid && f.hold);
        assert_eq!(f.xlim, Some([1., 100.]));
        assert_eq!(
            f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 1.], [10., 2.], [100., 4.]]
        );
        assert_eq!(f.series[0].pattern, LinePattern::Dashed);
        assert_eq!(f.series[0].color, [255, 0, 0]);
        assert!(f.series[0].markers);
        assert_eq!(
            f.series[1].points.iter().collect::<Vec<_>>().as_slice(),
            &[[1., 3.], [10., 4.], [100., 6.]]
        );
    }
    let f = files::load_figure(Path::new("tests/fixtures/fig/matlab-log-scatter.fig")).unwrap();
    assert_eq!(f.xscale, AxisScale::Log);
    assert_eq!(f.series[1].kind, PlotKind::Scatter);
    assert!(f.series[1].marker_filled);
    assert_eq!(f.series[1].color, [255, 0, 0]);
    let f = files::load_figure(Path::new("tests/fixtures/fig/matlab-bars.fig")).unwrap();
    assert_eq!(f.series[0].kind, PlotKind::Bar);
    assert!((f.series[0].bar_width - 0.8 * 2. / 7.).abs() < 1e-14);
    assert_eq!(f.series[0].bar_offset, -1. / 7.);
    assert_eq!(f.series[1].bar_offset, 1. / 7.);
}

#[test]
fn matlab_bar_geometry_uses_axis_units_and_bar_width() {
    // Independent reference fixtures; Face.VertexData was checked by the reference.
    for (name, width, offset) in [
        ("matlab-bar-spacing-1.fig", 0.8 * 6. / 7., 3. / 7.),
        ("matlab-bar-spacing-2.fig", 0.55 * 6. / 7., 3. / 7.),
        ("matlab-bar-spacing-3.fig", 0.6, 0.),
    ] {
        let f = files::load_figure(&Path::new("tests/fixtures/fig").join(name)).unwrap();
        assert!((f.series[0].bar_width - width).abs() < 1e-14);
        assert!((f.series[0].bar_offset + offset).abs() < 1e-14);
        if f.series.len() == 2 {
            assert!((f.series[1].bar_width - width).abs() < 1e-14);
            assert!((f.series[1].bar_offset - offset).abs() < 1e-14);
            assert_eq!(
                f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
                &[[2., 2.], [5., 3.], [9., 4.]]
            );
        } else {
            assert_eq!(
                f.series[0].points.iter().collect::<Vec<_>>().as_slice(),
                &[[7., 4.]]
            );
        }
    }
}

#[test]
fn fig_writer_preserves_sample_bits_and_utf16_across_many_series() {
    use matrust::plot::{Figure, Series};
    let mut f = Figure {
        identity: matrust::ObjectIdentity::default(),
        axes: matrust::plot::Axes {
            title: "\u{03bb}\u{1f680}".into(),
            ..Default::default()
        },
        layout: None,
        textboxes: Vec::new(),
        name: String::new(),
        appearance: None,
        invert_hardcopy: None,
        color: [255; 3],
        ..Figure::default()
    };
    let samples = vec![
        [1., -0.],
        [2., f64::from_bits(0x7ff8_0000_0000_1234)],
        [3., f64::INFINITY],
        [4., 5.],
    ];
    for i in 0..64 {
        let mut s = Series::new(samples.clone(), PlotKind::Line);
        s.label = format!("series {i} \u{03bb}");
        s.visible = i % 2 == 0;
        f.add(vec![s]).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.fig");
    files::save_matlab_figure(&path, &f).unwrap();
    let reopened = files::load_matlab_figure(&path).unwrap();
    assert_eq!(reopened.title, f.title);
    assert_eq!(reopened.series.len(), 64);
    for (a, b) in reopened.series.iter().zip(&f.series) {
        assert_eq!((a.visible, &a.label), (b.visible, &b.label));
        for (a, b) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
}

#[test]
fn fig_exports_roundtrip_without_using_custom_mfig_bytes() {
    let dir = tempfile::tempdir().unwrap();
    for source in [
        "semilogx([1 10 100],[1 2 3],'r--o');title('Unicode λ');xlabel('Time');ylabel('Value');",
        "plot([1 2],[3 4]);hold on;scatter([1 2],[5 6]);legend('A','B');",
        "bar([1 2 3],[2 4;3 5;4 6]);legend('A','B');",
    ] {
        let mut e = Engine::default();
        e.execute(source).unwrap();
        let f = e.figure.unwrap();
        let path = dir.path().join("test.fig");
        let mut f = f;
        if f.legend {
            std::fs::write(&path, b"previous").unwrap();
            assert!(files::save_figure(&path, &f).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            f.legend = false;
            // Hidden explicit legend state is still outside the supported FIG
            // writer schema; disabling visibility must not silently discard it.
            assert!(f.legend_state.is_some());
            assert!(files::save_figure(&path, &f).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            // This branch intentionally tests the separate nonlegend baseline.
            f.legend_state = None;
        }
        if f.series.iter().any(|s| s.kind == PlotKind::Bar) {
            assert!(files::save_figure(&path, &f).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            continue;
        }
        files::save_figure(&path, &f).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"MATLAB 5.0"));
        let reopened = files::load_figure(&path).unwrap();
        assert_eq!(reopened.title, f.title);
        assert_eq!(reopened.xscale, f.xscale);
        assert_eq!(reopened.legend, f.legend);
        assert_eq!(reopened.series.len(), f.series.len());
        for (a, b) in reopened.series.iter().zip(&f.series) {
            assert_eq!(a.points, b.points);
            assert_eq!(
                a.color,
                f.resolve_appearance(matrust::plot::BaseTheme::Light)
                    .series_color(b)
            );
            assert!(a.explicit_color && a.auto_palette.is_none());
            if b.explicit_color || b.auto_palette.is_none() {
                assert_eq!(a.color, b.color);
            }
            assert_eq!(a.pattern, b.pattern);
            assert_eq!(a.kind, b.kind);
            assert!((a.bar_width - b.bar_width).abs() < 1e-14);
        }
    }
}

#[test]
fn unsupported_and_truncated_figures_fail_without_panics() {
    assert!(files::load_figure(Path::new("tests/fixtures/fig/matlab-surface.fig")).is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.fig");
    let source = std::fs::read("tests/fixtures/fig/matlab-lines.fig").unwrap();
    for end in (0..128)
        .chain((128..source.len()).step_by(137))
        .chain(source.len() - 8..source.len())
    {
        std::fs::write(&path, &source[..end]).unwrap();
        assert!(files::load_figure(&path).is_err(), "truncated at {end}");
    }
    let mut corrupt = source;
    corrupt[132..136].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&path, corrupt).unwrap();
    assert!(files::load_figure(&path).is_err());
    let mut random = 0x12345678_u32;
    for size in [0, 127, 128, 129, 256, 1024, 4096] {
        let mut bytes = vec![0; size];
        for b in &mut bytes {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *b = random as u8;
        }
        if size >= 128 {
            bytes[126..128].copy_from_slice(b"IM");
        }
        std::fs::write(&path, bytes).unwrap();
        assert!(files::load_figure(&path).is_err());
    }
}

#[test]
fn mixed_volume_and_styled_subplot_preserve_both_on_native_reopen() {
    use matrust::plot::{LabelStyle, Position, RenderingStyle};

    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "subplot(1,2,1); \
         V = reshape(1:24, 2, 3, 4); \
         volshow(V, RenderingStyle='MaximumIntensityProjection'); \
         subplot(1,2,2); \
         plot(1:4, (1:4).^2); \
         title('Styled Pane','FontSize',22,'FontWeight','normal','FontAngle','italic'); \
         xlabel('Time (s)','FontSize',15,'FontWeight','bold','FontAngle','italic'); \
         ylabel('Signal','FontSize',13,'FontWeight','bold','FontAngle','normal'); \
         savefig('mixed.mrfig'); \
         close all; \
         openfig('mixed.mrfig');",
    )
    .unwrap();

    let reopened = e.figures().next().unwrap().1;
    let mut panes: Vec<_> = reopened.all_axes().collect();
    panes.sort_by_key(|(pos, _)| *pos);
    assert_eq!(panes.len(), 2);

    let left_pos = Position::from_grid(1, 2, &[1]).unwrap();
    let right_pos = Position::from_grid(1, 2, &[2]).unwrap();
    assert_eq!(panes[0].0, left_pos);
    assert_eq!(panes[1].0, right_pos);
    assert_eq!(reopened.active_position(), right_pos);

    let (vol_pane, plot_pane) = (panes[0].1, panes[1].1);
    let expected_voxels: Vec<f64> = (1..=24).map(f64::from).collect();
    assert_eq!(vol_pane.extras.volumes.len(), 1);
    let vol = &vol_pane.extras.volumes[0];
    assert_eq!(vol.dims, [2, 3, 4]);
    assert_eq!(vol.data.as_ref(), expected_voxels.as_slice());
    assert_eq!(
        vol.rendering_style,
        RenderingStyle::MaximumIntensityProjection
    );
    assert!(vol_pane.series.is_empty());

    assert!(plot_pane.extras.volumes.is_empty());
    assert_eq!(plot_pane.series.len(), 1);
    assert_eq!(plot_pane.title, "Styled Pane");
    assert_eq!(plot_pane.xlabel, "Time (s)");
    assert_eq!(plot_pane.ylabel, "Signal");
    assert_eq!(
        plot_pane.extras.title_style,
        LabelStyle {
            font_size: Some(22.0),
            bold: Some(false),
            italic: Some(true),
        }
    );
    assert_eq!(
        plot_pane.extras.label_styles[0],
        LabelStyle {
            font_size: Some(15.0),
            bold: Some(true),
            italic: Some(true),
        }
    );
    assert_eq!(
        plot_pane.extras.label_styles[1],
        LabelStyle {
            font_size: Some(13.0),
            bold: Some(true),
            italic: Some(false),
        }
    );

    let fig_path = dir.path().join("prior.fig");
    std::fs::write(&fig_path, b"known-prior-fig").unwrap();
    let err = e.execute("savefig('prior.fig');").unwrap_err();
    assert!(
        err.contains("volshow objects are saved only in .mrfig"),
        "{err}"
    );
    assert_eq!(std::fs::read(&fig_path).unwrap(), b"known-prior-fig");
}

#[test]
fn native_volume_children_and_categorical_labels_roundtrip_per_axes() {
    use matrust::plot::render_volume;

    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "L = reshape([2 0 0 1 0 1 2 0],2,2,2); \
         C = categorical(L,[2 1],{'second','first'},'Ordinal',true); \
         subplot(1,2,1); hold on; \
         volshow(zeros(2,2,2), RenderingStyle='MaximumIntensityProjection', Colormap='gray', \
             OverlayData=C, OverlayColormap=[1 0 0;0 0 1], OverlayAlphamap=[0.25 0.75]); \
         view(0,90); plot(1:2,[11 12]); left=gca; \
         subplot(1,2,2); hold on; plot(1:2,[21 22]); \
         volshow(reshape(31:38,2,2,2)); plot(1:2,[41 42]); right=gca;",
    )
    .unwrap();
    let check_children = "lc = left.Children; rc = right.Children; \
         assert(numel(lc)==2 && numel(rc)==3); \
         ll=lc(1); lv=lc(2); rl=rc(1); rv=rc(2); old=rc(3); \
         assert(strcmp(ll.Type,'line') && strcmp(lv.Type,'volume')); \
         assert(strcmp(rl.Type,'line') && strcmp(rv.Type,'volume') && strcmp(old.Type,'line')); \
         assert(isequal(ll.YData,[11 12]) && isequal(old.YData,[21 22])); \
         assert(isequal(rl.YData,[41 42]) && isequal(rv.Data,reshape(31:38,2,2,2)));";
    e.execute(check_children).unwrap();
    let render_categorical = |e: &Engine| {
        let fig = e.figures().next().unwrap().1;
        let (_, ax) = fig
            .all_axes()
            .find(|(_, a)| a.extras.volumes[0].overlay_data.is_some())
            .unwrap();
        render_volume(&ax.extras.volumes[0], ax.view, None)
    };
    let before = render_categorical(&e);
    assert!(before.2.iter().any(|p| p[0] > 0 && p[0] < 255));
    assert!(before.2.iter().any(|p| p[2] > 0 && p[2] < 255));
    e.execute(
        "savefig('children.mrfig'); close all; openfig('children.mrfig'); \
         subplot(1,2,1); left=gca; subplot(1,2,2); right=gca;",
    )
    .unwrap();
    e.execute(check_children).unwrap();
    e.execute(
        "R = lv.OverlayData; assert(strcmp(class(R),'categorical')); \
         assert(isequal(size(R),[2 2 2])); \
         assert(isequal(categories(R),{'second';'first'})); \
         assert(strcmp(lv.OverlayRenderingStyle,'LabelOverlay')); \
         assert(isequal(lv.OverlayColormap,[1 0 0;0 0 1])); \
         opacity = lv.OverlayAlphamap; assert(isequal(opacity(:),[0.25;0.75]));",
    )
    .unwrap();
    let workspace = e.workspace();
    let matrust::Array::Categorical(labels) = workspace.get("R").unwrap() else {
        panic!("expected reopened categorical labels");
    };
    assert_eq!(labels.codes(), [1, 0, 0, 2, 0, 2, 1, 0]);
    assert!(labels.is_ordinal());
    assert_eq!(render_categorical(&e), before);
}

fn native_record(magic: &[u8; 8], json: &[u8], samples: &[u8]) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(json);
    bytes.extend_from_slice(samples);
    bytes
}

#[test]
fn native_legacy_unlabeled_volume_keeps_its_f64_encoding() {
    // Construct the old record independently, including the f64 payload of a
    // legacy volume whose Data property was reported as uint64.
    let metadata = br#"{"title":"","xlabel":"","ylabel":"","count":0,"volumes":[{
        "dims":[1,2,2],"data_class":"UInt64","rendering_style":"MaximumIntensityProjection",
        "colormap":[[0,0,0],[1,1,1]],"alphamap":[1],"data_limits":[1,4],
        "isosurface_value":2.5,"transformation":[1,1,1],"visible":true}]}"#;
    let values = [1.0_f64, 2.0, 3.0, 4.0];
    let samples: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.mrfig");
    std::fs::write(&path, native_record(b"MATRFIGZ", metadata, &samples)).unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    let check = "ax=f.Children; kids=ax.Children; v=kids(1); R=v.Data; \
         assert(strcmp(class(R),'uint64') && isequal(size(R),[1 2 2])); \
         assert(all(bitxor(R(:),uint64([1;2;3;4]))==uint64(0))); \
         assert(isempty(v.OverlayData));";
    e.execute("f=openfig('legacy.mrfig');").unwrap();
    e.execute(check).unwrap();
    e.execute("savefig('resaved.mrfig'); close all; f=openfig('resaved.mrfig');")
        .unwrap();
    e.execute(check).unwrap();
}

#[test]
fn native_volume_rejects_invalid_wire_records_before_opening() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("plot(1:2); hold on; volshow(zeros(2,2,2), OverlayData=true(2,2,2)); savefig('valid.mrfig');")
        .unwrap();
    let bytes = std::fs::read(dir.path().join("valid.mrfig")).unwrap();
    assert_eq!(&bytes[..8], b"MATRFIGc");
    let end = 12 + u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&bytes[12..end]).unwrap();
    let payload = &bytes[end..];
    let mut invalid = Vec::new();
    for order in [
        serde_json::json!([{"Series":0},{"Series":0}]),
        serde_json::json!([{"Series":0},{"Volume":1}]),
        serde_json::json!([{"Volume":0}]),
        serde_json::Value::Null,
    ] {
        let mut json = json.clone();
        json["data_children"] = order;
        invalid.push((
            native_record(b"MATRFIGc", &serde_json::to_vec(&json).unwrap(), payload),
            "child order",
        ));
    }
    let mut oversized = json.clone();
    oversized["volumes"][0]["dims"] = serde_json::json!([4000, 4000, 3]);
    invalid.push((
        native_record(
            b"MATRFIGc",
            &serde_json::to_vec(&oversized).unwrap(),
            payload,
        ),
        "voxel",
    ));
    let mut bad_map = json.clone();
    bad_map["volumes"][0]["overlay"]["alphamap_len"] = 0.into();
    invalid.push((
        native_record(b"MATRFIGc", &serde_json::to_vec(&bad_map).unwrap(), payload),
        "overlay metadata",
    ));
    let mut bad_exact = json;
    bad_exact["volumes"][0]["exact_data"] = true.into();
    invalid.push((
        native_record(
            b"MATRFIGc",
            &serde_json::to_vec(&bad_exact).unwrap(),
            payload,
        ),
        "exact volume data class",
    ));
    invalid.push((
        native_record(b"MATRFIGZ", &bytes[12..end], payload),
        "requires MFIGc",
    ));
    invalid.push((
        native_record(b"MATRFIG?", &bytes[12..end], payload),
        "supported MatRust figure",
    ));
    invalid.push((bytes[..bytes.len() - 1].to_vec(), "truncated"));
    let mut trailing = bytes.clone();
    trailing.push(0);
    invalid.push((trailing, "trailing"));
    for (bytes, expected) in invalid {
        std::fs::write(dir.path().join("invalid.mrfig"), bytes).unwrap();
        let error = e.execute("openfig('invalid.mrfig');").unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert_eq!(e.figures().count(), 1);
    }
}

#[test]
fn native_subplot_checks_combined_metadata_budget() {
    use matrust::plot::{Figure, OverlayDataClass};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "subplot(1,2,1); volshow(zeros(2,2,2), OverlayData=ones(2,2,2)); \
         subplot(1,2,2); volshow(zeros(2,2,2), OverlayData=ones(2,2,2)); \
         savefig('small.mrfig');",
    )
    .unwrap();
    let path = dir.path().join("small.mrfig");
    let prior = std::fs::read(&path).unwrap();
    assert_eq!(&prior[..8], b"MATRFIGd");
    let end = 12 + u32::from_le_bytes(prior[8..12].try_into().unwrap()) as usize;
    let mut old_magic = prior.clone();
    old_magic[..8].copy_from_slice(b"MATRFIGa");
    let bad_path = dir.path().join("invalid.mrfig");
    std::fs::write(&bad_path, old_magic).unwrap();
    assert!(
        files::load_figure(&bad_path)
            .unwrap_err()
            .contains("require MFIGd")
    );
    std::fs::remove_file(&bad_path).unwrap();

    let mut combined = e.figures().next().unwrap().1.clone();
    let class = OverlayDataClass::Categorical {
        categories: Arc::new(vec!["c".repeat(190_000)]),
        ordinal: false,
    };
    let mut child_payloads = Vec::new();
    for axes in std::iter::once(&mut combined.axes).chain(
        combined
            .layout
            .as_mut()
            .unwrap()
            .others
            .iter_mut()
            .map(|p| &mut p.axes),
    ) {
        axes.extras.volumes[0].overlay_data.as_mut().unwrap().class = class.clone();
        let single = Figure {
            axes: axes.clone(),
            appearance: None,
            ..Default::default()
        };
        let encoded = files::encode_native_figure(&single).unwrap();
        child_payloads.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        child_payloads.extend_from_slice(&encoded);
    }
    for target in [&path, &bad_path] {
        let err = files::save_figure(target, &combined).unwrap_err();
        assert!(err.contains("combined metadata limit"), "{err}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), prior);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    let oversize = native_record(b"MATRFIGd", &prior[12..end], &child_payloads);
    std::fs::write(&bad_path, oversize).unwrap();
    let err = files::load_figure(&bad_path).unwrap_err();
    assert!(err.contains("combined metadata limit"), "{err}");
}

#[test]
#[cfg(feature = "plot-export")]
fn native_slice_viewer_reopens_with_pixels_and_can_change_slice() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "s=sliceViewer(reshape(1:8,2,2,2),'SliceNumber',2,'DisplayRange',[1 9], \
           'Colormap',[0 0 1;0 1 0;1 0 0;1 1 0],'Tag','saved slice'); \
         exportgraphics(gcf,'slice-before.svg');",
    )
    .unwrap();
    let before = svg_images(&dir.path().join("slice-before.svg"));
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].1.dimensions(), (2, 2));
    assert_eq!(
        before[0].1.as_raw(),
        &[255, 0, 0, 255, 255, 0, 255, 0, 0, 255, 255, 0]
    );
    let old = e.figure.as_ref().unwrap().slice_viewers[0].clone();
    let old_handle = old.graphics_handle(&e.figure.as_ref().unwrap().identity);
    let old_image = e.figure.as_ref().unwrap().axes.extras.images[0]
        .identity
        .clone();
    e.execute("savefig('slice.mrfig'); close all;").unwrap();

    let mut reopened = Engine::default();
    reopened.set_file_directory(dir.path()).unwrap();
    reopened
        .execute(
            "f=openfig('slice.mrfig'); assert(isgraphics(f)); exportgraphics(f,'slice-after.svg');",
        )
        .unwrap();
    let after = svg_images(&dir.path().join("slice-after.svg"));
    assert_eq!(after, before);
    let figure = reopened.figure.as_ref().unwrap();
    assert_eq!(figure.slice_viewers.len(), 1);
    assert_eq!(figure.axes.extras.images.len(), 1);
    assert!(
        !figure.axes.extras.images[0]
            .identity
            .same_object(&old_image)
    );
    let viewer = &figure.slice_viewers[0];
    assert!(!viewer.identity.same_object(&old.identity));
    assert!(!viewer.axes_identity.same_object(&old.axes_identity));
    assert!(viewer.axes_identity.same_object(&figure.axes.identity));
    assert_eq!(viewer.slice_number, 2);
    assert_eq!(viewer.data.as_ref(), &[1., 2., 3., 4., 5., 6., 7., 8.]);
    assert_eq!(viewer.colormap, old.colormap);
    assert_eq!(viewer.effective_display_range(), [1., 9.]);
    assert_eq!(viewer.tag, "saved slice");
    let handle = viewer.graphics_handle(&figure.identity);
    assert!(handle.is_live_in(figure));
    assert!(!old_handle.is_live_in(figure));
    bind_reopened_viewer(&mut reopened, handle);
    reopened
        .execute(
            "assert(isgraphics(viewer)); assert(isequal(viewer.Parent,f)); \
         assert(isequal(getAxesHandles(viewer),findobj(f,'Type','axes'))); \
         viewer.SliceNumber=1; assert(viewer.SliceNumber==1); \
         exportgraphics(f,'slice-step.svg');",
        )
        .unwrap();
    let step = svg_images(&dir.path().join("slice-step.svg"));
    assert_eq!(step.len(), 1);
    assert_eq!(
        step[0].1.as_raw(),
        &[0, 0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 0]
    );
}

#[test]
#[cfg(feature = "plot-export")]
fn native_orthoslice_viewer_reopens_three_distinct_panes_and_crosshairs() {
    use matrust::plot::{ImageDataClass, Position};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "map=repmat((0:31)'/31,1,3); \
         o=orthosliceViewer(reshape(uint16(1:24),2,3,4),'SliceNumbers',[1 2 4], \
           'Colormap',map,'DisplayRange',[0 32],'ScaleFactors',[2 3 4],'Tag','saved ortho'); \
         [xy,yz,xz]=getAxesHandles(o); hold(yz,'on'); \
         h=plot(yz,[4 12],[3 6],'DisplayName','held'); assert(isgraphics(h)); \
         exportgraphics(gcf,'ortho-before.svg');",
    )
    .unwrap();
    let before_path = dir.path().join("ortho-before.svg");
    let before = ortho_images(&before_path);
    let before_crosshairs = svg_crosshairs(&before_path);
    assert_eq!(before_crosshairs.len(), 6);
    let old = e.figure.as_ref().unwrap().clone();
    let old_viewer = &old.orthoslice_viewers[0];
    e.execute("savefig('ortho.mrfig'); close all;").unwrap();
    let voxels: Vec<_> = (1..=24).map(f64::from).collect();
    let voxel_bytes: Vec<_> = voxels.iter().flat_map(|v| v.to_le_bytes()).collect();
    let native = std::fs::read(dir.path().join("ortho.mrfig")).unwrap();
    assert_eq!(&native[..8], b"MATRFIGd");
    assert_eq!(
        native
            .windows(voxel_bytes.len())
            .filter(|&w| w == voxel_bytes)
            .count(),
        1
    );

    let mut reopened = Engine::default();
    reopened.set_file_directory(dir.path()).unwrap();
    reopened
        .execute("f=openfig('ortho.mrfig'); exportgraphics(f,'ortho-after.svg');")
        .unwrap();
    let after_path = dir.path().join("ortho-after.svg");
    assert_eq!(ortho_images(&after_path), before);
    assert_eq!(svg_crosshairs(&after_path), before_crosshairs);
    let figure = reopened.figure.as_ref().unwrap();
    assert_eq!(figure.orthoslice_viewers.len(), 1);
    let viewer = &figure.orthoslice_viewers[0];
    let handle = viewer.graphics_handle(&figure.identity);
    assert!(handle.is_live_in(figure));
    assert!(!old_viewer.graphics_handle(&old.identity).is_live_in(figure));
    assert_eq!(viewer.dims, [2, 3, 4]);
    assert_eq!(
        viewer.data_class,
        ImageDataClass::Typed(matrust::NumericClass::Uint16)
    );
    assert_eq!(viewer.data.as_ref(), voxels);
    assert_eq!(viewer.slice_numbers, [1, 2, 4]);
    assert_eq!(viewer.colormap, old_viewer.colormap);
    assert_eq!(viewer.effective_display_range(), [0., 32.]);
    assert_eq!(viewer.scale_factors, [2., 3., 4.]);
    assert_eq!(viewer.tag, "saved ortho");
    assert!(viewer.visible && viewer.crosshair_enabled);
    for (i, id) in viewer.axes_identities.iter().enumerate() {
        let (position, axes) = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(id))
            .unwrap();
        let (old_position, old_axes) = old
            .all_axes()
            .find(|(_, a)| a.identity.same_object(&old_viewer.axes_identities[i]))
            .unwrap();
        assert_eq!(position, Position::from_grid(2, 2, &[i + 1]).unwrap());
        assert_eq!(position, old_position);
        assert!(!id.same_object(&old_axes.identity));
        assert_eq!(axes.extras.images.len(), 1);
        assert!(
            !axes.extras.images[0]
                .identity
                .same_object(&old_axes.extras.images[0].identity)
        );
        assert!(Arc::ptr_eq(
            axes.extras.colormap.as_ref().unwrap(),
            &viewer.colormap
        ));
        assert_eq!(axes.series.len(), 2 + usize::from(i == 1));
        for (line, old_line) in axes.series.iter().zip(&old_axes.series) {
            assert!(!line.identity.same_object(&old_line.identity));
            assert_eq!(
                line.points.iter().collect::<Vec<_>>(),
                old_line.points.iter().collect::<Vec<_>>()
            );
        }
    }

    let volume = viewer.data.clone();
    bind_reopened_viewer(&mut reopened, handle);
    reopened.execute(
        "assert(isgraphics(viewer)); assert(isequal(viewer.Parent,f)); \
         [xy,yz,xz]=getAxesHandles(viewer); assert(isgraphics(xy) && isgraphics(yz) && isgraphics(xz)); \
         h=findobj(f,'Type','line','DisplayName','held'); assert(isgraphics(h)); \
         assert(isequal(h.Parent,yz)); assert(isequal(h.XData,[4 12])); assert(isequal(h.YData,[3 6])); \
         viewer.CrosshairEnabled='off'; assert(isgraphics(h)); \
         exportgraphics(f,'ortho-hidden.svg'); savefig('ortho-hidden.mrfig');",
    ).unwrap();
    let viewer = &reopened.figure.as_ref().unwrap().orthoslice_viewers[0];
    assert!(Arc::ptr_eq(&viewer.data, &volume));
    let hidden_path = dir.path().join("ortho-hidden.svg");
    assert_eq!(ortho_images(&hidden_path), before);
    assert!(svg_crosshairs(&hidden_path).is_empty());

    let mut hidden = Engine::default();
    hidden.set_file_directory(dir.path()).unwrap();
    hidden
        .execute("f=openfig('ortho-hidden.mrfig'); exportgraphics(f,'ortho-hidden-after.svg');")
        .unwrap();
    assert!(!hidden.figure.as_ref().unwrap().orthoslice_viewers[0].crosshair_enabled);
    let hidden_path = dir.path().join("ortho-hidden-after.svg");
    assert_eq!(ortho_images(&hidden_path), before);
    assert!(svg_crosshairs(&hidden_path).is_empty());
}

#[cfg(feature = "plot-export")]
fn ortho_images(path: &Path) -> Vec<(resvg::usvg::Rect, image::RgbImage)> {
    let mut images = svg_images(path);
    assert_eq!(images.len(), 3);
    images.sort_by(|(a, _), (b, _)| a.top().total_cmp(&b.top()));
    images[..2].sort_by(|(a, _), (b, _)| a.left().total_cmp(&b.left()));
    let [(xy, _), (yz, _), (xz, _)] = images.as_slice() else {
        unreachable!()
    };
    assert!(xy.right() < yz.left());
    assert!(xz.right() < yz.left());
    assert!(xy.bottom().max(yz.bottom()) < xz.top());
    for ((_, image), (width, height, values)) in images.iter().zip([
        (3, 2, &[19, 21, 23, 20, 22, 24][..]),
        (4, 2, &[1, 7, 13, 19, 2, 8, 14, 20][..]),
        (3, 4, &[2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24][..]),
    ]) {
        assert_eq!(image.dimensions(), (width, height));
        let expected: Vec<_> = values
            .iter()
            .flat_map(|&v| [((f64::from(v) * 255.) / 31.).round() as u8; 3])
            .collect();
        assert_eq!(image.as_raw(), &expected);
    }
    images
}

#[cfg(feature = "plot-export")]
fn svg_crosshairs(path: &Path) -> Vec<String> {
    let svg = std::fs::read_to_string(path).unwrap();
    let document = resvg::usvg::roxmltree::Document::parse(&svg).unwrap();
    document
        .descendants()
        .filter(|node| node.has_tag_name("path") && node.attribute("stroke") == Some("#00d2ff"))
        .map(|node| node.attribute("d").unwrap().to_owned())
        .collect()
}

#[cfg(feature = "plot-export")]
fn bind_reopened_viewer(engine: &mut Engine, handle: matrust::GraphicsHandle) {
    use std::sync::{Arc, atomic::AtomicBool};

    let graphics =
        matrust::GraphicsArray::from_shared(1, 1, vec![Arc::new(handle)], &AtomicBool::new(false))
            .unwrap();
    engine
        .merge_workspace([("viewer".into(), matrust::Array::Graphics(graphics))].into())
        .unwrap();
}

#[cfg(feature = "plot-export")]
fn svg_images(path: &Path) -> Vec<(resvg::usvg::Rect, image::RgbImage)> {
    use resvg::usvg::{Group, ImageKind, Node, Rect, Tree};

    fn visit(group: &Group, images: &mut Vec<(Rect, image::RgbImage)>) {
        for node in group.children() {
            match node {
                Node::Group(group) => visit(group, images),
                Node::Image(node) => {
                    let ImageKind::PNG(bytes) = node.kind() else {
                        panic!("expected an embedded slice PNG");
                    };
                    images.push((
                        node.bounding_box().transform(node.abs_transform()).unwrap(),
                        image::load_from_memory(bytes).unwrap().to_rgb8(),
                    ));
                }
                _ => {}
            }
        }
    }

    let svg = std::fs::read_to_string(path).unwrap();
    let tree = Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
    let mut images = Vec::new();
    visit(tree.root(), &mut images);
    images
}

#[test]
fn invalid_or_over_budget_native_slice_viewer_save_leaves_paths_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("plot(1:2); savefig('prior.mrfig');").unwrap();
    let prior_path = dir.path().join("prior.mrfig");
    let prior = std::fs::read(&prior_path).unwrap();
    files::load_figure(&prior_path).unwrap();
    for viewer in ["sliceViewer", "orthosliceViewer"] {
        e.execute(&format!("close all; {viewer}(reshape(1:8,2,2,2));"))
            .unwrap();
        let figure = e.figure.as_ref().unwrap();
        let mut invalid = figure.clone();
        if let Some(single) = invalid.slice_viewers.first_mut() {
            single.slice_number = 3;
        } else {
            invalid.orthoslice_viewers[0].slice_numbers = [1, 1, 3];
        }
        for path in [&prior_path, &dir.path().join("missing.mrfig")] {
            let err = files::save_figure(path, &invalid).unwrap_err();
            assert!(err.contains("SliceNumber"), "{err}");
            let complete_size = files::encode_native_figure(figure).unwrap().len();
            let err = files::save_figure_with_limit(path, figure, complete_size - 1).unwrap_err();
            assert!(err.contains("byte save limit"), "{err}");
        }
        assert_eq!(std::fs::read(&prior_path).unwrap(), prior);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn native_slice_viewer_rejects_invalid_settings_panes_and_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "sliceViewer(reshape(1:8,2,2,2)); savefig('single.mrfig'); close all; \
         orthosliceViewer(reshape(1:8,2,2,2)); savefig('ortho.mrfig'); close all; plot(1:2);",
    )
    .unwrap();
    let open_path = dir.path().join("bad.mrfig");
    for (filename, magic, legacy) in [
        ("single.mrfig", b"MATRFIGc", b"MATRFIGb"),
        ("ortho.mrfig", b"MATRFIGd", b"MATRFIGa"),
    ] {
        let bytes = std::fs::read(dir.path().join(filename)).unwrap();
        assert_eq!(&bytes[..8], magic);
        let end = 12 + u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let json: serde_json::Value = serde_json::from_slice(&bytes[12..end]).unwrap();
        let payload = &bytes[end..];
        let mut invalid = vec![(
            native_record(legacy, &bytes[12..end], payload),
            "Slice viewer metadata requires",
        )];
        let mut oversized = json.clone();
        oversized["slice_viewers"][0]["dims"] = serde_json::json!([4000, 4000, 100]);
        invalid.push((
            native_record(magic, &serde_json::to_vec(&oversized).unwrap(), payload),
            "dimensions or voxel count",
        ));
        let mut unsupported = json.clone();
        unsupported["slice_viewers"][0]["unsupported_setting"] = true.into();
        invalid.push((
            native_record(magic, &serde_json::to_vec(&unsupported).unwrap(), payload),
            "unknown field",
        ));
        if magic == b"MATRFIGc" {
            invalid.push((bytes[..bytes.len() - 1].to_vec(), "truncated"));
            let mut trailing = bytes;
            trailing.push(0);
            invalid.push((trailing, "trailing"));
        } else {
            let mut bad_panes = json.clone();
            let panes = &mut bad_panes["slice_viewers"][0]["selection"]["Ortho"]["panes"];
            panes[1]["axes"] = panes[0]["axes"].clone();
            invalid.push((
                native_record(magic, &serde_json::to_vec(&bad_panes).unwrap(), payload),
                "repeated native slice viewer pane",
            ));
            let mut bad_crosshairs = json;
            bad_crosshairs["slice_viewers"][0]["selection"]["Ortho"]["crosshair_slots"][0] =
                serde_json::json!([0, 0]);
            invalid.push((
                native_record(
                    magic,
                    &serde_json::to_vec(&bad_crosshairs).unwrap(),
                    payload,
                ),
                "crosshair slots",
            ));
        }
        for (bytes, expected) in invalid {
            std::fs::write(&open_path, bytes).unwrap();
            let error = e.execute("openfig('bad.mrfig');").unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert_eq!(e.figures().count(), 1);
        }
    }
}
