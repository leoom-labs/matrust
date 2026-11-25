use matrust::{Array, Engine, GraphicsArray, GraphicsKind, files};
use std::sync::{Arc, atomic::Ordering};

fn handles(e: &Engine, name: &str) -> GraphicsArray {
    let Array::Graphics(value) = e.array(name).unwrap() else {
        panic!("Expected handle array {name}");
    };
    value
}
fn same(e: &Engine, left: &str, right: &str) {
    assert!(
        handles(e, left)
            .scalar_handle()
            .unwrap()
            .same_object(handles(e, right).scalar_handle().unwrap())
    );
}

#[test]
fn typed_figure_axes_and_series_aliases_survive_cells_functions_and_selection() {
    let mut e = Engine::default();
    e.execute("f=figure(7);a=axes('Parent',f,'Position',[.05 .1 .4 .8],'NextPlot','add');h=plot(a,[1 2],[-0 3],'LineWidth',2);alias=h;C={h,a,f};again=identity(C{1});p=get(h,'Parent');kind=get(h,'Type');set(alias,'DisplayName','trace','Visible','off');b=axes('Parent',f,'Position',[.55 .1 .4 .8]);second=plot(b,[1 2],[4 5]);set(f,'CurrentAxes',a);function y=identity(x);y=x;end").unwrap();
    for name in ["alias", "again"] {
        same(&e, "h", name);
    }
    same(&e, "a", "p");
    assert_eq!(
        handles(&e, "f").scalar_handle().unwrap().kind(),
        GraphicsKind::Figure
    );
    assert_eq!(e.figure.as_ref().unwrap().axes.series[0].label, "trace");
    assert!(!e.figure.as_ref().unwrap().axes.series[0].visible);
    assert_eq!(
        e.figure.as_ref().unwrap().axes.series[0]
            .points
            .get(0)
            .unwrap()[1]
            .to_bits(),
        (-0_f64).to_bits()
    );
    assert_eq!(e.figure.as_ref().unwrap().all_axes().count(), 2);
    e.execute("clear alias again C;live=isgraphics(h);other=figure(8);set(h,'Visible','on');")
        .unwrap();
    assert_eq!(e.logical_variables["live"].data(), &[1]);
    assert_eq!(e.current_figure, 8, "set must not select its target figure");
    let old = e.figures().find(|(id, _)| *id == 7).unwrap().1;
    assert!(old.axes.series[0].visible);
}

#[test]
fn stale_handles_never_address_recreated_numbers_or_replacement_series() {
    let mut e = Engine::default();
    e.execute("f=figure(2);a=gca;h=plot(a,[1 2],[3 4]);alias=h;new=plot(a,[1 2],[5 6]);old=isgraphics(h);axis_live=isgraphics(a);figure_live=isgraphics(f);").unwrap();
    assert_eq!(e.logical_variables["old"].data(), &[0]);
    assert_eq!(e.logical_variables["axis_live"].data(), &[1]);
    assert_eq!(e.logical_variables["figure_live"].data(), &[1]);
    let before = e.figure.clone().unwrap();
    assert!(e.execute("set(alias,'Visible','off');").is_err());
    assert!(e.figure.as_ref().unwrap().same_snapshot(&before));
    e.execute("close(f);replacement=figure(2);valid=isgraphics(f);fresh=isgraphics(replacement);")
        .unwrap();
    assert_eq!(e.logical_variables["valid"].data(), &[0]);
    assert_eq!(e.logical_variables["fresh"].data(), &[1]);
    assert!(
        !handles(&e, "f")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&e, "replacement").scalar_handle().unwrap())
    );
}

#[test]
fn property_errors_stop_and_failed_targeted_plots_preserve_graph_and_active_selection() {
    let mut e = Engine::default();
    e.execute(
        "f=figure(1);a=gca;h=plot(a,[1 2],[3 4],'LineWidth',2);g=figure(2);plot([0 1],[8 9]);",
    )
    .unwrap();
    let before: Vec<_> = e.figures().map(|(id, f)| (id, f.clone())).collect();
    let workspace = e.workspace();
    for source in [
        "set(a,'XLim',[0 1],'YScale','invalid');",
        "set(h,'LineWidth',4,'NoSuchProperty',1);",
        "set(g,'CurrentAxes',a);",
        "plot(a,[1 2],[1 2 3]);",
        "axes('Parent',f,'Position',[0 0 0 1]);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert_eq!(e.current_figure, 2);
        for (id, f) in &before {
            assert!(
                e.figures()
                    .find(|(n, _)| n == id)
                    .unwrap()
                    .1
                    .same_snapshot(f),
                "{source}"
            );
        }
        for (name, value) in &workspace {
            assert!(value.same_snapshot(&e.array(name).unwrap()));
        }
    }
    let token = e.cancellation();
    token.store(true, Ordering::Relaxed);
    assert!(
        e.execute("set(h,'LineWidth',4);")
            .unwrap_err()
            .contains("cancel")
    );
    token.store(false, Ordering::Relaxed);
    e.execute("set(h,'LineWidth',4);width=get(h,'LineWidth');")
        .unwrap();
    assert_eq!(e.variables["width"].data(), &[4.]);
}

#[test]
fn handle_concatenation_selection_and_replacement_preserve_order_without_numeric_coercion() {
    let mut e = Engine::default();
    e.execute("f=figure;a=gca;set(a,'NextPlot','add');first=plot(a,[1 2],[3 4]);second=plot(a,[1 2],[5 6]);C={first,second};H=[C{:}];old=H;reversed=H([2 1]);H(1)=second;R=[first second;second first];").unwrap();
    let original = handles(&e, "old");
    let reverse = handles(&e, "reversed");
    let changed = handles(&e, "H");
    assert_eq!((original.rows(), original.cols()), (1, 2));
    assert!(
        original
            .get_linear(0)
            .unwrap()
            .same_object(reverse.get_linear(1).unwrap())
    );
    assert!(
        changed
            .get_linear(0)
            .unwrap()
            .same_object(original.get_linear(1).unwrap())
    );
    let r = handles(&e, "R");
    assert_eq!((r.rows(), r.cols()), (2, 2));
    for (index, source) in [0, 1, 1, 0].into_iter().enumerate() {
        assert!(
            r.get_linear(index)
                .unwrap()
                .same_object(original.get_linear(source).unwrap())
        );
    }
    let before = e.workspace();
    for source in ["H=H+1;", "H=repmat(H,1,1);", "H=sort(H);", "H={H}{1};"] {
        assert!(e.execute(source).is_err());
        for (name, value) in &before {
            assert!(value.same_snapshot(&e.array(name).unwrap()));
        }
    }
}

