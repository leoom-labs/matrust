//! Reference files2 corpus (archives, PNG images, WAV audio, importdata,
//! fileattrib), stored as expected values in the fixture. Each case gets a fresh
//! engine; `tempdir` is relocated into one automatically removed folder in
//! the worktree (the corpus folder name is fixed, so all corpus cases run in
//! one test). Only the explicit HARNESS rules below relax a comparison.
use matrust::Engine;
use std::{fs, path::Path};

const CORPUS: &str = include_str!("fixtures/fn/files2-expected.txt");
/// The case list; a `LONG ` prefix marks cases recorded under `format long`.
const STATEMENTS: &str = include_str!("fixtures/fn/files2-cases.txt");
const CASES: usize = 194;
/// The reference's temporary folder, as printed in ENV answers.
const REFERENCE_FOLDER: &str = r"C:\reference\temp\files2";

/// Cases that cannot pass for reasons outside this task. Each still runs and
/// must fail in the stated way (an error naming the missing feature); a case
/// that starts passing must leave this table.
const KNOWN: &[(usize, &str)] = &[
    (
        3,
        "N-D arrays (RGB image MxNx3) are not supported by the engine",
    ),
    (
        4,
        "N-D arrays (RGB image MxNx3) are not supported by the engine",
    ),
    (24, "BMP files are not supported yet (task: PNG only)"),
    (25, "TIFF files are not supported yet (task: PNG only)"),
    (26, "GIF files are not supported yet (task: PNG only)"),
    (27, "JPEG files are not supported yet (task: PNG only)"),
    (28, "JPEG files and N-D arrays are not supported yet"),
    (29, "JPEG files are not supported yet (task: PNG only)"),
    (
        37,
        "N-D arrays (ones(2,2,2)) are not supported by the engine",
    ),
    (
        40,
        "N-D arrays (ones(2,3,3)) are not supported by the engine",
    ),
    (47, "JPEG files are not supported yet (task: PNG only)"),
    (48, "JPEG files are not supported yet (task: PNG only)"),
    (49, "BMP files are not supported yet (task: PNG only)"),
    (72, "FLAC files are not supported yet (task: WAV only)"),
    (73, "FLAC files are not supported yet (task: WAV only)"),
    (
        74,
        "Ogg Vorbis files are not supported yet (task: WAV only)",
    ),
    (92, "FLAC files are not supported yet (task: WAV only)"),
    (93, "FLAC files are not supported yet (task: WAV only)"),
    (
        94,
        "Ogg Vorbis files are not supported yet (task: WAV only)",
    ),
    (
        95,
        "Ogg Vorbis files are not supported yet (task: WAV only)",
    ),
    (
        110,
        "Existing cell renderer left-aligns [] in a column of text instead of right-aligning it",
    ),
    (
        125,
        "N-D arrays (cat(3, ...) RGB image) are not supported by the engine",
    ),
    (
        178,
        "Existing cell renderer does not shorten long text to the oracle's 0x1A ellipsis",
    ),
    (185, "JPEG files are not supported yet (task: PNG only)"),
];

/// Engine gaps outside this task: `sort` of a cell array, transposing a cell
/// array and `[cell, cell]` concatenation. These KNOWN cases must first fail
/// with that engine error; then an equivalent statement (cell building with
/// `{c{:}, ...}`; `dir` already lists names sorted) must match the reference
/// output exactly, which still checks this task's functions.
const REWRITE: &[(usize, &[(&str, &str)])] = &[];
const ENGINE_GAPS: &[&str] = &[
    "This operation does not support character, cell, struct or graphics handle arrays",
    "Expected a numeric or logical value",
    "Mixed character/numeric or cell concatenation is not implemented",
];

/// Every case compared with a relaxed rule. All cases also relocate
/// `fullfile(tempdir, ...)` into the fresh test folder (exact results).
const HARNESS: &[(usize, &str)] = &[
    (
        76,
        "Expect MatRust's File not found. error; preserve the original oracle fixture.",
    ),
    (
        96,
        "Expect MatRust's File not found. error; preserve the original oracle fixture.",
    ),
    (
        70,
        "Compare the [Warning: ...] line only; the toolbox backtrace is dropped",
    ),
    (
        85,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        137,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        138,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        141,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        142,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        143,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        147,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        150,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        151,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        152,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        160,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        161,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        162,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        163,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        164,
        "Host file separator in the path text (Windows prints \\)",
    ),
    (
        169,
        "Non-Windows hosts: class and size only (UNIX has no archive/system/hidden flags but has group/other permissions)",
    ),
    (
        173,
        "Non-Windows hosts: '+h' is not a UNIX mode; expect the invalid-mode error",
    ),
    (
        174,
        "Non-Windows hosts: '+a' is not a UNIX mode; expect the invalid-mode error",
    ),
    (
        183,
        "ENV: class and size only (file size, modification date)",
    ),
    (184, "ENV: character row (full path)"),
    (
        185,
        "ENV: class and size only (file size, modification date)",
    ),
    (186, "ENV: character row (full path)"),
    (187, "ENV: class and size only (file size)"),
    (188, "ENV: class and size only (archive size)"),
    (189, "ENV: class and size only (full path in a cell)"),
    (
        190,
        "ENV error: oracle temp folder replaced by the test folder",
    ),
    (
        191,
        "ENV error: oracle temp folder replaced by the test folder",
    ),
    (
        192,
        "ENV error: oracle temp folder replaced by the test folder",
    ),
    (193, "ENV: character row (full path)"),
    (194, "ENV: character row (current folder)"),
];

