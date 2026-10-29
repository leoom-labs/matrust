//! Release timings (one warm-up run, then the median of repeated runs) for
//! filtering, matrix products, sin and MAT/CSV loading; `--verify` checks the
//! large input files.
use matrust::{
    Matrix, Result,
    dsp::{self, Band},
    files,
};
use std::{hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};

fn measure<T>(
    name: &str,
    size: usize,
    threads: usize,
    repetitions: usize,
    bytes: u64,
    mut run: impl FnMut() -> Result<T>,
) {
    let result = (|| -> Result<Vec<f64>> {
        drop(black_box(run()?));
        let mut times = Vec::with_capacity(repetitions);
        for _ in 0..repetitions {
            let start = Instant::now();
            let value = black_box(run()?);
            times.push(start.elapsed().as_secs_f64() * 1000.);
            drop(value);
        }
        times.sort_by(f64::total_cmp);
        Ok(times)
    })();
    match result {
        Ok(t) => {
            let median = (t[(t.len() - 1) / 2] + t[t.len() / 2]) / 2.;
            println!(
                "{name},{size},{threads},{repetitions},{median:.6},{:.6},{:.6},{bytes},ok",
                t[0],
                t[t.len() - 1]
            );
        }
        Err(e) => println!(
            "{name},{size},{threads},0,0,0,0,{bytes},{}",
            e.replace([',', '\n', '\r'], ";")
        ),
    }
}
fn main() {
    if std::env::args().any(|arg| arg == "--verify") {
        verify_large_files();
        return;
    }
    println!("operation,size,threads,repetitions,median_ms,min_ms,max_ms,file_bytes,status");
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let cancel = AtomicBool::new(false);
            let x = Matrix::from_column_vec(
                100000,
                8,
                (1..=800000).map(|i| (i as f64 * 0.013).sin()).collect(),
            )
            .unwrap();
            let b = dsp::fir1(64, &[0.2], Band::Lowpass).unwrap();
            let zi = Matrix::filled(64, 8, 0.1).unwrap();
            measure("fir64", 100000, threads, 20, 0, || {
                dsp::filter(&b, &[1.], &x, &cancel)
            });
            measure("fir64_state", 100000, threads, 20, 0, || {
                dsp::filter_with_state(&b, &[1.], &x, Some(&zi), None, &cancel)
            });
            let xr = x.transpose();
            measure("fir64_rows", 100000, threads, 20, 0, || {
                dsp::filter_with_state(&b, &[1.], &xr, Some(&zi), Some(2), &cancel)
            });
            for n in [256, 512, 1024] {
                let a = Matrix::from_column_vec(
                    n,
                    n,
                    (0..n * n).map(|i| (i % 97) as f64 / 97.).collect(),
                )
                .unwrap();
                measure("gemm", n, threads, 10, 0, || a.mul(&a));
            }
            let v = Matrix::from_column_vec(
                1,
                1000000,
                (1..=1000000).map(|i| (i as f64 * 0.001).sin()).collect(),
            )
            .unwrap();
            measure("sin", 1000000, threads, 20, 0, || Ok(v.sin()));
        });
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    pool.install(|| {
        for mib in [16, 64, 128, 256] {
            for format in ["v6", "v7"] {
                let path = format!("target/evidence/performance/data/input_{mib}m_{format}.mat");
                let path = Path::new(&path);
                let bytes = path.metadata().unwrap().len();
                measure(&format!("load_{format}"), mib, 8, 5, bytes, || {
                    files::load_workspace(path)
                });
            }
            if mib <= 64 {
                let path = format!("target/evidence/performance/data/input_{mib}m.csv");
                let path = Path::new(&path);
                measure(
                    "load_csv",
                    mib,
                    8,
                    3,
                    path.metadata().unwrap().len(),
                    || files::read_csv(path),
                );
            }
        }
    });
}

fn verify_large_files() {
    for mib in [16, 64, 128] {
        let path = format!("target/evidence/performance/data/input_{mib}m_v6.mat");
        let v6 = files::load_workspace(Path::new(&path)).unwrap();
        let path = format!("target/evidence/performance/data/input_{mib}m_v7.mat");
        let v7 = files::load_workspace(Path::new(&path)).unwrap();
        assert_eq!(
            (v6["data"].rows(), v6["data"].cols()),
            (mib * 1024 * 1024 / 64, 8)
        );
        assert_eq!(v6["data"].data(), v7["data"].data());
        if mib <= 64 {
            let path = format!("target/evidence/performance/data/input_{mib}m.csv");
            let csv = files::read_csv(Path::new(&path)).unwrap();
            assert_eq!(v6["data"].data(), csv.data());
        }
        println!(
            "{mib} MiB: all {} values match exactly across formats",
            v6["data"].len()
        );
        if mib == 128 {
            let mut vars = v6;
            vars.insert("second".into(), vars["data"].clone());
            let path = Path::new("target/evidence/performance/data/roundtrip_256m.mat");
            std::fs::create_dir_all("target/evidence/performance/data").unwrap();
            files::save_workspace(path, &vars).unwrap();
            let loaded = files::load_workspace(path).unwrap();
            assert_eq!(loaded["data"].data(), vars["data"].data());
            assert_eq!(loaded["second"].data(), vars["second"].data());
            println!("256 MiB workspace (two 128 MiB matrices): save/reload matches exactly");
        }
    }
    for format in ["v6", "v7"] {
        let path = format!("target/evidence/performance/data/input_256m_{format}.mat");
        assert!(
            files::load_workspace(Path::new(&path))
                .unwrap_err()
                .contains("element limit")
        );
    }
    println!("256 MiB single matrices: rejected at the documented matrix bound");
}
