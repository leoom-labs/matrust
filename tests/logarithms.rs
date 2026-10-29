use matrust::{Array, Complex64, ComplexMatrix, Engine, Matrix};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn put(engine: &mut Engine, value: Array) {
    engine
        .merge_workspace(BTreeMap::from([
            ("A".into(), value.clone()),
            ("alias".into(), value),
        ]))
        .unwrap();
}

fn preserved(engine: &Engine, before: &BTreeMap<String, Array>) {
    for (name, value) in before {
        assert!(engine.array(name).unwrap().same_snapshot(value), "{name}");
    }
}

#[test]
fn shapes_storage_classes_and_aliases_in_one_and_eight_worker_contexts() {
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            assert_eq!(rayon::current_num_threads(), workers);
            let cancel = AtomicBool::new(false);
            for (rows, cols) in [
                (0, 0),
                (1, 0),
                (0, 1),
                (0, 3),
                (3, 0),
                (1, 1),
                (1, 3),
                (3, 1),
                (2, 2),
            ] {
                for complex in [false, true] {
                    let mut engine = Engine::default();
                    let value: Array = if complex {
                        ComplexMatrix::from_column_vec(
                            rows,
                            cols,
                            vec![Complex64::new(1., -0.); rows * cols],
                            &cancel,
                        )
                        .unwrap()
                        .into()
                    } else {
                        Matrix::from_column_vec(rows, cols, vec![1.; rows * cols])
                            .unwrap()
                            .into()
                    };
                    put(&mut engine, value);
                    let before = engine.workspace();
                    engine.execute("two=log2(A);ten=log10(A);").unwrap();
                    preserved(&engine, &before);
                    for name in ["two", "ten"] {
                        let result = engine.array(name).unwrap();
                        assert_eq!((result.rows(), result.cols()), (rows, cols));
                        if complex {
                            let Array::Complex(result) = result else {
                                panic!("complex storage must remain complex")
                            };
                            for z in result.data() {
                                assert_eq!(z.re, 0.);
                                assert_eq!(z.im.to_bits(), (-0.0f64).to_bits());
                            }
                        } else {
                            let Array::Numeric(result) = result else {
                                panic!("real positive input stays real")
                            };
                            assert!(result.data().iter().all(|&x| x == 0.));
                        }
                    }
                    if !complex {
                        engine
                            .execute(
                                "[F,E]=log2(A);[onlyF,~]=log2(A);[~,onlyE]=log2(A);[~,~]=log2(A);",
                            )
                            .unwrap();
                        for name in ["F", "E", "onlyF", "onlyE"] {
                            assert_eq!(
                                (engine.variables[name].rows(), engine.variables[name].cols()),
                                (rows, cols)
                            );
                        }
                        assert_eq!(
                            engine.variables["F"].data(),
                            engine.variables["onlyF"].data()
                        );
                        assert_eq!(
                            engine.variables["E"].data(),
                            engine.variables["onlyE"].data()
                        );
                    }
                    preserved(&engine, &before);
                }
            }
        });
    }
}

#[test]
fn real_powers_promotion_and_explicit_complex_branch_signs() {
    let mut engine = Engine::default();
    engine
        .execute("a=log2([0.25 0.5 1 2 4 1024]);b=log10([0.01 0.1 1 10 100 1000]);")
        .unwrap();
    assert_eq!(engine.variables["a"].data(), &[-2., -1., 0., 1., 2., 10.]);
    assert_eq!(engine.variables["b"].data(), &[-2., -1., 0., 1., 2., 3.]);
    let input = Matrix::from_column_vec(
        2,
        3,
        vec![-2., -0., 0., f64::NAN, f64::INFINITY, f64::NEG_INFINITY],
    )
    .unwrap();
    put(&mut engine, input.into());
    let before = engine.workspace();
    engine.execute("r=log2(A);").unwrap();
    preserved(&engine, &before);
    let r = &engine.complex_variables["r"];
    assert_eq!((r.rows(), r.cols()), (2, 3));
    assert_eq!(r.data()[0].re, 1.);
    assert_eq!(
        r.data()[0].im,
        std::f64::consts::PI / std::f64::consts::LN_2
    );
    for i in [1, 2] {
        assert_eq!(r.data()[i].re, f64::NEG_INFINITY);
        assert_eq!(r.data()[i].im.to_bits(), 0);
    }
    assert!(r.data()[3].re.is_nan());
    assert_eq!(r.data()[3].im.to_bits(), 0);
    assert_eq!(r.data()[4].re, f64::INFINITY);
    assert_eq!(r.data()[4].im.to_bits(), 0);
    assert_eq!(r.data()[5].re, f64::INFINITY);
    assert_eq!(r.data()[5].im, r.data()[0].im);
    let cancel = AtomicBool::new(false);
    let values = [
        Complex64::new(-2., 0.),
        Complex64::new(-2., -0.),
        Complex64::new(2., 0.),
        Complex64::new(2., -0.),
        Complex64::new(f64::INFINITY, f64::NAN),
    ];
    put(
        &mut engine,
        ComplexMatrix::from_column_vec(1, values.len(), values.to_vec(), &cancel)
            .unwrap()
            .into(),
    );
    engine.execute("c=log2(A);").unwrap();
    let c = engine.complex_variables["c"].data();
    assert!(c[0].im > 0. && c[1].im < 0.);
    assert_eq!(c[0].im, -c[1].im);
    assert_eq!(c[2].im.to_bits(), 0);
    assert_eq!(c[3].im.to_bits(), (-0.0f64).to_bits());
    assert!(c[4].re.is_nan() && c[4].im.is_nan());
}

