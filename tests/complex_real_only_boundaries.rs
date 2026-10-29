use matrust::Engine;

#[test]
fn cell_literal_budget_includes_surrounding_whitespace_before_trim() {
    use matrust::Scalar;
    let within = format!("{}1", " ".repeat(255));
    assert_eq!(within.len(), 256);
    assert!(Scalar::parse_cell(&within, true, false).is_ok());
    for text in [format!(" {within}"), format!("{within} "), " ".repeat(257)] {
        for (complex, logical) in [(true, false), (false, false), (false, true)] {
            assert!(
                Scalar::parse_cell(&text, complex, logical)
                    .unwrap_err()
                    .contains("oversized")
            );
        }
    }
    assert_eq!(
        Scalar::parse_cell(" 3-4i ", true, false)
            .unwrap()
            .into_complex(),
        matrust::Complex64::new(3., -4.)
    );
}

#[test]
fn unsupported_real_consumers_reject_complex_storage_without_output_or_figure_changes() {
    for initialization in [
        "Z=complex(2,0);",
        "Z=complex([2 3],[4 5]);",
        "Z=complex(zeros(0,3));",
    ] {
        let mut engine = Engine::default();
        engine.execute("R=42;plot([1 2],[3 4]);").unwrap();
        engine.execute(initialization).unwrap();
        let figure = engine.figure.clone().unwrap();
        let id = engine.current_figure;
        // Complex powers and sin are supported (tests/fn_core.rs); so are
        // trapz, filter and filtfilt (tests/fn_cplx3.rs).
        for source in [
            "R=Z<1;",
            "R=~Z;",
            "R=logical(Z);",
            "R=isnan(Z);",
            "R=upsample(Z,2);",
            "R=downsample(Z,2);",
            "R=upfirdn(Z,1,2,1);",
            "scatter(Z,Z);",
            "if Z;R=9;end",
        ] {
            let before = engine.workspace();
            let ids = engine.figure_ids();
            assert!(
                engine.execute(source).is_err(),
                "{initialization} accepted {source}"
            );
            assert_eq!(engine.workspace_count(), before.len(), "{source}");
            for (name, value) in before {
                assert!(
                    value.same_snapshot(&engine.array(&name).unwrap()),
                    "{source}: {name}"
                );
            }
            assert_eq!(engine.current_figure, id, "{source}");
            assert_eq!(engine.figure_ids(), ids, "{source}");
            assert!(
                engine.figure.as_ref().unwrap().same_snapshot(&figure),
                "{source}"
            );
        }
    }
}
