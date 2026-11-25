use matrust::{Array, Engine};

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/video/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn video_raw_metadata() {
    let mut e = Engine::default();
    e.execute(&format!(
        "v=VideoReader('{}'); n=v.NumFrames; w=v.Width; h=v.Height;",
        fixture("bgr24.avi")
    ))
    .unwrap();
    for (name, expected) in [("n", 6.), ("w", 64.), ("h", 48.)] {
        let Some(Array::Numeric(m)) = e.array(name) else {
            panic!("missing {name}")
        };
        assert_eq!(m.data(), &[expected]);
    }
}

fn number(e: &Engine, name: &str) -> f64 {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data()[0],
        Array::Logical(m) => f64::from(m.data()[0]),
        other => panic!("{name}: {other:?}"),
    }
}
fn bytes(e: &Engine, name: &str) -> (Vec<usize>, Vec<u8>) {
    let Some(Array::Typed(t)) = e.array(name) else {
        panic!("missing typed array {name}")
    };
    assert_eq!(t.class(), matrust::NumericClass::Uint8);
    (
        t.dims().to_vec(),
        (0..t.len()).map(|i| t.int(i) as u8).collect(),
    )
}
fn interleaved(e: &Engine, name: &str, channels: usize) -> Vec<u8> {
    let (dims, b) = bytes(e, name);
    let h = dims[0];
    let w = dims[1];
    let pixels = h * w;
    let mut out = vec![0; b.len()];
    for f in 0..b.len() / (pixels * channels) {
        for y in 0..h {
            for x in 0..w {
                for c in 0..channels {
                    out[f * pixels * channels + (y * w + x) * channels + c] =
                        b[f * pixels * channels + y + x * h + c * pixels];
                }
            }
        }
    }
    out
}
fn open(name: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(&format!("v=VideoReader('{}');", fixture(name)))
        .unwrap();
    e
}

#[test]
fn video_uncompressed_oracles() {
    for (name, reference, c) in [
        ("bgr24", "bgr24.rgb", 3),
        ("bgrx", "bgrx.rgb", 3),
        ("odd", "odd.rgb", 3),
        ("gray", "gray.gray", 1),
        ("pal8", "pal8.rgb", 3),
        ("bottomup", "bgr24.rgb", 3),
    ] {
        let mut e = open(&format!("{name}.avi"));
        e.execute("a=read(v);").unwrap();
        let expected = std::fs::read(fixture(reference)).unwrap();
        assert_eq!(interleaved(&e, "a", c), expected, "{name}");
    }
}

#[test]
fn video_mjpeg_oracles() {
    for (name, reference) in [
        ("mjpeg", "mjpeg.rgb"),
        ("standard", "standard.rgb"),
        ("nodht", "standard.rgb"),
        ("mjpeg420", "mjpeg420.rgb"),
    ] {
        let mut e = open(&format!("{name}.avi"));
        e.execute("a=read(v);").unwrap();
        let actual = interleaved(&e, "a", 3);
        let expected = std::fs::read(fixture(reference)).unwrap();
        assert_eq!(actual.len(), expected.len());
        let diffs: Vec<_> = actual
            .iter()
            .zip(expected)
            .map(|(&a, b)| a.abs_diff(b))
            .collect();
        let max = *diffs.iter().max().unwrap();
        let mean = diffs.iter().map(|&x| f64::from(x)).sum::<f64>() / diffs.len() as f64;
        eprintln!("{name} MJPEG max={max}, mean={mean:.6}");
        assert!(max <= 2 && mean <= 0.5, "{name}: max={max}, mean={mean}");
    }
}

