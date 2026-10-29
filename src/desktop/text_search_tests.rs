use super::*;
fn query(folder: &Path, text: &str) -> Query {
    Query {
        folder: folder.to_owned(),
        text: text.into(),
        match_case: true,
        subfolders: false,
    }
}
fn limits() -> Limits {
    Limits {
        files: 20,
        entries: 30,
        read: 4096,
        results: 20,
        output: 32768,
        depth: 3,
    }
}

#[test]
fn literal_unicode_fold_returns_original_offsets_and_cr_lf_bom_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unicode.m");
    let body = "first=a+b;\r\nK=a+b;\rΣ=1; σ=2; ς=3;\nlast=a+b;";
    fs::write(&path, format!("\u{feff}{body}")).unwrap();
    let cancel = AtomicBool::new(false);
    let found = scan(&query(dir.path(), "a+b"), &cancel).unwrap();
    assert_eq!(
        found.hits.iter().map(|h| h.line).collect::<Vec<_>>(),
        vec![1, 2, 4]
    );
    for hit in &found.hits {
        assert_eq!(&body[hit.bytes.clone()], "a+b");
        verify(hit, &cancel).unwrap();
    }
    let mut folded = query(dir.path(), "k");
    folded.match_case = false;
    let hit = scan(&folded, &cancel).unwrap().hits.remove(0);
    assert_eq!(&body[hit.bytes], "K");
    assert_eq!(hit.line, 2);
    folded.text = "σ".into();
    let found = scan(&folded, &cancel).unwrap();
    assert_eq!(found.hits.len(), 3);
    assert_eq!(
        found
            .hits
            .iter()
            .map(|h| &body[h.bytes.clone()])
            .collect::<Vec<_>>(),
        vec!["Σ", "σ", "ς"]
    );
    folded.match_case = true;
    assert_eq!(scan(&folded, &cancel).unwrap().hits.len(), 1);
}

#[test]
fn skipped_types_subfolder_choice_limits_and_cancel_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("one.m"), "hit hit hit").unwrap();
    fs::write(dir.path().join("nested/two.txt"), "hit").unwrap();
    fs::write(dir.path().join("binary.m"), [0u8, 1, 2]).unwrap();
    fs::write(dir.path().join("invalid.txt"), [255u8]).unwrap();
    fs::write(dir.path().join("data.csv"), "hit").unwrap();
    let file = File::create(dir.path().join("large.m")).unwrap();
    file.set_len((MAX_FILE + 1) as u64).unwrap();
    drop(file);
    let cancel = AtomicBool::new(false);
    let mut q = query(dir.path(), "hit");
    let direct = scan_limited(&q, &cancel, limits()).unwrap();
    assert_eq!(direct.hits.len(), 3);
    assert_eq!(direct.counts.binary, 2);
    assert_eq!(direct.counts.oversized, 1);
    assert_eq!(direct.counts.unsupported, 1);
    assert!(direct.truncated.is_none());
    q.subfolders = true;
    assert_eq!(scan_limited(&q, &cancel, limits()).unwrap().hits.len(), 4);
    let mut bound = limits();
    bound.results = 2;
    let partial = scan_limited(&q, &cancel, bound).unwrap();
    assert_eq!(partial.hits.len(), 2);
    assert_eq!(partial.truncated, Some("result limit"));
    let mut direct_query = q.clone();
    direct_query.subfolders = false;
    let mut bound = limits();
    bound.output = 1;
    assert_eq!(
        scan_limited(&direct_query, &cancel, bound)
            .unwrap()
            .truncated,
        Some("result limit")
    );
    let mut bound = limits();
    bound.entries = 1;
    assert_eq!(
        scan_limited(&q, &cancel, bound).unwrap().truncated,
        Some("directory-entry limit")
    );
    let mut bound = limits();
    bound.files = 0;
    assert_eq!(
        scan_limited(&q, &cancel, bound).unwrap().truncated,
        Some("file limit")
    );
    let mut bound = limits();
    bound.read = 1;
    assert_eq!(
        scan_limited(&q, &cancel, bound).unwrap().truncated,
        Some("cumulative read limit")
    );
    let mut bound = limits();
    bound.depth = 0;
    let shallow = scan_limited(&q, &cancel, bound).unwrap();
    assert_eq!(shallow.hits.len(), 3);
    assert_eq!(shallow.counts.depth, 1);
    assert_eq!(shallow.truncated, Some("subfolder depth limit"));
    cancel.store(true, Ordering::Relaxed);
    assert!(scan(&q, &cancel).is_err());
}

#[test]
fn verify_rejects_same_length_replacement_and_query_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.m");
    fs::write(&path, "needle=7;").unwrap();
    let cancel = AtomicBool::new(false);
    let q = query(dir.path(), "needle");
    let results = scan(&q, &cancel).unwrap();
    verify(&results.hits[0], &cancel).unwrap();
    fs::write(&path, "change=7;").unwrap();
    assert!(
        verify(&results.hits[0], &cancel)
            .unwrap_err()
            .contains("changed")
    );
    for text in [String::new(), "x".repeat(257), "x\ny".into()] {
        assert!(scan(&query(dir.path(), &text), &cancel).is_err());
    }
    fs::remove_file(path).unwrap();
    assert!(verify(&results.hits[0], &cancel).is_err());
}

#[cfg(unix)]
#[test]
fn linked_descendants_are_skipped_but_selected_linked_parent_is_allowed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let external = dir.path().join("external");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&external).unwrap();
    fs::write(root.join("own.m"), "hit").unwrap();
    fs::write(external.join("foreign.m"), "hit").unwrap();
    symlink(&external, root.join("linked")).unwrap();
    symlink(external.join("foreign.m"), root.join("linked.m")).unwrap();
    let selected = dir.path().join("selected");
    symlink(&root, &selected).unwrap();
    let mut q = query(&selected, "hit");
    q.subfolders = true;
    let result = scan(&q, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.counts.linked, 2);
}

#[cfg(windows)]
#[test]
fn windows_reparse_attribute_is_excluded_when_a_link_can_be_created() {
    use std::os::windows::fs::symlink_file;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("own.m");
    fs::write(&path, "hit").unwrap();
    let link = dir.path().join("linked.m");
    match symlink_file(&path, &link) {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == Some(1314) => return, // OS privilege unavailable, no link-coverage claim.
        Err(e) => panic!("create test link: {e}"),
    }
    assert!(linked(&fs::symlink_metadata(&link).unwrap()));
    let result = scan(&query(dir.path(), "hit"), &AtomicBool::new(false)).unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.counts.linked, 1);
}
