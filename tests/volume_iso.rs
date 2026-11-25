//! Geometry references from tool/refs/volume/iso.py and tool/refs/volume/iso.m.
use matrust::{Engine, Matrix, plot::view3d};
use serde::Deserialize;

const GRID: &str = "[x,y,z] = meshgrid(1:9,1:7,1:8);";
const SPHERE: &str = "S = 2.75-sqrt((x-5).^2+(y-4).^2+(z-4).^2);";
const FLOW: &str = "
    T=.875+.0625*(x-1)-((y-4+.125*(x-4)).^2+(z-4).^2);
    xx=[.25,1,1.5,1.875,2.25,2.625,3.125,3.75,4.25];
    yy=[-1,-.625,-.25,.125,.5,.875,1.25];
    zz=[-2,-1.25,-.75,-.125,.375,1,1.75,2.75];
";

#[derive(Deserialize)]
struct Skimage {
    sphere: SortedMesh,
    flow: SortedMesh,
    plane: SortedMesh,
}

#[derive(Deserialize)]
struct SortedMesh {
    faces: usize,
    vertices: Vec<[f64; 3]>,
}

#[derive(Deserialize)]
struct Octave {
    sphere: OrderedMesh,
    flow: OrderedMesh,
    noshare: [usize; 2],
    default: [usize; 2],
    sphere_normal: [f64; 3],
    caps: CapCounts,
}

#[derive(Deserialize)]
struct OrderedMesh {
    faces: Vec<[f64; 3]>,
    vertices: Vec<[f64; 3]>,
    colors: Option<Vec<f64>>,
}

#[derive(Deserialize)]
struct CapCounts {
    above: [[usize; 3]; 7],
    below: [[usize; 3]; 7],
}

fn skimage() -> Skimage {
    serde_json::from_str(include_str!("fixtures/volume/iso-skimage.json")).unwrap()
}

fn octave() -> Octave {
    serde_json::from_str(include_str!("fixtures/volume/iso-octave.json")).unwrap()
}

fn run(script: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(script).unwrap();
    e
}

fn rows(m: &Matrix) -> Vec<[f64; 3]> {
    assert_eq!(m.cols(), 3);
    (0..m.rows())
        .map(|r| [m.get(r, 0), m.get(r, 1), m.get(r, 2)])
        .collect()
}

fn assert_vertices(actual: &[[f64; 3]], expected: &[[f64; 3]]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert!(
            (0..3).all(|axis| (a[axis] - b[axis]).abs() <= 1e-9),
            "{a:?} != {b:?}"
        );
    }
}

fn check_geometry(e: &Engine, py: &SortedMesh, octave: &OrderedMesh) {
    let mut vertices = rows(&e.variables["v"]);
    let faces = rows(&e.variables["f"]);
    assert_eq!(faces.len(), octave.faces.len());
    assert_eq!(faces.len(), py.faces);
    assert_eq!(vertices.len(), octave.vertices.len());
    // Compare each oriented triangle in sequence. Octave may choose a
    // different representative of a shared edge after rounding.
    for (face, reference) in faces.into_iter().zip(&octave.faces) {
        let triangle = face.map(|i| vertices[i as usize - 1]);
        let expected = reference.map(|i| octave.vertices[i as usize - 1]);
        assert_vertices(&triangle, &expected);
    }
    let key = |p: &[f64; 3]| p.map(|v| (v * 1e10).round() as i64);
    vertices.sort_by_key(key);
    let mut expected = py.vertices.clone();
    expected.sort_by_key(key);
    assert_vertices(&vertices, &expected);
}

#[test]
fn volume_iso_sphere_has_reference_vertex_set_winding_and_triangle_order() {
    let e = run(&format!("{GRID}{SPHERE}[f,v] = isosurface(S,0);"));
    check_geometry(&e, &skimage().sphere, &octave().sphere);
}

#[test]
fn volume_iso_flow_uses_nonuniform_coordinates_and_interpolates_vertex_colors() {
    let e = run(&format!(
        "{GRID}{FLOW}
        [f,v,c]=isosurface(xx,yy,zz,T,0,2*x-3*y+5*z);
        [X,Y,Z]=meshgrid(xx,yy,zz);
        fv=isosurface(X,Y,Z,T,0,2*x-3*y+5*z);
        fv_vertices=fv.vertices; fv_faces=fv.faces; fv_c=fv.facevertexcdata;"
    ));
    let reference = octave().flow;
    check_geometry(&e, &skimage().flow, &reference);
    let vertices = rows(&e.variables["v"]);
    let colors = e.variables["c"].data();
    let expected = reference.colors.unwrap();
    assert_eq!(colors.len(), expected.len());
    for (point, &color) in vertices.iter().zip(colors) {
        let index = reference
            .vertices
            .iter()
            .position(|v| (0..3).all(|axis| (point[axis] - v[axis]).abs() <= 1e-9))
            .unwrap();
        assert!((color - expected[index]).abs() <= 1e-12);
    }
    for (a, b) in [("f", "fv_faces"), ("v", "fv_vertices"), ("c", "fv_c")] {
        assert_eq!(e.variables[a].data(), e.variables[b].data());
    }
}