#[test]
fn video_indexes_and_drops() {
    let reference = std::fs::read(fixture("bgr24.rgb")).unwrap();
    for name in ["absolute", "scan", "opendml", "multistream"] {
        let mut e = open(&format!("{name}.avi"));
        e.execute("a=v.read();").unwrap();
        assert_eq!(interleaved(&e, "a", 3), reference, "{name}");
    }
    for name in ["drop.avi", "indexdrop.avi"] {
        let mut e = open(name);
        e.execute("a=read(v,2); b=read(v,3); n=v.NumFrames;")
            .unwrap();
        assert_eq!(bytes(&e, "a"), bytes(&e, "b"));
        assert_eq!(number(&e, "n"), 6.);
    }
}

#[test]
fn video_properties_methods_and_shared_handle() {
    let mut e = Engine::default();
    e.execute(&format!("v=VideoReader('{}','CurrentTime',0.4,'Tag','hello','UserData',{{42}}); p=v; k=v.FrameRate; d=v.Duration; bits=v.BitsPerPixel; a=readFrame(p); t=v.CurrentTime; b=read(v,3); same=isequal(a,b); unchanged=v.CurrentTime; r=v.read([2 4]); last=read(v,Inf); final=read(v,6); z=isequal(last,final); v.Tag='updated'; tag=p.Tag; p.UserData=magic(2); u=v.UserData;",fixture("bgr24.avi"))).unwrap();
    assert_eq!(number(&e, "k"), 5.);
    assert_eq!(number(&e, "d"), 1.2);
    assert_eq!(number(&e, "bits"), 24.);
    assert!((number(&e, "t") - 0.6).abs() < 1e-12);
    assert_eq!(number(&e, "t"), number(&e, "unchanged"));
    assert_eq!(bytes(&e, "a"), bytes(&e, "b"));
    assert_eq!(bytes(&e, "last"), bytes(&e, "final"));
    assert_eq!(bytes(&e, "r").0, [48, 64, 3, 3]);
    let output = e.execute("v").unwrap().output.join("\n");
    for prop in [
        "VideoReader with properties",
        "FrameRate",
        "Colormap",
        "updated",
        "UserData",
    ] {
        assert!(output.contains(prop), "{output}");
    }
    for source in [
        "v.Width=1;",
        "v.CurrentTime=-1;",
        "v.CurrentTime=Inf;",
        "v.CurrentTime=NaN;",
        "v.CurrentTime=2;",
        "read(v,0);",
        "read(v,[4 2]);",
        "readFrame(v,'invalid');",
        "parse(v);",
        "copy(v);",
    ] {
        assert!(e.execute(source).is_err(), "{source}");
    }
    e.execute("v.CurrentTime=0; count=0; while v.hasFrame(), f=v.readFrame(); count=count+1; end; t=v.CurrentTime;").unwrap();
    assert_eq!(number(&e, "count"), 6.);
    assert_eq!(number(&e, "t"), 1.2);
    let error = e.execute("readFrame(v);").unwrap_err();
    assert!(error.contains("No more frames"), "{error}");
    e.execute("v.CurrentTime=v.Duration; available=hasFrame(v);")
        .unwrap();
    assert!(matches!(e.array("available"), Some(Array::Logical(m)) if m.data()==[0]));
}

#[test]
fn video_native_palette_gray_and_formats() {
    for (file, format, map_rows) in [("gray.avi", "Grayscale", 0), ("pal8.avi", "Indexed", 256)] {
        let mut e = open(file);
        e.execute("fmt=v.VideoFormat; map=v.Colormap; a=readFrame(v,'native'); pixels=a.cdata; cm=a.colormap; all=read(v,[2 3],'native'); count=numel(all); rgb=read(v,1);").unwrap();
        assert_eq!(bytes(&e, "pixels").0, [48, 64]);
        assert_eq!(number(&e, "count"), 2.);
        let Some(Array::Character(f)) = e.array("fmt") else {
            panic!("format")
        };
        assert_eq!(String::from_utf16_lossy(f.units()), format);
        assert_eq!(e.array("map").unwrap().rows(), map_rows);
        assert_eq!(e.array("cm").unwrap().rows(), map_rows);
    }
    let mut e = Engine::default();
    e.execute("f=VideoReader.getFileFormats(); ext=f.Extension; video=f.ContainsVideo; audio=f.ContainsAudio;").unwrap();
    let Some(Array::Character(ext)) = e.array("ext") else {
        panic!("extension")
    };
    assert_eq!(String::from_utf16_lossy(ext.units()), "avi");
    assert!(matches!(e.array("video"),Some(Array::Logical(m)) if m.data()==[1]));
    assert!(matches!(e.array("audio"),Some(Array::Logical(m)) if m.data()==[0]));
}

