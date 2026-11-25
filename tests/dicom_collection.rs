//! Tests for dicomCollection: folder/file scanning into a series table.
use matrust::{Array, Engine};
use std::alloc::GlobalAlloc;
use std::sync::atomic::Ordering;

// This suite has no other global allocator. Measure only the calling thread;
// directory/fixture creation and Engine construction are outside the window.
struct ScanAllocator;
thread_local! {
    static SCAN_ALLOCATIONS: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
}
fn scan_allocation(allocated: usize, freed: usize) {
    let _ = SCAN_ALLOCATIONS.try_with(|counter| {
        if let Some((live, peak)) = counter.get() {
            let high = live.saturating_add(allocated);
            counter.set(Some((high.saturating_sub(freed), peak.max(high))));
        }
    });
}
// SAFETY: forwards the original pointers/layouts to System; the TLS counter
// neither allocates nor dereferences an allocation.
unsafe impl GlobalAlloc for ScanAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        scan_allocation(layout.size(), 0);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        scan_allocation(layout.size(), 0);
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        scan_allocation(size, layout.size());
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        scan_allocation(0, layout.size());
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static SCAN_ALLOCATOR: ScanAllocator = ScanAllocator;

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dicom/collection")
}

fn quote_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn trim_dt_string(item: Option<&str>) -> Option<String> {
    let raw = item?;
    let trimmed = raw.trim_end_matches('0').trim_end_matches('.');
    Some(trimmed.to_owned())
}

#[test]
fn baseline_two_files_one_series() {
    let dir = tempfile::tempdir().unwrap();
    let src = fixture_dir();
    std::fs::copy(src.join("study1_series1_a.dcm"), dir.path().join("a.dcm")).unwrap();
    std::fs::copy(src.join("study1_series1_b.dcm"), dir.path().join("b.dcm")).unwrap();

    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "c = dicomCollection('{}');",
            quote_path(dir.path())
        ))
        .unwrap();
    let Array::Table(tbl) = engine.array("c").unwrap() else {
        panic!("expected table");
    };
    assert_eq!(tbl.height(), 1);
    assert_eq!(tbl.width(), 14);
    assert_eq!(tbl.row_names().unwrap(), &["s1".to_owned()]);
}

