use matrust::{
    Array, Engine, GraphicsArray,
    document::Document,
    session::{
        Session, SessionStore,
        figures::{FigureDocument, FigureSession, restore_figures},
        workspace,
    },
    simulation::Model,
};
use std::{path::Path, sync::atomic::AtomicBool};

fn snapshot(e: &Engine) -> Session {
    Session {
        layout_presets: Default::default(),
        export: Default::default(),
        workspace: Some(e.workspace()),
        workspace_manifest: None,
        preview: None,
        eps_renderer: None,
        figure_manifest: None,
        figures: Some(FigureSession {
            current: e.current_figure,
            figures: e
                .figures()
                .map(|(id, f)| FigureDocument {
                    window: None,
                    id,
                    figure: f.clone(),
                    path: None,
                    source_version: None,
                    dirty: true,
                    view: None,
                })
                .collect(),
        }),
        layout: Default::default(),
        version: 1,
        documents: vec![Document::new()],
        active: 0,
        folder: std::env::current_dir().unwrap(),
        recent: vec![],
        history: vec![],
        model: Model::default(),
        model_path: None,
        model_baseline: None,
    }
}
fn handles(e: &Engine, name: &str) -> GraphicsArray {
    let Array::Graphics(a) = e.array(name).unwrap() else {
        panic!("Expected Graphics {name}")
    };
    a
}
fn recover(path: &Path) -> (Engine, Vec<String>) {
    let s = Session::load(path).unwrap();
    let cancel = AtomicBool::new(false);
    let report = restore_figures(path, s.figure_manifest.as_ref().unwrap(), &cancel);
    let binding = report.workspace_bindings().clone();
    let mut warnings = report.warnings;
    let mut e = Engine::default();
    e.restore_figures(
        report.figures.current,
        report
            .figures
            .figures
            .into_iter()
            .map(|d| (d.id, d.figure))
            .collect(),
    )
    .unwrap();
    let recovered = workspace::restore_with_figures(
        path,
        s.workspace_manifest.as_ref().unwrap(),
        &binding,
        &cancel,
    )
    .unwrap();
    warnings.extend(recovered.warnings);
    e.merge_workspace(recovered.values).unwrap();
    (e, warnings)
}
fn fixture() -> Engine {
    let mut e = Engine::default();
    e.execute("f=figure(7);subplot(1,2,1);a=gca;h=plot(a,[1 2],[-0 3]);alias=h;lg=legend(a,h,{'line'},'Interpreter','none','AutoUpdate','off');subplot(1,2,2);b=gca;q=stem(b,[1 2],[4 5],'BaseValue',-1);t=annotation(f,'textbox',[.1 .9 .8 .08],'String','literal <&>','Interpreter','none','EdgeColor','none');C={h,a,f,lg,t,{h}};A=ones(10,10);AA=A;set(f,'CurrentAxes',a);").unwrap();
    e
}
#[test]
fn all_live_kinds_rebind_fresh_identities_and_aliases_including_nested_cells() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let source = fixture();
    SessionStore::new(&path).save(snapshot(&source)).unwrap();
    let saved = Session::load(&path).unwrap();
    assert_eq!(saved.workspace_manifest.as_ref().unwrap().version, 4);
    assert!(
        workspace::restore(
            &path,
            saved.workspace_manifest.as_ref().unwrap(),
            &AtomicBool::new(false)
        )
        .is_err(),
        "No context must not bind by figure number"
    );
    let (mut got, warnings) = recover(&path);
    assert!(warnings.is_empty(), "{warnings:?}");
    for name in ["f", "a", "b", "h", "q", "lg", "t"] {
        assert!(
            !handles(&source, name)
                .scalar_handle()
                .unwrap()
                .same_object(handles(&got, name).scalar_handle().unwrap())
        );
        got.execute(&format!("live=isgraphics({name});")).unwrap();
        assert_eq!(got.logical_variables["live"].data(), &[1]);
    }
    assert!(handles(&got, "h").same_storage(&handles(&got, "alias")));
    let Array::Cell(c) = got.array("C").unwrap() else {
        panic!()
    };
    assert!(
        c.get_linear(0)
            .unwrap()
            .same_snapshot(&got.array("h").unwrap())
    );
    let Array::Cell(nested) = c.get_linear(5).unwrap().as_ref() else {
        panic!()
    };
    assert!(
        nested
            .get_linear(0)
            .unwrap()
            .same_snapshot(&got.array("h").unwrap())
    );
    assert!(
        got.array("A")
            .unwrap()
            .same_snapshot(&got.array("AA").unwrap())
    );
    got.execute("set(alias,'DisplayName','rebound');set(lg,'Visible','off');set(t,'String','restored');set(q,'BaseValue',-2);current=gca;").unwrap();
    assert!(
        handles(&got, "current")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&got, "a").scalar_handle().unwrap())
    );
    assert_eq!(got.figure.as_ref().unwrap().axes.series[0].label, "rebound");
    assert_eq!(
        got.figure.as_ref().unwrap().axes.series[0]
            .points
            .get(0)
            .unwrap()[1]
            .to_bits(),
        (-0_f64).to_bits()
    );
    assert_ne!(
        source.figure.as_ref().unwrap().axes.series[0].label,
        "rebound"
    );
}
#[test]
fn deleted_handles_and_recreated_figure_numbers_remain_typed_and_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut e = Engine::default();
    e.execute("f=figure(2);a=gca;h=plot(a,[1 2],[3 4]);alias=h;replacement=plot(a,[1 2],[5 6]);dead_figure=figure(9);dead_axes=gca;dead_series=plot([1 2],[7 8]);close(dead_figure);fresh=figure(9);plot([1 2],[9 10]);C={h,dead_figure,dead_axes,dead_series};").unwrap();
    SessionStore::new(&path).save(snapshot(&e)).unwrap();
    let (mut got, warnings) = recover(&path);
    assert!(warnings.is_empty());
    for name in ["h", "alias", "dead_figure", "dead_axes", "dead_series"] {
        got.execute(&format!("live=isgraphics({name});")).unwrap();
        assert_eq!(got.logical_variables["live"].data(), &[0]);
        assert!(matches!(got.array(name).unwrap(), Array::Graphics(_)));
    }
    assert!(handles(&got, "h").same_storage(&handles(&got, "alias")));
    got.execute("live=isgraphics(replacement);fresh_live=isgraphics(fresh);")
        .unwrap();
    assert_eq!(got.logical_variables["live"].data(), &[1]);
    assert_eq!(got.logical_variables["fresh_live"].data(), &[1]);
    let before = got.figure.clone().unwrap();
    assert!(got.execute("set(dead_figure,'Name','wrong');").is_err());
    assert!(got.figure.as_ref().unwrap().same_snapshot(&before));
}
#[test]
fn missing_figure_preserves_invalid_refs_and_other_live_figures_without_number_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut e = fixture();
    e.execute("other=figure(9);other_line=plot([1 2],[7 8]);")
        .unwrap();
    SessionStore::new(&path).save(snapshot(&e)).unwrap();
    let saved = Session::load(&path).unwrap();
    let missing = saved
        .figure_manifest
        .unwrap()
        .entries
        .into_iter()
        .find(|f| f.id == 7)
        .unwrap();
    // Derive the private directory from the actual retained file, not a public format guess.
    let payload = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|p| p.path().join(&missing.file).is_file())
        .unwrap()
        .path()
        .join(&missing.file);
    std::fs::remove_file(payload).unwrap();
    let (mut got, warnings) = recover(&path);
    assert!(!warnings.is_empty());
    got.execute(
        "live=isgraphics(h);other_live=isgraphics(other_line);figure(7);after=isgraphics(h);",
    )
    .unwrap();
    assert_eq!(got.logical_variables["live"].data(), &[0]);
    assert_eq!(got.logical_variables["after"].data(), &[0]);
    assert_eq!(got.logical_variables["other_live"].data(), &[1]);
    assert!(handles(&got, "h").same_storage(&handles(&got, "alias")));
    for name in e.workspace().keys() {
        assert!(
            got.array(name).is_some(),
            "Missing recovered variable {name}"
        );
    }
}
#[test]
fn every_admitted_series_kind_restores_as_its_actual_model_object() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut e = Engine::default();
    e.execute("f=figure(3);a=gca;hold(a,'on');line_h=plot(a,[1 2],[3 4]);scatter_h=scatter(a,[1 2],[4 5]);stairs_h=stairs(a,[1 2],[5 6]);stem_h=stem(a,[1 2],[6 7],'BaseValue',-1);bar_h=bar(a,[1 2],[7 8]);patch_h=patch('Parent',a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');").unwrap();
    SessionStore::new(&path).save(snapshot(&e)).unwrap();
    let (mut got, warnings) = recover(&path);
    assert!(warnings.is_empty());
    assert_eq!(got.figure.as_ref().unwrap().axes.series.len(), 6);
    for (i, name) in [
        "line_h",
        "scatter_h",
        "stairs_h",
        "stem_h",
        "bar_h",
        "patch_h",
    ]
    .into_iter()
    .enumerate()
    {
        got.execute(&format!(
            "live=isgraphics({name});set({name},'DisplayName','recovered{i}');"
        ))
        .unwrap();
        assert_eq!(got.logical_variables["live"].data(), &[1]);
        let series = &got.figure.as_ref().unwrap().axes.series[i];
        assert_eq!(series.label, format!("recovered{i}"));
        assert_eq!(series.kind, e.figure.as_ref().unwrap().axes.series[i].kind);
    }
}
#[test]
fn v1_v2_v3_remain_selected_without_graphics_even_inside_cells() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut store = SessionStore::new(&path);
    for (script, version) in [
        ("A=[1 2];alias=A;", 1),
        ("A=complex([1 2],[3 4]);alias=A;", 2),
        ("A=[1 2];C={A,{'text'}};", 3),
    ] {
        let mut e = Engine::default();
        e.execute(script).unwrap();
        store.save(snapshot(&e)).unwrap();
        let first = std::fs::read(&path).unwrap();
        store.save(snapshot(&e)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), first);
        let manifest = Session::load(&path).unwrap().workspace_manifest.unwrap();
        assert_eq!(manifest.version, version);
        assert!(
            !serde_json::to_string(&manifest)
                .unwrap()
                .contains("\"graphics\"")
        );
        assert_eq!(
            workspace::restore(&path, &manifest, &AtomicBool::new(false))
                .unwrap()
                .len(),
            e.workspace_count()
        );
    }
}

