//! Public dictionary/session behavior and original synthetic private-tag fixtures.
use matrust::{Array, Engine};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dicom")
        .join(name)
}
fn quoted(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}
fn scalar(engine: &Engine, name: &str) -> f64 {
    engine
        .array(name)
        .unwrap()
        .scalar(0, 0)
        .unwrap()
        .into_real()
        .unwrap()
}
fn text(engine: &Engine, name: &str) -> String {
    let Array::Character(value) = engine.array(name).unwrap() else {
        panic!("expected character value {name}")
    };
    value.to_utf8(&AtomicBool::new(false)).unwrap()
}
fn set(engine: &mut Engine, path: &Path) {
    engine
        .execute(&format!("dicomdict('set','{}');", quoted(path)))
        .unwrap();
}

#[test]
fn dicom_lookup_types_reverse_repeaters_unknown_and_handles() {
    let mut e = Engine::default();
    e.execute("a=dicomlookup(16,16); b=dicomlookup(\"0010\",'0010'); c=dicomlookup(uint16(40),uint16(4)); [g,t]=dicomlookup(\"TransferSyntaxUID\"); f=@dicomlookup; d=f('6002','0010'); [rg,rt]=dicomlookup(d); z=dicomlookup('FFFF','FFFF'); [u,v]=dicomlookup('uifigure'); empty=[isempty(z) isempty(u) isempty(v)];").unwrap();
    assert_eq!(text(&e, "a"), "PatientName");
    assert_eq!(text(&e, "b"), "PatientName");
    assert_eq!(text(&e, "c"), "PhotometricInterpretation");
    assert_eq!((scalar(&e, "g"), scalar(&e, "t")), (2., 16.));
    assert_eq!(text(&e, "d"), "OverlayRows_2");
    assert_eq!((scalar(&e, "rg"), scalar(&e, "rt")), (24578., 16.));
    for i in 0..3 {
        assert_eq!(
            e.array("empty")
                .unwrap()
                .scalar(0, i)
                .unwrap()
                .into_real()
                .unwrap(),
            1.
        );
    }
    e.execute("z=dicomlookup('6001','0010'); missing=isempty(z);")
        .unwrap();
    assert_eq!(scalar(&e, "missing"), 1.);
    for call in [
        "dicomlookup(16,'0010')",
        "dicomlookup(-1,16)",
        "dicomlookup(65536,16)",
        "dicomlookup(1.5,16)",
        "dicomlookup([16 16],16)",
        "dicomlookup('001x','0010')",
        "dicomlookup('10000','0010')",
        "dicomlookup(true,16)",
    ] {
        assert!(e.execute(&format!("q={call};")).is_err(), "{call}");
    }
}

#[test]
fn dicom_dict_factory_file_round_trip() {
    let mut e = Engine::default();
    e.execute("p=dicomdict('get');").unwrap();
    let factory = PathBuf::from(text(&e, "p"));
    assert!(factory.is_file(), "{}", factory.display());
    let contents = std::fs::read_to_string(&factory).unwrap();
    let entries = contents
        .lines()
        .filter(|s| !s.trim().is_empty() && !s.trim_start().starts_with('#'))
        .count();
    assert_eq!(entries, 5299);
    set(&mut e, &factory);
    e.execute("a=dicomlookup('0010','0010'); r=dicomlookup('6002','0010');")
        .unwrap();
    assert_eq!(text(&e, "a"), "PatientName");
    assert_eq!(text(&e, "r"), "OverlayRows_2");
    e.execute("dicomdict('factory'); p=dicomdict('get');")
        .unwrap();
    assert_eq!(PathBuf::from(text(&e, "p")), factory);
}

#[test]
fn dicom_dict_state_isolation_reset_snapshot_and_explicit_actions() {
    let mut e = Engine::default();
    let custom = fixture("custom-dict.txt");
    set(&mut e, &custom);
    e.execute("p=dicomdict('get'); [g,t]=dicomlookup('PrivateCount'); a=dicomlookup('6001','0010'); b=dicomlookup('0010','0010'); missing=isempty(b);").unwrap();
    assert_eq!(
        PathBuf::from(text(&e, "p")).canonicalize().unwrap(),
        custom.canonicalize().unwrap()
    );
    assert_eq!((scalar(&e, "g"), scalar(&e, "t")), (17., 4097.));
    assert_eq!(text(&e, "a"), "CustomOverlayRows_1");
    assert_eq!(scalar(&e, "missing"), 1.);
    let mut other = Engine::default();
    other
        .execute("a=dicomlookup('0010','0010'); z=dicomlookup('0011','1001'); missing=isempty(z);")
        .unwrap();
    assert_eq!(text(&other, "a"), "PatientName");
    assert_eq!(scalar(&other, "missing"), 1.);
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("snapshot.txt");
    std::fs::copy(&custom, &snapshot).unwrap();
    set(&mut e, &snapshot);
    std::fs::write(&snapshot, "(0011,1001) US ChangedAfterSet 1\n").unwrap();
    e.execute("a=dicomlookup('0011','1001');").unwrap();
    assert_eq!(text(&e, "a"), "PrivateCount");
    e.execute("dicomdict('factory'); a=dicomlookup('0010','0010');")
        .unwrap();
    assert_eq!(text(&e, "a"), "PatientName");
    for action in ["get_current", "set_current", "factory_current", "uifigure"] {
        let err = e.execute(&format!("dicomdict('{action}');")).unwrap_err();
        assert!(
            err.contains(action) && err.contains("not supported yet"),
            "{err}"
        );
    }
}

