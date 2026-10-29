use matrust::{Array, Complex64, ComplexMatrix, Engine, Matrix, Scalar, files};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

fn bits(a: &ComplexMatrix) -> Vec<(u64, u64)> {
    a.data()
        .iter()
        .map(|v| (v.re.to_bits(), v.im.to_bits()))
        .collect()
}
fn expected(values: &[(f64, f64)]) -> Vec<(u64, u64)> {
    values
        .iter()
        .map(|&(r, i)| (r.to_bits(), i.to_bits()))
        .collect()
}

#[test]
fn complex_script_components_transposes_and_metadata_preserve_storage() {
    let mut e = Engine::default();
    e.execute("Z=complex([1 2;3 4],[5 6;7 8]);A=Z;R=real(Z);I=imag(Z);T=Z.';H=Z';C=conj(Z);D=double(Z);Q=reshape(Z,1,4);[r,c]=size(Z);n=numel(Z);k=isreal(Z);z=isreal(complex(3,0));m=isnumeric(Z);E=complex(zeros(0,7),0);").unwrap();
    assert_eq!(
        bits(&e.complex_variables["Z"]),
        expected(&[(1., 5.), (3., 7.), (2., 6.), (4., 8.)])
    );
    for name in ["A", "D", "Q"] {
        assert!(e.complex_variables["Z"].shares_storage(&e.complex_variables[name]));
    }
    assert_eq!(e.variables["R"].data(), &[1., 3., 2., 4.]);
    assert_eq!(e.variables["I"].data(), &[5., 7., 6., 8.]);
    assert_eq!(
        bits(&e.complex_variables["T"]),
        expected(&[(1., 5.), (2., 6.), (3., 7.), (4., 8.)])
    );
    assert_eq!(
        bits(&e.complex_variables["H"]),
        expected(&[(1., -5.), (2., -6.), (3., -7.), (4., -8.)])
    );
    assert_eq!(
        bits(&e.complex_variables["C"]),
        expected(&[(1., -5.), (3., -7.), (2., -6.), (4., -8.)])
    );
    assert_eq!(e.variables["r"].data(), &[2.]);
    assert_eq!(e.variables["c"].data(), &[2.]);
    assert_eq!(e.variables["n"].data(), &[4.]);
    assert_eq!(e.logical_variables["k"].data(), &[0]);
    assert_eq!(e.logical_variables["z"].data(), &[0]);
    assert_eq!(e.logical_variables["m"].data(), &[1]);
    assert_eq!(
        (
            e.complex_variables["E"].rows(),
            e.complex_variables["E"].cols()
        ),
        (0, 7)
    );
    e.execute("v=abs(complex([3 0],[-4 0]));p=angle(complex([1 0 -1],[0 1 0]));")
        .unwrap();
    assert_eq!(e.variables["v"].data(), &[5., 0.]);
    assert_eq!(
        e.variables["p"].data(),
        &[0., std::f64::consts::FRAC_PI_2, std::f64::consts::PI]
    );
    e.execute("L=complex(logical([0 1]),logical([1 0]));")
        .unwrap();
    assert_eq!(
        bits(&e.complex_variables["L"]),
        expected(&[(0., 1.), (1., 0.)])
    );
}