#[test]
fn grouping_and_sort_order_matches_pydicom() {
    let root = fixture_dir();
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("expected.json")).unwrap()).unwrap();
    let expected_rows = expected["rows"].as_array().unwrap();

    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "lastwarn(''); c = dicomCollection('{}'); [wmsg, wid] = lastwarn; study_dt_s = string(c.StudyDateTime, 'yyyy-MM-dd HH:mm:ss.SSSSSSSSS'); series_dt_s = string(c.SeriesDateTime, 'yyyy-MM-dd HH:mm:ss.SSSSSSSSS');",
            quote_path(&root)
        ))
        .unwrap();

    let Array::Character(wid) = engine.array("wid").unwrap() else {
        panic!("expected char wid");
    };
    let wid_text = wid
        .to_utf8(&std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert_eq!(wid_text, "dicomCollection:skipped");
    let Array::Character(wmsg) = engine.array("wmsg").unwrap() else {
        panic!("expected char wmsg");
    };
    let wmsg_text = wmsg
        .to_utf8(&std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert!(wmsg_text.contains('1'), "unexpected warning: {wmsg_text}");

    let Array::Table(tbl) = engine.array("c").unwrap() else {
        panic!("expected table");
    };
    assert_eq!(tbl.height(), expected_rows.len());
    assert_eq!(tbl.width(), 14);
    assert_eq!(
        tbl.names(),
        &[
            "StudyDateTime",
            "SeriesDateTime",
            "PatientName",
            "PatientSex",
            "Modality",
            "StudyDescription",
            "SeriesDescription",
            "StudyInstanceUID",
            "SeriesInstanceUID",
            "Rows",
            "Columns",
            "Channels",
            "Frames",
            "Filenames",
        ]
    );
    let expected_row_names: Vec<String> =
        (1..=expected_rows.len()).map(|i| format!("s{i}")).collect();
    assert_eq!(tbl.row_names().unwrap(), expected_row_names.as_slice());

    let Array::Str(study_dt) = engine.array("study_dt_s").unwrap() else {
        panic!("study_dt_s not string");
    };
    let Array::Str(series_dt) = engine.array("series_dt_s").unwrap() else {
        panic!("series_dt_s not string");
    };
    let Array::Cell(filenames_cell) = tbl.column(13) else {
        panic!("Filenames not cell");
    };

    for (row_idx, exp) in expected_rows.iter().enumerate() {
        assert_eq!(
            trim_dt_string(study_dt.data()[row_idx].as_deref()).as_deref(),
            exp["StudyDateTime"].as_str(),
            "row {row_idx} StudyDateTime"
        );
        assert_eq!(
            trim_dt_string(series_dt.data()[row_idx].as_deref()).as_deref(),
            exp["SeriesDateTime"].as_str(),
            "row {row_idx} SeriesDateTime"
        );

        for (col_idx, key) in [
            (2, "PatientName"),
            (3, "PatientSex"),
            (4, "Modality"),
            (5, "StudyDescription"),
            (6, "SeriesDescription"),
            (7, "StudyInstanceUID"),
            (8, "SeriesInstanceUID"),
        ] {
            let Array::Str(col) = tbl.column(col_idx) else {
                panic!("column {key} not string");
            };
            assert_eq!(
                col.data()[row_idx].as_deref().unwrap_or(""),
                exp[key].as_str().unwrap(),
                "row {row_idx} {key}"
            );
        }

        for (col_idx, key) in [
            (9, "Rows"),
            (10, "Columns"),
            (11, "Channels"),
            (12, "Frames"),
        ] {
            let Array::Numeric(col) = tbl.column(col_idx) else {
                panic!("column {key} not numeric");
            };
            assert_eq!(
                col.data()[row_idx],
                exp[key].as_f64().unwrap(),
                "row {row_idx} {key}"
            );
        }

        let Array::Str(files) = filenames_cell.get_linear(row_idx).unwrap().as_ref() else {
            panic!("Filenames cell {row_idx} not string array");
        };
        let actual_rel: Vec<String> = files
            .data()
            .iter()
            .map(|item| {
                let full = item.as_ref().unwrap().as_ref();
                std::path::Path::new(full)
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        let exp_files: Vec<String> = exp["Filenames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(actual_rel, exp_files, "row {row_idx} Filenames");
    }
}

#[test]
fn include_subfolders_false_skips_nested_dir() {
    let root = fixture_dir();
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "c = dicomCollection('{}', 'IncludeSubfolders', false);",
            quote_path(&root)
        ))
        .unwrap();
    let Array::Table(tbl) = engine.array("c").unwrap() else {
        panic!("expected table");
    };
    assert_eq!(tbl.height(), 1);
    assert_eq!(tbl.width(), 14);
}

#[test]
fn single_file_source_works_and_missing_dates_are_nat() {
    let file = fixture_dir().join("sub/study2_series3_rgb.dcm");
    let mut engine = Engine::default();
    engine
        .execute(&format!("c = dicomCollection('{}');", quote_path(&file)))
        .unwrap();
    let Array::Table(tbl) = engine.array("c").unwrap() else {
        panic!("expected table");
    };
    assert_eq!(tbl.height(), 1);
    assert_eq!(tbl.width(), 14);
    let Array::Datetime(study_dt) = tbl.column(0) else {
        panic!("expected datetime");
    };
    let Array::Datetime(series_dt) = tbl.column(1) else {
        panic!("expected datetime");
    };
    assert!(study_dt.data()[0].is_nat());
    assert!(series_dt.data()[0].is_nat());
    let Array::Numeric(channels) = tbl.column(11) else {
        panic!("expected numeric");
    };
    assert_eq!(channels.data()[0], 3.0);
}

#[test]
fn empty_folder_returns_0x14_table() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "c = dicomCollection('{}');",
            quote_path(dir.path())
        ))
        .unwrap();
    let Array::Table(tbl) = engine.array("c").unwrap() else {
        panic!("expected table");
    };
    assert_eq!(tbl.height(), 0);
    assert_eq!(tbl.width(), 14);
}