#[test]
fn dicom_custom_private_info_and_pixels_both_syntaxes_and_per_call() {
    for name in ["custom_implicit.dcm", "custom_explicit.dcm"] {
        let file = fixture(name);
        let dict = fixture("custom-dict.txt");
        let mut e = Engine::default();
        e.execute("before=dicomlookup('0010','0010');").unwrap();
        let before = text(&e, "before");
        e.execute(&format!("i=dicominfo('{}','Dictionary','{}'); a=i.PrivateCount; b=i.PrivateLabel; x=dicomread('{}','dictionary','{}'); after=dicomlookup('0010','0010');",quoted(&file),quoted(&dict),quoted(&file),quoted(&dict))).unwrap();
        assert_eq!(scalar(&e, "a"), 513.);
        assert_eq!(e.array("a").unwrap().class(), "uint16");
        assert_eq!(text(&e, "b"), "PRIVATE");
        assert_eq!(text(&e, "after"), before);
        let x = e.array("x").unwrap();
        assert_eq!((x.rows(), x.cols()), (2, 3));
        assert_eq!(x.class(), "uint8");
        for (index, want) in [1., 4., 2., 5., 3., 6.].into_iter().enumerate() {
            assert_eq!(
                x.scalar(index % 2, index / 2).unwrap().into_real().unwrap(),
                want
            );
        }
        e.execute(&format!(
            "i=dicominfo('{}'); missing=isfield(i,'PrivateCount');",
            quoted(&file)
        ))
        .unwrap();
        assert_eq!(scalar(&e, "missing"), 0.);
        set(&mut e, &dict);
        e.execute(&format!(
            "i=dicominfo('{}'); a=i.PrivateCount; b=i.PrivateLabel; x=dicomread(i);",
            quoted(&file)
        ))
        .unwrap();
        assert_eq!(scalar(&e, "a"), 513.);
        assert_eq!(text(&e, "b"), "PRIVATE");
    }
}

#[test]
fn dicom_dictionary_untrusted_files_are_atomic_and_name_file_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    set(&mut e, &fixture("custom-dict.txt"));
    e.execute("before=dicomdict('get');").unwrap();
    let before = text(&e, "before");
    let good = b"# header\n(0011,1001) US PrivateCount 1\n";
    let cases: Vec<(&str, Vec<u8>, usize, &str)> = vec![
        (
            "tag.txt",
            b"# header\n(001,1001) US A 1\n".to_vec(),
            2,
            "tag",
        ),
        (
            "vr.txt",
            b"# header\n(0011,1001) ZZ A 1\n".to_vec(),
            2,
            "VR",
        ),
        (
            "keyword.txt",
            b"# header\n(0011,1001) US 9bad 1\n".to_vec(),
            2,
            "keyword",
        ),
        (
            "duplicate.txt",
            [good.as_slice(), b"(0011,1001) US Other 1\n"].concat(),
            3,
            "duplicate",
        ),
        ("oversize.txt", vec![b'#'; 16 * 1024 * 1024 + 1], 1, "limit"),
        ("binary.txt", vec![0, 255, 128, 1], 1, "text"),
        ("empty.txt", b"# no entries\n".to_vec(), 1, "empty"),
        ("dictionary.mat", b"MATLAB 5.0 MAT-file".to_vec(), 1, "MAT"),
        ("vm.txt", b"(0011,1001) US A nonsense\n".to_vec(), 1, "VM"),
        ("columns.txt", b"(0011,1001) US A\n".to_vec(), 1, "VM"),
        (
            "collision.txt",
            [good.as_slice(), b"(0011,1002) US PrivateCount 1\n"].concat(),
            3,
            "keyword",
        ),
    ];
    for (name, data, line, reason) in cases {
        let path = dir.path().join(name);
        std::fs::write(&path, data).unwrap();
        let err = e
            .execute(&format!("dicomdict('set','{}');", quoted(&path)))
            .unwrap_err();
        assert!(
            err.contains(name) && err.contains(&format!("line {line}")) && err.contains(reason),
            "{err}"
        );
        e.execute("after=dicomdict('get');").unwrap();
        assert_eq!(text(&e, "after"), before);
    }
    let directory = dir.path().join("directory.txt");
    std::fs::create_dir(&directory).unwrap();
    let err = e
        .execute(&format!("dicomdict('set','{}');", quoted(&directory)))
        .unwrap_err();
    assert!(
        err.contains("directory.txt") && err.contains("line 1") && err.contains("regular"),
        "{err}"
    );
}