#[test]
fn video_corruption_truncation_and_unsupported() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".crew/video-corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    let original = std::fs::read(fixture("bgr24.avi")).unwrap();
    for (i, n) in [0, 1, 7, 11, 64, original.len() / 2, original.len() - 1]
        .into_iter()
        .enumerate()
    {
        let file = dir.join(format!("truncated-{i}.avi"));
        std::fs::write(&file, &original[..n]).unwrap();
        let mut e = Engine::default();
        assert!(
            e.execute(&format!("v=VideoReader('{}'); a=read(v);", file.display()))
                .is_err(),
            "truncation {n}"
        );
    }
    let mut unsupported = original.clone();
    let p = unsupported.windows(4).position(|x| x == b"strf").unwrap();
    unsupported[p + 24..p + 28].copy_from_slice(b"H264");
    let file = dir.join("codec.avi");
    std::fs::write(&file, unsupported).unwrap();
    let error = Engine::default()
        .execute(&format!("v=VideoReader('{}');", file.display()))
        .unwrap_err();
    assert!(
        error.contains("H264") && error.contains("ffmpeg"),
        "{error}"
    );
    for kind in 0..2 {
        let mut b = original.clone();
        if kind == 0 {
            let p = b.windows(4).position(|x| x == b"idx1").unwrap();
            b[p + 16..p + 20].copy_from_slice(&u32::MAX.to_le_bytes());
        } else {
            let p = b.windows(4).position(|x| x == b"strf").unwrap();
            b[p + 12..p + 16].copy_from_slice(&i32::MAX.to_le_bytes());
        }
        let file = dir.join(format!("corrupt-{kind}.avi"));
        std::fs::write(&file, b).unwrap();
        assert!(
            Engine::default()
                .execute(&format!("v=VideoReader('{}'); a=read(v);", file.display()))
                .is_err()
        );
    }
    let mjpeg = std::fs::read(fixture("mjpeg.avi")).unwrap();
    let start = mjpeg.windows(2).position(|b| b == [255, 216]).unwrap();
    let end = start
        + mjpeg[start..]
            .windows(2)
            .position(|b| b == [255, 217])
            .unwrap()
        + 2;
    for cut in [1, 16, 128, (end - start) / 2, end - start - 1] {
        let mut b = mjpeg.clone();
        b[start + cut..end].fill(0);
        let file = dir.join(format!("jpeg-truncated-{cut}.avi"));
        std::fs::write(&file, b).unwrap();
        assert!(
            Engine::default()
                .execute(&format!(
                    "v=VideoReader('{}'); a=readFrame(v);",
                    file.display()
                ))
                .is_err()
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn video_memory_admission() {
    use matrust::engine::Limits;
    let mut limits = Limits::browser();
    limits.evaluation_units = Some(1024);
    let mut e = Engine::with_limits(limits);
    let error = e
        .execute(&format!("v=VideoReader('{}');", fixture("bgr24.avi")))
        .unwrap_err();
    assert!(
        error.contains("memory") || error.contains("budget"),
        "{error}"
    );
    let mut limits = Limits::browser();
    limits.memory_bytes = 150_000;
    let mut e = Engine::with_limits(limits);
    e.execute(&format!("v=VideoReader('{}');", fixture("bgr24.avi")))
        .unwrap();
    let error = e.execute("a=read(v);").unwrap_err();
    assert!(error.contains("memory"), "{error}");
}

#[test]
fn video_cancellation_during_reads() {
    let mut e = open("mjpeg.avi");
    let cancel = e.cancellation();
    let run =
        std::thread::spawn(move || e.execute("for k=1:10000, v.CurrentTime=0; a=read(v); end;"));
    std::thread::sleep(std::time::Duration::from_millis(20));
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let error = run.join().unwrap().unwrap_err();
    assert!(error.to_lowercase().contains("cancel"), "{error}");
}

#[test]
fn video_handle_keeps_no_file_open() {
    let file =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".crew/video-file-lifetime.avi");
    std::fs::copy(fixture("bgr24.avi"), &file).unwrap();
    let mut e = Engine::default();
    e.execute(&format!("v=VideoReader('{}');", file.display()))
        .unwrap();
    std::fs::remove_file(&file).unwrap();
    assert!(e.execute("a=readFrame(v);").is_err());
    e.execute("t=v.CurrentTime;").unwrap();
    assert_eq!(number(&e, "t"), 0.);
}

#[test]
fn video_constructor_text_and_userdata() {
    let mut e = Engine::default();
    e.execute(&format!("file=string('{}'); v=VideoReader(file,'Tag',\"hello\",'UserData',ones(2,2,2)); v.Tag=\"changed\"; tag=v.Tag; u=v.UserData;", fixture("bgr24.avi"))).unwrap();
    assert_eq!(e.array("u").unwrap().dims().as_ref(), &[2, 2, 2]);
    let Some(Array::Character(tag)) = e.array("tag") else {
        panic!("Tag")
    };
    assert_eq!(String::from_utf16_lossy(tag.units()), "changed");
}

#[test]
fn video_userdata_traces_nested_handles() {
    let mut e = open("bgr24.avi");
    e.execute("m=containers.Map({'key'},{7}); v.UserData=m; clear m; child=v.UserData; result=child('key');").unwrap();
    assert_eq!(number(&e, "result"), 7.);
    e.execute("v.UserData=v; clear child; clear v;").unwrap();
}

// Recreates review finding #2 directly from RIFF/AVIPALCHANGE bytes. The
// published AVI layout and the review's ffmpeg oracle give black then red.
fn palette_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if !data.len().is_multiple_of(2) {
        out.push(0);
    }
    out
}
fn palette_list(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    palette_chunk(b"LIST", &[id.as_slice(), data].concat())
}
fn palette_avi(movi: &[u8], frames: u32, gray: bool, entries: u32, index: u8) -> Vec<u8> {
    let words = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
    let avih = words(&[200000, 0, 0, 0, frames, 0, 1, 4, 1, 1, 0, 0, 0, 0]);
    let mut strh = b"vidsDIB ".to_vec();
    strh.extend(words(&[0x10000, 0, 0, 1, 5, 0, frames, 4, u32::MAX, 0]));
    strh.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
    let mut strf = words(&[40, 1, 1]);
    strf.extend_from_slice(&[1, 0, 8, 0]);
    strf.extend(words(&[0, 4, 0, 0, entries, 0]));
    for i in 0..entries {
        strf.extend_from_slice(&[if gray { i as u8 } else { 0 }; 3]);
        strf.push(0);
    }
    let hdrl = palette_list(
        b"hdrl",
        &[
            palette_chunk(b"avih", &avih),
            palette_list(
                b"strl",
                &[palette_chunk(b"strh", &strh), palette_chunk(b"strf", &strf)].concat(),
            ),
        ]
        .concat(),
    );
    let mut idx = Vec::new();
    let mut pos = 0;
    let mut count = 0u32;
    while pos < movi.len() {
        let size = u32::from_le_bytes(movi[pos + 4..pos + 8].try_into().unwrap());
        if &movi[pos..pos + 4] == b"00db" {
            if index == 1 {
                idx.extend_from_slice(b"00db");
                idx.extend(words(&[0x10, pos as u32 + 4, size]));
            } else if index == 2 {
                idx.extend(words(&[pos as u32 + 8, size]));
            }
            count += 1;
        }
        pos += 8 + size as usize + (size as usize & 1);
    }
    let mut body = [b"AVI ".to_vec(), hdrl.clone(), palette_list(b"movi", movi)].concat();
    if index == 1 {
        body.extend(palette_chunk(b"idx1", &idx));
    } else if index == 2 {
        let mut header = vec![2, 0, 0, 1];
        header.extend(words(&[count]));
        header.extend_from_slice(b"00db");
        header.extend_from_slice(&(24 + hdrl.len() as u64).to_le_bytes());
        header.extend(words(&[0]));
        header.extend(idx);
        body.extend(palette_chunk(b"ix00", &header));
    }
    palette_chunk(b"RIFF", &body)
}
fn palette_open(data: &[u8]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changes.avi");
    std::fs::write(&path, data).unwrap();
    let mut e = Engine::default();
    e.execute(&format!("v=VideoReader('{}');", path.display()))
        .unwrap();
    (dir, e)
}

