use matrust::Engine;

#[test]
fn ordinary_handle_properties_legend_and_patch_do_not_create_subplot_layout() {
    let mut e = Engine::default();
    e.execute("f=figure;h=plot([0 1],[2 3]);a=gca;hold on;")
        .unwrap();
    let original = e.figure.as_ref().unwrap().clone();
    assert!(original.layout.is_none());
    for script in [
        "axes(a);",
        "set(a,'XGrid','off');grid_value=get(a,'XGrid');",
        "set(h,'LineWidth',2);",
        "lg=legend(a,h,{'first'},'Interpreter','none','AutoUpdate','off');",
        "set(lg,'Visible','off');",
        "legend(a,'show');",
        "set(f,'CurrentAxes',a);current=get(f,'CurrentAxes');",
        "title(a,'ordinary axes','Interpreter','none');",
        "p=patch('Parent',a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');",
        "set(p,'DisplayName','triangle');",
        "h2=plot(a,[0 1],[4 5]);",
    ] {
        e.execute(script).unwrap();
        let f = e.figure.as_ref().unwrap();
        assert!(
            f.layout.is_none(),
            "Handle operation manufactured layout: {script}"
        );
        assert!(f.identity.same_object(&original.identity));
        assert!(f.axes.identity.same_object(&original.axes.identity));
        assert!((f.series[0].points).same_storage(&original.series[0].points));
        f.validate().unwrap();
    }
    assert_eq!(e.figure.as_ref().unwrap().series.len(), 3);
}

#[test]
fn targeted_inactive_axes_updates_restore_active_identity_and_shared_samples() {
    let mut e = Engine::default();
    e.execute("f=figure;subplot(1,2,1);a=gca;h=plot([0 1],[2 3]);hold on;subplot(1,2,2);b=gca;plot([0 1],[4 5]);").unwrap();
    let original = e.figure.as_ref().unwrap().clone();
    for script in [
        "set(a,'XGrid','off');",
        "set(h,'LineWidth',2);",
        "lg=legend(a,h,{'left'},'Interpreter','none','AutoUpdate','off');",
        "set(lg,'Visible','off');",
        "p=patch('Parent',a,'XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none');",
        "set(p,'DisplayName','triangle');",
    ] {
        e.execute(script).unwrap();
        let f = e.figure.as_ref().unwrap();
        assert_eq!(f.all_axes().count(), 2);
        assert!(
            f.axes.identity.same_object(&original.axes.identity),
            "{script}"
        );
        assert!(f.axes.same_snapshot(&original.axes));
        for (_, old) in original.all_axes() {
            let current = f
                .all_axes()
                .find(|(_, a)| a.identity.same_object(&old.identity))
                .unwrap()
                .1;
            assert!((old.series[0].points).same_storage(&current.series[0].points));
        }
        f.validate().unwrap();
    }
}

#[test]
fn explicit_single_subplot_still_constructs_layout() {
    let mut e = Engine::default();
    e.execute("plot([0 1],[2 3]);subplot(1,1,1);").unwrap();
    assert!(e.figure.as_ref().unwrap().layout.is_some());
}