#[test]
fn dicomdir_input_errors_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let dicomdir = dir.path().join("DICOMDIR");
    std::fs::copy(fixture_dir().join("study1_series1_a.dcm"), &dicomdir).unwrap();
    let mut engine = Engine::default();
    let err = engine
        .execute(&format!(
            "c = dicomCollection('{}');",
            quote_path(&dicomdir)
        ))
        .unwrap_err();
    assert!(err.contains("DICOMDIR"), "unexpected error: {err}");
}

#[test]
fn dicomdir_inside_folder_is_skipped_and_link_loops_end() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture_dir().join("study1_series1_a.dcm"),
        dir.path().join("a.dcm"),
    )
    .unwrap();
    std::fs::copy(
        fixture_dir().join("study1_series1_a.dcm"),
        dir.path().join("DICOMDIR"),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "c = dicomCollection('{}');",
            quote_path(dir.path())
        ))
        .unwrap();
    let Array::Table(tbl) = engine.array("c").unwrap() else {
        panic!("expected table");
    };
    assert_eq!(tbl.height(), 1);
    // One file only: the DICOMDIR copy is skipped and the link back to the folder is walked once.
    engine
        .execute("n = numel(c.Filenames{1}); if n ~= 1, error('files: %d', n); end")
        .unwrap();
}

#[test]
fn cancellation_stops_scan() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture_dir().join("study1_series1_a.dcm"),
        dir.path().join("a.dcm"),
    )
    .unwrap();
    let mut engine = Engine::default();
    engine.cancellation().store(true, Ordering::Relaxed);
    let err = engine
        .execute(&format!(
            "c = dicomCollection('{}');",
            quote_path(dir.path())
        ))
        .unwrap_err();
    assert!(
        err.to_lowercase().contains("cancel"),
        "unexpected error: {err}"
    );
}

// Explicit-LE fixture mutations follow PS3.5 6.2/7.1; expected results below
// are calendar/VR derivations, independent of the collection implementation.
fn replace_value(bytes: &mut Vec<u8>, tag: [u8; 4], value: &[u8]) {
    let at = bytes.windows(4).position(|w| w == tag).unwrap();
    let old_len = u16::from_le_bytes([bytes[at + 6], bytes[at + 7]]) as usize;
    let mut padded = value.to_vec();
    if !padded.len().is_multiple_of(2) {
        padded.push(if &bytes[at + 4..at + 6] == b"UI" {
            0
        } else {
            b' '
        });
    }
    bytes[at + 6..at + 8].copy_from_slice(&(padded.len() as u16).to_le_bytes());
    bytes.splice(at + 8..at + 8 + old_len, padded);
}

fn collect_height(engine: &mut Engine, path: &std::path::Path) -> Result<usize, String> {
    engine.execute(&format!("c=dicomCollection('{}');", quote_path(path)))?;
    let Array::Table(table) = engine.array("c").unwrap() else {
        panic!("expected table")
    };
    Ok(table.height())
}

#[test]
fn regression_whole_scan_memory_500_hard_links() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("000.dcm");
    let mut bytes = std::fs::read(fixture_dir().join("study1_series1_a.dcm")).unwrap();
    replace_value(&mut bytes, [0x10, 0, 0x10, 0], &vec![b'A'; 60_000]);
    std::fs::write(&first, bytes).unwrap();
    for i in 1..500 {
        std::fs::hard_link(&first, dir.path().join(format!("{i:03}.dcm"))).unwrap();
    }
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 2 * 1024 * 1024,
        ..Default::default()
    });
    SCAN_ALLOCATIONS.with(|c| c.set(Some((0, 0))));
    let result = collect_height(&mut engine, dir.path());
    let (_, peak) = SCAN_ALLOCATIONS.with(|c| c.take().unwrap());
    println!("500-link scan peak additional allocation: {peak} bytes");
    assert!(
        peak <= 2 * 1024 * 1024,
        "scan peak allocation {peak} exceeds 2 MiB"
    );
    let error = result.expect_err("scan must exhaust memory budget");
    // Linux stops in the scan ("memory budget"); Windows can stop first in the
    // per-file dicominfo ("memory/output budget"). Both are the budget refusal.
    assert!(
        error.contains("memory budget") || error.contains("memory/output budget"),
        "{error}"
    );
}