#[test]
fn video_palette_changes_all_read_modes() {
    let frame = palette_chunk(b"00db", &[0; 4]);
    let change = palette_chunk(b"00pc", &[0, 1, 0, 0, 255, 0, 0, 0]);
    for gray in [false, true] {
        for index in 0..=2 {
            let movi = [frame.clone(), change.clone(), frame.clone()].concat();
            let (_dir, mut e) = palette_open(&palette_avi(&movi, 2, gray, 256, index));
            e.execute("a=readFrame(v); b=readFrame(v); last=read(v,2); first=read(v,1); range=read(v,[1 2]); all=read(v); t=v.CurrentTime; movie=read(v,'native'); m1=movie(1).colormap; m2=movie(2).colormap; c1=movie(1).cdata; c2=movie(2).cdata; v.CurrentTime=0; n1=readFrame(v,'native'); n2=readFrame(v,'native'); same=isequal(n1,movie(1)) && isequal(n2,movie(2)); selected=read(v,2,'native'); selectedSame=isequal(selected,movie(2)); ranged=read(v,[1 2],'native'); rangeSame=isequal(ranged,movie);").unwrap();
            for name in ["a", "first"] {
                assert_eq!(bytes(&e, name).1, [0, 0, 0]);
            }
            for name in ["b", "last"] {
                assert_eq!(bytes(&e, name).1, [255, 0, 0]);
            }
            for name in ["range", "all"] {
                assert_eq!(bytes(&e, name).1, [0, 0, 0, 255, 0, 0]);
            }
            assert_eq!(number(&e, "t"), 0.4);
            assert_eq!(number(&e, "same"), 1.);
            assert_eq!(number(&e, "selectedSame"), 1.);
            assert_eq!(number(&e, "rangeSame"), 1.);
            assert_eq!(bytes(&e, "c1").1, [0]);
            assert_eq!(bytes(&e, "c2").1, [0]);
            let Some(Array::Numeric(m1)) = e.array("m1") else {
                panic!("m1")
            };
            let Some(Array::Numeric(m2)) = e.array("m2") else {
                panic!("m2")
            };
            assert_eq!(m1.data()[0], 0.);
            assert_eq!(m2.data()[0], 1.);
            assert_eq!(m2.data()[256], 0.);
            assert_eq!(m2.data()[512], 0.);
        }
    }
}