#[test]
fn native_figure_roundtrip_creates_fresh_objects_and_handle_workspace_save_rejects() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("f=figure;a=gca;h=plot(a,[1 2],[3 4]);savefig(f,'scene.mfig');")
        .unwrap();
    let source = e.figure.clone().unwrap();
    let decoded = files::load_figure(&dir.path().join("scene.mfig")).unwrap();
    assert_eq!(source.svg(), decoded.svg());
    assert_eq!(source.series[0].points, decoded.series[0].points);
    assert!(!source.identity.same_object(&decoded.identity));
    assert!(!source.axes.identity.same_object(&decoded.axes.identity));
    assert!(
        !source.series[0]
            .identity
            .same_object(&decoded.series[0].identity)
    );
    let path = dir.path().join("workspace.mat");
    std::fs::write(&path, b"retained").unwrap();
    assert!(e.execute("save('workspace.mat');").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"retained");
    // A copied in-memory snapshot shares identity and samples; serialization
    // deliberately creates new objects rather than restoring stale references.
    let clone = source.clone();
    assert!(source.identity.same_object(&clone.identity));
    assert!((source.series[0].points).same_storage(&clone.series[0].points));
}

#[test]
fn duplicate_model_tokens_and_excess_external_structure_reject_without_lookup_or_payload_copies() {
    let mut e = Engine::default();
    e.execute("f=figure;a=gca;h=plot(a,[1 2],[3 4]);").unwrap();
    let original = e.figure.clone().unwrap();
    let mut duplicate = original.clone();
    let repeated_series = duplicate.series[0].clone();
    duplicate.series.push(repeated_series);
    assert!(duplicate.validate().unwrap_err().contains("identity"));
    // Public Rust fields can bypass constructors. The lookup guard checks a
    // vector length before traversing externally supplied series metadata.
    let mut invalid = original.clone();
    let over_budget = matrust::plot::MAX_PLOT_SERIES + 1;
    invalid.series = vec![invalid.series[0].clone(); over_budget];
    e.figure = Some(invalid);
    assert!(
        e.execute("set(h,'Visible','off');")
            .unwrap_err()
            .contains("storage budget")
    );
    assert_eq!(e.figure.as_ref().unwrap().series.len(), over_budget);
    assert!(e.figure.as_ref().unwrap().series.iter().all(|s| s.visible));
    e.figure = Some(original.clone());
    e.execute("set(h,'Visible','off');").unwrap();
    assert!(!e.figure.as_ref().unwrap().series[0].visible);
    assert!((original.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
}

#[test]
fn maximal_many_part_and_zero_extent_handle_concatenation_preserve_shapes_and_aliases() {
    use std::{collections::BTreeMap, sync::atomic::AtomicBool};
    let stop = AtomicBool::new(false);
    let mut e = Engine::default();
    e.execute("f=figure;a=gca;h=plot(a,[1 2],[3 4]);").unwrap();
    let handle = e.array("h").unwrap();
    let shared_handle = Arc::new(handle.clone());
    let cells: Array = matrust::CellArray::from_shared(1, 65536, vec![shared_handle; 65536], &stop)
        .unwrap()
        .into();
    let zero_rows: Array = GraphicsArray::from_shared(0, 32768, Vec::new(), &stop)
        .unwrap()
        .into();
    let zero_cols: Array = GraphicsArray::from_shared(1, 0, Vec::new(), &stop)
        .unwrap()
        .into();
    e.merge_workspace(BTreeMap::from([
        ("C".into(), cells.clone()),
        ("E".into(), zero_rows.clone()),
        ("Z".into(), zero_cols),
    ]))
    .unwrap();
    e.execute("H=[C{:}];empty=[E E];small=[Z h Z h Z];")
        .unwrap();
    let full = handles(&e, "H");
    assert_eq!((full.rows(), full.cols()), (1, 65536));
    let original = handles(&e, "h");
    for i in 0..full.len() {
        assert!(
            full.get_linear(i)
                .unwrap()
                .same_object(original.scalar_handle().unwrap())
        );
    }
    let empty = handles(&e, "empty");
    assert_eq!((empty.rows(), empty.cols(), empty.len()), (0, 65536, 0));
    let small = handles(&e, "small");
    assert_eq!((small.rows(), small.cols()), (1, 2));
    assert!(cells.same_snapshot(&e.array("C").unwrap()));
    assert!(zero_rows.same_snapshot(&e.array("E").unwrap()));
    // Invalid empty-row width still fails before replacing the prior result.
    let before = e.array("empty").unwrap();
    assert!(e.execute("empty=[E;h];").is_err());
    assert!(before.same_snapshot(&e.array("empty").unwrap()));
}

#[test]
fn direct_and_nested_graphics_mat_save_rejects_before_publication_and_retains_shared_values() {
    use std::{collections::BTreeMap, sync::atomic::AtomicBool};
    let dir = tempfile::tempdir().unwrap();
    let stop = AtomicBool::new(false);
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("f=figure;a=gca;h=plot(a,[1 2],[3 4]);C={1,{h,'literal'}};")
        .unwrap();
    let handle = e.array("h").unwrap();
    let cell = e.array("C").unwrap();
    let figure = e.figure.clone().unwrap();
    let empty: Array = GraphicsArray::from_shared(0, 0, Vec::new(), &stop)
        .unwrap()
        .into();
    let path = dir.path().join("retained.mat");
    for value in [handle.clone(), cell.clone(), empty] {
        let workspace = BTreeMap::from([("payload".into(), value)]);
        for compressed in [false, true] {
            std::fs::write(&path, b"retained bytes").unwrap();
            let result = if compressed {
                files::save_array_workspace(&path, &workspace, &stop)
            } else {
                files::save_array_workspace_uncompressed(&path, &workspace, &stop)
            };
            assert!(result.unwrap_err().contains("graphics handles"));
            assert_eq!(std::fs::read(&path).unwrap(), b"retained bytes");
        }
    }
    for script in ["save('retained.mat','h');", "save('retained.mat','C');"] {
        assert!(e.execute(script).unwrap_err().contains("graphics handles"));
        assert_eq!(std::fs::read(&path).unwrap(), b"retained bytes");
    }
    assert!(handle.same_snapshot(&e.array("h").unwrap()));
    assert!(cell.same_snapshot(&e.array("C").unwrap()));
    assert!(figure.same_snapshot(e.figure.as_ref().unwrap()));
    assert!((figure.series[0].points).same_storage(&e.figure.as_ref().unwrap().series[0].points));
    let numeric = BTreeMap::from([(
        "payload".into(),
        Array::Numeric(matrust::Matrix::from_column_vec(1, 1, vec![7.]).unwrap()),
    )]);
    files::save_array_workspace(&path, &numeric, &stop).unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), b"retained bytes");
}

