use matrust::{Array, Engine, GraphicsKind, Matrix, files, plot::PlotKind};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

fn text(e: &Engine, name: &str) -> String {
    let Array::Character(value) = e.array(name).unwrap() else {
        panic!("Expected character row");
    };
    value.to_utf8(&AtomicBool::new(false)).unwrap()
}
fn same_handle(e: &Engine, left: &str, right: &str) {
    let Array::Graphics(a) = e.array(left).unwrap() else {
        panic!()
    };
    let Array::Graphics(b) = e.array(right).unwrap() else {
        panic!()
    };
    assert!(
        a.scalar_handle()
            .unwrap()
            .same_object(b.scalar_handle().unwrap())
    );
}

#[test]
fn patch_commands_retain_one_object_exact_faces_and_add_without_clearing() {
    let mut e = Engine::default();
    e.execute("f=figure(1);a=gca;line_h=plot(a,[0 1],[2 3]);X=[-0 2;1 3;1 3;-0 2];Y=[0 1;0 1;1 2;1 2];p=patch('Parent',a,'XData',X,'YData',Y,'FaceColor',[1 0 .5],'EdgeColor','none');alias=p;C={p};again=C{1};parent=get(p,'Parent');kind=get(p,'Type');set(p,'DisplayName','filled faces','Visible','off');").unwrap();
    same_handle(&e, "p", "alias");
    same_handle(&e, "p", "again");
    same_handle(&e, "a", "parent");
    let f = e.figure.as_ref().unwrap();
    assert_eq!(f.series.len(), 2);
    let s = &f.series[1];
    assert_eq!(s.kind, PlotKind::Patch);
    assert_eq!(s.patch_face_count(), 2);
    for (i, p) in s.points.iter().enumerate() {
        assert_eq!(p[0].to_bits(), e.variables["X"].data()[i].to_bits());
        assert_eq!(p[1].to_bits(), e.variables["Y"].data()[i].to_bits());
    }
    assert_eq!(s.color, [255, 0, 128]);
    assert!(!s.visible);
    assert_eq!(s.label, "filled faces");
    assert_eq!(text(&e, "kind"), "patch");
    let shared = s.points.clone();
    e.execute("figure(2);set(alias,'FaceColor',[0 1 0],'Visible','on');still=isgraphics(line_h);")
        .unwrap();
    assert_eq!(e.current_figure, 2);
    assert_eq!(e.logical_variables["still"].data(), &[1]);
    let s = &e.figures().find(|(id, _)| *id == 1).unwrap().1.series[1];
    assert!((shared).same_storage(&s.points));
    assert_eq!(s.color, [0, 255, 0]);
    assert!(s.visible);
}

#[test]
fn textbox_aliases_target_other_figures_and_survive_hold_off_replacement() {
    let mut e = Engine::default();
    e.execute("f=figure(1);a=gca;t=annotation(f,'textbox',[0 .9 1 .1],'String','literal < & text','Interpreter','none','EdgeColor','none','HorizontalAlignment','center','FontWeight','bold');alias=t;holder={t};again=holder{1};parent=get(t,'Parent');figure(2);set(again,'String','updated','Color',[0 .5 1],'FontSize',18);value=get(alias,'String');position=get(t,'Position');").unwrap();
    same_handle(&e, "t", "alias");
    same_handle(&e, "t", "again");
    same_handle(&e, "f", "parent");
    let Array::Graphics(h) = e.array("t").unwrap() else {
        panic!()
    };
    assert_eq!(h.scalar_handle().unwrap().kind(), GraphicsKind::TextBox);
    assert_eq!(e.current_figure, 2);
    assert_eq!(text(&e, "value"), "updated");
    e.execute("plot(a,[0 1],[2 3]);live=isgraphics(t);")
        .unwrap();
    assert_eq!(
        e.current_figure, 1,
        "Existing high-level targeted plotting selects its figure"
    );
    assert_eq!(e.variables["position"].data(), &[0., 0.9, 1., 0.1]);
    assert_eq!(e.logical_variables["live"].data(), &[1]);
    let f = e.figures().find(|(id, _)| *id == 1).unwrap().1;
    assert_eq!(f.textboxes.len(), 1);
    assert_eq!(f.textboxes[0].color, [0, 128, 255]);
    assert_eq!(f.textboxes[0].font_size, 18.);
    assert!(f.textboxes[0].bold);
    assert_eq!(f.series.len(), 1);
    e.execute("close(f);replacement=figure(1);stale=isgraphics(alias);")
        .unwrap();
    assert_eq!(e.logical_variables["stale"].data(), &[0]);
    assert!(e.execute("set(alias,'String','must not attach');").is_err());
    assert!(e.figure.as_ref().unwrap().textboxes.is_empty());
}

