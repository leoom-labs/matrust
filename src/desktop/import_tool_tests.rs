// Import Tool app (apps/import_tool.rs): the generated script reproduces what
// Import creates for every output kind; detection settings, types, names,
// ranges and the omit-row rule appear in the script; names never clobber;
// errors; the former CSV import's result; every entry point that opened the
// CSV import opens the tool; a rendered tab. Fixture values are the ones
// tests/import_options.rs checks against pandas (tool/ref/import_options.py).
mod import_tool_tests {
    use super::*;
    use apps::import_tool::{Decimal, Delimiter, ImportTool, Output, Thousands};
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    const PATIENTS: &str = "LastName,Age,Height,Weight,Smoker\nSmith,38,71,176.5,1\nJohnson,43,69,163,0\nWilliams,,64,131,0\nJones,40,67,133.25,1\nBrown,49,64,119,0\n";
    const PRICES: &str = "Item;Price;Qty\nApple;1,25;10\nPear;0,8;5\nPlum;2,5;12\n";
    const SURVEY: &str =
        "id,score,group,weight\n1,7.5,A,60.2\n2,NA,B,\n3,8.25,,71\n4,6,A,NA\n5,,B,58.5\n";
    const EVENTS: &str = "Date,Due,Elapsed,Value\n2024-01-15,15/01/2024,00:10:00,1.5\n2024-02-29,29/02/2024,01:30:15,2.25\n2024-03-31,31/03/2024,12:00:00,3\n";
    const SENSOR: &str = "Sensor log exported 2024-03-01\nStation: North field\nTime\tTemp\tPressure\tStatus\ns\tdegC\tkPa\t-\n0\t20.5\t101.3\tok\n10\t20.7\t101.2\tok\n20\t21.1\t101\twarn\n30\t21.4\t100.9\tok\n";
    const WHEN: u64 = 1_794_389_400;
    const SIZE: egui::Vec2 = egui::vec2(1440., 900.);

    /// The fixtures in a temporary folder. book.xlsx has two sheets, Data
    /// and More, written by the engine's writetable.
    fn fixtures() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let long: String = std::iter::once("x,y\n".to_owned())
            .chain((1..=12).map(|k| format!("{k},{}\n", k * k)))
            .collect();
        for (name, text) in [
            ("patients.csv", PATIENTS),
            ("prices.csv", PRICES),
            ("survey.csv", SURVEY),
            ("events.csv", EVENTS),
            ("sensor.txt", SENSOR),
            ("long.csv", long.as_str()),
        ] {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        let mut engine = matrust::Engine::default();
        engine.set_current_folder(dir.path()).unwrap();
        engine
            .execute(
                "T0 = table([1; 2; 3; 4], [10.5; 20.25; 30; 40.75], {'a'; 'b'; 'c'; 'd'}, \
                 'VariableNames', {'ID', 'Score', 'Tag'});\n\
                 writetable(T0, 'book.xlsx', 'Sheet', 'Data');\n\
                 writetable(table([7; 8], 'VariableNames', {'k'}), 'book.xlsx', 'Sheet', 'More');",
            )
            .unwrap();
        dir
    }
    fn host_on(path: &std::path::Path) -> Host<ImportTool> {
        let mut host = Host::new(ImportTool::default());
        host.app.choose(path.to_owned());
        host
    }
    /// Private runs, synchronously, until the script stops changing (the
    /// first run detects; the automatic output kind may follow from it).
    fn settle_host(host: &mut Host<ImportTool>, ws: &Snapshot) -> Result<(), String> {
        let mut last = None;
        for _ in 0..4 {
            host.app.prepare(ws);
            let script = host.app.script(ws)?;
            if last.as_ref() == Some(&script) {
                return Ok(());
            }
            let request = host.request(ws)?;
            let outputs = apps::run(&mut matrust::Engine::default(), request)?;
            host.app.receive(&outputs)?;
            last = Some(script);
        }
        panic!("the Import Tool script kept changing")
    }
    fn script_of(host: &Host<ImportTool>, ws: &Snapshot) -> String {
        host.app.script(ws).unwrap_or_else(|e| panic!("{e}"))
    }
    /// The saved script (header and script) run in a fresh Engine.
    fn generated(host: &mut Host<ImportTool>, ws: &Snapshot) -> matrust::Engine {
        let Some(toolkits::Action::SaveAppScript(kind, saved)) = host.generate(ws, WHEN) else {
            panic!("generate action")
        };
        assert_eq!(kind, AppKind::ImportTool);
        assert!(
            saved.starts_with("% Import Tool (MatRust app), generated 2026-11-11 09:30 UTC.\n")
        );
        let mut engine = matrust::Engine::default();
        engine
            .execute(&saved)
            .unwrap_or_else(|e| panic!("{saved}\n{e}"));
        engine
    }
    /// `expr` is true in `engine`.
    #[track_caller]
    fn holds(engine: &mut matrust::Engine, expr: &str) {
        engine
            .execute(&format!("t__ = {expr};"))
            .unwrap_or_else(|e| panic!("{expr}: {e}"));
        let ok = match engine.array("t__") {
            Some(matrust::Array::Logical(m)) => {
                !m.data().is_empty() && m.data().iter().all(|v| *v != 0)
            }
            Some(matrust::Array::Numeric(m)) => {
                !m.data().is_empty() && m.data().iter().all(|v| *v != 0.)
            }
            other => panic!("{expr}: {other:?}"),
        };
        assert!(ok, "{expr} is false");
        engine.execute("clear t__").unwrap();
    }
    /// Same class, size and values. Tables compare per variable: the
    /// engine's isequaln is false for two equal tables.
    fn same(a: &matrust::Array, b: &matrust::Array) -> bool {
        let mut engine = matrust::Engine::default();
        engine.insert_array("a_", a.clone()).unwrap();
        engine.insert_array("b_", b.clone()).unwrap();
        engine
            .execute(
                "ok_ = strcmp(class(a_), class(b_)) && isequal(size(a_), size(b_));\n\
                 if ok_ && (istable(a_) || istimetable(a_))\n\
                 ok_ = isequal(a_.Properties.VariableNames, b_.Properties.VariableNames);\n\
                 for k_ = 1:width(a_)\n\
                 ok_ = ok_ && isequaln(a_{:, k_}, b_{:, k_});\n\
                 end\n\
                 if istimetable(a_)\n\
                 ok_ = ok_ && isequal(a_.Properties.RowTimes, b_.Properties.RowTimes);\n\
                 end\n\
                 elseif ok_\n\
                 ok_ = isequaln(a_, b_);\n\
                 end\n",
            )
            .unwrap();
        matches!(engine.array("ok_"), Some(matrust::Array::Logical(m)) if m.data() == [1])
    }
    fn dims(array: &matrust::Array) -> Vec<usize> {
        let mut engine = matrust::Engine::default();
        engine.insert_array("a_", array.clone()).unwrap();
        engine.execute("s_ = size(a_);").unwrap();
        match engine.array("s_") {
            Some(matrust::Array::Numeric(m)) => m.data().iter().map(|v| *v as usize).collect(),
            other => panic!("{other:?}"),
        }
    }

