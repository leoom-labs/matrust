//! Public native DICOM writer regressions, synthetic pixels only.
use matrust::Engine;

#[test]
fn uint16_public_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("write.dcm");
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "dicomwrite(uint16([0 1 65535;32768 42 256]),'{}');",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[128..132], b"DICM");
    assert!(bytes.ends_with(&[0, 0, 1, 0, 255, 255, 0, 128, 42, 0, 0, 1]));
}

/// 7a10 baseline: on the 7a9b HEAD this fails at the unsigned-only class guard.
#[test]
fn int16_public_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    e.execute("X=int16([0 -1 2 -32768 32767;5 -6 7 -8 9;-300 300 -4 4 0]); dicomwrite(X,'s16.dcm'); Y=dicomread('s16.dcm'); same=isequal(X,Y)&&isa(Y,'int16')&&isequal(size(Y),[3 5]);").unwrap();
    assert_eq!(
        e.array("same").unwrap().scalar(0, 0).unwrap(),
        matrust::Scalar::Logical(true)
    );
    let bytes = std::fs::read(dir.path().join("s16.dcm")).unwrap();
    // Row-major little-endian two's complement, default explicit LE.
    assert!(bytes.ends_with(&[
        0, 0, 255, 255, 2, 0, 0, 128, 255, 127, 5, 0, 250, 255, 7, 0, 248, 255, 9, 0, 212, 254, 44,
        1, 252, 255, 4, 0, 0, 0
    ]));
}

const SYNTAXES: [&str; 3] = [
    "1.2.840.10008.1.2.1",
    "1.2.840.10008.1.2",
    "1.2.840.10008.1.2.2",
];
const META: &str = "struct('PatientName','Synthétique^Éva','PatientID','TEST-ONLY','StudyDate','20000229','StudyTime','030405.125','Modality','OT','StudyDescription','Synthetic study','SeriesDescription','Synthetic series','SeriesNumber',7,'InstanceNumber','9')";
const IDS: &str = "struct('SOPInstanceUID','2.25.123','StudyInstanceUID','2.25.456','SeriesInstanceUID','2.25.789')";

fn folder(path: &std::path::Path) -> Engine {
    let mut e = Engine::default();
    e.set_file_directory(path).unwrap();
    e
}
fn text(e: &Engine, name: &str) -> String {
    match e.array(name).unwrap() {
        matrust::Array::Character(c) => c
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap(),
        a => panic!("expected character, got {}", a.class()),
    }
}
fn generate_outputs(dir: &std::path::Path) {
    let mut e = folder(dir);
    for bits in [8, 16] {
        let pixels = if bits == 8 {
            "[0 1 255;128 42 17]"
        } else {
            "[0 1 65535;32768 42 256]"
        };
        for (i, syntax) in SYNTAXES.iter().enumerate() {
            e.execute(&format!("dicomwrite(uint{bits}({pixels}),'u{bits}_{i}.dcm',{META},'tRaNsFeRsYnTaX','{syntax}');")).unwrap();
        }
    }
    for (i, syntax) in SYNTAXES.iter().enumerate() {
        e.execute(&format!(
            "dicomwrite(uint8([1 255 7]),'odd_{i}.dcm',{IDS},'TransferSyntax','{syntax}');"
        ))
        .unwrap();
    }
}

/// (file stem, X expression): signed, RGB and multiframe writer cases.
/// tool/refs/dicom/write_check.py mirrors every expression in NumPy.
const SHAPES: [(&str, &str); 10] = [
    ("i8", "int8([0 -1 127;-128 42 -17])"),
    (
        "i16",
        "int16([0 -1 2 -32768 32767;5 -6 7 -8 9;-300 300 -4 4 0])",
    ),
    ("rgb8", "uint8(reshape(mod((0:17)*37,256),2,3,3))"),
    ("rgb8odd", "uint8(reshape(1:9,1,3,3))"),
    ("rgb16", "uint16(reshape(mod((0:17)*7919,65536),2,3,3))"),
    ("g8f", "uint8(reshape(mod((0:23)*11,256),2,3,1,4))"),
    ("i8f", "int8(reshape((-10:10)*12,1,7,1,3))"),
    ("i16f", "int16(reshape((-12:11)*2730,2,3,1,4))"),
    ("rgb8f", "uint8(reshape(mod((0:53)*5,256),3,1,3,6))"),
    ("rgb16f", "uint16(reshape((0:35)*1821,2,3,3,2))"),
];

