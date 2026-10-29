use super::*;
use std::sync::{Arc, atomic::Ordering};
fn request(source: &Path, destination: &Path, bytes: &[u8]) -> Prepare {
    Prepare {
        source: source.to_owned(),
        destination: destination.to_owned(),
        original: Original::identity(bytes, &AtomicBool::new(false)).unwrap(),
    }
}
#[test]
fn streamed_original_copies_keep_vector_pages_raster_depth_and_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><path d='M0 0L4 4' stroke='red'/></svg>";
    let source = dir.path().join("source.svg");
    fs::write(&source, svg).unwrap();
    let preview = crate::svg_load::load(&source, 64, &cancel).unwrap();
    assert!(preview.original.as_ref().unwrap().bytes.is_none());
    let target = dir.path().join("copy.svg");
    save(
        prepare(
            Prepare {
                source: source.clone(),
                destination: target.clone(),
                original: preview.original.unwrap(),
            },
            &cancel,
        )
        .unwrap(),
        &cancel,
    )
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), svg);
    let source = dir.path().join("depth.png");
    let pixels = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_fn(4, 3, |x, y| {
        image::Luma([(x * 1000 + y * 17) as u16])
    });
    pixels.save(&source).unwrap();
    let bytes = fs::read(&source).unwrap();
    let preview = crate::image_load::load(&source, 64, &cancel).unwrap();
    let target = dir.path().join("depth-copy.png");
    save(
        prepare(
            Prepare {
                source: source.clone(),
                destination: target.clone(),
                original: preview.original.unwrap(),
            },
            &cancel,
        )
        .unwrap(),
        &cancel,
    )
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert_eq!(image::open(&target).unwrap().into_luma16(), pixels);
    // Paged TIFF reuses the decoder-owned encoded buffer without a sample copy.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/preview/tiff-preview/pages.tiff");
    let source = dir.path().join("pages.tiff");
    fs::copy(&fixture, &source).unwrap();
    let bytes = fs::read(&source).unwrap();
    let (page, navigation) = crate::image_pages::open(&source, 2, 64, &cancel).unwrap();
    assert_eq!(navigation.document.page_count, 3);
    let original = page.original.unwrap();
    let alias = original.clone();
    assert!(Arc::ptr_eq(
        original.bytes.as_ref().unwrap(),
        alias.bytes.as_ref().unwrap()
    ));
    fs::remove_file(&source).unwrap();
    let target = dir.path().join("all-pages.tiff");
    save(
        prepare(
            Prepare {
                source,
                destination: target.clone(),
                original,
            },
            &cancel,
        )
        .unwrap(),
        &cancel,
    )
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert_eq!(
        crate::image_pages::open(&target, 2, 64, &cancel)
            .unwrap()
            .1
            .document
            .page_count,
        3
    );
}
#[test]
fn source_identity_and_destination_review_reject_same_size_changes_and_preserve_files() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let source = dir.path().join("source.eps");
    let target = dir.path().join("copy.eps");
    let bytes = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 4 4\nshowpage\n";
    fs::write(&source, bytes).unwrap();
    fs::write(&target, b"existing destination").unwrap();
    let plan = prepare(request(&source, &target, bytes), &cancel).unwrap();
    assert!(plan.replaces());
    let mut changed = bytes.to_vec();
    changed[1] = b'?';
    fs::write(&source, &changed).unwrap();
    assert!(save(plan, &cancel).unwrap_err().contains("changed"));
    assert_eq!(fs::read(&target).unwrap(), b"existing destination");
    assert!(prepare(request(&source, &target, bytes), &cancel).is_err());
    fs::write(&source, bytes).unwrap();
    let plan = prepare(request(&source, &target, bytes), &cancel).unwrap();
    fs::write(&target, b"external edit").unwrap();
    assert!(
        save(plan, &cancel)
            .unwrap_err()
            .contains("Destination changed")
    );
    assert_eq!(fs::read(&target).unwrap(), b"external edit");
    fs::remove_file(&source).unwrap();
    assert!(
        prepare(request(&source, &target, bytes), &cancel)
            .err()
            .unwrap()
            .contains("unavailable")
    );
    fs::write(&source, bytes).unwrap();
    assert!(prepare(request(&source, &source, bytes), &cancel).is_err());
    assert!(
        prepare(
            request(&source, &dir.path().join("copy.png"), bytes),
            &cancel
        )
        .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), bytes);
    let alias = dir.path().join("hard-link.eps");
    fs::hard_link(&source, &alias).unwrap();
    save(
        prepare(request(&source, &alias, bytes), &cancel).unwrap(),
        &cancel,
    )
    .unwrap();
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fs::read(&alias).unwrap(), bytes);
    #[cfg(windows)]
    {
        let link = dir.path().join("linked.eps");
        if std::os::windows::fs::symlink_file(&target, &link).is_ok() {
            assert!(prepare(request(&source, &link, bytes), &cancel).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"external edit");
        }
    }
    #[cfg(unix)]
    {
        let link = dir.path().join("linked.eps");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(prepare(request(&source, &link, bytes), &cancel).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"external edit");
    }
}
#[test]
fn mid_stream_stop_and_final_conflict_drop_only_owned_temporary_and_retry() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let bytes = vec![17; 3 * 64 * 1024 + 7];
    let source = dir.path().join("source.pdf");
    let target = dir.path().join("copy.pdf");
    fs::write(&source, &bytes).unwrap();
    fs::write(&target, b"prior").unwrap();
    let plan = prepare(request(&source, &target, &bytes), &cancel).unwrap();
    let mut chunks = 0;
    assert_eq!(
        save_checked(plan, &cancel, || {
            chunks += 1;
            if chunks == 2 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err(),
        "Cancelled"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    assert_eq!(fs::read(&target).unwrap(), b"prior");
    assert_eq!(fs::read(&source).unwrap(), bytes);
    cancel.store(false, Ordering::Relaxed);
    let plan = prepare(request(&source, &target, &bytes), &cancel).unwrap();
    let mut edited = false;
    assert!(
        save_checked(plan, &cancel, || {
            if !edited {
                fs::write(&target, b"late external edit").unwrap();
                edited = true;
            }
        })
        .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"late external edit");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    let plan = prepare(request(&source, &target, &bytes), &cancel).unwrap();
    save(plan, &cancel).unwrap();
    assert_eq!(fs::read(&target).unwrap(), bytes);
    let fresh = dir.path().join("fresh.pdf");
    let plan = prepare(request(&source, &fresh, &bytes), &cancel).unwrap();
    assert!(!plan.replaces());
    assert!(
        save_checked(plan, &cancel, || {
            if !fresh.exists() {
                fs::write(&fresh, b"new competitor").unwrap();
            }
        })
        .is_err()
    );
    assert_eq!(fs::read(&fresh).unwrap(), b"new competitor");
}

#[cfg(windows)]
#[test]
fn pdf_original_digest_tracks_helper_snapshot_and_copies_all_pages() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let source = dir.path().join("source.pdf");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview/pdf-preview/pages.pdf"),
        &source,
    )
    .unwrap();
    let bytes = fs::read(&source).unwrap();
    let doc = crate::pdf_load::open(&source, &cancel).unwrap();
    let (preview, _page) = crate::pdf_load::render(doc.clone(), 2, 64, &cancel).unwrap();
    let original = preview.original.unwrap();
    assert_eq!(original.digest, *blake3::hash(&bytes).as_bytes());
    let target = dir.path().join("copy.pdf");
    save(
        prepare(
            Prepare {
                source: source.clone(),
                destination: target.clone(),
                original: original.clone(),
            },
            &cancel,
        )
        .unwrap(),
        &cancel,
    )
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert_eq!(
        crate::pdf_load::open(&target, &cancel).unwrap().page_count,
        doc.page_count
    );
    // Page navigation still uses its snapshot, but a later source-based copy
    // rejects a replaced original rather than silently copying different data.
    let mut changed = bytes.clone();
    changed[bytes.len() - 1] ^= 1;
    fs::write(&source, &changed).unwrap();
    assert!(crate::pdf_load::render(doc, 0, 64, &cancel).is_ok());
    assert!(
        prepare(
            Prepare {
                source,
                destination: target.clone(),
                original
            },
            &cancel
        )
        .is_err()
    );
    assert_eq!(fs::read(target).unwrap(), bytes);
}