#[test]
fn frexp_exact_ieee_boundary_fixtures_and_omitted_outputs() {
    // Independently specified IEEE-754 decompositions. In particular, no
    // overflowing pow(2,1024) reconstruction or implementation frexp reference.
    let cases = [
        (0x0000000000000000, 0x0000000000000000, 0.),
        (0x8000000000000000, 0x8000000000000000, 0.),
        (0x0000000000000001, 0x3fe0000000000000, -1073.),
        (0x8000000000000001, 0xbfe0000000000000, -1073.),
        (0x000fffffffffffff, 0x3feffffffffffffe, -1022.),
        (0x0010000000000000, 0x3fe0000000000000, -1021.),
        (0x3ff0000000000000, 0x3fe0000000000000, 1.),
        (0xc008000000000000, 0xbfe8000000000000, 2.),
        (0x7fefffffffffffff, 0x3fefffffffffffff, 1024.),
        (0x7ff0000000000000, 0x7ff0000000000000, 0.),
        (0xfff0000000000000, 0xfff0000000000000, 0.),
    ];
    let mut engine = Engine::default();
    put(
        &mut engine,
        Matrix::from_column_vec(
            1,
            cases.len() + 1,
            cases
                .iter()
                .map(|&(x, _, _)| f64::from_bits(x))
                .chain([f64::NAN])
                .collect(),
        )
        .unwrap()
        .into(),
    );
    let before = engine.workspace();
    engine
        .execute("[F,E]=log2(A);[onlyF,~]=log2(A);[~,onlyE]=log2(A);")
        .unwrap();
    for (i, &(_, f, e)) in cases.iter().enumerate() {
        assert_eq!(engine.variables["F"].data()[i].to_bits(), f, "{i}");
        assert_eq!(engine.variables["E"].data()[i], e, "{i}");
        assert_eq!(engine.variables["onlyF"].data()[i].to_bits(), f, "{i}");
        assert_eq!(engine.variables["onlyE"].data()[i], e, "{i}");
    }
    assert!(engine.variables["F"].data()[cases.len()].is_nan());
    assert!(engine.variables["onlyF"].data()[cases.len()].is_nan());
    assert_eq!(engine.variables["E"].data()[cases.len()].to_bits(), 0);
    preserved(&engine, &before);
}

#[test]
fn strict_types_and_complex_dissection_reject_without_publication() {
    let mut engine = Engine::default();
    engine.execute("result=17;F=19;E=23;L=true;LE=false(0,3);C=char([55296 0]);CE=char(zeros(0,3));K={1};KE=cell(0,3);G=figure(9);").unwrap();
    for name in ["log2", "log10"] {
        for arg in [
            "L",
            "LE",
            "C",
            "CE",
            "K",
            "KE",
            "G",
            "'abc'",
            "\"abc\"",
            "disp(123)",
        ] {
            let before = engine.workspace();
            let error = engine
                .execute(&format!("result={name}({arg});"))
                .unwrap_err();
            assert!(error.contains("floating double"), "{name}({arg}): {error}");
            preserved(&engine, &before);
        }
    }
    let cancel = AtomicBool::new(false);
    for (rows, cols) in [(1, 2), (0, 3)] {
        put(
            &mut engine,
            ComplexMatrix::from_column_vec(
                rows,
                cols,
                vec![Complex64::new(2., 0.); rows * cols],
                &cancel,
            )
            .unwrap()
            .into(),
        );
        let before = engine.workspace();
        for outputs in ["F,E", "F,~", "~,E", "~,~"] {
            let error = engine
                .execute(&format!("[{outputs}]=log2(A);"))
                .unwrap_err();
            assert!(error.contains("real double storage"), "{error}");
            preserved(&engine, &before);
        }
        engine.execute("[F,E]=log2(real(A));").unwrap();
        assert_eq!(
            (engine.variables["F"].rows(), engine.variables["F"].cols()),
            (rows, cols)
        );
    }
}