fn generate_shapes(dir: &std::path::Path) {
    let mut e = folder(dir);
    for (stem, x) in SHAPES {
        for (i, syntax) in SYNTAXES.iter().enumerate() {
            e.execute(&format!(
                "dicomwrite({x},'{stem}_{i}.dcm',{IDS},'TransferSyntax','{syntax}');"
            ))
            .unwrap();
        }
    }
}
fn logical(e: &Engine, name: &str) -> bool {
    e.array(name).unwrap().scalar(0, 0).unwrap() == matrust::Scalar::Logical(true)
}
fn real(e: &Engine, name: &str, index: usize) -> f64 {
    e.array(name)
        .unwrap()
        .scalar(0, index)
        .unwrap()
        .into_real()
        .unwrap()
}
/// PixelData element header for a value of `len` bytes in syntax `i`.
fn pixel_header(i: usize, wide: bool, len: usize) -> Vec<u8> {
    let len = u32::try_from(len).unwrap();
    let vr = [b'O', if wide { b'W' } else { b'B' }, 0, 0];
    match i {
        1 => [&[0xe0, 0x7f, 0x10, 0][..], &len.to_le_bytes()].concat(),
        2 => [&[0x7f, 0xe0, 0, 0x10][..], &vr, &len.to_be_bytes()].concat(),
        _ => [&[0xe0, 0x7f, 0x10, 0][..], &vr, &len.to_le_bytes()].concat(),
    }
}

#[test]
fn signed_rgb_and_multiframe_payloads_metadata_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    generate_shapes(dir.path());
    let mut e = folder(dir.path());
    for (stem, source) in SHAPES {
        e.execute(&format!("X={source};")).unwrap();
        let x = e.array("X").unwrap();
        let dims = x.dims().into_owned();
        let (rows, cols) = (dims[0], dims[1]);
        let samples = dims.get(2).copied().unwrap_or(1);
        let frames = dims.get(3).copied().unwrap_or(1);
        let class = x.class();
        let wide = class.ends_with("16");
        let signed = class.starts_with("int");
        for (i, syntax) in SYNTAXES.iter().enumerate() {
            // Frame after frame, row-major, the samples of one pixel adjacent;
            // only 16-bit cells follow the transfer syntax byte order.
            let mut expected = Vec::new();
            for f in 0..frames {
                for r in 0..rows {
                    for c in 0..cols {
                        for s in 0..samples {
                            let matrust::Scalar::Integer(v) =
                                x.scalar(r, c + cols * (s + samples * f)).unwrap()
                            else {
                                panic!("{stem}: typed integer expected")
                            };
                            let cell = v.to_le_bytes();
                            match (wide, i) {
                                (false, _) => expected.push(cell[0]),
                                (true, 2) => expected.extend([cell[1], cell[0]]),
                                (true, _) => expected.extend([cell[0], cell[1]]),
                            }
                        }
                    }
                }
            }
            if expected.len() % 2 == 1 {
                expected.push(0);
            }
            let name = format!("{stem}_{i}.dcm");
            let bytes = std::fs::read(dir.path().join(&name)).unwrap();
            assert!(bytes.ends_with(&expected), "{name}: payload");
            let header = pixel_header(i, wide || i == 1, expected.len());
            assert!(
                bytes[..bytes.len() - expected.len()].ends_with(&header),
                "{name}: PixelData header"
            );
            e.execute(&format!("I=dicominfo('{name}'); ts=I.TransferSyntaxUID; photo=I.PhotometricInterpretation; pix=double([I.Rows I.Columns I.SamplesPerPixel I.BitsAllocated I.BitsStored I.HighBit I.PixelRepresentation]); planar=isfield(I,'PlanarConfiguration'); multi=isfield(I,'NumberOfFrames');")).unwrap();
            assert_eq!(text(&e, "ts"), *syntax);
            assert_eq!(
                text(&e, "photo"),
                if samples == 3 { "RGB" } else { "MONOCHROME2" }
            );
            let bits = if wide { 16. } else { 8. };
            let want = [
                rows as f64,
                cols as f64,
                samples as f64,
                bits,
                bits,
                bits - 1.,
                f64::from(signed),
            ];
            for (k, want) in want.into_iter().enumerate() {
                assert_eq!(real(&e, "pix", k), want, "{name}: pixel attribute {k}");
            }
            // PlanarConfiguration only with RGB; NumberOfFrames only with frames.
            assert_eq!(logical(&e, "planar"), samples == 3, "{name}");
            assert_eq!(logical(&e, "multi"), frames > 1, "{name}");
            if samples == 3 {
                e.execute("pc=double(I.PlanarConfiguration);").unwrap();
                assert_eq!(real(&e, "pc", 0), 0., "{name}");
            }
            if frames > 1 {
                e.execute("nf=double(I.NumberOfFrames);").unwrap();
                assert_eq!(real(&e, "nf", 0), frames as f64, "{name}");
            }
            let read = format!(
                "Y=dicomread('{name}'); same=isequal(X,Y)&&strcmp(class(X),class(Y))&&isequal(size(X),size(Y));"
            );
            e.execute(&read)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(logical(&e, "same"), "{name}: round trip");
        }
    }
}

