use matrust::{Array, Engine, GraphicsArray, plot::PositionConstraint};

fn handles(engine: &Engine, name: &str) -> GraphicsArray {
    let Some(Array::Graphics(array)) = engine.array(name) else {
        panic!("expected graphics array {name}");
    };
    array
}

#[test]
fn w1_findall_infinite_depth_keeps_axis_labels() {
    let mut e = Engine::default();
    e.execute("f=figure;plot(0:1);t=title('a');plain=findall(f,'Type','text');deep=findall(f,'-depth',Inf,'Type','text');").unwrap();
    let plain = handles(&e, "plain");
    let deep = handles(&e, "deep");
    assert_eq!(plain.len(), 5);
    assert_eq!(deep.len(), plain.len());
    for i in 0..plain.len() {
        assert!(
            plain
                .get_linear(i)
                .unwrap()
                .same_object(deep.get_linear(i).unwrap())
        );
    }
    let title = handles(&e, "t");
    assert!((0..deep.len()).any(|i| {
        deep.get_linear(i)
            .unwrap()
            .same_object(title.scalar_handle().unwrap())
    }));
}

#[test]
fn w2_findall_visible_matches_uiaxes_line() {
    let mut e = Engine::default();
    e.execute("f=uifigure;a=uiaxes(f);l=plot(a,0:1);plain=findall(0,'Type','line');shown=findall(0,'Type','line','Visible','on');").unwrap();
    let line = handles(&e, "l");
    for name in ["plain", "shown"] {
        let found = handles(&e, name);
        assert_eq!(found.len(), 1, "{name}");
        assert!(
            found
                .scalar_handle()
                .unwrap()
                .same_object(line.scalar_handle().unwrap())
        );
    }
    e.execute(
        "set(a,'Visible','off');assert(isequal(findall(f,'Type','axes','Visible','off'),a));
               assert(isequal(findall(0,'Type','line','Visible','on'),l));",
    )
    .unwrap();
    e.execute("set(l,'Visible','off');shown=findall(0,'Type','line','Visible','on');hidden=findall(0,'Type','line','Visible','off');").unwrap();
    assert!(handles(&e, "shown").is_empty());
    e.execute("assert(isequal(findall(a,'Type','line','Visible','off'),l));")
        .unwrap();
    assert!(
        handles(&e, "hidden")
            .scalar_handle()
            .unwrap()
            .same_object(line.scalar_handle().unwrap())
    );
}

#[test]
fn w3_bulk_set_updates_uiaxes_lines_atomically() {
    let mut e = Engine::default();
    e.execute("f=uifigure;a=uiaxes(f);plot(a,[1 2;3 4]);lines=findall(a,'Type','line');")
        .unwrap();
    assert_eq!(handles(&e, "lines").len(), 2);
    e.execute("set(lines,'LineWidth',3);").unwrap();
    assert!(
        e.execute("set(lines,'LineWidth',4,'NoSuchProperty',1);")
            .is_err()
    );
    e.execute("widths=[get(lines(1),'LineWidth') get(lines(2),'LineWidth')];")
        .unwrap();
    assert_eq!(e.variables["widths"].data(), &[3., 3.]);
    e.execute(
        "figure(7);c=plot(0:1);set(findall(0,'Type','line'),'LineWidth',5);
               widths=[get(c,'LineWidth') get(lines(1),'LineWidth') get(lines(2),'LineWidth')];",
    )
    .unwrap();
    assert_eq!(e.variables["widths"].data(), &[5., 5., 5.]);
    assert_eq!(e.current_figure, 7);
}

#[test]
fn r1_13_subplot_accepts_mixed_case_active_position() {
    let mut e = Engine::default();
    e.execute("subplot(1,1,1,'ActivePositionProperty','Position');")
        .unwrap();
    assert_eq!(
        e.figure
            .as_ref()
            .unwrap()
            .axes
            .placement
            .as_ref()
            .unwrap()
            .constraint,
        PositionConstraint::InnerPosition
    );
}

#[test]
fn r1_15_builtin_handles_forward_multiple_outputs() {
    let mut e = Engine::default();
    e.execute("[dx,dy,dz]=sphere(2);h=@sphere;[x,y,z]=h(2);")
        .unwrap();
    for (via_handle, direct) in [("x", "dx"), ("y", "dy"), ("z", "dz")] {
        let grid = &e.variables[via_handle];
        assert_eq!((grid.rows(), grid.cols()), (3, 3));
        assert_eq!(grid.data(), e.variables[direct].data());
    }
    // A second builtin exercises feval, a discarded output, and a variable
    // with the same name as the bound target.
    e.execute("s=@sort;sort=42;[~,idx]=feval(s,[8 2 4]);")
        .unwrap();
    assert_eq!(e.variables["idx"].data(), &[2., 3., 1.]);
    assert_eq!(e.variables["sort"].data(), &[42.]);
    e.execute("eq=@eqtflength;[b,a]=eq([1 2],[1 2 3 0]);")
        .unwrap();
    assert_eq!(e.variables["b"].data(), &[1., 2., 0.]);
    assert_eq!(e.variables["a"].data(), &[1., 2., 3.]);
    assert!(e.execute("[a,b,c]=s([8 2 4]);").is_err());
    e.execute("capture=@evalc;[~,v]=capture('sort+1');")
        .unwrap();
    assert_eq!(e.variables["v"].data(), &[43.]);
}