#[test]
fn leading_axes_bar_uses_existing_bounded_backend_and_preserves_other_axes_and_figures() {
    let mut e = Engine::default();
    e.execute("keep=[1 2];alias=keep;f=figure(1);a=axes('Parent',f,'Position',[.05 .1 .4 .8],'NextPlot','add');line=plot(a,[1 2],[5 6]);b=axes('Parent',f,'Position',[.55 .1 .4 .8]);other=plot(b,[1 2],[7 8]);g=figure(2);plot([1 2],[9 10]);").unwrap();
    let first = e.figures().find(|(id, _)| *id == 1).unwrap().1.clone();
    let second = e.figure.clone().unwrap();
    let before = e.workspace();
    e.execute("bars=bar(a,[1 2],[-0 3],'FaceColor',[1 0 0]);parent=get(bars,'Parent');live=isgraphics(bars);").unwrap();
    assert_eq!(e.current_figure, 1);
    same(&e, "parent", "a");
    assert_eq!(e.logical_variables["live"].data(), &[1]);
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.axes.series.len(), 2);
    let bar = &figure.axes.series[1];
    assert_eq!(bar.kind, matrust::plot::PlotKind::Bar);
    assert_eq!(bar.color, [255, 0, 0]);
    assert_eq!(bar.points.len(), 2);
    assert_eq!(
        bar.points
            .iter()
            .map(|p| p.map(f64::to_bits))
            .collect::<Vec<_>>(),
        vec![
            [1f64.to_bits(), (-0f64).to_bits()],
            [2f64.to_bits(), 3f64.to_bits()]
        ]
    );
    let unchanged = first
        .all_axes()
        .find(|(_, axis)| axis.identity.same_object(&first.axes.identity))
        .unwrap()
        .1;
    assert!(
        figure
            .all_axes()
            .find(|(_, axis)| axis.identity.same_object(&unchanged.identity))
            .unwrap()
            .1
            .same_snapshot(unchanged)
    );
    assert!(second.same_snapshot(e.figures().find(|(id, _)| *id == 2).unwrap().1));
    for (name, value) in before {
        assert!(value.same_snapshot(&e.array(&name).unwrap()));
    }
}

#[test]
fn leading_axes_bar_invalid_stale_and_cancelled_calls_are_atomic() {
    let mut e = Engine::default();
    e.execute(
        "f=figure(1);a=gca;plot(a,[1 2],[3 4]);figure(2);plot([1 2],[7 8]);keep=[1 2];alias=keep;",
    )
    .unwrap();
    let figures: Vec<_> = e.figures().map(|(id, f)| (id, f.clone())).collect();
    let workspace = e.workspace();
    for source in [
        "bar(a,[1 2],[1 2 3]);",
        "bar(a,[2 1],[3 4]);",
        "bar(a,complex([1 2]),[3 4]);",
        "bar(f,[1 2],[3 4]);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert_eq!(e.current_figure, 2);
        for (id, figure) in &figures {
            assert!(
                figure.same_snapshot(e.figures().find(|(n, _)| n == id).unwrap().1),
                "{source}"
            );
        }
        for (name, value) in &workspace {
            assert!(value.same_snapshot(&e.array(name).unwrap()));
        }
    }
    let token = e.cancellation();
    token.store(true, Ordering::Relaxed);
    assert!(
        e.execute("bar(a,[1 2],[3 4]);")
            .unwrap_err()
            .contains("cancel")
    );
    token.store(false, Ordering::Relaxed);
    for (id, figure) in &figures {
        assert!(figure.same_snapshot(e.figures().find(|(n, _)| n == id).unwrap().1));
    }
    assert_eq!(e.current_figure, 2);
    e.execute("close(f);figure(1);plot([1 2],[11 12]);figure(2);")
        .unwrap();
    let replacement = e.figures().find(|(id, _)| *id == 1).unwrap().1.clone();
    assert!(e.execute("bar(a,[1 2],[3 4]);").is_err());
    assert_eq!(e.current_figure, 2);
    assert!(replacement.same_snapshot(e.figures().find(|(id, _)| *id == 1).unwrap().1));
    assert!(
        e.array("keep")
            .unwrap()
            .same_snapshot(&e.array("alias").unwrap())
    );
}

#[test]
fn baseline_observation_types_linked_edits_and_inactive_targets_are_real_handles() {
    let mut e = Engine::default();
    e.execute("f=figure(71);a=gca;b=bar(a,[1 2],[3 4]);hold(a,'on');b2=bar(a,[1 2],[5 6]);s=stem(a,[1 2],[7 8]);s2=stem(a,[1 2],[9 10]);").unwrap();
    let before = e.figure.as_ref().unwrap().clone();
    let svg = before.svg();
    e.execute("bl=get(b,'BaseLine');bl2=get(b2,'BaseLine');sl=get(s,'BaseLine');sl2=get(s2,'BaseLine');again=get(s,'BaseLine');bp=get(bl,'Parent');kind=get(bl,'Type');rgb=get(sl,'Color');width=get(sl,'LineWidth');on=get(s,'ShowBaseLine');bar_on=get(b,'ShowBaseLine');C={bl,{sl,sl}};").unwrap();
    same(&e, "bl", "bl2");
    same(&e, "sl", "again");
    same(&e, "a", "bp");
    assert!(
        !handles(&e, "sl")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&e, "sl2").scalar_handle().unwrap())
    );
    assert_eq!(
        handles(&e, "bl").scalar_handle().unwrap().kind(),
        GraphicsKind::Baseline
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    assert_eq!(e.figure.as_ref().unwrap().svg(), svg);
    assert!(e.figure.as_ref().unwrap().baselines.is_empty());
    let Array::Character(bar_on) = e.array("bar_on").unwrap() else {
        panic!("ShowBaseLine string");
    };
    assert_eq!(bar_on.units(), &[111u16, 110]);
    assert_eq!(e.variables["rgb"].data(), &[119. / 255.; 3]);
    assert_eq!(e.variables["width"].as_scalar().unwrap(), 0.75);
    e.execute("figure(72);set(bl,'BaseValue',-2,'Color',[.123456789012345 .2 1],'LineWidth',1.25,'LineStyle','--','Visible','on');set(s,'ShowBaseLine','off','DisplayName','stem');v1=get(b,'BaseValue');v2=get(b2,'BaseValue');").unwrap();
    assert_eq!(e.current_figure, 72);
    assert_eq!(e.variables["v1"].as_scalar().unwrap(), -2.);
    assert_eq!(e.variables["v2"].as_scalar().unwrap(), -2.);
    let source = e.figures().find(|(id, _)| *id == 71).unwrap().1;
    assert_eq!(source.baselines.len(), 2);
    assert_eq!(source.baselines[0].members.len(), 2);
    assert!(!source.baselines[1].style.visible);
    assert!(source.series.iter().all(|s| s.visible));
    for (a, b) in before.series.iter().zip(&source.series) {
        assert!(a.points.same_storage(&b.points));
    }
    let saved = source.clone();
    for command in [
        "set(bl,'Color',[1 0 0],'LineWidth',NaN);",
        "set(s,'DisplayName','bad','ShowBaseLine','bad');",
        "set(b,'BaseLine',sl);",
        "set(bl,'Parent',a);",
    ] {
        assert!(e.execute(command).is_err(), "{command}");
        assert!(saved.same_snapshot(e.figures().find(|(id, _)| *id == 71).unwrap().1));
        assert_eq!(e.current_figure, 72);
    }
}