#[test]
fn baseline_recovery_v6_binds_latent_and_explicit_tokens_after_real_figure_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline-session.json");
    let mut source = Engine::default();
    source.execute("f=figure(7);a=gca;b=bar(a,[1 2],[3 4]);hold(a,'on');b2=bar(a,[1 2],[5 6]);s=stem(a,[1 2],[7 8]);t=stem(a,[1 2],[9 10]);bl=get(b,'BaseLine');bl2=get(b2,'BaseLine');sl=get(s,'BaseLine');tl=get(t,'BaseLine');alias=sl;C={bl,{sl,sl,tl}};text=reshape('abcd',2,2);set(sl,'Color',[.1 .2 .3],'BaseValue',-2,'Visible','off');").unwrap();
    let original = source.figure.as_ref().unwrap().clone();
    SessionStore::new(&path).save(snapshot(&source)).unwrap();
    let saved = Session::load(&path).unwrap();
    assert_eq!(saved.workspace_manifest.as_ref().unwrap().version, 6);
    let (mut restored, warnings) = recover(&path);
    assert!(warnings.is_empty(), "{warnings:?}");
    for name in ["bl", "bl2", "sl", "tl"] {
        assert!(
            !handles(&source, name)
                .scalar_handle()
                .unwrap()
                .same_object(handles(&restored, name).scalar_handle().unwrap())
        );
        restored
            .execute(&format!("live=isgraphics({name});"))
            .unwrap();
        assert_eq!(restored.logical_variables["live"].data(), &[1]);
    }
    assert!(
        handles(&restored, "bl")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&restored, "bl2").scalar_handle().unwrap())
    );
    assert!(handles(&restored, "sl").same_storage(&handles(&restored, "alias")));
    restored.execute("nested=C{2};n=nested{1};set(n,'BaseValue',-3);v=get(s,'BaseValue');newb=bar(a,[3 4],[5 6]);newbl=get(newb,'BaseLine');").unwrap();
    assert_eq!(restored.variables["v"].as_scalar().unwrap(), -3.);
    assert!(
        handles(&restored, "bl")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&restored, "newbl").scalar_handle().unwrap())
    );
    let Array::Character(text) = restored.array("text").unwrap() else {
        panic!("character shape lost")
    };
    assert_eq!((text.rows(), text.cols()), (2, 2));
    assert!(source.figure.as_ref().unwrap().same_snapshot(&original));
    let report = restore_figures(
        &path,
        saved.figure_manifest.as_ref().unwrap(),
        &AtomicBool::new(false),
    );
    for version in [4, 5] {
        let mut downgraded = saved.workspace_manifest.clone().unwrap();
        downgraded.version = version;
        let result = workspace::restore_with_figures(
            &path,
            &downgraded,
            report.workspace_bindings(),
            &AtomicBool::new(false),
        );
        assert!(
            result.is_err(),
            "v{version} must not admit baseline handles"
        );
    }
    let mut replaced = source.clone();
    replaced
        .execute("close(f);figure(7);bar([1 2],[3 4]);")
        .unwrap();
    SessionStore::new(&path).save(snapshot(&replaced)).unwrap();
    let (mut dead, warnings) = recover(&path);
    assert!(warnings.is_empty(), "{warnings:?}");
    dead.execute("live=isgraphics(sl);other=isgraphics(bl);")
        .unwrap();
    assert_eq!(dead.logical_variables["live"].data(), &[0]);
    assert_eq!(dead.logical_variables["other"].data(), &[0]);
    assert!(handles(&dead, "sl").same_storage(&handles(&dead, "alias")));
}

