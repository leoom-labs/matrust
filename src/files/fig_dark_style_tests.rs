//! Candidate source tests, not MATLAB acceptance. The four scenes define the future reference scope.
use super::*;
use crate::{
    Engine, files,
    plot::{BaseTheme, FigureTheme, LegendColors, SnapshotStamp},
};
use std::sync::atomic::AtomicBool;

const SCENES: [&str; 4] = [
    "f=figure;h=plot([0 1 2],[2 3 1],[0 1 2],[4 1 5]);title('Dark lines');xlabel('Time');ylabel('Value');grid on;lg=legend(h,{'First','Second'},'AutoUpdate','off','Interpreter','none');",
    "f=figure('Color',[0.1 0.2 0.3]);h=plot([0 1],[2 3],[0 1],[4 1]);set(h(1),'Color',[0.9 0.2 0.3]);set(gca,'Color',[0.05 0.1 0.2],'XColor',[0.8 0.7 0.6],'YColor',[0.3 0.6 0.9]);title('Manual','Color',[1 0.5 0.2]);xlabel('X','Color',[0.7 0.2 0.4]);ylabel('Y','Color',[0.1 0.8 0.6]);lg=legend(h,{'First','Second'},'AutoUpdate','off','Interpreter','none','Color',[0.11 0.22 0.33],'TextColor',[0.91 0.82 0.73],'EdgeColor',[-0 0.52 0.63]);grid on;",
    "f=figure;plot([0 1 2],[2 3 1]);hold on;scatter([0 1 2],[4 1 5]);xlabel('Scatter X');ylabel('Scatter Y');grid on;",
    "f=figure;subplot(1,2,1);plot([0 1 2],[2 3 1]);title('X manual');xlabel('X');ylabel('Y');set(gca,'XColor',[0.9 0.2 0.3],'YColor',[0.2 0.7 0.9],'GridColor',[0.3 0.4 0.5],'GridAlpha',0.4,'XColorMode','manual','YColorMode','auto','GridColorMode','auto','GridAlphaMode','manual');grid on;subplot(1,2,2);plot([0 1 2],[4 1 5]);title('Y manual');xlabel('X');ylabel('Y');set(gca,'XColor',[0.1 0.8 0.4],'YColor',[0.9 0.6 0.2],'GridColor',[0.5 0.3 0.7],'GridAlpha',0.15,'XColorMode','auto','YColorMode','manual','GridColorMode','auto','GridAlphaMode','auto');grid on;",
];
fn same_samples(a: &Figure, b: &Figure) {
    assert_eq!(a.all_axes().count(), b.all_axes().count());
    for ((_, a), (_, b)) in a.all_axes().zip(b.all_axes()) {
        assert_eq!(a.series.len(), b.series.len());
        for (a, b) in a.series.iter().zip(&b.series) {
            assert_eq!(
                (a.kind, a.pattern, a.visible, &a.label),
                (b.kind, b.pattern, b.visible, &b.label)
            );
            assert_eq!(a.points.len(), b.points.len());
            for (a, b) in a.points.iter().zip(b.points.iter()) {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            }
        }
    }
}
fn same_artwork(a: &Figure, host: BaseTheme, b: &Figure) {
    let aa = a.resolve_appearance(host);
    let ba = b.resolve_appearance(BaseTheme::Light);
    assert_eq!(aa.canvas, ba.canvas);
    for ((_, a), (_, b)) in a.all_axes().zip(b.all_axes()) {
        let x = a.resolve_colors(aa);
        let y = b.resolve_colors(ba);
        assert_eq!(
            [
                x.background,
                x.x,
                x.y,
                x.title,
                x.xlabel,
                x.ylabel,
                x.grid_x,
                x.grid_y
            ],
            [
                y.background,
                y.x,
                y.y,
                y.title,
                y.xlabel,
                y.ylabel,
                y.grid_x,
                y.grid_y
            ]
        );
        assert_eq!(x.grid_alpha.to_bits(), y.grid_alpha.to_bits());
        if a.legend {
            assert_eq!(
                a.legend_colors().resolved(aa),
                b.legend_colors().resolved(ba)
            );
        }
        for (x, y) in a.series.iter().zip(&b.series) {
            assert_eq!(aa.series_color(x), ba.series_color(y));
        }
    }
}
#[test]
fn four_dark_public_fig_snapshots_preserve_artwork_and_supplied_modes_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    for (case, script) in SCENES.iter().enumerate() {
        for explicit_theme in [false, true] {
            for print in [None, Some(false), Some(true)] {
                let mut engine = Engine::default();
                engine.execute(script).unwrap();
                let source = engine.figure.as_mut().unwrap();
                if explicit_theme {
                    source.set_figure_theme(FigureTheme::Dark);
                }
                source.invert_hardcopy = print;
                let source = &*source;
                let host = if explicit_theme {
                    BaseTheme::Light
                } else {
                    BaseTheme::Dark
                };
                let before = source.clone();
                let stamp = SnapshotStamp::new(source);
                let path = dir.path().join(format!("scene{case}.fig"));
                files::save_matlab_figure_with_theme(&path, source, host, &stop).unwrap();
                let mut loaded = files::load_matlab_figure(&path).unwrap();
                assert!(loaded.appearance.is_none(), "Never infer Theme from RGB");
                assert_eq!(loaded.invert_hardcopy, Some(print.unwrap_or(false)));
                same_samples(source, &loaded);
                same_artwork(source, host, &loaded);
                for ((_, a), (_, b)) in source.all_axes().zip(loaded.all_axes()) {
                    if let Some(grid) = a.colors.and_then(|c| c.grid) {
                        assert_eq!(b.colors.unwrap().grid, Some(grid));
                    }
                    if let Some(legend) = a.colors.and_then(|c| c.legend) {
                        assert_eq!(b.colors.unwrap().legend, Some(legend));
                    }
                    for (old, now) in a.series.iter().zip(&b.series) {
                        assert!(now.explicit_color);
                        assert_eq!(old.points.len(), now.points.len());
                    }
                }
                for _ in 0..2 {
                    let native = files::encode_native_figure(&loaded).unwrap();
                    let decoded = files::decode_native_figure(&native).unwrap();
                    assert_eq!(files::encode_native_figure(&decoded).unwrap(), native);
                    let script = files::matlab_plot_script(&loaded, &stop).unwrap();
                    let mut replay = Engine::default();
                    replay.execute(&script).unwrap();
                    let replay = replay.figure.as_ref().unwrap();
                    assert_eq!(replay.invert_hardcopy, loaded.invert_hardcopy);
                    for ((_, a), (_, b)) in loaded.all_axes().zip(replay.all_axes()) {
                        assert_eq!(a.colors, b.colors);
                    }
                    same_samples(&loaded, replay);
                    files::save_matlab_figure_with_theme(&path, &loaded, BaseTheme::Light, &stop)
                        .unwrap();
                    loaded = files::load_matlab_figure(&path).unwrap();
                    same_artwork(source, host, &loaded);
                }
                assert!(stamp.matches(source) && before.same_snapshot(source));
                for ((_, a), (_, b)) in before.all_axes().zip(source.all_axes()) {
                    for (a, b) in a.series.iter().zip(&b.series) {
                        assert!(a.points.same_storage(&b.points));
                    }
                }
            }
        }
    }
}