#[test]
fn baseline_replacement_and_reused_figures_do_not_revive_old_handles() {
    let mut e = Engine::default();
    e.execute("f=figure(41);a=gca;b=bar(a,[1 2],[3 4]);old=get(b,'BaseLine');replacement=bar(a,[1 2],[5 6]);fresh=get(replacement,'BaseLine');dead=isgraphics(old);live=isgraphics(fresh);").unwrap();
    assert_eq!(e.logical_variables["dead"].data(), &[0]);
    assert_eq!(e.logical_variables["live"].data(), &[1]);
    assert!(e.execute("set(old,'Visible','on');").is_err());
    e.execute("close(f);figure(41);b=bar([1 2],[3 4]);dead=isgraphics(fresh);")
        .unwrap();
    assert_eq!(e.logical_variables["dead"].data(), &[0]);
}

#[test]
fn individual_stem_baselines_survive_complete_inline_and_companion_files_twice() {
    use std::sync::atomic::AtomicBool;
    let stop = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    for negative in [false, true] {
        let mut e = Engine::default();
        e.execute("a=gca;s=stem(a,[1 2],[3 4],'LineWidth',1.5);hold(a,'on');t=stem(a,[1 2],[5 6],'LineWidth',1.5);b=get(s,'BaseLine');c=get(t,'BaseLine');set(c,'Color',[.125 .25 .5],'LineStyle',':','LineWidth',1.25,'Visible','off');set(b,'Color',[.25 .5 .125],'LineStyle','--','LineWidth',2,'BaseValue',-0);").unwrap();
        if negative {
            e.execute("set(b,'BaseValue',-2);set(c,'BaseValue',-3);")
                .unwrap();
            // Keep real chart identity, styles and explicit record ordering; only
            // replace this public fixture's small sample allocations before validation.
            for s in &mut e.figure.as_mut().unwrap().series {
                s.points = s
                    .points
                    .iter()
                    .map(|p| [p[0], -p[1]])
                    .collect::<Vec<_>>()
                    .into();
            }
            e.execute("set(a,'YScale','log','YLim',[-10 -1]);").unwrap();
        }
        let original = e.figure.unwrap();
        let before = original.clone();
        for bundled in [false, true] {
            let mut current = original.clone();
            for pass in 0..2 {
                let path = dir
                    .path()
                    .join(format!("baseline_{negative}_{bundled}_{pass}.m"));
                if bundled {
                    files::write_matlab_plot_bundle_with_cancel(&path, &current, &stop).unwrap();
                } else {
                    files::write_matlab_plot_script_with_cancel(&path, &current, &stop).unwrap();
                }
                let document = matrust::document::Document::open(&path).unwrap();
                let mut replay = Engine::default();
                replay.execute_document(&document.text, &path).unwrap();
                let result = replay.figure.unwrap();
                assert_eq!(result.baselines.len(), 2);
                for (a, b) in current.baselines.iter().zip(&result.baselines) {
                    assert_eq!(
                        a.style.color.map(f64::to_bits),
                        b.style.color.map(f64::to_bits)
                    );
                    assert_eq!(
                        a.style.width_points.to_bits(),
                        b.style.width_points.to_bits()
                    );
                    assert_eq!(a.style.pattern, b.style.pattern);
                    assert_eq!(a.style.visible, b.style.visible);
                    let index = |f: &matrust::plot::Figure, id: &matrust::ObjectIdentity| {
                        f.series
                            .iter()
                            .position(|s| s.identity.same_object(id))
                            .unwrap()
                    };
                    assert_eq!(
                        index(&current, &a.members[0]),
                        index(&result, &b.members[0])
                    );
                }
                for (a, b) in current.series.iter().zip(&result.series) {
                    assert_eq!(a.points.len(), b.points.len());
                    for (p, q) in a.points.iter().zip(b.points.iter()) {
                        assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
                    }
                    assert_eq!(
                        a.base_value.map(f64::to_bits),
                        b.base_value.map(f64::to_bits)
                    );
                }
                assert_eq!(current.svg(), result.svg());
                current = result;
            }
        }
        assert!(original.same_snapshot(&before));
    }
}

fn figure_number(e: &Engine, id: u32) -> matrust::plot::Figure {
    e.figures()
        .find(|(n, _)| *n == id)
        .unwrap_or_else(|| panic!("missing figure {id}"))
        .1
        .clone()
}

fn scalar_prop(e: &mut Engine, expr: &str, name: &str) -> f64 {
    e.execute(&format!("bulk_prop=get({expr},'{name}');"))
        .unwrap_or_else(|err| panic!("get({expr}, {name}): {err}"));
    e.variables["bulk_prop"].as_scalar().unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-9, "{actual} != {expected}");
}

fn saved_handles(e: &Engine, name: &str) -> Vec<Arc<matrust::GraphicsHandle>> {
    let value = handles(e, name);
    (0..value.len())
        .map(|i| value.get_linear(i).unwrap().clone())
        .collect()
}

/// Stored title, label, subtitle and legend sizes stay unset. Their displayed
/// points follow the axes font (1.1, 1.1 and 0.9). A text() object keeps its
/// own 10 pt default and does not follow the axes font.
fn assert_font_inheritance(figure: &matrust::plot::Figure) {
    assert!(figure.extras.title_style.font_size.is_none());
    assert!(
        figure
            .extras
            .label_styles
            .iter()
            .all(|style| style.font_size.is_none())
    );
    assert!(figure.extras.subtitle.font_size.is_none());
    assert!(figure.extras.legend_style.font_size.is_none());
    assert_eq!(figure.extras.texts.len(), 1);
    assert_eq!(figure.extras.texts[0].font_size, 10.);
    assert!(!figure.extras.texts[0].font_size_explicit);
}