#[test]
fn baseline_v6_recovery_preserves_distinct_latent_bar_groups_and_record_removal() {
    use matrust::plot::{Baseline, BaselineStyle, BaselineValue};
    let cancel = AtomicBool::new(false);
    for materialized in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partitioned.json");
        let mut source = Engine::default();
        source
            .execute("f=figure(7);a=gca;b=bar(a,[1 2],[3 4]);hold(a,'on');s=stem(a,[1 2],[5 6]);")
            .unwrap();
        let axes = &mut source.figure.as_mut().unwrap().axes;
        let mixed = Baseline::new(
            BaselineValue::Automatic,
            BaselineStyle::legacy_stem(),
            axes.series.iter().map(|s| s.identity.clone()).collect(),
        )
        .unwrap();
        axes.replace_baselines(vec![mixed], &cancel).unwrap();
        source.execute("other=bar(a,[2 3],[6 7]);").unwrap();
        source
            .figure
            .as_mut()
            .unwrap()
            .axes
            .replace_baselines(Vec::new(), &cancel)
            .unwrap();
        source.execute("first=get(b,'BaseLine');second=get(other,'BaseLine');alias=first;C={first,{second,second}};").unwrap();
        assert!(
            !handles(&source, "first")
                .scalar_handle()
                .unwrap()
                .same_object(handles(&source, "second").scalar_handle().unwrap())
        );
        if materialized {
            source
                .execute("set(first,'Visible','on','LineWidth',1.25);")
                .unwrap();
        }
        let before = source.figure.as_ref().unwrap().clone();
        SessionStore::new(&path).save(snapshot(&source)).unwrap();
        assert_eq!(
            Session::load(&path)
                .unwrap()
                .workspace_manifest
                .unwrap()
                .version,
            6
        );
        let (mut restored, warnings) = recover(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            !handles(&restored, "first")
                .scalar_handle()
                .unwrap()
                .same_object(handles(&restored, "second").scalar_handle().unwrap())
        );
        assert!(handles(&restored, "first").same_storage(&handles(&restored, "alias")));
        if materialized {
            restored
                .figure
                .as_mut()
                .unwrap()
                .axes
                .replace_baselines(Vec::new(), &cancel)
                .unwrap();
        }
        restored.execute("live1=isgraphics(first);live2=isgraphics(second);nested=C{2};again=nested{1};set(first,'Visible','on','LineWidth',1.25);set(again,'Visible','off','LineWidth',1.25);v1=get(first,'Visible');v2=get(second,'Visible');").unwrap();
        assert_eq!(restored.logical_variables["live1"].data(), &[1]);
        assert_eq!(restored.logical_variables["live2"].data(), &[1]);
        assert_eq!(restored.figure.as_ref().unwrap().axes.baselines.len(), 2);
        let axes = &restored.figure.as_ref().unwrap().axes;
        assert!(
            axes.baseline_for_series(&axes.series[0].identity)
                .unwrap()
                .style
                .visible
        );
        assert!(
            !axes
                .baseline_for_series(&axes.series[2].identity)
                .unwrap()
                .style
                .visible
        );
        assert!(source.figure.as_ref().unwrap().same_snapshot(&before));
        SessionStore::new(&path).save(snapshot(&restored)).unwrap();
        let (twice, warnings) = recover(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            !handles(&twice, "first")
                .scalar_handle()
                .unwrap()
                .same_object(handles(&twice, "second").scalar_handle().unwrap())
        );
    }
}

