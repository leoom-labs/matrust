//! Engine-level checks of per-host run limits.
use super::{Engine, Limits};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[test]
fn limits_setter_replaces_the_active_limits() {
    let mut engine = Engine::default();
    engine.limits_set(Limits::local());
    assert_eq!(engine.limits_active(), Limits::local());
    assert_eq!(engine.limits_active().steps, None);
    engine.execute("for i=1:1100000, end").unwrap();
    engine.limits_set(Limits::browser());
    let error = engine.execute("for i=1:1100000, end").unwrap_err();
    assert!(error.contains("1000000 statement/loop steps"), "{error}");
}

#[test]
fn local_limits_end_an_endless_loop_only_on_cancel() {
    let mut engine = Engine::with_limits(Limits::local());
    let cancel = engine.cancellation();
    let run = std::thread::spawn(move || {
        let error = engine.execute("n=0; while true, n=n+1; end").unwrap_err();
        (error, Instant::now(), engine)
    });
    // Past the browser step limit before cancelling.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!run.is_finished());
    let stopped = Instant::now();
    cancel.store(true, Ordering::Relaxed);
    let (error, ended, engine) = run.join().unwrap();
    assert!(error.ends_with("Calculation cancelled"), "{error}");
    assert!(ended.duration_since(stopped) < Duration::from_secs(1));
    assert!(engine.variables["n"].as_scalar().unwrap() >= 1.);
}

#[test]
fn displayed_value_cap_follows_active_host_limits() {
    let mut engine = Engine::with_limits(Limits::local());
    let output = engine.execute("x = rand(500)").unwrap().output.join("\n");
    assert!(output.len() > 2 * 1024 * 1024);
    assert!(output.len() < 32 * 1024 * 1024);
    let error = engine.execute("x = rand(2000)").unwrap_err();
    assert!(
        error.contains("Displayed value exceeds 32 MiB; use semicolons"),
        "{error}"
    );
    // Like the browser's output cap, a script's try/catch cannot swallow it.
    let error = engine
        .execute("try, x = rand(2000), catch, y = 1; end")
        .unwrap_err();
    assert!(error.contains("Displayed value exceeds 32 MiB"), "{error}");
    engine.limits_set(Limits::browser());
    let error = engine.execute("x = rand(500)").unwrap_err();
    assert!(
        error.contains("Execution output exceeds 2000 entries or 2 MiB"),
        "{error}"
    );
    engine.limits_set(Limits::local());
    engine.execute("x = rand(500)").unwrap();
}
#[test]
fn round14_fast_path_matches_decimal_reference() {
    for n in -22..=22 {
        for i in -10000..=10000 {
            let x = f64::from(i) / 123.0 * 10_f64.powi((n % 9) - n);
            let reference = super::round14_decimal_text(&format!("{:.14e}", x.abs()), n)
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .map_or(x, |v| v.copysign(x));
            assert_eq!(
                super::round14_digits(x, n).to_bits(),
                reference.to_bits(),
                "x={x} n={n}"
            );
        }
    }
}