fn replace_json(record: &[u8], change: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let size = u32::from_le_bytes(record[8..12].try_into().unwrap()) as usize;
    let mut value = serde_json::from_slice(&record[12..12 + size]).unwrap();
    change(&mut value);
    let json = serde_json::to_vec(&value).unwrap();
    let mut result = record[..8].to_vec();
    result.extend_from_slice(&(json.len() as u32).to_le_bytes());
    result.extend_from_slice(&json);
    result.extend_from_slice(&record[12 + size..]);
    result
}
#[test]
fn x_y_capabilities_preserve_absence_and_preflight_all_child_styles_before_payloads() {
    let mut engine = Engine::default();
    engine.execute(SCENES[0]).unwrap();
    let mut source = engine.figure.unwrap();
    let original = files::encode_native_figure(&source).unwrap();
    assert!(original.starts_with(b"MATRFIGV"));
    source.invert_hardcopy = Some(true);
    let single = files::encode_native_figure(&source).unwrap();
    assert!(single.starts_with(b"MATRFIGX"));
    for header in [b"MATRFIGV", b"MATRFIGT", b"MATRFIGR"] {
        let mut bad = single.clone();
        bad[..8].copy_from_slice(header);
        assert!(
            files::decode_native_figure(&bad)
                .unwrap_err()
                .contains("MFIGX")
        );
    }
    source.invert_hardcopy = None;
    assert_eq!(files::encode_native_figure(&source).unwrap(), original);
    source.colors.get_or_insert_with(Default::default).legend = Some(LegendColors::default());
    let style = files::encode_native_figure(&source).unwrap();
    assert!(style.starts_with(b"MATRFIGX"));
    let mut bad = style.clone();
    bad[..8].copy_from_slice(b"MATRFIGV");
    assert!(
        files::decode_native_figure(&bad)
            .unwrap_err()
            .contains("MFIGX")
    );

    let mut engine = Engine::default();
    engine
        .execute("subplot(1,2,1);plot([0 1],[1 2]);subplot(1,2,2);plot([0 1],[3 4]);")
        .unwrap();
    let mut figure = engine.figure.unwrap();
    for axes in figure.all_axes_mut() {
        axes.colors.get_or_insert_with(Default::default).legend = Some(LegendColors {
            text: Some([0.1, 0.2, 0.3]),
            ..Default::default()
        });
    }
    figure.invert_hardcopy = Some(false);
    let bytes = files::encode_native_figure(&figure).unwrap();
    assert!(bytes.starts_with(b"MATRFIGY"));
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let start = 12 + size;
    let first_len = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
    let first = &bytes[start + 4..start + 4 + first_len];
    let second_start = start + 4 + first_len;
    let second_len =
        u32::from_le_bytes(bytes[second_start..second_start + 4].try_into().unwrap()) as usize;
    let second = &bytes[second_start + 4..second_start + 4 + second_len];
    let first_meta = u32::from_le_bytes(first[8..12].try_into().unwrap()) as usize;
    let missing_samples = &first[..12 + first_meta];
    for case in 0..3 {
        let mut invalid = replace_json(second, |v| match case {
            0 => v["axes_colors"]["legend"]["text"] = serde_json::json!([2., 0., 0.]),
            1 => v["invert_hardcopy"] = serde_json::json!(true),
            _ => {}
        });
        if case == 2 {
            invalid[..8].copy_from_slice(b"MATRFIGV");
        }
        let mut bad = bytes[..start].to_vec();
        for record in [missing_samples, invalid.as_slice()] {
            bad.extend_from_slice(&(record.len() as u32).to_le_bytes());
            bad.extend_from_slice(record);
        }
        let error = files::decode_native_figure(&bad).unwrap_err();
        assert!(
            error.contains(["finite RGB", "InvertHardcopy belongs", "MFIGY with MFIGX"][case]),
            "case{case}: {error}"
        );
    }
    let mut bad = bytes.clone();
    bad[..8].copy_from_slice(b"MATRFIGW");
    assert!(
        files::decode_native_figure(&bad)
            .unwrap_err()
            .contains("MFIGY")
    );
}