#[test]
fn regression_retained_summaries_use_remaining_workspace_budget() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("000.dcm");
    std::fs::copy(fixture_dir().join("study1_series1_a.dcm"), &first).unwrap();
    for i in 1..200 {
        std::fs::hard_link(&first, dir.path().join(format!("{i:03}.dcm"))).unwrap();
    }
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 2 * 1024 * 1024,
        ..Default::default()
    });
    engine.execute("held=zeros(1,131072);").unwrap();
    let error = collect_height(&mut engine, dir.path())
        .expect_err("retained scan must share workspace budget");
    assert!(
        error.contains("memory") && error.contains("budget"),
        "{error}"
    );
}

#[test]
fn regression_directory_count_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10_001 {
        std::fs::create_dir(dir.path().join(format!("d{i}"))).unwrap();
    }
    let error = collect_height(&mut Engine::default(), dir.path())
        .expect_err("scan must enforce a directory limit");
    assert!(error.contains("directory limit"), "{error}");
}

#[test]
fn regression_retained_text_caps_skip_the_file() {
    for (tag, cap) in [
        ([0x20, 0, 0x0d, 0], 64),     // UI
        ([0x10, 0, 0x10, 0], 64 * 5), // PN scan cap
        ([8, 0, 0x30, 0x10], 64),     // LO StudyDescription
        ([8, 0, 0x3e, 0x10], 64),     // LO SeriesDescription
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.dcm");
        let mut bytes = std::fs::read(fixture_dir().join("study1_series1_a.dcm")).unwrap();
        replace_value(&mut bytes, tag, &vec![b'A'; cap + 1]);
        std::fs::write(&path, bytes).unwrap();
        let mut engine = Engine::default();
        assert_eq!(
            collect_height(&mut engine, &path).unwrap(),
            0,
            "tag {tag:?}"
        );
        engine.execute("[msg,id]=lastwarn; if ~strcmp(id,'dicomCollection:skipped'), error('missing warning'); end").unwrap();
        let Array::Character(message) = engine.array("msg").unwrap() else {
            panic!("expected warning text");
        };
        let message = message
            .to_utf8(&std::sync::atomic::AtomicBool::new(false))
            .unwrap();
        assert!(
            !message.contains("unreadable"),
            "size skip warning: {message}"
        );
    }
}

#[test]
fn regression_sparse_large_pixels_are_listed() {
    use std::io::{Seek, SeekFrom, Write};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.dcm");
    let mut bytes = std::fs::read(fixture_dir().join("study1_series1_a.dcm")).unwrap();
    replace_value(&mut bytes, [0x28, 0, 0x10, 0], &4096u16.to_le_bytes());
    replace_value(&mut bytes, [0x28, 0, 0x11, 0], &32768u16.to_le_bytes());
    replace_value(&mut bytes, [0x28, 0, 0x00, 0x01], &16u16.to_le_bytes());
    let pixels = bytes
        .windows(4)
        .position(|w| w == [0xe0, 0x7f, 0x10, 0])
        .unwrap();
    bytes.truncate(pixels);
    bytes.extend_from_slice(&[0xe0, 0x7f, 0x10, 0, b'O', b'W', 0, 0]);
    let payload = 4096u32 * 32768 * 2;
    bytes.extend_from_slice(&payload.to_le_bytes());
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(&bytes).unwrap();
    file.seek(SeekFrom::Start(bytes.len() as u64 + u64::from(payload) - 1))
        .unwrap();
    file.write_all(&[0]).unwrap();
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 2 * 1024 * 1024,
        ..Default::default()
    });
    assert_eq!(
        collect_height(&mut engine, &path).unwrap(),
        1,
        "single sparse file"
    );
    assert_eq!(
        collect_height(&mut engine, dir.path()).unwrap(),
        1,
        "folder sparse file"
    );
    engine
        .execute("if c.Rows ~= 4096 || c.Columns ~= 32768, error('wrong geometry'); end")
        .unwrap();
}