/// Hand-written payloads, independent of any index formula.
#[test]
fn signed_rgb_and_multiframe_literal_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    let be = "'TransferSyntax','1.2.840.10008.1.2.2'";
    let cases: [(&str, &str, &[u8]); 7] = [
        // Big endian swaps each 16-bit two's complement cell.
        (
            "int16([0 -1 2 -32768 32767;5 -6 7 -8 9;-300 300 -4 4 0])",
            be,
            &[
                0, 0, 255, 255, 0, 2, 128, 0, 127, 255, 0, 5, 255, 250, 0, 7, 255, 248, 0, 9, 254,
                212, 1, 44, 255, 252, 0, 4, 0, 0,
            ],
        ),
        // R=[1 2;3 4], G=[258 513;1027 65535], B=[0 32768;255 256]: R,G,B per pixel.
        (
            "uint16(reshape([1 3 2 4 258 1027 513 65535 0 255 32768 256],2,2,3))",
            be,
            &[
                0, 1, 1, 2, 0, 0, 0, 2, 2, 1, 128, 0, 0, 3, 4, 3, 0, 255, 0, 4, 255, 255, 1, 0,
            ],
        ),
        (
            "uint16(reshape([1 3 2 4 258 1027 513 65535 0 255 32768 256],2,2,3))",
            "'TransferSyntax','1.2.840.10008.1.2'",
            &[
                1, 0, 2, 1, 0, 0, 2, 0, 1, 2, 0, 128, 3, 0, 3, 4, 255, 0, 4, 0, 255, 255, 0, 1,
            ],
        ),
        // 8-bit cells are never swapped, signed or not.
        (
            "int8([0 -1 127;-128 42 -17])",
            be,
            &[0, 255, 127, 128, 42, 239],
        ),
        // Frames [1 2 3] and [4 5 6] are concatenated without padding.
        ("uint8(reshape(1:6,1,3,1,2))", be, &[1, 2, 3, 4, 5, 6]),
        // Frame 1: R=[1 2] G=[3 4] B=[5 6]; frame 2: R=[7 8] G=[9 10] B=[11 12].
        (
            "uint8(reshape(1:12,1,2,3,2))",
            be,
            &[1, 3, 5, 2, 4, 6, 7, 9, 11, 8, 10, 12],
        ),
        // Odd RGB byte count: one trailing zero pad after the last frame only.
        (
            "uint8(reshape([10 20 30 40 50 60 70 80 90],1,1,3,3))",
            "'TransferSyntax','1.2.840.10008.1.2.1'",
            &[10, 20, 30, 40, 50, 60, 70, 80, 90, 0],
        ),
    ];
    for (k, (x, option, payload)) in cases.into_iter().enumerate() {
        e.execute(&format!("dicomwrite({x},'literal{k}.dcm',{option});"))
            .unwrap();
        let bytes = std::fs::read(dir.path().join(format!("literal{k}.dcm"))).unwrap();
        assert!(bytes.ends_with(payload), "{x} {option}");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), cases.len());
}

#[test]
fn class_rank_and_dimension_errors_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    for (source, needle) in [
        ("dicomwrite(int32(1),'out.dcm')", "class int32"),
        ("dicomwrite(uint32(1),'out.dcm')", "class uint32"),
        ("dicomwrite(int64(1),'out.dcm')", "class int64"),
        ("dicomwrite(uint64(1),'out.dcm')", "class uint64"),
        ("dicomwrite(single(1),'out.dcm')", "class single"),
        ("dicomwrite(1.5,'out.dcm')", "class double"),
        ("dicomwrite(true,'out.dcm')", "class logical"),
        ("dicomwrite(1+2i,'out.dcm')", "class complex double"),
        ("dicomwrite('ab','out.dcm')", "class char"),
        ("dicomwrite({1},'out.dcm')", "class cell"),
        // A third dimension other than 3 names the 4-D multiframe form.
        (
            "dicomwrite(uint8(ones(2,2,2)),'out.dcm')",
            "R-by-C-by-1-by-F",
        ),
        (
            "dicomwrite(uint8(ones(2,2,4)),'out.dcm')",
            "R-by-C-by-1-by-F",
        ),
        (
            "dicomwrite(int16(ones(2,2,7)),'out.dcm')",
            "R-by-C-by-1-by-F",
        ),
        (
            "dicomwrite(uint8(ones(2,2,2,2)),'out.dcm')",
            "R-by-C-by-3-by-F",
        ),
        (
            "dicomwrite(uint8(ones(2,2,4,2)),'out.dcm')",
            "R-by-C-by-3-by-F",
        ),
        (
            "dicomwrite(uint8(ones(2,2,1,2,2)),'out.dcm')",
            "more than 4 dimensions",
        ),
        (
            "dicomwrite(uint8(ones(2,2,3,2,2)),'out.dcm')",
            "more than 4 dimensions",
        ),
        (
            "dicomwrite(int8(ones(2,2,3)),'out.dcm')",
            "signed truecolor",
        ),
        (
            "dicomwrite(int16(ones(2,2,3,2)),'out.dcm')",
            "signed truecolor",
        ),
        ("dicomwrite(uint8(zeros(0,3)),'out.dcm')", "1..65535"),
        ("dicomwrite(uint8(zeros(0,3,3)),'out.dcm')", "1..65535"),
        ("dicomwrite(uint8(zeros(2,0,1,2)),'out.dcm')", "1..65535"),
        ("dicomwrite(uint8(zeros(65536,1,3)),'out.dcm')", "65535"),
        ("dicomwrite(uint8(zeros(1,65536,1,2)),'out.dcm')", "65535"),
        // Indexed images stay explicit, whatever the class of X.
        (
            "dicomwrite(uint8([1 2]),[0 0 0;1 1 1],'out.dcm')",
            "indexed images dicomwrite(X,cmap,filename) are not supported yet",
        ),
        (
            "dicomwrite([1 2],[0 0 0;1 1 1],'out.dcm')",
            "indexed images dicomwrite(X,cmap,filename) are not supported yet",
        ),
        (
            "dicomwrite(uint8([1 2]),single([0 0 0;1 1 1]),'out.dcm',struct('PatientID','x'))",
            "indexed images dicomwrite(X,cmap,filename) are not supported yet",
        ),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(needle), "{source}: {error}");
        assert!(error.contains("dicomwrite:"), "{source}: {error}");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "{source}"
        );
    }
}