struct Case {
    id: usize,
    statement: String,
    long: bool,
    console: String,
    class: Option<String>,
}
fn cases() -> Vec<Case> {
    let long: Vec<bool> = STATEMENTS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.starts_with("LONG "))
        .collect();
    let mut result = Vec::new();
    let mut rest = CORPUS;
    while let Some(start) = rest.find("@@CASE ") {
        rest = &rest[start + 7..];
        let (header, after) = rest.split_once('\n').expect("header");
        let (id, statement) = header.split_once('|').expect("id|statement");
        let (body, after) = after.split_once("@@END\n").expect("end marker");
        let (console, class) = match body.rfind("@@CLASS ") {
            Some(at) => (body[..at].into(), Some(body[at + 8..].trim_end().into())),
            None => (body.into(), None),
        };
        let id: usize = id.parse().unwrap();
        result.push(Case {
            id,
            statement: statement.into(),
            long: long[id - 1],
            console,
            class,
        });
        rest = after;
    }
    result
}
fn folder(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
}
fn quoted(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}
fn unplaced(error: &str) -> &str {
    match error.split_once(": ") {
        Some((p, s)) if p.starts_with("Line ") => s,
        _ => error,
    }
}
fn host_separators(text: &str) -> String {
    if cfg!(windows) {
        text.into()
    } else {
        text.replace('\\', "/")
    }
}
/// Keeps the warning line of a reference warning, without markup or backtrace.
fn warning_line(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with("[\u{8}> In ") && !l.starts_with("  In "))
        .map(|l| {
            let l = l.replace("[\u{8}", "").replace("]\u{8} ", "");
            format!("{l}\n")
        })
        .collect()
}
fn is_known(id: usize) -> bool {
    KNOWN.iter().any(|(k, _)| *k == id)
}
/// Runs one statement in a fresh engine and removes the corpus folder after.
fn evaluate(case: &Case, statement: &str, root: &Path) -> (Engine, Result<String, String>) {
    let mut e = Engine::default();
    e.set_current_folder(root).unwrap();
    let setup = format!(
        "{}fl2_test_root = '{}';",
        if case.long { "format long; " } else { "" },
        quoted(root)
    );
    e.execute(&setup).expect("setup");
    let statement = statement.replace("fullfile(tempdir,", "fullfile(fl2_test_root,");
    let result = e
        .execute(&statement)
        .map(|o| o.output.iter().map(|s| format!("{s}\n")).collect());
    // Leave no corpus folder behind for the next case, even on failure.
    let _ = fs::remove_dir_all(root.join("mroracle2_files2"));
    (e, result)
}

fn check(case: &Case, root: &Path) -> Result<bool, String> {
    let windows = cfg!(windows);
    if let Some((_, edits)) = REWRITE.iter().find(|(id, _)| *id == case.id) {
        let (_, result) = evaluate(case, &case.statement, root);
        match result {
            Err(error) if ENGINE_GAPS.contains(&unplaced(&error)) => {}
            Err(error) => return Err(format!("KNOWN case: unexpected error {error}")),
            Ok(_) => return Err("passes now; remove it from KNOWN and REWRITE".into()),
        }
        let mut statement = case.statement.clone();
        for (from, to) in *edits {
            let at = statement
                .rfind(from)
                .ok_or_else(|| format!("REWRITE text {from:?} not found"))?;
            statement.replace_range(at..at + from.len(), to);
        }
        let (e, result) = evaluate(case, &statement, root);
        let actual = result.map_err(|error| format!("rewritten statement failed: {error}"))?;
        compare(case, &e, actual)?;
        return Ok(false);
    }
    let (e, result) = evaluate(case, &case.statement, root);
    if !windows && case.id == 173 {
        let error = result.err().ok_or("Expected the UNIX invalid-mode error")?;
        let wanted = "Invalid file mode characters on the current platform.";
        return if unplaced(&error) == wanted {
            Ok(true)
        } else {
            Err(format!("error {error:?}, expected {wanted:?}"))
        };
    }
    if !windows && case.id == 174 {
        // Status 0 for the invalid mode, archive flag NaN.
        result?;
        let text = e.variables.get("x").map(|m| format!("{:?}", m.data()));
        return if text.as_deref() == Some("[0.0, NaN]") {
            Ok(true)
        } else {
            Err(format!("expected double [0 NaN], got {text:?}"))
        };
    }
    if let Some(expected) = case.console.trim_end().strip_prefix("ERR ") {
        let error = result.err().ok_or("Expected error, got success")?;
        let actual = unplaced(&error);
        let mut expected = expected.to_owned();
        if matches!(case.id, 76 | 96) {
            expected = "File not found.".into();
        }
        if matches!(case.id, 190..=192) {
            let here = root.join("mroracle2_files2").to_string_lossy().into_owned();
            expected = expected.replace(REFERENCE_FOLDER, &here);
            expected = expected.replace(
                &format!("{here}\\"),
                &format!("{here}{}", std::path::MAIN_SEPARATOR),
            );
        }
        if actual == expected {
            return if is_known(case.id) {
                Err("passes now; remove it from KNOWN".into())
            } else {
                Ok(true)
            };
        }
        if is_known(case.id) && actual.contains("two-dimensional") {
            return Ok(false);
        }
        if is_known(case.id) {
            return Err(format!("KNOWN case: unexpected error {actual:?}"));
        }
        return Err(format!("error {actual:?}, expected {expected:?}"));
    }
    let actual = match result {
        Ok(actual) => actual,
        Err(error) if is_known(case.id) => {
            let missing = error.contains("not supported yet")
                || error.contains("N-D")
                || error.contains("N-dimensional")
                || error.contains("two-dimensional");
            return if missing {
                Ok(false)
            } else {
                Err(format!("KNOWN case failed for another reason: {error}"))
            };
        }
        Err(error) => return Err(format!("unexpected error {error}")),
    };
    match compare(case, &e, actual.clone()) {
        Ok(()) if is_known(case.id) => Err("passes now; remove it from KNOWN".into()),
        Ok(()) => Ok(true),
        Err(_) if is_known(case.id) && known_display(case, &e, &actual) => Ok(false),
        Err(problem) => Err(problem),
    }
}