#[test]
fn regression_native_and_deflated_scans_stop_at_pixel_tag() {
    let dir = tempfile::tempdir().unwrap();
    let original = std::fs::read(fixture_dir().join("study1_series1_a.dcm")).unwrap();
    let pixels = original
        .windows(4)
        .position(|w| w == [0xe0, 0x7f, 0x10, 0])
        .unwrap();
    // PS3.5 header discovery needs no pixel value, even if it is unavailable.
    // Keep the pixel element header but deliberately omit its entire value.
    let native = dir.path().join("native.dcm");
    std::fs::write(&native, &original[..pixels + 12]).unwrap();
    let bytes = deflated_collection_bytes(original[..pixels + 12].to_vec(), 0);
    let deflated = dir.path().join("deflated.dcm");
    std::fs::write(&deflated, bytes).unwrap();
    let mut engine = Engine::default();
    assert_eq!(collect_height(&mut engine, &native).unwrap(), 1);
    assert_eq!(collect_height(&mut engine, &deflated).unwrap(), 1);
}

fn deflated_collection_bytes(mut bytes: Vec<u8>, empty_blocks: usize) -> Vec<u8> {
    use std::io::Write;
    let old_len = bytes.len();
    let meta_len = u32::from_le_bytes(bytes[140..144].try_into().unwrap()) as usize;
    replace_value(&mut bytes, [2, 0, 0x10, 0], b"1.2.840.10008.1.2.1.99");
    let meta_len = meta_len + bytes.len() - old_len;
    bytes[140..144].copy_from_slice(&(meta_len as u32).to_le_bytes());
    let start = 144 + meta_len;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&bytes[start..]).unwrap();
    let compressed = encoder.finish().unwrap();
    bytes.truncate(start);
    // RFC1951 section 3.2.4: non-final empty stored blocks, byte aligned.
    bytes.extend(std::iter::repeat_n([0, 0, 0, 255, 255], empty_blocks).flatten());
    bytes.extend_from_slice(&compressed);
    if !(empty_blocks * 5 + compressed.len()).is_multiple_of(2) {
        bytes.push(0);
    }
    bytes
}

#[test]
fn regression_deflated_empty_blocks_obey_work_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty_blocks.dcm");
    let bytes = std::fs::read(fixture_dir().join("study1_series1_a.dcm")).unwrap();
    std::fs::write(&path, deflated_collection_bytes(bytes, 5000)).unwrap();
    let mut engine = Engine::with_limits(matrust::engine::Limits {
        evaluation_units: Some(1024),
        ..Default::default()
    });
    let error =
        collect_height(&mut engine, &path).expect_err("empty blocks must exhaust the work budget");
    assert!(error.contains("budget"), "{error}");
}

#[cfg(unix)]
#[test]
fn regression_symlinked_file_in_folder_is_listed() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("linked.dcm");
    std::os::unix::fs::symlink(fixture_dir().join("study1_series1_a.dcm"), &link).unwrap();
    let mut engine = Engine::default();
    assert_eq!(collect_height(&mut engine, &link).unwrap(), 1);
    assert_eq!(
        collect_height(&mut engine, dir.path()).unwrap(),
        1,
        "folder file symlink"
    );
}

#[cfg(unix)]
#[test]
fn regression_folder_links_stay_inside_source() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture_dir().join("study1_series1_a.dcm"),
        outside.path().join("outside.dcm"),
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    let mut engine = Engine::default();
    assert_eq!(
        collect_height(&mut engine, root.path()).unwrap(),
        0,
        "external directory link"
    );
    let inside = root.path().join("inside");
    std::fs::create_dir(&inside).unwrap();
    std::fs::copy(
        fixture_dir().join("study1_series1_a.dcm"),
        inside.join("inside.dcm"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&inside, root.path().join("alias")).unwrap();
    assert_eq!(collect_height(&mut engine, root.path()).unwrap(), 1);
    engine
        .execute("if numel(c.Filenames{1}) ~= 1, error('duplicate linked folder'); end")
        .unwrap();
}