    /// Frames until the Import Tool's private run is done and stays done.
    pub(super) fn settle(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut idle = 0;
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            if app.toolkits.apps.import.running() {
                idle = 0;
            } else {
                idle += 1;
                if idle >= 3 {
                    return output;
                }
            }
            assert!(
                Instant::now() < deadline,
                "Import Tool run did not finish: {:?}",
                app.toolkits.apps.import.status()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// The file the Import Tool has open.
    pub(super) fn chosen(app: &Workspace) -> Option<PathBuf> {
        app.toolkits
            .apps
            .import
            .app
            .source
            .as_ref()
            .map(|s| s.path.clone())
    }
    fn painted(output: &egui::FullOutput, label: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, label).is_some())
    }
    fn numbers(value: &matrust::Array) -> (String, usize, usize, Vec<f64>) {
        let m = value.as_numeric().unwrap();
        (
            value.class().to_owned(),
            m.rows(),
            m.cols(),
            m.data().to_vec(),
        )
    }

    // -----------------------------------------------------------------------
    // The former CSV import

    /// What the removed CSV import (csv_import::perform, numeric CSV to a
    /// double matrix named after the file) created, captured from that code
    /// before it was removed: name, class, rows, columns, column-major data.
    /// File, its text, then the variable: name, rows, columns, data.
    type Captured = (
        &'static str,
        &'static str,
        &'static str,
        usize,
        usize,
        &'static [f64],
    );
    const OLD_CSV_IMPORT: [Captured; 3] = [
        ("plain.csv", "5,6\n7,8\n", "plain", 2, 2, &[5., 7., 6., 8.]),
        (
            "measurements.csv",
            "1, 2.5,-3\r\n4,5e-1,6\r\n\r\n7,8,9.25\r\n",
            "measurements",
            3,
            3,
            &[1., 4., 7., 2.5, 0.5, 8., -3., 6., 9.25],
        ),
        ("single.csv", "9\n", "single", 1, 1, &[9.]),
    ];

    #[test]
    fn import_tool_plain_csv_with_defaults_matches_the_former_csv_import() {
        for (file, text, name, rows, cols, data) in OLD_CSV_IMPORT {
            let ctx = egui::Context::default();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(file);
            std::fs::write(&path, text).unwrap();
            let mut app = Workspace::with_context(ctx.clone(), true);
            // Dropped like the former import's main entry point.
            drop_paths(&mut app, &ctx, vec![path.clone()]);
            finish(&mut app);
            let deadline = Instant::now() + Duration::from_secs(5);
            while chosen(&app).is_none() && Instant::now() < deadline {
                frame_at(&mut app, &ctx, SIZE, vec![]);
                finish(&mut app);
            }
            assert_eq!(chosen(&app), Some(path.canonicalize().unwrap()), "{file}");
            settle(&mut app, &ctx, SIZE);
            assert_eq!(
                app.toolkits.apps.import.app.output(),
                Output::Matrix,
                "{file}"
            );
            click_label_at(&mut app, &ctx, SIZE, "Import");
            finish(&mut app);
            assert_eq!(
                app.variables.keys().collect::<Vec<_>>(),
                [name],
                "{file}: only the data variable is created ({})",
                app.status
            );
            assert_eq!(
                numbers(&app.variables[name]),
                ("double".to_owned(), rows, cols, data.to_vec()),
                "{file}"
            );
            assert!(app.toolkits.apps.import.message.contains(name));
        }
    }

