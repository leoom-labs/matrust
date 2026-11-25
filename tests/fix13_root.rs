//! Root graphics object (`groot` / handle `0`) and default property values.
use matrust::Engine;

fn values(e: &mut Engine, expr: &str) -> Vec<f64> {
    e.execute(&format!("zz__ = double({expr});")).unwrap();
    e.variables["zz__"].data().to_vec()
}

fn text(e: &mut Engine, expr: &str) -> String {
    values(e, expr)
        .iter()
        .map(|&c| char::from_u32(c as u32).unwrap())
        .collect()
}

#[test]
fn root_properties_and_handle_equivalence() {
    let mut e = Engine::default();
    e.execute("sz = get(0, 'ScreenSize'); mp = get(groot, 'MonitorPositions');")
        .unwrap();
    assert_eq!(e.variables["sz"].dims().as_ref(), &[1, 4]);
    assert_eq!(values(&mut e, "sz"), vec![1., 1., 1920., 1080.]);
    assert_eq!(values(&mut e, "mp"), vec![1., 1., 1920., 1080.]);
    assert_eq!(text(&mut e, "get(groot, 'Type')"), "root");
    assert_eq!(text(&mut e, "get(0, 'Type')"), "root");
    assert_eq!(text(&mut e, "get(0, 'Units')"), "pixels");
    assert_eq!(values(&mut e, "get(0, 'ScreenPixelsPerInch')"), vec![96.]);
    assert_eq!(text(&mut e, "get(0, 'ShowHiddenHandles')"), "off");
    assert_eq!(values(&mut e, "isgraphics(0)"), vec![1.]);
    assert_eq!(values(&mut e, "isgraphics(groot)"), vec![1.]);
    assert_eq!(values(&mut e, "ishghandle(0)"), vec![1.]);
    assert_eq!(values(&mut e, "ishghandle(groot)"), vec![1.]);
    assert_eq!(values(&mut e, "ishandle(0)"), vec![1.]);

    e.execute("r = groot; rt = r.Type; r.Tag = 'root_tag'; r.UserData = [1 2 3]; r.ShowHiddenHandles = 'on';")
        .unwrap();
    assert_eq!(text(&mut e, "rt"), "root");
    assert_eq!(text(&mut e, "get(0, 'Tag')"), "root_tag");
    assert_eq!(values(&mut e, "get(0, 'UserData')"), vec![1., 2., 3.]);
    assert_eq!(text(&mut e, "get(0, 'ShowHiddenHandles')"), "on");
}

#[test]
fn root_children_and_current_figure() {
    let mut e = Engine::default();
    e.execute(
        "empty_cf = isempty(get(groot, 'CurrentFigure')); empty_ch = isempty(get(0, 'Children'));",
    )
    .unwrap();
    assert_eq!(values(&mut e, "empty_cf"), vec![1.]);
    assert_eq!(values(&mut e, "empty_ch"), vec![1.]);

    e.execute("f1 = figure(1); f2 = figure(2); ch = get(groot, 'Children');")
        .unwrap();
    assert_eq!(values(&mut e, "numel(ch)"), vec![2.]);
    assert_eq!(values(&mut e, "isequal(ch(1), f2)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(ch(2), f1)"), vec![1.]);

    e.execute("set(0, 'CurrentFigure', f1); cur = gcf;")
        .unwrap();
    assert_eq!(values(&mut e, "isequal(cur, f1)"), vec![1.]);
    assert_eq!(
        values(&mut e, "isequal(get(0, 'CurrentFigure'), f1)"),
        vec![1.]
    );
}

#[test]
fn fix15_root_findobj_matches_children_and_figure_parent() {
    // findobj keeps figure-number order; root Children keeps stacking order.
    let mut e = Engine::default();
    e.execute(
        "f1 = figure; f2 = figure;
         all_figures = findobj(groot, 'Type', 'figure');
         assert(numel(all_figures) == 2);
         assert(isequal(findobj(0, 'Type', 'figure'), all_figures));
         assert(isequal(findobj('Type', 'figure'), all_figures));
         assert(isequal(findobj(groot, '-depth', 1, 'Type', 'figure'), all_figures));
         assert(get(gcf, 'Parent') == groot);
         assert(f1.Parent == groot && f2.Parent == groot);
         assert(isequal(all_figures == f1, [true; false]));
         assert(isequal(get(groot, 'Children') == f2, [true; false]));
         assert(f1 ~= f2);
         close(all_figures(1));
         assert(~isvalid(f1) && isvalid(f2));",
    )
    .unwrap();
}

#[test]
fn fix15c_findobj_depth_zero_tests_hidden_dialog_when_show_hidden_handles_on() {
    let mut e = Engine::default();
    e.execute(
        "h = waitbar(0);
         set(0, 'ShowHiddenHandles', 'on');
         assert(isequal(findobj(h, '-depth', 0, 'Type', 'figure'), h));
         assert(isequal(findobj('Type', 'figure'), h));",
    )
    .unwrap();
}