#[test]
fn baseline_recovery_after_public_membership_split_preserves_old_and_fresh_handle_roles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("membership-split.json");
    let mut source = Engine::default();
    source.execute("a=gca;b=bar(a,[1 2],[3 4]);hold(a,'on');c=bar(a,[1 2],[5 6]);old=get(b,'BaseLine');old_alias=get(c,'BaseLine');set(old,'Visible','on','LineWidth',1.25);").unwrap();
    let axes = &mut source.figure.as_mut().unwrap().axes;
    let mut record = axes.baselines[0].clone();
    record.members.truncate(1);
    axes.replace_baselines(vec![record], &AtomicBool::new(false))
        .unwrap();
    source
        .execute("fresh=get(c,'BaseLine');C={old,{fresh,fresh}};")
        .unwrap();
    assert!(
        handles(&source, "old")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&source, "old_alias").scalar_handle().unwrap())
    );
    assert!(
        !handles(&source, "old")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&source, "fresh").scalar_handle().unwrap())
    );
    SessionStore::new(&path).save(snapshot(&source)).unwrap();
    let (mut restored, warnings) = recover(&path);
    assert!(warnings.is_empty(), "{warnings:?}");
    restored.execute("old_live=isgraphics(old);new_live=isgraphics(fresh);set(fresh,'Visible','off','LineWidth',1.25);").unwrap();
    assert_eq!(restored.logical_variables["old_live"].data(), &[1]);
    assert_eq!(restored.logical_variables["new_live"].data(), &[1]);
    assert!(
        handles(&restored, "old")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&restored, "old_alias").scalar_handle().unwrap())
    );
    let axes = &restored.figure.as_ref().unwrap().axes;
    assert!(
        axes.baseline_for_series(&axes.series[0].identity)
            .unwrap()
            .style
            .visible
    );
    assert!(
        !axes
            .baseline_for_series(&axes.series[1].identity)
            .unwrap()
            .style
            .visible
    );
    assert_eq!(axes.baselines.len(), 2);
}