#[test]
fn bulk_set_changes_every_matched_line_and_leaves_the_other_figure() {
    let mut e = Engine::default();
    e.execute(
        "f=figure(1);a=gca;set(a,'FontSize',11);set(a,'NextPlot','add');h1=plot(a,[1 2],[3 4],'LineWidth',1);h2=plot(a,[1 2],[5 6],'LineWidth',1.5);g=figure(2);h3=plot([0 1],[8 9],'LineWidth',0.5);",
    )
    .unwrap();
    assert_eq!(e.current_figure, 2);
    e.execute("set(h1,'LineWidth',3);").unwrap();
    assert_eq!(e.current_figure, 2);
    assert_eq!(
        figure_number(&e, 1)
            .series
            .iter()
            .map(|s| s.line_width)
            .collect::<Vec<_>>(),
        vec![Some(3.), Some(1.5)]
    );
    assert_eq!(figure_number(&e, 2).series[0].line_width, Some(0.5));
    assert_eq!(figure_number(&e, 1).extras.font_size, Some(11.));
    assert_eq!(scalar_prop(&mut e, "h1", "LineWidth"), 3.);
    assert_eq!(scalar_prop(&mut e, "h2", "LineWidth"), 1.5);
    assert_eq!(scalar_prop(&mut e, "h3", "LineWidth"), 0.5);
    assert_eq!(scalar_prop(&mut e, "a", "FontSize"), 11.);

    let fig1 = figure_number(&e, 1);
    let ids = (
        fig1.identity.clone(),
        fig1.axes.identity.clone(),
        fig1.series[0].identity.clone(),
        fig1.series[1].identity.clone(),
        figure_number(&e, 2).series[0].identity.clone(),
    );
    let h1 = handles(&e, "h1").scalar_handle().unwrap().clone();
    let h2 = handles(&e, "h2").scalar_handle().unwrap().clone();
    e.execute("set([h1 h2],'LineWidth',2);").unwrap();
    assert_eq!(e.current_figure, 2);
    let fig1 = figure_number(&e, 1);
    assert_eq!(fig1.series[0].line_width, Some(2.));
    assert_eq!(fig1.series[1].line_width, Some(2.));
    assert_eq!(figure_number(&e, 2).series[0].line_width, Some(0.5));
    assert_eq!(fig1.extras.font_size, Some(11.));
    assert!(fig1.identity.same_object(&ids.0));
    assert!(fig1.axes.identity.same_object(&ids.1));
    assert!(fig1.series[0].identity.same_object(&ids.2));
    assert!(fig1.series[1].identity.same_object(&ids.3));
    assert!(figure_number(&e, 2).series[0].identity.same_object(&ids.4));
    assert!(handles(&e, "h1").scalar_handle().unwrap().same_object(&h1));
    assert!(handles(&e, "h2").scalar_handle().unwrap().same_object(&h2));
    assert_eq!(scalar_prop(&mut e, "h1", "LineWidth"), 2.);
    assert_eq!(scalar_prop(&mut e, "h2", "LineWidth"), 2.);
    assert_eq!(scalar_prop(&mut e, "h3", "LineWidth"), 0.5);
    assert_eq!(scalar_prop(&mut e, "a", "FontSize"), 11.);

    e.execute("lines=findall(f,'type','line');").unwrap();
    let found = saved_handles(&e, "lines");
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|h| h.same_object(&h1)));
    assert!(found.iter().any(|h| h.same_object(&h2)));
    let before = figure_number(&e, 1);
    let other = figure_number(&e, 2);
    e.execute("set(lines,'linewidth',4);").unwrap();
    assert_eq!(e.current_figure, 2);
    assert_same_array(&found, &handles(&e, "lines"));
    for i in 1..=2 {
        assert_eq!(scalar_prop(&mut e, &format!("lines({i})"), "LineWidth"), 4.);
    }
    let fig1 = figure_number(&e, 1);
    assert_eq!(fig1.series[0].line_width, Some(4.));
    assert_eq!(fig1.series[1].line_width, Some(4.));
    assert_eq!(fig1.extras.font_size, Some(11.));
    assert_eq!(scalar_prop(&mut e, "a", "FontSize"), 11.);
    assert_eq!(figure_number(&e, 2).series[0].line_width, Some(0.5));
    assert!(
        fig1.series[0]
            .identity
            .same_object(&before.series[0].identity)
    );
    assert!(
        fig1.series[1]
            .identity
            .same_object(&before.series[1].identity)
    );
    assert!(figure_number(&e, 2).same_snapshot(&other));
    assert_eq!(e.figures().count(), 2);
}

fn assert_same_array(before: &[Arc<matrust::GraphicsHandle>], after: &GraphicsArray) {
    assert_eq!(before.len(), after.len());
    for (i, handle) in before.iter().enumerate() {
        assert!(
            after.get_linear(i).unwrap().same_object(handle),
            "handle {i} was replaced"
        );
    }
}

