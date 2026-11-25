//! Release-only solver smoke benchmark; includes parsing and sequential callbacks.
//! Pool construction and one warmup per case are outside the timed region.
use matrust::Engine;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let cases = [
        (
            "bfgs",
            2,
            1,
            "[x,f,flag] = fminunc(@(x) 100*(x(2)-x(1)^2)^2+(1-x(1))^2,[-1.2 1],optimoptions('fminunc','Display','off'));",
        ),
        (
            "lm_fit",
            3,
            25,
            "t=linspace(0,4,25)'; y=2.5*exp(-1.3*t)+0.5; [x,rn,r,flag]=lsqcurvefit(@(p,t) p(1)*exp(-p(2)*t)+p(3),[1 1 0],t,y,[],[],optimset('Display','off'));",
        ),
        (
            "dogleg",
            2,
            2,
            "[x,r,flag]=fsolve(@(x) [x(1)^2+x(2)^2-5; x(1)*x(2)-2],[0.5;3],optimset('Display','off'));",
        ),
        (
            "rectangular_lm",
            2,
            3,
            "[x,r,flag]=fsolve(@(x) [x(1)-1; x(2)-2; x(1)+x(2)-3],[0;0],optimset('Display','off'));",
        ),
    ];
    const REPS: usize = 20;
    const SAMPLES: usize = 5;
    println!("case,n,m,workers,reps,samples,median_us_per_solve,min_us,max_us");
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                for (name, n, m, script) in cases {
                    let mut engine = Engine::default();
                    engine.execute(script).unwrap();
                    assert!(engine.variables["flag"].data()[0] > 0.);
                    let mut times = [0.; SAMPLES];
                    for sample in &mut times {
                        let start = Instant::now();
                        for _ in 0..REPS {
                            black_box(engine.execute(script).unwrap());
                        }
                        *sample = start.elapsed().as_secs_f64() * 1e6 / REPS as f64;
                    }
                    times.sort_by(f64::total_cmp);
                    println!(
                        "{name},{n},{m},{workers},{REPS},{SAMPLES},{:.3},{:.3},{:.3}",
                        times[SAMPLES / 2],
                        times[0],
                        times[SAMPLES - 1]
                    );
                }
            });
    }
}