#[test]
fn arity_csl_argument_effects_and_output_preflight_keep_existing_order() {
    let mut engine = Engine::default();
    engine
        .execute("result=17;other=23;third=29;C={8};D={8,16};Z=cell(0,0);")
        .unwrap();
    for name in ["log2", "log10"] {
        for args in ["", "D{:}", "Z{:}"] {
            let before = engine.workspace();
            assert!(
                engine
                    .execute(&format!("result={name}({args});"))
                    .unwrap_err()
                    .contains("exactly one")
            );
            preserved(&engine, &before);
        }
        engine
            .execute(&format!(
                "from_cell={name}(C{{:}});from_empty={name}(Z{{:}},8);"
            ))
            .unwrap();
        assert_eq!(
            engine.variables["from_cell"].data(),
            engine.variables["from_empty"].data()
        );
        assert!(
            engine
                .execute(&format!("result={name}(figure(9),1);"))
                .unwrap_err()
                .contains("exactly one")
        );
        assert_eq!(engine.figure_ids(), vec![9]);
        assert_eq!(engine.variables["result"].data(), &[17.]);
    }
    for call in [
        "[result,other]=log10(figure(10));",
        "[result,other,third]=log2(figure(11));",
    ] {
        let before = engine.workspace();
        assert!(
            engine
                .execute(call)
                .unwrap_err()
                .contains("does not support")
        );
        assert_eq!(engine.figure_ids(), vec![9]);
        preserved(&engine, &before);
    }
    engine.execute("[F,E]=log2(C{:});").unwrap();
    assert_eq!(engine.variables["F"].data(), &[0.5]);
    assert_eq!(engine.variables["E"].data(), &[4.]);
}

#[test]
fn precedence_help_completion_and_cancelled_retry() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["log2", "log10"] {
        let mut local = Engine::default();
        local
            .execute(&format!(
                "[a,b]={name}(3,4);function [x,y]={name}(u,v);x=u;y=v;end"
            ))
            .unwrap();
        assert_eq!(local.variables["a"].data(), &[3.]);
        assert_eq!(local.variables["b"].data(), &[4.]);
        std::fs::write(
            directory.path().join(format!("{name}.m")),
            format!("function y={name}(x,z)\ny=x+z;\nend\n"),
        )
        .unwrap();
        let mut file = Engine::default();
        file.set_function_directory(directory.path()).unwrap();
        file.execute(&format!("answer={name}(7,8);")).unwrap();
        assert_eq!(file.variables["answer"].data(), &[15.]);
        let mut variable = Engine::default();
        variable
            .execute(&format!("{name}=[11 19];answer={name}(2);"))
            .unwrap();
        assert_eq!(variable.variables["answer"].data(), &[19.]);
        assert!(
            matrust::session::completions(name, std::iter::empty())
                .iter()
                .any(|item| item == name)
        );
    }
    let help = Engine::default().execute("help").unwrap().output.join("\n");
    assert!(help.contains("log2") && help.contains("log10"));
    let mut engine = Engine::default();
    engine
        .execute("A=[1 2 4];alias=A;result=17;F=19;E=23;")
        .unwrap();
    let before = engine.workspace();
    let stop = engine.cancellation();
    for call in ["result=log2(A);", "result=log10(A);", "[F,E]=log2(A);"] {
        stop.store(true, Ordering::Relaxed);
        assert!(engine.execute(call).unwrap_err().contains("cancelled"));
        preserved(&engine, &before);
        stop.store(false, Ordering::Relaxed);
    }
    engine
        .execute("result=log2(A);[F,E]=log2(A);log10(100);")
        .unwrap();
    assert_eq!(engine.variables["result"].data(), &[0., 1., 2.]);
    assert_eq!(engine.variables["ans"].data(), &[2.]);
    assert!(engine.array("A").unwrap().same_snapshot(&before["A"]));
    assert!(
        engine
            .array("alias")
            .unwrap()
            .same_snapshot(&before["alias"])
    );
}

fn ulp_distance(a: f64, b: f64) -> u64 {
    // Continuous monotone rank with the two signed zeros sharing a rank.
    fn rank(x: f64) -> u64 {
        let bits = x.to_bits();
        let magnitude = bits & 0x7fff_ffff_ffff_ffff;
        if bits >> 63 == 0 {
            0x8000_0000_0000_0000 + magnitude
        } else {
            0x8000_0000_0000_0000 - magnitude
        }
    }
    rank(a).abs_diff(rank(b))
}