#[test]
fn video_palette_changes_order_full_table_and_growth() {
    let frame = palette_chunk(b"00db", &[255, 0, 0, 0]);
    let mut full = vec![0, 0, 0, 0]; // bNumEntries=0 means 256.
    full.extend([0, 255, 0, 0].repeat(256));
    let movi = [
        palette_chunk(b"00pc", &full),
        palette_chunk(b"01pc", &[255]), // another stream cannot affect us
        palette_list(
            b"rec ",
            &[
                frame.clone(),
                palette_chunk(b"00pc", &[255, 1, 0, 0, 255, 0, 0, 0]),
                palette_chunk(b"00pc", &[255, 1, 0, 0, 0, 0, 255, 0]),
                frame,
            ]
            .concat(),
        ),
    ]
    .concat();
    let (_dir, mut e) = palette_open(&palette_avi(&movi, 2, false, 1, 0));
    e.execute("a=read(v); m=read(v,'native'); map=m(2).colormap; rows=size(map,1);")
        .unwrap();
    assert_eq!(bytes(&e, "a").1, [0, 255, 0, 0, 0, 255]);
    assert_eq!(number(&e, "rows"), 256.);
}

#[test]
fn video_palette_changes_reject_malformed() {
    for payload in [
        vec![],
        vec![0, 1, 0],
        vec![0, 1, 0, 0],
        vec![255, 2, 0, 0, 1, 2, 3, 0, 4, 5, 6, 0],
        vec![0, 1, 1, 0, 1, 2, 3, 0],
        vec![0, 1, 0, 0, 1, 2, 3, 0, 0],
        vec![1, 0, 0, 0],
        vec![0; 1032],
    ] {
        let movi = [
            palette_chunk(b"00db", &[0; 4]),
            palette_chunk(b"00pc", &payload),
        ]
        .concat();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("malformed.avi");
        std::fs::write(&path, palette_avi(&movi, 1, false, 256, 0)).unwrap();
        let error = Engine::default()
            .execute(&format!("VideoReader('{}');", path.display()))
            .unwrap_err();
        assert!(error.contains("palette-change"), "{error}");
    }
}