#[test]
fn derived_pixel_metadata_agrees_or_errors_for_new_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    // Agreeing values are accepted; the file still carries the derived ones.
    for (k, source) in [
        "dicomwrite(uint8(ones(2,2,3,2)),'FILE',struct('Rows',2,'Columns',2,'SamplesPerPixel',3,'PhotometricInterpretation','RGB','PlanarConfiguration',0,'NumberOfFrames',2,'BitsAllocated',8,'BitsStored',8,'HighBit',7,'PixelRepresentation',0))",
        "dicomwrite(uint16(ones(2,2,3)),'FILE',struct('SamplesPerPixel',3,'PhotometricInterpretation','RGB','PlanarConfiguration',uint16(0),'NumberOfFrames','1','BitsAllocated',16))",
        "dicomwrite(int16(-ones(2,2,1,3)),'FILE',struct('SamplesPerPixel',1,'PhotometricInterpretation','MONOCHROME2','NumberOfFrames','3','BitsStored',16,'HighBit',15,'PixelRepresentation',1))",
        "dicomwrite(int8([-1 1]),'FILE',struct('NumberOfFrames',1,'BitsAllocated',8,'BitsStored',8,'HighBit',7,'PixelRepresentation',1))",
        "dicomwrite(uint8(1),'FILE',struct('NumberOfFrames',1))",
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("ok{k}.dcm");
        e.execute(&source.replace("FILE", &name)).unwrap();
        e.execute(&format!("I=dicominfo('{name}'); multi=isfield(I,'NumberOfFrames'); planar=isfield(I,'PlanarConfiguration');")).unwrap();
        assert_eq!(logical(&e, "multi"), k == 0 || k == 2, "{source}");
        assert_eq!(logical(&e, "planar"), k < 2, "{source}");
    }
    let before = std::fs::read_dir(dir.path()).unwrap().count();
    for source in [
        "dicomwrite(uint8(ones(2,2,3)),'out.dcm',struct('SamplesPerPixel',1))",
        "dicomwrite(uint8(ones(2,2,3)),'out.dcm',struct('PhotometricInterpretation','MONOCHROME2'))",
        "dicomwrite(uint8(ones(2,2,3)),'out.dcm',struct('PhotometricInterpretation','YBR_FULL'))",
        "dicomwrite(uint8(ones(2,2,3)),'out.dcm',struct('PlanarConfiguration',1))",
        "dicomwrite(uint8(ones(2,2,3)),'out.dcm',struct('PlanarConfiguration','0'))",
        "dicomwrite(uint8(ones(2,2,3)),'out.dcm',struct('NumberOfFrames',3))",
        "dicomwrite(uint8(ones(2,2,3)),'out.dcm',struct('Columns',6))",
        "dicomwrite(uint8(ones(2,2,3,2)),'out.dcm',struct('Columns',12))",
        "dicomwrite(uint8(ones(2,2,3,2)),'out.dcm',struct('NumberOfFrames',1))",
        "dicomwrite(uint8(ones(2,2,3,2)),'out.dcm',struct('NumberOfFrames',6))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('NumberOfFrames',2))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('NumberOfFrames','4'))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('NumberOfFrames',''))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('NumberOfFrames','three'))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('NumberOfFrames',3.5))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('NumberOfFrames',[3 3]))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('SamplesPerPixel',3))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('PlanarConfiguration',0))",
        "dicomwrite(uint8(ones(2,2,1,3)),'out.dcm',struct('PhotometricInterpretation','RGB'))",
        "dicomwrite(int16(1),'out.dcm',struct('PixelRepresentation',0))",
        "dicomwrite(int16(1),'out.dcm',struct('BitsAllocated',8))",
        "dicomwrite(int16(1),'out.dcm',struct('BitsStored',12))",
        "dicomwrite(int8(1),'out.dcm',struct('PixelRepresentation',0))",
        "dicomwrite(int8(1),'out.dcm',struct('BitsAllocated',16))",
        "dicomwrite(int8(1),'out.dcm',struct('HighBit',15))",
        "dicomwrite(uint8(1),'out.dcm',struct('NumberOfFrames',2))",
        "dicomwrite(uint8(1),'out.dcm',struct('NumberOfFrames',0))",
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(
            error.contains("contradicts") || error.contains("must be") || error.contains("invalid"),
            "{source}: {error}"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            before,
            "{source}"
        );
    }
}