#[test]
fn invalid_patch_and_textbox_properties_leave_graph_and_aliases_unchanged() {
    let mut e = Engine::default();
    e.execute("f=figure;a=gca;p=patch(a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');t=annotation(f,'textbox',[0 .9 1 .1],'String','kept','Interpreter','none','EdgeColor','none');").unwrap();
    let before = e.figure.clone().unwrap();
    let workspace = e.workspace();
    for command in [
        "patch(a,'XData',[0;1;2;0],'YData',[0;0;1;2],'FaceColor',[1 0 0],'EdgeColor','none');",
        "patch(a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0]);",
        "patch(a,'Parent',a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');",
        "patch(a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none','Marker','o');",
        "annotation(f,'textbox',[0 0 .5 .1],'String','no implicit TeX');",
        "annotation(f,'textbox',[0 0 .5 .1],'Interpreter','none','EdgeColor','black');",
        "set(t,'String','not published','FontSize',201);",
        "set(t,'Color',[0 1 0],'Interpreter','tex');",
        "set(p,'FaceColor',[0 1 0],'EdgeColor','interp');",
        "set(p,'XData',[0;1;0]);",
        "set(t,'Position',[0 0 2 .5]);",
    ] {
        assert!(e.execute(command).is_err(), "{command}");
        assert!(
            before.same_snapshot(e.figure.as_ref().unwrap()),
            "{command}"
        );
        for (name, value) in &workspace {
            assert!(value.same_snapshot(&e.array(name).unwrap()));
        }
    }
}

#[test]
fn patch_graph_face_limit_and_textbox_aggregate_budget_reject_atomically() {
    let mut e = Engine::default();
    let x = Matrix::from_column_vec(3, 10_000, (0..10_000).flat_map(|_| [0., 1., 0.]).collect())
        .unwrap();
    let y = Matrix::from_column_vec(3, 10_000, (0..10_000).flat_map(|_| [0., 0., 1.]).collect())
        .unwrap();
    e.merge_workspace(BTreeMap::from([
        ("X".into(), Array::from(x)),
        ("Y".into(), Array::from(y)),
    ]))
    .unwrap();
    e.execute(
        "f=figure;a=gca;p=patch(a,'XData',X,'YData',Y,'FaceColor',[1 0 0],'EdgeColor','none');",
    )
    .unwrap();
    let before = e.figure.clone().unwrap();
    assert!(
        e.execute(
            "patch(a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');"
        )
        .unwrap_err()
        .contains("face budget")
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    e.execute("clf;").unwrap();
    let payload = "x".repeat(4096);
    for _ in 0..4 {
        e.execute(&format!("annotation('textbox',[0 0 1 .1],'String','{payload}','Interpreter','none','EdgeColor','none');")).unwrap();
    }
    let before = e.figure.clone().unwrap();
    assert!(
        e.execute(
            "annotation('textbox',[0 0 1 .1],'String','x','Interpreter','none','EdgeColor','none');"
        )
        .is_err()
    );
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
}

#[test]
fn created_objects_native_roundtrip_preserves_geometry_but_not_live_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("f=figure;a=gca;p=patch(a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');t=annotation(f,'textbox',[0 .9 1 .1],'String','literal & title','Interpreter','none','EdgeColor','none');savefig(f,'scene.mfig');").unwrap();
    let before = e.figure.clone().unwrap();
    let decoded = files::load_figure(&dir.path().join("scene.mfig")).unwrap();
    assert_eq!(before.svg(), decoded.svg());
    assert_eq!(before.textboxes, decoded.textboxes);
    assert_eq!(before.series[0].points, decoded.series[0].points);
    assert!(
        !before.textboxes[0]
            .identity
            .same_object(&decoded.textboxes[0].identity)
    );
    e.execute("g=openfig('scene.mfig');old_live=isgraphics(t);close(f);old_stale=isgraphics(t);")
        .unwrap();
    assert_eq!(e.logical_variables["old_live"].data(), &[1]);
    assert_eq!(e.logical_variables["old_stale"].data(), &[0]);
    assert_eq!(
        e.figure.as_ref().unwrap().textboxes[0].text,
        "literal & title"
    );
}

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].data()[0]
}