#[test]
fn complex_indexed_writes_growth_deletion_concatenation_and_function_scopes_are_typed() {
    let mut e = Engine::default();
    e.execute("Z=complex([1 2;3 4],[5 6;7 8]);A=Z;B=Z(:,2);Z(1,2)=complex(9,-10);Z(:,1)=[];Z(end+1)=complex(11,12);X(2)=complex(3,4);M=[A [9;10];logical([1 0]) complex(7,8)];for q=A;last=q;end;[F,G]=parts(A);function [x,y]=parts(z);x=z.';y=imag(z);clear z;end").unwrap();
    assert_eq!(
        bits(&e.complex_variables["A"]),
        expected(&[(1., 5.), (3., 7.), (2., 6.), (4., 8.)])
    );
    assert_eq!(
        bits(&e.complex_variables["Z"]),
        expected(&[(9., -10.), (4., 8.), (11., 12.)])
    );
    assert_eq!(
        bits(&e.complex_variables["B"]),
        expected(&[(2., 6.), (4., 8.)])
    );
    assert_eq!(
        bits(&e.complex_variables["X"]),
        expected(&[(0., 0.), (3., 4.)])
    );
    assert_eq!(
        bits(&e.complex_variables["M"]),
        expected(&[
            (1., 5.),
            (3., 7.),
            (1., 0.),
            (2., 6.),
            (4., 8.),
            (0., 0.),
            (9., 0.),
            (10., 0.),
            (7., 8.)
        ])
    );
    assert_eq!(
        bits(&e.complex_variables["last"]),
        expected(&[(2., 6.), (4., 8.)])
    );
    assert_eq!(
        bits(&e.complex_variables["F"]),
        expected(&[(1., 5.), (2., 6.), (3., 7.), (4., 8.)])
    );
    assert_eq!(e.variables["G"].data(), &[5., 7., 6., 8.]);
    let before = e.workspace();
    assert!(
        e.execute("A=bad(A);function y=bad(x);clear;x=complex(9,9);end")
            .is_err()
    );
    for (name, a) in before {
        assert!(a.same_snapshot(&e.array(&name).unwrap()));
    }
}

#[test]
fn complex_shadowing_clear_and_real_only_errors_preserve_targets() {
    let mut e = Engine::default();
    e.execute("Z=complex([3 4],[-0 2]);R=7;variables=Z;all=Z;clear variables all;")
        .unwrap();
    assert!(e.array("variables").is_none() && e.array("all").is_none());
    for source in [
        "R=Z^2;",
        "R=logical(Z);",
        "R(1)=Z(1);",
        "R=atan2(Z,1);",
        "R=complex(Z,1);",
        "R=Z(complex(1,0));",
    ] {
        let before = e.workspace();
        assert!(e.execute(source).is_err(), "{source}");
        for (name, a) in before {
            assert!(
                a.same_snapshot(&e.array(&name).unwrap()),
                "{source}: {name}"
            );
        }
    }
    e.execute("clear=Z;clear;v=clear(2);").unwrap();
    assert_eq!(bits(&e.complex_variables["v"]), expected(&[(4., 2.)]));
    assert!(e.execute("clear('R')").is_err());
    assert!(e.array("R").is_some());
    e.remove_array("clear");
    e.execute("clear Z v ans;").unwrap();
    assert!(!e.contains_array("Z") && e.contains_array("R"));
    e.execute("clear;").unwrap();
    assert_eq!(e.workspace_count(), 0);
}

#[test]
fn complex_workspace_accounting_and_cell_entry_preserve_components_and_aliases() {
    let cancel = AtomicBool::new(false);
    let nan = f64::from_bits(0x7ff8000000000123);
    let a = ComplexMatrix::from_column_slice(
        2,
        1,
        &[Complex64::new(-0., nan), Complex64::new(f64::MAX, -0.)],
        &cancel,
    )
    .unwrap();
    let original = bits(&a);
    let array = Array::Complex(a.clone());
    assert_eq!(array.bytes(), 32);
    assert_eq!(array.class(), "double");
    assert!(array.is_complex());
    assert!(array.get(0, 0).unwrap().into_real().is_err());
    assert!(array.view().is_err() && array.to_numeric(&cancel).is_err());
    let entry = Scalar::parse_cell("-0-0i", true, false).unwrap();
    let edited = array.with_scalar(0, 0, entry, &cancel).unwrap();
    assert_eq!(
        bits(edited.as_complex().unwrap())[0],
        ((-0.0f64).to_bits(), (-0.0f64).to_bits())
    );
    assert_eq!(bits(&a), original);
    for literal in ["i", "-i", "1e-20-2e+30i", "3+4j", "Inf-NaNi"] {
        assert!(
            Scalar::parse_cell(literal, true, false).is_ok(),
            "{literal}"
        );
    }
    for literal in ["1+2", "complex(1,2)", "1++2i", "NaN+Inf+2i", ""] {
        assert!(
            Scalar::parse_cell(literal, true, false).is_err(),
            "{literal}"
        );
    }
    let mut e = Engine::default();
    e.merge_workspace(BTreeMap::from([
        ("Z".into(), array.clone()),
        ("A".into(), array.clone()),
    ]))
    .unwrap();
    assert_eq!(e.workspace_bytes(), 64);
    assert!(e.array("A").unwrap().same_snapshot(&array));
    e.cancellation().store(true, Ordering::Relaxed);
    assert!(
        e.merge_workspace(BTreeMap::from([("Z".into(), Matrix::scalar(1.).into())]))
            .is_err()
    );
    assert!(e.array("Z").unwrap().same_snapshot(&array));
    cancel.store(true, Ordering::Relaxed);
    assert!(array.with_scalar(1, 0, Scalar::Real(3.), &cancel).is_err());
    assert_eq!(bits(&a), original);
}

