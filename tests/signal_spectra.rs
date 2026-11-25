//! Expected values: tool/refs/signal/spectra.py (SciPy, never MATLAB).
use matrust::{
    Array, Engine, Matrix,
    dsp::spectral::{self, Estimate, Options, Range},
};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/signal/spectra.json")).unwrap()
}
fn numbers(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}
// Relative 1e-10, except mathematical zeros (<= 1e-12 of the array peak),
// where an absolute 1e-13 times the array peak allows FFT roundoff.
fn close(actual: f64, expected: f64, scale: f64) {
    let tolerance = if expected.abs() > scale * 1e-12 {
        1e-10 * expected.abs()
    } else {
        1e-13 * scale
    };
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual:.17e} != {expected:.17e} (scale {scale:e})"
    );
}
fn compare(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    let scale = expected.iter().map(|v| v.abs()).fold(1e-300, f64::max);
    for (&a, &b) in actual.iter().zip(expected) {
        close(a, b, scale);
    }
}
fn compare_array(engine: &Engine, name: &str, case: &Value) {
    let actual = engine.array(name).unwrap();
    assert_eq!(
        (actual.rows(), actual.cols()),
        (
            case["rows"].as_u64().unwrap() as usize,
            case["cols"].as_u64().unwrap() as usize
        )
    );
    let re = numbers(&case["re"]);
    let im = case.get("im").map(numbers);
    match actual {
        Array::Numeric(m) => {
            compare(m.data(), &re);
            if let Some(im) = im {
                assert!(im.iter().all(|v| v.abs() < 1e-12));
            }
        }
        Array::Complex(m) => {
            // Relative error for a complex value uses its magnitude, rather
            // than independently dividing by a nearly zero real/imag part.
            let expected: Vec<_> = re
                .iter()
                .enumerate()
                .map(|(i, &re)| matrust::Complex64::new(re, im.as_ref().map_or(0., |v| v[i])))
                .collect();
            let scale = expected.iter().map(|z| z.norm()).fold(1e-300, f64::max);
            for (a, b) in m.data().iter().zip(expected) {
                let tolerance = if b.norm() > 1e-12 * scale {
                    1e-10 * b.norm()
                } else {
                    1e-13 * scale
                };
                assert!((*a - b).norm() <= tolerance, "{}: {a} != {b}", case["call"]);
            }
        }
        _ => panic!("unexpected array type"),
    }
}
#[test]
fn scipy_signal_spectral_fixtures() {
    let f = fixture();
    let mut e = Engine::default();
    for name in ["x", "y"] {
        let data = numbers(&f[name]);
        e.variables.insert(
            name.into(),
            Matrix::from_column_vec(data.len(), 1, data).unwrap(),
        );
    }
    for case in f["cases"].as_array().unwrap() {
        let call = case["call"].as_str().unwrap();
        e.execute(call)
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        compare_array(&e, "z", case);
        for key in ["f", "t", "ps"] {
            if let Some(expected) = case.get(key) {
                compare(e.variables[key].data(), &numbers(expected));
            }
        }
    }
}
#[test]
fn coherence_identity_and_h1_known_fir_response() {
    let f = fixture();
    let mut e = Engine::default();
    let data = numbers(&f["x"]);
    e.variables.insert(
        "x".into(),
        Matrix::from_column_vec(data.len(), 1, data).unwrap(),
    );
    e.execute("[c,w]=mscohere(x,x);").unwrap();
    for &v in e.variables["c"].data() {
        assert!((v - 1.).abs() < 1e-14);
    }
    // An isolated impulse and its complete FIR response eliminate segment-edge
    // bias: H1 should exactly equal the filter's independently computed freqz.
    e.execute("u=zeros(256,1);u(1)=1;v=filter([0.4 0.3 0.2 0.1],1,u);[h,w]=tfestimate(u,v,rectwin(256),0,256);").unwrap();
    let h = match e.array("h").unwrap() {
        Array::Complex(h) => h,
        _ => panic!("complex H1"),
    };
    let expected = &f["fir_response"];
    compare(
        &h.data().iter().map(|z| z.re).collect::<Vec<_>>(),
        &numbers(&expected["re"]),
    );
    compare(
        &h.data().iter().map(|z| z.im).collect::<Vec<_>>(),
        &numbers(&expected["im"]),
    );
    compare(e.variables["w"].data(), &numbers(&expected["f"]));
}
#[test]
fn stft_roundtrip_defaults_custom_windows_and_unaligned_lengths() {
    let f = fixture();
    let data = numbers(&f["x"]);
    for (length, options, inverse_options) in [
        (512, "", ""),
        (
            512,
            ",'FrequencyRange','onesided'",
            ",'FrequencyRange','onesided'",
        ),
        (
            512,
            ",'Window',hann(64),'OverlapLength',32,'FFTLength',128",
            ",'Window',hann(64),'OverlapLength',32,'FFTLength',128",
        ),
        (
            511,
            ",'Window',hamming(63),'OverlapLength',40,'FFTLength',127",
            ",'Window',hamming(63),'OverlapLength',40,'FFTLength',127,'OutputLength',511",
        ),
        (
            512,
            ",'Window',hann(64),'OverlapLength',48,'FrequencyRange','onesided'",
            ",'Window',hann(64),'OverlapLength',48,'FrequencyRange','onesided'",
        ),
    ] {
        let mut e = Engine::default();
        e.variables.insert(
            "x".into(),
            Matrix::from_column_vec(length, 1, data[..length].to_vec()).unwrap(),
        );
        e.execute(&format!(
            "[s,f,t]=stft(x{options});[z,tz]=istft(s{inverse_options});"
        ))
        .unwrap();
        assert_eq!(
            (e.variables["z"].rows(), e.variables["z"].cols()),
            (length, 1)
        );
        for (&a, &b) in e.variables["z"].data().iter().zip(&data) {
            assert!((a - b).abs() < 1e-12, "{a} != {b}");
        }
        assert_eq!(e.variables["tz"].data()[length - 1], (length - 1) as f64);
    }
    let mut e = Engine::default();
    e.execute("x=(1:64)'+1i*(64:-1:1)';[s,f,t]=stft(x,'Window',hamming(16),'OverlapLength',8);[z,tz]=istft(s,'Window',hamming(16),'OverlapLength',8);d=max(abs(x-z));").unwrap();
    assert!(e.variables["d"].data()[0] < 1e-12);
}
#[test]
fn output_arity_plotting_registration_and_help_examples() {
    for name in [
        "periodogram",
        "pwelch",
        "cpsd",
        "mscohere",
        "tfestimate",
        "spectrogram",
        "stft",
        "istft",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name));
        assert!(matrust::help::summary(name).is_some());
    }
    for call in [
        "periodogram(1:32)",
        "pwelch(1:32)",
        "cpsd(1:32,1:32)",
        "mscohere(1:32,1:32)",
        "tfestimate(1:32,1:32)",
    ] {
        let mut e = Engine::default();
        e.execute(call).unwrap();
        assert!(e.figure.is_some(), "{call}");
        assert!(!e.variables.contains_key("ans"));
    }
    let mut e = Engine::default();
    e.execute("z=pwelch(1:32);").unwrap();
    assert!(e.figure.is_none());
    for name in ["spectrogram", "stft"] {
        let error = e.execute(&format!("{name}(1:32)")).unwrap_err();
        assert!(
            error.contains("plots need image plots") && error.contains("request outputs"),
            "{error}"
        );
    }
    assert!(
        e.execute("[a,b,c]=pwelch(1:32);")
            .unwrap_err()
            .contains("at most 2")
    );
}
#[test]
fn invalid_parameters_empty_inputs_complex_ranges_and_nola_fail_cleanly() {
    for call in [
        "z=pwelch(1:16,32);",
        "z=pwelch(1:16,8,8);",
        "z=pwelch(1:16,8,-1);",
        "z=pwelch(1:16,8,2,4);",
        "z=pwelch(1:16,8,2,16,0);",
        "z=pwelch(1:16,8,2,16,Inf);",
        "z=pwelch(1:16,zeros(8,1));",
        "z=pwelch([1 NaN 3]);",
        "z=pwelch([]);",
        "z=cpsd(1:16,1:8);",
        "z=periodogram(1:16,hamming(8));",
        "z=pwelch((1:16)+1i,'onesided');",
        "z=pwelch(1:16,'bad');",
        "z=pwelch(1:16,'psd','power');",
        "z=stft(1:32,'Unknown',3);",
        "z=stft(1:32,'Window',hann(8),'Window',hann(8));",
        "z=istft([]);",
        "[s,f,t]=stft(1:32,'Window',hann(8),'OverlapLength',0);z=istft(s,'Window',hann(8),'OverlapLength',0);",
    ] {
        assert!(Engine::default().execute(call).is_err(), "{call}");
    }
}
#[test]
fn bounded_work_budget_cancellation_and_storage() {
    let x: Array = Matrix::from_column_vec(32, 1, (0..32).map(|x| x as f64).collect())
        .unwrap()
        .into();
    let o = Options {
        window: vec![1.; 8],
        overlap: 4,
        nfft: 16,
        fs: None,
        range: Range::OneSided,
        power: false,
    };
    assert!(
        spectral::estimate(&x, None, Estimate::Welch, &o, 0, &AtomicBool::new(false))
            .unwrap_err()
            .contains("budget")
    );
    assert!(
        spectral::estimate(
            &x,
            None,
            Estimate::Welch,
            &o,
            256 * 1024 * 1024,
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .contains("cancelled")
    );
    let huge = Options {
        window: vec![1.],
        overlap: 0,
        nfft: 1_048_576,
        ..o
    };
    assert!(
        spectral::estimate(
            &x,
            None,
            Estimate::Welch,
            &huge,
            256 * 1024 * 1024,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(
        match x {
            Array::Numeric(m) => m.data()[31],
            _ => unreachable!(),
        },
        31.
    );
}

#[test]
fn parallel_columns_match_serial_and_reduce_jobs_to_fit_budget() {
    let cancel = AtomicBool::new(false);
    let x: Array = Matrix::from_column_vec(
        8192,
        8,
        (0..8192 * 8)
            .map(|j| (j as f64 * 0.017).sin() + 0.2 * (j as f64 * 0.071).cos())
            .collect(),
    )
    .unwrap()
    .into();
    let y: Array = Matrix::from_column_vec(
        8192,
        8,
        (0..8192 * 8).map(|j| (j as f64 * 0.017).cos()).collect(),
    )
    .unwrap()
    .into();
    let o = Options {
        window: vec![1.; 256],
        overlap: 128,
        nfft: 256,
        fs: None,
        range: Range::OneSided,
        power: false,
    };
    let single = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let multi = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    for kind in [
        Estimate::Welch,
        Estimate::Cross,
        Estimate::Coherence,
        Estimate::Transfer,
    ] {
        let other = if kind == Estimate::Welch {
            None
        } else {
            Some(&y)
        };
        let (a, fa) = single.install(|| {
            spectral::estimate(&x, other, kind, &o, 256 * 1024 * 1024, &cancel).unwrap()
        });
        for available in [256 * 1024 * 1024, 1_200_000] {
            let (b, fb) = multi
                .install(|| spectral::estimate(&x, other, kind, &o, available, &cancel).unwrap());
            assert_eq!(fa.data(), fb.data());
            match (&a, &b) {
                (Array::Numeric(a), Array::Numeric(b)) => assert_eq!(a.data(), b.data()),
                (Array::Complex(a), Array::Complex(b)) => assert_eq!(a.data(), b.data()),
                _ => panic!("mismatched output types"),
            }
        }
    }
}
#[test]
fn tiny_complex_stft_information_survives_inverse() {
    let mut e = Engine::default();
    e.execute("x=1i*1e-15*ones(32,1);s=stft(x,'Window',hamming(8),'OverlapLength',4);z=istft(s,'Window',hamming(8),'OverlapLength',4);").unwrap();
    match e.array("z").unwrap() {
        Array::Complex(z) => {
            for v in z.data() {
                assert!((v.im - 1e-15).abs() < 1e-28);
            }
        }
        _ => panic!("discarded small complex components"),
    }
}

#[test]
fn dc_nyquist_parseval_and_zero_power_bins() {
    let mut e = Engine::default();
    e.execute("[dc,w]=periodogram([3 3 3 3],[],4);[ny,w]=periodogram([1 -1 1 -1],[],4);p=periodogram([3 3 3 3],[],4,'power');[c,w]=mscohere(zeros(16,1),zeros(16,1));").unwrap();
    let tau = std::f64::consts::TAU;
    assert!((e.variables["dc"].data()[0] * tau / 4. - 9.).abs() < 1e-13);
    assert!((e.variables["ny"].data()[2] * tau / 4. - 1.).abs() < 1e-13);
    assert_eq!(e.variables["p"].data()[0], 9.);
    assert!(e.variables["c"].data().iter().all(|v| v.is_nan()));
}

#[test]
fn parallel_stft_frames_match_serial_with_all_outputs() {
    let cancel = AtomicBool::new(false);
    let x: Array = Matrix::from_column_vec(
        32768,
        1,
        (0..32768)
            .map(|j| (j as f64 * 0.017).sin() + 0.2 * (j as f64 * 0.071).cos())
            .collect(),
    )
    .unwrap()
    .into();
    let single = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let multi = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    for range in [Range::OneSided, Range::TwoSided, Range::Centered] {
        let o = Options {
            window: vec![1.; 256],
            overlap: 192,
            nfft: 257,
            fs: None,
            range,
            power: false,
        };
        let a = single.install(|| {
            spectral::short_time(&x, &o, true, true, 256 * 1024 * 1024, &cancel).unwrap()
        });
        let b = multi.install(|| {
            spectral::short_time(&x, &o, true, true, 256 * 1024 * 1024, &cancel).unwrap()
        });
        assert_eq!(a.spectrum.data(), b.spectrum.data());
        assert_eq!(a.frequencies.data(), b.frequencies.data());
        assert_eq!(a.times.data(), b.times.data());
        assert_eq!(a.power.unwrap().data(), b.power.unwrap().data());
    }
}

#[test]
fn awkward_cross_fft_lengths_use_actual_scratch_budget() {
    let mut e = Engine::default();
    // An impulse gives nonzero, identical spectra in every bin. Independent
    // identities: Pxy=Pxx, coherence=1, and H1=1.
    e.execute("x=zeros(100,1);x(1)=1;").unwrap();
    for nfft in [83, 263, 997, 1018, 2062] {
        for name in ["cpsd", "mscohere", "tfestimate"] {
            e.execute(&format!("[z,f]={name}(x,x,rectwin(8),0,{nfft});"))
                .unwrap_or_else(|error| panic!("{name}, FFT {nfft}: {error}"));
            assert_eq!(e.array("z").unwrap().rows(), nfft / 2 + 1);
            if name == "cpsd" {
                e.execute(&format!("p=pwelch(x,rectwin(8),0,{nfft});d=max(abs(z-p));"))
                    .unwrap();
            } else {
                e.execute("d=max(abs(z-1));").unwrap();
            }
            assert!(e.variables["d"].data()[0] < 1e-12);
        }
    }
}

#[test]
fn centered_endpoints_and_inverse_for_even_and_odd_ffts() {
    let mut e = Engine::default();
    e.execute("x=sin((0:511)'/7);").unwrap();
    for nfft in [127, 128] {
        for call in [
            format!(
                "[s,f]=stft(x,'Window',hann(64),'OverlapLength',48,'FFTLength',{nfft});z=istft(s,'Window',hann(64),'OverlapLength',48,'FFTLength',{nfft});d=max(abs(x-z));"
            ),
            format!("[s,f]=pwelch(x,64,32,{nfft},'centered');"),
            format!("[s,f]=cpsd(x,x,64,32,{nfft},'centered');"),
            format!("[s,f]=mscohere(x,x,64,32,{nfft},'centered');"),
            format!("[s,f]=tfestimate(x,x,64,32,{nfft},'centered');"),
            format!("[s,f]=spectrogram(x,64,32,{nfft},'centered');"),
            format!("[s,f]=periodogram(x(1:64),[],{nfft},'centered');"),
        ] {
            e.execute(&call).unwrap();
            let f = e.variables["f"].data();
            let step = std::f64::consts::TAU / nfft as f64;
            assert!((f[0] + ((nfft - 1) / 2) as f64 * step).abs() < 1e-14);
            assert!((f[nfft - 1] - (nfft / 2) as f64 * step).abs() < 1e-14);
            assert!(f.windows(2).all(|v| v[1] > v[0]));
            assert!(e.variables["d"].data()[0] < 1e-12);
        }
    }
}
