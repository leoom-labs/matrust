//! Authored endian fixtures, not vendor-produced big-endian FIG references.
use super::*;
use crate::files::{encode_native_figure, load_figure, save_figure};
use crate::plot::{Figure, PlotKind, Series};
use std::io::Write;

fn word(value: u32, endian: Endian) -> [u8; 4] {
    match endian {
        Endian::Little => value.to_le_bytes(),
        Endian::Big => value.to_be_bytes(),
    }
}
fn authored_tag(kind: u32, data: &[u8], endian: Endian, compact: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if compact && (1..=4).contains(&data.len()) && kind != 14 && kind != 15 {
        out.extend(word(((data.len() as u32) << 16) | kind, endian));
        out.extend(data);
        out.resize(8, 0);
    } else {
        out.extend(word(kind, endian));
        out.extend(word(data.len() as u32, endian));
        out.extend(data);
        if kind != 15 {
            out.resize(out.len().div_ceil(8) * 8, 0);
        }
    }
    out
}
fn authored_matrix(class: u32, count: usize, kind: u32, data: &[u8], endian: Endian) -> Vec<u8> {
    let mut flags = word(class, endian).to_vec();
    flags.extend(word(0, endian));
    let mut dims = word(1, endian).to_vec();
    dims.extend(word(count as u32, endian));
    let mut payload = authored_tag(6, &flags, endian, true);
    payload.extend(authored_tag(5, &dims, endian, true));
    payload.extend(authored_tag(1, b"hgS_070000", endian, true));
    payload.extend(authored_tag(kind, data, endian, true));
    authored_tag(14, &payload, endian, false)
}
fn wrap(elements: &[u8], endian: Endian, compressed: bool) -> Vec<u8> {
    let mut out = vec![b' '; 128];
    let label = b"MATLAB 5.0 MAT-file, authored ";
    out[..label.len()].copy_from_slice(label);
    out[116..124].fill(0);
    out[124..126].copy_from_slice(match endian {
        Endian::Little => &[0, 1],
        Endian::Big => &[1, 0],
    });
    out[126..].copy_from_slice(match endian {
        Endian::Little => b"IM",
        Endian::Big => b"MI",
    });
    if compressed {
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zlib.write_all(elements).unwrap();
        out.extend(authored_tag(15, &zlib.finish().unwrap(), endian, false));
    } else {
        out.extend(elements);
    }
    out
}

// A test-only independent LE tag walk rewrites an authored supported hierarchy.
// It never calls the production element/header/number decoders. The application
// reader does NOT normalize numeric payloads this way.
fn transcode_le(elements: &[u8], endian: Endian, compact: bool) -> Vec<u8> {
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < elements.len() {
        let tag = u32::from_le_bytes(elements[pos..pos + 4].try_into().unwrap());
        let short = (tag >> 16) as usize;
        let (kind, data, next) = if short != 0 {
            assert!(short <= 4);
            (tag & 65535, &elements[pos + 4..pos + 4 + short], pos + 8)
        } else {
            let len = u32::from_le_bytes(elements[pos + 4..pos + 8].try_into().unwrap()) as usize;
            (
                tag,
                &elements[pos + 8..pos + 8 + len],
                pos + 8 + len.div_ceil(8) * 8,
            )
        };
        assert_ne!(kind, 15, "fixture source must be uncompressed");
        let changed = if kind == 14 {
            transcode_le(data, endian, compact)
        } else {
            let size = match kind {
                3 | 4 | 17 => 2,
                5..=7 | 18 => 4,
                9 | 12 | 13 => 8,
                1 | 2 | 16 => 1,
                _ => panic!("unsupported authored tag {kind}"),
            };
            assert_eq!(data.len() % size, 0);
            let mut changed = data.to_vec();
            if endian == Endian::Big {
                for scalar in changed.chunks_exact_mut(size) {
                    scalar.reverse();
                }
            }
            changed
        };
        out.extend(authored_tag(kind, &changed, endian, compact));
        pos = next;
    }
    assert_eq!(pos, elements.len());
    out
}