#[test]
fn baseline_v6_handles_recover_with_compact_coordinates_and_independent_outlines() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("compact-baseline.json");
    let mut source = Engine::default();
    source.execute("x=[1;2;3];y=[2;-1;4];a=x-.25;b=x+.25;c=y*0;p=patch('XData',[a b b a].','YData',[c c y y].','FaceColor',[.2 .4 .6],'EdgeColor',[.6 .2 .1]);hold on;s=stem([1 2],[3 4]);bl=get(s,'BaseLine');set(bl,'Color',[.125 .25 .5],'BaseValue',-0);alias=bl;C={p,{bl,bl}};").unwrap();
    let original = source.figure.as_ref().unwrap().clone();
    assert!(original.series[0].points.patch().is_some());
    SessionStore::new(&path).save(snapshot(&source)).unwrap();
    assert_eq!(
        Session::load(&path)
            .unwrap()
            .workspace_manifest
            .unwrap()
            .version,
        6
    );
    let (mut restored, warnings) = recover(&path);
    assert!(warnings.is_empty(), "{warnings:?}");
    let figure = restored.figure.as_ref().unwrap();
    assert!(figure.series[0].points.patch().is_some());
    assert!(figure.same_metadata(&original));
    assert_eq!(
        figure.geometry_ledger().unwrap().cost(),
        original.geometry_ledger().unwrap().cost()
    );
    assert_eq!(figure.svg(), original.svg());
    for (a, b) in figure.series.iter().zip(&original.series) {
        for (p, q) in a.points.iter().zip(b.points.iter()) {
            assert_eq!(p.map(f64::to_bits), q.map(f64::to_bits));
        }
    }
    assert!(handles(&restored, "bl").same_storage(&handles(&restored, "alias")));
    assert!(
        !handles(&source, "bl")
            .scalar_handle()
            .unwrap()
            .same_object(handles(&restored, "bl").scalar_handle().unwrap())
    );
    let points = figure.series[0].points.clone();
    restored.execute("nested=C{2};again=nested{1};set(again,'BaseValue',-2);live=isgraphics(p);value=get(s,'BaseValue');").unwrap();
    assert_eq!(restored.logical_variables["live"].data(), &[1]);
    assert_eq!(restored.variables["value"].as_scalar().unwrap(), -2.);
    assert!(points.same_storage(&restored.figure.as_ref().unwrap().series[0].points));
    assert!(source.figure.as_ref().unwrap().same_snapshot(&original));
}