#[test]
fn bounded_public_print_and_legend_properties_are_transactional_and_keep_storage() {
    let mut engine = Engine::default();
    engine.execute(SCENES[0]).unwrap();
    engine.execute("before=get(f,'InvertHardcopy');set(f,'InvertHardcopy','off');set(lg,'Color',[0.1 0.2 0.3],'TextColor',[0.9 0.8 0.7],'EdgeColor',[-0 0.5 0.6]);bg=get(lg,'Color');ink=get(lg,'TextColor');edge=get(lg,'EdgeColor');").unwrap();
    let before = engine.figure.as_ref().unwrap().clone();
    let stamp = SnapshotStamp::new(&before);
    let crate::Array::Numeric(bg) = engine.array("bg").unwrap() else {
        panic!("numeric legend Color");
    };
    assert_eq!(bg.data(), &[0.1, 0.2, 0.3]);
    let crate::Array::Numeric(edge) = engine.array("edge").unwrap() else {
        panic!("numeric legend EdgeColor");
    };
    assert_eq!(edge.data()[0].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(
        before.colors.unwrap().legend.unwrap().border.unwrap()[0].to_bits(),
        (-0.0_f64).to_bits()
    );
    let crate::Array::Character(default) = engine.array("before").unwrap() else {
        panic!("character InvertHardcopy");
    };
    assert_eq!(default.to_utf8(&AtomicBool::new(false)).unwrap(), "on");
    assert_eq!(before.invert_hardcopy, Some(false));
    for command in [
        "set(f,'InvertHardcopy','on','Unknown',1);",
        "set(f,'InvertHardcopy',1);",
        "set(lg,'TextColor',[1 0 0],'EdgeColor',[2 0 0]);",
        "set(lg,'Color','none');",
    ] {
        assert!(engine.execute(command).is_err());
        assert!(stamp.matches(engine.figure.as_ref().unwrap()));
    }
    engine.execute("legend hide;").unwrap();
    assert_eq!(engine.figure.as_ref().unwrap().colors, before.colors);
    engine.execute("legend off;").unwrap();
    assert!(
        engine
            .figure
            .as_ref()
            .unwrap()
            .colors
            .unwrap()
            .legend
            .is_none()
    );
    engine
        .execute("clf;plot([0 1],[2 3]);tiledlayout(1,2);nexttile;plot([0 1],[1 2]);")
        .unwrap();
    assert_eq!(engine.figure.as_ref().unwrap().invert_hardcopy, Some(false));
    let mut implicit = Engine::default();
    implicit
        .execute("f=figure('InvertHardcopy','on');nexttile;plot([0 1],[1 2]);")
        .unwrap();
    assert_eq!(
        implicit.figure.as_ref().unwrap().invert_hardcopy,
        Some(true)
    );
}

#[test]
fn unsupported_legend_rgb_and_print_values_reject_without_silent_projection() {
    let mut engine = Engine::default();
    engine.execute(SCENES[0]).unwrap();
    let source = engine.figure.unwrap();
    let appearance = source.resolve_appearance(BaseTheme::Light);
    for case in 0..4 {
        let mut next = 10.;
        let mut plan = LegendEncoding {
            handle: 3.,
            members: Vec::new(),
        };
        let axes = encode_axes(
            &source.axes,
            2.,
            &mut next,
            None,
            appearance,
            Some(&mut plan),
        )
        .unwrap();
        let labels: Vec<_> = source.series.iter().map(|s| s.label.as_str()).collect();
        let mut properties = vec![
            ("Axes", num(&[2.])),
            ("String", mat::legend_text_cells(&labels).unwrap()),
            ("Location", mat::text("northeast")),
            ("Visible", mat::text("on")),
            ("Interpreter", mat::text("none")),
            ("Orientation", mat::text("vertical")),
            (
                "ApplicationData",
                mat::structure(
                    "",
                    vec![("PlotChildren", mat::numbers(plan.members.iter().copied()))],
                ),
            ),
        ];
        match case {
            0 => properties.push(("Color", mat::text("none"))),
            1 => properties.push(("TextColor", num(&[2., 0., 0.]))),
            2 => properties.push(("Box", mat::text("off"))),
            _ => {}
        }
        let legend = node("scribe.legend", 3., properties, vec![], &[]);
        let root = node(
            "figure",
            1.,
            if case == 3 {
                vec![("InvertHardcopy", mat::text("auto"))]
            } else {
                vec![]
            },
            vec![axes, legend],
            &[],
        );
        let bytes = mat::file(mat::structure("hgS_070000", root)).unwrap();
        let error = decode_matlab_figure(&bytes).unwrap_err();
        assert!(
            error.contains(["Color", "Color", "Box off", "InvertHardcopy"][case]),
            "case{case}: {error}"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("atomic.fig");
    std::fs::write(&path, b"old destination").unwrap();
    let mut invalid = source.clone();
    invalid.colors.get_or_insert_with(Default::default).legend = Some(LegendColors {
        text: Some([f64::NAN, 0., 0.]),
        ..Default::default()
    });
    assert!(files::save_matlab_figure(&path, &invalid).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"old destination");
    assert!(
        files::save_matlab_figure_with_theme(
            &path,
            &source,
            BaseTheme::Dark,
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"old destination");
}
