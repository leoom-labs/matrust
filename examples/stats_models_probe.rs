//! Release timings and independent fixture error summary.
use matrust::{Array, Engine};
use std::{collections::BTreeMap, time::Instant};
fn main() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/stats_models/reference.json"
    ))
    .unwrap();
    let mut errors: BTreeMap<String, f64> = BTreeMap::new();
    for c in fixture["cases"].as_array().unwrap() {
        let mut e = Engine::default();
        e.execute(c["code"].as_str().unwrap()).unwrap();
        let w = e.workspace();
        let Array::Numeric(a) = w.get("zz").unwrap() else {
            panic!("numeric output")
        };
        let name = c["name"]
            .as_str()
            .unwrap()
            .split('_')
            .next()
            .unwrap()
            .to_string();
        let max = a
            .data()
            .iter()
            .zip(c["data"].as_array().unwrap())
            .map(|(a, b)| (a - b.as_f64().unwrap()).abs())
            .fold(0., f64::max);
        errors
            .entry(name)
            .and_modify(|v| *v = v.max(max))
            .or_insert(max);
    }
    println!("Max absolute errors vs independent reference: {errors:?}");
    let cases = [
        (
            "linkage condensed m=100 ward Euclidean check",
            "rng(3);X=randn(100,4);v=pdist(X);",
            "z=linkage(v,'ward');",
            3,
        ),
        (
            "threshold mx=2048 my=8 p=64 (131072 work)",
            "rng(3);X=randn(2048,64);Y=randn(8,64);",
            "d=pdist2(X,Y);",
            5,
        ),
        (
            "threshold mx=4096 my=8 p=64 (262144 work)",
            "rng(3);X=randn(4096,64);Y=randn(8,64);",
            "d=pdist2(X,Y);",
            5,
        ),
        (
            "threshold mx=8192 my=8 p=64 (524288 work)",
            "rng(3);X=randn(8192,64);Y=randn(8,64);",
            "d=pdist2(X,Y);",
            5,
        ),
        (
            "regress n=5000 p=12",
            "rng(3);X=[ones(5000,1),randn(5000,11)];y=randn(5000,1);",
            "[b,bi,r,ri,s]=regress(y,X);",
            5,
        ),
        (
            "ridge n=5000 p=12 L=3",
            "rng(3);X=randn(5000,12);y=randn(5000,1);",
            "b=ridge(y,X,[0 1 10]);",
            5,
        ),
        (
            "lasso n=2000 p=12 L=10",
            "rng(3);X=randn(2000,12);y=randn(2000,1);",
            "[b,f]=lasso(X,y,'NumLambda',10);",
            5,
        ),
        (
            "pca n=4000 p=12",
            "rng(3);X=randn(4000,12);",
            "[c,s,l]=pca(X);",
            5,
        ),
        (
            "pdist2 mx=5000 my=64 p=64",
            "rng(3);X=randn(5000,64);Y=randn(64,64);",
            "d=pdist2(X,Y);",
            5,
        ),
        (
            "pdist m=1000 p=12",
            "rng(3);X=randn(1000,12);",
            "d=pdist(X);",
            5,
        ),
        (
            "kmeans n=2000 p=8 k=4 fixed",
            "rng(3);X=randn(2000,8);S=X(1:4,:);",
            "[i,c]=kmeans(X,4,'Start',S);",
            3,
        ),
        (
            "kmedoids n=100 p=4 k=3 BUILD",
            "rng(3);X=randn(100,4);",
            "[i,c]=kmedoids(X,3);",
            3,
        ),
        (
            "linkage m=300 p=4 average",
            "rng(3);X=randn(300,4);",
            "z=linkage(X,'average');",
            3,
        ),
        (
            "knn mx=5000 my=64 p=64 K=4",
            "rng(3);X=randn(5000,64);Y=randn(64,64);",
            "[i,d]=knnsearch(X,Y,'K',4);",
            5,
        ),
        (
            "tree/square/pcacov/confusion",
            "rng(3);X=randn(100,4);Z=linkage(X);V=cov(X);v=pdist(X);g=randi(4,100,1);",
            "c=pcacov(V);s=squareform(v);r=inconsistent(Z);t=cluster(Z,'MaxClust',4);C=confusionmat(g,g);",
            5,
        ),
    ];
    // Reverse the order for a second pass to expose warmup/contention effects.
    for workers in [1, 4, 4, 1] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for (name, setup, code, reps) in cases {
                let mut e = Engine::default();
                e.execute(setup).unwrap();
                e.execute(code).unwrap();
                let start = Instant::now();
                for _ in 0..reps {
                    e.execute(code).unwrap();
                }
                println!(
                    "workers={workers} {name}; repetitions={reps}; mean_ms={:.4}",
                    start.elapsed().as_secs_f64() * 1000. / reps as f64
                );
            }
        });
    }
}