#[test]
fn bulk_font_size_set_mutates_matches_without_replacing_handles() {
    let mut e = Engine::default();
    e.execute(
        "f=figure(1);a=gca;set(a,'NextPlot','add');h1=plot(a,[1 2],[3 4],'LineWidth',1);h2=plot(a,[1 2],[5 6],'LineWidth',1.5);title('Hello');xlabel('Time');tx=text(1,4,'note');lg=legend(a,'one','two');th=get(a,'Title');xh=get(a,'XLabel');",
    )
    .unwrap();
    let fig = figure_number(&e, 1);
    assert_eq!(e.figures().count(), 1);
    assert_eq!(fig.extras.font_size, None);
    assert_font_inheritance(&fig);
    assert_eq!(fig.title, "Hello");
    assert_eq!(fig.extras.texts[0].string, "note");
    assert!(fig.legend_state.is_some());
    near(scalar_prop(&mut e, "a", "FontSize"), 10.);
    near(scalar_prop(&mut e, "th", "FontSize"), 11.);
    near(scalar_prop(&mut e, "xh", "FontSize"), 11.);
    near(scalar_prop(&mut e, "lg", "FontSize"), 9.);
    near(scalar_prop(&mut e, "tx", "FontSize"), 10.);

    let ids = figure_number(&e, 1);
    let title_id = ids.extras.title_identity.clone();
    let xlabel_id = ids.extras.xlabel_identity.clone();
    let ylabel_id = ids.extras.ylabel_identity.clone();
    let zlabel_id = ids.extras.zlabel_identity.clone();
    let subtitle_id = ids.extras.subtitle_identity.clone();
    let text_id = ids.extras.texts[0].identity.clone();
    let legend_id = ids.legend_state.as_ref().unwrap().identity.clone();
    let axes_id = ids.axes.identity.clone();
    let figure_id = ids.identity.clone();
    let line_ids = [
        ids.series[0].identity.clone(),
        ids.series[1].identity.clone(),
    ];
    let anchors = [
        saved_handles(&e, "f"),
        saved_handles(&e, "a"),
        saved_handles(&e, "th"),
        saved_handles(&e, "xh"),
        saved_handles(&e, "tx"),
        saved_handles(&e, "lg"),
        saved_handles(&e, "h1"),
        saved_handles(&e, "h2"),
    ];

    let before_axes = figure_number(&e, 1);
    e.execute("axes_matches=findall(f,'type','axes');set(axes_matches,'FontSize',20);")
        .unwrap();
    assert_eq!(handles(&e, "axes_matches").len(), 1);
    assert!(
        handles(&e, "axes_matches")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&e, "a").scalar_handle().unwrap())
    );
    let fig = figure_number(&e, 1);
    assert_eq!(fig.extras.font_size, Some(20.));
    assert_font_inheritance(&fig);
    near(scalar_prop(&mut e, "a", "FontSize"), 20.);
    near(scalar_prop(&mut e, "th", "FontSize"), 22.);
    near(scalar_prop(&mut e, "xh", "FontSize"), 22.);
    near(scalar_prop(&mut e, "lg", "FontSize"), 18.);
    near(scalar_prop(&mut e, "tx", "FontSize"), 10.);
    assert_eq!(fig.series[0].line_width, Some(1.));
    assert_eq!(fig.series[1].line_width, Some(1.5));
    assert!(!fig.same_snapshot(&before_axes));
    assert_identities(
        &fig,
        &figure_id,
        &axes_id,
        &title_id,
        &xlabel_id,
        &ylabel_id,
        &zlabel_id,
        &subtitle_id,
        &text_id,
        &legend_id,
        &line_ids,
    );
    assert_parents(&mut e);

    e.execute("texts=findall(f,'type','text');").unwrap();
    let texts = saved_handles(&e, "texts");
    assert!(texts.len() >= 2);
    assert!(
        texts
            .iter()
            .any(|h| h.same_object(handles(&e, "th").scalar_handle().unwrap()))
    );
    assert!(
        texts
            .iter()
            .any(|h| h.same_object(handles(&e, "tx").scalar_handle().unwrap()))
    );
    let before_text = figure_number(&e, 1);
    e.execute("set(texts,'FontSize',14);").unwrap();
    assert_same_array(&texts, &handles(&e, "texts"));
    for i in 1..=texts.len() {
        assert_eq!(scalar_prop(&mut e, &format!("texts({i})"), "FontSize"), 14.);
    }
    let fig = figure_number(&e, 1);
    assert_eq!(fig.extras.font_size, Some(20.));
    assert_eq!(fig.extras.title_style.font_size, Some(14.));
    assert!(
        fig.extras
            .label_styles
            .iter()
            .all(|style| style.font_size == Some(14.))
    );
    assert_eq!(fig.extras.subtitle.font_size, Some(14.));
    assert!(fig.extras.legend_style.font_size.is_none());
    assert_eq!(fig.extras.texts[0].font_size, 14.);
    assert!(fig.extras.texts[0].font_size_explicit);
    near(scalar_prop(&mut e, "lg", "FontSize"), 18.);
    near(scalar_prop(&mut e, "th", "FontSize"), 14.);
    near(scalar_prop(&mut e, "tx", "FontSize"), 14.);
    assert_eq!(fig.title, before_text.title);
    assert_eq!(fig.extras.texts[0].string, "note");
    assert_eq!(fig.series[0].line_width, Some(1.));
    assert_eq!(fig.series[1].line_width, Some(1.5));
    assert_identities(
        &fig,
        &figure_id,
        &axes_id,
        &title_id,
        &xlabel_id,
        &ylabel_id,
        &zlabel_id,
        &subtitle_id,
        &text_id,
        &legend_id,
        &line_ids,
    );
    assert_parents(&mut e);
    assert_anchor_handles(&e, &anchors);

    let before_legend = figure_number(&e, 1);
    e.execute("legends=findall(f,'type','legend');set(legends,'FontSize',13);")
        .unwrap();
    assert_eq!(handles(&e, "legends").len(), 1);
    assert!(
        handles(&e, "legends")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&e, "lg").scalar_handle().unwrap())
    );
    let fig = figure_number(&e, 1);
    assert_eq!(fig.extras.legend_style.font_size, Some(13.));
    near(scalar_prop(&mut e, "lg", "FontSize"), 13.);
    assert_eq!(fig.extras.font_size, Some(20.));
    assert_eq!(fig.extras.title_style.font_size, Some(14.));
    assert_eq!(fig.extras.texts[0].font_size, 14.);
    assert!(fig.extras.texts[0].font_size_explicit);
    assert_eq!(fig.series[0].line_width, before_legend.series[0].line_width);
    assert_eq!(e.figures().count(), 1);
    assert_identities(
        &fig,
        &figure_id,
        &axes_id,
        &title_id,
        &xlabel_id,
        &ylabel_id,
        &zlabel_id,
        &subtitle_id,
        &text_id,
        &legend_id,
        &line_ids,
    );
    assert_parents(&mut e);
    assert_anchor_handles(&e, &anchors);
    e.execute("one=isgraphics([f a th xh tx lg h1 h2]);live=isgraphics(texts);")
        .unwrap();
    assert!(e.logical_variables["one"].data().iter().all(|v| *v == 1));
    assert!(e.logical_variables["live"].data().iter().all(|v| *v == 1));
}

fn assert_anchor_handles(e: &Engine, anchors: &[Vec<Arc<matrust::GraphicsHandle>>]) {
    for (name, saved) in ["f", "a", "th", "xh", "tx", "lg", "h1", "h2"]
        .into_iter()
        .zip(anchors)
    {
        assert_same_array(saved, &handles(e, name));
    }
}

fn assert_parents(e: &mut Engine) {
    let axes = handles(e, "a").scalar_handle().unwrap().clone();
    let figure = handles(e, "f").scalar_handle().unwrap().clone();
    for expr in ["th", "xh", "tx"] {
        e.execute(&format!("bulk_parent=get({expr},'Parent');"))
            .unwrap();
        assert!(
            handles(e, "bulk_parent")
                .scalar_handle()
                .unwrap()
                .same_object(&axes),
            "{expr} was reparented"
        );
    }
    e.execute("bulk_parent=get(a,'Parent');").unwrap();
    assert!(
        handles(e, "bulk_parent")
            .scalar_handle()
            .unwrap()
            .same_object(&figure)
    );
}