#[test]
fn volume_iso_defaults_noshare_and_verbose_follow_the_requested_form() {
    let mut e = run(&format!("{GRID}{SPHERE}"));
    let output = e
        .execute(
            "[f,v]=isosurface(S,0,'noshare','verbose');
             [df,dv]=isosurface(S); [ef,ev]=isosurface(S,[]);
             [pf,pv]=isosurface(z,4.25); [xf,xv]=isosurface(z,20);",
        )
        .unwrap();
    let reference = octave();
    let size = |f, v| [e.variables[f].rows(), e.variables[v].rows()];
    assert_eq!(size("f", "v"), reference.noshare);
    assert_eq!(size("df", "dv"), reference.default);
    assert_eq!(e.variables["df"].data(), e.variables["ef"].data());
    assert_eq!(e.variables["dv"].data(), e.variables["ev"].data());
    assert!(output.output.iter().any(|s| s.contains("extracting")));
    assert!(output.output.iter().any(|s| s.contains("faces")));
    let py = skimage().plane;
    let mut points = rows(&e.variables["pv"]);
    points.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_vertices(&points, &py.vertices);
    assert_eq!(e.variables["pf"].rows(), py.faces);
    assert_eq!(size("xf", "xv"), [0, 0]);
    assert_eq!(e.variables["xv"].cols(), 3);
}

#[test]
fn volume_iso_isonormals_are_outward_for_a_positive_inside_sphere() {
    let e = run(&format!(
        "{GRID}{SPHERE}
        [f,v]=isosurface(S,0); n=isonormals(S,v); neg=isonormals(x,y,z,S,v,'negate');
        sample=isonormals(S,[7.75 4 4]);
        [xx,yy,zz]=meshgrid([2 1 -1],[0 3 4],[-3 1 3]);
        linear_normal=isonormals(xx,yy,zz,2*xx-3*yy+6*zz,[.5 2 2],'negate');"
    ));
    let vertices = rows(&e.variables["v"]);
    let normals = rows(&e.variables["n"]);
    let neg = rows(&e.variables["neg"]);
    assert_eq!(normals.len(), vertices.len());
    for ((v, n), negative) in vertices.iter().zip(&normals).zip(&neg) {
        let r = [v[0] - 5., v[1] - 4., v[2] - 4.];
        let length = r[0].hypot(r[1]).hypot(r[2]);
        let dot = (0..3).map(|axis| r[axis] * n[axis]).sum::<f64>() / length;
        assert!(dot > 0.99, "{v:?} {n:?}: {dot}");
        assert!((n[0].hypot(n[1]).hypot(n[2]) - 1.).abs() <= 1e-12);
        assert_eq!(*negative, n.map(|v| -v));
    }
    let reference = octave().sphere_normal;
    let length = reference[0].hypot(reference[1]).hypot(reference[2]);
    assert_vertices(
        &rows(&e.variables["sample"]),
        &[reference.map(|v| v / length)],
    );
    assert_vertices(
        &rows(&e.variables["linear_normal"]),
        &[[2. / 7., -3. / 7., 6. / 7.]],
    );
}

#[test]
fn volume_iso_cap_counts_match_octave_and_colors_follow_boundary_samples() {
    let mut e = run(&format!("{GRID}Q=z+.5*(x-3).*(y-3);"));
    let reference = octave().caps;
    let planes = ["all", "xmin", "xmax", "ymin", "ymax", "zmin", "zmax"];
    for (side, counts) in [("above", reference.above), ("below", reference.below)] {
        for (i, plane) in planes.iter().enumerate() {
            e.execute(&format!("[f,v,c]=isocaps(Q,4.375,'{side}','{plane}');"))
                .unwrap();
            let [f, v, c] = ["f", "v", "c"].map(|n| &e.variables[n]);
            assert_eq!([f.rows(), v.rows(), c.rows()], counts[i], "{side} {plane}");
            if i > 0 {
                let (axis, coordinate) =
                    [(0, 1.), (0, 9.), (1, 1.), (1, 7.), (2, 1.), (2, 8.)][i - 1];
                for (point, &color) in rows(v).iter().zip(c.data()) {
                    assert_eq!(point[axis], coordinate);
                    assert!(if side == "above" {
                        color >= 4.375 - 1e-12
                    } else {
                        color <= 4.375 + 1e-12
                    });
                }
            }
        }
    }
    e.execute("[f,v,c]=isocaps(x,y,z,Q,4.375,'xmin');").unwrap();
    assert_eq!(e.variables["f"].rows(), reference.above[1][0]);
}