fn cancellation_floor(x: f64, y: f64, divisor: f64) -> f64 {
    let a = x.abs().max(y.abs());
    let b = x.abs().min(y.abs());
    // This fixed fixture class satisfies Sterbenz: 1-a is exact. Each
    // remaining nonnegative operation rounds upward conservatively.
    assert!((0.5..=1.).contains(&a));
    let ap = (1. + a).next_up();
    let t = ((1. - a) * ap).next_up();
    let q = (b * b).next_up();
    let s_up = (t + q).next_up();
    let eight_u = 8. * f64::EPSILON / 2.;
    ((eight_u * s_up).next_up() / divisor.next_down()).next_up()
}

#[test]
fn finite_complex_highprecision_fixture_with_disclosed_cancellation_limits() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/logarithms.json")).unwrap();
    assert_eq!(fixture["schema"], 1);
    assert_eq!(fixture["mpmath_version"], "1.3.0");
    assert_eq!(fixture["precision_bits"], serde_json::json!([256, 512]));
    let rows = fixture["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 80);
    let bits = |row: &serde_json::Value, name: &str| {
        let text = row[name].as_str().unwrap();
        assert_eq!(text.len(), 16);
        assert!(
            text.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        );
        u64::from_str_radix(text, 16).unwrap()
    };
    let cancel = AtomicBool::new(false);
    let mut ids = std::collections::BTreeSet::new();
    let mut cancellation_rows = 0;
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let name = row["base"].as_str().unwrap();
        assert!(ids.insert((id, name)));
        let divisor = match name {
            "log2" => std::f64::consts::LN_2,
            "log10" => std::f64::consts::LN_10,
            _ => panic!("unknown fixture base"),
        };
        let x = f64::from_bits(bits(row, "input_re_bits"));
        let y = f64::from_bits(bits(row, "input_im_bits"));
        let expected = [
            f64::from_bits(bits(row, "expected_re_bits")),
            f64::from_bits(bits(row, "expected_im_bits")),
        ];
        assert!(x.is_finite() && y.is_finite());
        assert!(expected.iter().all(|x| x.is_finite()));
        let mut engine = Engine::default();
        put(
            &mut engine,
            ComplexMatrix::from_column_vec(1, 1, vec![Complex64::new(x, y)], &cancel)
                .unwrap()
                .into(),
        );
        let before = engine.workspace();
        engine.execute(&format!("answer={name}(A);")).unwrap();
        preserved(&engine, &before);
        let result = &engine.complex_variables["answer"];
        assert_eq!((result.rows(), result.cols()), (1, 1));
        let actual = [result.data()[0].re, result.data()[0].im];
        let near = match row["comparison_class"].as_str().unwrap() {
            "ordinary" => false,
            "near_unit_cancellation" => true,
            _ => panic!("unknown comparison class"),
        };
        cancellation_rows += usize::from(near);
        for component in 0..2 {
            let got = actual[component];
            let reference = expected[component];
            assert!(got.is_finite(), "{id}/{name}/{component}: {got}");
            let distance = ulp_distance(got, reference);
            let absolute = (got - reference).abs();
            let relative = if reference == 0. {
                if absolute == 0. { 0. } else { f64::INFINITY }
            } else {
                absolute / reference.abs()
            };
            let spacing = reference.abs().next_up() - reference.abs();
            let limit = if near && component == 0 {
                (8. * spacing).max(cancellation_floor(x, y, divisor))
            } else {
                0.
            };
            // Raw diagnostics are emitted for every row/component, including
            // admitted near-unit results with poor relative/ULP accuracy.
            println!(
                "LOGARITHM_ACCURACY,{id},{name},{component},{},{:016x},{:016x},{distance},{absolute:.17e},{relative:.17e},{limit:.17e}",
                if near && component == 0 {
                    "cancellation_bound"
                } else {
                    "ordinary_8ulp"
                },
                got.to_bits(),
                reference.to_bits(),
            );
            if component == 1 && reference == 0. {
                assert_eq!(
                    got.to_bits(),
                    reference.to_bits(),
                    "{id}/{name}: signed imaginary zero"
                );
            }
            if near && component == 0 {
                assert!(
                    absolute <= limit,
                    "{id}/{name}: {absolute:e} > {limit:e}; ULP={distance}, relative={relative:e}"
                );
            } else {
                assert!(
                    distance <= 8,
                    "{id}/{name}/{component}: ULP={distance}, absolute={absolute:e}, relative={relative:e}"
                );
            }
        }
    }
    assert_eq!(cancellation_rows, 10);
}