#[test]
fn fix15c_findobj_dialog_has_no_number_and_follows_numbered_figures() {
    let mut e = Engine::default();
    e.execute(
        "f2 = figure(2); h = waitbar(0); f1 = figure(1);
         set(0, 'ShowHiddenHandles', 'on');
         assert(isempty(findobj('Type', 'figure', 'Number', 1000001)));
         all_figures = findobj('Type', 'figure');
         assert(isequal(all_figures(1), f1));
         assert(isequal(all_figures(2), f2));
         assert(isequal(all_figures(3), h));",
    )
    .unwrap();
}

#[test]
fn fix15_root_findobj_depth_is_relative_to_scope() {
    let mut e = Engine::default();
    e.execute(
        "f = figure(7); a = axes; h = plot(1:3, 'DisplayName', 'sample');
         assert(isempty(findobj(groot, '-depth', 0, 'Type', 'figure')));
         assert(isempty(findobj(groot, '-depth', 1, 'Type', 'axes')));
         assert(isequal(findobj(groot, '-depth', 2, 'Type', 'axes'), a));
         assert(isempty(findobj(groot, '-depth', 2, 'Type', 'line')));
         assert(isequal(findobj(groot, '-depth', 3, 'Type', 'line', 'DisplayName', 'sample'), h));
         assert(isequal(findobj(7, '-depth', 0, 'Type', 'figure'), f));
         assert(isempty(findobj(f, '-depth', 1, 'Type', 'line')));
         assert(isequal(findobj(f, '-depth', Inf, 'Type', 'line'), h));
         assert(isequal(findobj(a, '-depth', 1, 'Type', 'line'), h));",
    )
    .unwrap();
    for depth in ["-1", "1.5", "NaN", "[1 2]"] {
        assert!(
            e.execute(&format!(
                "findobj(groot, '-depth', {depth}, 'Type', 'figure')"
            ))
            .is_err()
        );
    }
}

#[test]
fn root_defaults_axes_line_figure_and_remove() {
    let mut e = Engine::default();
    e.execute("ax1 = axes; fs1 = get(ax1, 'FontSize');")
        .unwrap();
    assert_eq!(values(&mut e, "fs1"), vec![10.]);

    e.execute("set(groot, 'DefaultAxesFontSize', 14); f2 = figure; ax2 = axes('Parent', f2); fs2 = get(ax2, 'FontSize'); fs1_after = get(ax1, 'FontSize');")
        .unwrap();
    assert_eq!(values(&mut e, "fs2"), vec![14.]);
    assert_eq!(values(&mut e, "fs1_after"), vec![10.]);

    e.execute("set(0, 'defaultLineLineWidth', 2); h1 = plot(1:3, 1:3); lw1 = get(h1, 'LineWidth'); h2 = plot(1:3, 1:3, 'LineWidth', 1); lw2 = get(h2, 'LineWidth');")
        .unwrap();
    assert_eq!(values(&mut e, "lw1"), vec![2.]);
    assert_eq!(values(&mut e, "lw2"), vec![1.]);

    e.execute(
        "set(0, 'DefaultFigureVisible', 'off'); f_hid = figure; vis = get(f_hid, 'Visible');",
    )
    .unwrap();
    assert_eq!(text(&mut e, "vis"), "off");

    e.execute("d = get(groot, 'default'); has_fs = isfield(d, 'defaultAxesFontSize'); has_lw = isfield(d, 'defaultLineLineWidth'); has_vis = isfield(d, 'defaultFigureVisible'); d_fs = d.defaultAxesFontSize;")
        .unwrap();
    assert_eq!(values(&mut e, "has_fs"), vec![1.]);
    assert_eq!(values(&mut e, "has_lw"), vec![1.]);
    assert_eq!(values(&mut e, "has_vis"), vec![1.]);
    assert_eq!(values(&mut e, "d_fs"), vec![14.]);

    // close all does not clear root defaults.
    e.execute("close all; f_still = get(0, 'DefaultAxesFontSize');")
        .unwrap();
    assert_eq!(values(&mut e, "f_still"), vec![14.]);

    // 'remove' restores factory value.
    e.execute("set(0, 'DefaultAxesFontSize', 'remove'); restored_fs = get(groot, 'DefaultAxesFontSize'); factory_fs = get(groot, 'FactoryAxesFontSize'); ax3 = axes; fs3 = get(ax3, 'FontSize');")
        .unwrap();
    assert_eq!(values(&mut e, "restored_fs"), vec![10.]);
    assert_eq!(values(&mut e, "factory_fs"), vec![10.]);
    assert_eq!(values(&mut e, "fs3"), vec![10.]);

    // reset(groot) clears all remaining root defaults.
    e.execute(
        "reset(groot); d_after = get(groot, 'default'); lw_after = get(0, 'DefaultLineLineWidth');",
    )
    .unwrap();
    assert_eq!(values(&mut e, "numel(fieldnames(d_after))"), vec![0.]);
    assert_eq!(values(&mut e, "lw_after"), vec![0.5]);
}