#[cfg(unix)]
#[test]
fn video_nonregular_inputs_and_reopens_do_not_block() {
    use std::{
        os::unix::{fs::symlink, net::UnixListener},
        sync::mpsc,
        time::Duration,
    };
    for kind in ["fifo", "socket", "directory", "device"] {
        for reopen in [false, true] {
            let (send, recv) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("nonregular.avi");
                let mut e = Engine::default();
                if reopen {
                    std::fs::copy(fixture("bgr24.avi"), &path).unwrap();
                    e.execute(&format!("v=VideoReader('{}');", path.display()))
                        .unwrap();
                    std::fs::remove_file(&path).unwrap();
                }
                let _socket = match kind {
                    "fifo" => {
                        assert!(
                            std::process::Command::new("mkfifo")
                                .arg(&path)
                                .status()
                                .unwrap()
                                .success()
                        );
                        None
                    }
                    "socket" => Some(UnixListener::bind(&path).unwrap()),
                    "directory" => {
                        std::fs::create_dir(&path).unwrap();
                        None
                    }
                    "device" => {
                        symlink("/dev/null", &path).unwrap();
                        None
                    }
                    _ => unreachable!(),
                };
                let code = if reopen {
                    "a=readFrame(v);".into()
                } else {
                    format!("v=VideoReader('{}');", path.display())
                };
                let start = std::time::Instant::now();
                let error = e.execute(&code).unwrap_err();
                assert!(start.elapsed() < Duration::from_secs(1));
                assert!(
                    error.contains("regular file"),
                    "{kind} reopen={reopen}: {error}"
                );
                if reopen {
                    e.execute("t=v.CurrentTime;").unwrap();
                    assert_eq!(number(&e, "t"), 0.);
                }
                send.send(()).unwrap();
            });
            recv.recv_timeout(Duration::from_secs(1))
                .expect("nonregular video open must finish within one second");
            worker.join().unwrap();
        }
    }
}

