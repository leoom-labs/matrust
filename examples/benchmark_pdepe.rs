//! Reproduce: cargo run --release --example benchmark_pdepe
//! Explicit caller pools; the bounded PDE and dense NDF kernels stay serial.
use matrust::Engine;
use std::{hint::black_box, time::Instant};

fn measure(workers: usize, nx: usize, name: &str, setup: &str, source: &str) {
    let mut e = Engine::default();
    e.execute(setup).unwrap();
    e.execute(source).unwrap();
    let mut times = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        black_box(e.execute(black_box(source)).unwrap());
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{name},{workers},{nx},3,{:.6},{:.6},{:.6}",
        times[1], times[0], times[2]
    );
}

fn main() {
    println!("case,workers,mesh_points,repetitions,median_ms,min_ms,max_ms");
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
            for nx in [21, 81, 161] {
                for (name, m) in [("slab_heat", 0), ("cylindrical_heat", 1), ("spherical_heat", 2)] {
                    measure(workers, nx, name,
                        &format!("p=@(x,t,u,d) deal(1,d,0); b=@(xl,ul,xr,ur,t) deal(0,1,ur-1-xr^2-2*({m}+1)*t,0); x=linspace(0,1,{nx});"),
                        &format!("sol=pdepe({m},p,@(x) 1+x^2,b,x,[0 .1 .5],odeset('RelTol',1e-6,'AbsTol',1e-8));"));
                }
            }
            for nx in [31, 81] {
                measure(workers, nx, "parabolic_elliptic",
                    &format!("p=@(x,t,u,d) deal([1;0],d,[0;pi^2*u(1)]); b=@(xl,ul,xr,ur,t) deal(ul,[0;0],ur,[0;0]); x=linspace(0,1,{nx});"),
                    "sol=pdepe(0,p,@(x) [sin(pi*x);0],b,x,[0 .02 .1],odeset('RelTol',1e-6,'AbsTol',1e-8));");
            }
            measure(workers, 81, "pdeval_10001_queries",
                "x=linspace(0,1,81); ui=sin(pi*x); q=linspace(0,1,10001);",
                "[u,d]=pdeval(0,x,ui,q);");
            for (name, m, ic) in [("cylindrical_annulus", 1, "log(x)"), ("spherical_annulus", 2, "1./x")] {
                measure(workers, 81, name,
                    &format!("p=@(x,t,u,d) deal(1,d,0); ic=@(x) {ic}; b=@(xl,ul,xr,ur,t) deal(ul-ic(xl),0,ur-ic(xr),0); x=linspace(1,2,81);"),
                    &format!("sol=pdepe({m},p,ic,b,x,[0 .1 .5]);"));
                measure(workers, 81, &format!("pdeval_{name}_10001_queries"),
                    &format!("x=linspace(1,2,81); ui={ic}; q=linspace(1,2,10001);"),
                    &format!("[u,d]=pdeval({m},x,ui,q);"));
            }
        });
    }
}
