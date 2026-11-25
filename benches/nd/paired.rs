//! Persistent benchmark endpoint: the runner alternates adjacent HEAD/current
//! samples while both use the same fixed worker/core placement.
use matrust::Engine;
use std::{
    hint::black_box,
    io::{self, BufRead, Write},
    time::Instant,
};

#[cfg(target_os = "linux")]
fn cpu_seconds() -> f64 {
    #[repr(C)]
    struct Time {
        seconds: i64,
        nanos: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, value: *mut Time) -> i32;
    }
    let mut value = Time {
        seconds: 0,
        nanos: 0,
    };
    // SAFETY: CLOCK_PROCESS_CPUTIME_ID=2; value points to a writable timespec on Linux x64.
    assert_eq!(unsafe { clock_gettime(2, &mut value) }, 0);
    value.seconds as f64 + value.nanos as f64 * 1e-9
}
#[cfg(not(target_os = "linux"))]
fn cpu_seconds() -> f64 {
    0.
}

pub fn serve(threads: usize) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .start_handler(crate::affinity::bind_worker)
        .build()
        .unwrap();
    pool.broadcast(|context| {
        if context.index() != 0 {
            return;
        }
        let mut engine = Engine::default();
        engine
            .execute("A=reshape(1:1000000,1000,1000);B=ones(1000,1000);")
            .unwrap();
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "ready").unwrap();
        stdout.flush().unwrap();
        for line in stdin.lock().lines() {
            let line = line.unwrap();
            let op = line.parse::<usize>().unwrap();
            let (statement, batch) = match op {
                0 => ("C=A+B;", 1),
                1 => ("[m,i]=max(A+B);", 1),
                2 => ("C=sum(A);", 1),
                3 => ("C=A(:,500);", 1000),
                _ => panic!("invalid benchmark operation"),
            };
            let cpu = cpu_seconds();
            let start = Instant::now();
            for _ in 0..batch {
                black_box(engine.execute(black_box(statement)).unwrap());
            }
            let wall = start.elapsed().as_secs_f64() * 1000. / batch as f64;
            let cpu = (cpu_seconds() - cpu) * 1000. / batch as f64;
            writeln!(stdout, "{wall:.9},{cpu:.9}").unwrap();
            stdout.flush().unwrap();
        }
    });
}