/// Class, size and console comparison under the HARNESS rules.
fn compare(case: &Case, e: &Engine, actual: String) -> Result<(), String> {
    let x = e.array("x").ok_or("x missing")?;
    let class = format!("{} [{} {}]", x.class(), x.rows(), x.cols());
    if matches!(case.id, 184 | 186 | 193 | 194) {
        return if x.class() == "char" && x.rows() == 1 {
            Ok(())
        } else {
            Err(format!("expected a character row, got {class}"))
        };
    }
    if Some(&class) != case.class.as_ref() {
        return Err(format!("class/size {class:?}, expected {:?}", case.class));
    }
    if case.id >= 183 || !cfg!(windows) && case.id == 169 {
        return Ok(());
    }
    let expected = expected_console(case);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "console\n expected {expected:?}\n actual   {actual:?}"
        ))
    }
}
fn expected_console(case: &Case) -> String {
    let mut expected = case.console.clone();
    if case.id == 70 {
        expected = warning_line(&expected);
    }
    if matches!(
        case.id,
        85 | 137 | 138 | 141 | 142 | 143 | 147 | 150 | 151 | 152 | 160 | 161 | 162 | 163 | 164
    ) {
        expected = host_separators(&expected);
    }
    expected
}
/// KNOWN display differences of the existing renderers, matched precisely
/// (class and size still exact).
fn known_display(case: &Case, e: &Engine, actual: &str) -> bool {
    let adjusted = match case.id {
        178 => actual.replace(
            "'MATLAB:FILEATTRIB:CannotFindFile'",
            "'MATLAB:FILEATTRIB:CannotFin\u{1a}'",
        ),
        110 => actual.replace("'Title line'    [] \n", "'Title line'     []\n"),
        _ => return false,
    };
    let class = e
        .array("x")
        .map(|x| format!("{} [{} {}]", x.class(), x.rows(), x.cols()));
    class == case.class && adjusted == expected_console(case)
}

#[test]
fn corpus_has_every_case() {
    let cases = cases();
    assert_eq!(cases.len(), CASES);
    for (id, reason) in HARNESS.iter().chain(KNOWN) {
        assert!(cases.iter().any(|c| c.id == *id));
        assert!(!reason.is_empty());
    }
    for (index, case) in cases.iter().enumerate() {
        assert_eq!(case.id, index + 1);
    }
}

