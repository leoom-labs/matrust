// Image Batch Processor: folder runs against NumPy references
// (tool/refs/image_batch_processor/reference.py), Stop, re-run skipping,
// results table, generated script reproduction, overwrite guard, the
// handle-call engine fix and headless rendering.
mod image_batch_processor_tests {
    use super::*;
    use apps::image_batch_processor::{
        Function, ImageBatchProcessor, Returns, Status, contained_target,
    };
    use apps::{AppKind, Snapshot};
    use std::path::Path;
    use toolkits::Tool;

    fn reference() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/apps/image_batch_processor/reference.json"
        ))
        .unwrap()
    }
    fn path(p: &Path) -> String {
        p.to_string_lossy().replace('\'', "''")
    }
    /// The reference script's image k (1-based rows and columns).
    fn rgb_code(k: usize) -> String {
        let (rows, cols) = (5 + k, 7 + k);
        format!(
            "[c, r] = meshgrid(1:{cols}, 1:{rows}); \
             rgb = uint8(cat(3, mod(37*{k} + 11*r + 7*c, 256), mod(13*{k} + 5*r.*c, 256), \
             mod(200 - 9*r + 3*c*{k}, 256)));"
        )
    }
    /// Five reference PNGs plus a corrupt file named to sort last.
    fn images(dir: &Path, corrupt: bool) {
        let mut engine = matrust::Engine::default();
        for k in 1..=5 {
            engine
                .execute(&format!(
                    "{} imwrite(rgb, '{}');",
                    rgb_code(k),
                    path(&dir.join(format!("img{k}.png")))
                ))
                .unwrap();
        }
        if corrupt {
            std::fs::write(dir.join("zz_corrupt.png"), b"not a png file").unwrap();
        }
    }
    fn batch(input: &Path, output: &Path) -> ImageBatchProcessor {
        let mut app = ImageBatchProcessor::default();
        app.input = input.to_string_lossy().into_owned();
        app.output = output.to_string_lossy().into_owned();
        app
    }
    fn until(
        app: &mut ImageBatchProcessor,
        ctx: &egui::Context,
        ws: &Snapshot,
        done: impl Fn(&ImageBatchProcessor) -> bool,
    ) {
        let start = Instant::now();
        loop {
            app.poll(ctx, ws);
            if done(app) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(120),
                "timed out: {:?}",
                app.status()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn listed(app: &mut ImageBatchProcessor, ctx: &egui::Context, ws: &Snapshot, n: usize) {
        until(app, ctx, ws, |a| a.files.len() == n && !a.listing_running());
    }
    fn run_all(app: &mut ImageBatchProcessor, ctx: &egui::Context, ws: &Snapshot) {
        app.run_all();
        until(app, ctx, ws, |a| !a.running());
    }
    fn uint8(engine: &mut matrust::Engine, code: &str) -> (Vec<usize>, Vec<u8>) {
        engine.execute(&format!("test_value = {code};")).unwrap();
        match engine.array("test_value").unwrap() {
            matrust::Array::Typed(t) => match t.data() {
                matrust::typed_array::TypedData::Uint8(d) => {
                    (t.dims().to_vec(), d.as_slice().to_vec())
                }
                _ => panic!("{code}: not uint8"),
            },
            a => panic!("{code}: {}", a.class()),
        }
    }
    fn gray_of(case: &serde_json::Value) -> (Vec<usize>, Vec<u8>) {
        let size = case["size"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        let gray = case["gray"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        (size, gray)
    }
    /// PNG bytes without the tIME chunk (imwrite stamps the write time);
    /// other files unchanged.
    fn timeless(bytes: Vec<u8>) -> Vec<u8> {
        const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
        if !bytes.starts_with(SIGNATURE) {
            return bytes;
        }
        let mut out = SIGNATURE.to_vec();
        let mut at = SIGNATURE.len();
        while at + 12 <= bytes.len() {
            let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let end = at + 12 + length;
            assert!(end <= bytes.len(), "truncated PNG chunk");
            if &bytes[at + 4..at + 8] != b"tIME" {
                out.extend_from_slice(&bytes[at..end]);
            }
            at = end;
        }
        assert_eq!(at, bytes.len());
        out
    }
    /// Every file under `dir`, relative path to bytes (PNG write time removed).
    fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                    out.insert(rel, timeless(std::fs::read(&p).unwrap()));
                }
            }
        }
        out
    }
    fn table_of(engine: &matrust::Engine, name: &str) -> (Vec<String>, Vec<String>, Vec<Vec<f64>>) {
        let Some(matrust::Array::Table(t)) = engine.array(name) else {
            panic!("{name} is not a table");
        };
        let files = match t.column(0) {
            matrust::Array::Cell(c) => (0..c.len())
                .map(|i| match c.get_linear(i).unwrap().as_ref() {
                    matrust::Array::Character(s) => String::from_utf16_lossy(s.units()),
                    _ => panic!("file name"),
                })
                .collect(),
            other => panic!("File column: {}", other.class()),
        };
        let columns = t.columns()[1..]
            .iter()
            .map(|c| match c {
                matrust::Array::Numeric(m) => m.data().to_vec(),
                other => panic!("column {}", other.class()),
            })
            .collect();
        (t.names().to_vec(), files, columns)
    }
    fn close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= 1e-12 * expected.abs().max(1.),
            "{what}: {actual} != {expected}"
        );
    }
    const STATS: &str = "function s = stats_fn(img)\n\
                         s.MeanValue = mean(double(img(:)));\n\
                         s.MaxValue = double(max(img(:)));\n\
                         end\n";
    const BOTH: &str = "function [out, s] = both_fn(img)\n\
                        out = rgb2gray(img);\n\
                        s.MeanValue = mean(double(img(:)));\n\
                        s.MaxValue = double(max(img(:)));\n\
                        end\n";

    #[test]
    fn image_batch_processor_gray_template_matches_rgb2gray_and_corrupt_file_is_an_error_row() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in"), dir.path().join("out"));
        std::fs::create_dir(&input).unwrap();
        images(&input, true);
        let ws = Snapshot::default();
        let ctx = egui::Context::default();
        let mut app = batch(&input, &output);
        listed(&mut app, &ctx, &ws, 6);
        assert_eq!(app.files[5].name, "zz_corrupt.png");
        run_all(&mut app, &ctx, &ws);
        assert_eq!(app.counts(), (5, 1, 0), "{:?}", app.status());
        let Status::Error(message) = &app.files[5].status else {
            panic!(
                "the corrupt file is an error row: {:?}",
                app.files[5].status
            );
        };
        assert!(!message.is_empty());
        assert_eq!(app.status().1, toolkits::AppStatus::Error);
        assert!(app.files.iter().all(|f| f.seconds.is_some()));
        // Each output equals rgb2gray (published weights, NumPy) of its input.
        let mut engine = matrust::Engine::default();
        for case in reference()["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let got = uint8(
                &mut engine,
                &format!("imread('{}')", path(&output.join(name))),
            );
            assert_eq!(got, gray_of(case), "{name}");
        }
        assert!(output.join("batch_to_gray.m").is_file());
        assert!(!output.join("zz_corrupt.png").exists());
        // A re-run skips the ok files; forced, it runs them again.
        run_all(&mut app, &ctx, &ws);
        assert_eq!((app.skipped, app.counts()), (5, (5, 1, 0)));
        app.force = true;
        run_all(&mut app, &ctx, &ws);
        assert_eq!((app.skipped, app.counts()), (0, (5, 1, 0)));
        // Thumbnails of the selected file: its input and the written result.
        app.selected = Some(1);
        until(&mut app, &ctx, &ws, |a| {
            !a.thumbs_running()
                && a.thumbnails()
                    .is_some_and(|(i, o)| i.size == [7, 9] && o.is_some())
        });
        let (input_thumb, output_thumb) = app.thumbnails().unwrap();
        let output_thumb = output_thumb.unwrap();
        let (_, gray) = gray_of(&reference()["cases"][1]);
        assert_eq!(output_thumb.size, [7, 9]);
        // Row-major RGB of a gray image: pixel (row 2, col 3) is gray(2 + 3*7).
        assert_eq!(output_thumb.rgb[(2 * 9 + 3) * 3], gray[2 + 3 * 7]);
        assert_eq!(input_thumb.rgb.len(), 7 * 9 * 3);
        // Run selected re-runs one file even when it is ok.
        app.force = false;
        app.selected = Some(5);
        app.run_selected();
        until(&mut app, &ctx, &ws, |a| !a.running());
        assert_eq!(app.counts(), (5, 1, 0));
    }

    #[test]
    fn image_batch_processor_stop_after_two_files_leaves_the_rest_pending() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in"), dir.path().join("out"));
        std::fs::create_dir(&input).unwrap();
        images(&input, true);
        let ws = Snapshot::default();
        let ctx = egui::Context::default();
        let mut app = batch(&input, &output);
        listed(&mut app, &ctx, &ws, 6);
        app.run_all();
        // The first file delivered starts the second: Stop cancels that file.
        until(&mut app, &ctx, &ws, |a| a.counts().0 == 1);
        assert!(app.running());
        app.stop();
        until(&mut app, &ctx, &ws, |a| !a.running() && !a.file_running());
        assert!(app.stopped);
        assert!(app.status().0.contains("stopped"), "{:?}", app.status());
        assert_eq!(app.files[0].status, Status::Ok);
        assert_eq!(app.files[1].status, Status::Stopped);
        assert!(app.files[2..].iter().all(|f| f.status == Status::Pending));
        assert!(output.join("img1.png").is_file());
        assert!(!output.join("img3.png").exists());
        // Run all continues with the files that are not already ok.
        run_all(&mut app, &ctx, &ws);
        assert_eq!((app.skipped, app.counts()), (1, (5, 1, 0)));
    }

    #[test]
    fn image_batch_processor_review_b6_containment_rejects_dotdot_and_accepts_missing_tail() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("out");
        std::fs::create_dir(&root).unwrap();
        let escaped = contained_target(&root, Path::new("sub/../../outside.png"), &[], false);
        let message = escaped.unwrap_err();
        assert!(message.contains("leaves the output folder"), "{message}");
        assert!(!dir.path().join("outside.png").exists());
        // `..` stays inside, and the non-existing tail is not created as a file.
        let got = contained_target(&root, Path::new("sub/../new/file.png"), &[], false).unwrap();
        let root = root.canonicalize().unwrap();
        assert_eq!(got, root.join("new").join("file.png"));
        assert!(root.join("new").is_dir());
        assert!(!got.exists());
    }

    #[cfg(unix)]
    #[test]
    fn image_batch_processor_review_b6_symlink_subdir_is_an_error_and_batch_continues() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in"), dir.path().join("out"));
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(input.join("sub")).unwrap();
        std::fs::create_dir(&output).unwrap();
        std::fs::create_dir(&outside).unwrap();
        images(&input, false);
        std::fs::copy(input.join("img1.png"), input.join("sub").join("a.png")).unwrap();
        std::fs::copy(input.join("img1.png"), input.join("zz_ok.png")).unwrap();
        std::os::unix::fs::symlink(input.join("sub"), output.join("sub")).unwrap();
        std::os::unix::fs::symlink(&outside, output.join("linked")).unwrap();
        let linked = contained_target(&output, Path::new("linked/a.png"), &[], false);
        assert!(linked.unwrap_err().contains("leaves the output folder"));
        assert!(!outside.join("a.png").exists());
        std::fs::hard_link(input.join("img1.png"), output.join("img1.png")).unwrap();
        let same = contained_target(
            &output,
            Path::new("img1.png"),
            &[input.join("img1.png")],
            false,
        );
        assert!(
            same.unwrap_err().contains("same file"),
            "hard link to an input"
        );
        std::fs::remove_file(output.join("img1.png")).unwrap();
        let before = std::fs::read(input.join("sub").join("a.png")).unwrap();
        let ws = Snapshot::default();
        let ctx = egui::Context::default();
        let mut app = batch(&input, &output);
        app.subfolders = true;
        listed(&mut app, &ctx, &ws, 7);
        run_all(&mut app, &ctx, &ws);
        assert_eq!(app.counts(), (6, 1, 0), "{:?}", app.status());
        let row = app.files.iter().find(|f| f.name.contains("a.png")).unwrap();
        let Status::Error(message) = &row.status else {
            panic!("symlink write is an error row: {:?}", row.status);
        };
        assert!(message.contains("leaves the output folder"), "{message}");
        assert_eq!(
            std::fs::read(input.join("sub").join("a.png")).unwrap(),
            before
        );
        assert!(
            std::fs::symlink_metadata(output.join("sub"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(output.join("zz_ok.png").is_file());
        assert!(!outside.join("a.png").exists());
    }

    #[test]
    fn image_batch_processor_review_b6_stop_cancels_infinite_file_and_second_run_works() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output, code) = (
            dir.path().join("in"),
            dir.path().join("out"),
            dir.path().join("code"),
        );
        std::fs::create_dir(&input).unwrap();
        std::fs::create_dir(&code).unwrap();
        images(&input, false);
        // img1 has 8 columns and returns; later files enter `while true, end`.
        std::fs::write(
            code.join("hang_b6.m"),
            "function out = hang_b6(image)\n\
             if size(image, 2) > 8\n\
             while true, end\n\
             end\n\
             out = image;\n\
             end\n",
        )
        .unwrap();
        let mut ws = Snapshot::default();
        ws.set_folder(Some(code));
        let ctx = egui::Context::default();
        let mut app = batch(&input, &output);
        app.function = Function::Named;
        app.name = "hang_b6".into();
        listed(&mut app, &ctx, &ws, 5);
        app.run_all();
        until(&mut app, &ctx, &ws, |a| a.counts().0 == 1);
        assert!(app.running());
        let started = Instant::now();
        app.stop();
        loop {
            app.poll(&ctx, &ws);
            if !app.file_running() {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "runner still going after {:?}",
                started.elapsed()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!app.running());
        assert!(app.stopped);
        assert_eq!(app.files[0].status, Status::Ok);
        assert_eq!(app.files[1].status, Status::Stopped);
        assert!(app.files[2..].iter().all(|f| f.status == Status::Pending));
        app.function = Function::Gray;
        app.poll(&ctx, &ws);
        assert!(app.files.iter().all(|f| f.status == Status::Pending));
        run_all(&mut app, &ctx, &ws);
        assert_eq!(app.counts(), (5, 0, 0), "{:?}", app.status());
        assert!(!app.file_running());
    }

    #[test]
    fn image_batch_processor_struct_results_table_matches_reference_and_exports() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output, code) = (
            dir.path().join("in"),
            dir.path().join("out"),
            dir.path().join("code"),
        );
        std::fs::create_dir(&input).unwrap();
        std::fs::create_dir(&code).unwrap();
        images(&input, true);
        std::fs::write(code.join("stats_fn.m"), STATS).unwrap();
        let mut ws = Snapshot::default();
        ws.set_folder(Some(code.clone()));
        let ctx = egui::Context::default();
        let mut app = batch(&input, &output);
        app.function = Function::Named;
        app.name = "stats_fn".into();
        listed(&mut app, &ctx, &ws, 6);
        run_all(&mut app, &ctx, &ws);
        assert_eq!(app.counts(), (5, 1, 0), "{:?}", app.status());
        let t = app.table();
        assert_eq!(t.names, ["MeanValue", "MaxValue"]);
        let cases = reference()["cases"].as_array().unwrap().clone();
        assert_eq!(
            t.files,
            cases
                .iter()
                .map(|c| c["name"].as_str().unwrap())
                .collect::<Vec<_>>()
        );
        for (r, case) in cases.iter().enumerate() {
            close(t.columns[0][r], case["MeanValue"].as_f64().unwrap(), "mean");
            close(t.columns[1][r], case["MaxValue"].as_f64().unwrap(), "max");
        }
        // A struct-only function writes no images.
        assert!(!output.join("img1.png").exists());
        // Export: the collected table under its checked name, all-or-nothing.
        app.table_name = "stats; x".into();
        assert!(app.export_script(&ws).is_err());
        app.table_name = "mrexp_old".into();
        assert!(app.export_script(&ws).is_err());
        app.table_name = "stats_table".into();
        let Some(toolkits::Action::ExportScript(script)) = app.export(&ws) else {
            panic!("export");
        };
        let mut engine = matrust::Engine::default();
        engine.execute(&script).unwrap();
        let (names, files, columns) = table_of(&engine, "stats_table");
        assert_eq!(names, ["File", "MeanValue", "MaxValue"]);
        assert_eq!((files, columns), (t.files.clone(), t.columns.clone()));
    }

    #[test]
    fn image_batch_processor_generated_script_reproduces_table_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output, code) = (
            dir.path().join("in"),
            dir.path().join("out"),
            dir.path().join("code"),
        );
        std::fs::create_dir_all(input.join("sub")).unwrap();
        std::fs::create_dir(&code).unwrap();
        images(&input, true);
        // A subfolder file is mirrored under the output folder.
        std::fs::copy(input.join("img1.png"), input.join("sub").join("img1.png")).unwrap();
        std::fs::write(code.join("both_fn.m"), BOTH).unwrap();
        let mut ws = Snapshot::default();
        ws.set_folder(Some(code.clone()));
        let ctx = egui::Context::default();
        let mut app = batch(&input, &output);
        app.subfolders = true;
        app.function = Function::Named;
        app.name = "both_fn".into();
        app.returns = Returns::Both;
        listed(&mut app, &ctx, &ws, 7);
        run_all(&mut app, &ctx, &ws);
        assert_eq!(app.counts(), (6, 1, 0), "{:?}", app.status());
        let t = app.table();
        assert_eq!(t.files.len(), 6);
        let files = tree(&output);
        assert!(files.keys().any(|k| k.starts_with("sub")), "{files:?}");
        assert_eq!(files.len(), 6);
        let (_, gray) = gray_of(&reference()["cases"][0]);
        let mut engine = matrust::Engine::default();
        let sub = output.join("sub").join("img1.png");
        assert_eq!(
            uint8(&mut engine, &format!("imread('{}')", path(&sub))).1,
            gray
        );
        // The saved script, in a fresh Engine started in the function's folder.
        let Some(toolkits::Action::SaveAppScript(kind, script)) = app.generate(&ws, 0) else {
            panic!("generate");
        };
        assert_eq!(kind, AppKind::ImageBatchProcessor);
        for part in ["dir(", "try", "catch", "imwrite(", "table("] {
            assert!(script.contains(part), "{part}");
        }
        std::fs::remove_dir_all(&output).unwrap();
        let mut fresh = matrust::Engine::default();
        fresh.set_file_directory(&code).unwrap();
        fresh.execute(&script).unwrap();
        assert_eq!(tree(&output), files);
        let (names, rows, columns) = table_of(&fresh, "batchResults");
        assert_eq!(names, ["File", "MeanValue", "MaxValue"]);
        let expected: Vec<String> = t
            .files
            .iter()
            .map(|f| f.replace('/', std::path::MAIN_SEPARATOR_STR))
            .collect();
        assert_eq!((rows, columns), (expected, t.columns.clone()));
        // The template's script also writes its function file.
        let mut app = batch(&input, &output);
        app.function = Function::Resize;
        app.percent = "50".into();
        let script = app.script(&ws).unwrap();
        std::fs::remove_dir_all(&output).unwrap();
        let mut fresh = matrust::Engine::default();
        fresh.execute(&script).unwrap();
        assert!(output.join("batch_resize.m").is_file());
        assert_eq!(
            uint8(
                &mut fresh,
                &format!("imread('{}')", path(&output.join("img5.png")))
            )
            .0,
            [5, 6, 3]
        );
        let (_, files, _) = table_of(&fresh, "batchResults");
        assert_eq!(files.len(), 5);
    }

    #[test]
    fn image_batch_processor_refuses_output_equal_to_input_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Snapshot::default();
        let mut app = batch(dir.path(), dir.path());
        let e = app.plan(&ws).err().unwrap();
        assert!(e.contains("Overwrite"), "{e}");
        // The same folder spelled differently is still the same folder.
        app.output = format!("{}/.", dir.path().to_string_lossy());
        assert!(app.plan(&ws).is_err());
        // Ticking Overwrite asks once.
        app.request_overwrite();
        assert!(app.asking_overwrite() && !app.overwrite);
        app.confirm_overwrite_now();
        assert!(app.overwrite && app.plan(&ws).is_ok());
        app.overwrite = false;
        app.request_overwrite();
        assert!(app.overwrite && !app.asking_overwrite());
        // Subfolders on: an output folder inside the input is refused.
        let mut app = batch(dir.path(), &dir.path().join("out"));
        assert!(app.plan(&ws).is_ok());
        app.subfolders = true;
        assert!(app.plan(&ws).is_err());
        // Every free-text field enters the script as one expression.
        let mut app = batch(&dir.path().join("a"), &dir.path().join("b"));
        app.function = Function::Named;
        for bad in ["", "f; g", "f % c", "1x", "f(1)"] {
            app.name = bad.into();
            assert!(app.plan(&ws).is_err(), "{bad}");
        }
        app.name = "rgb2gray".into();
        assert!(app.plan(&ws).is_ok());
        app.function = Function::Resize;
        for bad in ["50; 1", "0", "1001", "abc", "50)"] {
            app.percent = bad.into();
            assert!(app.plan(&ws).is_err(), "{bad}");
        }
        app.percent = "25".into();
        assert!(
            app.script(&ws)
                .unwrap()
                .contains("imresize(image, 25 / 100)")
        );
        for bad in ["sub/*.png", "", "*.png\n*.jpg"] {
            app.pattern = bad.into();
            assert!(app.plan(&ws).is_err(), "{bad:?}");
        }
        app.pattern = "it's*.png".into();
        assert!(app.script(&ws).unwrap().contains("'it''s*.png'"));
        app.input = "a\nb".into();
        assert!(app.plan(&ws).is_err());
        app.input = dir.path().join("a").to_string_lossy().into_owned();
        app.table_name = "ibp_process".into();
        assert!(app.script(&ws).is_err());
    }

    #[test]
    fn image_batch_processor_rgb_handles_reach_image_functions() {
        // Engine fix: handle calls hit the N-D guard before the image
        // builtins ("rgb2gray does not support N-D arrays yet").
        let case = &reference()["cases"][0];
        let (_, gray) = gray_of(case);
        let mut engine = matrust::Engine::default();
        engine.execute(&rgb_code(1)).unwrap();
        engine.execute("h = str2func('rgb2gray');").unwrap();
        assert_eq!(uint8(&mut engine, "h(rgb)").1, gray);
        engine.execute("h = @rgb2gray;").unwrap();
        assert_eq!(uint8(&mut engine, "h(rgb)").1, gray);
        assert_eq!(uint8(&mut engine, "imcomplement(h(rgb))").1, {
            gray.iter().map(|g| 255 - g).collect::<Vec<_>>()
        });
        // Other builtins keep their N-D message.
        let e = engine.execute("g = @inv; g(rgb)").unwrap_err();
        assert!(e.contains("N-D"), "{e}");
        engine.execute("clear g").unwrap();
        // A workspace handle works as the app's function.
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in"), dir.path().join("out"));
        std::fs::create_dir(&input).unwrap();
        images(&input, false);
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        let ctx = egui::Context::default();
        let mut app = batch(&input, &output);
        app.function = Function::Named;
        app.name = "rgb".into();
        assert!(app.plan(&ws).is_err(), "not a handle");
        app.name = "h".into();
        listed(&mut app, &ctx, &ws, 5);
        run_all(&mut app, &ctx, &ws);
        assert_eq!(app.counts(), (5, 0, 0), "{:?}", app.status());
        let got = uint8(
            &mut engine,
            &format!("imread('{}')", path(&output.join("img1.png"))),
        );
        assert_eq!(got.1, gray);
    }

    #[test]
    fn image_batch_processor_tool_tab_and_headless_render_empty_5_500() {
        let kind = AppKind::ImageBatchProcessor;
        let tab = kind.tab();
        assert_eq!(AppKind::from_tab(tab), Some(kind));
        assert_eq!(
            serde_json::to_string(&tab).unwrap(),
            "\"image_batch_processor\""
        );
        let decoded: matrust::session::ToolTab =
            serde_json::from_str("\"image_batch_processor\"").unwrap();
        assert_eq!(decoded, tab);
        assert!(kind.is_built() && !kind.detail().is_empty());
        let dir = tempfile::tempdir().unwrap();
        let five = dir.path().join("five");
        let many = dir.path().join("many");
        std::fs::create_dir(&five).unwrap();
        std::fs::create_dir(&many).unwrap();
        images(&five, false);
        let png = std::fs::read(five.join("img1.png")).unwrap();
        for k in 0..500 {
            std::fs::write(many.join(format!("f{k:03}.png")), &png).unwrap();
        }
        for (input, count) in [(None, 0), (Some(&five), 5), (Some(&many), 500)] {
            for width in [820., 1600.] {
                let ctx = egui::Context::default();
                let mut desktop = Workspace::with_context(ctx.clone(), true);
                desktop.toolkits.open_tool(Tool::App(kind));
                if let Some(input) = input {
                    let app = &mut desktop.toolkits.apps.batch;
                    app.input = input.to_string_lossy().into_owned();
                    app.output = dir.path().join("out").to_string_lossy().into_owned();
                }
                let start = Instant::now();
                let mut slowest = Duration::ZERO;
                loop {
                    let frame = Instant::now();
                    let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                    slowest = slowest.max(frame.elapsed());
                    assert!(!output.shapes.is_empty());
                    let app = &desktop.toolkits.apps.batch;
                    if app.files.len() == count && !app.listing_running() {
                        break;
                    }
                    assert!(start.elapsed() < Duration::from_secs(60));
                    std::thread::sleep(Duration::from_millis(2));
                }
                if count == 5 {
                    // Run all from the frame loop: the UI never waits.
                    desktop.toolkits.apps.batch.run_all();
                    loop {
                        let frame = Instant::now();
                        frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                        slowest = slowest.max(frame.elapsed());
                        let app = &desktop.toolkits.apps.batch;
                        if !app.running() && app.thumbnails().is_some() && !app.thumbs_running() {
                            break;
                        }
                        assert!(start.elapsed() < Duration::from_secs(60));
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    assert_eq!(desktop.toolkits.apps.batch.counts(), (5, 0, 0));
                }
                for _ in 0..2 {
                    let frame = Instant::now();
                    let output = frame_at(&mut desktop, &ctx, egui::vec2(width, 950.), vec![]);
                    slowest = slowest.max(frame.elapsed());
                    assert!(!output.shapes.is_empty());
                }
                eprintln!(
                    "image_batch_processor {count} files at {width}: slowest frame {slowest:?}"
                );
                assert!(slowest < Duration::from_millis(500), "{slowest:?}");
                assert_eq!(desktop.toolkits.selected_tool(), Some(Tool::App(kind)));
                desktop.close_tool_tab(Tool::App(kind));
            }
        }
    }
}