#[test]
fn complex_payload_and_workspace_limits_count_both_components_without_alias_copies() {
    let cancel = AtomicBool::new(false);
    assert!(ComplexMatrix::checked_len(8_388_609, 1).is_err());
    assert!(ComplexMatrix::checked_len(usize::MAX, 2).is_err());
    let payload = ComplexMatrix::filled(65536, 1, Complex64::new(1., 2.), &cancel).unwrap();
    let mut e = Engine::default();
    e.merge_workspace(
        (0..256)
            .map(|i| (format!("Z{i}"), Array::Complex(payload.clone())))
            .collect(),
    )
    .unwrap();
    assert_eq!(e.workspace_bytes(), 256 * 1024 * 1024);
    let pointer = payload.data().as_ptr();
    assert!(
        e.execute("extra=complex(1,2);")
            .unwrap_err()
            .contains("256 MiB")
    );
    assert!(!e.contains_array("extra"));
    assert_eq!(e.complex_variables["Z0"].data().as_ptr(), pointer);
    e.execute("clear Z0;extra=complex(1,2);").unwrap();
    assert_eq!(e.workspace_bytes(), 255 * 1024 * 1024 + 16);
    let empty = ComplexMatrix::filled(0, 3, Complex64::new(0., 0.), &cancel).unwrap();
    e.clear_arrays();
    e.merge_workspace(
        (0..10000)
            .map(|i| (format!("Z{i}"), Array::Complex(empty.clone())))
            .collect(),
    )
    .unwrap();
    assert!(
        e.execute("extra=complex(1,2);")
            .unwrap_err()
            .contains("10000")
    );
    assert_eq!(e.workspace_count(), 10000);
}

#[test]
fn remaining_complex_file_boundaries_reject_before_publishing_even_empty_complex() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for matrix in [
        ComplexMatrix::scalar(Complex64::new(1., 2.), &cancel).unwrap(),
        ComplexMatrix::filled(0, 7, Complex64::new(0., 0.), &cancel).unwrap(),
    ] {
        let array = Array::Complex(matrix);
        let path = dir.path().join("preserved.mat");
        std::fs::write(&path, b"original").unwrap();
        assert!(files::write_array_csv(&path, &array, &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(files::write_matrix_text(&path, &array, ',', &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }
}

#[test]
fn complex_script_large_gather_components_and_edits_match_across_worker_counts() {
    let cancel = AtomicBool::new(false);
    let count = 1_048_577;
    let source = ComplexMatrix::from_column_vec(
        count,
        1,
        (0..count)
            .map(|i| {
                Complex64::new(
                    (i % 997) as f64,
                    if i % 17 == 0 { -0. } else { -(i as f64) },
                )
            })
            .collect(),
        &cancel,
    )
    .unwrap();
    let before = bits(&source);
    let mut previous = None;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let mut e = Engine::default();
        e.complex_variables.insert("Z".into(), source.clone());
        pool.install(|| e.execute("A=Z;G=Z(1:2:end);R=imag(Z);Z(end)=complex(4,5);"))
            .unwrap();
        let actual = bits(&e.complex_variables["G"]);
        assert_eq!(actual.len(), count.div_ceil(2));
        for (i, &pair) in actual.iter().enumerate() {
            assert_eq!(pair, before[i * 2]);
        }
        if let Some(previous) = &previous {
            assert_eq!(&actual, previous);
        }
        previous = Some(actual);
        assert!(source.shares_storage(&e.complex_variables["A"]));
        assert_eq!(
            e.complex_variables["Z"].data()[count - 1],
            Complex64::new(4., 5.)
        );
        for (i, &v) in e.variables["R"].data().iter().enumerate() {
            assert_eq!(v.to_bits(), before[i].1);
        }
        assert_eq!(bits(&source), before);
    }
}
