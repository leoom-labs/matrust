use matrust::{
    Engine,
    engine::random::{mrg32k3a_step, philox4x32_10},
};

fn is_true(engine: &mut Engine, code: &str) -> bool {
    engine.execute(code).unwrap();
    engine.logical_variables["ans"].data() == [1]
}

#[test]
fn rng_state_save_restore_and_previous_settings() {
    let mut e = Engine::default();

    // Default state: s = rng
    e.execute("s = rng;").unwrap();
    assert!(is_true(&mut e, "strcmp(s.Type, 'twister')"));
    assert!(is_true(
        &mut e,
        "isa(s.Seed, 'uint32') && isscalar(s.Seed) && s.Seed == 0"
    ));
    assert!(is_true(
        &mut e,
        "isa(s.State, 'uint32') && isequal(size(s.State), [625 1])"
    ));

    // Round-trip rand, randn, randi, randperm from default
    assert!(is_true(
        &mut e,
        "s = rng; a = rand(1, 3); rng(s); b = rand(1, 3); isequal(a, b)"
    ));
    assert!(is_true(
        &mut e,
        "s = rng; a = randn(1, 4); rng(s); b = randn(1, 4); isequal(a, b)"
    ));
    assert!(is_true(
        &mut e,
        "s = rng; a = randi(100, 1, 5); rng(s); b = randi(100, 1, 5); isequal(a, b)"
    ));
    assert!(is_true(
        &mut e,
        "s = rng; a = randperm(12); rng(s); b = randperm(12); isequal(a, b)"
    ));

    // Across rng(42) and rng('default')
    assert!(is_true(
        &mut e,
        "rng(42); s = rng; a = rand(1, 3); rng('default'); b = rand(1, 3); rng(s); c = rand(1, 3); isequal(a, c) && ~isequal(a, b)"
    ));
    assert!(is_true(
        &mut e,
        "rng('default'); s = rng; a = randn(1, 3); rng(s); b = randn(1, 3); isequal(a, b)"
    ));
    assert!(is_true(
        &mut e,
        "rng(42); s = rng; a = randi([10 20], 2, 3); rng(s); b = randi([10 20], 2, 3); isequal(a, b)"
    ));

    // s = rng(...) returns PREVIOUS settings
    assert!(is_true(
        &mut e,
        "rng(17); s1 = rng; s2 = rng(99); s3 = rng; isequal(s1.Seed, uint32(17)) && isequal(s2, s1) && isequal(s3.Seed, uint32(99))"
    ));
    assert!(is_true(
        &mut e,
        "rng(123, 'philox'); s_prev = rng('default'); s_now = rng; strcmp(s_prev.Type, 'philox') && s_prev.Seed == 123 && strcmp(s_now.Type, 'twister') && s_now.Seed == 0"
    ));
    assert!(is_true(
        &mut e,
        "rng('shuffle'); s = rng; isa(s.Seed, 'uint32') && isscalar(s.Seed)"
    ));
}

#[test]
fn generators_round_trip_and_unknown_name_errors() {
    let mut e = Engine::default();

    for generator in ["twister", "philox", "combRecursive"] {
        let script = format!(
            "rng(7, '{generator}'); discard = rand(1, 5); s = rng; \
             a1 = rand(1, 4); a2 = randn(1, 4); a3 = randi(50, 1, 4); a4 = randperm(10); \
             rng('default'); rng(s); \
             b1 = rand(1, 4); b2 = randn(1, 4); b3 = randi(50, 1, 4); b4 = randperm(10); \
             strcmp(s.Type, '{generator}') && s.Seed == 7 && isequal(a1, b1) && isequal(a2, b2) && isequal(a3, b3) && isequal(a4, b4)"
        );
        assert!(
            is_true(&mut e, &script),
            "round-trip failed for {generator}"
        );

        // rng(gen) seeds with 0
        let gen_only =
            format!("rng('{generator}'); s = rng; strcmp(s.Type, '{generator}') && s.Seed == 0");
        assert!(is_true(&mut e, &gen_only), "rng('{generator}') failed");
    }

    for call in ["rng(0, 'futureGenerator');", "rng('futureGenerator');"] {
        let err = e.execute(call).unwrap_err();
        assert!(err.contains("unrecognized generator"), "{call}: {err}");
    }
}

#[test]
fn invalid_rng_structs_return_expected_error() {
    let mut e = Engine::default();
    for bad in [
        "rng(struct('Type', 'twister'))",
        "rng(struct('Type', 'twister', 'Seed', uint32(0), 'State', uint32(1:10)'))",
        "s = rng; s.State(625) = uint32(999); rng(s)",
        "s = rng; s.State = zeros(625, 1, 'uint32'); s.State(625) = uint32(624); rng(s)",
        "s = rng; s.Type = 'unknown'; rng(s)",
        "rng(struct('Type', 'combRecursive', 'Seed', uint32(0), 'State', zeros(6, 1, 'uint32')))",
    ] {
        let err = e.execute(bad).unwrap_err();
        assert!(
            err.contains("valid generator settings structure"),
            "{bad}: {err}"
        );
    }
}

#[test]
fn philox_and_mrg32k3a_kat_vectors() {
    // Random123 Philox4x32-10 KAT vectors
    assert_eq!(
        philox4x32_10([0, 0, 0, 0], [0, 0]),
        [0x6627_e8d5, 0xe169_c58d, 0xbc57_ac4c, 0x9b00_dbd8]
    );
    assert_eq!(
        philox4x32_10([0xffff_ffff; 4], [0xffff_ffff; 2]),
        [0x408f_276d, 0x41c8_3b0e, 0xa20b_c7c6, 0x6d54_51fd]
    );
    assert_eq!(
        philox4x32_10(
            [0x243f_6a88, 0x85a3_08d3, 0x1319_8a2e, 0x0370_7344],
            [0xa409_3822, 0x299f_31d0]
        ),
        [0xd16c_fe09, 0x94fd_cceb, 0x5001_e420, 0x2412_6ea1]
    );

    // L'Ecuyer MRG32k3a raw outputs from seed state [12345; 3], [12345; 3]
    let mut s1 = [12345, 12345, 12345];
    let mut s2 = [12345, 12345, 12345];
    let raw: Vec<u32> = (0..5).map(|_| mrg32k3a_step(&mut s1, &mut s2)).collect();
    assert_eq!(
        raw,
        [
            545_508_589,
            1_368_065_410,
            1_327_943_761,
            3_546_985_096,
            951_893_194
        ]
    );
}