#[test]
fn figure_level_defaults_and_other_types() {
    let mut e = Engine::default();
    e.execute("f1 = figure(1); set(f1, 'DefaultLineLineWidth', 3); h1 = plot(1:3, 1:3); lw1 = get(h1, 'LineWidth');")
        .unwrap();
    assert_eq!(values(&mut e, "lw1"), vec![3.]);

    e.execute("f2 = figure(2); h2 = plot(1:3, 1:3);").unwrap();
    assert!(e.execute("get(h2, 'LineWidth')").is_err());

    e.execute("set(0, 'DefaultTextFontSize', 16, 'DefaultPatchLineWidth', 2, 'DefaultSurfaceLineWidth', 1.5, 'DefaultImageVisible', 'off');")
        .unwrap();
    e.execute("t = text(1, 1, 'hi'); p = patch('XData', [0; 1; 0], 'YData', [0; 0; 1], 'FaceColor', [1 0 0], 'EdgeColor', [0 0 0]);")
        .unwrap();
    assert_eq!(values(&mut e, "get(t, 'FontSize')"), vec![16.]);
    assert_eq!(values(&mut e, "get(p, 'LineWidth')"), vec![2.]);
    e.execute("[X, Y] = meshgrid(1:2, 1:2); s = surf(X, Y, X+Y);")
        .unwrap();
    assert_eq!(values(&mut e, "get(s, 'LineWidth')"), vec![1.5]);
    e.execute("im = image([1 2; 3 4]);").unwrap();
    assert_eq!(text(&mut e, "get(im, 'Visible')"), "off");
}

#[test]
fn unknown_default_name_errors_clearly() {
    let mut e = Engine::default();
    let err1 = e.execute("set(0, 'DefaultBogusProp', 1);").unwrap_err();
    assert!(err1.contains("Unknown root default property"), "{err1:?}");

    let err2 = e
        .execute("set(groot, 'DefaultAxesNoSuchProp', 1);")
        .unwrap_err();
    assert!(err2.contains("Unknown property"), "{err2:?}");
}

#[test]
fn patch_rgb_edge_defaults_to_solid() {
    let mut e = Engine::default();
    e.execute("p=patch('XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor',[0 0 0]);")
        .unwrap();
    assert_eq!(text(&mut e, "get(p,'LineStyle')"), "-");
}

#[test]
fn line_spec_overrides_defaults() {
    let mut e = Engine::default();
    e.execute("set(0,'DefaultLineMarker','x'); a=plot(1:3,'-o');")
        .unwrap();
    assert_eq!(text(&mut e, "get(a,'Marker')"), "o");
    e.execute("b=plot(1:3,'r');").unwrap();
    assert_eq!(text(&mut e, "get(b,'Type')"), "line");
    assert_eq!(text(&mut e, "get(b,'LineStyle')"), "-");
}

#[test]
fn image_explicit_mapping_overrides_default() {
    let mut e = Engine::default();
    e.execute(
        "set(0,'DefaultImageCDataMapping','scaled'); im=image([1 2;3 4],'CDataMapping','direct');",
    )
    .unwrap();
    assert_eq!(text(&mut e, "get(im,'CDataMapping')"), "direct");
}

#[test]
fn text_positional_strings_override_default() {
    let mut e = Engine::default();
    e.execute("set(0,'DefaultTextString','default'); t=text([1 2],[1 2],{'a','b'});")
        .unwrap();
    assert_eq!(text(&mut e, "get(t(1),'String')"), "a");
    assert_eq!(text(&mut e, "get(t(2),'String')"), "b");
}

#[test]
fn implicit_layout_and_text_apply_defaults() {
    for command in ["subplot(2,1,1)", "nexttile", "text(1,1,'a')"] {
        let mut e = Engine::default();
        e.execute(&format!("set(0,'DefaultAxesFontSize',27,'DefaultFigureVisible','off'); {command}; ax=gca; f=gcf;")).unwrap();
        assert_eq!(values(&mut e, "get(ax,'FontSize')"), vec![27.], "{command}");
        assert_eq!(text(&mut e, "get(f,'Visible')"), "off", "{command}");
    }
}

#[test]
fn patch_explicit_visible_overrides_default() {
    let mut e = Engine::default();
    e.execute("set(0,'DefaultPatchVisible','off'); p=patch('XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor',[0 0 0],'Visible','on');").unwrap();
    assert_eq!(text(&mut e, "get(p,'Visible')"), "on");
}