/// `fill` and `fill-opacity` of each painted patch polygon, in document order.
fn painted_fills(svg: &str) -> Vec<(String, String)> {
    let mut fills = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find("<polygon fill=\"") {
        rest = &rest[start + "<polygon fill=\"".len()..];
        let color = rest.split('"').next().unwrap().to_owned();
        let header = rest.split("points=\"").next().unwrap();
        let opacity = header
            .split("fill-opacity=\"")
            .nth(1)
            .map(|v| v.split('"').next().unwrap().to_owned())
            .unwrap_or_else(|| "1".into());
        fills.push((color, opacity));
    }
    fills
}

fn red_polygon(svg: &str) -> String {
    let start = svg.find("<polygon fill=\"#ff0000\"").unwrap();
    let end = svg[start..].find("/>").unwrap();
    svg[start..start + end].to_owned()
}

#[test]
fn indexed_patch_face_alpha_stays_while_only_none_hides_the_fill() {
    let mut e = Engine::default();
    e.execute(
        "xlim([-1 2]); ylim([-1 2]); \
         p=patch('Faces',[1 2 3],'Vertices',[0 0 1;1 0 1;0 1 1],'FaceAlpha',.25); \
         a=p.FaceAlpha;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "a"), 0.25);
    assert_eq!(
        painted_fills(&e.figure.as_ref().unwrap().svg()),
        vec![("#000000".into(), "0.25".into())]
    );
    let edge = e.figure.as_ref().unwrap().series[0].face_edge_color;
    for (color, fill) in [
        ("'r'", Some("#ff0000")),
        ("'none'", None),
        ("'b'", Some("#0000ff")),
    ] {
        e.execute(&format!("p.FaceColor={color}; a=p.FaceAlpha;"))
            .unwrap();
        assert_eq!(scalar(&e, "a"), 0.25, "{color}");
        let figure = e.figure.as_ref().unwrap();
        let series = &figure.series[0];
        assert_eq!(series.face_alpha, Some(0.25), "{color}");
        assert_eq!(series.face_edge_color, edge, "{color}");
        assert_eq!(series.edge_alpha, None, "{color}");
        let fills = painted_fills(&figure.svg());
        match fill {
            Some(hex) => assert_eq!(fills, vec![(hex.into(), "0.25".into())], "{color}"),
            None => {
                assert!(series.face_color_none, "{color}");
                assert!(fills.is_empty(), "{color}: {fills:?}");
                assert!(figure.svg().contains("stroke=\"#000000\""));
            }
        }
    }
    e.execute("p.FaceAlpha=0; p.FaceColor='g'; a=p.FaceAlpha;")
        .unwrap();
    assert_eq!(scalar(&e, "a"), 0.);
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.series[0].face_alpha, Some(0.));
    assert_eq!(figure.series[0].color, [0, 255, 0]);
    assert!(!figure.series[0].face_color_none);
    assert!(painted_fills(&figure.svg()).is_empty());

    e.execute(
        "figure; xlim([-1 2]); ylim([-1 2]); \
         q=patch('Faces',[1 2 3],'Vertices',[0 0 1;1 0 1;0 1 1],'FaceColor','none','FaceAlpha',0); \
         z=q.FaceAlpha;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "z"), 0.);
    assert!(painted_fills(&e.figure.as_ref().unwrap().svg()).is_empty());
    e.execute("q.FaceColor='r'; z=q.FaceAlpha;").unwrap();
    assert_eq!(scalar(&e, "z"), 0.);
    assert_eq!(e.figure.as_ref().unwrap().series[0].color, [255, 0, 0]);
    assert!(painted_fills(&e.figure.as_ref().unwrap().svg()).is_empty());

    e.execute(
        "figure; xlim([-1 2]); ylim([-1 2]); \
         m=patch('Faces',[1 2 3],'Vertices',[0 0 1;1 0 1;0 1 1],'FaceColor','flat','FaceAlpha',.25); \
         mode=m.FaceColor; a=m.FaceAlpha;",
    )
    .unwrap();
    assert_eq!(text(&e, "mode"), "flat");
    assert_eq!(scalar(&e, "a"), 0.25);
    assert!(e.figure.as_ref().unwrap().series[0].face_color_none);
    assert!(painted_fills(&e.figure.as_ref().unwrap().svg()).is_empty());
    e.execute("m.FaceColor='r'; a=m.FaceAlpha;").unwrap();
    assert_eq!(scalar(&e, "a"), 0.25);
    assert_eq!(
        painted_fills(&e.figure.as_ref().unwrap().svg()),
        vec![("#ff0000".into(), "0.25".into())]
    );

    e.execute(
        "figure; xlim([-1 2]); ylim([-1 2]); \
         c=patch('Faces',[1 2 3],'Vertices',[0 0 1;1 0 1;0 1 1], \
         'FaceVertexCData',[0;1;2],'FaceColor','flat','FaceAlpha',.25,'EdgeColor','none'); \
         a=c.FaceAlpha;",
    )
    .unwrap();
    assert_eq!(scalar(&e, "a"), 0.25);
    let mapped = painted_fills(&e.figure.as_ref().unwrap().svg());
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].1, "0.25");
    e.execute("c.FaceColor='none'; a=c.FaceAlpha;").unwrap();
    assert_eq!(scalar(&e, "a"), 0.25);
    assert!(painted_fills(&e.figure.as_ref().unwrap().svg()).is_empty());
    e.execute("c.FaceColor='flat'; a=c.FaceAlpha;").unwrap();
    assert_eq!(scalar(&e, "a"), 0.25);
    assert_eq!(painted_fills(&e.figure.as_ref().unwrap().svg()), mapped);
}

