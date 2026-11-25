//! h5read, h5create, h5write and ncread. Fixtures: tests/fixtures/sci/,
//! written by tool/refs/sci/make_fixtures.py with h5py and netCDF4; the
//! expected values follow from the arrays in that script (a C-order shape
//! (a, b) reads as the MATLAB size [b a] with the same buffer).
use matrust::{Array, Engine};
use std::fs;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sci");

fn engine() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    for entry in fs::read_dir(FIXTURES).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
    }
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    (e, dir)
}

fn run(e: &mut Engine, source: &str) {
    if let Err(error) = e.execute(source) {
        panic!("{source}: {error}");
    }
}

fn fails(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(x) => panic!("{source} should fail, printed {:?}", x.output),
        Err(error) => error.to_string(),
    }
}

#[track_caller]
fn check(e: &mut Engine, expr: &str) {
    run(e, &format!("t__ = {expr};"));
    let ok = match e.array("t__") {
        Some(Array::Logical(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0),
        Some(Array::Numeric(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0.),
        other => panic!("{expr}: not logical: {other:?}"),
    };
    assert!(ok, "{expr} is false");
}

#[test]
fn h5read_types_shapes_and_selections() {
    let (mut e, _dir) = engine();
    check(
        &mut e,
        "isequal(h5read('data.h5', '/m'), reshape(0:11, 4, 3))",
    );
    check(
        &mut e,
        "isequal(h5read('data.h5', 'm'), reshape(0:11, 4, 3))",
    );
    check(
        &mut e,
        "isequal(h5read('data.h5', '/g/i32'), int32([1; -2; 3]))",
    );
    check(&mut e, "isequal(h5read('data.h5', '/f32'), single(1.5))");
    check(
        &mut e,
        "isequal(h5read('data.h5', '/u8'), uint8([1 3; 2 255]))",
    );
    check(
        &mut e,
        "isequal(h5read('data.h5', '/b'), [true; false; true])",
    );
    check(
        &mut e,
        "isequal(h5read('data.h5', '/s'), {'ab'; char([99 233])})",
    );
    check(&mut e, "isequal(h5read('data.h5', '/fs'), {'xy'; 'z'})");
    check(
        &mut e,
        "isequal(h5read('data.h5', '/cube'), reshape(0:23, 4, 3, 2))",
    );
    // Deflate + shuffle chunks decode.
    check(&mut e, "isequal(h5read('data.h5', '/z'), (0:99)')");
    // int64 values beyond 2^53 stay exact.
    check(
        &mut e,
        "isequal(h5read('data.h5', '/i64'), [int64(2)^62 + 1; -int64(2)^62])",
    );
    // Malformed values: a boolean enum byte of 2 is true; invalid UTF-8 is replaced.
    check(
        &mut e,
        "isequal(h5read('data.h5', '/badbool'), [false; true; true])",
    );
    run(&mut e, "bt = h5read('data.h5', '/badtext');");
    check(&mut e, "isequal(double(bt{1}), [97 65533 98])");
    // start/count/stride are one-based, in MATLAB order; Inf reads to the end.
    check(
        &mut e,
        "isequal(h5read('data.h5', '/m', [2 1], [2 Inf]), [1 5 9; 2 6 10])",
    );
    check(
        &mut e,
        "isequal(h5read('data.h5', '/m', [1 1], [2 2], [3 2]), [0 8; 3 11])",
    );
    check(
        &mut e,
        "isequal(h5read('data.h5', '/z', 91, 5, 2), [90; 92; 94; 96; 98])",
    );
    check(
        &mut e,
        "isequal(h5read('data.h5', '/cube', [1 2 2], [Inf 1 1]), [16; 17; 18; 19])",
    );
}

#[test]
fn h5create_and_h5write_round_trip() {
    let (mut e, dir) = engine();
    run(
        &mut e,
        "h5create('w.h5', '/g/x', [3 4]); M = magic(4); h5write('w.h5', '/g/x', M(1:3, :));",
    );
    check(&mut e, "isequal(h5read('w.h5', '/g/x'), M(1:3, :))");
    run(
        &mut e,
        "h5create('w.h5', '/y', [10 1], 'Datatype', 'int16', 'ChunkSize', [5 1], 'Deflate', 6, 'FillValue', -1);",
    );
    // Rounding half away from zero and saturation, as int16() does.
    run(
        &mut e,
        "h5write('w.h5', '/y', [7.5; -40000], [3 1], [2 1]);",
    );
    check(
        &mut e,
        "isequal(h5read('w.h5', '/y'), int16([-1; -1; 8; -32768; -1; -1; -1; -1; -1; -1]))",
    );
    run(
        &mut e,
        "h5create('w.h5', '/v', 6, 'Datatype', 'single'); h5write('w.h5', '/v', 1:6);",
    );
    check(&mut e, "isequal(h5read('w.h5', '/v'), single((1:6)'))");
    run(&mut e, "h5write('w.h5', '/v', [10 20], 2, 2, 3);");
    check(
        &mut e,
        "isequal(h5read('w.h5', '/v'), single([1; 10; 3; 4; 20; 6]))",
    );
    run(
        &mut e,
        "h5create('w.h5', '/c', [2 3 4]); C = reshape(1:24, 2, 3, 4); h5write('w.h5', '/c', C);",
    );
    check(&mut e, "isequal(h5read('w.h5', '/c'), C)");
    assert!(dir.path().join("w.h5").is_file());
}

#[test]
fn h5_errors_are_clear() {
    let (mut e, _dir) = engine();
    assert!(fails(&mut e, "h5read('data.h5', '/nope')").contains("no dataset '/nope'"));
    assert!(fails(&mut e, "h5read('data.h5', '/g')").contains("not a dataset"));
    assert!(fails(&mut e, "h5read('missing.h5', '/m')").contains("unable to find"));
    assert!(fails(&mut e, "h5read('data.h5', '/m', [1 1])").contains("start and count"));
    assert!(fails(&mut e, "h5read('data.h5', '/m', [4 1], [2 1])").contains("reach past"));
    assert!(fails(&mut e, "h5read('data.h5', '/m', [1 1 1], [1 1 1])").contains("2 element"));
    assert!(fails(&mut e, "h5read('data.h5', '/s', 1, 1)").contains("text dataset"));
    run(&mut e, "h5create('n.h5', '/a', [2 2]);");
    assert!(fails(&mut e, "h5create('n.h5', '/a', [2 2])").contains("already exists"));
    assert!(fails(&mut e, "h5create('n.h5', '/b', [2 Inf])").contains("unlimited"));
    assert!(
        fails(
            &mut e,
            "h5create('n.h5', '/b', [2 2], 'Datatype', 'string')"
        )
        .contains("Datatype")
    );
    assert!(fails(&mut e, "h5create('n.h5', '/b', [2 2], 'Deflate', 5)").contains("ChunkSize"));
    assert!(fails(&mut e, "h5write('n.h5', '/a', [1 2 3])").contains("the data is 1x3"));
    assert!(fails(&mut e, "h5write('n.h5', '/a', 'text')").contains("numeric"));
    assert!(
        fails(&mut e, "h5write('n.h5', '/a', [1 2], [1 1], [2 1])").contains("count must match")
    );
}

#[test]
fn ncread_classic_and_netcdf4_files() {
    let (mut e, _dir) = engine();
    for file in ["classic.nc", "offset64.nc", "data64.nc", "nc4.nc"] {
        run(&mut e, &format!("f = '{file}';"));
        // temp(time, lat, lon) float with _FillValue -999 at time 2, lat 1, lon 3.
        run(&mut e, "t = ncread(f, 'temp');");
        check(
            &mut e,
            "strcmp(class(t), 'single') && isequal(size(t), [4 2 3])",
        );
        check(&mut e, "isnan(t(3, 1, 2)) && sum(isnan(t(:))) == 1");
        check(
            &mut e,
            "t(4, 2, 3) == 23 && t(1, 1, 1) == 0 && t(1, 2, 1) == 4",
        );
        // A record variable next to another record variable.
        check(&mut e, "isequal(ncread(f, 'time'), [0.5; 1.5; 2.5])");
        // scale_factor 0.5 and add_offset 10 unpack to double.
        check(
            &mut e,
            "isequal(ncread(f, 'packed'), [0 4; 1 5; 2 6; 3 7] * 0.5 + 10)",
        );
        // NC_CHAR(lat, strlen) is a 3x2 character array.
        run(&mut e, "l = ncread(f, 'label');");
        check(
            &mut e,
            "ischar(l) && isequal(size(l), [3 2]) && isequal(l(:, 2)', 'xyz')",
        );
        check(&mut e, "isequal(double(l(:, 1))', [97 98 0])");
        check(&mut e, "isequal(ncread(f, 'counts'), int32([7; 8; 9; 10]))");
        check(&mut e, "isequal(ncread(f, 'counts', 2, 2), int32([8; 9]))");
        check(
            &mut e,
            "isequal(ncread(f, 'temp', [2 1 3], [2 1 Inf]), single([17; 18]))",
        );
        assert!(fails(&mut e, "ncread(f, 'nothing')").contains("no variable 'nothing'"));
    }
    // CDF-5 64-bit integers.
    check(
        &mut e,
        "isequal(ncread('data64.nc', 'big'), int64([2^40; -5]))",
    );
}

#[test]
fn ncread_rejects_other_files() {
    let (mut e, dir) = engine();
    fs::write(dir.path().join("x.nc"), b"plain text").unwrap();
    assert!(fails(&mut e, "ncread('x.nc', 'v')").contains("not a NetCDF file"));
    // A truncated classic file fails cleanly.
    let bytes = fs::read(dir.path().join("classic.nc")).unwrap();
    fs::write(dir.path().join("cut.nc"), &bytes[..bytes.len() - 20]).unwrap();
    assert!(fails(&mut e, "ncread('cut.nc', 'temp')").contains("truncated"));
    fs::write(dir.path().join("head.nc"), &bytes[..30]).unwrap();
    assert!(fails(&mut e, "ncread('head.nc', 'temp')").contains("ncread:"));
}