/// Opt-in measurements of new work, not an end-to-end before/after comparison.
#[test]
#[ignore = "release measurement; parent-owned exact-test runner only"]
fn original_copy_release_measurements() {
    use std::{hint::black_box, time::Instant};
    if cfg!(debug_assertions) {
        panic!("Use the release test artifact");
    }
    let order = std::env::var("MATRUST_ORIGINAL_COPY_ORDER").unwrap_or_else(|_| "normal".into());
    assert!(matches!(order.as_str(), "normal" | "reverse"));
    let cancel = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    struct Fixture {
        format: &'static str,
        size: [u32; 2],
        source: PathBuf,
        encoded: Vec<u8>,
        original: Original,
    }
    let mut fixtures = Vec::new();
    for (width, height) in [(64, 64), (4096, 2048)] {
        let pixels = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 253) as u8, ((x + 3 * y) % 255) as u8])
        });
        for format in ["bmp", "tiff"] {
            let source = root.join(format!("source-{width}-{height}.{format}"));
            pixels.save(&source).unwrap();
            let encoded = fs::read(&source).unwrap();
            assert!(encoded.len() < MAX_FILE_BYTES);
            assert_eq!(image::open(&source).unwrap().into_rgb8(), pixels);
            let preview = if format == "tiff" {
                let (preview, navigation) =
                    crate::image_pages::open(&source, 0, 128, &cancel).unwrap();
                assert_eq!(navigation.document.page_count, 1);
                preview
            } else {
                crate::image_load::load(&source, 128, &cancel).unwrap()
            };
            assert_eq!(preview.original_size, [width, height]);
            let original = preview.original.unwrap();
            assert_eq!(original.len, encoded.len());
            assert_eq!(&original.digest, blake3::hash(&encoded).as_bytes());
            assert_eq!(original.bytes.is_some(), format == "tiff");
            if let Some(bytes) = &original.bytes {
                assert_eq!(bytes.as_slice(), encoded.as_slice());
            }
            println!(
                "ORIGINAL_COPY_FIXTURE\t{format}\t{width}\t{height}\t{}\t{}",
                encoded.len(),
                blake3::hash(&encoded)
            );
            fixtures.push(Fixture {
                format,
                size: [width, height],
                source,
                encoded,
                original,
            });
        }
    }
    // Pools and all encoded fixtures are constructed before any measured work.
    let pools: Vec<_> = [1, 8]
        .into_iter()
        .map(|workers| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| assert_eq!(rayon::current_num_threads(), workers));
            (workers, pool)
        })
        .collect();
    let mut groups = Vec::new();
    for fixture in 0..fixtures.len() {
        for pool in 0..pools.len() {
            for operation in ["identity", "prepare", "commit"] {
                groups.push((fixture, pool, operation));
            }
        }
    }
    assert_eq!(groups.len(), 24);
    if order == "reverse" {
        groups.reverse();
    }
    println!(
        "ORIGINAL_COPY\tformat\twidth\theight\tencoded_bytes\troute\tworkers\toperation\torder\tphase\tsample\telapsed_ns"
    );
    for (group, (fixture_index, pool_index, operation)) in groups.into_iter().enumerate() {
        let fixture = &fixtures[fixture_index];
        let (workers, pool) = &pools[pool_index];
        let route = if fixture.original.bytes.is_some() {
            "retained_tiff"
        } else {
            "streamed_bmp"
        };
        for repetition in 0..9 {
            let destination = root.join(format!("copy-{group}-{repetition}.{}", fixture.format));
            assert!(!destination.exists());
            let make_request = || Prepare {
                source: fixture.source.clone(),
                destination: destination.clone(),
                original: fixture.original.clone(),
            };
            let elapsed = pool.install(|| {
                assert_eq!(rayon::current_num_threads(), *workers);
                match operation {
                    "identity" => {
                        let start = Instant::now();
                        let actual =
                            Original::identity(black_box(&fixture.encoded), &cancel).unwrap();
                        let elapsed = start.elapsed();
                        assert_eq!(actual.len, fixture.original.len);
                        assert_eq!(actual.digest, fixture.original.digest);
                        assert!(actual.bytes.is_none());
                        elapsed
                    }
                    "prepare" => {
                        let request = make_request();
                        let start = Instant::now();
                        let plan = prepare(black_box(request), &cancel).unwrap();
                        let elapsed = start.elapsed();
                        assert!(!plan.replaces());
                        assert_eq!(plan.destination(), destination);
                        assert!(!destination.exists());
                        drop(plan);
                        elapsed
                    }
                    "commit" => {
                        let plan = prepare(make_request(), &cancel).unwrap();
                        assert!(!plan.replaces());
                        let start = Instant::now();
                        let actual = save(black_box(plan), &cancel).unwrap();
                        let elapsed = start.elapsed();
                        assert_eq!(actual, destination);
                        assert_eq!(fs::read(&actual).unwrap(), fixture.encoded);
                        fs::remove_file(actual).unwrap();
                        elapsed
                    }
                    _ => unreachable!(),
                }
            });
            // Keep raw warmups and retained observations, including zero/tied values.
            println!(
                "ORIGINAL_COPY\t{}\t{}\t{}\t{}\t{route}\t{workers}\t{operation}\t{order}\t{}\t{}\t{}",
                fixture.format,
                fixture.size[0],
                fixture.size[1],
                fixture.encoded.len(),
                if repetition < 2 { "warmup" } else { "retained" },
                if repetition < 2 {
                    repetition
                } else {
                    repetition - 2
                },
                elapsed.as_nanos()
            );
            assert!(!destination.exists());
        }
        assert_eq!(fs::read(&fixture.source).unwrap(), fixture.encoded);
        if let Some(bytes) = &fixture.original.bytes {
            let alias = fixture.original.clone();
            assert!(Arc::ptr_eq(bytes, alias.bytes.as_ref().unwrap()));
            assert_eq!(bytes.as_slice(), fixture.encoded.as_slice());
        }
    }
}