#[test]
fn multiframe_memory_evaluation_and_cancellation_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.dcm");
    let mut e = folder(dir.path());
    // 64*64*3*4 = 49152 samples: input + payload + 96 KiB metadata headroom.
    // Z only raises the workspace so the 8-byte element cap stays above X.
    e.execute("X=uint8(ones(64,64,3,4)); S=int16(-ones(64,64,1,6)); Z=zeros(1,40000);")
        .unwrap();
    for exists in [false, true] {
        if exists {
            std::fs::write(&path, b"SENTINEL").unwrap();
        }
        // Enough for one R*C plane of output, not for all samples and frames.
        e.limits_set(matrust::engine::Limits {
            memory_bytes: e.workspace_bytes() + 190_000,
            ..Default::default()
        });
        for source in ["dicomwrite(X,'out.dcm')", "dicomwrite(S,'out.dcm')"] {
            let error = e.execute(source).unwrap_err();
            assert!(error.contains("memory budget"), "{source}: {error}");
        }
        // Evaluation is charged for every sample of every frame.
        e.limits_set(matrust::engine::Limits {
            evaluation_units: Some(49152 + 100),
            ..Default::default()
        });
        assert!(e.execute("dicomwrite(X,'out.dcm')").is_err());
        e.limits_set(Default::default());
        let cancel = e.cancellation();
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        for source in ["dicomwrite(X,'out.dcm')", "dicomwrite(S,'out.dcm')"] {
            assert!(
                e.execute(source)
                    .unwrap_err()
                    .to_lowercase()
                    .contains("cancel"),
                "{source}"
            );
        }
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        if exists {
            assert_eq!(std::fs::read(&path).unwrap(), b"SENTINEL");
        } else {
            assert!(!path.exists());
        }
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            usize::from(exists)
        );
    }
    // The same call succeeds just inside the limits and replaces the sentinel.
    e.limits_set(matrust::engine::Limits {
        memory_bytes: e.workspace_bytes() + 210_000,
        evaluation_units: Some(100_000),
        ..Default::default()
    });
    e.execute("dicomwrite(X,'out.dcm');").unwrap();
    e.limits_set(Default::default());
    e.execute("Y=dicomread('out.dcm'); same=isequal(X,Y);")
        .unwrap();
    assert!(logical(&e, "same"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn native_syntax_classes_metadata_and_exact_payloads() {
    let dir = tempfile::tempdir().unwrap();
    generate_outputs(dir.path());
    let mut e = folder(dir.path());
    for bits in [8, 16] {
        for (i, syntax) in SYNTAXES.iter().enumerate() {
            let bytes = std::fs::read(dir.path().join(format!("u{bits}_{i}.dcm"))).unwrap();
            let expected: Vec<u8> = if bits == 8 {
                vec![0, 1, 255, 128, 42, 17]
            } else if i == 2 {
                vec![0, 0, 0, 1, 255, 255, 128, 0, 0, 42, 1, 0]
            } else {
                vec![0, 0, 1, 0, 255, 255, 0, 128, 42, 0, 0, 1]
            };
            assert!(bytes.ends_with(&expected));
            let header: &[u8] = match (bits, i) {
                (_, 1) => &[0xe0, 0x7f, 0x10, 0],
                (8, 2) => &[0x7f, 0xe0, 0, 0x10, b'O', b'B', 0, 0, 0, 0, 0, 6],
                (16, 2) => &[0x7f, 0xe0, 0, 0x10, b'O', b'W', 0, 0, 0, 0, 0, 12],
                (8, _) => &[0xe0, 0x7f, 0x10, 0, b'O', b'B', 0, 0, 6, 0, 0, 0],
                _ => &[0xe0, 0x7f, 0x10, 0, b'O', b'W', 0, 0, 12, 0, 0, 0],
            };
            assert!(bytes.windows(header.len()).any(|b| b == header));
            e.execute(&format!("I=dicominfo('u{bits}_{i}.dcm'); X=dicomread('u{bits}_{i}.dcm'); ts=I.TransferSyntaxUID; family=I.PatientName.FamilyName; id=I.PatientID; charset=I.SpecificCharacterSet; sop=I.SOPInstanceUID; msop=I.MediaStorageSOPInstanceUID;")).unwrap();
            assert_eq!(text(&e, "ts"), *syntax);
            assert_eq!(text(&e, "family"), "Synthétique");
            assert_eq!(text(&e, "id"), "TEST-ONLY");
            assert_eq!(text(&e, "charset"), "ISO_IR 192");
            assert_eq!(text(&e, "sop"), text(&e, "msop"));
            let x = e.array("X").unwrap();
            assert_eq!(x.class(), format!("uint{bits}"));
            assert_eq!((x.rows(), x.cols()), (2, 3));
            let want = if bits == 8 {
                [0, 128, 1, 42, 255, 17]
            } else {
                [0, 32768, 1, 42, 65535, 256]
            };
            for (k, v) in want.into_iter().enumerate() {
                assert_eq!(x.scalar(k % 2, k / 2).unwrap(), matrust::Scalar::Integer(v));
            }
        }
    }
    for i in 0..3 {
        let bytes = std::fs::read(dir.path().join(format!("odd_{i}.dcm"))).unwrap();
        assert!(bytes.ends_with(&[1, 255, 7, 0]));
        e.execute(&format!("I=dicominfo('odd_{i}.dcm'); X=dicomread('odd_{i}.dcm'); sop=I.SOPInstanceUID; study=I.StudyInstanceUID; series=I.SeriesInstanceUID;")).unwrap();
        assert_eq!(text(&e, "sop"), "2.25.123");
        assert_eq!(text(&e, "study"), "2.25.456");
        assert_eq!(text(&e, "series"), "2.25.789");
    }
}

#[test]
fn fresh_uuid_v4_and_supplied_pixel_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..3 {
        e.execute("rng(17); dicomwrite(uint16([0 65535]),\"same.dcm\",struct('Rows',1,'Columns',2,'SamplesPerPixel',1,'PhotometricInterpretation','MONOCHROME2','BitsAllocated',16,'BitsStored',16,'HighBit',15,'PixelRepresentation',0,'SpecificCharacterSet','ISO_IR 6','SOPClassUID','1.2.840.10008.5.1.4.1.1.7')); I=dicominfo('same.dcm'); a=I.SOPInstanceUID;b=I.StudyInstanceUID;c=I.SeriesInstanceUID;d=I.ImplementationClassUID;").unwrap();
        for name in ["a", "b", "c"] {
            let uid = text(&e, name);
            assert!(uid.len() <= 64);
            let n: u128 = uid.strip_prefix("2.25.").unwrap().parse().unwrap();
            assert_eq!((n >> 76) & 15, 4);
            assert_eq!((n >> 62) & 3, 2);
            assert!(seen.insert(uid));
        }
        assert_eq!(text(&e, "d"), "2.25.96426248368486820571789363904423688279");
        assert!(e.array("ans").is_none());
    }
}

#[test]
fn validation_and_outputs_never_publish_or_leave_temps() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    // Create mode drops recognized extras and accepts the dicominfo PN form.
    for source in [
        "dicomwrite(uint8(1),'out.dcm',struct('PatientName',struct('FamilyName','Synthetic')))",
        "dicomwrite(uint8(1),'out.dcm',struct('ReferencedStudySequence',struct()))",
        "dicomwrite(uint8(1),'out.dcm',struct('Private_0011_0010','secret'))",
        "dicomwrite(uint8(1),'out.dcm',struct('Filename','copy.dcm'))",
    ] {
        e.execute(source).unwrap();
        e.execute("I=dicominfo('out.dcm'); clean=~isfield(I,'ReferencedStudySequence')&&~isfield(I,'Private_0011_0010');").unwrap();
        assert!(logical(&e, "clean"));
        std::fs::remove_file(dir.path().join("out.dcm")).unwrap();
    }
    let failures = [
        "dicomwrite(uint8(1),'out.dcm',struct('uifigureGroupLength',1))",
        "dicomwrite(uint8(1),'out.dcm',struct('uifigure',1))",
        "dicomwrite()",
        "dicomwrite(uint8(1))",
        "dicomwrite(uint8(1),9)",
        "dicomwrite([1 2],'out.dcm')",
        "dicomwrite(int32(1),'out.dcm')",
        "dicomwrite(single(1),'out.dcm')",
        "dicomwrite(true,'out.dcm')",
        "dicomwrite(uint8([]),'out.dcm')",
        "dicomwrite(uint8(ones(2,2,2)),'out.dcm')",
        "dicomwrite(int8(ones(2,2,3)),'out.dcm')",
        "dicomwrite(uint8(1),'out.dcm',3)",
        "dicomwrite(uint8(1),'out.dcm',struct('PatientID',{'a','b'}))",
        "dicomwrite(uint8(1),'out.dcm','TransferSyntax')",
        "dicomwrite(uint8(1),'out.dcm','TransferSyntax',9)",
        "dicomwrite(uint8(1),'out.dcm','TransferSyntax','1.2.840.10008.1.2.4.50')",
        "dicomwrite(uint8(1),'out.dcm','TransferSyntax','unknown')",
        "dicomwrite(uint8(1),'out.dcm','TransferSyntax','1.2.840.10008.1.2','transfersyntax','1.2.840.10008.1.2')",
        "dicomwrite(uint8(1),'out.dcm','CreateMode','copy')",
        "dicomwrite(uint8(1),'out.dcm','CompressionMode','JPEG')",
        "dicomwrite(uint8(1),'out.dcm',struct('SOPClassUID','1.2.840.10008.5.1.4.1.1.2'))",
        "dicomwrite(uint8(1),'out.dcm',struct('Rows',2))",
        "dicomwrite(uint8(1),'out.dcm',struct('Columns',2))",
        "dicomwrite(uint8(1),'out.dcm',struct('BitsAllocated',16))",
        "dicomwrite(uint8(1),'out.dcm',struct('BitsStored',7))",
        "dicomwrite(uint8(1),'out.dcm',struct('HighBit',6))",
        "dicomwrite(uint8(1),'out.dcm',struct('PixelRepresentation',1))",
        "dicomwrite(uint8(1),'out.dcm',struct('SamplesPerPixel',3))",
        "dicomwrite(uint8(1),'out.dcm',struct('PhotometricInterpretation','RGB'))",
        "dicomwrite(uint8(1),'out.dcm',struct('NumberOfFrames',2))",
        "dicomwrite(uint8(1),'out.dcm',struct('PlanarConfiguration',0))",
        "dicomwrite(uint8(1),'out.dcm',struct('Rows',[1 1]))",
        "dicomwrite(uint8(1),'out.dcm',struct('Rows',NaN))",
        "dicomwrite(uint8(1),'out.dcm',struct('SOPInstanceUID','1.02.3'))",
        "dicomwrite(uint8(1),'out.dcm',struct('StudyInstanceUID','1..3'))",
        "dicomwrite(uint8(1),'out.dcm',struct('SeriesInstanceUID','2.25.1 '))",
        "dicomwrite(uint8(1),'out.dcm',struct('SOPInstanceUID',''))",
        "dicomwrite(uint8(1),'out.dcm',struct('PatientID',char([65 0 66])))",
        r"dicomwrite(uint8(1),'out.dcm',struct('PatientID','a\b'))",
        "dicomwrite(uint8(1),'out.dcm',struct('PatientID','padded '))",
        "dicomwrite(uint8(1),'out.dcm',struct('PatientID',repmat('x',1,65)))",
        "dicomwrite(uint8(1),'out.dcm',struct('PatientName','a^b^c^d^e^f'))",
        "dicomwrite(uint8(1),'out.dcm',struct('PatientName','a=b=c=d'))",
        "dicomwrite(uint8(1),'out.dcm',struct('StudyDate','20010229'))",
        "dicomwrite(uint8(1),'out.dcm',struct('StudyDate','2000.01.02'))",
        "dicomwrite(uint8(1),'out.dcm',struct('StudyTime','250001'))",
        "dicomwrite(uint8(1),'out.dcm',struct('StudyTime','12.5'))",
        "dicomwrite(uint8(1),'out.dcm',struct('Modality','lowercase'))",
        "dicomwrite(uint8(1),'out.dcm',struct('SeriesNumber',1.5))",
        "dicomwrite(uint8(1),'out.dcm',struct('InstanceNumber','2147483648'))",
        "dicomwrite(uint8(1),'out.dcm',struct('SpecificCharacterSet','ISO_IR 100'))",
        "dicomwrite(uint8(1),'out.dcm',struct('SpecificCharacterSet','ISO_IR 6','PatientName','Éva'))",
        "status=dicomwrite(uint8(1),'out.dcm')",
        "[a,b]=dicomwrite(uint8(1),'out.dcm')",
    ];
    for exists in [false, true] {
        for source in failures {
            let path = dir.path().join("out.dcm");
            if exists {
                std::fs::write(&path, b"SENTINEL").unwrap();
            }
            let err = e.execute(source).unwrap_err();
            assert!(!err.is_empty());
            if exists {
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    b"SENTINEL",
                    "{source}: {err}"
                );
            } else {
                assert!(!path.exists(), "{source}: {err}");
            }
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                usize::from(exists),
                "{source}"
            );
        }
    }
}