#[test]
fn automatic_large_queries_match_serial_context_bits_and_preserve_aliases() {
    fn bits(array: &Array) -> Vec<u64> {
        match array {
            Array::Numeric(matrix) => matrix.data().iter().map(|x| x.to_bits()).collect(),
            Array::Complex(matrix) => matrix
                .data()
                .iter()
                .flat_map(|z| [z.re.to_bits(), z.im.to_bits()])
                .collect(),
            _ => panic!("floating output"),
        }
    }
    let serial_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let parallel_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    for len in [65_535, 65_536, 65_537] {
        for class in 0..3 {
            let positive = [
                0.25,
                0.5,
                1.,
                2.,
                10.,
                f64::MIN_POSITIVE,
                f64::from_bits(1),
                f64::MAX,
            ];
            let data: Vec<f64> = (0..len)
                .map(|i| {
                    let x = positive[i % positive.len()];
                    if class == 1 && i % 5 == 0 { -x } else { x }
                })
                .collect();
            let input: Array = if class == 2 {
                ComplexMatrix::from_column_vec(
                    1,
                    len,
                    data.iter()
                        .enumerate()
                        .map(|(i, &x)| Complex64::new(x, if i % 3 == 0 { -0. } else { -0.25 }))
                        .collect(),
                    &AtomicBool::new(false),
                )
                .unwrap()
                .into()
            } else {
                Matrix::from_column_vec(1, len, data).unwrap().into()
            };
            let run = |workers, pool: &rayon::ThreadPool| {
                pool.install(|| {
                    assert_eq!(rayon::current_num_threads(), workers);
                    let mut engine = Engine::default();
                    put(&mut engine, input.clone());
                    let before = engine.workspace();
                    engine.execute("two=log2(A);ten=log10(A);").unwrap();
                    preserved(&engine, &before);
                    let result = ["two", "ten"].map(|name| {
                        let output = engine.array(name).unwrap();
                        assert_eq!((output.rows(), output.cols()), (1, len));
                        assert_eq!(matches!(output, Array::Complex(_)), class != 0);
                        bits(&output)
                    });
                    if class == 0 {
                        engine.execute("[F,E]=log2(A);[~,onlyE]=log2(A);").unwrap();
                        assert_eq!(
                            bits(&engine.array("E").unwrap()),
                            bits(&engine.array("onlyE").unwrap())
                        );
                    }
                    preserved(&engine, &before);
                    result
                })
            };
            assert_eq!(
                run(1, &serial_pool),
                run(8, &parallel_pool),
                "len={len}, class={class}"
            );
        }
    }
}

#[test]
fn near_unit_prototype_targets_eight_ulp_without_relaxing_existing_observations() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/logarithms.json")).unwrap();
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    for row in fixture["rows"].as_array().unwrap() {
        if row["comparison_class"] != "near_unit_cancellation" {
            continue;
        }
        count += 1;
        let decode = |name: &str| {
            f64::from_bits(u64::from_str_radix(row[name].as_str().unwrap(), 16).unwrap())
        };
        let x = decode("input_re_bits");
        let y = decode("input_im_bits");
        let expected = decode("expected_re_bits");
        let name = row["base"].as_str().unwrap();
        let id = row["id"].as_str().unwrap();
        let mut original = None;
        // Magnitude is invariant under component permutation and sign.
        // These cases reuse the original reference, not generated approximations.
        for (a, b) in [(x, y), (y, x), (-x, y), (x, -y)] {
            let mut engine = Engine::default();
            put(
                &mut engine,
                ComplexMatrix::from_column_vec(1, 1, vec![Complex64::new(a, b)], &cancel)
                    .unwrap()
                    .into(),
            );
            let before = engine.workspace();
            engine.execute(&format!("answer={name}(A);")).unwrap();
            preserved(&engine, &before);
            let value = engine.complex_variables["answer"].data()[0].re;
            assert!(value.is_finite(), "{id}/{name}");
            let distance = ulp_distance(value, expected);
            if let Some(bits) = original {
                assert_eq!(value.to_bits(), bits, "{id}/{name}: magnitude symmetry");
            } else {
                original = Some(value.to_bits());
                println!(
                    "LOGARITHM_COMPENSATION_TARGET,{id},{name},{:016x},{:016x},{distance}",
                    value.to_bits(),
                    expected.to_bits()
                );
            }
            assert!(
                distance <= 8,
                "{id}/{name}: prototype target ULP={distance}"
            );
        }
    }
    assert_eq!(count, 10);
}