#[test]
fn patch_inherits_display_name() {
    let mut e = Engine::default();
    e.execute("set(0,'DefaultPatchDisplayName','inherited'); p=patch('XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor',[0 0 0]);").unwrap();
    assert_eq!(text(&mut e, "get(p,'DisplayName')"), "inherited");
}

#[test]
fn axes_position_defaults_rejected() {
    let mut e = Engine::default();
    for prop in ["Position", "InnerPosition", "OuterPosition"] {
        let err = e
            .execute(&format!("set(0,'DefaultAxes{prop}',[0.1 0.1 0.5 0.5]);"))
            .unwrap_err();
        assert!(err.contains("Unknown"), "{err}");
    }
}

#[test]
fn children_follow_creation_order_not_current_figure() {
    let mut e = Engine::default();
    e.execute("f3=figure(3); f1=figure(1); f2=figure(2); set(0,'CurrentFigure',f3); ch=get(0,'Children');").unwrap();
    for (index, fig) in [(1, "f2"), (2, "f1"), (3, "f3")] {
        assert_eq!(
            values(&mut e, &format!("isequal(ch({index}),{fig})")),
            vec![1.]
        );
    }
}

#[test]
fn imagesc_preserves_command_mapping_and_factory_default() {
    let mut e = Engine::default();
    e.execute("h=imagesc([3 5],'CDataMapping','direct'); ax=gca;")
        .unwrap();
    assert_eq!(text(&mut e, "get(h,'CDataMapping')"), "scaled");
    assert_eq!(values(&mut e, "get(ax,'CLim')"), vec![3., 5.]);
    e.execute("set(0,'DefaultImageCDataMapping','factory'); h=imagesc([3 5]);")
        .unwrap();
    assert_eq!(text(&mut e, "get(h,'CDataMapping')"), "scaled");
    assert_eq!(values(&mut e, "get(gca,'CLim')"), vec![3., 5.]);
}

#[test]
fn existing_axes_and_plot_command_scales() {
    let mut e = Engine::default();
    e.execute("ax=gca; set(0,'DefaultAxesFontSize',27); plot(1:3);")
        .unwrap();
    assert_eq!(values(&mut e, "ax.FontSize"), vec![10.]);
    e.execute("set(0,'DefaultAxesXScale','linear'); semilogx(1:3);")
        .unwrap();
    assert_eq!(text(&mut e, "get(gca,'XScale')"), "log");
    e.execute("close all; semilogx(1:3);").unwrap();
    assert_eq!(text(&mut e, "get(gca,'XScale')"), "log");
}

#[test]
fn constructors_keep_inherited_values_until_explicitly_overridden() {
    let mut e = Engine::default();
    e.execute("set(0,'DefaultPatchLineStyle','--','DefaultLineLineWidth',2,'DefaultSurfaceLineWidth',2,'DefaultTextFontSize',17,'DefaultImageVisible','off','DefaultAxesFontSize',18,'DefaultFigureVisible','off'); f=figure; ax=axes; p=patch('XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor',[0 0 0]);").unwrap();
    assert_eq!(text(&mut e, "get(p,'LineStyle')"), "--");
    assert_eq!(values(&mut e, "get(ax,'FontSize')"), vec![18.]);
    assert_eq!(text(&mut e, "get(f,'Visible')"), "off");
    e.execute("p2=patch('XData',[0;1;0],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor',[0 0 0],'LineStyle',':');").unwrap();
    assert_eq!(text(&mut e, "get(p2,'LineStyle')"), ":");
    e.execute("l=plot(1:3,'LineWidth',3);").unwrap();
    assert_eq!(values(&mut e, "get(l,'LineWidth')"), vec![3.]);
    e.execute("t=text(1,1,'a','FontSize',19);").unwrap();
    assert_eq!(values(&mut e, "get(t,'FontSize')"), vec![19.]);
    e.execute("s=surf([1 2;3 4],'LineWidth',3);").unwrap();
    assert_eq!(values(&mut e, "get(s,'LineWidth')"), vec![3.]);
    e.execute("im=image([1 2],'Visible','on');").unwrap();
    assert_eq!(text(&mut e, "get(im,'Visible')"), "on");
    e.execute("set(0,'DefaultAxesNextPlot','add'); f2=figure('Visible','on'); ax2=axes('NextPlot','replace');")
        .unwrap();
    assert_eq!(text(&mut e, "get(f2,'Visible')"), "on");
    assert_eq!(text(&mut e, "get(ax2,'NextPlot')"), "replace");
}