#[test]
fn dimension_memory_evaluation_and_cancellation_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.dcm");
    let mut e = folder(dir.path());
    e.execute("X=uint8(zeros(65536,1));").unwrap();
    for exists in [false, true] {
        if exists {
            std::fs::write(&path, b"SENTINEL").unwrap();
        }
        assert!(
            e.execute("dicomwrite(X,'out.dcm')")
                .unwrap_err()
                .contains("65535")
        );
        for limits in [
            matrust::engine::Limits {
                memory_bytes: e.workspace_bytes() + 1024,
                ..Default::default()
            },
            matrust::engine::Limits {
                evaluation_units: Some(32),
                ..Default::default()
            },
        ] {
            e.limits_set(limits);
            assert!(e.execute("dicomwrite(uint8(1),'out.dcm')").is_err());
        }
        e.limits_set(Default::default());
        let cancel = e.cancellation();
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            e.execute("dicomwrite(uint8(1),'out.dcm')")
                .unwrap_err()
                .to_lowercase()
                .contains("cancel")
        );
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        if exists {
            assert_eq!(std::fs::read(&path).unwrap(), b"SENTINEL");
        } else {
            assert!(!path.exists());
        }
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            usize::from(exists)
        );
    }
}

#[test]
#[ignore = "generate current Rust public-call files for independent pydicom inspection"]
fn export_for_pydicom() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dicom/write_current");
    std::fs::create_dir_all(&dir).unwrap();
    generate_outputs(&dir);
    generate_shapes(&dir);
    eprintln!(
        "Generated {} synthetic public-call DICOM files in {}",
        9 + SHAPES.len() * SYNTAXES.len(),
        dir.display()
    );
}

