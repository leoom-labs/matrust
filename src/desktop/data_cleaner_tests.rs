// Data Cleaner app (apps/data_cleaner.rs): the generated script gives
// exactly the table the app shows, a matrix input, removed rows and masks,
// DataVariables subsets, step order and enabling, retime, export without
// clobbering, errors and a rendered tab. Expected values are worked by hand
// from the published definitions (linear interpolation, the scaled-MAD
// median rule with k = 3, centered moving means that shrink at the ends,
// rescaling to [0, 1]).
mod data_cleaner_tests {
    use super::*;
    use apps::data_cleaner::{
        Cleaned, Column, DataCleaner, Mark, NormMethod, Op, RetimeMethod, StepKind, TimeStep,
        TimeUnit,
    };
    use apps::{App, AppKind, Host, Snapshot};
    use toolkits::Tool;

    /// A timetable with two missing values and two outliers in `a`, one
    /// missing value in `b` and a string variable.
    const SETUP: &str = "t = datetime(2024,1,1) + hours(0:11)'; \
        a = [1 2 NaN 4 5 50 7 8 NaN 10 11 -40]'; \
        b = (2:2:24)'; b(4) = NaN; \
        s = [\"x\";\"y\";missing;\"z\";\"a\";\"b\";\"c\";\"d\";\"e\";\"f\";\"g\";\"h\"]; \
        TT = timetable(t, a, b, s);";

    fn workspace(setup: &str) -> Snapshot {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        let mut ws = Snapshot::default();
        ws.sync(&engine.workspace());
        ws
    }
    fn cleaner(data: &str, ws: &Snapshot) -> Host<DataCleaner> {
        let mut host = Host::new(DataCleaner::default());
        host.app.data = data.into();
        host.app.prepare(ws);
        host
    }
    /// One private run, synchronously, delivered to the app.
    fn clean(host: &mut Host<DataCleaner>, ws: &Snapshot) -> Result<(), String> {
        host.app.prepare(ws);
        let request = host.request(ws)?;
        if let Ok(dir) = std::env::var("DATA_CLEANER_DUMP") {
            let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
            std::fs::write(format!("{dir}/run{n}.m"), &request.code).unwrap();
        }
        let outputs = apps::run(&mut matrust::Engine::default(), request)?;
        host.app.receive(&outputs)
    }
    fn result(host: &Host<DataCleaner>) -> &Cleaned {
        host.app.result.as_ref().expect("a cleaned table")
    }
    fn numbers(r: &Cleaned, name: &str) -> Vec<f64> {
        let k = r.names.iter().position(|n| n == name).expect(name);
        match &r.columns[k] {
            Column::Numbers(v) => v.clone(),
            other => panic!("{name}: {other:?}"),
        }
    }
    fn close(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
        for (a, e) in actual.iter().zip(expected) {
            assert!(
                (a - e).abs() <= 1e-12 * e.abs().max(1.) || (a.is_nan() && e.is_nan()),
                "{actual:?} vs {expected:?}"
            );
        }
    }
    /// `script` in a fresh Engine (after `setup`) leaves `out` equal (with
    /// equal missing values) to the table the app shows.
    fn reproduces(setup: &str, script: &str, out: &str, shown: &matrust::Array) {
        let mut engine = matrust::Engine::default();
        engine.execute(setup).unwrap();
        engine
            .execute(script)
            .unwrap_or_else(|e| panic!("{script}\n{e}"));
        engine.insert_array("shown__", shown.clone()).unwrap();
        engine
            .execute(&format!("same__ = isequaln({out}, shown__);"))
            .unwrap();
        assert!(
            matches!(engine.array("same__"), Some(matrust::Array::Logical(l)) if l.data() == [1]),
            "{script}"
        );
    }
    fn set<T>(host: &mut Host<DataCleaner>, k: usize, edit: impl FnOnce(&mut Op) -> T) -> T {
        edit(&mut host.app.steps[k].op)
    }