#[test]
fn dicom_writer_rejects_custom_state_and_dictionary_option_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("unchanged.dcm");
    std::fs::write(&output, b"sentinel").unwrap();
    let mut e = Engine::default();
    let err = e
        .execute(&format!(
            "dicomwrite(uint8([1 2]),'{}','Dictionary','{}');",
            quoted(&output),
            quoted(&fixture("custom-dict.txt"))
        ))
        .unwrap_err();
    assert!(
        err.contains("Dictionary") && err.contains("not supported yet"),
        "{err}"
    );
    set(&mut e, &fixture("custom-dict.txt"));
    let err = e
        .execute(&format!("dicomwrite(uint8([1 2]),'{}');", quoted(&output)))
        .unwrap_err();
    assert!(
        err.contains("custom dictionary") && err.contains("not supported yet"),
        "{err}"
    );
    assert_eq!(std::fs::read(output).unwrap(), b"sentinel");
}

/// `custom_implicit.dcm` with (0008,0005) inserted first in the dataset. The
/// fixture dictionary omits that tag, but projection reads it in every dataset.
fn with_character_set(dir: &Path, name: &str, value: &[u8]) -> PathBuf {
    let base = std::fs::read(fixture("custom_implicit.dcm")).unwrap();
    let start = 144 + u32::from_le_bytes(base[140..144].try_into().unwrap()) as usize;
    let mut bytes = base[..start].to_vec();
    bytes.extend_from_slice(&[8, 0, 5, 0]);
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value);
    bytes.extend_from_slice(&base[start..]);
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn dicom_custom_dictionary_keeps_character_set_admission() {
    let dir = tempfile::tempdir().unwrap();
    let dict = quoted(&fixture("custom-dict.txt"));
    let call = |e: &mut Engine, function: &str, file: &Path| {
        e.execute(&format!(
            "v={function}('{}','Dictionary','{dict}');",
            quoted(file)
        ))
    };
    let padded = |length: usize| {
        let mut value = b"ISO_IR 192".to_vec();
        value.resize(length, b' ');
        value
    };
    let small = with_character_set(dir.path(), "small.dcm", &padded(10));
    let mut e = Engine::default();
    e.execute("missing=isempty(dicomlookup('0008','0005'));")
        .unwrap();
    assert_eq!(scalar(&e, "missing"), 0.);
    call(&mut e, "dicominfo", &small).unwrap();
    e.execute("a=v.PrivateCount; named=isfield(v,'SpecificCharacterSet');")
        .unwrap();
    assert_eq!((scalar(&e, "a"), scalar(&e, "named")), (513., 0.));
    call(&mut e, "dicomread", &small).unwrap();
    assert_eq!(scalar(&e, "v"), 1.);
    // Read although unnamed: an unsupported character set is still an error.
    let latin = with_character_set(dir.path(), "latin.dcm", b"ISO_IR 100");
    let err = call(&mut e, "dicominfo", &latin).unwrap_err();
    assert!(err.contains("character set"), "{err}");
    // Over the 1 MiB value cap: rejected while parsing, so dicomread fails too.
    let over = with_character_set(dir.path(), "over.dcm", &padded(1024 * 1024 + 2));
    for function in ["dicominfo", "dicomread"] {
        let err = call(&mut e, function, &over).unwrap_err();
        assert!(err.contains("1048576-byte limit"), "{function}: {err}");
    }
    // 64 KiB of text is charged 16x = 1 MiB: fine by default, over a 512 KiB engine.
    let large = with_character_set(dir.path(), "large.dcm", &padded(65536));
    call(&mut e, "dicominfo", &large).unwrap();
    e.execute("a=v.PrivateCount;").unwrap();
    assert_eq!(scalar(&e, "a"), 513.);
    let mut limited = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 512 * 1024,
        ..Default::default()
    });
    for function in ["dicominfo", "dicomread"] {
        call(&mut limited, function, &small).unwrap();
        let err = call(&mut limited, function, &large).unwrap_err();
        assert!(err.contains("budget"), "{function}: {err}");
    }
}

#[test]
#[cfg(all(unix, not(target_os = "macos")))]
fn no_per_user_cache_directory_errors() {
    const CHILD: &str = "MATRUST_DICOM_CACHE_REGRESSION";
    if std::env::var_os(CHILD).is_some() {
        let error = Engine::default()
            .execute("p=dicomdict('get');")
            .unwrap_err();
        assert!(error.contains("per-user cache"), "{error}");
        return;
    }
    let cache_dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("dicom_dict_custom::no_per_user_cache_directory_errors")
        .env(CHILD, "1")
        .env("TMPDIR", cache_dir.path())
        .env_remove("HOME")
        .env_remove("XDG_CACHE_HOME")
        .status()
        .unwrap();
    assert!(status.success(), "missing-cache child failed: {status}");
}
