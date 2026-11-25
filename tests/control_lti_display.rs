use matrust::Engine;

fn run_output(code: &str) -> String {
    let mut e = Engine::default();
    e.execute(code).unwrap().output.join("\n")
}

#[test]
fn tf_continuous_display_matches_reference() {
    // Mirrors MathWorks R2024b `tf` page continuous-time SISO transfer function example.
    let expected_body = concat!(
        "      s + 3\n",
        "  -------------\n",
        "  s^2 + 2 s + 5\n\n",
        "Continuous-time transfer function.",
    );
    assert_eq!(
        run_output("sys = tf([1 3],[1 2 5])"),
        format!("sys =\n\n{expected_body}")
    );
    assert_eq!(
        run_output("sys = tf([1 3],[1 2 5]); disp(sys)"),
        expected_body
    );
    assert_eq!(
        run_output("sys = tf([1 3],[1 2 5]); display(sys)"),
        format!("sys =\n\n{expected_body}")
    );
    assert_eq!(run_output("display(tf([1 3],[1 2 5]))"), expected_body);
}

#[test]
fn tf_discrete_display_matches_reference() {
    // Mirrors MathWorks R2024b `tf` / `c2d` page discrete-time transfer function example (Ts = 0.1).
    let expected_body = concat!(
        "        z - 1\n",
        "  ------------------\n",
        "  z^2 - 1.85 z + 0.9\n \n",
        "Sample time: 0.1 seconds\n",
        "Discrete-time transfer function.",
    );
    assert_eq!(
        run_output("H = tf([1 -1],[1 -1.85 0.9],0.1)"),
        format!("H =\n\n{expected_body}")
    );
    assert_eq!(
        run_output("H = tf([1 -1],[1 -1.85 0.9],0.1); disp(H)"),
        expected_body
    );

    // 4 significant digits on c2d-style coefficients.
    let expected_c2d = concat!(
        "Hd =\n\n",
        "   0.09516\n",
        "  ----------\n",
        "  z - 0.9048\n \n",
        "Sample time: 0.1 seconds\n",
        "Discrete-time transfer function.",
    );
    assert_eq!(run_output("Hd = c2d(tf(1,[1 1]),0.1)"), expected_c2d);
}

#[test]
fn zpk_gain_and_zeros_display_matches_reference() {
    // Mirrors MathWorks R2024b `zpk` page continuous-time factored example with gain and zero.
    let expected_body = concat!(
        "    4 (s+1)\n",
        "  -----------\n",
        "  (s+2) (s+3)\n\n",
        "Continuous-time zero/pole/gain model.",
    );
    assert_eq!(
        run_output("sys = zpk(-1,[-2 -3],4)"),
        format!("sys =\n\n{expected_body}")
    );
    assert_eq!(
        run_output("sys = zpk(-1,[-2 -3],4); disp(sys)"),
        expected_body
    );
}

#[test]
fn zpk_no_zeros_repeated_root_display_matches_reference() {
    // Mirrors MathWorks R2024b `zpk` page example with constant-only numerator and repeated pole.
    let expected_body = concat!(
        "     1\n",
        "  -------\n",
        "  (s+1)^2\n\n",
        "Continuous-time zero/pole/gain model.",
    );
    assert_eq!(
        run_output("sys = zpk([],[-1 -1],1)"),
        format!("sys =\n\n{expected_body}")
    );
    assert_eq!(
        run_output("sys = zpk([],[-1 -1],1); disp(sys)"),
        expected_body
    );
}

