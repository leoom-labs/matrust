// Expected rows/counts are constructed by hand, never sampled from MatRust.
mod comparison_tool_tests {
    use super::*;
    use apps::comparison_tool::{ComparisonTool, Counts, Spec, View, path_literal, row_tint};
    use apps::{App, AppKind, Host, Snapshot};
    use matrust::engine::compare::{
        self, Comparison, ComparisonType, DiffStatus, DifferenceReason, HunkKind, TextDiffOptions,
    };
    use std::sync::atomic::AtomicBool;

    fn files(dir: &std::path::Path, a: &str, b: &str) -> Spec {
        let left = dir.join("left.txt");
        let right = dir.join("right.txt");
        std::fs::write(&left, a).unwrap();
        std::fs::write(&right, b).unwrap();
        Spec {
            left: left.to_str().unwrap().into(),
            right: right.to_str().unwrap().into(),
            ..Default::default()
        }
    }
    fn view(a: &str, b: &str) -> View {
        View::build(
            Spec {
                left: "left.txt".into(),
                right: "right.txt".into(),
                ..Default::default()
            },
            Comparison::Text(compare::text_diff(a, b, TextDiffOptions::default()).unwrap()),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    fn wait(host: &mut Host<ComparisonTool>, ctx: &egui::Context, ws: &Snapshot) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            host.refresh(ctx, ws);
            if !host.running() {
                break;
            }
            assert!(Instant::now() < deadline, "comparison worker timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn start(spec: Spec) -> (Host<ComparisonTool>, egui::Context) {
        let mut host = Host::new(ComparisonTool::default());
        host.app.draft = spec;
        host.app.compare();
        let ctx = egui::Context::default();
        wait(&mut host, &ctx, &Snapshot::default());
        (host, ctx)
    }

    #[test]
    fn comparison_tool_text_rows_tints_inline_navigation_and_context() {
        let a = "same\nold cat\nanchor\ndelete me\nend\n";
        let b = "same\nnew cat\nanchor\nend\ninsert me\n";
        let mut app = ComparisonTool::default();
        app.current = Some(Arc::new(view(a, b)));
        let v = app.current.as_ref().unwrap();
        let rows = v
            .rows
            .iter()
            .map(|r| (r.left, r.right, r.kind))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                (Some(1), Some(1), HunkKind::Equal),
                (Some(2), Some(2), HunkKind::Replace),
                (Some(3), Some(3), HunkKind::Equal),
                (Some(4), None, HunkKind::Delete),
                (Some(5), Some(4), HunkKind::Equal),
                (None, Some(5), HunkKind::Insert),
            ]
        );
        assert_eq!(
            v.counts,
            Counts {
                changed: 1,
                inserted: 1,
                deleted: 1,
                same: 3
            }
        );
        for ranges in &v.rows[1].inline {
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0], 0..3);
        }
        assert_eq!(v.changes, [1, 3, 5]);
        assert_eq!(app.navigate(true), Some(1));
        assert_eq!(app.navigate(true), Some(3));
        assert_eq!(app.navigate(true), Some(5));
        assert_eq!(app.navigate(true), Some(1));
        assert_eq!(app.navigate(false), Some(5));
        assert_eq!(app.navigate(false), Some(3));
        let ctx = egui::Context::default();
        let p = tokens::palette(&ctx);
        assert_eq!(
            row_tint(HunkKind::Replace, 0, p),
            p.warning.gamma_multiply(0.12)
        );
        assert_eq!(
            row_tint(HunkKind::Insert, 1, p),
            p.success.gamma_multiply(0.12)
        );
        assert_eq!(
            row_tint(HunkKind::Delete, 0, p),
            p.error.gamma_multiply(0.12)
        );
        assert_eq!(row_tint(HunkKind::Insert, 0, p), p.surface);
        let a = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
        let b = a.replace("line 15\n", "replacement\n");
        let v = view(&a, &b);
        assert_eq!(
            v.context,
            (12..19).collect::<Vec<_>>(),
            "exactly 3 context lines on each side"
        );
    }

    #[test]
    fn comparison_tool_text_filters_and_unicode_inline_ranges() {
        for (a, b, whitespace, case, endings) in [
            ("a = 1;\n", "a  =  1;\n", true, false, false),
            ("Alpha\n", "alpha\n", false, true, false),
            ("ΟΣ\n", "οσ\n", false, true, false),
            ("a\r\nb\r\n", "a\nb\n", false, false, true),
        ] {
            let raw = compare::text_diff(a, b, TextDiffOptions::default()).unwrap();
            assert!(raw.hunks.iter().any(|h| h.kind != HunkKind::Equal));
            let v = View::build(
                Spec {
                    left: "a".into(),
                    right: "b".into(),
                    ignore_whitespace: whitespace,
                    ignore_case: case,
                    ignore_line_endings: endings,
                    ..Default::default()
                },
                Comparison::Text(raw),
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(v.changes.is_empty());
            assert!(v.report.is_empty());
        }
        let v = view("αβγ\n", "αδγ\n");
        for ranges in &v.rows[0].inline {
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0], 1..2);
        }
    }

    #[test]
    fn comparison_tool_generated_script_and_export_apply_text_filters() {
        for (left, right, whitespace, case, endings) in [
            ("a = 1;\n", "a  =  1;\n", true, false, false),
            ("Alpha\n", "alpha\n", false, true, false),
            ("ΟΣ\n", "οσ\n", false, true, false),
            ("a\r\nb\r\n", "a\nb\n", false, false, true),
            ("  ΑLPHA\t =  1; \r\n", "αlpha = 1;\n", true, true, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut spec = files(dir.path(), left, right);
            spec.ignore_whitespace = whitespace;
            spec.ignore_case = case;
            spec.ignore_line_endings = endings;
            let (mut host, _) = start(spec.clone());
            let ws = Snapshot::default();
            let view = host.app.current.as_ref().unwrap();
            assert!(view.changes.is_empty());
            let same = view.counts.same as f64;
            let Some(toolkits::Action::SaveAppScript(AppKind::ComparisonTool, saved)) =
                host.generate(&ws, 0)
            else {
                panic!("generated script")
            };
            let mut cli = matrust::Engine::default();
            let output = cli.execute(&saved).unwrap().output.join("\n");
            assert!(
                output.is_empty(),
                "filtered script reported changes: {output}"
            );
            host.app.output_name = "counts".into();
            host.app.output_edited = true;
            let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
                panic!("counts export")
            };
            assert!(
                apps::execute_export(&mut cli, &export)
                    .unwrap()
                    .output
                    .is_empty()
            );
            cli.execute("actual=[counts.changed counts.inserted counts.deleted counts.same];")
                .unwrap();
            assert_eq!(cli.variables["actual"].data(), &[0., 0., 0., same]);
            spec.ignore_whitespace = false;
            spec.ignore_case = false;
            spec.ignore_line_endings = false;
            let (unfiltered, _) = start(spec);
            let output = matrust::Engine::default()
                .execute(&unfiltered.app.script(&ws).unwrap())
                .unwrap();
            assert!(!output.output.is_empty());
            if whitespace && !case && !endings {
                assert_eq!(unfiltered.app.current.as_ref().unwrap().counts.changed, 1);
            }
        }
    }

    #[test]
    fn comparison_tool_filtered_script_matches_counts_and_cleans_files() {
        for (left, right, changed, inserted, deleted, same) in [
            ("  a\t =  1; \n", "a = 1;\n", 0, 0, 0, 1),
            ("ab\n", "a b\n", 1, 0, 0, 0),
            ("ΟΣ\n", "ος\n", 1, 0, 0, 0),
            ("a\nb\n", "a\nB\nextra\n", 0, 1, 0, 2),
            ("a\n", "a", 1, 0, 0, 0),
            (" \n\n", "", 0, 0, 2, 0),
            (" \t", "", 0, 0, 1, 0),
            ("a\n \t", "a\n", 0, 0, 1, 1),
            (" ", "\t", 0, 0, 0, 1),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut spec = files(dir.path(), left, right);
            spec.ignore_whitespace = true;
            spec.ignore_case = true;
            spec.ignore_line_endings = true;
            let (mut host, _) = start(spec.clone());
            let expected = Counts {
                changed,
                inserted,
                deleted,
                same,
            };
            assert_eq!(host.app.current.as_ref().unwrap().counts, expected);
            // The app continues to show original text, including whitespace.
            let Comparison::Text(text) = &host.app.current.as_ref().unwrap().comparison else {
                panic!()
            };
            assert_eq!(text.left_lines.concat(), left);
            let captured = Arc::new(std::sync::Mutex::new(None));
            let result = captured.clone();
            let _presentation =
                matrust::engine::compare_builtin::present_comparisons(move |a, b, comparison| {
                    *result.lock().unwrap() =
                        Some((a.to_owned(), b.to_owned(), comparison.clone()));
                    Ok(())
                });
            host.app.output_name = "counts".into();
            host.app.output_edited = true;
            let Some(toolkits::Action::ExportScript(export)) = host.export(&Snapshot::default())
            else {
                panic!()
            };
            let mut cli = matrust::Engine::default();
            apps::execute_export(&mut cli, &export).unwrap();
            cli.execute("actual=[counts.changed counts.inserted counts.deleted counts.same];")
                .unwrap();
            assert_eq!(
                cli.variables["actual"].data(),
                &[changed as f64, inserted as f64, deleted as f64, same as f64]
            );
            let (a, b, comparison) = captured.lock().unwrap().take().unwrap();
            let performed =
                View::build(Spec::default(), comparison, &AtomicBool::new(false)).unwrap();
            assert_eq!(
                performed.counts, expected,
                "script and exported counts diverged"
            );
            assert!(
                !a.exists() && !b.exists(),
                "temporary comparison files were not deleted"
            );
            assert_eq!(std::fs::read_to_string(spec.left).unwrap(), left);
            assert_eq!(std::fs::read_to_string(spec.right).unwrap(), right);
        }
    }

    #[test]
    fn comparison_tool_filtered_script_cleans_files_when_visdiff_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = files(dir.path(), "a\n", "b\n");
        spec.ignore_case = true;
        let captured = Arc::new(std::sync::Mutex::new(None));
        let result = captured.clone();
        let _presentation =
            matrust::engine::compare_builtin::present_comparisons(move |a, b, _| {
                *result.lock().unwrap() = Some((a.to_owned(), b.to_owned()));
                Err("comparison presentation fixture failed".into())
            });
        let error = matrust::Engine::default()
            .execute(&spec.script().unwrap())
            .unwrap_err();
        assert!(
            error.contains("comparison presentation fixture failed"),
            "{error}"
        );
        let (a, b) = captured.lock().unwrap().take().unwrap();
        assert!(
            !a.exists() && !b.exists(),
            "failed comparison left temporary files"
        );
    }

    #[test]
    fn comparison_tool_runner_script_export_names_and_last_good_error_status() {
        let dir = tempfile::tempdir().unwrap();
        let spec = files(dir.path(), "a\nb\nc\n", "a\nchanged\nc\n");
        let (mut host, ctx) = start(spec.clone());
        let ws = Snapshot::default();
        assert_eq!(host.app.current.as_ref().unwrap().counts.changed, 1);
        let script = host.app.script(&ws).unwrap();
        assert_eq!(
            script,
            format!(
                "visdiff({}, {});\n",
                path_literal(&spec.left).unwrap(),
                path_literal(&spec.right).unwrap()
            )
        );
        assert!(host.request(&ws).unwrap().code.contains(&script));
        let mut fresh = matrust::Engine::default();
        let result = fresh.execute(&script).unwrap();
        assert!(
            result.output.join("\n").contains("changed"),
            "CLI retains its text report"
        );
        let Some(toolkits::Action::SaveAppScript(AppKind::ComparisonTool, saved)) =
            host.generate(&ws, 0)
        else {
            panic!("saved script")
        };
        assert!(saved.ends_with(&script));
        fresh.execute(&saved).unwrap();
        host.app.output_name = "counts".into();
        host.app.output_edited = true;
        let Some(toolkits::Action::ExportScript(export)) = host.export(&ws) else {
            panic!("counts export")
        };
        assert!(export.contains(&script));
        apps::execute_export(&mut fresh, &export).unwrap();
        fresh
            .execute("actual=[counts.changed counts.inserted counts.deleted counts.same];")
            .unwrap();
        assert_eq!(fresh.variables["actual"].data(), &[1., 0., 0., 2.]);
        for name in [
            "1bad",
            "x; y=1",
            "x %",
            "end",
            "mrexp_old",
            "mrexp_had",
            "mrexp_complete",
            "matrust_export_error",
        ] {
            host.app.output_name = name.into();
            assert!(host.app.export_code(&ws).is_err(), "{name}");
        }
        host.app.output_name = "counts".into();
        let mut taken = Snapshot::default();
        taken.sync(&fresh.workspace());
        assert!(
            host.app
                .output(&taken)
                .unwrap_err()
                .contains("already exists")
        );
        host.app.draft.left = dir.path().join("missing.txt").to_str().unwrap().into();
        host.app.compare();
        wait(&mut host, &ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Error);
        assert!(host.status().0.contains("missing.txt"));
        assert!(
            host.export(&ws).is_none(),
            "cannot pair old counts with failed new inputs"
        );
        assert_eq!(
            host.app.current.as_ref().unwrap().counts.changed,
            1,
            "failed runs keep the last good result"
        );
        host.app.draft = spec;
        host.app.compare();
        wait(&mut host, &ctx, &ws);
        assert_eq!(host.status().1, toolkits::AppStatus::Success);
    }

    #[test]
    fn comparison_tool_paths_are_data_and_relative_paths_use_workspace_folder() {
        for bad in ["", "a\nb", "a\0b"] {
            assert!(path_literal(bad).is_err());
        }
        assert!(path_literal(&"a".repeat(4097)).is_err());
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a'; error('injection'); %.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "a\n").unwrap();
        std::fs::write(&b, "b\n").unwrap();
        let mut host = Host::new(ComparisonTool::default());
        let mut ws = Snapshot::default();
        ws.set_folder(Some(dir.path().to_path_buf()));
        host.app.draft.left = a.file_name().unwrap().to_str().unwrap().into();
        host.app.draft.right = "b.txt".into();
        host.app.compare();
        wait(&mut host, &egui::Context::default(), &ws);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
        let mut engine = matrust::Engine::default();
        engine.execute(&host.app.script(&ws).unwrap()).unwrap();
    }

    #[test]
    fn comparison_tool_mat_status_metadata_and_nested_reasons() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = matrust::Engine::default();
        engine.set_file_directory(dir.path()).unwrap();
        engine.execute("same=[1 2]; changed=[1 3]; removed=4; nested={[1 2]}; save('left.mat','same','changed','removed','nested'); clear removed; changed=[1 8]; added=5; nested={[1 9]}; save('right.mat','same','changed','added','nested');").unwrap();
        let spec = Spec {
            left: dir.path().join("left.mat").to_str().unwrap().into(),
            right: dir.path().join("right.mat").to_str().unwrap().into(),
            ..Default::default()
        };
        let (host, _) = start(spec);
        assert_eq!(
            host.status().1,
            toolkits::AppStatus::Success,
            "{}",
            host.status().0
        );
        let v = host.app.current.as_ref().unwrap();
        let Comparison::Mat(diff) = &v.comparison else {
            panic!("MAT view")
        };
        assert_eq!(
            diff.variables
                .iter()
                .map(|r| (r.name.as_str(), r.status))
                .collect::<Vec<_>>(),
            [
                ("added", DiffStatus::OnlyRight),
                ("changed", DiffStatus::Differs),
                ("nested", DiffStatus::Differs),
                ("removed", DiffStatus::OnlyLeft),
                ("same", DiffStatus::Same),
            ]
        );
        assert_eq!(
            v.counts,
            Counts {
                changed: 2,
                inserted: 1,
                deleted: 1,
                same: 1
            }
        );
        assert_eq!(v.metadata[0]["changed"], "double [1, 2]");
        let values = diff.variables[1]
            .reasons
            .iter()
            .find_map(|reason| {
                if let DifferenceReason::Values(v) = reason {
                    Some(v)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(values.first_index, 2);
        assert_eq!(values.count, 1);
        assert_eq!(values.max_abs_difference, Some(5.));
        assert!(v.report.contains("nested"));
        assert!(v.report.contains("first index 2"));
        assert!(v.report.contains("max abs difference 7"));
        let details = v.details[2].join("\n");
        assert!(details.contains("nested.{1}"), "{details}");
        assert!(details.contains("first differing index: 2"));
        assert!(details.contains("max abs diff: 7"));
    }

    #[test]
    fn comparison_tool_folder_status_drill_down_back_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        for (folder, name, contents) in [
            (&a, "same.txt", "same"),
            (&b, "same.txt", "same"),
            (&a, "changed.txt", "old"),
            (&b, "changed.txt", "new"),
            (&a, "removed.txt", "a"),
            (&b, "added.txt", "b"),
        ] {
            std::fs::write(folder.join(name), contents).unwrap();
        }
        let (mut host, ctx) = start(Spec {
            left: a.to_str().unwrap().into(),
            right: b.to_str().unwrap().into(),
            ..Default::default()
        });
        let v = host.app.current.as_ref().unwrap();
        let Comparison::Folder(diff) = &v.comparison else {
            panic!("folder view")
        };
        assert_eq!(
            diff.files
                .iter()
                .map(|f| (f.name.to_str().unwrap(), f.status))
                .collect::<Vec<_>>(),
            [
                ("added.txt", DiffStatus::OnlyRight),
                ("changed.txt", DiffStatus::Differs),
                ("removed.txt", DiffStatus::OnlyLeft),
                ("same.txt", DiffStatus::Same),
            ]
        );
        assert_eq!(
            v.counts,
            Counts {
                changed: 1,
                inserted: 1,
                deleted: 1,
                same: 1
            }
        );
        assert!(!host.app.drill_down(0));
        assert!(host.app.drill_down(1));
        wait(&mut host, &ctx, &Snapshot::default());
        assert!(matches!(
            host.app.current.as_ref().unwrap().comparison,
            Comparison::Text(_)
        ));
        assert!(
            host.app
                .current
                .as_ref()
                .unwrap()
                .spec
                .left
                .ends_with("changed.txt")
        );
        host.app.back();
        wait(&mut host, &ctx, &Snapshot::default());
        assert!(matches!(
            host.app.current.as_ref().unwrap().comparison,
            Comparison::Folder(_)
        ));
        let mut spec = files(dir.path(), "abc", "axc");
        spec.kind = ComparisonType::Binary;
        let (host, _) = start(spec);
        let Comparison::Binary(binary) = &host.app.current.as_ref().unwrap().comparison else {
            panic!("binary")
        };
        assert_eq!(binary.first_offset, Some(1));
    }

    #[test]
    fn comparison_tool_visdiff_desktop_opens_app_and_tool_tab_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let spec = files(dir.path(), "left\n", "right\n");
        let ctx = egui::Context::default();
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.submit(worker::Job::ScriptInFolder(
            spec.script().unwrap(),
            dir.path().to_path_buf(),
        ));
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.busy {
            app.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.toolkits.apps.is_open(AppKind::ComparisonTool));
        assert_eq!(
            app.toolkits.selected_tool(),
            Some(toolkits::Tool::App(AppKind::ComparisonTool))
        );
        assert_eq!(app.toolkits.apps.comparison.app.draft.left, spec.left);
        assert_eq!(
            AppKind::ComparisonTool.tab(),
            matrust::session::ToolTab::ComparisonTool
        );
        assert_eq!(
            AppKind::from_tab(matrust::session::ToolTab::ComparisonTool),
            Some(AppKind::ComparisonTool)
        );
        assert_eq!(
            serde_json::to_string(&AppKind::ComparisonTool.tab()).unwrap(),
            "\"comparison_tool\""
        );
        let layout = app.toolkits.tab_layout().unwrap();
        let mut restored = toolkits::Toolkits::default();
        restored.restore_tab_layout(Some(layout));
        assert!(restored.apps.is_open(AppKind::ComparisonTool));
    }

    #[test]
    fn comparison_tool_reopening_export_pair_preserves_text_filters() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = files(dir.path(), "a = 1;\n", "a  =  1;\n");
        spec.ignore_whitespace = true;
        let (mut host, ctx) = start(spec.clone());
        assert_eq!(host.app.current.as_ref().unwrap().counts.changed, 0);
        host.app.choose(
            spec.left.clone().into(),
            spec.right.clone().into(),
            ComparisonType::Text,
        );
        wait(&mut host, &ctx, &Snapshot::default());
        assert!(host.app.draft.ignore_whitespace);
        assert!(host.app.current.as_ref().unwrap().changes.is_empty());
    }

    #[test]
    fn comparison_tool_presentation_is_scoped_and_thread_local_cli_stays_text() {
        use std::sync::atomic::AtomicUsize;
        let dir = tempfile::tempdir().unwrap();
        let spec = files(dir.path(), "a\n", "b\n");
        let code = spec.script().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let capture = calls.clone();
        let mut engine = matrust::Engine::default();
        {
            let _guard = matrust::engine::compare_builtin::present_comparisons(move |_, _, _| {
                capture.fetch_add(1, Ordering::Relaxed);
                Ok(())
            });
            assert!(engine.execute(&code).unwrap().output.is_empty());
            let child_code = code.clone();
            let output = std::thread::spawn(move || {
                matrust::Engine::default()
                    .execute(&child_code)
                    .unwrap()
                    .output
            })
            .join()
            .unwrap();
            assert!(!output.is_empty());
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
        assert!(!engine.execute(&code).unwrap().output.is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn comparison_tool_headless_virtualizes_empty_three_and_twenty_thousand_lines() {
        let ctx = egui::Context::default();
        for lines in [0, 3, 20_000] {
            let text = (0..lines).map(|i| format!("row {i}\n")).collect::<String>();
            let mut host = Host::new(ComparisonTool::default());
            host.app.current = Some(Arc::new(view(&text, &text)));
            for _ in 0..2 {
                let before = Instant::now();
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280., 720.),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        apps::comparison_tool::show(
                            &mut host,
                            ui,
                            &Snapshot::default(),
                            false,
                            &mut None,
                        );
                    },
                );
                eprintln!(
                    "comparison frame: {lines} lines, {} visible rows, {:?}",
                    host.app.laid_out_rows,
                    before.elapsed()
                );
                assert!(
                    before.elapsed() < Duration::from_secs(2),
                    "{lines} line frame took {:?}",
                    before.elapsed()
                );
                assert!(
                    host.app.laid_out_rows <= 40,
                    "laid out {} rows for {lines} lines",
                    host.app.laid_out_rows
                );
            }
        }
        let mut host = Host::new(ComparisonTool::default());
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            apps::comparison_tool::show(&mut host, ui, &Snapshot::default(), false, &mut None);
        });
    }

    #[test]
    fn comparison_tool_field_and_cell_detail_paths_use_engine_reasons() {
        let mut engine = matrust::Engine::default();
        engine
            .execute("a=struct('cell',{{[1 2]}}); b=struct('cell',{{[1 9]}});")
            .unwrap();
        let cancel = AtomicBool::new(false);
        let row = compare::array_diff(
            "s",
            &engine.array("a").unwrap(),
            &engine.array("b").unwrap(),
            &cancel,
        )
        .unwrap();
        let mut lines = Vec::new();
        apps::comparison_tool::detail_lines(&row, "", &cancel, &mut lines, &mut 0).unwrap();
        let detail = lines.join("\n");
        assert!(detail.contains("s.cell.(1).{1}: Changed"), "{detail}");
        assert!(detail.contains("first differing index: 2"));
        assert!(detail.contains("max abs diff: 7"));
    }

    #[test]
    fn comparison_tool_navigation_scrolls_to_distant_changes_in_both_views() {
        let ctx = egui::Context::default();
        let text = (0..20_000)
            .map(|i| format!("row {i}\n"))
            .collect::<String>();
        let changed = text.replace("row 19000\n", "modified row\n");
        let mut app = ComparisonTool::default();
        app.current = Some(Arc::new(view(&text, &changed)));
        for only_changes in [false, true] {
            app.show_only_changes = only_changes;
            app.selected_change = None;
            assert_eq!(app.navigate(true), Some(19_000));
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 720.),
                    )),
                    ..Default::default()
                },
                |ui| app.main_ui(ui),
            );
            let index = if only_changes { 3 } else { 19_000 };
            assert!(
                app.laid_out_range.contains(&index),
                "{only_changes}: {:?}",
                app.laid_out_range
            );
            assert!(app.laid_out_rows <= 40);
        }
    }

    #[test]
    fn comparison_tool_long_unicode_lines_clip_work_and_keep_inline_marks() {
        let ctx = egui::Context::default();
        let p = tokens::palette(&ctx);
        let text = format!("{}δtail", "α".repeat(60_000));
        let changed = 60_000..60_001;
        let job = apps::comparison_tool::line_job(
            &text,
            std::slice::from_ref(&changed),
            59_999,
            egui::FontId::proportional(12.),
            p,
        );
        assert_eq!(job.text, "αδtail");
        assert!(
            job.sections.iter().any(
                |s| &job.text[s.byte_range.start.0..s.byte_range.end.0] == "δ"
                    && s.format.underline.width == 1.
            )
        );
        let clipped = apps::comparison_tool::line_job(
            &text,
            std::slice::from_ref(&changed),
            0,
            egui::FontId::proportional(12.),
            p,
        );
        assert_eq!(clipped.text.chars().count(), 512);
    }

    #[test]
    fn comparison_tool_report_atomic_save_and_failed_export_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let spec = files(dir.path(), "left\n", "right\n");
        let (mut host, ctx) = start(spec.clone());
        let report = host.app.current.as_ref().unwrap().report.clone();
        host.app.output_name = "counts".into();
        host.app.output_edited = true;
        let Some(toolkits::Action::ExportScript(export)) = host.export(&Snapshot::default()) else {
            panic!("counts export")
        };
        let path = dir.path().join("report.txt");
        let mut app = Workspace::with_context(ctx.clone(), true);
        app.toolkits.apps.comparison = host;
        app.toolkits.apps.comparison.app.report_path = Some(path.clone());
        app.queue_app_script_save(AppKind::ComparisonTool, report.clone());
        app.finish_app_script_save(&ctx);
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.busy {
            app.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            report,
            "no script header in text reports"
        );
        assert!(app.toolkits.apps.comparison.message.contains("Saved"));
        app.toolkits.apps.comparison.app.report_path = Some(dir.path().to_path_buf());
        app.queue_app_script_save(AppKind::ComparisonTool, report);
        app.finish_app_script_save(&ctx);
        while app.busy {
            app.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            app.toolkits.apps.comparison.status().1,
            toolkits::AppStatus::Error
        );
        std::fs::remove_file(&spec.right).unwrap();
        let mut engine = matrust::Engine::default();
        engine.execute("counts=12; keep=7;").unwrap();
        assert!(apps::execute_export(&mut engine, &export).is_err());
        assert_eq!(engine.variables["counts"].data(), &[12.]);
        assert_eq!(engine.variables["keep"].data(), &[7.]);
        for reserved in [
            "mrexp_had",
            "mrexp_old",
            "mrexp_complete",
            "matrust_export_error",
        ] {
            assert!(!engine.contains_array(reserved));
        }
    }

    #[test]
    fn comparison_tool_runner_stop_retires_results_and_allows_restart() {
        let ctx = egui::Context::default();
        let mut runner = apps::Runner::default();
        let started = Arc::new(AtomicBool::new(false));
        let worker_started = started.clone();
        runner.submit_task(
            &ctx,
            1,
            Box::new(move |engine| {
                worker_started.store(true, Ordering::Relaxed);
                engine.execute("pause(60);")?;
                Ok(Default::default())
            }),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !started.load(Ordering::Relaxed) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        runner.stop();
        runner.submit_task(&ctx, 2, Box::new(|_| Ok(Default::default())));
        while runner.result().is_none() {
            runner.poll(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(runner.result().unwrap().is_ok());
        assert_eq!(runner.key(), Some(2));
    }
}