#[test]
fn figure_factory_and_remove_defaults() {
    let mut e = Engine::default();
    e.execute("f=figure; set(0,'DefaultLineLineWidth',4); set(f,'DefaultLineLineWidth','factory'); v=get(f,'DefaultLineLineWidth'); factory=get(0,'FactoryLineLineWidth'); h=plot(1:3);").unwrap();
    assert_eq!(values(&mut e, "v"), vec![0.5]);
    assert_eq!(values(&mut e, "factory"), vec![0.5]);
    assert_eq!(values(&mut e, "get(h,'LineWidth')"), vec![0.5]);
    e.execute("set(f,'DefaultLineLineWidth','remove'); h=plot(1:3);")
        .unwrap();
    assert_eq!(values(&mut e, "get(f,'DefaultLineLineWidth')"), vec![4.]);
    assert_eq!(values(&mut e, "get(h,'LineWidth')"), vec![4.]);
}

#[test]
fn root_has_its_own_property_table() {
    let mut e = Engine::default();
    assert_eq!(
        values(
            &mut e,
            "[isprop(groot,'ScreenSize') isprop(groot,'LineWidth')]"
        ),
        vec![1., 0.]
    );
    e.execute("n=fieldnames(groot);").unwrap();
    assert_eq!(values(&mut e, "any(strcmp(n,'ScreenSize'))"), vec![1.]);
    assert_eq!(values(&mut e, "any(strcmp(n,'LineWidth'))"), vec![0.]);
}

#[test]
fn existing_axes_no_defaults_merge_base_regression() {
    let mut e = Engine::default();
    e.execute("ax=gca; im=imagesc([3 5],'CDataMapping','direct');")
        .unwrap();
    assert_eq!(text(&mut e, "get(im,'CDataMapping')"), "scaled");
    assert_eq!(values(&mut e, "get(ax,'CLim')"), vec![3., 5.]);
    e.execute("semilogx(1:3);").unwrap();
    assert_eq!(text(&mut e, "get(ax,'XScale')"), "log");
    e.execute("plot(1:3);").unwrap();
    assert_eq!(text(&mut e, "get(ax,'XScale')"), "linear");
    assert_eq!(values(&mut e, "get(ax,'FontSize')"), vec![10.]);
}

#[test]
fn replacing_plot_keeps_creation_defaults() {
    for create in [
        "figure",
        "axes",
        "subplot(2,1,1)",
        "nexttile",
        "gca",
        "plot(1:3)",
    ] {
        let mut e = Engine::default();
        e.execute(&format!(
            "set(0,'DefaultAxesFontSize',27,'DefaultAxesXGrid','on'); {create}; plot(1:3);"
        ))
        .unwrap();
        assert_eq!(values(&mut e, "get(gca,'FontSize')"), vec![27.], "{create}");
        assert_eq!(text(&mut e, "get(gca,'XGrid')"), "on", "{create}");
        e.execute("set(0,'DefaultAxesFontSize',30,'DefaultAxesXGrid','off'); plot(1:4);")
            .unwrap();
        assert_eq!(values(&mut e, "get(gca,'FontSize')"), vec![27.], "{create}");
        assert_eq!(text(&mut e, "get(gca,'XGrid')"), "on", "{create}");
    }

    let mut e = Engine::default();
    e.execute("f=figure; set(f,'DefaultAxesFontSize',20); plot(1:3);")
        .unwrap();
    assert_eq!(values(&mut e, "get(gca,'FontSize')"), vec![20.]);
    e.execute("set(f,'DefaultAxesFontSize',25); plot(1:4);")
        .unwrap();
    assert_eq!(values(&mut e, "get(gca,'FontSize')"), vec![20.]);

    let mut e = Engine::default();
    e.execute("f=figure; ax=gca; set(f,'DefaultAxesFontSize',20); set(0,'DefaultAxesFontSize',27); plot(1:3);")
        .unwrap();
    assert_eq!(values(&mut e, "ax.FontSize"), vec![10.]);
}

#[test]
fn replacing_image_and_surface_keep_creation_defaults() {
    for draw in ["image", "imagesc", "imshow", "surf", "mesh"] {
        let mut e = Engine::default();
        e.execute(&format!(
            "set(0,'DefaultAxesFontSize',27); figure; {draw}([1 2;3 4]);"
        ))
        .unwrap();
        assert_eq!(values(&mut e, "get(gca,'FontSize')"), vec![27.], "{draw}");
        e.execute(&format!(
            "set(0,'DefaultAxesFontSize',30); {draw}([1 2;3 4]);"
        ))
        .unwrap();
        assert_eq!(values(&mut e, "get(gca,'FontSize')"), vec![27.], "{draw}");
    }
}

#[test]
fn imshow_uses_its_own_mapping_over_image_defaults() {
    let mut e = Engine::default();
    e.execute("set(0,'DefaultImageCDataMapping','factory'); h=imshow(uint8([0 128 255]));")
        .unwrap();
    assert_eq!(text(&mut e, "get(h,'CDataMapping')"), "scaled");
    assert_eq!(values(&mut e, "get(gca,'CLim')"), vec![0., 255.]);
    e.execute("set(0,'DefaultImageCDataMapping','scaled'); h=imshow([1 2 3],gray(3));")
        .unwrap();
    assert_eq!(text(&mut e, "get(h,'CDataMapping')"), "direct");
}

