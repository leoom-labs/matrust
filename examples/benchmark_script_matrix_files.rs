//! Release API/script text I/O, same numeric fixtures and interleaved operations.
use matrust::{Array, Engine, Matrix, files};
use std::{io::Read, path::Path, sync::atomic::AtomicBool, time::Instant};

fn file_hash(path: &Path) -> blake3::Hash {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    hash.finalize()
}
fn matrix_hash(matrix: &Matrix) -> blake3::Hash {
    blake3::hash(bytemuck::cast_slice(matrix.data()))
}

fn main() {
    let reverse = std::env::args().any(|arg| arg == "--reverse");
    let mut worker_counts = [1, 8];
    if reverse {
        worker_counts.reverse();
    }
    println!(
        "rows,cols,decoded_bytes,file_bytes,workers,operation,repetitions,median_ms,min_ms,max_ms,samples_ms"
    );
    for workers in worker_counts {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (rows,cols) in [(1024,4),(524288,4),(2097152,1),(4194304,4)] {
                let dir=tempfile::tempdir().unwrap();
                let input=Matrix::from_column_vec(rows,cols,(0..rows*cols).map(|i|(i%1000003) as f64/1024.-500.).collect()).unwrap();
                let expected=matrix_hash(&input);let source:Array=input.clone().into();let cancel=AtomicBool::new(false);
                let fixture=dir.path().join("input.csv");let output=dir.path().join("output.csv");
                files::write_matrix_text(&fixture,&source,',',&cancel).unwrap();
                let expected_file=file_hash(&fixture);let file_bytes=std::fs::metadata(&fixture).unwrap().len();
                let mut engine=Engine::default();engine.set_file_directory(dir.path()).unwrap();engine.set_function_directory(dir.path()).unwrap();
                engine.variables.insert("A".into(),input.clone());
                let mut operations=["legacy_read","strict_read","script_read","legacy_write","stream_write","script_write"];
                if reverse {operations.reverse();}
                let mut samples:Vec<Vec<f64>>=vec![Vec::new();operations.len()];
                for round in 0..9 {
                    for offset in 0..operations.len() {
                        let index=(round+offset)%operations.len();let operation=operations[index];
                        engine.variables.remove("R");
                        let start=Instant::now();
                        let loaded=match operation {
                            "legacy_read"=>Some(files::read_csv(&fixture).unwrap()),
                            "strict_read"=>Some(files::read_matrix_text(&fixture,None,&cancel).unwrap()),
                            "script_read"=>{engine.execute("R=readmatrix('input.csv');").unwrap();None},
                            "legacy_write"=>{files::write_array_csv(&output,&source,&cancel).unwrap();None},
                            "stream_write"=>{files::write_matrix_text(&output,&source,',',&cancel).unwrap();None},
                            "script_write"=>{engine.execute("writematrix(A,'output.csv');").unwrap();None},
                            _=>unreachable!(),
                        };
                        let elapsed=start.elapsed().as_secs_f64()*1000.;
                        if round>=2 {samples[index].push(elapsed);}
                        if operation.ends_with("read") {
                            let result=loaded.as_ref().unwrap_or_else(||&engine.variables["R"]);
                            assert_eq!((result.rows(),result.cols()),(rows,cols));assert_eq!(matrix_hash(result),expected);
                        } else {assert_eq!(file_hash(&output),expected_file);}
                        assert_eq!(engine.variables["A"].data().as_ptr(),input.data().as_ptr());
                        drop(loaded);
                    }
                }
                for (operation,samples) in operations.into_iter().zip(samples) {
                    let mut sorted=samples.clone();sorted.sort_by(f64::total_cmp);
                    println!("{rows},{cols},{},{file_bytes},{workers},{operation},7,{:.6},{:.6},{:.6},\"{:?}\"",input.len()*8,sorted[3],sorted[0],sorted[6],samples);
                }
            }
        });
    }
}