#[test]
fn endian_numeric_payloads_remain_borrowed_and_preserve_scalar_bits() {
    // Independent expected values include signed bounds, negative zero and NaN
    // payloads; integer values beyond exact f64 range retain existing rejection.
    let cases = [
        (1, vec![0x80], Some(-128_f64)),
        (2, vec![0xff], Some(255.)),
        (3, (-1234_i16).to_le_bytes().to_vec(), Some(-1234.)),
        (4, 65000_u16.to_le_bytes().to_vec(), Some(65000.)),
        (5, i32::MIN.to_le_bytes().to_vec(), Some(i32::MIN as f64)),
        (6, u32::MAX.to_le_bytes().to_vec(), Some(u32::MAX as f64)),
        (7, (-0_f32).to_le_bytes().to_vec(), Some(-0_f64)),
        (
            7,
            f32::from_bits(1).to_le_bytes().to_vec(),
            Some(f32::from_bits(1) as f64),
        ),
        (9, (-0_f64).to_le_bytes().to_vec(), Some(-0_f64)),
        (
            9,
            f64::from_bits(1).to_le_bytes().to_vec(),
            Some(f64::from_bits(1)),
        ),
        (
            9,
            0x7ff8_0000_0000_1234_u64.to_le_bytes().to_vec(),
            Some(f64::from_bits(0x7ff8_0000_0000_1234)),
        ),
        (
            9,
            f64::NEG_INFINITY.to_le_bytes().to_vec(),
            Some(f64::NEG_INFINITY),
        ),
        (
            12,
            (-9007199254740992_i64).to_le_bytes().to_vec(),
            Some(-9007199254740992.),
        ),
        (
            13,
            9007199254740992_u64.to_le_bytes().to_vec(),
            Some(9007199254740992.),
        ),
        (13, 9007199254740993_u64.to_le_bytes().to_vec(), None),
    ];
    for endian in [Endian::Little, Endian::Big] {
        for (kind, little, expected) in &cases {
            let mut data = little.clone();
            if endian == Endian::Big {
                data.reverse();
            }
            let elements = authored_matrix(6, 1, *kind, &data, endian);
            for compressed in [false, true] {
                let expanded = expand(&wrap(&elements, endian, compressed)).unwrap();
                assert_eq!(expanded.endian, endian);
                assert_eq!(
                    expanded.bytes, elements,
                    "expansion preserves the encoded payload"
                );
                let value = expanded.hierarchy().unwrap();
                assert_eq!(
                    value.number(0).map(f64::to_bits),
                    expected.map(f64::to_bits)
                );
                assert!(value.number(1).is_none());
                let Value::Number { bytes, .. } = value else {
                    panic!("numeric")
                };
                assert_eq!(bytes, data);
                let start = expanded.bytes.as_ptr() as usize;
                assert!((start..start + expanded.bytes.len()).contains(&(bytes.as_ptr() as usize)));
            }
        }
    }
}

#[test]
fn compact_tags_and_unicode_have_independent_known_endian_bytes() {
    // Observed original filters_impedance.fig offset 192 (saved b556 receipt):
    // count 4, miINT32 5, field-name width 11. Not generated by our builder.
    let observed = [0, 4, 0, 5, 0, 0, 0, 11];
    let (kind, payload) = element(&observed, &mut 0, Endian::Big).unwrap();
    assert_eq!(kind, 5);
    assert_eq!(i32::from_be_bytes(payload.try_into().unwrap()), 11);
    // High count/low type in the decoded word: do not swap halfword meaning.
    for (endian, tag) in [
        (Endian::Little, [1, 0, 3, 0, b'a', b'b', b'c', 0]),
        (Endian::Big, [0, 3, 0, 1, b'a', b'b', b'c', 0]),
    ] {
        let mut pos = 0;
        assert_eq!(element(&tag, &mut pos, endian).unwrap(), (1, &b"abc"[..]));
        assert_eq!(pos, 8);
        for kind in [4, 17, 18] {
            let text = "\u{03bb}\u{1f642}";
            let data: Vec<u8> = if kind == 18 {
                text.chars().flat_map(|c| word(c as u32, endian)).collect()
            } else {
                text.encode_utf16()
                    .flat_map(|c| match endian {
                        Endian::Little => c.to_le_bytes(),
                        Endian::Big => c.to_be_bytes(),
                    })
                    .collect()
            };
            let matrix = authored_matrix(4, if kind == 18 { 2 } else { 3 }, kind, &data, endian);
            let expanded = expand(&wrap(&matrix, endian, true)).unwrap();
            assert_eq!(expanded.hierarchy().unwrap().text(), Some(text));
        }
    }
}

