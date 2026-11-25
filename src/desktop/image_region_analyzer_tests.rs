// Image Region Analyzer: region properties against scikit-image
// (tool/refs/image_region_analyzer/reference.py), filters, click selection,
// generated function, export names and headless rendering.
//
// Convention notes (see the reference script): centroids are 1-based [x y];
// axis lengths and eccentricity include the 1/12 pixel variance skimage omits;
// orientation is in degrees from the x axis with y up; Perimeter is the chain
// length of the outer boundary trace (skimage's perimeter is a weighted pixel
// count, and the ring's hole boundary is not part of the perimeter).
mod image_region_analyzer_tests {
    use super::*;
    use apps::image_region_analyzer::{ImageRegionAnalyzer, Property, Source};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/image_region_analyzer/reference.json"
        ))
        .unwrap()
    }
    fn workspace(code: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(code).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    /// `name = mask` from a reference case (size and 1-based true indices).
    fn mask_code(name: &str, case: &serde_json::Value) -> String {
        let size = case["size"].as_array().unwrap();
        let idx: Vec<String> = case["true"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap().to_string())
            .collect();
        // Short statements: the parser bounds expression size.
        let mut code = format!("{name} = false({}, {});\n", size[0], size[1]);
        for chunk in idx.chunks(64) {
            code.push_str(&format!("{name}([{}]) = true;\n", chunk.join(" ")));
        }
        code
    }
    fn compute(
        host: &mut Host<ImageRegionAnalyzer>,
        ws: &Snapshot,
    ) -> Result<apps::Outputs, String> {
        host.app.prepare(ws);
        let outputs = apps::run(&mut matrust::Engine::default(), host.request(ws)?)?;
        host.app.receive(&outputs)?;
        Ok(outputs)
    }
    fn analyzer(variable: &str) -> Host<ImageRegionAnalyzer> {
        let mut host = Host::new(ImageRegionAnalyzer::default());
        host.app.variable = variable.into();
        host.app.shown = [true; 10];
        host
    }
    fn areas(host: &Host<ImageRegionAnalyzer>) -> Vec<f64> {
        let a = host.app.current.as_ref().unwrap();
        (0..a.count)
            .map(|r| a.value(Property::Area, r, 0).unwrap())
            .collect()
    }
    fn values(engine: &matrust::Engine, expr: &str) -> Vec<f64> {
        let mut e = engine.clone();
        e.execute(&format!("test_value = double({expr});")).unwrap();
        match e.array("test_value").unwrap() {
            matrust::Array::Numeric(m) => m.data().to_vec(),
            a => panic!("{expr}: {}", a.class()),
        }
    }
    fn close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: {actual:.15} != {expected:.15}"
        );
    }
    fn export_code(host: &mut Host<ImageRegionAnalyzer>, ws: &Snapshot) -> String {
        match host.export(ws).unwrap() {
            toolkits::Action::ExportScript(s) => s,
            _ => panic!("wrong export"),
        }
    }
    fn wait(host: &mut Host<ImageRegionAnalyzer>, ctx: &egui::Context, ws: &Snapshot) {
        let start = Instant::now();
        loop {
            host.refresh(ctx, ws);
            if !host.running() {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(120), "run timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn image_region_analyzer_properties_match_skimage_on_fixed_masks() {
        let r = reference();
        let shapes = &r["shapes"];
        let ws = workspace(&mask_code("shapes", shapes));
        let mut host = analyzer("shapes");
        compute(&mut host, &ws).unwrap();
        let a = host.app.current.as_ref().unwrap();
        let expected = shapes["props8"].as_array().unwrap();
        assert_eq!(a.count, expected.len());
        assert_eq!(a.count, shapes["n8"].as_u64().unwrap() as usize);
        for (k, e) in expected.iter().enumerate() {
            let value = |p: Property, part: usize| a.value(p, k, part).unwrap();
            let num = |name: &str| e[name].as_f64().unwrap();
            let what = |name: &str| format!("region {} {name}", k + 1);
            for (p, tolerance) in [
                (Property::Area, 0.),
                (Property::FilledArea, 0.),
                (Property::Perimeter, 1e-9),
                (Property::EquivDiameter, 1e-12),
                (Property::Extent, 1e-12),
                (Property::MajorAxisLength, 1e-9),
                (Property::MinorAxisLength, 1e-9),
                (Property::Eccentricity, 1e-6),
            ] {
                close(value(p, 0), num(p.name()), tolerance, &what(p.name()));
            }
            let c = e["Centroid"].as_array().unwrap();
            close(
                value(Property::Centroid, 0),
                c[0].as_f64().unwrap(),
                1e-12,
                &what("x"),
            );
            close(
                value(Property::Centroid, 1),
                c[1].as_f64().unwrap(),
                1e-12,
                &what("y"),
            );
            // Orientation is only defined for elongated regions.
            if num("Eccentricity") > 0.1 {
                close(
                    value(Property::Orientation, 0),
                    num("Orientation"),
                    1e-6,
                    &what("Orientation"),
                );
            }
        }
        // The L shape has a real orientation; the ring's hole is filled.
        close(
            a.value(Property::Orientation, 2, 0).unwrap(),
            -57.25,
            0.01,
            "L",
        );
        assert_eq!(a.value(Property::FilledArea, 4, 0).unwrap(), 197.);
        // Every pixel's region matches the reference labelling.
        let labels = shapes["labels8"].as_array().unwrap();
        for (i, l) in labels.iter().enumerate() {
            let (row, col) = (i % a.rows, i / a.rows);
            let l = l.as_u64().unwrap() as usize;
            assert_eq!(a.region_at(row, col), l.checked_sub(1), "pixel {i}");
        }
        // Unsupported properties are listed, never requested.
        let script = host.app.script(&ws).unwrap();
        for name in apps::image_region_analyzer::UNAVAILABLE {
            assert!(!script.contains(name));
        }
    }

    #[test]
    fn image_region_analyzer_connectivity_4_vs_8() {
        let r = reference();
        let touch = &r["touch"];
        let ws = workspace(&mask_code("touch", touch));
        let mut host = analyzer("touch");
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), [18.]);
        host.app.connectivity = 4;
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), [9., 9.]);
        assert_eq!(touch["n4"].as_u64(), Some(2));
        assert_eq!(touch["n8"].as_u64(), Some(1));
        // The label image follows the same connectivity.
        let a = host.app.current.as_ref().unwrap();
        assert_eq!(a.region_at(1, 1), Some(0));
        assert_eq!(a.region_at(5, 5), Some(1));
        assert_eq!(a.region_at(0, 0), None);
    }

    #[test]
    fn image_region_analyzer_filters_fill_and_clear_border() {
        let r = reference();
        let shapes = &r["shapes"];
        let f = &r["filters"];
        let all: Vec<f64> = shapes["props8"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["Area"].as_f64().unwrap())
            .collect();
        let kept = |key: &str| -> Vec<f64> {
            f[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| all[l.as_u64().unwrap() as usize - 1])
                .collect()
        };
        let ws = workspace(&mask_code("shapes", shapes));
        let mut host = analyzer("shapes");
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), all);
        // Area in [20, 120].
        let range = f["area_range"].as_array().unwrap();
        let area = host.app.filter_mut(Property::Area).unwrap();
        area.enabled = true;
        area.min = range[0].as_f64().unwrap().to_string();
        area.max = range[1].as_f64().unwrap().to_string();
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), kept("kept_area_range"));
        // Then keep the single largest.
        host.app.keep_largest = true;
        host.app.keep_count = "1".into();
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), kept("kept_area_then_largest1"));
        // Keep the 2 largest alone.
        host.app.filter_mut(Property::Area).unwrap().enabled = false;
        host.app.keep_count = "2".into();
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), kept("kept_largest2"));
        // Eccentricity <= 0.5 with an open lower bound.
        host.app.keep_largest = false;
        let e = host.app.filter_mut(Property::Eccentricity).unwrap();
        e.enabled = true;
        e.min = String::new();
        e.max = f["eccentricity_max"].as_f64().unwrap().to_string();
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), kept("kept_eccentricity"));
        host.app.filter_mut(Property::Eccentricity).unwrap().enabled = false;
        // Fill holes: the ring becomes its filled disc (skimage area_filled).
        host.app.fill_holes = true;
        compute(&mut host, &ws).unwrap();
        let filled = areas(&host);
        assert_eq!(
            filled[4],
            shapes["props8"][4]["FilledArea"].as_f64().unwrap()
        );
        assert_eq!(&filled[..4], &all[..4]);
        // Clear border removes the region touching the edge.
        let ws = workspace(
            "edge = false(6, 7); edge(1, 1:2) = true; edge(3:4, 3:4) = true; edge(6, 7) = true;",
        );
        let mut host = analyzer("edge");
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), [2., 4., 1.]);
        host.app.clear_border = true;
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), [4.]);
        // Removed pixels are grey in the view.
        let a = host.app.current.as_ref().unwrap();
        assert_eq!(a.region_at(0, 0), None);
    }

    #[test]
    fn image_region_analyzer_click_selects_row_and_outline() {
        let r = reference();
        let shapes = &r["shapes"];
        let ws = workspace(&mask_code("shapes", shapes));
        let mut host = analyzer("shapes");
        compute(&mut host, &ws).unwrap();
        // The 40x64 image drawn at 5 px per pixel.
        let rect = egui::Rect::from_min_size(egui::pos2(10., 20.), egui::vec2(320., 200.));
        let at = |row: f32, col: f32| egui::pos2(10. + (col + 0.5) * 5., 20. + (row + 0.5) * 5.);
        // Disc centre: row 12, column 33 (0-based) is region 4.
        assert_eq!(host.app.select_at(rect, at(12., 33.)), Some(3));
        assert_eq!(host.app.selected_row(), Some(3));
        // Sorted by Area descending, the disc (113) is the second row.
        host.app.toggle_sort(Property::Area, 0);
        host.app.toggle_sort(Property::Area, 0);
        assert_eq!(host.app.sort, Some((Property::Area, 0, true)));
        assert_eq!(host.app.sorted_rows(), [4, 3, 2, 1, 0]);
        assert_eq!(host.app.selected_row(), Some(1));
        // The L's foot (row 18, column 20) is region 3, sorted row 2.
        assert_eq!(host.app.select_at(rect, at(18., 20.)), Some(2));
        assert_eq!(host.app.selected_row(), Some(2));
        // Sort by centroid y ascending: squares at y 4 then the L at 16.2, ...
        host.app.toggle_sort(Property::Centroid, 1);
        assert_eq!(host.app.sorted_rows(), [0, 3, 2, 4, 1]);
        host.app.toggle_sort(Property::Centroid, 1);
        host.app.toggle_sort(Property::Centroid, 1);
        assert_eq!(host.app.sort, None);
        // Background and outside clicks.
        assert_eq!(host.app.select_at(rect, at(0., 0.)), None);
        assert_eq!(host.app.selected, None);
        assert_eq!(host.app.select_at(rect, egui::pos2(0., 0.)), None);
        // The 3x3 square's outline: its 8 border pixel centres (x, y 1-based).
        host.app.select_at(rect, at(3., 3.));
        assert_eq!(host.app.selected, Some(0));
        let request = host.app.outline_request().unwrap();
        let key = request.key;
        let outputs = apps::run(&mut matrust::Engine::default(), request).unwrap();
        host.app.receive_outline(key, &outputs).unwrap();
        let outline = host.app.outline().unwrap();
        let mut points: Vec<(i64, i64)> = outline
            .iter()
            .map(|[x, y]| (*x as i64, *y as i64))
            .collect();
        points.sort();
        points.dedup();
        assert_eq!(points.len(), 8);
        assert!(
            points
                .iter()
                .all(|&(x, y)| (3..=5).contains(&x) && (3..=5).contains(&y))
        );
        assert!(!points.contains(&(4, 4)));
        // A new selection drops the outline until traced.
        host.app.selected = Some(1);
        assert!(host.app.outline().is_none());
    }

    #[test]
    fn image_region_analyzer_generated_function_matches_fresh_engine() {
        let r = reference();
        let shapes = &r["shapes"];
        let ws = workspace(&mask_code("shapes", shapes));
        let mut host = analyzer("shapes");
        host.app.connectivity = 4;
        host.app.fill_holes = true;
        let area = host.app.filter_mut(Property::Area).unwrap();
        area.enabled = true;
        area.min = "10".into();
        host.app.keep_largest = true;
        host.app.keep_count = "3".into();
        compute(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("function [BW_out, properties] = filterRegions(BW_in)"));
        assert!(script.contains("bwpropfilt(BW_out, 'Area', [10 Inf], conn)"));
        assert!(script.contains("bwareafilt(BW_out, 3, conn)"));
        let saved = match host.generate(&ws, 42).unwrap() {
            toolkits::Action::SaveAppScript(AppKind::ImageRegionAnalyzer, text) => text,
            _ => panic!("wrong save action"),
        };
        assert!(saved.ends_with(&script));
        let mut fresh = matrust::Engine::default();
        for (name, a) in ws.variables() {
            fresh.insert_array(name, a.clone()).unwrap();
        }
        fresh.execute(&saved).unwrap();
        let a = host.app.current.as_ref().unwrap();
        assert_eq!(a.count, 3);
        let bw = values(&fresh, "BW_out");
        assert_eq!(bw.len(), a.rows * a.cols);
        for (i, v) in bw.iter().enumerate() {
            let inside = a.region_at(i % a.rows, i / a.rows).is_some();
            assert_eq!(*v == 1., inside, "pixel {i}");
        }
        assert_eq!(values(&fresh, "size(properties)"), [3., 10.]);
        for (p, data) in &a.columns {
            assert_eq!(&values(&fresh, &format!("properties.{}", p.name())), data);
        }
        // The function alone, on the mask, in a new engine.
        let mut alone = matrust::Engine::default();
        alone
            .execute(&format!(
                "{}[B, T] = filterRegions(shapes);\n{}",
                mask_code("shapes", shapes),
                apps::split_local_functions(&script).1
            ))
            .unwrap();
        assert_eq!(values(&alone, "B"), bw);
        assert_eq!(
            values(&alone, "T.Area"),
            a.columns
                .iter()
                .find(|(p, _)| *p == Property::Area)
                .unwrap()
                .1
        );
        // Export writes both names; the probe's aliases read them back.
        let code = export_code(&mut host, &ws);
        let mut engine = matrust::Engine::default();
        for (name, a) in ws.variables() {
            engine.insert_array(name, a.clone()).unwrap();
        }
        engine.execute(&code).unwrap();
        assert_eq!(values(&engine, "BW_out"), bw);
        assert_eq!(engine.array("BW_out").unwrap().class(), "logical");
        assert_eq!(engine.array("properties").unwrap().class(), "table");
    }

    #[test]
    fn image_region_analyzer_export_names_validation_and_last_good_result() {
        let r = reference();
        let ws = workspace(&format!(
            "{}gray = uint8(magic(6) * 7); rgb = cat(3, gray, gray, gray); \
             filterRegions = 5; ira_bw = 7; cube = true(2, 2, 2); z = complex(1, 1);",
            mask_code("shapes", &r["shapes"])
        ));
        let mut host = analyzer("shapes");
        compute(&mut host, &ws).unwrap();
        let before = areas(&host);
        // Names that clash with workspace variables get fresh helper names.
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("function [BW_out, properties] = filterRegions1(BW_in)"));
        assert!(script.contains("ira_bw1 = shapes;"));
        for name in [
            "mrexp_old",
            "mrexp_had",
            "matrust_export_error",
            "mrexp_complete",
            "1bad",
            "for",
            "x);bad=1;%",
            "",
        ] {
            host.app.bw_name = name.into();
            assert!(host.app.script(&ws).is_err(), "{name:?}");
            host.app.bw_name = "BW_out".into();
            host.app.table_name = name.into();
            assert!(host.app.script(&ws).is_err(), "{name:?}");
            host.app.table_name = "properties".into();
        }
        host.app.table_name = "BW_out".into();
        assert!(host.app.script(&ws).unwrap_err().contains("duplicate"));
        host.app.table_name = "stats".into();
        host.app.bw_name = "shapes".into();
        assert_eq!(host.app.exported(&ws), ["shapes", "stats"]);
        // Bad filter bounds and counts never reach a script.
        let area = host.app.filter_mut(Property::Area).unwrap();
        area.enabled = true;
        for (min, max) in [
            ("1); error('x'); %", ""),
            ("NaN", ""),
            ("2^3", ""),
            ("10", "5"),
            ("", "1\nerror('boom')"),
        ] {
            let area = host.app.filter_mut(Property::Area).unwrap();
            area.min = min.into();
            area.max = max.into();
            assert!(host.request(&ws).is_err(), "{min:?} {max:?}");
            assert_eq!(areas(&host), before);
        }
        let area = host.app.filter_mut(Property::Area).unwrap();
        area.min = "-Inf".into();
        area.max = "Inf".into();
        assert!(host.request(&ws).is_ok());
        host.app.keep_largest = true;
        for bad in ["0", "1.5", "65537", "x", "2); error('x"] {
            host.app.keep_count = bad.into();
            assert!(host.request(&ws).is_err(), "{bad}");
        }
        host.app.keep_largest = false;
        host.app.shown = [false; 10];
        assert!(host.request(&ws).err().unwrap().contains("column"));
        host.app.shown = [true; 10];
        // Non-logical images: an error naming Binarize, unless it is ticked.
        for (v, class) in [("gray", "uint8"), ("rgb", "uint8 RGB")] {
            host.app.variable = v.into();
            let e = host.request(&ws).err().unwrap();
            assert!(e.contains(class) && e.contains("Binarize"), "{e}");
        }
        for bad in ["cube", "z", "missing", "shapes); error('x'); %"] {
            host.app.variable = bad.into();
            assert!(host.request(&ws).is_err(), "{bad}");
        }
        host.app.binarize = true;
        host.app.variable = "rgb".into();
        compute(&mut host, &ws).unwrap();
        // magic(6)*7 thresholded by Otsu: 18 values above, as one region set.
        let otsu = values(
            &{
                let mut e = matrust::Engine::default();
                e.execute("m = imbinarize(uint8(magic(6) * 7));").unwrap();
                e
            },
            "sum(m(:))",
        );
        assert_eq!(areas(&host).iter().sum::<f64>(), otsu[0]);
        // A run error (an image that went away) keeps the last result and
        // reaches the status line.
        let kept = areas(&host);
        host.app.source = Source::File;
        host.app.file = "no_such_image_file.png".into();
        let ctx = egui::Context::default();
        wait(&mut host, &ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert_eq!(areas(&host), kept);
        host.app.file = "a\nb.png".into();
        assert!(host.request(&ws).is_err());
        // Export rolls both names back when it fails.
        host.app.source = Source::Workspace;
        host.app.bw_name = "BW_out".into();
        let code = export_code(&mut host, &ws);
        let mut engine = matrust::Engine::default();
        engine
            .execute("rgb = 'not an image'; BW_out = 1; stats = 2;")
            .unwrap();
        assert!(engine.execute(&code).is_err());
        assert_eq!(values(&engine, "BW_out"), [1.]);
        assert_eq!(values(&engine, "stats"), [2.]);
    }

    #[test]
    fn image_region_analyzer_reads_image_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("it's a mask.png");
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "I = zeros(20, 30, 'uint8'); I(3:6, 4:9) = 255; I(12:18, 15:20) = 200; imwrite(I, '{}');",
                path.to_string_lossy().replace('\'', "''")
            ))
            .unwrap();
        let ws = Snapshot::default();
        let mut host = Host::new(ImageRegionAnalyzer::default());
        host.app.source = Source::File;
        host.app.file = path.to_string_lossy().into_owned();
        // A grey PNG needs Binarize.
        let e = compute(&mut host, &ws).unwrap_err();
        assert!(e.contains("Binarize"), "{e}");
        host.app.binarize = true;
        compute(&mut host, &ws).unwrap();
        assert_eq!(areas(&host), [24., 42.]);
        assert!(host.app.script(&ws).unwrap().contains("imread("));
    }

    #[test]
    fn image_region_analyzer_tool_tab_and_headless_render_0_1_500() {
        let kind = AppKind::ImageRegionAnalyzer;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(
            serde_json::to_string(&tab).unwrap(),
            "\"image_region_analyzer\""
        );
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"image_region_analyzer\"").unwrap();
        assert_eq!(decoded, tab);
        assert!(kind.is_built() && !kind.detail().is_empty());
        let ws = workspace(
            "none = false(30, 40); one = none; one(5:9, 6:12) = true; \
             many = false(80, 100); many(2:4:end, 2:4:end) = true;",
        );
        for (name, count) in [("none", 0), ("one", 1), ("many", 500)] {
            for width in [820., 1600.] {
                let ctx = egui::Context::default();
                let mut desktop = Workspace::with_context(ctx.clone(), true);
                desktop.toolkits.open_tool(Tool::App(kind));
                let host = &mut desktop.toolkits.apps.region;
                host.app.variable = name.into();
                compute(host, &ws).unwrap();
                assert_eq!(host.app.current.as_ref().unwrap().count, count);
                if count > 0 {
                    host.app.selected = Some(count - 1);
                    host.app.sort = Some((Property::Area, 0, true));
                }
                for _ in 0..2 {
                    let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                    assert!(!output.shapes.is_empty());
                }
                assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
                desktop.close_tool_tab(Tool::App(kind));
            }
        }
    }

    fn large_rgb(rows: usize, cols: usize) -> Snapshot {
        // Same memory guards as the desktop Runner (`Limits::local()`).
        let mut engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
        engine
            .execute(&format!(
                "[X, Y] = meshgrid(1:{cols}, 1:{rows}); \
                 G = uint8(40 + 180 * (sin(X / 15) .* sin(Y / 11) > 0.5)); \
                 big = cat(3, G, G, 255 - G); clear X Y G",
            ))
            .unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn large_rgb_stays_responsive(rows: usize, cols: usize) {
        let ws = large_rgb(rows, cols);
        let mut host = Host::new(ImageRegionAnalyzer::default());
        host.app.variable = "big".into();
        host.app.binarize = true;
        host.app.clear_border = true;
        let ctx = egui::Context::default();
        // The UI thread only submits: the frame returns at once.
        let start = Instant::now();
        host.refresh(&ctx, &ws);
        let submit = start.elapsed();
        assert!(host.running());
        assert!(submit < Duration::from_millis(200), "{submit:?}");
        wait(&mut host, &ctx, &ws);
        let elapsed = start.elapsed();
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{:?}",
            host.status()
        );
        let a = host.app.current.as_ref().unwrap();
        assert_eq!((a.rows, a.cols), (rows, cols));
        let step = rows
            .max(cols)
            .div_ceil(apps::image_region_analyzer::DISPLAY_MAX);
        assert_eq!(a.step, step);
        assert!(a.count > 0);
        eprintln!(
            "image_region_analyzer {rows}x{cols} RGB binarize+clear border+regionprops: {elapsed:?} ({} regions)",
            a.count
        );
        // Stop cancels a running analysis and keeps the last result.
        let count = a.count;
        host.app.connectivity = 4;
        host.refresh(&ctx, &ws);
        assert!(host.running());
        host.app.stop();
        let stopping = Instant::now();
        wait(&mut host, &ctx, &ws);
        assert!(stopping.elapsed() < Duration::from_secs(5));
        assert!(host.status().0.contains("stopped"), "{:?}", host.status());
        assert_eq!(host.app.current.as_ref().unwrap().count, count);
        // Run computes again.
        host.app.run();
        wait(&mut host, &ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Success);
    }

    #[test]
    fn image_region_analyzer_large_rgb_1000x750_runs_off_thread_with_stop() {
        large_rgb_stays_responsive(750, 1000);
    }

    #[test]
    fn image_region_analyzer_large_rgb_4000x3000_runs_off_thread_with_stop() {
        large_rgb_stays_responsive(3000, 4000);
    }
}