#[allow(clippy::too_many_arguments)]
fn assert_identities(
    fig: &matrust::plot::Figure,
    figure_id: &matrust::ObjectIdentity,
    axes_id: &matrust::ObjectIdentity,
    title_id: &matrust::ObjectIdentity,
    xlabel_id: &matrust::ObjectIdentity,
    ylabel_id: &matrust::ObjectIdentity,
    zlabel_id: &matrust::ObjectIdentity,
    subtitle_id: &matrust::ObjectIdentity,
    text_id: &matrust::ObjectIdentity,
    legend_id: &matrust::ObjectIdentity,
    line_ids: &[matrust::ObjectIdentity; 2],
) {
    assert!(fig.identity.same_object(figure_id));
    assert!(fig.axes.identity.same_object(axes_id));
    assert!(fig.extras.title_identity.same_object(title_id));
    assert!(fig.extras.xlabel_identity.same_object(xlabel_id));
    assert!(fig.extras.ylabel_identity.same_object(ylabel_id));
    assert!(fig.extras.zlabel_identity.same_object(zlabel_id));
    assert!(fig.extras.subtitle_identity.same_object(subtitle_id));
    assert_eq!(fig.extras.texts.len(), 1);
    assert!(fig.extras.texts[0].identity.same_object(text_id));
    assert!(
        fig.legend_state
            .as_ref()
            .unwrap()
            .identity
            .same_object(legend_id)
    );
    assert!(fig.series[0].identity.same_object(&line_ids[0]));
    assert!(fig.series[1].identity.same_object(&line_ids[1]));
}

#[test]
fn empty_legend_set_is_noop_and_bad_targets_do_not_publish() {
    let mut e = Engine::default();
    e.execute(
        "f=figure(1);a=gca;set(a,'FontSize',11);set(a,'NextPlot','add');h1=plot(a,[1 2],[3 4],'LineWidth',1);h2=plot(a,[1 2],[5 6],'LineWidth',1.5);g=figure(2);h3=plot([0 1],[8 9],'LineWidth',0.5);empty=findall(g,'type','legend');",
    )
    .unwrap();
    assert_eq!(handles(&e, "empty").len(), 0);
    assert!(figure_number(&e, 2).legend_state.is_none());
    assert!(figure_number(&e, 2).extras.texts.is_empty());
    assert_eq!(figure_number(&e, 2).series.len(), 1);
    let snap: Vec<_> = e.figures().map(|(id, f)| (id, f.clone())).collect();
    let current = e.current_figure;
    e.execute("set(findall(g,'type','legend'),'FontSize',12);")
        .unwrap();
    assert_eq!(e.current_figure, current);
    assert_eq!(e.figures().count(), snap.len());
    for (id, before) in &snap {
        let after = figure_number(&e, *id);
        assert!(before.same_snapshot(&after), "figure {id} changed");
        assert!(before.identity.same_object(&after.identity));
    }
    assert!(figure_number(&e, 2).legend_state.is_none());
    assert!(figure_number(&e, 2).extras.texts.is_empty());
    assert_eq!(handles(&e, "empty").len(), 0);
    assert_eq!(e.figures().count(), 2);

    let before = figure_number(&e, 1);
    assert!(e.execute("set([h1 h2],'NoSuchProperty',1);").is_err());
    assert!(figure_number(&e, 1).same_snapshot(&before));
    assert!(
        e.execute("set([h1 h2],'LineWidth',4,'NoSuchProperty',1);")
            .is_err()
    );
    assert_eq!(figure_number(&e, 1).series[0].line_width, Some(1.));
    assert_eq!(figure_number(&e, 1).series[1].line_width, Some(1.5));
    assert_eq!(figure_number(&e, 1).extras.font_size, Some(11.));

    assert!(e.execute("set([a h1],'FontSize',12);").is_err());
    assert_eq!(figure_number(&e, 1).extras.font_size, Some(11.));
    assert_eq!(figure_number(&e, 1).series[0].line_width, Some(1.));
    assert_eq!(scalar_prop(&mut e, "a", "FontSize"), 11.);
    assert_eq!(figure_number(&e, 2).series[0].line_width, Some(0.5));

    let snap: Vec<_> = e.figures().map(|(id, f)| (id, f.clone())).collect();
    let numeric = e.execute("set([1 2],'LineWidth',2);").unwrap_err();
    assert!(numeric.contains("typed graphics handle"), "{numeric}");
    for (id, before) in &snap {
        assert!(figure_number(&e, *id).same_snapshot(before));
    }
    assert!(e.execute("set({h1,h2},'LineWidth',2);").is_err());
    assert!(e.execute("set([h1 h2],'LineWidth',{2,3});").is_err());
    assert_eq!(figure_number(&e, 1).series[0].line_width, Some(1.));
    assert_eq!(figure_number(&e, 1).series[1].line_width, Some(1.5));

    e.execute("delete(h2);").unwrap();
    assert_eq!(figure_number(&e, 1).series.len(), 1);
    assert!(e.execute("set(h2,'LineWidth',9);").is_err());
    assert!(e.execute("set([h1 h2],'LineWidth',9);").is_err());
    assert_eq!(figure_number(&e, 1).series.len(), 1);
    assert_eq!(figure_number(&e, 1).series[0].line_width, Some(1.));
    assert_eq!(figure_number(&e, 2).series[0].line_width, Some(0.5));
    assert_eq!(figure_number(&e, 1).extras.font_size, Some(11.));

    e.execute("set(h1,'LineWidth',3);").unwrap();
    assert_eq!(figure_number(&e, 1).series[0].line_width, Some(3.));
    assert_eq!(scalar_prop(&mut e, "h1", "LineWidth"), 3.);
    assert_eq!(figure_number(&e, 2).series[0].line_width, Some(0.5));
    assert_eq!(figure_number(&e, 1).extras.font_size, Some(11.));
}

#[test]
fn bulk_rejected_figure_defaults_leave_inheritance_unchanged() {
    let mut e = Engine::default();
    e.execute("f=figure(1); a=gca; set(f,'DefaultLineLineWidth',1);")
        .unwrap();
    assert_eq!(scalar_prop(&mut e, "f", "DefaultLineLineWidth"), 1.);
    assert!(e.execute("set([f a],'DefaultLineLineWidth',7);").is_err());
    assert_eq!(scalar_prop(&mut e, "f", "DefaultLineLineWidth"), 1.);
    e.execute("h=plot(1:2);").unwrap();
    assert_eq!(scalar_prop(&mut e, "h", "LineWidth"), 1.);
}

