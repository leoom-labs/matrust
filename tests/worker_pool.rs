//! One test in its own process so the initial global-pool state is meaningful.
use matrust::{Engine, Matrix, array::evaluate_polynomial, initialize_worker_pool};
use std::sync::atomic::AtomicBool;

#[test]
fn explicit_global_setup_preserves_off_pool_and_local_pool_execution() {
    let p = Matrix::from_column_slice(1, 4, &[2., -3., 0., 5.]).unwrap();
    let n = 262_144;
    let x = Matrix::from_column_vec(2, n / 2, vec![1.; n]).unwrap();
    let inputs = (p.clone(), x.clone());
    let cancel = AtomicBool::new(false);

    // An eligible numerical call outside a pool must not initialize Rayon's
    // implicit global pool. Successful explicit setup below proves this.
    assert!(rayon::current_thread_index().is_none());
    let before = evaluate_polynomial(&p, &x, &cancel).unwrap();
    assert!(before.data().iter().all(|v| v.to_bits() == 4_f64.to_bits()));
    assert!(initialize_worker_pool(0).is_err());
    assert!(initialize_worker_pool(65).is_err());
    initialize_worker_pool(8).unwrap();

    // CLI/desktop engine calls originate on ordinary threads after global setup.
    assert!(rayon::current_thread_index().is_none());
    let mut engine = Engine::default();
    engine.variables.insert("p".into(), p.clone());
    engine.variables.insert("x".into(), x.clone());
    engine.execute("y=polyval(p,x);").unwrap();
    assert_eq!(
        (engine.variables["y"].rows(), engine.variables["y"].cols()),
        (2, n / 2)
    );
    assert_eq!(engine.variables["y"].data(), before.data());

    // A caller's one-worker pool overrides the configured eight-worker global
    // pool. It stays usable; setup failures do not replace the registered pool.
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let local = one
        .install(|| evaluate_polynomial(&p, &x, &cancel))
        .unwrap();
    assert_eq!(local.data(), before.data());
    assert!(initialize_worker_pool(1).is_err());
    assert_eq!(rayon::current_num_threads(), 8);
    assert_eq!(
        evaluate_polynomial(&p, &x, &cancel).unwrap().data(),
        before.data()
    );
    assert_eq!(p.data().as_ptr(), inputs.0.data().as_ptr());
    assert_eq!(x.data().as_ptr(), inputs.1.data().as_ptr());
}