#[test]
#[ignore = "manual release-profile writer timings for 1 and N caller workers"]
fn writer_release_timing() {
    let workers: usize = std::env::var("DICOM_BENCH_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    pool.install(|| {
        let dir = tempfile::tempdir().unwrap();
        let mut e = folder(dir.path());
        for (label, x) in [
            (
                "uint8, 512x1025",
                "uint8(reshape(mod(0:524799,251),512,1025))",
            ),
            (
                "uint16, 512x1025",
                "uint16(reshape(mod(0:524799,251),512,1025))",
            ),
            (
                "int16, 512x1025",
                "int16(reshape(mod(0:524799,251)-125,512,1025))",
            ),
            (
                "uint8 RGB, 256x205x3x4",
                "uint8(reshape(mod(0:629759,251),256,205,3,4))",
            ),
            (
                "uint16 RGB, 256x205x3x4",
                "uint16(reshape(mod(0:629759,251),256,205,3,4))",
            ),
            (
                "int16 frames, 256x205x1x10",
                "int16(reshape(mod(0:524799,251)-125,256,205,1,10))",
            ),
        ] {
            e.execute(&format!("X={x}; dicomwrite(X,'bench.dcm');"))
                .unwrap();
            let start = std::time::Instant::now();
            for _ in 0..20 {
                e.execute("dicomwrite(X,'bench.dcm');").unwrap();
            }
            eprintln!(
                "DICOM write {label}, workers={workers}, iterations=20, bytes={}, elapsed_ms={:.3}",
                std::fs::metadata(dir.path().join("bench.dcm"))
                    .unwrap()
                    .len(),
                start.elapsed().as_secs_f64() * 1000.
            );
        }
    });
}

