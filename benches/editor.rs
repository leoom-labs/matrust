use matrust::editor;
use std::{hint::black_box, time::Instant};
fn main() {
    println!("operation,bytes,repetitions,ms_per_iteration");
    let line = "% Filter a sampled signal\ny=filter(b,1,x); plot(t,y,'b-');\n";
    for size in [10_000, 100_000, 1_000_000] {
        let mut text = line.repeat(size / line.len());
        text.push_str(&" ".repeat(size - text.len()));
        for operation in ["highlight", "find_absent", "replace_all"] {
            let run = || match operation {
                "highlight" => {
                    black_box(editor::highlights(black_box(&text)));
                }
                "find_absent" => {
                    black_box(editor::find(black_box(&text), "absent", 0, false));
                }
                _ => {
                    black_box(editor::replace_all(black_box(&text), "filter", "filt").unwrap());
                }
            };
            run();
            let start = Instant::now();
            for _ in 0..50 {
                run();
            }
            println!(
                "{operation},{size},50,{:.4}",
                start.elapsed().as_secs_f64() * 20.
            );
        }
    }
}