#[test]
fn three_dimensional_plot_view_overrides_axes_default() {
    for draw in [
        "surf([1 2;3 4])",
        "mesh([1 2;3 4])",
        "plot3(1:3,1:3,1:3)",
        "scatter3(1:3,1:3,1:3)",
    ] {
        let mut e = Engine::default();
        e.execute(&format!("set(0,'DefaultAxesView',[0 90]); {draw};"))
            .unwrap();
        assert_eq!(
            values(&mut e, "get(gca,'View')"),
            vec![-37.5, 30.],
            "{draw}"
        );
    }
}

#[test]
fn root_dot_defaults_share_get_and_set_validation() {
    let mut e = Engine::default();
    e.execute("g=groot;").unwrap();
    let set_error = e
        .execute("set(groot,'DefaultLineLineWidth',-1);")
        .unwrap_err();
    let dot_error = e.execute("g.DefaultLineLineWidth=-1;").unwrap_err();
    assert_eq!(
        dot_error.split_once(": ").unwrap().1,
        set_error.split_once(": ").unwrap().1
    );
    assert!(
        dot_error.contains("Line width must be 0.1..50 points"),
        "{dot_error}"
    );
    e.execute("g.DefaultLineLineWidth=2;").unwrap();
    assert_eq!(values(&mut e, "g.DefaultLineLineWidth"), vec![2.]);
    assert_eq!(
        values(&mut e, "get(groot,'DefaultLineLineWidth')"),
        vec![2.]
    );
    assert_eq!(values(&mut e, "g.FactoryLineLineWidth"), vec![0.5]);
    let factory_error = e.execute("g.FactoryLineLineWidth=2;").unwrap_err();
    assert!(
        factory_error.contains("Factory property 'FactoryLineLineWidth' is read-only"),
        "{factory_error}"
    );
}

