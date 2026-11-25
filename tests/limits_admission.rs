//! Memory/element admission before allocation (step review 2026-11-12, items 3-9).
//! Each refused call runs under a small custom `Limits` and is wrapped in `numel`
//! (or is a scalar request), so only a scalar could reach the workspace: an error
//! proves the refusal happened in the preflight, not in workspace accounting.
use matrust::{Engine, engine::Limits};

const MIB: usize = 1024 * 1024;

/// Browser limits with a 1 MiB memory budget (131072 8-byte units) and no
/// evaluation budget, so only the memory budget can refuse.
fn small() -> Engine {
    Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: MIB,
        ..Limits::browser()
    })
}
fn ok(e: &mut Engine, source: &str) {
    if let Err(err) = e.execute(source) {
        panic!("{source}\n{err}");
    }
}
fn refused(e: &mut Engine, source: &str, needle: &str) {
    match e.execute(source) {
        Ok(_) => panic!("{source}: expected a refusal"),
        Err(err) => {
            let err = err.to_string();
            assert!(err.contains(needle), "{source}: {err}");
            assert!(!err.contains("Workspace"), "{source}: refused late: {err}");
        }
    }
}

#[test]
fn preflight_elements_honours_memory_bytes() {
    // Item 3: a 1024-element temporary (8 KiB) against a 1 KiB budget.
    let mut e = Engine::with_limits(Limits {
        memory_bytes: 1024,
        ..Limits::browser()
    });
    refused(&mut e, "x = sum(zeros(1, 1024));", "memory limit");
    ok(&mut e, "x = sum(zeros(1, 100));");
}

#[test]
fn nd_index_admits_before_positions() {
    // Item 4: 400*400*2 = 320000 positions > 131072 units.
    let mut e = small();
    ok(&mut e, "A = ones(1, 1, 2);");
    refused(
        &mut e,
        "n = numel(A(ones(1, 400), ones(1, 400), :));",
        "memory limit",
    );
    ok(&mut e, "n = numel(A(ones(1, 100), ones(1, 100), :));");
    // Element limit before allocation too (native: 48 Mi elements).
    let mut e = Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: 4096 * MIB,
        ..Limits::browser()
    });
    ok(&mut e, "A = ones(1, 1, 2);");
    refused(
        &mut e,
        "n = numel(A(ones(1, 8192), ones(1, 8192), :));",
        "element limit",
    );
}

#[test]
fn nd_repmat_and_growth_admit_before_allocation() {
    // Item 5 and its siblings (typed/complex repmat, typed/complex growth, N-D max).
    let mut e = small();
    ok(
        &mut e,
        "A = ones(1, 1, 2); U = uint8(A); Z = complex(A, A);",
    );
    refused(
        &mut e,
        "n = numel(repmat(A, [1 1 100000]));",
        "memory limit",
    );
    refused(
        &mut e,
        "n = numel(repmat(U, [1 1 100000]));",
        "memory limit",
    );
    refused(&mut e, "n = numel(repmat(Z, [1 1 50000]));", "memory limit");
    refused(&mut e, "A(1, 1, 200000) = 1;", "memory limit");
    refused(&mut e, "Z(1, 1, 100000) = 1;", "memory limit");
    refused(&mut e, "U(1, 1, 2000000) = 1;", "memory limit");
    refused(
        &mut e,
        "n = numel(max(ones(1, 1, 400), ones(400, 1)));",
        "memory limit",
    );
    ok(
        &mut e,
        "n = numel(repmat(A, [1 1 1000])); A(1, 1, 1000) = 1; U(1, 1, 1000) = 1;",
    );
    assert_eq!(e.variables["n"].data(), &[2000.]);
}

#[test]
fn imresize_and_imrotate_admit_output_and_scratch() {
    // Item 6: 600*600 output plus scratch > 131072 units.
    let mut e = small();
    refused(&mut e, "n = numel(imresize(0, [600 600]));", "memory limit");
    refused(
        &mut e,
        "n = numel(imrotate(zeros(300, 300), 45));",
        "memory limit",
    );
    ok(
        &mut e,
        "n = numel(imresize(0, [100 100])); m = numel(imrotate(zeros(100, 100), 45));",
    );
}

#[test]
fn imoverlay_admits_rgb_output_before_allocation() {
    // Item 7: 5000x4000x3 = 60e6 > 48 Mi elements under a 4 GiB budget.
    let mut e = Engine::with_limits(Limits {
        evaluation_units: None,
        memory_bytes: 4096 * MIB,
        ..Limits::browser()
    });
    ok(
        &mut e,
        "I = zeros(5000, 4000, 'uint8'); M = false(5000, 4000);",
    );
    refused(
        &mut e,
        "n = numel(imoverlay(I, M));",
        "5000-by-4000-by-3 output",
    );
    refused(
        &mut e,
        "n = numel(labeloverlay(I, double(M)));",
        "5000-by-4000-by-3 output",
    );
}

#[test]
fn stretchlim_and_imadjust_admit_scratch() {
    // Item 8: gray needs n units (no clone), RGB 2n, imadjust one more n.
    // uint8 inputs keep the workspace small; the budget is 131072 double units.
    let mut e = small();
    ok(
        &mut e,
        "G = zeros(300, 300, 'uint8'); C = zeros(150, 150, 3, 'uint8');",
    );
    refused(&mut e, "L = stretchlim(C);", "element limit");
    refused(&mut e, "n = numel(imadjust(G));", "element limit");
    ok(
        &mut e,
        "L = stretchlim(G); n = numel(imadjust(G(1:200, :)));",
    );
    ok(
        &mut e,
        "J = imadjust(uint8([0 64; 128 255])); L = stretchlim(uint8(cat(3, [0 9], [1 2], [3 4])));",
    );
}

#[test]
fn cvpartition_checks_rows_before_allocating() {
    // Item 9: 50331648 observations were a 384 MiB group vector before the check.
    let mut e = Engine::default();
    for n in ["50331648", "20001"] {
        let err = e
            .execute(&format!("c = cvpartition({n}, 'KFold', 2);"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("between 1 and 20000"), "{n}: {err}");
    }
    ok(&mut e, "c = cvpartition(20, 'KFold', 2);");
}
