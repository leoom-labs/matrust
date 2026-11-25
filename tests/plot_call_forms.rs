use matrust::{
    Engine, files,
    plot::{Figure, MAX_PLOT_SERIES, Position},
};

#[test]
fn subplot_three_digit_forms_match_grid_positions() {
    let mut engine = Engine::default();
    for rows in 1..=9 {
        for columns in 1..=9 {
            for index in 1..=9 {
                let code = 100 * rows + 10 * columns + index;
                let result = engine.execute(&format!("clf;subplot({code});"));
                if index > rows * columns {
                    assert!(result.is_err(), "{code}");
                    continue;
                }
                result.unwrap_or_else(|error| panic!("{code}: {error}"));
                assert_eq!(
                    engine.figure.as_ref().unwrap().active_position(),
                    Position::from_grid(rows, columns, &[index]).unwrap(),
                    "{code}"
                );
            }
        }
    }
    engine
        .execute(
            "clf;a=subplot(211);p1=get(a,'Position');b=subplot(2,1,1);p2=get(b,'Position');\
             cli=subplot(221);p3=get(cli,'Position');subplot 2 2 1;subplot 221;",
        )
        .unwrap();
    assert_eq!(engine.variables["p1"].data(), engine.variables["p2"].data());
    assert_eq!(
        engine.figure.as_ref().unwrap().active_position(),
        Position::from_grid(2, 2, &[1]).unwrap()
    );
    for bad in [
        "subplot(21)",
        "subplot(201)",
        "subplot(211.5)",
        "subplot(1111)",
    ] {
        assert!(engine.execute(bad).is_err(), "{bad}");
    }
}

#[test]
fn subplot_reuses_moves_replaces_and_sets_axes_properties() {
    let mut engine = Engine::default();
    engine
        .execute(
            "a=subplot(2,2,1,'align','Tag','kept','XGrid','on');h=plot([1 2]);hold on;\
             b=subplot(2,2,2);plot([3 4]);c=subplot(2,2,2,a);\
             position=get(c,'Position');tag=get(c,'Tag');old=isgraphics(b);\
             same=isequal(a,c,gca);",
        )
        .unwrap();
    let figure = engine.figure.as_ref().unwrap();
    assert_eq!(engine.logical_variables["same"].data(), &[1]);
    assert_eq!(engine.logical_variables["old"].data(), &[0]);
    assert_eq!(figure.axes.extras.tag, "kept");
    assert_eq!(figure.series[0].points.get(1), Some([2., 2.]));
    assert_eq!(figure.all_axes().count(), 1);
    assert_eq!(
        figure.active_position(),
        Position::from_grid(2, 2, &[2]).unwrap()
    );
    let before = figure.clone();
    assert!(engine.execute("subplot(2,2,1,a,'Visible','bad')").is_err());
    assert!(engine.figure.as_ref().unwrap().same_snapshot(&before));

    engine
        .execute(
            "r=subplot(2,2,2,'REPLACE','Position',[0.2 0.2 0.3 0.3],\
             'Tag','new','Visible','off','XGrid','on');\
             p=get(r,'Position');hidden=strcmp(get(r,'Visible'),'off');dead=isgraphics(a);\
             set(gca,'Tag','edited','Visible','on');",
        )
        .unwrap();
    let figure = engine.figure.as_ref().unwrap();
    assert!(figure.series.is_empty());
    assert_eq!(engine.logical_variables["dead"].data(), &[0]);
    assert_eq!(engine.logical_variables["hidden"].data(), &[1]);
    assert_eq!(engine.variables["p"].data(), &[0.2, 0.2, 0.3, 0.3]);
    assert!(figure.axes.xgrid);
    assert!(!figure.axes.extras.axis_off);
    assert_eq!(figure.axes.extras.tag, "edited");

    engine.execute("figure(2);subplot(1,1,1,r);").unwrap();
    assert_eq!(engine.current_figure, 1);
    assert_eq!(
        engine.figure.as_ref().unwrap().active_position(),
        Position::default()
    );
}