#[test]
fn bulk_rejected_figure_visibility_preserves_membership_and_current_figure() {
    let mut e = Engine::default();
    e.execute("f=figure(1); a=gca; g=figure(2); z=figure(3); set(z,'Visible','off'); figure(f); before_all=findall(groot,'type','figure'); before_ordinary=findobj(groot,'type','figure');").unwrap();
    let all = saved_handles(&e, "before_all");
    let ordinary = saved_handles(&e, "before_ordinary");
    let current = e.current_figure;
    let first = figure_number(&e, 1);
    let second = figure_number(&e, 2);
    assert!(
        e.execute("set([f a],'Visible','off','NumberTitle','off');")
            .is_err()
    );
    assert_eq!(e.current_figure, current);
    assert!(figure_number(&e, 1).same_snapshot(&first));
    assert!(figure_number(&e, 2).same_snapshot(&second));
    e.execute("still_on=strcmp(get(f,'Visible'),'on'); still_off=strcmp(get(z,'Visible'),'off'); visible=findall(groot,'type','figure'); ordinary=findobj(groot,'type','figure');").unwrap();
    assert_eq!(e.logical_variables["still_on"].data(), &[1]);
    assert_eq!(e.logical_variables["still_off"].data(), &[1]);
    assert_same_array(&all, &handles(&e, "visible"));
    assert_same_array(&ordinary, &handles(&e, "ordinary"));
}

#[test]
fn bulk_rejected_surface_edit_preserves_side_record_and_other_figure() {
    let mut e = Engine::default();
    e.execute("f=figure(1); s=surf([1 2;3 4]); g=figure(2); tx=text(1,1,'note');")
        .unwrap();
    let first = figure_number(&e, 1);
    let second = figure_number(&e, 2);
    let current = e.current_figure;
    assert_eq!(scalar_prop(&mut e, "s", "LineWidth"), 0.5);
    assert!(e.execute("set([s tx],'LineWidth',4);").is_err());
    assert_eq!(scalar_prop(&mut e, "s", "LineWidth"), 0.5);
    assert_eq!(e.current_figure, current);
    assert!(figure_number(&e, 1).same_snapshot(&first));
    assert!(figure_number(&e, 2).same_snapshot(&second));
    e.execute("set(s,'LineWidth',2);").unwrap();
    assert_eq!(scalar_prop(&mut e, "s", "LineWidth"), 2.);
    e.execute("figure(f); hold on; s2=surf([2 3;4 5]); set([s s2],'LineWidth',3);")
        .unwrap();
    assert_eq!(scalar_prop(&mut e, "s", "LineWidth"), 3.);
    assert_eq!(scalar_prop(&mut e, "s2", "LineWidth"), 3.);
}

#[test]
fn bulk_mixed_valid_targets_are_published_together() {
    let mut e = Engine::default();
    e.execute("f=figure(1); a=gca; h=plot(1:2,'LineWidth',0.5); tx=text(1,1,'note'); lg=legend('line'); set([a tx lg],'FontSize',14);").unwrap();
    for name in ["a", "tx", "lg"] {
        assert_eq!(scalar_prop(&mut e, name, "FontSize"), 14.);
    }
    assert_eq!(scalar_prop(&mut e, "h", "LineWidth"), 0.5);
}

fn current_axes_is(e: &mut Engine, name: &str) -> bool {
    e.execute(&format!("probe_current_axes_tf=isequal(gca,{name});"))
        .unwrap();
    e.logical_variables["probe_current_axes_tf"].data() == [1]
}

#[test]
fn movegui_on_current_axes_is_a_validated_no_op() {
    let mut e = Engine::default();
    e.execute("figure; plot([1 2],[3 4]); a=gca; lim=xlim(a);")
        .unwrap();
    let id = e.current_figure;
    let before = figure_number(&e, id);
    e.execute("movegui(gca); after=xlim(a);").unwrap();
    assert_eq!(e.figure_ids().len(), 1, "movegui must not create a figure");
    assert_eq!(e.current_figure, id);
    assert!(figure_number(&e, id).same_snapshot(&before));
    assert_eq!(e.figure.as_ref().unwrap().axes.series.len(), 1);
    assert_eq!(
        e.figure.as_ref().unwrap().axes.series[0]
            .points
            .get(1)
            .unwrap(),
        [2., 4.]
    );
    e.execute("same_lim=isequal(lim,after);").unwrap();
    assert_eq!(e.logical_variables["same_lim"].data(), &[1]);
}

#[test]
fn movegui_accepts_other_figure_handles_without_changing_selection() {
    let mut e = Engine::default();
    e.execute("f1=figure(1); plot([1 2],[3 4]); a1=gca; f2=figure(2); plot([5 6],[7 8]); a2=gca;")
        .unwrap();
    let first = figure_number(&e, 1);
    let second = figure_number(&e, 2);
    e.execute("movegui(a1); movegui(f1); movegui(f1,'center'); movegui(a1,'northeast'); movegui(f2,\"onscreen\"); movegui(f2,[10 -20]);")
        .unwrap();
    assert_eq!(e.current_figure, 2, "movegui must not select its target");
    assert_eq!(e.figure_ids().len(), 2);
    assert!(figure_number(&e, 1).same_snapshot(&first));
    assert!(figure_number(&e, 2).same_snapshot(&second));
    assert!(current_axes_is(&mut e, "a2"));
}

#[test]
fn movegui_rejects_stale_invalid_and_unsupported_forms() {
    let mut e = Engine::default();
    e.execute(
        "f1=figure(1); plot([1 2],[3 4]); a1=gca; f2=figure(2); plot(1:3); a2=gca; close(f1);",
    )
    .unwrap();
    let second = figure_number(&e, 2);
    for bad in [
        "movegui(f1)",
        "movegui(a1)",
        "movegui(5)",
        "movegui(2)",
        "movegui('center')",
        "movegui()",
        "movegui",
        "movegui(groot)",
        "movegui(0)",
        "movegui([f2 f2])",
        "movegui(f2,'middle')",
        "movegui(f2,[1 2 3])",
        "movegui(f2,[NaN 1])",
        "movegui(f2,{1})",
        "movegui(f2,'center',1)",
        "x=movegui(f2)",
    ] {
        let err = e.execute(&format!("{bad};")).unwrap_err();
        assert!(
            err.contains("movegui") || bad.starts_with("x="),
            "{bad}: {err}"
        );
    }
    assert_eq!(e.figure_ids(), vec![2]);
    assert_eq!(e.current_figure, 2);
    assert!(figure_number(&e, 2).same_snapshot(&second));
    assert!(current_axes_is(&mut e, "a2"));
    e.execute("hasx=exist('x','var')==1;").unwrap();
    assert_eq!(e.logical_variables["hasx"].data(), &[0]);
}
