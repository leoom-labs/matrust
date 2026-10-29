//! Ordinary script object operations; serial graph work under 1/8 pool contexts.
use matrust::{Array, Engine, Matrix};
use std::{collections::BTreeMap, hint::black_box, time::Instant};
const PATCH: &str = "p=patch(a,'XData',X,'YData',Y,'FaceColor',[1 0 0],'EdgeColor','none');";
const TEXTBOX: &str = "t=annotation(f,'textbox',[0 .9 1 .1],'String','literal title','Interpreter','none','EdgeColor','none');";
fn main() {
    if cfg!(debug_assertions) {
        panic!("Use --release");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.is_empty() || args == ["--reverse"]);
    let reverse = !args.is_empty();
    let mut workers = vec![1, 8];
    let mut faces = vec![0, 1, 64, 10000];
    let mut operations = vec![
        "patch_create",
        "textbox_create",
        "patch_update",
        "textbox_update",
    ];
    if reverse {
        workers.reverse();
        faces.reverse();
        operations.reverse();
    }
    println!("workers,faces,operation,sample,microseconds");
    for workers in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        for &faces in &faces {
            let x = Matrix::from_column_vec(
                3,
                faces,
                (0..faces)
                    .flat_map(|i| [i as f64, i as f64 + 0.5, i as f64])
                    .collect(),
            )
            .unwrap();
            let y =
                Matrix::from_column_vec(3, faces, (0..faces).flat_map(|_| [0., 0., 1.]).collect())
                    .unwrap();
            for &operation in &operations {
                for iteration in 0..9 {
                    // Input arrays, fresh graph setup and previous-output drops
                    // stay outside timing; input assignment shares storage.
                    let mut e = Engine::default();
                    e.merge_workspace(BTreeMap::from([
                        ("X".into(), Array::from(x.clone())),
                        ("Y".into(), Array::from(y.clone())),
                    ]))
                    .unwrap();
                    e.execute("f=figure;a=gca;").unwrap();
                    if operation != "patch_create" {
                        e.execute(PATCH).unwrap();
                    }
                    if operation == "textbox_update" {
                        e.execute(TEXTBOX).unwrap();
                    }
                    let before = e.figure.clone().unwrap();
                    let script = match operation {
                        "patch_create" => PATCH,
                        "textbox_create" => TEXTBOX,
                        "patch_update" => "set(p,'FaceColor',[0 1 0],'Visible','off');",
                        "textbox_update" => {
                            "set(t,'String','changed literal title','FontSize',18);"
                        }
                        _ => unreachable!(),
                    };
                    // Includes existing pool entry, parser/dispatch, bounded
                    // validation/allocation and graph publication. No rendering.
                    let start = Instant::now();
                    let result = pool.install(|| e.execute(black_box(script)));
                    let elapsed = start.elapsed();
                    result.unwrap();
                    let after = e.figure.as_ref().unwrap();
                    after.validate().unwrap();
                    assert!(before.identity.same_object(&after.identity));
                    assert!(Array::from(x.clone()).same_snapshot(&e.array("X").unwrap()));
                    assert_eq!(after.series[0].patch_face_count(), faces);
                    for (i, p) in after.series[0].points.iter().enumerate() {
                        assert_eq!(p[0].to_bits(), x.data()[i].to_bits());
                        assert_eq!(p[1].to_bits(), y.data()[i].to_bits());
                    }
                    if operation != "patch_create" {
                        assert!((before.series[0].points).same_storage(&after.series[0].points));
                    }
                    if operation.starts_with("textbox") {
                        assert_eq!(after.textboxes.len(), 1);
                    }
                    if operation == "patch_update" {
                        assert_eq!(after.series[0].color, [0, 255, 0]);
                        assert!(!after.series[0].visible);
                    }
                    if operation == "textbox_update" {
                        assert_eq!(after.textboxes[0].text, "changed literal title");
                        assert_eq!(after.textboxes[0].font_size, 18.);
                    }
                    if iteration >= 2 {
                        println!(
                            "{workers},{faces},{operation},{},{:.6}",
                            iteration - 2,
                            elapsed.as_secs_f64() * 1e6
                        );
                    }
                }
            }
        }
    }
}