fn svg_plot_box(figure: &Figure) -> [f64; 2] {
    let svg = figure.svg();
    let rect = svg
        .split_once("<clipPath")
        .unwrap()
        .1
        .split_once("<rect ")
        .unwrap()
        .1
        .split_once("/>")
        .unwrap()
        .0;
    ["width=\"", "height=\""].map(|attribute| {
        rect.split_once(attribute)
            .unwrap()
            .1
            .split_once('"')
            .unwrap()
            .0
            .parse()
            .unwrap()
    })
}

#[test]
fn square_axis_changes_drawn_box_and_normal_restores_it() {
    let mut engine = Engine::default();
    engine
        .execute("plot([0 100],[0 1]);axis([0 100 0 1]);axis square;ratio=get(gca,'PlotBoxAspectRatio');")
        .unwrap();
    let figure = engine.figure.as_ref().unwrap();
    let [width, height] = svg_plot_box(figure);
    assert!(width > 0.);
    assert!((width - height).abs() < 1e-9);
    assert_eq!(engine.variables["ratio"].data(), &[1., 1., 1.]);
    assert_eq!(figure.bounds(), ([0., 100.], [0., 1.]));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("square.mrfig");
    files::save_figure(&path, figure).unwrap();
    assert_eq!(
        svg_plot_box(&files::load_figure(&path).unwrap()),
        [width, height]
    );

    engine.execute("axis normal;").unwrap();
    let [width, height] = svg_plot_box(engine.figure.as_ref().unwrap());
    assert!(width > height);
    engine.execute("axis('square');").unwrap();
    let [width, height] = svg_plot_box(engine.figure.as_ref().unwrap());
    assert!((width - height).abs() < 1e-9);
}

#[test]
fn legend_warns_for_extra_labels_and_leaves_remaining_series_out() {
    let mut engine = Engine::default();
    engine.execute("h=plot([1 2;3 4;5 6]);").unwrap();
    let output = engine
        .execute("lg=legend('first','second','extra','Location','south');")
        .unwrap();
    assert_eq!(output.output, ["Warning: Ignoring extra legend entries."]);
    let entries = |engine: &Engine| {
        engine
            .figure
            .as_ref()
            .unwrap()
            .legend_entries()
            .map(|(index, series)| (index, series.label.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        entries(&engine),
        [(0, "first".into()), (1, "second".into())]
    );
    let output = engine.execute("legend({'only'});").unwrap();
    assert!(output.output.is_empty());
    assert_eq!(entries(&engine), [(0, "only".into())]);
    let output = engine
        .execute("legend(h(2),{'selected','extra'});")
        .unwrap();
    assert_eq!(output.output, ["Warning: Ignoring extra legend entries."]);
    assert_eq!(entries(&engine), [(1, "selected".into())]);
}

#[test]
fn wide_plot_draws_all_lines_and_cycles_color_order_within_storage_budget() {
    let mut engine = Engine::default();
    engine
        .execute(
            "colororder([1 0 0;0 1 0;0 0 1]);h=plot(rand(3,70));n=numel(h);c=get(h(70),'Color');",
        )
        .unwrap();
    let figure = engine.figure.as_ref().unwrap();
    assert_eq!(figure.series_count(), 70);
    assert_eq!(figure.point_count(), 210);
    assert_eq!(engine.variables["n"].data(), &[70.]);
    assert_eq!(engine.variables["c"].data(), &[1., 0., 0.]);
    let svg = figure.svg();
    for color in ["#ff0000", "#00ff00", "#0000ff"] {
        assert!(svg.matches(&format!("stroke=\"{color}\"")).count() >= 23);
    }
    for (index, series) in figure.series.iter().enumerate() {
        let mut expected = [0; 3];
        expected[index % 3] = 255;
        assert_eq!(series.color, expected);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.mrfig");
    files::save_figure(&path, figure).unwrap();
    assert_eq!(files::load_figure(&path).unwrap().svg(), svg);
    let before = figure.clone();
    for script in [
        "plot(zeros(10001,100));".to_owned(),
        format!("plot(zeros(2,{}));", MAX_PLOT_SERIES + 1),
    ] {
        assert!(
            engine.execute(&script).unwrap_err().contains("exceeds"),
            "{script}"
        );
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&before));
    }
}
