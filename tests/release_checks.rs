use matrust::{Engine, HostIo};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn is_true(engine: &mut Engine, code: &str) -> bool {
    engine.execute(code).unwrap();
    engine.logical_variables["ans"].data() == [1]
}

#[test]
fn matlab_release_checks_and_version() {
    let mut e = Engine::default();

    assert!(is_true(&mut e, "isMATLABReleaseOlderThan(\"R2025a\")"));
    assert!(!is_true(&mut e, "isMATLABReleaseOlderThan(\"R2024b\")"));
    assert!(!is_true(&mut e, "isMATLABReleaseOlderThan(\"R2020a\")"));
    assert!(!is_true(
        &mut e,
        "isMATLABReleaseOlderThan('R2024b', 'prerelease')"
    ));
    assert!(is_true(
        &mut e,
        "isMATLABReleaseOlderThan('R2024b', 'release', 1)"
    ));
    assert!(is_true(
        &mut e,
        "isMATLABReleaseOlderThan('R2025a', 'prerelease')"
    ));

    assert!(!is_true(&mut e, "verLessThan('matlab', '9.0')"));
    assert!(!is_true(&mut e, "verLessThan('matlab', '24.2')"));
    assert!(is_true(&mut e, "verLessThan('matlab', '25.1')"));

    assert!(is_true(&mut e, "strcmp(version('-release'), '2024b')"));
    assert!(is_true(
        &mut e,
        "strcmp(version('-date'), 'September 12, 2024')"
    ));
    // Public version page: no arguments return '<numbers> (R<release>)'.
    // https://www.mathworks.com/help/matlab/ref/version.html
    // ifem/tool/myunique.m:10-14: v=version; ix=regexp(v,'(')+2; iy=regexp(v,')')-2
    assert!(is_true(&mut e, "strcmp(version, '24.2.0.0 (R2024b)')"));
    assert!(is_true(
        &mut e,
        "v=version; ix=regexp(v,'(')+2; iy=regexp(v,')')-2; str2double(v(ix:iy))==2024"
    ));
    assert!(is_true(
        &mut e,
        "v=version; a=regexp(v,'('); b=regexp(v,')'); strcmp(v(a+1:b-1), 'R2024b')"
    ));
    // Same page: '-description' is empty for a general release with no update.
    assert!(is_true(
        &mut e,
        "d = version('-description'); ischar(d) && isempty(d) && isequal(size(d), [0 0])"
    ));
    assert!(is_true(&mut e, "v = ver; strcmp(v.Name, 'MatRust')"));
    assert!(is_true(
        &mut e,
        "v = ver('matlab'); strcmp(v.Name, 'MatRust') && strcmp(v.Version, '24.2') && strcmp(v.Release, '(R2024b)')"
    ));
    assert!(
        e.execute("version('-nope')")
            .unwrap_err()
            .contains("Unknown command option.")
    );
    assert!(
        e.execute("version(1)")
            .unwrap_err()
            .contains("Argument must contain a string.")
    );
    assert!(
        e.execute("version('-release', 1)")
            .unwrap_err()
            .contains("Too many input arguments.")
    );

    for bad in [
        "isMATLABReleaseOlderThan('2024b')",
        "isMATLABReleaseOlderThan('R2024c')",
        "isMATLABReleaseOlderThan('R24b')",
        "isMATLABReleaseOlderThan('R2024b', 'beta')",
        "isMATLABReleaseOlderThan('R2024b', 'release', -1)",
        "isMATLABReleaseOlderThan('R2024b', 'release', 1.5)",
    ] {
        assert!(e.execute(bad).is_err(), "expected error for: {bad}");
    }
}

struct CountingBellHost(Arc<AtomicUsize>);

impl HostIo for CountingBellHost {
    fn read_line(&mut self, _prompt: &str) -> Option<String> {
        None
    }
    fn beep(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn beep_state_and_host_hook() {
    let count = Arc::new(AtomicUsize::new(0));
    let mut e = Engine::default();
    e.set_host_io(Some(Box::new(CountingBellHost(count.clone()))));

    // Default state is 'on'
    assert!(is_true(&mut e, "s = beep; strcmp(s, 'on')"));
    e.execute("beep").unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 1);

    // Turn off via command and function forms
    e.execute("beep off").unwrap();
    assert!(is_true(&mut e, "s = beep; strcmp(s, 'off')"));
    e.execute("beep;").unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 1);

    e.execute("beep('on')").unwrap();
    assert!(is_true(&mut e, "s = beep; strcmp(s, 'on')"));
    e.execute("beep").unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 2);

    assert!(e.execute("beep('maybe')").is_err());

    // CLI / default host (no HostIo) is a no-op
    let mut cli = Engine::default();
    cli.execute("beep").unwrap();
    assert!(is_true(&mut cli, "s = beep; strcmp(s, 'on')"));
}