fn projected_paint(figure: &matrust::plot::Figure) -> Vec<([u8; 3], Option<f64>)> {
    let display = matrust::plot::view3d::display_view(&figure.axes);
    let patches: Vec<_> = display
        .series
        .iter()
        .filter(|s| s.kind == PlotKind::Patch)
        .collect();
    assert_eq!(patches.len(), 2);
    let drawn = patches
        .iter()
        .find(|s| s.patch_face_count() == 2)
        .expect("merged host");
    let separate = patches
        .iter()
        .find(|s| s.patch_face_count() == 0)
        .expect("separately drawn mesh");
    assert!(drawn.face_styles.is_some());
    let model = figure
        .series
        .iter()
        .find(|s| s.identity.same_object(&separate.identity))
        .unwrap();
    assert_eq!(separate.face_color_none, model.face_color_none);
    assert_eq!(separate.face_alpha, Some(0.25));
    assert_eq!(model.face_alpha, Some(0.25));
    let mut faces = Vec::new();
    for face in 0..drawn.patch_face_count() {
        let color = drawn.face_fill_color(face).unwrap_or(drawn.color);
        let alpha = drawn.face_style(face).alpha;
        if alpha == Some(0.25) {
            assert_eq!(color, [255, 0, 0]);
        } else {
            assert_eq!(alpha, Some(0.));
            assert_eq!(color, [0, 0, 0]);
        }
        faces.push((color, alpha));
    }
    faces.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap()));
    faces
}

#[test]
fn indexed_hidden_fill_does_not_steal_a_neighbors_quarter_alpha() {
    let hidden = "patch('Faces',[1 2 3],'Vertices',[0 0 0;1 0 0;0 1 0],'FaceColor','none','FaceAlpha',.25,'EdgeColor','none')";
    let colored = "patch('Faces',[1 2 3],'Vertices',[2 0 1;3 0 1;2 1 1],'FaceColor','r','FaceAlpha',.25,'EdgeColor','none')";
    let mut forward = None;
    let mut forward_svg = String::new();
    for (first, second) in [(hidden, colored), (colored, hidden)] {
        let mut e = Engine::default();
        e.execute(&format!(
            "h={first}; k={second}; xlim([0 3]); ylim([0 1]); zlim([0 1]); view(3); \
             ha=h.FaceAlpha; ka=k.FaceAlpha;"
        ))
        .unwrap();
        assert_eq!(scalar(&e, "ha"), 0.25);
        assert_eq!(scalar(&e, "ka"), 0.25);
        let figure = e.figure.as_ref().unwrap();
        let paint = projected_paint(figure);
        let svg = figure.svg();
        let fills = painted_fills(&svg);
        assert_eq!(fills, vec![("#ff0000".into(), "0.25".into())]);
        assert!(!svg.contains("<polygon fill=\"#000000\""));
        let polygon = red_polygon(&svg);
        match forward {
            None => {
                forward = Some(paint);
                forward_svg = polygon;
            }
            Some(ref expected) => {
                assert_eq!(&paint, expected);
                assert_eq!(polygon, forward_svg);
            }
        }
    }
}