fn assert_same_figure(a: &Figure, b: &Figure) {
    assert_eq!(
        encode_native_figure(a).unwrap(),
        encode_native_figure(b).unwrap()
    );
    assert_eq!(a.svg(), b.svg());
    for ((_, a), (_, b)) in a.all_axes().zip(b.all_axes()) {
        for (a, b) in a.series.iter().zip(&b.series) {
            assert_eq!(
                a.points
                    .iter()
                    .map(|p| p.map(f64::to_bits))
                    .collect::<Vec<_>>(),
                b.points
                    .iter()
                    .map(|p| p.map(f64::to_bits))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn authored_big_endian_figs_open_publicly_with_ordered_legends_and_native_roundtrip() {
    let mut engine = crate::Engine::default();
    engine.execute("plot([1 2 3],[-0 2 4],'r--o');title('\u{03bb}\u{1f642}');xlabel('Time');ylabel('Signal');").unwrap();
    let ordinary =
        crate::files::matlab_fig::encode_matlab_figure(engine.figure.as_ref().unwrap()).unwrap();
    let mut exceptional = Figure::default();
    exceptional
        .add(vec![Series::new(
            vec![[1., -0.], [2., f64::from_bits(1)], [3., 1.]],
            PlotKind::Line,
        )])
        .unwrap();
    let exceptional = crate::files::matlab_fig::encode_matlab_figure(&exceptional).unwrap();
    let reordered = include_bytes!("../../tests/fixtures/fig/reordered.fig");
    let subset = include_bytes!("../../tests/fixtures/fig/subset.fig");
    let directory = tempfile::tempdir().unwrap();
    for little in [
        ordinary.as_slice(),
        exceptional.as_slice(),
        reordered.as_slice(),
        subset.as_slice(),
    ] {
        let source = directory.path().join("authored.fig");
        std::fs::write(&source, little).unwrap();
        let expected = load_figure(&source).unwrap();
        for endian in [Endian::Little, Endian::Big] {
            for compact in [false, true] {
                for compressed in [false, true] {
                    let elements = transcode_le(&little[128..], endian, compact);
                    let authored = wrap(&elements, endian, compressed);
                    std::fs::write(&source, &authored).unwrap();
                    let mut opened = crate::Engine::default();
                    let path = source
                        .to_string_lossy()
                        .replace('\\', "/")
                        .replace('\'', "''");
                    opened.execute(&format!("openfig('{path}');")).unwrap();
                    let figure = opened.figure.as_ref().unwrap();
                    assert_same_figure(&expected, figure);
                    let native = directory.path().join("roundtrip.mfig");
                    save_figure(&native, figure).unwrap();
                    assert_same_figure(figure, &load_figure(&native).unwrap());
                    assert_eq!(std::fs::read(&source).unwrap(), authored);
                }
            }
        }
    }
}

#[test]
fn endian_malformed_tags_dimensions_text_and_compression_reject_without_publication() {
    for endian in [Endian::Little, Endian::Big] {
        let data = match endian {
            Endian::Little => 1_f64.to_le_bytes(),
            Endian::Big => 1_f64.to_be_bytes(),
        };
        let good = authored_matrix(6, 1, 9, &data, endian);
        let mut old_name = good.clone();
        let offset = old_name
            .windows(10)
            .position(|w| w == b"hgS_070000")
            .unwrap();
        old_name[offset..offset + 10].copy_from_slice(b"hgS_050200");
        assert_eq!(
            expand(&wrap(&old_name, endian, false))
                .unwrap()
                .hierarchy()
                .unwrap()
                .number(0),
            Some(1.)
        );
        let mut nested = numbers([1.].into_iter());
        for _ in 0..34 {
            nested = structure("", vec![("nested", nested)]);
        }
        let nested = file(structure("hgS_070000", vec![("root", nested)])).unwrap();
        let nested = transcode_le(&nested[128..], endian, true);
        assert!(
            expand(&wrap(&nested, endian, true))
                .unwrap()
                .hierarchy()
                .unwrap_err()
                .contains("nesting")
        );
        let mut negative = good.clone();
        negative[32..36].copy_from_slice(&word(u32::MAX, endian));
        let mut huge = good.clone();
        huge[36..40].copy_from_slice(&word(4_000_001, endian));
        for matrix in [negative, huge, authored_matrix(6, 2, 9, &data, endian)] {
            assert!(
                expand(&wrap(&matrix, endian, false))
                    .unwrap()
                    .hierarchy()
                    .is_err()
            );
        }
        for (kind, data) in [
            (17, vec![0]),
            (
                17,
                match endian {
                    Endian::Little => vec![0, 0xd8],
                    Endian::Big => vec![0xd8, 0],
                },
            ),
            (18, word(0x110000, endian).to_vec()),
        ] {
            assert!(
                expand(&wrap(
                    &authored_matrix(4, 1, kind, &data, endian),
                    endian,
                    false
                ))
                .unwrap()
                .hierarchy()
                .is_err()
            );
        }
        let bad_tag = word((5 << 16) | 1, endian)
            .into_iter()
            .chain([0; 4])
            .collect::<Vec<_>>();
        assert!(
            element(&bad_tag, &mut 0, endian)
                .unwrap_err()
                .contains("compact")
        );
        let mut truncated = wrap(&good, endian, false);
        truncated.pop();
        assert!(expand(&truncated).is_err());
        let mut corrupt = wrap(&good, endian, true);
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(expand(&corrupt).is_err());
        let double = [good.clone(), good].concat();
        assert!(expand(&wrap(&double, endian, true)).is_err());
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("invalid.fig");
        std::fs::write(&source, &corrupt).unwrap();
        let mut engine = crate::Engine::default();
        engine.execute("plot([1 2],[3 4]);").unwrap();
        let before = engine.figure.as_ref().unwrap().clone();
        let ids = engine.figure_ids();
        let path = source
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "''");
        assert!(engine.execute(&format!("openfig('{path}');")).is_err());
        assert_eq!(engine.figure_ids(), ids);
        assert!(crate::plot::SnapshotStamp::new(&before).matches(engine.figure.as_ref().unwrap()));
        assert_eq!(std::fs::read(source).unwrap(), corrupt);
    }
}

#[test]
fn big_endian_nonfinite_handles_and_cancelled_open_preserve_existing_figure() {
    use std::sync::atomic::Ordering;
    let little = include_bytes!("../../tests/fixtures/fig/reordered.fig");
    let elements = transcode_le(&little[128..], Endian::Big, true);
    let offset = {
        let root = hierarchy(&elements, Endian::Big).unwrap();
        let Value::Number { bytes, kind: 9, .. } = &root.structs().unwrap()[0]["handle"] else {
            panic!("double handle")
        };
        bytes.as_ptr() as usize - elements.as_ptr() as usize
    };
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("handle.fig");
    let path = source
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "''");
    let command = format!("openfig('{path}');");
    let mut engine = crate::Engine::default();
    engine
        .execute("A=[1 2];Alias=A;plot([1 2],[3 4]);")
        .unwrap();
    let input = engine.array("A").unwrap();
    let before = engine.figure.as_ref().unwrap().clone();
    let ids = engine.figure_ids();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut mutated = elements.clone();
        mutated[offset..offset + 8].copy_from_slice(&bad.to_be_bytes());
        let bytes = wrap(&mutated, Endian::Big, true);
        std::fs::write(&source, &bytes).unwrap();
        assert!(engine.execute(&command).is_err());
        assert_eq!(engine.figure_ids(), ids);
        assert!(crate::plot::SnapshotStamp::new(&before).matches(engine.figure.as_ref().unwrap()));
        assert!(engine.take_figure_file_events().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
    let good = wrap(&elements, Endian::Big, true);
    std::fs::write(&source, &good).unwrap();
    engine.cancellation().store(true, Ordering::Relaxed);
    assert!(engine.execute(&command).is_err());
    assert_eq!(engine.figure_ids(), ids);
    assert!(crate::plot::SnapshotStamp::new(&before).matches(engine.figure.as_ref().unwrap()));
    engine.cancellation().store(false, Ordering::Relaxed);
    engine.execute(&command).unwrap();
    assert_eq!(
        engine
            .figure
            .as_ref()
            .unwrap()
            .legend_members()
            .map(|(i, _)| i)
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(input.same_snapshot(&engine.array("A").unwrap()));
    assert!(input.same_snapshot(&engine.array("Alias").unwrap()));
    assert_eq!(std::fs::read(source).unwrap(), good);
}

#[test]
#[ignore = "opt-in release measurement; parent owns quiet lane and accepted binary provenance"]
fn fig_endian_release_measurements() {
    if cfg!(debug_assertions) {
        panic!("release mode required");
    }
    let directory = tempfile::tempdir().unwrap();
    let pools: Vec<_> = [1, 8]
        .into_iter()
        .map(|workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
        })
        .collect();
    let mut schedule = Vec::new();
    for count in [64, 8192, 65536] {
        for compressed in [false, true] {
            for endian in [Endian::Little, Endian::Big] {
                for worker in [0, 1] {
                    schedule.push((count, compressed, endian, worker));
                }
            }
        }
    }
    if std::env::var_os("MATRUST_FIG_ENDIAN_BENCH_REVERSE").is_some() {
        schedule.reverse();
    }
    println!(
        "FIG_ENDIAN\tpoints\tcompressed\tendian\tworkers\tphase\tsample\tinput_bytes\telapsed_ns"
    );
    for (count, compressed, endian, worker) in schedule {
        let mut figure = Figure::default();
        figure
            .add(vec![Series::new(
                (0..count)
                    .map(|i| [i as f64 + 1., (i % 17) as f64])
                    .collect(),
                PlotKind::Line,
            )])
            .unwrap();
        let little = crate::files::matlab_fig::encode_matlab_figure(&figure).unwrap();
        let expected = crate::files::matlab_fig::decode_matlab_figure(&little).unwrap();
        let authored = wrap(
            &transcode_le(&little[128..], endian, true),
            endian,
            compressed,
        );
        let source = directory.path().join("measurement.fig");
        std::fs::write(&source, &authored).unwrap();
        for sample in 0..9 {
            let (opened, elapsed) = pools[worker].install(|| {
                assert_eq!(
                    rayon::current_num_threads(),
                    [1, 8][worker],
                    "observed pool size"
                );
                let start = std::time::Instant::now();
                let opened = load_figure(&source).unwrap();
                (opened, start.elapsed().as_nanos())
            });
            assert_same_figure(&expected, &opened);
            println!(
                "FIG_ENDIAN\t{count}\t{compressed}\t{endian:?}\t{}\t{}\t{}\t{}\t{elapsed}",
                [1, 8][worker],
                if sample < 2 { "warmup" } else { "retained" },
                if sample < 2 { sample } else { sample - 2 },
                authored.len()
            );
            drop(opened);
        }
    }
}
