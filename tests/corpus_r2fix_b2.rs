use matrust::Engine;

fn roundtrip_engine() -> (tempfile::TempDir, Engine) {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(directory.path()).unwrap();
    (directory, engine)
}

#[test]
fn w4_native_colorbar_keeps_title() {
    let (_dir, mut e) = roundtrip_engine();
    e.execute(
        "plot(0:1); c=colorbar; c.Title.String='T'; savefig('bar.mrfig'); f=openfig('bar.mrfig');",
    )
    .unwrap();
    assert!(
        e.figure.as_ref().unwrap().extras.colorbar.is_some(),
        "reopened axes have no colorbar"
    );
    e.execute("t=findall(f,'Type','text','String','T'); b=get(t,'Parent'); \
         assert(strcmp(b.Visible,'on')); assert(strcmp(b.Title.String,'T')); assert(~isequal(b,c));").unwrap();
}

#[test]
fn w5_fig_keeps_axes_layer() {
    let (_dir, mut e) = roundtrip_engine();
    e.execute("plot(0:1); set(gca,'Layer','top'); savefig('layer.fig'); openfig('layer.fig');")
        .unwrap();
    assert_eq!(e.figure.as_ref().unwrap().extras.layer.as_str(), "top");
}

#[test]
fn w6_indexed_patch_supports_xy_vertices_and_xyz_quads() {
    let mut e = Engine::default();
    let triangle = e.execute("t=patch('Parent',gca,'vertices',[0 0;1 0;0 1],'faces',[1 2 3]);");
    let quad =
        e.execute("q=patch('Parent',gca,'Vertices',[0 0 0;1 0 1;1 1 2;0 1 3],'Faces',[1 2 3 4]);");
    assert!(
        triangle.is_ok() && quad.is_ok(),
        "triangle {:?}, quad {:?}",
        triangle.err(),
        quad.err()
    );
    e.execute(
        "assert(isequal(t.Faces,[1 2 3])); assert(isequal(t.Vertices,[0 0;1 0;0 1])); \
         assert(isequal(q.Faces,[1 2 3 4]));",
    )
    .unwrap();
    let series = &e.figure.as_ref().unwrap().series;
    assert_eq!(series[0].patch_face_count(), 1);
    assert!(!series[0].is_3d());
    assert_eq!(series[1].patch_face_count(), 1);
    assert_eq!(
        series[1].points.iter().collect::<Vec<_>>(),
        [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
    );
    assert_eq!(&series[1].z[..], &[0., 1., 2., 3.]);
    assert!(
        e.execute("patch('Vertices',q.Vertices,'Faces',[1 2 3 5]);")
            .unwrap_err()
            .contains("valid one-based indices")
    );
}

#[test]
fn w7_closed_five_row_patch_draws_one_square() {
    let mut e = Engine::default();
    e.execute("patch([0;1;1;0;0],[0;0;1;1;0],1,'FaceColor',[1 0 0]);")
        .unwrap();
    let figure = e.figure.as_ref().unwrap();
    let series = &figure.series[0];
    assert_eq!(figure.svg().matches("<polygon fill=\"#ff0000\"").count(), 1);
    assert_eq!(series.patch_face_count(), 1);
    assert_eq!(series.color, [255, 0, 0]);
    assert_eq!(
        series.points.iter().collect::<Vec<_>>(),
        [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
    );
}

#[test]
fn r1_16_trisurf_native_roundtrip_keeps_connectivity_and_vertices() {
    let (_dir, mut e) = roundtrip_engine();
    e.execute(
        "F=[3 1 2;2 4 3]; X=[0 1 0 1 9]; Y=[0 0 1 1 8]; Z=[0 1 2 3 7]; \
         trisurf(F,X,Y,Z); savefig('mesh.mrfig'); f=openfig('mesh.mrfig');",
    )
    .unwrap();
    e.execute(
        "p=findobj(f,'Type','patch'); assert(isequal(p.Faces,F)); \
         assert(isequal(p.Vertices,[X' Y' Z'])); assert(isequal(p.FaceVertexCData,Z'));",
    )
    .unwrap();
    let series = &e.figure.as_ref().unwrap().series[0];
    assert_eq!(series.patch_face_count(), 2);
    assert_eq!(&series.z[..], &[2., 0., 1., 1., 3., 2.]);
}