#[test]
fn volume_iso_patch_struct_and_named_inputs_render_and_keep_handle_normals() {
    let mut e = run(&format!(
        "{GRID}{SPHERE}
         fv=isosurface(S,0,x); p=patch(fv,'FaceColor','interp','EdgeColor','none'); view(3);
         faces=get(p,'Faces'); expected_faces=fv.faces; vertices=p.Vertices; c=get(p,'FaceVertexCData');
         isonormals(S,p); n=get(p,'VertexNormals'); returned=isonormals(x,y,z,S,p);"
    ));
    let triangle_count = octave().sphere.faces.len();
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.axes.view, view3d::VIEW_3D);
    assert_eq!(fig.series[0].patch_face_count(), triangle_count);
    assert_eq!(
        fig.series[0].face_colors.as_ref().unwrap().len(),
        triangle_count
    );
    assert_eq!(fig.series[0].z.len(), triangle_count * 3);
    assert_eq!(
        view3d::display_view(&fig.axes).series[0].patch_face_count(),
        triangle_count
    );
    assert_eq!(e.variables["n"].data(), e.variables["returned"].data());
    assert_eq!(
        e.variables["faces"].data(),
        e.variables["expected_faces"].data()
    );
    e.execute("pn=patch('Faces',faces,'Vertices',vertices,'FaceVertexCData',c); view(3);")
        .unwrap();
    assert_eq!(
        e.figure.as_ref().unwrap().series[1].patch_face_count(),
        triangle_count
    );
}

#[test]
fn volume_iso_only_the_unheld_isosurface_draw_changes_view() {
    let e = run(&format!(
        "{GRID}
        figure; patch('Faces',[1 2 3],'Vertices',[0 0 0;1 0 0;0 1 0]);
        assert(isequal(get(gca,'View'),[0 90]));
        figure; view(12,34); p=patch(isosurface(x,2.5));
        assert(isequal(get(gca,'View'),[12 34]));
        isocaps(x,2.5); assert(isequal(get(gca,'View'),[12 34]));
        hold on; isosurface(x,2.5); assert(isequal(get(gca,'View'),[12 34]));
        hold off; isosurface(x,2.5);"
    ));
    assert_eq!(e.figure.unwrap().axes.view, view3d::VIEW_3D);
}

#[test]
fn volume_iso_indexed_patch_facecolor_set_get_and_dot() {
    run(&format!(
        "{GRID}
        p=patch(isocaps(x,2.5));
        set(p,'FaceColor','flat'); assert(strcmp(p.FaceColor,'flat'));
        set(p,'FaceColor','none'); assert(strcmp(get(p,'FaceColor'),'none'));
        p.FaceColor='interp'; assert(strcmp(get(p,'FaceColor'),'interp'));
        set(p,'FaceColor',[1 0 0]); assert(isequal(p.FaceColor,[1 0 0]));
        set(p,'FaceColor','interp'); assert(strcmp(p.FaceColor,'interp'));"
    ));
}

