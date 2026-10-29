use matrust::{Engine, files};
use std::path::Path;
fn main() -> Result<(), String> {
    let vars = files::load_workspace(Path::new("tests/fixtures/reference/plot-oracle.mat"))?;
    let mut count = 0;
    for (script, names) in [
        ("plot([1 2 3],[2 4;3 5;4 6]);", vec!["columns1", "columns2"]),
        ("plot([10 20],[1 2;3 4;5 6]);", vec!["", "", "rows3"]),
        ("plot([1 2;3 4;5 6],[10 20 30]);", vec!["", "sharedy2"]),
        ("bar([10 20 30],[1 -2;3 4;5 6]);", vec!["bars1", "bars2"]),
    ] {
        let mut engine = Engine::default();
        engine.execute(script)?;
        let f = engine.figure.unwrap();
        for (series, name) in f.series.iter().zip(names) {
            if name.is_empty() {
                continue;
            }
            let expected = &vars[name];
            if expected.rows() != series.points.len() {
                return Err(format!("{name}: shape differs"));
            }
            for (i, point) in series.points.iter().enumerate() {
                for (c, &v) in point.iter().enumerate() {
                    if v != expected.get(i, c) {
                        return Err(format!("{name}: coordinate differs"));
                    }
                    count += 1;
                }
            }
        }
    }
    let mut engine = Engine::default();
    engine
        .execute("plot([1 2],[2 3],'r--o',[2 4],[5 6],'b:');axis([-1 10 -2 8]);limits=axis();")?;
    for (i, s) in engine.figure.unwrap().series.iter().enumerate() {
        for c in 0..3 {
            if s.color[c] as f64 / 255. != vars["colors"].get(i, c) {
                return Err("Color mismatch".into());
            }
            count += 1;
        }
    }
    assert_eq!(engine.variables["limits"].data(), vars["limits"].data());
    count += 4;
    println!("Plot reference cases: {count} coordinates, colors and limits matched exactly");
    Ok(())
}