#[test]
fn output_handles_copy_mode_and_io_failures_preserve_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    e.execute("dicomwrite(uint8(1),'source.dcm'); I=dicominfo('source.dcm'); f=@dicomwrite;")
        .unwrap();
    e.execute("dicomwrite(uint8(1),'created.dcm',I); J=dicominfo('created.dcm'); X=dicomread('created.dcm'); same=isequal(X,uint8(1))&&strcmp(I.SOPInstanceUID,J.SOPInstanceUID);").unwrap();
    assert!(logical(&e, "same"));
    std::fs::remove_file(dir.path().join("created.dcm")).unwrap();
    let path = dir.path().join("out.dcm");
    for exists in [false, true] {
        if exists {
            std::fs::write(&path, b"sentinel").unwrap();
        }
        for source in [
            "x=f(uint8(1),'out.dcm')",
            "x=feval('dicomwrite',uint8(1),'out.dcm')",
        ] {
            assert!(e.execute(source).is_err(), "{source}");
            if exists {
                assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
            } else {
                assert!(!path.exists());
            }
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                1 + usize::from(exists)
            );
        }
    }
    let blocked = dir.path().join("directory.dcm");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("sentinel"), b"keep").unwrap();
    assert!(e.execute("dicomwrite(uint8(1),'directory.dcm')").is_err());
    assert_eq!(std::fs::read(blocked.join("sentinel")).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    assert!(e.execute("dicomwrite(uint8(1),'missing/out.dcm')").is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[test]
fn create_mode_accepts_full_info_and_drops_extras() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = folder(dir.path());
    for name in ["pixels_explicit_le.dcm", "write_current/u8_0.dcm"] {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/dicom")
            .join(name);
        e.execute(&format!(
            "I=dicominfo('{}'); X=dicomread('{}'); I.SliceThickness=struct('uifigure',1); dicomwrite(X,'created.dcm',I); J=dicominfo('created.dcm'); Y=dicomread('created.dcm'); same=isequal(X,Y)&&isequal(I.PatientName,J.PatientName)&&strcmp(I.PatientID,J.PatientID)&&~isfield(J,'SliceThickness');",
            input.display(), input.display()
        )).unwrap();
        assert!(logical(&e, "same"));
        let error = e
            .execute("I.uifigure=1; dicomwrite(X,'created.dcm',I);")
            .unwrap_err();
        assert!(error.contains("uifigure"), "{error}");
    }
}