#[test]
fn regression_invalid_da_tm_are_nat() {
    // PS3.5 6.2: Gregorian DA, TM HH[MM[SS[.FFFFFF]]]. A leap second
    // cannot be represented by this engine's fixed 86400-second days: NaT.
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dicom/collection/regressions.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let date = case["date"].as_str().unwrap();
        let time = case["time"].as_str().unwrap();
        let nat = case["nat"].as_bool().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dated.dcm");
        let mut bytes = std::fs::read(fixture_dir().join("study1_series1_a.dcm")).unwrap();
        replace_value(&mut bytes, [8, 0, 0x20, 0], date.as_bytes());
        replace_value(&mut bytes, [8, 0, 0x30, 0], time.as_bytes());
        replace_value(&mut bytes, [8, 0, 0x21, 0], date.as_bytes());
        replace_value(&mut bytes, [8, 0, 0x31, 0], time.as_bytes());
        std::fs::write(&path, bytes).unwrap();
        let mut engine = Engine::default();
        assert_eq!(collect_height(&mut engine, &path).unwrap(), 1);
        let Array::Table(table) = engine.array("c").unwrap() else {
            unreachable!()
        };
        for column in [0, 1] {
            let Array::Datetime(value) = table.column(column) else {
                unreachable!()
            };
            assert_eq!(
                value.data()[0].is_nat(),
                nat,
                "DA={date} TM={time} column {column}"
            );
        }
    }
}

#[test]
#[ignore]
fn benchmark_2000_small_files() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = std::fs::read(fixture_dir().join("study1_series1_a.dcm")).unwrap();
    for i in 0..2000 {
        std::fs::write(dir.path().join(format!("img_{i:04}.dcm")), &bytes).unwrap();
    }
    let mut engine = Engine::default();
    let start = std::time::Instant::now();
    engine
        .execute(&format!(
            "c = dicomCollection('{}');",
            quote_path(dir.path())
        ))
        .unwrap();
    let elapsed = start.elapsed();
    let Array::Table(tbl) = engine.array("c").unwrap() else {
        panic!("expected table");
    };
    assert_eq!(tbl.height(), 1);
    let Array::Numeric(frames) = tbl.column(12) else {
        panic!("expected numeric");
    };
    assert_eq!(frames.data()[0], 2000.0);
    println!(
        "workers={}, 2000 small files scan time: {:.3} ms",
        rayon::current_num_threads(),
        elapsed.as_secs_f64() * 1000.0
    );
}

#[test]
fn browser_count_is_scoped_and_preserves_public_outputs_and_warnings() {
    use matrust::engine::with_dicom_collection_count;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"not DICOM").unwrap();
    let mut engine = Engine::default();
    let (run, count) = with_dicom_collection_count(|| {
        engine.execute(&format!(
            "lastwarn(''); c=dicomCollection('{}'); [msg,id]=lastwarn;",
            quote_path(dir.path())
        ))
    });
    run.unwrap();
    assert_eq!(count, Ok(1));
    engine
        .execute("if ~isempty(msg) || ~isempty(id), error('public warning changed'); end")
        .unwrap();
    let Array::Table(table) = engine.array("c").unwrap() else {
        panic!("expected table")
    };
    assert_eq!((table.height(), table.width()), (0, 14));
    let (_, missing) = with_dicom_collection_count(|| {});
    assert!(missing.unwrap_err().contains("did not report"));
    let (run, missing) = with_dicom_collection_count(|| {
        engine.execute(&format!(
            "c=dicomCollection('{}');",
            quote_path(&dir.path().join("missing"))
        ))
    });
    let error = run.unwrap_err();
    let reason = missing.unwrap_err();
    assert!(error.contains(&reason), "{error} did not include {reason}");
}

#[test]
fn browser_count_uses_collection_cap_not_dir_limit() {
    use matrust::engine::with_dicom_collection_count;
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10_001 {
        std::fs::write(dir.path().join(format!("f{i}.bin")), b"").unwrap();
    }
    let mut engine = Engine::default();
    let (result, count) = with_dicom_collection_count(|| collect_height(&mut engine, dir.path()));
    assert_eq!(result.unwrap(), 0);
    assert_eq!(count, Ok(10_001));

    let mut limited = Engine::with_limits(matrust::engine::Limits {
        memory_bytes: 64 * 1024,
        ..Default::default()
    });
    let (result, count) = with_dicom_collection_count(|| collect_height(&mut limited, dir.path()));
    assert!(result.unwrap_err().contains("budget"));
    assert!(count.unwrap_err().contains("budget"));
}
