use matrust::{Engine, files, plot::LegendLocation};
use std::sync::atomic::AtomicBool;

#[test]
fn legend_location_scripts_preserve_samples_labels_and_other_axes() {
    let mut e = Engine::default();
    e.execute("subplot(1,2,1);plot([1 2],[3 4]);legend('Left','Location','southwest');subplot(1,2,2);plot([1 2],[5 6]);").unwrap();
    let points = e.figure.as_ref().unwrap().series[0].points.clone();
    for location in LegendLocation::ALL {
        e.execute(&format!(
            "legend('Right','lOcAtIoN','{}');",
            location.name().to_uppercase()
        ))
        .unwrap();
        let f = e.figure.as_ref().unwrap();
        assert_eq!(f.legend_location, location);
        assert_eq!(f.series[0].label, "Right");
        assert!((points).same_storage(&f.series[0].points));
        assert_eq!(
            f.layout.as_ref().unwrap().others[0].axes.legend_location,
            LegendLocation::Southwest
        );
    }
    e.execute("legend('Location','north','Location','south');legend off;")
        .unwrap();
    assert!(!e.figure.as_ref().unwrap().legend);
    assert_eq!(
        e.figure.as_ref().unwrap().legend_location,
        LegendLocation::South
    );
    e.execute("legend on;").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series[0].label, "Right");
}

#[test]
fn legend_location_invalid_calls_are_atomic() {
    let mut e = Engine::default();
    e.execute("plot([1 2],[3 4]);legend('Signal');").unwrap();
    let before = e.figure.clone().unwrap();
    for command in [
        "legend('Location','none')",
        "legend('Location','layout')",
        "legend('Location',1)",
        "legend('Location','wrong','Location','north')",
        "legend('One','Two','Location','south')",
    ] {
        assert!(e.execute(command).is_err(), "{command}");
        assert!(e.figure.as_ref().unwrap().same_snapshot(&before));
    }
    e.execute("legend('Location');").unwrap();
    assert_eq!(e.figure.as_ref().unwrap().series[0].label, "Location");
}

#[test]
fn legend_location_native_roundtrip_versions_and_rejects_downgrades() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legend.mfig");
    let mut e = Engine::default();
    e.execute("plot([1 2],[3 4],'Marker','square','MarkerSize',9,'LineWidth',2);legend('Signal');")
        .unwrap();
    // These are legacy presentation-only codec envelopes, with no process
    // legend object. J/K runtime-object persistence has separate regressions.
    e.figure.as_mut().unwrap().appearance = None; // Historical native capability.
    {
        let historical = e.figure.as_mut().unwrap();
        historical.axes.palette_cycle = None; // Empty layout axes are not in all_axes_mut().
        for axes in historical.all_axes_mut() {
            axes.palette_cycle = None;
            for series in &mut axes.series {
                series.auto_palette = None;
            }
        }
    }
    e.figure.as_mut().unwrap().legend_state = None;
    for location in LegendLocation::ALL {
        let f = e.figure.as_mut().unwrap();
        f.legend_location = location;
        files::save_figure(&path, f).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            &bytes[..8],
            if location.requires_extended_layout() {
                b"MATRFIGH"
            } else if location.is_default() {
                b"MATRFIGB"
            } else {
                b"MATRFIGD"
            }
        );
        let restored = files::load_figure(&path).unwrap();
        assert_eq!(restored.legend_location, location);
        assert_eq!(restored.svg(), f.svg());
        assert_eq!(restored.series[0].points, f.series[0].points);
        if !location.is_default() {
            bytes[..8].copy_from_slice(b"MATRFIGB");
            std::fs::write(&path, bytes).unwrap();
            assert!(files::load_figure(&path).unwrap_err().contains(
                if location.requires_extended_layout() {
                    "MFIGH"
                } else {
                    "MFIGD"
                }
            ));
        }
    }
    e.execute("subplot(1,2,1);plot([1 2]);legend('L','Location','southwest');subplot(1,2,2);plot([3 4]);legend('R','Location','north');").unwrap();
    for axes in e.figure.as_mut().unwrap().all_axes_mut() {
        axes.legend_state = None;
    }
    let f = e.figure.as_ref().unwrap();
    files::save_figure(&path, f).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"MATRFIGE"));
    assert_eq!(files::load_figure(&path).unwrap().svg(), f.svg());
    let mut nested = bytes.clone();
    let offset = 16 + u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    nested[offset..offset + 8].copy_from_slice(b"MATRFIGE");
    std::fs::write(&path, nested).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("Nested"));
    bytes[..8].copy_from_slice(b"MATRFIGC");
    std::fs::write(&path, bytes).unwrap();
    assert!(files::load_figure(&path).unwrap_err().contains("MFIGE"));
}

#[test]
fn legend_location_exports_anchor_box_and_text_and_preserve_source_property() {
    let mut e = Engine::default();
    e.execute("plot([1 2],[3 4]);legend('Signal');").unwrap();
    for (location, x, y) in [
        (LegendLocation::Northeast, 700, 70),
        (LegendLocation::Northwest, 80, 70),
        (LegendLocation::Southeast, 700, 446),
        (LegendLocation::Southwest, 80, 446),
        (LegendLocation::North, 390, 70),
        (LegendLocation::South, 390, 446),
        (LegendLocation::East, 700, 258),
        (LegendLocation::West, 80, 258),
    ] {
        let f = e.figure.as_mut().unwrap();
        f.legend_location = location;
        let svg = f.svg();
        assert!(
            svg.contains(&format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"220\" height=\"34\""
            )),
            "{location:?}"
        );
        assert!(svg.contains(&format!(
            "<text x=\"{}\" y=\"{}\" fill=\"#242424\">Signal</text>",
            x + 42,
            y + 24
        )));
        let script = files::matlab_plot_script(f, &AtomicBool::new(false)).unwrap();
        assert!(script.contains(&format!("'Location','{}'", location.name())));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protected.fig");
    std::fs::write(&path, b"original").unwrap();
    let f = e.figure.as_mut().unwrap();
    f.legend = false;
    let script = files::matlab_plot_script(f, &AtomicBool::new(false)).unwrap();
    assert!(script.contains("'Location','west','Visible','off'"));
    assert!(!f.svg().contains("stroke=\"#aaa\""));
    assert!(files::save_figure(&path, f).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"original");
}