#[test]
fn volume_iso_mapped_patch_recolors_rendered_triangles() {
    let mut e = run("V=zeros(2,2,2); V(1,1,1)=1; fv=isosurface(V,.5);
        c=ones(3,1); c(fv.faces(1,1))=0;
        p=patch(fv,'FaceVertexCData',c,'FaceColor','interp','EdgeColor','none');
        view(3);");
    let fill = |e: &Engine| {
        let svg = e.figure.as_ref().unwrap().svg();
        svg.split_once("<polygon fill=\"")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap()
            .to_owned()
    };
    let original = fill(&e);
    e.execute("colormap(hot);").unwrap();
    assert_ne!(fill(&e), original);
    e.execute("colormap([1 0 0;0 0 1]);").unwrap();
    assert_eq!(fill(&e), "#5500aa");
    e.execute("caxis([0 4]);").unwrap();
    assert_eq!(fill(&e), "#ff0000");
    e.execute("clim('auto');").unwrap();
    assert_eq!(fill(&e), "#5500aa");
    e.execute("p.FaceColor='flat';").unwrap();
    assert_eq!(fill(&e), "#ff0000");
    e.execute("ax=gca; ax.CLim=[-2 0];").unwrap();
    assert_eq!(fill(&e), "#0000ff");
    e.execute("set(p,'FaceColor','none');").unwrap();
    assert!(!e.figure.as_ref().unwrap().svg().contains("<polygon fill="));
    e.execute("p.FaceColor=[0 1 0]; colormap(gray);").unwrap();
    assert_eq!(fill(&e), "#00ff00");
}

#[test]
fn volume_iso_patch_defaults_and_explicit_draw_colors() {
    run(&format!(
        "{GRID}
        fv=isosurface(x,2.5); p=patch(fv);
        assert(isequal(p.FaceColor,[0 0 0])); assert(isequal(p.EdgeColor,[0 0 0]));
        q=patch(isocaps(x,2.5)); assert(isequal(q.FaceColor,[0 0 0]));
        assert(isequal(q.EdgeColor,[0 0 0]));
        set(gcf,'DefaultPatchFaceColor',[1 0 0],'DefaultPatchEdgeColor',[0 1 0]);
        p=patch(fv); assert(isequal(p.FaceColor,[1 0 0])); assert(isequal(p.EdgeColor,[0 1 0]));
        figure; isosurface(x,2.5); h=get(gca,'Children');
        assert(strcmp(h.FaceColor,'flat')); assert(strcmp(h.EdgeColor,'none'));
        assert(isequal(h.FaceVertexCData,2.5*ones(size(h.Vertices,1),1)));
        figure; isosurface(x,2.5,y); h=get(gca,'Children');
        assert(strcmp(h.FaceColor,'flat')); assert(isequal(h.FaceVertexCData,h.Vertices(:,2)));
        figure; isocaps(x,2.5); h=get(gca,'Children');
        assert(strcmp(h.FaceColor,'interp')); assert(strcmp(h.EdgeColor,'none'));"
    ));
}

#[test]
fn volume_iso_patch_and_surf_share_axes_color_limits() {
    let e = run("V=zeros(2,2,2); V(1,1,1)=1; fv=isosurface(V,.5);
        p=patch(fv,'FaceVertexCData',ones(3,1),'FaceColor','flat');
        hold on; surf(zeros(2),[-1 0;0 0]); ax=gca; limits=clim;
        figure; colormap(ax,[1 0 0;0 1 0]); figure(1);");
    assert_eq!(e.variables["limits"].data(), &[-1., 1.]);
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.series[0].face_fill_color(0), Some([0, 255, 0]));
    assert_eq!(fig.series[1].face_fill_color(0), Some([255, 0, 0]));
}

#[test]
fn volume_iso_draw_forms_add_patches_to_current_axes() {
    let mut e = run(&format!("{GRID}{SPHERE}figure; ax=gca; isosurface(S,0);"));
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.axes.view, view3d::VIEW_3D);
    assert_eq!(
        fig.series[0].patch_face_count(),
        octave().sphere.faces.len()
    );
    e.execute("Q=z+.5*(x-3).*(y-3); isocaps(Q,4.375,'above','xmin');")
        .unwrap();
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.series[1].patch_face_count(), octave().caps.above[1][0]);
    assert!(fig.series[1].face_colors.is_some());
}

#[test]
fn volume_iso_validates_inputs_and_handles_threshold_corners() {
    let mut e = run(&format!(
        "{GRID}{SPHERE}[f,v]=isosurface(x,5); [ef,ev]=isosurface(ones(2,2,2),1);"
    ));
    assert_eq!(e.variables["f"].rows(), 2 * 6 * 7);
    assert_eq!(e.variables["v"].rows(), 7 * 8);
    assert_eq!(e.variables["ef"].rows(), 0);
    assert_eq!(e.variables["ev"].rows(), 0);
    for source in [
        "isosurface(zeros(2),0);",
        "isosurface(S,NaN);",
        "isosurface(S,0,ones(2,2,2));",
        "isosurface(S,0,'unknown');",
        "isonormals(S,ones(2,2));",
        "isocaps(S,0,'xmid');",
        "patch('Faces',[1 2 4],'Vertices',[0 0 1;1 0 1;0 1 1]);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
        assert!(e.figure.is_none(), "{source}");
    }
}

#[test]
fn volume_iso_triangles_keep_their_corners_when_depth_sorted_with_surf_quads() {
    let e = run("surf([2 2;2 2]); hold on;
         patch('Faces',[1 2 3],'Vertices',[1 1 1;2 1 1;1 2 1]); view(3);");
    let fig = e.figure.as_ref().unwrap();
    let display = view3d::display_view(&fig.axes);
    let draw = &display.series[0];
    assert_eq!(draw.patch_face_count(), 2);
    assert_eq!(draw.points.len(), 8);
    let points: Vec<_> = draw.points.iter().collect();
    let triangles = points
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[2] == p[3])
        .count();
    assert_eq!(triangles, 1);
    assert!(points.iter().flatten().all(|v| v.is_finite()));
}