#[test]
fn video_palette_changes_coalesce_and_obey_memory_budget() {
    let frame = palette_chunk(b"00db", &[0; 4]);
    let change = palette_chunk(b"00pc", &[0, 1, 0, 0, 255, 0, 0, 0]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many-changes.avi");
    for per_frame in [false, true] {
        let mut movi = frame.clone();
        for _ in 0..2000 {
            movi.extend_from_slice(&change);
            if per_frame {
                movi.extend_from_slice(&frame);
            }
        }
        movi.extend_from_slice(&frame);
        std::fs::write(
            &path,
            palette_avi(&movi, if per_frame { 2002 } else { 2 }, false, 256, 0),
        )
        .unwrap();
        let mut limits = matrust::engine::Limits::browser();
        limits.memory_bytes = 200_000;
        limits.evaluation_units = None;
        let mut e = Engine::with_limits(limits);
        let result = e.execute(&format!(
            "v=VideoReader('{}'); a=read(v,2);",
            path.display()
        ));
        if per_frame {
            let error = result.unwrap_err();
            assert!(error.contains("memory"), "{error}");
        } else {
            result.unwrap();
            assert_eq!(bytes(&e, "a").1, [255, 0, 0]);
        }
    }
}

#[test]
fn video_palette_changes_precede_dropped_chunks_and_avix() {
    let frame = palette_chunk(b"00db", &[0; 4]);
    let red = palette_chunk(b"00pc", &[0, 1, 0, 0, 255, 0, 0, 0]);
    for index in 0..=2 {
        let movi = [
            frame.clone(),
            red.clone(),
            palette_chunk(b"00db", &[]),
            frame.clone(),
        ]
        .concat();
        let (_dir, mut e) = palette_open(&palette_avi(&movi, 3, false, 256, index));
        e.execute("a=read(v); b=read(v,2); m=read(v,2,'native'); map=m.colormap; r=map(1,1);")
            .unwrap();
        assert_eq!(bytes(&e, "a").1, [0, 0, 0, 255, 0, 0, 255, 0, 0]);
        assert_eq!(bytes(&e, "b").1, [255, 0, 0]);
        assert_eq!(number(&e, "r"), 1.);
    }
    let mut avi = palette_avi(&frame, 2, false, 256, 0);
    avi.extend(palette_chunk(
        b"RIFF",
        &[
            b"AVIX".to_vec(),
            palette_list(b"movi", &[red, frame].concat()),
        ]
        .concat(),
    ));
    let (_dir, mut e) = palette_open(&avi);
    e.execute("a=read(v);").unwrap();
    assert_eq!(bytes(&e, "a").1, [0, 0, 0, 255, 0, 0]);
}

#[test]
fn video_palette_changes_release_measurement() {
    let workers = std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(1);
    rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {

    let count = 4000;
    let mut movi = Vec::new();
    for i in 0..count {
        movi.extend(palette_chunk(
            b"00pc",
            &[0, 1, 0, 0, (i % 256) as u8, 0, 0, 0],
        ));
        movi.extend(palette_chunk(b"00db", &[0; 4]));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("palette-bench.avi");
    std::fs::write(&path, palette_avi(&movi, count, false, 256, 1)).unwrap();
    let mut limits = matrust::engine::Limits::browser();
    limits.evaluation_units = None;
    let memory = limits.memory_bytes;
    let mut e = Engine::with_limits(limits);
    let start = std::time::Instant::now();
    e.execute(&format!("v=VideoReader('{}');", path.display()))
        .unwrap();
    let opened = start.elapsed();
    let start = std::time::Instant::now();
    e.execute("a=read(v);").unwrap();
    let decoded = start.elapsed();
    let actual = bytes(&e, "a").1;
    for (i, rgb) in actual.as_chunks::<3>().0.iter().enumerate() {
        assert_eq!(*rgb, [(i % 256) as u8, 0, 0]);
    }
    eprintln!(
        "palette workers={} {count} 1x1 indexed frames: open {:.3} ms, RGB read {:.3} ms; memory={memory}, evaluation_units=None",
        rayon::current_num_threads(),
        opened.as_secs_f64() * 1000.,
        decoded.as_secs_f64() * 1000.
    );
    });
}