    #[test]
    fn import_tool_default_names_follow_the_file_and_stay_valid() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Snapshot::default();
        for (file, expected) in [
            ("while.csv", "data"),
            ("9data.csv", "data"),
            ("bad-name.csv", "data"),
            ("size.csv", "data"),
            ("Valid_2.csv", "Valid_2"),
        ] {
            let mut tool = ImportTool::default();
            tool.choose(dir.path().join(file));
            assert_eq!(tool.default_name(&ws), expected, "{file}");
        }
        let long = format!("{}.csv", "a".repeat(63));
        let mut tool = ImportTool::default();
        tool.choose(dir.path().join(long));
        let name = tool.default_name(&ws);
        assert_eq!(name.len(), 63);
        workspace_edit::validate_name(&name).unwrap();
        // Taken: the number fits within 63 characters (no endless search).
        let mut taken = Snapshot::default();
        taken.sync(&BTreeMap::from([(
            name.clone(),
            matrust::Array::Numeric(matrust::Matrix::scalar(1.)),
        )]));
        assert_eq!(tool.default_name(&taken), format!("{}1", "a".repeat(56)));
        // A column vector named after a taken 63-character column, too.
        tool.output = Some(Output::Columns);
        tool.columns = Some(vec![apps::import_tool::Column::new(
            name.clone(),
            "double".into(),
        )]);
        let plan = tool.plan(&taken).unwrap();
        assert_eq!(plan.names, [(format!("{}1", "a".repeat(56)), name)]);
    }

    // -----------------------------------------------------------------------
    // Script and import agree

    /// Checks on the saved script's output `{v}` per file and output kind.
    fn reference_checks(file: &str, kind: Output) -> Vec<&'static str> {
        match (file, kind) {
            ("patients.csv", Output::Table) => vec![
                "isequaln({v}.Age, [38; 43; NaN; 40; 49])",
                "isequal({v}.Weight, [176.5; 163; 131; 133.25; 119])",
                "iscellstr({v}.LastName) && strcmp({v}.LastName{3}, 'Williams')",
            ],
            ("patients.csv", Output::Matrix) => vec![
                "isequal(size({v}), [5 5]) && all(isnan({v}(:, 1)))",
                "isequaln({v}(:, 2:3), [38 71; 43 69; NaN 64; 40 67; 49 64])",
            ],
            ("patients.csv", Output::Cells) => {
                vec![
                    "iscell({v}) && isequal(size({v}), [5 5]) && strcmp({v}{2, 1}, 'Johnson') && {v}{4, 4} == 133.25",
                ]
            }
            ("patients.csv", Output::Strings) => {
                vec![
                    "isstring({v}) && isequal(size({v}), [5 5]) && {v}(1, 1) == \"Smith\" && {v}(5, 4) == \"119\"",
                ]
            }
            ("patients.csv", Output::Columns) => vec![
                "isequaln(Age, [38; 43; NaN; 40; 49]) && isequal(Smoker, [1; 0; 0; 1; 0])",
                "iscellstr(LastName) && numel(LastName) == 5",
            ],
            ("prices.csv", Output::Table) => vec![
                "isequal({v}.Price, [1.25; 0.8; 2.5]) && isequal({v}.Qty, [10; 5; 12])",
                "sum({v}.Price .* {v}.Qty) == 46.5",
            ],
            ("prices.csv", Output::Matrix) => {
                vec!["isequal({v}(:, 2:3), [1.25 10; 0.8 5; 2.5 12])"]
            }
            ("survey.csv", Output::Table) => vec![
                "isequaln({v}.score, [7.5; NaN; 8.25; 6; NaN])",
                "isequaln({v}.weight, [60.2; NaN; 71; NaN; 58.5])",
                "isequal({v}.group, {'A'; 'B'; ''; 'A'; 'B'})",
            ],
            ("events.csv", Output::Table) => vec![
                "isdatetime({v}.Date) && isequal(days(diff({v}.Date)), [45; 31])",
                "isequal([year({v}.Date) month({v}.Date) day({v}.Date)], [2024 1 15; 2024 2 29; 2024 3 31])",
                "isduration({v}.Elapsed) && isequal(seconds({v}.Elapsed), [600; 5415; 43200])",
            ],
            ("events.csv", Output::Timetable) => vec![
                "istimetable({v}) && height({v}) == 3",
                "strcmp({v}.Properties.DimensionNames{1}, 'Date')",
                "isequal({v}.Properties.VariableNames, {'Due', 'Elapsed', 'Value'}) && isequal({v}.Value, [1.5; 2.25; 3])",
            ],
            ("book.xlsx", Output::Table) => vec![
                "isequal({v}.ID, [1; 2; 3; 4]) && isequal({v}.Score, [10.5; 20.25; 30; 40.75])",
                "isequal({v}.Tag, {'a'; 'b'; 'c'; 'd'})",
            ],
            ("book.xlsx", Output::Matrix) => {
                vec!["isequal({v}(:, 1:2), [1 10.5; 2 20.25; 3 30; 4 40.75])"]
            }
            _ => vec![],
        }
    }

    #[test]
    fn import_tool_generated_script_matches_the_import_for_every_output_kind() {
        let dir = fixtures();
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        let mut checked = 0;
        for file in [
            "patients.csv",
            "prices.csv",
            "survey.csv",
            "events.csv",
            "book.xlsx",
        ] {
            let path = dir.path().join(file);
            for kind in Output::ALL {
                app.run("clear".into());
                finish(&mut app);
                app.toolkits.apps.workspace.sync(&app.variables);
                let ws = &app.toolkits.apps.workspace;
                let host = &mut app.toolkits.apps.import;
                host.app.choose(path.clone());
                host.app.output = Some(kind);
                match settle_host(host, ws) {
                    Ok(()) => {}
                    Err(error) => {
                        // Only a file without a datetime or duration column
                        // cannot make a timetable.
                        assert!(
                            kind == Output::Timetable && error.contains("datetime or duration"),
                            "{file} {kind:?}: {error}"
                        );
                        continue;
                    }
                }
                let names = host.app.exported(ws);
                let created = host.app.created.clone().expect("the run's size and class");
                let mut fresh = generated(host, ws);
                // Import: the framework's export, run in the workspace.
                let Some(toolkits::Action::ExportScript(export)) = host.export(ws) else {
                    panic!("export action")
                };
                assert!(export.contains("detectImportOptions("), "{export}");
                app.run(export.clone());
                finish(&mut app);
                for name in &names {
                    let imported = app.variables.get(name).unwrap_or_else(|| {
                        panic!("{file} {kind:?}: {name} not imported: {}", app.status)
                    });
                    let saved = fresh.array(name).unwrap();
                    assert!(same(imported, &saved), "{file} {kind:?}: {name}\n{export}");
                }
                // Only the outputs are left: no options or temporary table.
                assert_eq!(
                    app.variables.len(),
                    names.len(),
                    "{file} {kind:?}: {:?}",
                    app.variables.keys()
                );
                if kind == Output::Columns {
                    assert_eq!(created.size[1], names.len());
                } else {
                    assert_eq!(
                        dims(&app.variables[&names[0]]),
                        created.size,
                        "{file} {kind:?}"
                    );
                    assert_eq!(app.variables[&names[0]].class(), created.class);
                }
                for check in reference_checks(file, kind) {
                    holds(&mut fresh, &check.replace("{v}", &names[0]));
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 24, "reference checks");
    }

    #[test]
    fn import_tool_preview_is_the_engine_read_with_missing_cells_marked() {
        let dir = fixtures();
        let ws = Snapshot::default();
        let mut host = host_on(&dir.path().join("survey.csv"));
        settle_host(&mut host, &ws).unwrap();
        let tool = &host.app;
        let preview = tool.preview.as_ref().unwrap();
        assert_eq!((preview.rows, preview.cols), (5, 4));
        assert_eq!(preview.cell(0, 1), "7.5");
        assert_eq!(preview.cell(0, 2), "A");
        // NA (an import error), empty fields: filled, so marked missing.
        let marked: Vec<(usize, usize)> = (0..5)
            .flat_map(|r| (0..4).map(move |c| (r, c)))
            .filter(|(r, c)| preview.is_missing(*r, *c))
            .collect();
        assert_eq!(marked, [(1, 1), (1, 3), (2, 2), (3, 3), (4, 1)]);
        let columns = tool.columns.as_ref().unwrap();
        assert_eq!(preview.missing_count(columns), 5);
        let kinds: Vec<&str> = columns.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, ["double", "double", "char", "double"]);
        assert_eq!(
            tool.results()[2..],
            [("Size", "5 × 4".to_owned()), ("Class", "table".to_owned())]
        );
        // A long file: the grid reads the first 200 rows only.
        let long = dir.path().join("many.csv");
        let text: String = std::iter::once("n\n".to_owned())
            .chain((1..=450).map(|k| format!("{k}\n")))
            .collect();
        std::fs::write(&long, text).unwrap();
        let mut host = host_on(&long);
        settle_host(&mut host, &ws).unwrap();
        let preview = host.app.preview.as_ref().unwrap();
        assert_eq!(preview.rows, apps::import_tool::PREVIEW_ROWS);
        assert_eq!(preview.cell(199, 0), "200");
        assert_eq!(host.app.created.as_ref().unwrap().size, [450, 1]);
    }

    // -----------------------------------------------------------------------
    // Inputs become script lines

    #[test]
    fn import_tool_unchanged_inputs_give_the_three_line_script() {
        let dir = fixtures();
        let ws = Snapshot::default();
        let path = dir.path().join("patients.csv");
        let mut host = host_on(&path);
        settle_host(&mut host, &ws).unwrap();
        let file = apps::import_tool::quote(&path.to_string_lossy());
        assert_eq!(
            script_of(&host, &ws),
            format!(
                "opts = detectImportOptions({file});\npatients = readtable({file}, opts);\nclear opts\n"
            )
        );
        // The preview, Import and Generate script share that text.
        let request = host.request(&ws).unwrap();
        assert!(request.code.starts_with(&script_of(&host, &ws)));
        assert!(request.outputs.iter().all(|n| n.starts_with("app_")));
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!("export")
        };
        let script = script_of(&host, &ws);
        assert!(export.contains(&format!("try\n{}\ncatch", script.trim_end())));
    }

    #[test]
    fn import_tool_type_changes_renames_and_selection_appear_in_the_script() {
        let dir = fixtures();
        let ws = Snapshot::default();
        let path = dir.path().join("patients.csv");
        let mut host = host_on(&path);
        settle_host(&mut host, &ws).unwrap();
        {
            let columns = host.app.columns.as_mut().unwrap();
            columns[0].kind = "string".into();
            columns[1].kind = "int32".into();
            columns[2].name = "HeightIn".into();
            columns[4].selected = false;
        }
        let file = apps::import_tool::quote(&path.to_string_lossy());
        assert_eq!(
            script_of(&host, &ws),
            format!(
                "opts = detectImportOptions({file});\n\
                 opts = setvartype(opts, 'LastName', 'string');\n\
                 opts = setvartype(opts, 'Age', 'int32');\n\
                 opts.VariableNames = {{'LastName', 'Age', 'HeightIn', 'Weight', 'Smoker'}};\n\
                 opts.SelectedVariableNames = {{'LastName', 'Age', 'HeightIn', 'Weight'}};\n\
                 patients = readtable({file}, opts);\n\
                 clear opts\n"
            )
        );
        settle_host(&mut host, &ws).unwrap();
        assert_eq!(host.app.created.as_ref().unwrap().size, [5, 4]);
        // The preview shows every column; Smoker is the greyed one.
        assert_eq!(host.app.preview.as_ref().unwrap().cols, 5);
        let mut engine = generated(&mut host, &ws);
        holds(
            &mut engine,
            "isequal(patients.Properties.VariableNames, {'LastName', 'Age', 'HeightIn', 'Weight'})",
        );
        holds(
            &mut engine,
            "isa(patients.Age, 'int32') && patients.Age(2) == 43",
        );
        holds(
            &mut engine,
            "isstring(patients.LastName) && patients.LastName(1) == \"Smith\"",
        );
        holds(
            &mut engine,
            "isequal(patients.HeightIn, [71; 69; 64; 67; 64])",
        );
        // A datetime column read with an explicit input format.
        let path = dir.path().join("events.csv");
        let mut host = host_on(&path);
        settle_host(&mut host, &ws).unwrap();
        {
            let columns = host.app.columns.as_mut().unwrap();
            assert_eq!(columns[1].kind, "datetime");
            columns[1].format = "dd/MM/yyyy".into();
            columns[3].kind = "single".into();
        }
        let script = script_of(&host, &ws);
        assert!(
            script.contains("opts = setvaropts(opts, 'Due', 'InputFormat', 'dd/MM/yyyy');\n"),
            "{script}"
        );
        assert!(
            script.contains("opts = setvartype(opts, 'Value', 'single');\n"),
            "{script}"
        );
        settle_host(&mut host, &ws).unwrap();
        let mut engine = generated(&mut host, &ws);
        holds(
            &mut engine,
            "strcmp(events.Due.Format, 'dd/MM/yyyy') && isequal(events.Date, events.Due)",
        );
        holds(&mut engine, "isa(events.Value, 'single')");
    }

    #[test]
    fn import_tool_detection_settings_are_detect_arguments() {
        let dir = fixtures();
        let ws = Snapshot::default();
        // Semicolons and decimal commas are found without settings.
        let path = dir.path().join("prices.csv");
        let mut host = host_on(&path);
        settle_host(&mut host, &ws).unwrap();
        assert_eq!(host.app.detected.as_ref().unwrap().delimiter, [";"]);
        assert!(!script_of(&host, &ws).contains("'Delimiter'"));
        // Given explicitly, they are detectImportOptions arguments.
        host.app.delimiter = Delimiter::Semicolon;
        host.app.decimal = Decimal::Comma;
        settle_host(&mut host, &ws).unwrap();
        let script = script_of(&host, &ws);
        assert!(
            script.contains("detectImportOptions('")
                && script.contains("', 'Delimiter', ';', 'DecimalSeparator', ',');\n"),
            "{script}"
        );
        let mut engine = generated(&mut host, &ws);
        holds(&mut engine, "isequal(prices.Price, [1.25; 0.8; 2.5])");
        // A comma delimiter makes other columns of the same file.
        host.app.delimiter = Delimiter::Comma;
        host.app.decimal = Decimal::Auto;
        settle_host(&mut host, &ws).unwrap();
        assert!(script_of(&host, &ws).contains("'Delimiter', ','"));
        assert_eq!(host.app.columns.as_ref().unwrap().len(), 2);
        // Header lines and the names line of a logged file.
        let mut host = host_on(&dir.path().join("sensor.txt"));
        settle_host(&mut host, &ws).unwrap();
        assert_eq!(host.app.detected.as_ref().unwrap().names_line, 3);
        assert_eq!(host.app.detected.as_ref().unwrap().first, 5);
        host.app.names_line = "0".into();
        host.app.header_lines = "4".into();
        settle_host(&mut host, &ws).unwrap();
        let script = script_of(&host, &ws);
        assert!(
            script.contains("'NumHeaderLines', 4, 'VariableNamesLine', 0"),
            "{script}"
        );
        let names: Vec<&str> = host
            .app
            .columns
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.detected.as_str())
            .collect();
        assert_eq!(names, ["Var1", "Var2", "Var3", "Var4"]);
        let mut engine = generated(&mut host, &ws);
        holds(
            &mut engine,
            "isequal(sensor.Var1, [0; 10; 20; 30]) && isequal(sensor.Var2, [20.5; 20.7; 21.1; 21.4])",
        );
        // Tab, space and other delimiters.
        for (delimiter, custom, text) in [
            (Delimiter::Tab, "", "'Delimiter', '\\t'"),
            (Delimiter::Space, "", "'Delimiter', ' '"),
            (Delimiter::Custom, "|", "'Delimiter', '|'"),
        ] {
            host.app.delimiter = delimiter;
            host.app.custom = custom.into();
            assert!(
                host.app.detect_arguments().unwrap().contains(text),
                "{delimiter:?}"
            );
        }
        host.app.custom.clear();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("other delimiter")
        );
        // Grouped thousands with a decimal comma.
        let path = dir.path().join("grouped.csv");
        std::fs::write(&path, "v;w\n1.234,5;2\n10,25;3\n").unwrap();
        let mut host = host_on(&path);
        host.app.delimiter = Delimiter::Semicolon;
        host.app.decimal = Decimal::Comma;
        host.app.thousands = Thousands::Period;
        settle_host(&mut host, &ws).unwrap();
        let script = script_of(&host, &ws);
        assert!(script.contains("'ThousandsSeparator', '.'"), "{script}");
        assert_eq!(
            host.app.columns.as_ref().unwrap()[0].detected_type,
            "double"
        );
        let mut engine = generated(&mut host, &ws);
        holds(
            &mut engine,
            "isequal(grouped.v, [1234.5; 10.25]) && isequal(grouped.w, [2; 3])",
        );
    }

    #[test]
    fn import_tool_sheets_of_a_workbook() {
        let dir = fixtures();
        let ws = Snapshot::default();
        let path = dir.path().join("book.xlsx");
        let mut host = host_on(&path);
        assert_eq!(host.app.source.as_ref().unwrap().sheets, ["Data", "More"]);
        settle_host(&mut host, &ws).unwrap();
        // The first sheet is the default: no Sheet argument.
        assert!(!script_of(&host, &ws).contains("'Sheet'"));
        assert_eq!(host.app.columns.as_ref().unwrap().len(), 3);
        host.app.sheet = 1;
        settle_host(&mut host, &ws).unwrap();
        let script = script_of(&host, &ws);
        assert!(script.contains("'Sheet', 'More'"), "{script}");
        let names: Vec<&str> = host
            .app
            .columns
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["k"]);
        let mut engine = generated(&mut host, &ws);
        holds(&mut engine, "istable(book) && isequal(book.k, [7; 8])");
        assert_eq!(
            apps::import_tool::sheet_names(&path).unwrap(),
            ["Data", "More"]
        );
    }

    #[test]
    fn import_tool_data_lines_and_rows_set_the_range() {
        let dir = fixtures();
        let ws = Snapshot::default();
        let mut host = host_on(&dir.path().join("long.csv"));
        settle_host(&mut host, &ws).unwrap();
        assert_eq!(host.app.detected.as_ref().unwrap().first, 2);
        for (first, last, line, x) in [
            ("3", "6", "opts.DataLines = [3 6];", "[2; 3; 4; 5]"),
            ("10", "", "opts.DataLines = [10 Inf];", "[9; 10; 11; 12]"),
            ("", "4", "opts.DataLines = [2 4];", "[1; 2; 3]"),
        ] {
            host.app.first = first.into();
            host.app.last = last.into();
            settle_host(&mut host, &ws).unwrap();
            let script = script_of(&host, &ws);
            assert!(script.contains(line), "{script}");
            let mut engine = generated(&mut host, &ws);
            holds(&mut engine, &format!("isequal(long.x, {x})"));
            // The preview reads the same lines.
            let preview = host.app.preview.as_ref().unwrap();
            assert_eq!(preview.cell(0, 0), x[1..].split(';').next().unwrap());
        }
        // Column selection.
        host.app.first.clear();
        host.app.last.clear();
        host.app.columns.as_mut().unwrap()[1].selected = false;
        settle_host(&mut host, &ws).unwrap();
        assert!(script_of(&host, &ws).contains("opts.SelectedVariableNames = {'x'};"));
        assert_eq!(host.app.created.as_ref().unwrap().size, [12, 1]);
        // Rows of a workbook.
        let mut host = host_on(&dir.path().join("book.xlsx"));
        settle_host(&mut host, &ws).unwrap();
        assert_eq!(host.app.detected.as_ref().unwrap().first, 2);
        for (first, last, line, ids) in [
            ("3", "", "opts.DataRange = 'A3';", "[2; 3; 4]"),
            ("2", "3", "opts.DataRange = '2:3';", "[1; 2]"),
        ] {
            host.app.first = first.into();
            host.app.last = last.into();
            settle_host(&mut host, &ws).unwrap();
            let script = script_of(&host, &ws);
            assert!(script.contains(line), "{script}");
            let mut engine = generated(&mut host, &ws);
            holds(&mut engine, &format!("isequal(book.ID, {ids})"));
        }
        // Lines that are not numbers, or out of order.
        for (first, last, error) in [
            ("x", "", "whole number"),
            ("5", "2", "before the first"),
            ("0", "", "start at 1"),
        ] {
            host.app.first = first.into();
            host.app.last = last.into();
            assert!(
                host.app.script(&ws).unwrap_err().contains(error),
                "{first} {last}"
            );
        }
    }

    #[test]
    fn import_tool_omit_row_rule_leaves_out_rows_with_missing_cells() {
        let dir = fixtures();
        let ws = Snapshot::default();
        let mut host = host_on(&dir.path().join("survey.csv"));
        settle_host(&mut host, &ws).unwrap();
        host.app.omit_rows = true;
        settle_host(&mut host, &ws).unwrap();
        let script = script_of(&host, &ws);
        assert!(
            script.contains("opts.MissingRule = 'omitrow';\nopts.ImportErrorRule = 'omitrow';\n"),
            "{script}"
        );
        // Only the first row has every field.
        assert_eq!(host.app.created.as_ref().unwrap().size, [1, 4]);
        let mut engine = generated(&mut host, &ws);
        holds(
            &mut engine,
            "isequal(survey.id, 1) && survey.weight == 60.2",
        );
        // With id and score only, rows 1, 3 and 4 stay (pandas dropna).
        for column in &mut host.app.columns.as_mut().unwrap()[2..] {
            column.selected = false;
        }
        settle_host(&mut host, &ws).unwrap();
        let mut engine = generated(&mut host, &ws);
        holds(
            &mut engine,
            "isequal(survey.id, [1; 3; 4]) && isequal(survey.score, [7.5; 8.25; 6])",
        );
        // The preview still shows every row; rows 2 and 5 are the omitted ones.
        let tool = &host.app;
        let preview = tool.preview.as_ref().unwrap();
        let columns = tool.columns.as_ref().unwrap();
        let omitted: Vec<usize> = (0..preview.rows)
            .filter(|r| (0..preview.cols).any(|c| columns[c].selected && preview.is_missing(*r, c)))
            .collect();
        assert_eq!(omitted, [1, 4]);
    }

    #[test]
    fn import_tool_names_never_clobber_workspace_variables() {
        let dir = fixtures();
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run("patients = 7; opts = 3; tbl = 1; Age = 2;".into());
        finish(&mut app);
        app.open_in_import_tool(dir.path().join("patients.csv"));
        settle(&mut app, &ctx, SIZE);
        let file = apps::import_tool::quote(&dir.path().join("patients.csv").to_string_lossy());
        {
            let ws = &app.toolkits.apps.workspace;
            let script = app.toolkits.apps.import.app.script(ws).unwrap();
            assert_eq!(
                script,
                format!(
                    "opts1 = detectImportOptions({file});\npatients1 = readtable({file}, opts1);\nclear opts1\n"
                )
            );
        }
        click_label_at(&mut app, &ctx, SIZE, "Import");
        finish(&mut app);
        for (name, value) in [("patients", 7.), ("opts", 3.), ("tbl", 1.), ("Age", 2.)] {
            assert_eq!(
                app.variables[name]
                    .scalar(0, 0)
                    .unwrap()
                    .into_real()
                    .unwrap(),
                value
            );
        }
        assert_eq!(app.variables["patients1"].class(), "table");
        assert!(!app.variables.contains_key("opts1"));
        // Taken now: the next default is patients2.
        settle(&mut app, &ctx, SIZE);
        assert_eq!(app.toolkits.apps.import.app.name, "patients2");
        // Column vectors are named after the columns, never over a variable.
        app.toolkits.apps.import.app.output = Some(Output::Columns);
        settle(&mut app, &ctx, SIZE);
        {
            let ws = &app.toolkits.apps.workspace;
            let script = app.toolkits.apps.import.app.script(ws).unwrap();
            assert!(script.contains("tbl1 = readtable("), "{script}");
            assert!(script.contains("Age1 = tbl1.Age;\n"), "{script}");
            assert!(script.ends_with("clear opts1 tbl1\n"), "{script}");
        }
        click_label_at(&mut app, &ctx, SIZE, "Import");
        finish(&mut app);
        assert_eq!(
            app.variables["Age"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            2.
        );
        assert_eq!(app.variables["Age1"].len(), 5);
        assert!(app.variables.contains_key("LastName") && !app.variables.contains_key("tbl1"));
    }

    #[test]
    fn import_tool_errors_name_the_problem() {
        let dir = fixtures();
        let ws = Snapshot::default();
        // No file yet.
        let mut host = Host::new(ImportTool::default());
        assert!(host.app.script(&ws).unwrap_err().contains("choose a file"));
        // A missing file: the engine's message.
        host.app.choose(dir.path().join("nofile.csv"));
        assert_eq!(host.app.source.as_ref().unwrap().bytes, None);
        let error = settle_host(&mut host, &ws).unwrap_err();
        assert!(error.contains("Unable to find or open"), "{error}");
        // A binary file is refused before any run.
        let binary = dir.path().join("image.dat");
        std::fs::write(&binary, [0x89, b'P', b'N', b'G', 0, 0, 0, 13, 1, 2, 3]).unwrap();
        host.app.choose(binary);
        assert!(host.app.source.as_ref().unwrap().binary);
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("image.dat is not a text file"), "{error}");
        // Names, columns and the timetable's row times.
        let mut host = host_on(&dir.path().join("patients.csv"));
        settle_host(&mut host, &ws).unwrap();
        for (name, error) in [
            ("while", "not a valid variable name"),
            ("2x", "not a valid"),
            ("size", "function the import calls"),
        ] {
            host.app.name = name.into();
            host.app.name_edited = true;
            assert!(host.app.script(&ws).unwrap_err().contains(error), "{name}");
        }
        host.app.name_edited = false;
        host.app.columns.as_mut().unwrap()[1].name = "LastName".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("two columns are named LastName")
        );
        host.app.columns.as_mut().unwrap()[1].name = "bad name".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("column 2 needs a valid")
        );
        host.app.columns.as_mut().unwrap()[1].name = "Age".into();
        host.app
            .columns
            .as_mut()
            .unwrap()
            .iter_mut()
            .for_each(|c| c.selected = false);
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("at least one column")
        );
        host.app
            .columns
            .as_mut()
            .unwrap()
            .iter_mut()
            .for_each(|c| c.selected = true);
        host.app.output = Some(Output::Timetable);
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("datetime or duration")
        );
        // A type the data cannot take: the engine's error, the last preview stays.
        host.app.output = None;
        settle_host(&mut host, &ws).unwrap();
        host.app.columns.as_mut().unwrap()[0].kind = "datetime".into();
        host.app.columns.as_mut().unwrap()[0].format = "yyyy-MM-dd".into();
        let outcome = settle_host(&mut host, &ws);
        assert!(host.app.preview.is_some());
        if let Err(error) = outcome {
            assert!(!error.is_empty());
        }
        // In the desktop: the status line names it and Import does nothing.
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.open_in_import_tool(dir.path().join("nofile.csv"));
        settle(&mut app, &ctx, SIZE);
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::ImportTool), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("Unable to find or open"), "{text}");
        click_label_at(&mut app, &ctx, SIZE, "Import");
        assert!(!app.busy && app.variables.is_empty());
    }

    // -----------------------------------------------------------------------
    // The desktop: entry points, rendering, saved script

    #[test]
    fn import_tool_tab_renders_inputs_grid_and_output() {
        let dir = fixtures();
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.open_in_import_tool(dir.path().join("patients.csv"));
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::ImportTool))
        );
        // Tall enough for every input without scrolling.
        let output = settle(&mut app, &ctx, egui::vec2(1440., 1200.));
        for label in [
            "INPUTS",
            "PREVIEW",
            "OUTPUT",
            "patients.csv",
            "Choose file…",
            "Delimiter",
            "Header lines",
            "Names line",
            "Decimal",
            "Thousands",
            "Data lines",
            "Unimportable cells",
            "Output type",
            "Variable name",
            "Import",
            "Generate script",
            "LastName",
            "char",
            "Smith",
            "176.5",
            "Williams",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        // In a 900 px window the Import button is in view.
        let output = frame_at(&mut app, &ctx, SIZE, vec![]);
        for label in ["Import", "Generate script", "Smith", "Delimiter"] {
            assert!(painted(&output, label), "{label} not painted at 900 px");
        }
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::ImportTool), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success);
        assert_eq!(text, "patients.csv → table 5×5");
        // The type dropdown changes the column and the script.
        click_label_at(&mut app, &ctx, SIZE, "char");
        click_label_at(&mut app, &ctx, SIZE, "string");
        settle(&mut app, &ctx, SIZE);
        let ws = &app.toolkits.apps.workspace;
        assert!(
            app.toolkits
                .apps
                .import
                .app
                .script(ws)
                .unwrap()
                .contains("setvartype(opts, 'LastName', 'string')")
        );
        // A narrow host stacks OUTPUT under the preview.
        let output = frame_at(&mut app, &ctx, egui::vec2(900., 700.), vec![]);
        assert!(painted(&output, "OUTPUT"));
        // A wide, long file: only the cells in view are laid out.
        let wide = dir.path().join("wide.csv");
        let header: Vec<String> = (1..=400).map(|c| format!("c{c}")).collect();
        let mut text = header.join(",") + "\n";
        for r in 0..300 {
            let row: Vec<String> = (0..400).map(|c| (r * 400 + c).to_string()).collect();
            text.push_str(&(row.join(",") + "\n"));
        }
        std::fs::write(&wide, text).unwrap();
        app.open_in_import_tool(wide);
        let output = settle(&mut app, &ctx, SIZE);
        let preview = app.toolkits.apps.import.app.preview.as_ref().unwrap();
        assert_eq!((preview.rows, preview.cols), (200, 400));
        assert_eq!(
            app.toolkits.apps.import.app.created.as_ref().unwrap().size,
            [300, 400]
        );
        assert!(painted(&output, "c1") && painted(&output, "0"));
        assert!(
            !painted(&output, "c400"),
            "columns out of view are not laid out"
        );
        let texts = output
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::epaint::Shape::Text(_)))
            .count();
        assert!(texts < 2500, "{texts} text shapes for a 200 x 400 preview");
        // Closing keeps the inputs for the next open.
        app.close_tool_tab(Tool::App(AppKind::ImportTool));
        assert!(app.toolkits.tabs().next().is_none());
        assert!(app.toolkits.apps.import.app.source.is_some());
    }

    #[test]
    fn import_tool_generate_script_saves_header_and_the_same_script() {
        let dir = fixtures();
        let path = dir.path().join("import_events.m");
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.open_in_import_tool(dir.path().join("events.csv"));
        settle(&mut app, &ctx, SIZE);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.import.app.script(ws).unwrap()
        };
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, SIZE, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, SIZE, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Import Tool (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        assert!(app.toolkits.apps.import.message.starts_with("Saved"));
        let mut engine = matrust::Engine::default();
        engine.execute(&saved).unwrap();
        holds(&mut engine, "istable(events) && height(events) == 3");
        assert!(engine.array("opts").is_none());
    }

    #[test]
    fn import_tool_drop_opens_the_tool_and_keeps_workspace_documents_and_figure() {
        for size in [egui::vec2(960., 650.), SIZE] {
            let ctx = egui::Context::default();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("data.csv");
            std::fs::write(&path, "5,6\n7,8\n").unwrap();
            let mut app = folder_navigation_app(&ctx, dir.path());
            app.run("data=[1 2];Alias=data;keep=complex(3,4);plot(data);".into());
            finish(&mut app);
            app.selected = Some("keep".into());
            let before = app.variables.clone();
            let figure = app.figure.clone().unwrap();
            let text = app.documents[app.active].text.clone();
            let id = app.documents[app.active].id;
            drop_paths(&mut app, &ctx, vec![path.clone()]);
            finish(&mut app);
            settle(&mut app, &ctx, size);
            assert_eq!(chosen(&app), Some(path.canonicalize().unwrap()));
            assert_eq!(
                app.toolkits.selected_tool(),
                Some(Tool::App(AppKind::ImportTool))
            );
            // Opening is not importing: nothing in the workspace changed.
            assert_eq!(before.len(), app.variables.len());
            for (name, value) in &before {
                assert!(value.same_snapshot(&app.variables[name]), "{name}");
            }
            assert_eq!(app.selected.as_deref(), Some("keep"));
            assert_eq!(app.documents[app.active].id, id);
            assert_eq!(app.documents[app.active].text, text);
            assert!(app.figure.as_ref().unwrap().same_snapshot(&figure));
            // data is taken: Import makes data1 and keeps data and its alias.
            assert_eq!(app.toolkits.apps.import.app.name, "data1");
            if size != SIZE {
                // The compact layout puts OUTPUT under the preview.
                continue;
            }
            click_label_at(&mut app, &ctx, size, "Import");
            finish(&mut app);
            assert!(before["data"].same_snapshot(&app.variables["data"]));
            assert!(before["data"].same_snapshot(&app.variables["Alias"]));
            assert_eq!(
                app.variables["data1"]
                    .scalar(1, 1)
                    .unwrap()
                    .into_real()
                    .unwrap(),
                8.
            );
        }
    }

    #[test]
    fn import_tool_ribbon_menu_and_search_ask_for_a_file_once() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let picked = dir.path().join("values.csv");
        std::fs::write(&picked, "9\n").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path());
        app.layout.toolstrip_collapsed = false;
        app.set_mode(modes::Mode::Command);
        // Ribbon tile: one picker, the chosen file opens in the tool.
        app.toolkits.apps.test_pick = Some(picked.clone());
        click_label_at(&mut app, &ctx, SIZE, "Import data");
        assert_eq!(app.toolkits.apps.test_picks, 1);
        assert_eq!(chosen(&app), Some(picked.clone()));
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::ImportTool))
        );
        assert!(!app.variables.contains_key("values") && !app.busy);
        // Workspace menu.
        app.toolkits.deselect_tool();
        let other = dir.path().join("other.csv");
        std::fs::write(&other, "1,2\n").unwrap();
        app.toolkits.apps.test_pick = Some(other.clone());
        click_label_at(&mut app, &ctx, SIZE, "Menu");
        let mut top = None;
        for _ in 0..3 {
            top = frame_at(&mut app, &ctx, SIZE, vec![])
                .shapes
                .iter()
                .filter_map(|s| {
                    text_position(&s.shape, "Workspace").filter(|p| s.clip_rect.contains(*p))
                })
                .min_by(|a, b| a.y.total_cmp(&b.y));
        }
        let top = top.expect("Workspace menu is painted");
        for pressed in [true, false] {
            pointer_frame(&mut app, &ctx, SIZE, top, pressed);
        }
        click_label_at(&mut app, &ctx, SIZE, "Import data\u{2026}");
        assert_eq!(app.toolkits.apps.test_picks, 2);
        assert_eq!(chosen(&app), Some(other.clone()));
        // The Recent list keeps both files.
        let recent: Vec<&PathBuf> = app
            .toolkits
            .apps
            .import
            .app
            .recent
            .iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(recent, [&other, &picked]);
        // Cancel: nothing changes.
        app.toolkits.apps.test_pick = None;
        click_label_at(&mut app, &ctx, SIZE, "Import data");
        assert_eq!(app.toolkits.apps.test_picks, 3);
        assert_eq!(chosen(&app), Some(other));
        // Global search runs the same action after its popup closes.
        click_label_at(&mut app, &ctx, SIZE, "Search functions and actions");
        for _ in 0..2 {
            frame_at(&mut app, &ctx, SIZE, vec![]);
        }
        assert!(app.global_search.is_open());
        frame_at(
            &mut app,
            &ctx,
            SIZE,
            vec![egui::Event::Text("Import data".into())],
        );
        frame_at(&mut app, &ctx, SIZE, vec![]);
        app.toolkits.apps.test_pick = Some(picked.clone());
        command_first_key(
            &mut app,
            &ctx,
            SIZE,
            egui::Key::Enter,
            egui::Modifiers::NONE,
        );
        assert!(!app.global_search.is_open());
        assert_eq!(app.toolkits.apps.test_picks, 4);
        assert_eq!(chosen(&app), Some(picked));
        assert!(!app.busy && app.open_queue.is_empty());
    }

    #[test]
    fn import_tool_picker_opens_once_when_a_pass_is_discarded() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let picked = dir.path().join("picked.csv");
        std::fs::write(&picked, "7,8\n9,10\n").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path());
        app.open_in_import_tool(picked.clone());
        settle(&mut app, &ctx, SIZE);
        let mut point = None;
        for _ in 0..3 {
            point = frame_at(&mut app, &ctx, SIZE, vec![])
                .shapes
                .iter()
                .rev()
                .find_map(|s| text_position(&s.shape, "Choose file…"));
        }
        let point = point.expect("Choose file button");
        pointer_frame(&mut app, &ctx, SIZE, point, true);
        app.toolkits.apps.test_pick = Some(picked.clone());
        let mut passes = 0;
        crate::headless::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
                events: vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |root| {
                passes += 1;
                if passes == 1 {
                    root.ctx()
                        .request_discard("the picker waits for the final pass");
                }
                app.ui(root);
            },
        );
        assert!(passes >= 2);
        assert_eq!(app.toolkits.apps.test_picks, 1);
        frame_at(&mut app, &ctx, SIZE, vec![]);
        assert_eq!(
            app.toolkits.apps.test_picks, 1,
            "no replay on a later frame"
        );
    }

    #[test]
    fn import_tool_browser_double_click_opens_the_tool() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("browser.csv"), "17\n").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path());
        let mut pos = None;
        for _ in 0..3 {
            pos = frame_at(&mut app, &ctx, SIZE, vec![])
                .shapes
                .iter()
                .rev()
                .find_map(|s| text_position(&s.shape, "browser.csv"));
        }
        let pos = pos.expect("folder row");
        for _ in 0..2 {
            for pressed in [true, false] {
                pointer_frame(&mut app, &ctx, SIZE, pos, pressed);
            }
        }
        finish(&mut app);
        settle(&mut app, &ctx, SIZE);
        assert_eq!(
            chosen(&app),
            Some(dir.path().join("browser.csv").canonicalize().unwrap())
        );
        assert!(!app.variables.contains_key("browser"));
        click_label_at(&mut app, &ctx, SIZE, "Import");
        finish(&mut app);
        assert_eq!(
            app.variables["browser"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            17.
        );
    }

    #[test]
    fn import_tool_reserved_output_name_blocks_import() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valid.csv");
        std::fs::write(&path, "4\n").unwrap();
        let mut app = folder_navigation_app(&ctx, dir.path());
        app.open_in_import_tool(path);
        settle(&mut app, &ctx, SIZE);
        app.toolkits.apps.import.app.name = "while".into();
        app.toolkits.apps.import.app.name_edited = true;
        settle(&mut app, &ctx, SIZE);
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::ImportTool), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(text.contains("not a valid variable name"), "{text}");
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let original = std::mem::replace(&mut app.worker.send, send);
        click_label_at(&mut app, &ctx, SIZE, "Import");
        assert!(
            receive.try_recv().is_err(),
            "a disabled Import submits nothing"
        );
        app.worker.send = original;
        app.toolkits.apps.import.app.name = "chosen".into();
        settle(&mut app, &ctx, SIZE);
        click_label_at(&mut app, &ctx, SIZE, "Import");
        finish(&mut app);
        assert_eq!(
            app.variables["chosen"]
                .scalar(0, 0)
                .unwrap()
                .into_real()
                .unwrap(),
            4.
        );
    }
}