    #[test]
    fn data_cleaner_pipeline_script_reproduces_the_table_shown() {
        let ws = workspace(SETUP);
        let mut host = cleaner("TT", &ws);
        // New data ticks its numeric variables.
        assert_eq!(host.app.ticks, ["a", "b"]);
        for kind in [
            StepKind::Missing,
            StepKind::Outliers,
            StepKind::Smooth,
            StepKind::Normalize,
        ] {
            host.app.add(kind);
        }
        set(&mut host, 2, |op| {
            if let Op::Smooth(s) = op {
                s.window = 3;
            }
        });
        set(&mut host, 3, |op| {
            if let Op::Normalize(n) = op {
                n.method = NormMethod::Range;
            }
        });
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert_eq!(
            script,
            "% Clean TT (12 rows, 3 variables), one statement per step.\n\
             cleanedData = TT;\n\
             cleanedData = fillmissing(cleanedData, 'linear', 'DataVariables', {'a', 'b'});\n\
             cleanedData = filloutliers(cleanedData, 'linear', 'median', 'DataVariables', {'a', 'b'});\n\
             cleanedData{:, {'a', 'b'}} = smoothdata(cleanedData{:, {'a', 'b'}}, 1, 'movmean', 3);\n\
             cleanedData{:, {'a', 'b'}} = normalize(cleanedData{:, {'a', 'b'}}, 1, 'range');\n"
        );
        let r = result(&host);
        reproduces(SETUP, &script, "cleanedData", &r.table);
        // fillmissing: a(3) = 3, a(9) = 9, b(4) = 8. Outliers by the median
        // rule (median 6, MAD 3.5, 3 * 1.4826 * 3.5 = 15.57): 50 and -40,
        // refilled linearly to 6 and 12 (end: extrapolated). So a = 1..12 and
        // b = 2:2:24; a 3-point moving mean keeps the inside and makes the
        // ends 1.5, 11.5 (3, 23); range rescales to [0, 1].
        let mut a: Vec<f64> = (1..=12).map(f64::from).collect();
        a[0] = 1.5;
        a[11] = 11.5;
        let b: Vec<f64> = (1..=12)
            .map(|k| match k {
                1 => 3.,
                12 => 23.,
                k => 2. * f64::from(k),
            })
            .collect();
        close(
            &numbers(r, "a"),
            &a.iter().map(|v| (v - 1.5) / 10.).collect::<Vec<_>>(),
        );
        close(
            &numbers(r, "b"),
            &b.iter().map(|v| (v - 3.) / 20.).collect::<Vec<_>>(),
        );
        assert_eq!(r.rows, 12);
        assert_eq!(r.heights, [12, 12, 12, 12, 12]);
        // Every a and b cell changed, s did not.
        assert_eq!(r.changed_count(), 24);
        assert!(r.is_changed(0, 0) && !r.is_changed(2, 2));
        assert_eq!(r.cell(2, 2), "<missing>");
        assert_eq!(r.time(0), "01-Jan-2024 00:00:00");
        // Plot of a: two filled missing values, two outliers found and
        // filled, x in seconds from the first row time.
        assert_eq!(r.plot.as_deref(), Some("a"));
        assert!(r.time_axis);
        let marks = |mark: Mark| -> Vec<[f64; 2]> {
            r.marks
                .iter()
                .filter(|m| m.1 == mark)
                .map(|m| m.2)
                .collect()
        };
        assert_eq!(marks(Mark::Outlier), [[18000., 50.], [39600., -40.]]);
        assert_eq!(
            marks(Mark::Filled),
            [[7200., 3.], [28800., 9.], [18000., 6.], [39600., 12.]]
        );
        assert!(marks(Mark::Removed).is_empty());
        // Thresholds of step 2 (index 1): median 6 +- 3 * 1.4826 * 3.5.
        assert_eq!(r.thresholds.len(), 12);
        let [x, lower, upper, center] = r.thresholds[0].1;
        let spread = 3. * 1.482_602_218_505_601_8 * 3.5;
        assert_eq!((r.thresholds[0].0, x, center), (1, 0., 6.));
        assert!((lower - (6. - spread)).abs() < 1e-12 && (upper - (6. + spread)).abs() < 1e-12);
        assert_eq!(r.original.len(), 10, "NaN points are not drawn");
        assert_eq!(r.cleaned.len(), 12);
        // Summary of a before: 12 values, 2 missing, 2 outliers; min -40,
        // median 6, max 50, mean 5.8 and the sample std of the ten values.
        let before = r.before["a"];
        let data = [1., 2., 4., 5., 50., 7., 8., 10., 11., -40.];
        let std = (data.iter().map(|v| (v - 5.8f64).powi(2)).sum::<f64>() / 9.).sqrt();
        close(&before, &[12., 2., 2., -40., 6., 50., 5.8, std]);
        let after = r.after["a"];
        close(&after[..4], &[12., 0., 0., 0.]);
        assert_eq!((after[5], r.before["b"][1]), (1., 1.));
        // Missing values per variable for the Inputs list.
        assert_eq!(
            r.missing
                .iter()
                .map(|(k, v)| (k.as_str(), *v))
                .collect::<Vec<_>>(),
            [("a", 2.), ("b", 1.), ("s", 1.)]
        );
        // The Output block.
        let rows = host.app.results();
        assert_eq!(rows[0], ("Rows", "12".to_owned()));
        assert_eq!(rows[1], ("Variables", "3".to_owned()));
        assert_eq!(rows[2], ("Rows removed", "none".to_owned()));
        assert_eq!(rows[3], ("Changed cells", "24".to_owned()));
        assert_eq!(host.app.exported(&ws), ["cleanedData"]);
        // Options at their defaults are left out; others appear.
        set(&mut host, 1, |op| {
            if let Op::Outliers(o) = op {
                o.threshold = 2.;
            }
        });
        set(&mut host, 2, |op| {
            if let Op::Smooth(s) = op {
                s.by_window = false;
                s.factor = 0.4;
            }
        });
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("'median', 'ThresholdFactor', 2, 'DataVariables'")
                && script.contains("'movmean', 'SmoothingFactor', 0.4);"),
            "{script}"
        );
        clean(&mut host, &ws).unwrap();
        reproduces(SETUP, &script, "cleanedData", &result(&host).table);
    }

    #[test]
    fn data_cleaner_numeric_matrix_becomes_var1_to_varn() {
        let setup = "X = [1 NaN 3; 4 5 6; 7 8 NaN; 10 11 12];";
        let ws = workspace(setup);
        let mut host = cleaner("X", &ws);
        assert_eq!(host.app.ticks, ["Var1", "Var2", "Var3"]);
        host.app.add(StepKind::Missing);
        set(&mut host, 0, |op| {
            if let Op::Missing(m) = op {
                m.fill = apps::data_cleaner::MissingFill::Previous;
            }
        });
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        // Every variable is ticked: no DataVariables.
        assert_eq!(
            script,
            "% Clean X (4 rows, 3 variables), one statement per step.\n\
             cleanedData = array2table(X, 'VariableNames', \"Var\" + (1:3));\n\
             cleanedData = fillmissing(cleanedData, 'previous');\n"
        );
        let r = result(&host);
        assert_eq!(r.names, ["Var1", "Var2", "Var3"]);
        assert!(r.times.is_none() && !r.time_axis);
        // previous: Var2(1) has no earlier value and stays NaN; Var3(3) = 6.
        close(&numbers(r, "Var2"), &[f64::NAN, 5., 8., 11.]);
        close(&numbers(r, "Var3"), &[3., 6., 6., 12.]);
        assert_eq!(r.changed_count(), 1);
        assert!(r.is_changed(2, 2));
        assert_eq!(r.cell(0, 1), "NaN");
        reproduces(setup, &script, "cleanedData", &r.table);
        // A table works the same way, without the conversion.
        let setup = "T = array2table([1 NaN; 2 3], 'VariableNames', {'p', 'q'});";
        let ws = workspace(setup);
        let mut host = cleaner("T", &ws);
        host.app.add(StepKind::Missing);
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(script.contains("cleanedData = T;\n"), "{script}");
        reproduces(setup, &script, "cleanedData", &result(&host).table);
    }

    #[test]
    fn data_cleaner_other_variable_types_in_a_table() {
        let setup = "T = table([1;NaN;3;4], categorical({'a';'';'b';'a'}), {1;2;3;4}, \
                     [true;false;true;true], 'VariableNames', {'p', 'c', 'k', 'l'});";
        let ws = workspace(setup);
        let mut host = cleaner("T", &ws);
        // Only p is numeric and single-column; logical is not ticked.
        assert_eq!(host.app.ticks, ["p"]);
        host.app.ticks = vec!["p".into(), "c".into()];
        host.app.add(StepKind::Missing);
        set(&mut host, 0, |op| {
            if let Op::Missing(m) = op {
                m.fill = apps::data_cleaner::MissingFill::Previous;
            }
        });
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.ends_with(
                "cleanedData = fillmissing(cleanedData, 'previous', 'DataVariables', {'p', 'c'});\n"
            ),
            "{script}"
        );
        let r = result(&host);
        assert_eq!((r.cell(1, 0).as_str(), r.cell(1, 1).as_str()), ("1", "a"));
        assert!(r.is_changed(1, 0) && r.is_changed(1, 1) && !r.is_changed(1, 3));
        assert_eq!(r.changed_count(), 2);
        assert_eq!(r.cell(0, 2), "…", "a cell of numbers has no text");
        assert_eq!(r.cell(1, 3), "0");
        reproduces(setup, &script, "cleanedData", &r.table);
        // Removing rows: a plain table plots against the source row numbers.
        set(&mut host, 0, |op| {
            if let Op::Missing(m) = op {
                m.remove = true;
            }
        });
        clean(&mut host, &ws).unwrap();
        let r = result(&host);
        assert_eq!(r.cell(1, 1), "b");
        assert!(!r.time_axis);
        assert_eq!(r.cleaned, [[1., 1.], [3., 3.], [4., 4.]]);
        assert_eq!(r.original, [[1., 1.], [3., 3.], [4., 4.]]);
        reproduces(
            setup,
            &host.app.script(&ws).unwrap(),
            "cleanedData",
            &r.table,
        );
    }

    #[test]
    fn data_cleaner_removed_rows_per_step_and_masks() {
        let ws = workspace(SETUP);
        let mut host = cleaner("TT", &ws);
        host.app.add(StepKind::Missing);
        host.app.add(StepKind::Outliers);
        for k in 0..2 {
            set(&mut host, k, |op| match op {
                Op::Missing(m) => m.remove = true,
                Op::Outliers(o) => o.remove = true,
                _ => {}
            });
        }
        clean(&mut host, &ws).unwrap();
        // Rows 3, 4 and 9 miss a or b; then 50 and -40 are outliers of the
        // nine rows left (median 7, MAD 4, bounds 7 +- 3 * 1.4826 * 4).
        let r = result(&host);
        assert_eq!(r.heights, [12, 9, 7]);
        close(&numbers(r, "a"), &[1., 2., 5., 7., 8., 10., 11.]);
        assert_eq!(r.changed_count(), 0, "removing rows changes no cell");
        let rows = host.app.results();
        assert_eq!(rows[0], ("Rows", "12 → 7".to_owned()));
        assert_eq!(
            rows[2],
            ("Rows removed", "3 by step 1 · 2 by step 2".to_owned())
        );
        // Removed rows of a are marked where they were.
        let removed: Vec<[f64; 2]> = r
            .marks
            .iter()
            .filter(|m| m.1 == Mark::Removed)
            .map(|m| m.2)
            .collect();
        assert_eq!(removed, [[10800., 4.]], "rows with a NaN a are not drawn");
        assert_eq!(r.marks.iter().filter(|m| m.1 == Mark::Outlier).count(), 2);
        // Masks on request: one per step, in the script and the export.
        host.app.masks = true;
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains(
                "[cleanedData, removedMissing] = rmmissing(cleanedData, 'DataVariables', {'a', 'b'});\n\
                 [cleanedData, removedOutliers] = rmoutliers(cleanedData, 'median', 'DataVariables', {'a', 'b'});\n"
            ),
            "{script}"
        );
        assert_eq!(
            host.app.exported(&ws),
            ["cleanedData", "removedMissing", "removedOutliers"]
        );
        clean(&mut host, &ws).unwrap();
        let mut engine = matrust::Engine::default();
        engine.execute(SETUP).unwrap();
        engine.execute(&script).unwrap();
        engine
            .execute(
                "ok__ = isequal(removedMissing, logical([0 0 1 1 0 0 0 0 1 0 0 0]')) && \
                 isequal(removedOutliers, logical([0 0 0 1 0 0 0 0 1]'));",
            )
            .unwrap();
        assert!(
            matches!(engine.array("ok__"), Some(matrust::Array::Logical(l)) if l.data() == [1])
        );
        reproduces(SETUP, &script, "cleanedData", &result(&host).table);
    }

    #[test]
    fn data_cleaner_steps_act_only_on_ticked_variables() {
        let ws = workspace(SETUP);
        let mut host = cleaner("TT", &ws);
        host.app.ticks = vec!["a".into()];
        host.app.add(StepKind::Missing);
        assert_eq!(host.app.steps[0].vars, ["a"]);
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("fillmissing(cleanedData, 'linear', 'DataVariables', {'a'});"),
            "{script}"
        );
        let r = result(&host);
        close(&numbers(r, "a")[..4], &[1., 2., 3., 4.]);
        assert!(numbers(r, "b")[3].is_nan(), "b is not ticked");
        assert_eq!(r.changed_count(), 2);
        reproduces(SETUP, &script, "cleanedData", &r.table);
        // Every variable ticked (the string too): no DataVariables, and the
        // string's missing value is filled from the previous row.
        host.app.steps[0].vars = vec!["a".into(), "b".into(), "s".into()];
        set(&mut host, 0, |op| {
            if let Op::Missing(m) = op {
                m.fill = apps::data_cleaner::MissingFill::Previous;
            }
        });
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.ends_with("cleanedData = fillmissing(cleanedData, 'previous');\n"),
            "{script}"
        );
        let r = result(&host);
        assert_eq!(r.cell(2, 2), "y");
        assert!(r.is_changed(2, 2));
        reproduces(SETUP, &script, "cleanedData", &r.table);
        // A step after a rename sees the new name.
        host.app.selected = Some(0);
        host.app.ticks = vec!["b".into()];
        host.app.add(StepKind::Rename);
        set(&mut host, 1, |op| {
            if let Op::Rename(map) = op {
                map.insert("b".into(), "pressure".into());
            }
        });
        host.app.ticks = vec!["pressure".into()];
        host.app.add(StepKind::Normalize);
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.contains("cleanedData = renamevars(cleanedData, {'b'}, {'pressure'});\n")
                && script.contains("cleanedData{:, {'pressure'}} = normalize(cleanedData{:, {'pressure'}}, 1, 'zscore');"),
            "{script}"
        );
        let r = result(&host);
        assert_eq!(r.names, ["a", "pressure", "s"]);
        assert!(
            r.after.contains_key("b"),
            "after statistics keep the source name"
        );
        reproduces(SETUP, &script, "cleanedData", &r.table);
    }

    #[test]
    fn data_cleaner_reorder_disable_and_delete_change_the_script() {
        let ws = workspace(SETUP);
        let mut host = cleaner("TT", &ws);
        host.app.add(StepKind::Missing);
        host.app.add(StepKind::Smooth);
        let lines = |host: &Host<DataCleaner>| -> Vec<String> {
            host.app
                .script(&ws)
                .unwrap()
                .lines()
                .skip(2)
                .map(|l| l.split('(').next().unwrap().to_owned())
                .collect()
        };
        assert_eq!(
            lines(&host),
            [
                "cleanedData = fillmissing",
                "cleanedData{:, {'a', 'b'}} = smoothdata"
            ]
        );
        // Smoothing first: the NaN windows shrink around the gaps.
        host.app.move_up(1);
        assert_eq!(host.app.selected, Some(0));
        assert_eq!(
            lines(&host),
            [
                "cleanedData{:, {'a', 'b'}} = smoothdata",
                "cleanedData = fillmissing"
            ]
        );
        clean(&mut host, &ws).unwrap();
        let first = result(&host).table.clone();
        reproduces(SETUP, &host.app.script(&ws).unwrap(), "cleanedData", &first);
        host.app.move_down(0);
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        reproduces(SETUP, &script, "cleanedData", &result(&host).table);
        // A disabled step leaves the script and the run.
        host.app.steps[1].enabled = false;
        assert_eq!(lines(&host), ["cleanedData = fillmissing"]);
        clean(&mut host, &ws).unwrap();
        assert_eq!(result(&host).heights, [12, 12]);
        assert_eq!(result(&host).steps, [0]);
        host.app.delete(0);
        assert_eq!(host.app.steps.len(), 1);
        assert_eq!(host.app.selected, Some(0));
        assert!(lines(&host).is_empty());
    }

    #[test]
    fn data_cleaner_retime_a_timetable() {
        let ws = workspace(SETUP);
        let mut host = cleaner("TT", &ws);
        host.app.ticks = vec!["s".into()];
        host.app.add(StepKind::Remove);
        host.app.add(StepKind::Retime);
        set(&mut host, 1, |op| {
            if let Op::Retime(r) = op {
                r.step = TimeStep::Custom;
                r.every = 30.;
                r.unit = TimeUnit::Minutes;
            }
        });
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.ends_with(
                "cleanedData = removevars(cleanedData, {'s'});\n\
                 cleanedData = retime(cleanedData, (cleanedData.Properties.RowTimes(1):minutes(30):\
                 cleanedData.Properties.RowTimes(end))', 'linear');\n"
            ),
            "{script}"
        );
        let r = result(&host);
        // 00:00 to 11:00 every 30 minutes: 23 rows; halfway values are the
        // means of their neighbours (NaN next to a missing value).
        assert_eq!(r.heights, [12, 12, 23]);
        assert_eq!(r.names, ["a", "b"]);
        let a = numbers(r, "a");
        close(&a[..4], &[1., 1.5, 2., f64::NAN]);
        assert_eq!(r.time(1), "01-Jan-2024 00:30:00");
        assert!(r.changed.is_empty(), "retimed rows are new");
        assert_eq!(
            host.app.results()[3],
            ("Rows added", "11 by step 2".to_owned())
        );
        reproduces(SETUP, &script, "cleanedData", &r.table);
        // A named step and a bin method; the default method is left out.
        set(&mut host, 1, |op| {
            if let Op::Retime(r) = op {
                r.step = TimeStep::Daily;
                r.method = RetimeMethod::Mean;
            }
        });
        clean(&mut host, &ws).unwrap();
        let script = host.app.script(&ws).unwrap();
        assert!(
            script.ends_with("retime(cleanedData, 'daily', 'mean');\n"),
            "{script}"
        );
        assert_eq!(result(&host).rows, 1);
        close(&numbers(result(&host), "a"), &[5.8]);
        set(&mut host, 1, |op| {
            if let Op::Retime(r) = op {
                r.method = RetimeMethod::FillWithMissing;
            }
        });
        assert!(
            host.app
                .script(&ws)
                .unwrap()
                .ends_with("retime(cleanedData, 'daily');\n")
        );
        // A table has no row times to retime.
        let ws = workspace("T = table([1;2], 'VariableNames', {'p'});");
        let mut host = cleaner("T", &ws);
        host.app.add(StepKind::Retime);
        let error = host.app.script(&ws).unwrap_err();
        assert!(
            error.contains("Step 1 (Retime timetable) needs a timetable"),
            "{error}"
        );
    }

    #[test]
    fn data_cleaner_errors_name_the_problem_and_keep_the_last_result() {
        let ws = workspace(&format!("{SETUP} E = TT([], :); M = {{1, 2}};"));
        let mut host = cleaner("TT", &ws);
        host.app.add(StepKind::Missing);
        clean(&mut host, &ws).unwrap();
        let good = result(&host).table.clone();
        // A string variable picked for smoothing: named before any run.
        host.app.ticks = vec!["a".into(), "s".into()];
        host.app.add(StepKind::Smooth);
        let error = host.request(&ws).err().unwrap();
        assert!(
            error.contains("Step 2 (Smooth data): s is a string variable")
                && error.contains("numeric"),
            "{error}"
        );
        assert!(host.app.exported(&ws).is_empty());
        host.app.steps[1].vars = vec!["a".into()];
        assert!(host.app.script(&ws).is_ok());
        // No variable ticked.
        host.app.steps[1].vars.clear();
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("tick the variables"), "{error}");
        host.app.delete(1);
        // An empty table, a cell array and a missing variable.
        for (data, part) in [
            ("E", "E has no rows to clean"),
            ("M", "M is not a table, timetable or real numeric matrix"),
            ("gone", "gone does not exist"),
        ] {
            host.app.data = data.into();
            let error = host.app.script(&ws).unwrap_err();
            assert!(error.contains(part), "{error}");
        }
        host.app.data = "TT".into();
        // The output cannot replace the data; names must be valid.
        host.app.output_edited = true;
        host.app.output_name = "TT".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("other than the data's")
        );
        host.app.output_name = "1x".into();
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("not a valid variable name")
        );
        host.app.output_edited = false;
        // An engine error (a fill method the engine lacks) reaches the
        // status line through the runner; the last good table stays.
        set(&mut host, 0, |op| {
            if let Op::Missing(m) = op {
                m.fill = apps::data_cleaner::MissingFill::Spline;
            }
        });
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            host.refresh(&ctx, &ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let (text, status) = host.status();
        assert_eq!(status, toolkits::AppStatus::Error);
        assert!(
            text.contains("fillmissing") && text.contains("spline"),
            "{text}"
        );
        assert!(result(&host).table.same_snapshot(&good));
        // Renaming to a taken or invalid name.
        set(&mut host, 0, |op| {
            if let Op::Missing(m) = op {
                m.fill = apps::data_cleaner::MissingFill::Linear;
            }
        });
        host.app.ticks = vec!["a".into()];
        host.app.add(StepKind::Rename);
        set(&mut host, 1, |op| {
            if let Op::Rename(map) = op {
                map.insert("a".into(), "b".into());
            }
        });
        let error = host.app.script(&ws).unwrap_err();
        assert!(error.contains("two variables would be named b"), "{error}");
        set(&mut host, 1, |op| {
            if let Op::Rename(map) = op {
                map.insert("a".into(), "not valid".into());
            }
        });
        assert!(
            host.app
                .script(&ws)
                .unwrap_err()
                .contains("not a valid variable name")
        );
    }

    fn open_cleaner(
        app: &mut Workspace,
        ctx: &egui::Context,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        app.toolkits.open_tool(Tool::App(AppKind::DataCleaner));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            let host = &app.toolkits.apps.cleaner;
            if host.app.result.is_some() && !host.running() {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline, "{:?}", host.status());
            drop(output);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// Frames until the cleaner's run for the current inputs finished.
    fn settle(app: &mut Workspace, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = frame_at(app, ctx, size, vec![]);
            if !app.toolkits.apps.cleaner.running() {
                return frame_at(app, ctx, size, vec![]);
            }
            assert!(Instant::now() < deadline);
            drop(output);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn painted(output: &egui::FullOutput, label: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, label).is_some())
    }

    #[test]
    fn data_cleaner_tab_renders_three_areas_and_every_view() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(SETUP.into());
        finish(&mut app);
        open_cleaner(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.apps.cleaner.app.data, "TT",
            "the first candidate is chosen"
        );
        app.toolkits.apps.cleaner.app.add(StepKind::Missing);
        app.toolkits.apps.cleaner.app.add(StepKind::Outliers);
        let output = settle(&mut app, &ctx, size);
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(Tool::App(AppKind::DataCleaner))
        );
        for label in [
            "INPUTS",
            "CLEANED DATA",
            "OUTPUT",
            "Data",
            "Plot",
            "Summary",
            "Variables",
            "Cleaning steps",
            "Add step",
            "Step 2 · Clean outlier data",
            "Export to workspace",
            "Generate script",
            "Cleaned table name",
            "Time",
            "01-Jan-2024 00:00:00",
            "<missing>",
            "Rows",
            "Changed cells",
        ] {
            assert!(painted(&output, label), "{label} not painted");
        }
        let (text, status) = app
            .toolkits
            .app_status(Tool::App(AppKind::DataCleaner), false)
            .unwrap();
        assert_eq!(status, toolkits::AppStatus::Success, "{text}");
        assert!(
            text.starts_with("TT: 12 rows · 2 enabled steps · 5 cells changed"),
            "{text}"
        );
        // The Plot and Summary views render.
        click_label_at(&mut app, &ctx, size, "Plot");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        assert_eq!(
            app.toolkits.apps.cleaner.app.view,
            apps::data_cleaner::View::Plot
        );
        assert!(painted(&output, "Variable"));
        click_label_at(&mut app, &ctx, size, "Summary");
        let output = frame_at(&mut app, &ctx, size, vec![]);
        for label in ["Median", "Outliers", "2 → 0"] {
            assert!(painted(&output, label), "{label} not painted");
        }
        // A narrow host stacks OUTPUT under the main view and still renders.
        let output = frame_at(&mut app, &ctx, egui::vec2(900., 700.), vec![]);
        assert!(painted(&output, "OUTPUT"));
        // Closing keeps the inputs for the next open.
        app.close_tool_tab(Tool::App(AppKind::DataCleaner));
        assert!(app.toolkits.tabs().next().is_none());
        assert_eq!(app.toolkits.apps.cleaner.app.steps.len(), 2);
    }

    #[test]
    fn data_cleaner_export_names_do_not_clobber() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(format!("{SETUP} cleanedData = 7; removedMissing = 'keep';"));
        finish(&mut app);
        app.toolkits.apps.cleaner.app.data = "TT".into();
        open_cleaner(&mut app, &ctx, size);
        app.toolkits.apps.cleaner.app.add(StepKind::Missing);
        app.toolkits.apps.cleaner.app.masks = true;
        set(&mut app.toolkits.apps.cleaner, 0, |op| {
            if let Op::Missing(m) = op {
                m.remove = true;
            }
        });
        settle(&mut app, &ctx, size);
        let shown = result(&app.toolkits.apps.cleaner).table.clone();
        click_label_at(&mut app, &ctx, size, "Export to workspace");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        assert!(
            matches!(&app.variables["cleanedData"], matrust::Array::Numeric(m) if m.data() == [7.]),
            "cleanedData is kept"
        );
        assert!(matches!(
            &app.variables["removedMissing"],
            matrust::Array::Character(_)
        ));
        assert!(
            app.variables["cleanedData1"].same_snapshot(&shown) || {
                let mut engine = matrust::Engine::default();
                engine
                    .insert_array("x__", app.variables["cleanedData1"].clone())
                    .unwrap();
                engine.insert_array("y__", shown.clone()).unwrap();
                engine.execute("ok__ = isequaln(x__, y__);").unwrap();
                matches!(engine.array("ok__"), Some(matrust::Array::Logical(l)) if l.data() == [1])
            }
        );
        assert!(
            matches!(&app.variables["removedMissing1"], matrust::Array::Logical(l) if l.len() == 12)
        );
        assert!(
            app.toolkits
                .apps
                .cleaner
                .message
                .contains("cleanedData1, removedMissing1")
        );
        // The exported names are taken now: the next defaults move on.
        frame_at(&mut app, &ctx, size, vec![]);
        let ws = &app.toolkits.apps.workspace;
        assert_eq!(
            app.toolkits.apps.cleaner.app.exported(ws),
            ["cleanedData2", "removedMissing2"]
        );
    }

    #[test]
    fn data_cleaner_generate_script_saves_header_and_the_same_script() {
        let size = egui::vec2(1440., 900.);
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clean.m");
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.run(SETUP.into());
        finish(&mut app);
        open_cleaner(&mut app, &ctx, size);
        app.toolkits.apps.cleaner.app.add(StepKind::Missing);
        app.toolkits.apps.cleaner.app.add(StepKind::Sort);
        set(&mut app.toolkits.apps.cleaner, 1, |op| {
            if let Op::Sort { descend } = op {
                *descend = true;
            }
        });
        settle(&mut app, &ctx, size);
        let script = {
            let ws = &app.toolkits.apps.workspace;
            app.toolkits.apps.cleaner.app.script(ws).unwrap()
        };
        assert!(
            script.ends_with("sortrows(cleanedData, {'a', 'b'}, 'descend');\n"),
            "{script}"
        );
        app.toolkits.apps.test_save_path = Some(path.clone());
        click_label_at(&mut app, &ctx, size, "Generate script");
        finish(&mut app);
        frame_at(&mut app, &ctx, size, vec![]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("% Data Cleaner (MatRust app), generated "),
            "{saved}"
        );
        assert!(saved.ends_with(&script), "{saved}");
        assert!(
            app.toolkits.apps.cleaner.message.starts_with("Saved"),
            "{}",
            app.toolkits.apps.cleaner.message
        );
        // The saved file reproduces the table shown, sorted rows included.
        let r = result(&app.toolkits.apps.cleaner);
        // a descending: 50 (b 12), 11 (b 22), 10 (b 20).
        close(&numbers(r, "b")[..3], &[12., 22., 20.]);
        reproduces(SETUP, &saved, "cleanedData", &r.table);
    }
}