#[test]
fn zpk_complex_pair_and_discrete_display_match_reference() {
    // Mirrors MathWorks R2024b `zpk` page complex-conjugate pole pair example.
    let expected_complex = concat!(
        "          -2 s\n",
        "  --------------------\n",
        "  (s-2) (s^2 - 2s + 2)\n\n",
        "Continuous-time zero/pole/gain model.",
    );
    assert_eq!(
        run_output("H = zpk(0,[2 1+1i 1-1i],-2)"),
        format!("H =\n\n{expected_complex}")
    );

    // Mirrors MathWorks R2024b `zpk` page discrete-time example (Ts = 0.1).
    let expected_discrete = concat!(
        "  (z-0.7)\n",
        "  -------\n",
        "  (z-0.5)\n \n",
        "Sample time: 0.1 seconds\n",
        "Discrete-time zero/pole/gain model.",
    );
    assert_eq!(
        run_output("Hd = zpk(0.7,0.5,1,0.1)"),
        format!("Hd =\n\n{expected_discrete}")
    );
}

#[test]
fn ss_two_state_display_matches_reference() {
    // Mirrors MathWorks R2024b `ss` page 2-state continuous-time state-space example.
    let expected_body = concat!(
        "  A = \n",
        "       x1  x2\n",
        "   x1   0   1\n",
        "   x2  -5  -2\n \n",
        "  B = \n",
        "       u1\n",
        "   x1   0\n",
        "   x2   3\n \n",
        "  C = \n",
        "       x1  x2\n",
        "   y1   1   0\n \n",
        "  D = \n",
        "       u1\n",
        "   y1   0\n \n",
        "Continuous-time state-space model.",
    );
    assert_eq!(
        run_output("sys = ss([0 1;-5 -2],[0;3],[1 0],0)"),
        format!("sys =\n\n{expected_body}")
    );
    assert_eq!(
        run_output("sys = ss([0 1;-5 -2],[0;3],[1 0],0); disp(sys)"),
        expected_body
    );
    assert_eq!(
        run_output("sys = ss([0 1;-5 -2],[0;3],[1 0],0); display(sys)"),
        format!("sys =\n\n{expected_body}")
    );
}

#[test]
fn ss_discrete_display_matches_reference() {
    // Mirrors MathWorks R2024b `ss` / `c2d` page discrete-time state-space example (Ts = 0.1).
    let expected_body = concat!(
        "  A = \n",
        "           x1\n",
        "   x1  0.9048\n \n",
        "  B = \n",
        "            u1\n",
        "   x1  0.09516\n \n",
        "  C = \n",
        "       x1\n",
        "   y1   1\n \n",
        "  D = \n",
        "       u1\n",
        "   y1   0\n \n",
        "Sample time: 0.1 seconds\n",
        "Discrete-time state-space model.",
    );
    assert_eq!(
        run_output("sysd = ss(0.9048374,0.09516258,1,0,0.1)"),
        format!("sysd =\n\n{expected_body}")
    );
}

#[test]
fn control_lti_ss_explicit_realization_matches_reference_spacing() {
    // MathWorks R2024b ss explicit realization.
    assert_eq!(
        run_output("disp(ss([2 -4;20 -20],[-1;-5],[-.5 -2],-1))"),
        concat!(
            "  A = \n",
            "        x1   x2\n",
            "   x1    2   -4\n",
            "   x2   20  -20\n \n",
            "  B = \n",
            "       u1\n",
            "   x1  -1\n",
            "   x2  -5\n \n",
            "  C = \n",
            "         x1    x2\n",
            "   y1  -0.5    -2\n \n",
            "  D = \n",
            "       u1\n",
            "   y1  -1\n \n",
            "Continuous-time state-space model.",
        )
    );
}

#[test]
fn control_lti_zpk_display_preserves_tiny_and_distinct_factors() {
    // Each pole in the ZPK definition contributes a factor.
    for (poles, denominator) in [
        ("-1e-13", "  (s+1.000e-13)"),
        ("[-1e-11 -9e-11]", "  (s+1.000e-11) (s+9.000e-11)"),
        (
            "[-1e-13+2e-13i -1e-13-2e-13i]",
            "  (s^2 + 2.000e-13s + 5.000e-26)",
        ),
    ] {
        let output = run_output(&format!("disp(zpk([],{poles},1))"));
        assert_eq!(output.lines().nth(2), Some(denominator), "{output}");
    }
}