#[test]
fn baseline_default_policy_survives_real_checkpoint_recovery_and_observation() {
    for legacy in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bar-policy.json");
        let mut source = Engine::default();
        source
            .execute("b=bar([1 2],[3 4]);bl=get(b,'BaseLine');alias=bl;C={bl,bl};hold on;")
            .unwrap();
        if legacy {
            source.figure.as_mut().unwrap().series[0].bar_baseline_visible = false;
        }
        let original = source.figure.as_ref().unwrap().clone();
        SessionStore::new(&path).save(snapshot(&source)).unwrap();
        let (mut restored, warnings) = recover(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        let before = restored.figure.as_ref().unwrap().clone();
        restored
            .execute("shown=get(bl,'Visible');again=get(b,'BaseLine');")
            .unwrap();
        let Array::Character(shown) = restored.array("shown").unwrap() else {
            panic!("visibility string");
        };
        assert_eq!(
            shown.units(),
            if legacy {
                &[111u16, 102, 102][..]
            } else {
                &[111u16, 110][..]
            }
        );
        assert!(restored.figure.as_ref().unwrap().same_snapshot(&before));
        assert!(handles(&restored, "bl").same_storage(&handles(&restored, "alias")));
        assert!(
            handles(&restored, "bl")
                .scalar_handle()
                .unwrap()
                .same_object(handles(&restored, "again").scalar_handle().unwrap())
        );
        restored
            .execute("other=bar([3 4],[5 6]);newbl=get(other,'BaseLine');")
            .unwrap();
        assert_eq!(
            restored
                .figure
                .as_ref()
                .unwrap()
                .observe_series_baseline(1)
                .unwrap()
                .style
                .visible,
            !legacy
        );
        assert!(
            handles(&restored, "bl")
                .scalar_handle()
                .unwrap()
                .same_object(handles(&restored, "newbl").scalar_handle().unwrap())
        );
        restored.execute("set(bl,'Visible','off');").unwrap();
        SessionStore::new(&path).save(snapshot(&restored)).unwrap();
        let (twice, warnings) = recover(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(
            !twice
                .figure
                .as_ref()
                .unwrap()
                .observe_series_baseline(0)
                .unwrap()
                .style
                .visible
        );
        assert!(source.figure.as_ref().unwrap().same_snapshot(&original));
    }
}