#[test]
fn fix13_findall_returns_lines_from_the_selected_figure() {
    let mut e = Engine::default();
    e.execute(
        "f1 = figure(1);
         h1 = plot(1:2);
         hold on;
         h2 = plot(2:3);
         f2 = figure(2);
         h3 = plot(3:4);
         found = findall(f1, 'type', 'line');
         current = findall(gcf, 'type', 'line');
         typed = findall(f1, 'TYPE', 'LiNe');
         by_number = findall(1, 'type', 'line');
         from_root = findall(0, 'type', 'line');
         from_groot = findall(groot, 'type', 'line');
         from_line = findall(h1, 'type', 'line');
         from_axes = findall(get(h1, 'Parent'), 'type', 'line');
         not_axes = findall(h1, 'type', 'axes');
         same = findobj(f1, 'Type', 'line');
         depth = findobj(f1, '-depth', Inf, 'Type', 'line');",
    )
    .unwrap();
    assert_eq!(values(&mut e, "numel(found)"), vec![2.]);
    assert_eq!(values(&mut e, "isequal(found, [h1; h2])"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(found(1), h1)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(found(2), h2)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(found(1), h3)"), vec![0.]);
    assert_eq!(values(&mut e, "isequal(found(2), h3)"), vec![0.]);
    assert_eq!(values(&mut e, "isequal(current, h3)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(typed, found)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(by_number, found)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(from_root, [h1; h2; h3])"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(from_groot, from_root)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(from_line, h1)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(from_axes, found)"), vec![1.]);
    assert_eq!(values(&mut e, "isempty(not_axes)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(same, found)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(depth, found)"), vec![1.]);
    for (script, phrase) in [
        ("findall()", "property/value pairs"),
        ("findall(f1)", "property/value pairs"),
        ("findall(f1, 'type')", "property/value pairs"),
        (
            "findall(f1, 'type', 'surface')",
            "figure, axes, line, text or legend",
        ),
        ("findall(f1, 'NotAProp', 1)", "FontSize"),
        (
            "findall(f1, 'type', 'line', 'Type', 'line')",
            "must be unique",
        ),
    ] {
        let err = e.execute(script).unwrap_err();
        assert!(err.contains(phrase), "{script}: {err}");
    }
}

#[test]
fn fix13_findall_colorbar_title_reproduction() {
    let mut e = Engine::default();
    e.execute("f=figure; ax=axes; plot(1:3); at=title(ax,'Axes title'); cb=colorbar; cb.Label.String='Side label'; cb.Title.String='Bar title'; bl=get(cb,'Label'); bt=get(cb,'Title');
        assert(isequal(bl,cb.Label)); assert(isequal(bt,cb.Title));
        assert(isequal(get(bl,'Parent'),cb)); assert(isequal(get(bt,'Parent'),cb)); assert(isequal(get(cb,'Parent'),f)); assert(isequal(get(at,'Parent'),ax));
        assert(isequal(findall(f,'Type','text','String','Axes title'),at));
        assert(isequal(findall(f,'Type','text','String','Side label'),bl)); assert(isequal(findall(f,'Type','text','String','Bar title'),bt));
        assert(isequal(findall(bl,'Type','text'),bl)); assert(isequal(findall(bt,'Type','text'),bt));
        f2=figure; ax2=axes; assert(isempty(findall(f2,'Type','text','String','Side label'))); assert(isempty(findall(f2,'Type','text','String','Bar title')));
        assert(isempty(findall(ax,'Type','text','String','Side label'))); assert(isempty(findall(ax,'Type','text','String','Bar title')));
        figure(f); colorbar off; assert(isempty(findall(f,'Type','text','String','Side label'))); assert(isempty(findall(f,'Type','text','String','Bar title'))); assert(isequal(findall(f,'Type','text','String','Axes title'),at));").unwrap();
    assert!(e.execute("findall(bl,'Type','text')").is_err());
    assert!(e.execute("findall(bt,'Type','text')").is_err());
}

#[test]
fn fix13_findall_colorbar_scoped_reproduction() {
    let mut e = Engine::default();
    e.execute("f=figure('Visible','off'); ax=axes; f2=figure; ax2=axes; set(ax2,'Visible','off');
        set(ax,'Visible','on'); assert(strcmp(get(ax,'Visible'),'on')); assert(isequal(findall(f,'Type','axes','Visible',get(ax,'Visible')),ax)); assert(isempty(findall(f,'Type','axes','Visible','off'))); assert(isequal(findall(ax,'Type','axes','Visible','on'),ax)); assert(isempty(findall(f2,'Type','axes','Visible','on')));
        set(ax,'Visible','off'); assert(strcmp(get(ax,'Visible'),'off')); assert(isequal(findall(f,'Type','axes','Visible',get(ax,'Visible')),ax)); assert(isempty(findall(f,'Type','axes','Visible','on'))); assert(isequal(findall(f2,'Type','axes','Visible','off'),ax2)); assert(isempty(findall(f,'Type','line','Visible','off')));
        assert(isequal(findobj(f,'Type','axes'),ax)); assert(strcmp(get(0,'ShowHiddenHandles'),'off'));").unwrap();
}

#[test]
fn fix13_findall_mixed_figure_handles_hidden_text_and_removal() {
    let mut e = Engine::default();
    e.execute(
        "f = figure(4);
         ax = axes;
         h1 = plot(1:2, 'DisplayName', 'alpha');
         hold on;
         h2 = plot(3:4, 'DisplayName', 'beta');
         set(h2, 'LineWidth', 2);
         th = title(ax, 'TitleText');
         xh = xlabel(ax, 'XText');
         yh = ylabel(ax, 'YText');
         tx = text(0.2, 0.3, 'Note');
         lg = legend(ax, 'show');
         zh = get(ax, 'ZLabel');
         sh = get(ax, 'Subtitle');
         title_again = get(ax, 'Title');
         lines = findall(f, 'type', 'line');
         axes_h = findall(f, 'type', 'axes');
         legends = findall(f, 'type', 'legend');
         texts = findall(f, 'tYpE', 'TeXt');
         named = findall(f, 'Type', 'line', 'DisplayName', 'alpha');
         wide = findall(f, 'type', 'line', 'LineWidth', 2);
         note = findall(f, 'type', 'text', 'String', 'Note');
         missing = findall(f, 'type', 'line', 'DisplayName', 'missing');
         wrong_case = findall(f, 'type', 'text', 'String', 'note');",
    )
    .unwrap();
    assert_eq!(values(&mut e, "isequal(title_again, th)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(get(ax, 'Title'), th)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(lines, [h1; h2])"), vec![1.]);
    assert_eq!(
        values(&mut e, "isequal(lines, findobj(f, 'type', 'line'))"),
        vec![1.]
    );
    assert_eq!(values(&mut e, "isequal(axes_h, ax)"), vec![1.]);
    assert_eq!(
        values(&mut e, "isequal(axes_h, findobj(f, 'Type', 'axes'))"),
        vec![1.]
    );
    assert_eq!(values(&mut e, "isequal(legends, lg)"), vec![1.]);
    assert_eq!(
        values(&mut e, "isequal(texts, [th; xh; yh; zh; sh; tx])"),
        vec![1.]
    );
    assert_eq!(values(&mut e, "isequal(named, h1)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(wide, h2)"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(note, tx)"), vec![1.]);
    assert_eq!(values(&mut e, "isempty(missing)"), vec![1.]);
    assert_eq!(values(&mut e, "isempty(wrong_case)"), vec![1.]);
    assert_eq!(text(&mut e, "get(0, 'ShowHiddenHandles')"), "off");
    e.execute(
        "for k = 1:numel(lines)
             assert(strcmp(get(lines(k), 'Type'), 'line'));
         end
         assert(strcmp(get(lines(1), 'DisplayName'), 'alpha'));
         assert(strcmp(get(lines(2), 'DisplayName'), 'beta'));
         assert(isequal(get(lines(2), 'LineWidth'), 2));
         expected = {'TitleText'; 'XText'; 'YText'; ''; ''; 'Note'};
         for k = 1:numel(texts)
             assert(strcmp(get(texts(k), 'Type'), 'text'));
             assert(strcmp(get(texts(k), 'String'), expected{k}));
         end
         assert(strcmp(get(legends, 'Type'), 'legend'));
         assert(strcmp(get(axes_h, 'Type'), 'axes'));
         kids = get(ax, 'Children');
         in_kids = 0;
         for k = 1:numel(kids)
             if isequal(kids(k), th)
                 in_kids = in_kids + 1;
             end
         end
         assert(in_kids == 0);",
    )
    .unwrap();
    // findobj finds visible text only; hidden title/labels stay out (main 9b widened Type).
    assert_eq!(
        values(&mut e, "isequal(findobj(f, 'type', 'text'), tx)"),
        vec![1.]
    );
    e.execute(
        "f2 = figure(5);
         h3 = plot(8:9);
         set(h3, 'LineWidth', 1);
         assert(isequal(findall(gcf, 'type', 'line'), h3));
         assert(isequal(findall(f, 'type', 'line'), [h1; h2]));
         assert(isempty(findall(f2, 'type', 'legend')));
         assert(isequal(findall(f, 'type', 'line', 'LineWidth', 2), h2));
         assert(isequal(findall(ax, 'type', 'line'), [h1; h2]));
         assert(isequal(findall(h1, 'type', 'line'), h1));
         set(h2, 'Visible', 'off');
         assert(isequal(findobj(f, 'type', 'line'), [h1; h2]));
         assert(isequal(findall(f, 'type', 'line', 'Visible', 'off'), h2));
         assert(isequal(findall(f, 'TYPE', 'line', 'visible', 'ON'), h1));
         delete(h2);
         left = findall(f, 'type', 'line');
         assert(isequal(left, h1));
         assert(isequal(findall(f2, 'type', 'line'), h3));
         delete(tx);
         rest = findall(f, 'type', 'text');
         assert(isequal(rest, [th; xh; yh; zh; sh]));
         assert(isequal(get(ax, 'Title'), th));
         delete(h1);
         assert(isempty(findall(f, 'type', 'line')));",
    )
    .unwrap();
    let stale_line = e.execute("findall(h2, 'type', 'line')").unwrap_err();
    assert!(
        stale_line.contains("stale"),
        "deleted line scope: {stale_line}"
    );
    let stale_text = e.execute("findall(tx, 'type', 'text')").unwrap_err();
    assert!(
        stale_text.contains("stale"),
        "deleted text scope: {stale_text}"
    );
}

#[test]
fn fix13_findall_includes_hidden_dialogs_without_reordering_findobj() {
    let mut e = Engine::default();
    e.execute(
        "f2 = figure(2);
         d = waitbar(0);
         f1 = figure(1);
         set(0, 'ShowHiddenHandles', 'off');
         hidden = findall(0, 'type', 'figure');
         visible = findobj(0, 'type', 'figure');
         direct = findall(d, 'type', 'figure');",
    )
    .unwrap();
    assert_eq!(values(&mut e, "isequal(visible, [f1; f2])"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(hidden, [f1; f2; d])"), vec![1.]);
    assert_eq!(values(&mut e, "isequal(direct, d)"), vec![1.]);
    e.execute(
        "set(0, 'ShowHiddenHandles', 'on');
         both = findobj(0, 'type', 'figure');
         assert(isequal(both, [f1; f2; d]));
         assert(isequal(findall(0, 'type', 'figure'), both));
         assert(isequal(findall(groot, 'Type', 'FIGURE'), both));",
    )
    .unwrap();
}

#[test]
fn root_properties_lists_root_table_with_or_without_assignment() {
    let mut e = Engine::default();
    let displayed = e.execute("properties(groot)").unwrap().output.join("\n");
    assert!(displayed.contains("ScreenSize"), "{displayed}");
    assert!(!displayed.contains("LineWidth"), "{displayed}");
    e.execute("p=properties(groot); m=properties('MException');")
        .unwrap();
    assert_eq!(values(&mut e, "isequal(p,fieldnames(groot))"), vec![1.]);
    assert_eq!(values(&mut e, "any(strcmp(p,'ScreenSize'))"), vec![1.]);
    assert_eq!(values(&mut e, "any(strcmp(p,'LineWidth'))"), vec![0.]);
    assert_eq!(text(&mut e, "m{1}"), "identifier");
}