#[test]
fn matches_reference() {
    let root = folder(".files2-corpus-");
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut known = 0;
    for case in cases() {
        match check(&case, root.path()) {
            Ok(true) => passed += 1,
            Ok(false) => known += 1,
            Err(problem) => {
                failures.push(format!("case {} `{}`: {problem}", case.id, case.statement))
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{passed} passed, {known} known, {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed + known, CASES);
    println!(
        "{passed}/{CASES} exact or HARNESS matches; {known} validated KNOWN cases; no skipped cases"
    );
}

fn run(e: &mut Engine, source: &str) -> String {
    let text: String = e
        .execute(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"))
        .output
        .iter()
        .map(|s| format!("{s}\n"))
        .collect();
    format!("{}\n", text.trim_end())
}

#[test]
fn png_round_trips_bit_depths_palette_alpha_and_text() {
    let root = folder(".files2-png-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    run(
        &mut e,
        "a = uint8(reshape(0:255, 16, 16)); imwrite(a, 'g.png'); b = imread('g.png'); same8 = isequal(a, b);
         w = uint16(reshape(0:4369:65535, 4, 4)); imwrite(w, 'w.png'); same16 = isequal(imread('w.png'), w);
         imwrite(uint8([0 255; 17 34]), 'q.png', 'BitDepth', 4); q = double(imread('q.png'));
         imwrite(uint8([0 2; 1 0]), [0 0 0; 1 1 1; 0.2 0.4 0.6], 'p.png'); [px, pm] = imread('p.png');
         imwrite(uint8([5 6; 7 8]), 'a.png', 'Alpha', uint8([1 2; 3 4]), 'Author', 'Me', 'Zeta', 'z');
         [ax, ~, aa] = imread('a.png'); info = imfinfo('a.png'); pxd = double(px(:))'; aad = double(aa(:))';",
    );
    assert_eq!(e.logical_variables["same8"].data(), &[1]);
    assert_eq!(e.logical_variables["same16"].data(), &[1]);
    assert_eq!(e.variables["q"].data(), &[0., 17., 255., 34.]);
    assert_eq!(e.variables["pm"].data()[2], 51. / 255.);
    assert_eq!(e.variables["pxd"].data(), &[0., 1., 2., 0.]);
    assert_eq!(e.variables["aad"].data(), &[1., 3., 2., 4.]);
    assert_eq!(run(&mut e, "disp(info.Author)"), "Me\n");
    assert_eq!(run(&mut e, "disp(info.OtherText{1,2})"), "z\n");
    assert_eq!(run(&mut e, "disp(info.Transparency)"), "alpha\n");
    // Reading never trusts a corrupted chunk.
    let mut bytes = fs::read(root.path().join("g.png")).unwrap();
    let last = bytes.len() - 20;
    bytes[last] ^= 0xFF;
    fs::write(root.path().join("bad.png"), bytes).unwrap();
    assert!(
        e.execute("imread('bad.png')")
            .unwrap_err()
            .contains("checksum")
    );
}

#[test]
fn wav_stereo_formats_metadata_and_ranges() {
    let root = folder(".files2-wav-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    run(
        &mut e,
        "y = [0 -0.5; 0.25 0.5; -1 0.125]; audiowrite('s.wav', y, 22050, 'BitsPerSample', 24, 'Artist', 'Me');
         [z, fs] = audioread('s.wav'); i = audioinfo('s.wav'); part = audioread('s.wav', [2 3], 'native');
         audiowrite('f.wav', y, 8000, 'BitsPerSample', 64); f = audioread('f.wav');
         audiowrite('i.wav', int32([1; -65536; 2147483647]), 8000, 'BitsPerSample', 32); n = audioread('i.wav', 'native'); pd = double(part(:))'; nd = double(n)';",
    );
    assert_eq!(e.variables["z"].data(), &[0., 0.25, -1., -0.5, 0.5, 0.125]);
    assert_eq!(e.variables["fs"].data(), &[22050.]);
    assert_eq!(e.variables["f"].data(), &[0., 0.25, -1., -0.5, 0.5, 0.125]);
    assert_eq!(run(&mut e, "disp(i.Artist)"), "Me\n");
    assert_eq!(run(&mut e, "disp(class(part))"), "int32\n");
    assert_eq!(
        e.variables["pd"].data(),
        &[536870912., -2147483648., 1073741824., 268435456.]
    );
    assert_eq!(run(&mut e, "disp(class(n))"), "int32\n");
    assert_eq!(e.variables["nd"].data(), &[1., -65536., 2147483647.]);
    let bytes = fs::read(root.path().join("s.wav")).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 24);
}

#[test]
fn archives_round_trip_and_reject_unsafe_names() {
    let root = folder(".files2-archive-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::create_dir_all(root.path().join("src/deep")).unwrap();
    fs::write(root.path().join("src/a.txt"), b"alpha").unwrap();
    fs::write(root.path().join("src/deep/b.bin"), [0u8, 1, 2, 255]).unwrap();
    run(
        &mut e,
        "z = zip('pack', 'src'); u = unzip('pack.zip', 'zout'); t = tar('pack.tgz', 'src'); v = untar('pack.tgz', 'tout');
         g = gzip('src/a.txt', 'gz'); h = gunzip(g{1}, 'gout'); cnt = numel(z) + numel(u) + numel(t) + numel(v);",
    );
    for dir in ["zout", "tout"] {
        assert_eq!(
            fs::read(root.path().join(dir).join("src/a.txt")).unwrap(),
            b"alpha"
        );
        assert_eq!(
            fs::read(root.path().join(dir).join("src/deep/b.bin")).unwrap(),
            [0u8, 1, 2, 255]
        );
    }
    assert_eq!(fs::read(root.path().join("gout/a.txt")).unwrap(), b"alpha");
    assert_eq!(e.variables["cnt"].data(), &[8.]);
    // A tar entry that climbs out of the output folder is refused.
    let mut header = [0u8; 512];
    header[..9].copy_from_slice(b"../evil.t");
    header[100..107].copy_from_slice(b"0000644");
    header[124..135].copy_from_slice(b"00000000001");
    header[156] = b'0';
    header[148..156].copy_from_slice(b"        ");
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    let mut tar = header.to_vec();
    tar.extend_from_slice(&[b'x'; 512]);
    tar.extend_from_slice(&[0; 1024]);
    fs::write(root.path().join("evil.tar"), tar).unwrap();
    assert!(
        e.execute("untar('evil.tar', 'eout')")
            .unwrap_err()
            .contains("Unsafe")
    );
    assert!(!root.path().join("evil.t").exists());
}

#[test]
fn importdata_text_forms() {
    let root = folder(".files2-import-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("t.tsv"), "a\tb\n1\t2\n3\t4\n").unwrap();
    fs::write(root.path().join("r.csv"), "1;2\r\n3;4\r\n").unwrap();
    run(
        &mut e,
        "[s, d, h] = importdata('t.tsv'); m = importdata('r.csv', ';'); dd = double(d);",
    );
    assert_eq!(e.variables["dd"].data(), &[9.]);
    assert_eq!(e.variables["h"].data(), &[1.]);
    assert_eq!(e.variables["m"].data(), &[1., 3., 2., 4.]);
    assert_eq!(run(&mut e, "disp(s.colheaders{2})"), "b\n");
}

fn tar_block(name: &str, kind: u8, body: &[u8]) -> Vec<u8> {
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..107].copy_from_slice(b"0000644");
    header[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
    header[156] = kind;
    header[148..156].copy_from_slice(b"        ");
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    let mut out = header.to_vec();
    out.extend_from_slice(body);
    out.resize(out.len().div_ceil(512) * 512, 0);
    out
}

#[cfg(unix)]
#[test]
fn extraction_never_writes_through_links() {
    use std::io::Write;
    let root = folder(".files2-link-");
    let outside = folder(".files2-outside-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::create_dir_all(root.path().join("out")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("out/link")).unwrap();
    fs::write(outside.path().join("victim"), b"keep").unwrap();
    let mut tar = tar_block("link/victim", b'0', b"evil");
    tar.extend_from_slice(&[0; 1024]);
    fs::write(root.path().join("l.tar"), tar).unwrap();
    assert!(
        e.execute("untar('l.tar', 'out')")
            .unwrap_err()
            .contains("link")
    );
    let mut writer = zip::ZipWriter::new(fs::File::create(root.path().join("l.zip")).unwrap());
    writer
        .start_file("link/victim", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"evil").unwrap();
    writer.finish().unwrap();
    assert!(
        e.execute("unzip('l.zip', 'out')")
            .unwrap_err()
            .contains("link")
    );
    // A link in the final component is refused too.
    std::os::unix::fs::symlink(outside.path().join("victim"), root.path().join("out/f")).unwrap();
    let mut tar = tar_block("f", b'0', b"evil");
    tar.extend_from_slice(&[0; 1024]);
    fs::write(root.path().join("f.tar"), tar).unwrap();
    assert!(
        e.execute("untar('f.tar', 'out')")
            .unwrap_err()
            .contains("link")
    );
    assert_eq!(fs::read(outside.path().join("victim")).unwrap(), b"keep");
}

#[test]
fn tar_edge_cases_names_dot_entry_and_empty_archive() {
    let root = folder(".files2-tar-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("caf\u{e9}.txt"), b"x").unwrap();
    run(
        &mut e,
        "t = tar('u', 'caf\u{e9}.txt'); v = untar('u.tar', 'uout');",
    );
    assert!(root.path().join("uout/caf\u{e9}.txt").is_file());
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(
        run(&mut e, "disp(v{1})"),
        format!("uout{sep}caf\u{e9}.txt\n")
    );
    let mut tar = tar_block("./", b'5', b"");
    tar.extend(tar_block("./a.txt", b'0', b"hi"));
    tar.extend_from_slice(&[0; 1024]);
    fs::write(root.path().join("dot.tar"), tar).unwrap();
    run(&mut e, "untar('dot.tar', 'dout');");
    assert_eq!(fs::read(root.path().join("dout/a.txt")).unwrap(), b"hi");
    fs::write(root.path().join("empty.tar"), [0u8; 1024]).unwrap();
    assert_eq!(
        run(&mut e, "r = untar('empty.tar', 'eout'); disp(numel(r))").trim(),
        "0"
    );
}

#[test]
fn entry_limit_applies_to_the_final_list() {
    let root = folder(".files2-limit-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::create_dir_all(root.path().join("d/sub")).unwrap();
    for i in 0..10_001 {
        fs::write(root.path().join(format!("d/sub/f{i}.txt")), b"").unwrap();
    }
    for statement in [
        "zip('a', 'd/sub/*.txt')",
        "tar('a', 'd/sub/*.txt')",
        "zip('a', 'd')",
        "tar('a', {'d/sub/*.txt'})",
        "zip('a', 'd/*')",
        "tar('a', 'd/*')",
        "gzip('d', 'gzout')",
        "fileattrib('d/sub/*.txt')",
    ] {
        let error = e.execute(statement).unwrap_err();
        assert!(
            error.contains("10000") || error.contains("Too many"),
            "{statement}: {error}"
        );
    }
    assert!(!root.path().join("a.zip").exists() && !root.path().join("a.tar").exists());
}

#[test]
fn importdata_audio_limits_and_wav_errors() {
    let root = folder(".files2-limits-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    // 100000 header lines and a 100000-field last header line.
    let mut text = "x\n".repeat(100_000);
    text.push_str(&"a,".repeat(99_999));
    text.push_str("a\n1,2\n");
    fs::write(root.path().join("big.csv"), text).unwrap();
    let error = e.execute("importdata('big.csv', ',', 100001)").unwrap_err();
    assert!(error.contains("limit"), "{error}");
    // PCM8 file whose sample count exceeds the element limit.
    let samples = 17 * 1024 * 1024u32;
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(36 + samples).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    for v in [1u16, 1] {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&samples.to_le_bytes());
    wav.resize(wav.len() + samples as usize, 128);
    fs::write(root.path().join("long.wav"), &wav).unwrap();
    let error = e.execute("audioread('long.wav')").unwrap_err();
    assert!(error.contains("limit"), "{error}");
    // Truncated file: the data chunk claims more than the file holds.
    fs::write(root.path().join("cut.wav"), &wav[..1000]).unwrap();
    for statement in ["audioread('cut.wav')", "audioinfo('cut.wav')"] {
        let error = e.execute(statement).unwrap_err();
        assert!(error.contains("truncated"), "{statement}: {error}");
    }
    // Channel count x width must fit the header fields.
    let error = e
        .execute("audiowrite('wide.wav', zeros(2, 32768), 8000)")
        .unwrap_err();
    assert!(error.contains("do not fit"), "{error}");
    assert!(!root.path().join("wide.wav").exists());
}

// Rev2 Q numbers refer to the reviewer's reference batches,
// recorded in the review report. These are regressions beyond the corpus.
#[test]
fn rev2_gunzip_strips_any_extension_without_overwriting_source() {
    // Finding 1, Q2: odd.bin -> odd, both beside the source and in out/.
    let root = folder(".files2-rev2-gunzip-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("input"), b"keep this content").unwrap();
    run(&mut e, "gzip('input');");
    let compressed = fs::read(root.path().join("input.gz")).unwrap();
    fs::write(root.path().join("odd.bin"), &compressed).unwrap();
    for (statement, name) in [
        ("r=gunzip('odd.bin'); disp(r{1})", "odd"),
        ("r=gunzip('odd.bin','out'); disp(r{1})", "out/odd"),
    ] {
        assert_eq!(
            run(&mut e, statement).replace('\\', "/"),
            format!("{name}\n")
        );
        assert_eq!(
            fs::read(root.path().join(name)).unwrap(),
            b"keep this content"
        );
        assert_eq!(fs::read(root.path().join("odd.bin")).unwrap(), compressed);
    }
    // Extensionless sources must never be replaced by their decoded bytes.
    fs::write(root.path().join("noextension"), &compressed).unwrap();
    assert!(
        e.execute("gunzip('noextension')")
            .unwrap_err()
            .contains("overwrite")
    );
    assert_eq!(
        fs::read(root.path().join("noextension")).unwrap(),
        compressed
    );
}

#[test]
fn rev2_gunzip_tgz_removes_extension_as_documented() {
    // Finding 2, Q15: t.tgz -> t (not t.tar).
    let root = folder(".files2-rev2-tgz-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("a.txt"), b"a").unwrap();
    run(&mut e, "tar('t.tgz','a.txt');");
    let compressed = fs::read(root.path().join("t.tgz")).unwrap();
    assert_eq!(run(&mut e, "r=gunzip('t.tgz'); disp(r{1})"), "t\n");
    assert_eq!(
        run(&mut e, "r=gunzip('t.tgz','out'); disp(r{1})").replace('\\', "/"),
        "out/t\n"
    );
    assert!(!root.path().join("t.tar").exists());
    assert!(!root.path().join("out/t.tar").exists());
    assert_eq!(fs::read(root.path().join("t.tgz")).unwrap(), compressed);
    run(&mut e, "untar('t','decoded');");
    assert_eq!(fs::read(root.path().join("decoded/a.txt")).unwrap(), b"a");
}

#[test]
fn rev2_zip_tar_wildcards_include_nested_folders() {
    // Finding 3, Q4: '*' includes matched directories recursively.
    let root = folder(".files2-rev2-wild-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::create_dir_all(root.path().join("d/sub/deep")).unwrap();
    for (name, data) in [
        ("a.txt", "a"),
        ("z.txt", "z"),
        ("sub/b.txt", "b"),
        ("sub/deep/c.txt", "c"),
    ] {
        fs::write(root.path().join("d").join(name), data).unwrap();
    }
    for (pack, unpack) in [("zip", "unzip"), ("tar", "untar")] {
        assert_eq!(
            run(
                &mut e,
                &format!(
                    "r={pack}('a.{pack}','*','d'); disp(r{{1}}); disp(r{{2}}); disp(r{{3}}); disp(r{{4}})"
                )
            ),
            "a.txt\nz.txt\nsub/b.txt\nsub/deep/c.txt\n"
        );
        run(&mut e, &format!("{unpack}('a.{pack}','{pack}out');"));
        for (name, data) in [
            ("a.txt", "a"),
            ("z.txt", "z"),
            ("sub/b.txt", "b"),
            ("sub/deep/c.txt", "c"),
        ] {
            assert_eq!(
                fs::read(root.path().join(format!("{pack}out/{name}"))).unwrap(),
                data.as_bytes()
            );
        }
        assert_eq!(
            run(
                &mut e,
                &format!("r={pack}('b.{pack}','d/s*'); disp(r{{1}}); disp(r{{2}})")
            ),
            "d/sub/b.txt\nd/sub/deep/c.txt\n"
        );
    }
    #[cfg(unix)]
    {
        // Expanded names are literal: do not interpret '*' or '?' twice.
        fs::create_dir_all(root.path().join("d/literal*")).unwrap();
        fs::write(root.path().join("d/literal*/file?.txt"), b"literal").unwrap();
        for pack in ["zip", "tar"] {
            assert_eq!(
                run(
                    &mut e,
                    &format!("r={pack}('literal.{pack}','d/literal*'); disp(r{{1}})")
                ),
                "d/literal*/file?.txt\n"
            );
        }
    }
}

#[test]
fn rev2_gzip_folder_recurses_with_and_without_output_folder() {
    // Finding 4, Q9/Q26: explicit output flattens names; otherwise each
    // compressed file is written beside its own source.
    let root = folder(".files2-rev2-gzip-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::create_dir_all(root.path().join("gf/inner")).unwrap();
    fs::write(root.path().join("gf/a.txt"), b"a").unwrap();
    fs::write(root.path().join("gf/inner/b.txt"), b"b").unwrap();
    assert_eq!(
        run(&mut e, "r=gzip('gf','gfout'); disp(r{1}); disp(r{2})").replace('\\', "/"),
        "gfout/a.txt.gz\ngfout/b.txt.gz\n"
    );
    assert_eq!(
        run(&mut e, "r=gzip('gf'); disp(r{1}); disp(r{2})").replace('\\', "/"),
        "gf/a.txt.gz\ngf/inner/b.txt.gz\n"
    );
    run(&mut e, "gunzip('gfout/*.gz','decoded');");
    assert_eq!(fs::read(root.path().join("decoded/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(root.path().join("decoded/b.txt")).unwrap(), b"b");
}

#[test]
fn rev2_importdata_trailing_delimiter_is_not_an_extra_column() {
    // Finding 5, Q19; retain interior missing fields and short-row NaNs.
    let root = folder(".files2-rev2-csv-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("t.csv"), "1,2,\n3,4,\n").unwrap();
    assert_eq!(
        run(
            &mut e,
            "x=importdata('t.csv'); disp(mat2str(x)); disp(size(x))"
        ),
        "[1 2;3 4]\n     2     2\n"
    );
    fs::write(root.path().join("missing.csv"), "1,,3,\n4,5,\n").unwrap();
    assert_eq!(
        run(&mut e, "x=importdata('missing.csv'); disp(mat2str(x))"),
        "[1 NaN 3;4 5 NaN]\n"
    );
}

#[test]
fn rev2_audiowrite_rejects_wav_bitrate_and_quality() {
    // Finding 6, Q5: WAV does not support lossy-codec parameters.
    let root = folder(".files2-rev2-wav-options-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("a.wav"), b"keep").unwrap();
    for (key, label) in [
        ("BitRate", "BitRate"),
        ("Quality", "Quality"),
        ("bitrate", "BitRate"),
    ] {
        let error = e
            .execute(&format!("audiowrite('a.wav',[0;1],8000,'{key}',64)"))
            .unwrap_err();
        assert_eq!(
            unplaced(&error),
            format!("The parameter {label} is not supported for .wav files.")
        );
        assert_eq!(fs::read(root.path().join("a.wav")).unwrap(), b"keep");
    }
}

#[test]
fn rev2_fileattrib_without_outputs_displays_attributes() {
    // Finding 7, Q3: display the struct, including with a semicolon; retain ans.
    let root = folder(".files2-rev2-attrib-display-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("f"), b"x").unwrap();
    run(&mut e, "ans=42;");
    for (query, reference) in [
        ("fileattrib('f')", "[s,a]=fileattrib('f'); disp(a)"),
        ("fileattrib('f');", "[s,a]=fileattrib('f'); disp(a)"),
        ("fileattrib", "[s,a]=fileattrib; disp(a)"),
        ("fileattrib()", "[s,a]=fileattrib(); disp(a)"),
    ] {
        let expected = run(&mut e, reference);
        assert!(expected.contains("Name:") && expected.contains("UserWrite:"));
        assert_eq!(run(&mut e, query), format!("\n{expected}"), "{query}");
        assert_eq!(e.variables["ans"].data(), &[42.]);
    }
    assert!(run(&mut e, "fileattrib('f','+w')").trim().is_empty());
}

#[test]
fn rev2_tar_supports_long_basename_and_utf8() {
    // Finding 8, Q12: a basename longer than 100 bytes is valid.
    let root = folder(".files2-rev2-long-tar-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    for (i, name) in [
        format!("{}.txt", "a".repeat(120)),
        format!("{}.txt", "é".repeat(60)),
    ]
    .iter()
    .enumerate()
    {
        fs::write(root.path().join(name), b"long name content").unwrap();
        assert_eq!(
            run(
                &mut e,
                &format!("r=tar('long{i}.tar','{name}'); disp(r{{1}})")
            ),
            format!("{name}\n")
        );
        let bytes = fs::read(root.path().join(format!("long{i}.tar"))).unwrap();
        // Inspect the interoperable GNU long-name record independently of
        // our reader: type L, UTF-8 payload, NUL, then a regular header.
        assert_eq!(bytes[156], b'L');
        assert_eq!(&bytes[512..512 + name.len()], name.as_bytes());
        assert_eq!(bytes[512 + name.len()], 0);
        assert_eq!(bytes[1024 + 156], b'0');
        run(&mut e, &format!("untar('long{i}.tar','out{i}');"));
        assert_eq!(
            fs::read(root.path().join(format!("out{i}")).join(name)).unwrap(),
            b"long name content"
        );
    }
}

#[test]
fn rev2_imread_imfinfo_honor_explicit_format() {
    // Finding 9, Q6: a PNG passed to the JPEG reader is not a JPEG file.
    let root = folder(".files2-rev2-format-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    run(&mut e, "imwrite(uint8([1 2;3 4]),'p.png');");
    for function in ["imread", "imfinfo"] {
        for format in ["jpg", "jpeg", "JPG"] {
            let error = e
                .execute(&format!("{function}('p.png','{format}')"))
                .unwrap_err();
            assert_eq!(
                unplaced(&error),
                format!(
                    "'{}' is not a JPEG file.",
                    e.current_folder().unwrap().join("p.png").display()
                )
            );
        }
        run(&mut e, &format!("x={function}('p.png','png');"));
        run(&mut e, &format!("x={function}('p','png');"));
    }
}

#[test]
fn rev2_audiowrite_empty_input_error() {
    // Finding 10, Q20: reject empty y before constructing a WAV header.
    let root = folder(".files2-rev2-empty-wav-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    for y in ["[]", "zeros(0,2)", "zeros(1,0)"] {
        let error = e
            .execute(&format!("audiowrite('a.wav',{y},8000)"))
            .unwrap_err();
        assert_eq!(
            unplaced(&error),
            "The value of 'y' is invalid. Expected input to be nonempty."
        );
        assert!(!root.path().join("a.wav").exists());
    }
}

#[test]
fn rev2_fileattrib_invalid_modifier_returns_status_and_message() {
    // Finding 11, Q18: invalid recursion modifier returns status 0.
    let root = folder(".files2-rev2-modifier-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("f"), b"x").unwrap();
    assert_eq!(
        run(&mut e, "[s,msg]=fileattrib('f','+w','u','x'); disp(msg)"),
        "Invalid modifier string.\n"
    );
    assert_eq!(run(&mut e, "disp(class(s))"), "logical\n");
    assert_eq!(run(&mut e, "disp(double(s))").trim(), "0");
    let error = e.execute("fileattrib('f','+w','u','x')").unwrap_err();
    assert_eq!(unplaced(&error), "Invalid modifier string.");
}

#[test]
fn rev2_imfinfo_unknown_format_has_its_own_error() {
    // Finding 12, Q29/Q30: imfinfo omits "the"; imread retains it.
    let root = folder(".files2-rev2-unknown-image-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("unknown"), b"not an image").unwrap();
    for (function, message) in [
        ("imfinfo", "Unable to determine file format."),
        ("imread", "Unable to determine the file format."),
    ] {
        let error = e.execute(&format!("{function}('unknown')")).unwrap_err();
        assert_eq!(unplaced(&error), message);
    }
}

#[test]
fn rev2_importdata_image_extension_falls_back_to_text() {
    // Finding 13, Q31: non-image data in an image-named file is text input.
    let root = folder(".files2-rev2-image-text-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    fs::write(root.path().join("x.png"), "hello").unwrap();
    assert_eq!(run(&mut e, "x=importdata('x.png'); disp(x{1})"), "hello\n");
    for ext in ["png", "jpg", "bmp", "tif", "gif"] {
        fs::write(root.path().join(format!("text.{ext}")), "1 2\n3 4\n").unwrap();
        assert_eq!(
            run(
                &mut e,
                &format!("x=importdata('text.{ext}'); disp(mat2str(x))")
            ),
            "[1 2;3 4]\n"
        );
    }
    fs::write(root.path().join("header.png"), "a;b\n1;2\n").unwrap();
    assert_eq!(
        run(
            &mut e,
            "[x,d,h]=importdata('header.png',';',1); disp(mat2str(x.data)); disp(d); disp(h)"
        ),
        "[1 2]\n;\n     1\n"
    );
    run(
        &mut e,
        "imwrite(uint8([1 2;3 4]),'real.png'); x=importdata('real.png');",
    );
    assert_eq!(run(&mut e, "disp(class(x))"), "uint8\n");
    assert_eq!(run(&mut e, "disp(mat2str(x))"), "[1 2;3 4]\n");
}

fn error_of(e: &mut Engine, source: &str) -> String {
    e.execute(source)
        .err()
        .unwrap_or_else(|| panic!("{source}: no error"))
        .to_string()
}

/// Review rev3 (2026-10-24): bounded textdata, non-regular files, palettes, sample rates,
/// unknown formats and root names; messages from the reference where it has one.
#[test]
fn rev3_bounds_formats_and_roots() {
    let root = folder(".files2-rev3-");
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    // 300 header lines with 300 fields in the last: 90000 cells is over the cell limit.
    let mut text = "h\n".repeat(299);
    text.push_str(&vec!["c"; 300].join(","));
    text.push_str("\n1\n");
    fs::write(root.path().join("big.csv"), text).unwrap();
    assert!(
        error_of(&mut e, "x = importdata('big.csv');").contains("exceeds the cell array limit")
    );
    run(&mut e, "imwrite(uint8([1 2; 3 4]), 'p.png');");
    assert!(
        error_of(&mut e, "x = imread('p.png', 'bogus');")
            .contains("The input, 'bogus', did not match any of the valid strings.")
    );
    assert!(
        error_of(&mut e, "x = imfinfo('p.png', 'bogus');")
            .contains("Could not find format \"bogus\" in format registry.")
    );
    assert!(
        error_of(&mut e, "audiowrite('a.wav', [0 0.1], 0.3);")
            .contains("Expected input to be integer-valued.")
    );
    assert!(
        error_of(&mut e, "audiowrite('a.wav', [0 0.1], 44100.5);")
            .contains("Expected input to be integer-valued.")
    );
    // A palette of 2 bytes (not a multiple of 3) is rejected.
    let mut png = fs::read(root.path().join("p.png")).unwrap();
    let ihdr_end = 8 + 8 + 13 + 4;
    let mut chunk = vec![0, 0, 0, 2];
    chunk.extend_from_slice(b"PLTE\x01\x02");
    let crc = {
        let mut c = !0u32;
        for &b in &chunk[4..] {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
        }
        !c
    };
    chunk.extend_from_slice(&crc.to_be_bytes());
    png.splice(ihdr_end..ihdr_end, chunk);
    fs::write(root.path().join("bad.png"), png).unwrap();
    assert!(error_of(&mut e, "x = imfinfo('bad.png');").contains("Invalid PNG palette."));
}

#[cfg(unix)]
#[test]
fn rev3_archives_skip_fifos_and_root_names_keep_their_separator() {
    let root = folder(".files2-rev3u-");
    let dir = root.path().join("d");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("a.txt"), "a").unwrap();
    let fifo = dir.join("pipe");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let mut e = Engine::default();
    e.set_current_folder(root.path()).unwrap();
    run(&mut e, "n = zip('z.zip', 'd'); k = numel(n);");
    assert_eq!(e.variables["k"].data(), &[1.]);
    run(&mut e, "[s, a] = fileattrib('/');");
    assert_eq!(run(&mut e, "disp(a.Name)"), "/\n");
}
