use matrust::{Array, Engine};
use std::process::Command;
fn available() -> bool {
    let ok = Command::new("ffmpeg").arg("-version").output().is_ok()
        && Command::new("ffprobe").arg("-version").output().is_ok();
    if !ok {
        eprintln!("skipping: ffmpeg/ffprobe absent from PATH");
    }
    ok
}
fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/video/{name}", env!("CARGO_MANIFEST_DIR"))
}
fn num(e: &Engine, name: &str) -> f64 {
    match e.array(name).unwrap() {
        Array::Numeric(a) => a.data()[0],
        _ => panic!("number"),
    }
}
#[test]
fn video_ffmpeg_mp4_metadata() {
    if !available() {
        return;
    }
    let mut e = Engine::default();
    e.execute(&format!(
        "v=VideoReader('{}'); n=v.NumFrames; r=v.FrameRate; w=v.Width; h=v.Height;",
        fixture("ffmpeg-lossless.mp4")
    ))
    .unwrap();
    for (k, v) in [("n", 10.), ("r", 10.), ("w", 64.), ("h", 48.)] {
        assert_eq!(num(&e, k), v);
    }
}

fn dir() -> tempfile::TempDir {
    std::fs::create_dir_all(format!("{}/.crew", env!("CARGO_MANIFEST_DIR"))).unwrap();
    tempfile::Builder::new()
        .prefix("ffmpeg-")
        .tempdir_in(format!("{}/.crew", env!("CARGO_MANIFEST_DIR")))
        .unwrap()
}
fn quote(p: &std::path::Path) -> String {
    p.to_str().unwrap().replace('\'', "''")
}
fn raw(path: &std::path::Path, gray: bool) -> Vec<u8> {
    let result = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-f",
            "rawvideo",
            "-pix_fmt",
            if gray { "gray" } else { "rgb24" },
            "-threads",
            "1",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn interleaved(e: &Engine, name: &str) -> Vec<u8> {
    let Some(Array::Typed(t)) = e.array(name) else {
        panic!("uint8 array {name}")
    };
    let d = t.dims();
    let (h, w) = (d[0], d[1]);
    let channels = d.get(2).copied().unwrap_or(1);
    let mut out = vec![0; t.len()];
    for f in 0..t.len() / (w * h * channels) {
        for y in 0..h {
            for x in 0..w {
                for c in 0..channels {
                    out[f * w * h * channels + (y * w + x) * channels + c] =
                        t.int(f * w * h * channels + y + x * h + c * w * h) as u8;
                }
            }
        }
    }
    out
}
fn no_temp(path: &std::path::Path) {
    assert!(!std::fs::read_dir(path).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".matrust-video")
    }));
}
#[test]
fn video_ffmpeg_lossless_read_seek_and_fourcc() {
    if !available() {
        return;
    }
    for file in [
        "ffmpeg-lossless.mp4",
        "ffmpeg-lossless.mkv",
        "ffmpeg-lossless.mov",
        "ffmpeg-ffv1.avi",
        "ffmpeg-gray.mkv",
    ] {
        let path = std::path::PathBuf::from(fixture(file));
        let gray = file.contains("gray");
        let c = if gray { 1 } else { 3 };
        let expected = raw(&path, gray);
        let size = 64 * 48 * c;
        let mut e = Engine::default();
        e.execute(&format!("v=VideoReader('{}'); a=read(v); v.CurrentTime=0.3; b=readFrame(v); random=read(v,8); t=v.CurrentTime; next=readFrame(v);",quote(&path))).unwrap();
        assert_eq!(interleaved(&e, "a"), expected, "{file}");
        assert_eq!(interleaved(&e, "b"), expected[3 * size..4 * size], "{file}");
        assert_eq!(
            interleaved(&e, "random"),
            expected[7 * size..8 * size],
            "{file}"
        );
        assert_eq!(
            interleaved(&e, "next"),
            expected[4 * size..5 * size],
            "{file}"
        );
        assert!((num(&e, "t") - 0.4).abs() < 1e-12);
        e.execute(
            "v.CurrentTime=0; a=readFrame(v); b=readFrame(v); v.CurrentTime=0; c=readFrame(v);",
        )
        .unwrap();
        assert_eq!(interleaved(&e, "a"), interleaved(&e, "c"));
        assert_eq!(interleaved(&e, "b"), expected[size..2 * size]);
    }
}
#[test]
fn video_ffmpeg_missing_tools_and_cache_reset() {
    let directory = dir();
    let absent = quote(&directory.path().join("nonexistent"));
    let mut e = Engine::default();
    e.execute(&format!("setenv('MATRUST_FFMPEG','{absent}');"))
        .unwrap();
    let error = e
        .execute(&format!(
            "v=VideoReader('{}');",
            fixture("ffmpeg-lossless.mp4")
        ))
        .unwrap_err();
    assert!(
        error.contains("VideoReader: reading mp4 needs ffmpeg on PATH (not found). Native: AVI"),
        "{error}"
    );
    let error = e
        .execute("w=VideoWriter('missing.mp4','MPEG-4');")
        .unwrap_err();
    assert!(
        error.contains("VideoWriter: writing mp4 needs ffmpeg on PATH (not found)"),
        "{error}"
    );
    e.execute(
        "f=VideoReader.getFileFormats(); p=VideoWriter.getProfiles(); n=numel(f); m=numel(p);",
    )
    .unwrap();
    assert_eq!(num(&e, "n"), 1.);
    assert_eq!(num(&e, "m"), 4.);
    if available() {
        // Reset only this engine's cached lookup; the process environment is untouched.
        let tool = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| {
                p.join(if cfg!(windows) {
                    "ffmpeg.exe"
                } else {
                    "ffmpeg"
                })
            })
            .find(|p| p.is_file())
            .unwrap();
        e.execute(&format!(
            "setenv('MATRUST_FFMPEG','{}'); f=VideoReader.getFileFormats(); n=numel(f);",
            quote(&tool)
        ))
        .unwrap();
        assert!(num(&e, "n") > 1.);
        e.execute(&format!("setenv('MATRUST_FFPROBE','{absent}');"))
            .unwrap();
        assert!(
            e.execute(&format!(
                "VideoReader('{}');",
                fixture("ffmpeg-lossless.mp4")
            ))
            .unwrap_err()
            .contains("not found")
        );
    }
}
#[test]
fn video_ffmpeg_write_roundtrips_and_properties() {
    if !available() {
        return;
    }
    let directory = dir();
    for (profile, ext) in [
        ("MPEG-4", "mp4"),
        ("Archival", "mj2"),
        ("Motion JPEG 2000", "mj2"),
    ] {
        let path = directory.path().join(format!("{profile}.{ext}"));
        let mut e = Engine::default();
        e.execute(&format!(
            "r=VideoReader('{}'); a=read(r); w=VideoWriter('{}','{profile}'); w.FrameRate=10;",
            fixture("ffmpeg-lossless.mp4"),
            quote(&path)
        ))
        .unwrap();
        if profile == "MPEG-4" {
            // A smooth RGB ramp measures quantization without unavoidable
            // high-frequency 4:2:0 chroma loss dominating the metric.
            let mut data = Vec::new();
            for f in 0..10 {
                for c in 0..3 {
                    for x in 0..64 {
                        for y in 0..48 {
                            data.push(match c {
                                0 => x * 3 + f,
                                1 => y * 4 + f,
                                _ => (x + y) * 2 + f,
                            } as i128);
                        }
                    }
                }
            }
            let t = matrust::TypedArray::from_ints(
                matrust::NumericClass::Uint8,
                48,
                64 * 3 * 10,
                data.into_iter(),
            )
            .unwrap()
            .with_dims(&[48, 64, 3, 10])
            .unwrap();
            e.insert_array("a", Array::Typed(t)).unwrap();
            e.execute("w.Quality=100;").unwrap();
        } else {
            e.execute(
                "bits=w.MJ2BitDepth; lossless=w.LosslessCompression; ratio=w.CompressionRatio;",
            )
            .unwrap();
            assert_eq!(num(&e, "bits"), 8.);
            assert_eq!(
                num(&e, "lossless"),
                if profile == "Archival" { 1. } else { 0. }
            );
            assert_eq!(num(&e, "ratio"), 10.);
        }
        e.execute("writeVideo(w,a); close(w);").unwrap();
        no_temp(directory.path());
        let expected = interleaved(&e, "a");
        let actual = raw(&path, false);
        assert_eq!(actual.len(), expected.len());
        if profile == "Archival" {
            assert_eq!(actual, expected);
        }
        if profile == "MPEG-4" {
            let mse = actual
                .iter()
                .zip(&expected)
                .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
                .sum::<f64>()
                / actual.len() as f64;
            let psnr = 10. * (255f64.powi(2) / mse).log10();
            eprintln!("MPEG-4 PSNR {psnr:.3} dB");
            assert!(psnr >= 30., "{psnr}");
        }
        e.execute(&format!(
            "v=VideoReader('{}'); n=v.NumFrames; rate=v.FrameRate;",
            quote(&path)
        ))
        .unwrap();
        assert_eq!(num(&e, "n"), 10.);
        assert_eq!(num(&e, "rate"), 10.);
    }
}
#[test]
fn video_ffmpeg_hostile_filenames() {
    if !available() {
        return;
    }
    let directory = dir();
    let names = vec!["-y.mp4", "sp ace'squote.mp4"];
    #[cfg(unix)]
    let names = {
        let mut names = names;
        names.extend([
            "sp ace's\"quote.mp4",
            "concat:a|b",
            "http:evil.mp4",
            "pipe:1",
            "line\nbreak.mp4",
            "$(touch PWNED).mp4",
        ]);
        names
    };
    for name in names {
        let path = directory.path().join(name);
        std::fs::copy(fixture("ffmpeg-lossless.mp4"), &path).unwrap();
        let mut e = Engine::default();
        e.set_current_folder(directory.path()).unwrap();
        // insert avoids language string escaping being mistaken for process escaping.
        e.insert_array(
            "filename",
            Array::Character(matrust::CharacterRow::from_text(name, &e.cancellation()).unwrap()),
        )
        .unwrap();
        e.execute("r=VideoReader(filename); a=readFrame(r); w=VideoWriter(filename,'MPEG-4'); w.Quality=100; writeVideo(w,a); close(w); actual=[w.Path filesep w.Filename]; r=VideoReader(actual); n=r.NumFrames;").unwrap();
        assert_eq!(num(&e, "n"), 1.);
        no_temp(directory.path());
    }
    assert!(!directory.path().join("PWNED").exists());
}

#[cfg(unix)]
fn fake(directory: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(
        &path,
        format!("#!/usr/bin/python3\nimport sys, os, time, json\n{body}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}
#[cfg(unix)]
fn override_tool(e: &mut Engine, key: &str, path: &std::path::Path) {
    e.execute(&format!("setenv('{key}','{}');", quote(path)))
        .unwrap();
}
#[cfg(target_os = "linux")]
fn gone(pid: u32) {
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "child {pid} was not reaped"
    );
}
#[cfg(unix)]
const META: &str = r#"{'streams':[{'codec_type':'video','width':64,'height':48,'avg_frame_rate':'10/1','pix_fmt':'rgb24','nb_frames':'10','duration':'1'}]}"#;
#[test]
#[cfg(unix)]
fn video_ffmpeg_stderr_json_caps_and_dimensions() {
    if !available() {
        return;
    }
    let directory = dir();
    let mut e = Engine::default();
    let safe = fake(
        directory.path(),
        "probe-safe",
        &format!(
            "assert sys.stdin.buffer.read() == b''\nassert '-hide_banner' in sys.argv and '-loglevel' in sys.argv\nassert sys.argv[sys.argv.index('-protocol_whitelist')+1] == 'file'\nassert any(a.startswith('file:/') for a in sys.argv)\nprint(json.dumps({META}))"
        ),
    );
    override_tool(&mut e, "MATRUST_FFPROBE", &safe);
    e.execute(&format!(
        "v=VideoReader('{}');",
        fixture("ffmpeg-lossless.mp4")
    ))
    .unwrap();
    let bad = fake(
        directory.path(),
        "probe-stderr",
        "sys.stderr.write('  SENTINEL '+ 'x'*200000); sys.stderr.flush(); sys.exit(7)",
    );
    override_tool(&mut e, "MATRUST_FFPROBE", &bad);
    let error = e
        .execute(&format!(
            "VideoReader('{}');",
            fixture("ffmpeg-lossless.mp4")
        ))
        .unwrap_err();
    assert!(
        error.contains("SENTINEL") && error.contains("stderr truncated"),
        "{error}"
    );
    assert!(error.len() < 34000, "{}", error.len());
    let huge = fake(
        directory.path(),
        "probe-json",
        "sys.stdout.write('x'*2000000); sys.stdout.flush()",
    );
    override_tool(&mut e, "MATRUST_FFPROBE", &huge);
    assert!(
        e.execute(&format!(
            "VideoReader('{}');",
            fixture("ffmpeg-lossless.mp4")
        ))
        .unwrap_err()
        .contains("capture limit")
    );
    let large = fake(
        directory.path(),
        "probe-large",
        &format!("d={META}; d['streams'][0]['width']=2147483647; print(json.dumps(d))"),
    );
    override_tool(&mut e, "MATRUST_FFPROBE", &large);
    assert!(
        e.execute(&format!(
            "VideoReader('{}');",
            fixture("ffmpeg-lossless.mp4")
        ))
        .unwrap_err()
        .contains("dimensions")
    );
    let invalid = fake(directory.path(), "probe-invalid", "print('{}')");
    override_tool(&mut e, "MATRUST_FFPROBE", &invalid);
    assert!(
        e.execute(&format!(
            "VideoReader('{}');",
            fixture("ffmpeg-lossless.mp4")
        ))
        .unwrap_err()
        .contains("no video stream")
    );
}
#[test]
#[cfg(target_os = "linux")]
fn video_ffmpeg_cancellation_mid_write_reaps_and_removes_temp() {
    use std::sync::atomic::Ordering;
    let directory = dir();
    let pidfile = directory.path().join("pid");
    let output = directory.path().join("cancel.mp4");
    std::fs::write(&output, b"old destination").unwrap();
    let tool = fake(
        directory.path(),
        "hung-encoder",
        &format!(
            "if '-encoders' in sys.argv:\n print(' V..... libx264 fake encoder')\nelse:\n open({:?},'w').write(str(os.getpid()))\n while True: time.sleep(0.02)",
            pidfile.to_str().unwrap()
        ),
    );
    let mut e = Engine::default();
    override_tool(&mut e, "MATRUST_FFMPEG", &tool);
    e.execute(&format!(
        "w=VideoWriter('{}','MPEG-4'); a=uint8(zeros(48,64,3,20));",
        quote(&output)
    ))
    .unwrap();
    let cancel = e.cancellation();
    let childpid = pidfile.clone();
    let worker = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        while !childpid.exists() && start.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        cancel.store(true, Ordering::Relaxed);
    });
    let start = std::time::Instant::now();
    let error = e.execute("writeVideo(w,a);").unwrap_err();
    worker.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert!(start.elapsed().as_secs() < 5);
    let pid = std::fs::read_to_string(pidfile).unwrap().parse().unwrap();
    gone(pid);
    no_temp(directory.path());
    assert_eq!(std::fs::read(output).unwrap(), b"old destination");
}
#[test]
#[cfg(target_os = "linux")]
fn video_ffmpeg_stream_lifetime_seek_collection_and_drop() {
    if !available() {
        return;
    }
    let directory = dir();
    let pidfile = directory.path().join("pid");
    let argsfile = directory.path().join("args");
    let tool = fake(
        directory.path(),
        "stream",
        &format!(
            "open({:?},'w').write(str(os.getpid()))\nopen({:?},'w').write(json.dumps(sys.argv[1:]))\nsys.stdout.buffer.write(bytes([42])*(64*48*3)); sys.stdout.flush()\nwhile True: time.sleep(0.02)",
            pidfile.to_str().unwrap(),
            argsfile.to_str().unwrap()
        ),
    );
    let mut e = Engine::default();
    override_tool(&mut e, "MATRUST_FFMPEG", &tool);
    e.execute(&format!(
        "v=VideoReader('{}'); a=readFrame(v);",
        fixture("ffmpeg-lossless.mp4")
    ))
    .unwrap();
    let pid = std::fs::read_to_string(&pidfile).unwrap().parse().unwrap();
    assert!(interleaved(&e, "a").iter().all(|b| *b == 42));
    let args: Vec<String> = serde_json::from_slice(&std::fs::read(argsfile).unwrap()).unwrap();
    assert!(
        args.windows(4)
            .any(|v| v == ["-nostdin", "-hide_banner", "-loglevel", "error"])
    );
    assert!(
        args.windows(2)
            .any(|v| v == ["-protocol_whitelist", "file"])
    );
    assert!(args.iter().any(|a| a.starts_with("file:/")));
    assert_eq!(args.last().unwrap(), "pipe:1");
    e.execute("v.CurrentTime=0;").unwrap();
    gone(pid);
    e.execute("a=readFrame(v); alias=v; clear v;").unwrap();
    let pid = std::fs::read_to_string(&pidfile).unwrap().parse().unwrap();
    assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
    e.execute("clear alias;").unwrap();
    gone(pid);
    e.execute(&format!(
        "v=VideoReader('{}'); a=readFrame(v);",
        fixture("ffmpeg-lossless.mp4")
    ))
    .unwrap();
    let pid = std::fs::read_to_string(&pidfile).unwrap().parse().unwrap();
    drop(e);
    gone(pid);
}
#[test]
#[cfg(target_os = "linux")]
fn video_ffmpeg_read_timeout_reaps_hung_child() {
    if !available() {
        return;
    }
    let directory = dir();
    let pidfile = directory.path().join("pid");
    let tool = fake(
        directory.path(),
        "timeout",
        &format!(
            "open({:?},'w').write(str(os.getpid()))\nwhile True: time.sleep(0.02)",
            pidfile.to_str().unwrap()
        ),
    );
    let mut e = Engine::default();
    override_tool(&mut e, "MATRUST_FFMPEG", &tool);
    e.execute(&format!(
        "v=VideoReader('{}');",
        fixture("ffmpeg-lossless.mp4")
    ))
    .unwrap();
    let start = std::time::Instant::now();
    let error = e.execute("a=readFrame(v);").unwrap_err();
    assert!(error.contains("timed out after 10 seconds"), "{error}");
    assert!(start.elapsed().as_secs() < 13);
    gone(std::fs::read_to_string(pidfile).unwrap().parse().unwrap());
}

#[test]
#[cfg(unix)]
fn video_ffmpeg_encoder_fallbacks_and_exit_error() {
    if !available() {
        return;
    }
    let directory = dir();
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("ffmpeg"))
        .find(|p| p.is_file())
        .unwrap();
    let fallback = fake(
        directory.path(),
        "fallback",
        &format!(
            "if '-encoders' in sys.argv:\n print(' V..... mpeg4 MPEG4\\n V..... jpeg2000 JPEG2000')\nelse:\n os.execv({0:?}, [{0:?}]+sys.argv[1:])",
            real.to_str().unwrap()
        ),
    );
    for (profile, ext) in [("MPEG-4", "mp4"), ("Archival", "mj2")] {
        let mut e = Engine::default();
        override_tool(&mut e, "MATRUST_FFMPEG", &fallback);
        let path = directory.path().join(format!("fallback.{ext}"));
        e.execute(&format!(
            "r=VideoReader('{}'); a=readFrame(r); w=VideoWriter('{}','{profile}');",
            fixture("ffmpeg-lossless.mp4"),
            quote(&path)
        ))
        .unwrap();
        if profile == "MPEG-4" {
            e.execute("w.Quality=100;").unwrap();
        }
        e.execute("writeVideo(w,a); close(w);").unwrap();
        no_temp(directory.path());
        if profile == "Archival" {
            assert_eq!(raw(&path, false), interleaved(&e, "a"));
        }
        let result = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "csv=p=0",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            if profile == "MPEG-4" {
                "mpeg4"
            } else {
                "jpeg2000"
            }
        );
    }
    let none = fake(directory.path(), "no-jpeg", "print(' V..... mpeg4 MPEG4')");
    let mut e = Engine::default();
    override_tool(&mut e, "MATRUST_FFMPEG", &none);
    let path = directory.path().join("none.mj2");
    e.execute(&format!("w=VideoWriter('{}','Archival');", quote(&path)))
        .unwrap();
    assert!(
        e.execute("open(w);")
            .unwrap_err()
            .contains("no JPEG 2000 encoder")
    );
    no_temp(directory.path());
    let bad = fake(
        directory.path(),
        "exit-error",
        "if '-encoders' in sys.argv:\n print(' V..... libx264 H264')\nelse:\n sys.stdin.buffer.read(); sys.stderr.write('FINAL-ENCODER-ERROR'); sys.exit(9)",
    );
    override_tool(&mut e, "MATRUST_FFMPEG", &bad);
    let path = directory.path().join("existing.mp4");
    std::fs::write(&path, b"old").unwrap();
    e.execute(&format!(
        "w=VideoWriter('{}','MPEG-4'); writeVideo(w,uint8(zeros(2,2,3)));",
        quote(&path)
    ))
    .unwrap();
    assert!(
        e.execute("close(w);")
            .unwrap_err()
            .contains("FINAL-ENCODER-ERROR")
    );
    no_temp(directory.path());
    assert_eq!(std::fs::read(path).unwrap(), b"old");
}
#[test]
fn video_ffmpeg_release_measurement() {
    if !available() {
        return;
    }
    let directory = dir();
    let path = directory.path().join("bench.mj2");
    let mut limits = matrust::engine::Limits::browser();
    limits.evaluation_units = None;
    let mut e = Engine::with_limits(limits);
    e.execute(&format!(
        "r=VideoReader('{}'); a=readFrame(r); w=VideoWriter('{}','Archival');",
        fixture("ffmpeg-lossless.mp4"),
        quote(&path)
    ))
    .unwrap();
    let start = std::time::Instant::now();
    e.execute("for k=1:120; writeVideo(w,a); end; close(w);")
        .unwrap();
    let write = start.elapsed();
    let start = std::time::Instant::now();
    e.execute(&format!(
        "v=VideoReader('{}'); for k=1:120; b=readFrame(v); end;",
        quote(&path)
    ))
    .unwrap();
    eprintln!(
        "ffmpeg 120 64x48 frames workers={} write={:.3} ms read={:.3} ms",
        std::env::var("RAYON_NUM_THREADS").unwrap_or_default(),
        write.as_secs_f64() * 1000.,
        start.elapsed().as_secs_f64() * 1000.
    );
    no_temp(directory.path());
}

#[test]
fn video_ffmpeg_odd_dimensions_empty_and_invalid_properties() {
    if !available() {
        return;
    }
    let directory = dir();
    let path = directory.path().join("odd.mp4");
    let mut e = Engine::default();
    e.execute(&format!("w=VideoWriter('{}','MPEG-4'); writeVideo(w,uint8(ones(3,5,3,2)*128)); width=w.Width; height=w.Height; close(w); r=VideoReader('{}'); rw=r.Width; rh=r.Height; n=r.NumFrames;",quote(&path),quote(&path))).unwrap();
    for (name, value) in [
        ("width", 6.),
        ("height", 4.),
        ("rw", 6.),
        ("rh", 4.),
        ("n", 2.),
    ] {
        assert_eq!(num(&e, name), value);
    }
    let mut limits = matrust::engine::Limits::browser();
    limits.memory_bytes = 256 * 1024;
    let mut bounded = Engine::with_limits(limits);
    bounded
        .execute(&format!("w=VideoWriter('{}','MPEG-4');", quote(&path)))
        .unwrap();
    assert!(
        bounded
            .execute("open(w);")
            .unwrap_err()
            .contains("memory limit")
    );
    no_temp(directory.path());
    let path = directory.path().join("empty.mj2");
    std::fs::write(&path, b"old").unwrap();
    e.execute(&format!(
        "w=VideoWriter('{}','Archival'); open(w); close(w);",
        quote(&path)
    ))
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    no_temp(directory.path());
    e.execute(&format!("w=VideoWriter('{}','Archival');", quote(&path)))
        .unwrap();
    assert!(
        e.execute("w.CompressionRatio=3;")
            .unwrap_err()
            .contains("LosslessCompression")
    );
    e.execute("w.LosslessCompression=false; w.CompressionRatio=3; w.MJ2BitDepth=8; open(w);")
        .unwrap();
    assert!(
        e.execute("w.CompressionRatio=4;")
            .unwrap_err()
            .contains("after calling open")
    );
    no_temp(directory.path());
}

#[test]
fn video_ffmpeg_archival_gray_and_sixteen_bit_exact() {
    if !available() {
        return;
    }
    let directory = dir();
    for channels in [1, 3] {
        let path = directory.path().join(format!("sixteen-{channels}.mj2"));
        let mut e = Engine::default();
        let (h, w, frames) = (4, 6, 2);
        let values = (0..h * w * channels * frames).map(|i| ((i * 389 + 12345) % 65536) as i128);
        let a = matrust::TypedArray::from_ints(
            matrust::NumericClass::Uint16,
            h,
            w * channels * frames,
            values,
        )
        .unwrap()
        .with_dims(&[h, w, channels, frames])
        .unwrap();
        let mut expected = Vec::new();
        for f in 0..frames {
            for y in 0..h {
                for x in 0..w {
                    for c in 0..channels {
                        expected.extend_from_slice(
                            &(a.int(f * w * h * channels + y + x * h + c * w * h) as u16)
                                .to_le_bytes(),
                        );
                    }
                }
            }
        }
        e.insert_array("a", Array::Typed(a)).unwrap();
        e.execute(&format!("w=VideoWriter('{}','Archival'); writeVideo(w,a); depth=w.MJ2BitDepth; bits=w.VideoBitsPerPixel; c=w.ColorChannels; close(w);",quote(&path))).unwrap();
        assert_eq!(num(&e, "depth"), 16.);
        assert_eq!(num(&e, "bits"), (channels * 16) as f64);
        assert_eq!(num(&e, "c"), channels as f64);
        let result = Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&path)
            .args([
                "-f",
                "rawvideo",
                "-pix_fmt",
                if channels == 1 { "gray16le" } else { "rgb48le" },
                "-threads",
                "1",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected);
        no_temp(directory.path());
    }
}

#[test]
#[cfg(target_os = "linux")]
fn video_ffmpeg_writer_collection_and_engine_drop_reap() {
    let directory = dir();
    let pidfile = directory.path().join("pid");
    let tool = fake(
        directory.path(),
        "collector",
        &format!(
            "if '-encoders' in sys.argv:\n print(' V..... libx264 H264')\nelse:\n open({:?},'w').write(str(os.getpid()))\n sys.stdin.buffer.read(12)\n while True: time.sleep(0.02)",
            pidfile.to_str().unwrap()
        ),
    );
    for clear in [true, false] {
        let mut e = Engine::default();
        override_tool(&mut e, "MATRUST_FFMPEG", &tool);
        let path = directory.path().join("unpublished.mp4");
        e.execute(&format!(
            "w=VideoWriter('{}','MPEG-4'); writeVideo(w,uint8(zeros(2,2,3)));",
            quote(&path)
        ))
        .unwrap();
        let deadline = std::time::Instant::now();
        let pid = loop {
            if let Some(pid) = std::fs::read_to_string(&pidfile)
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
            {
                break pid;
            }
            assert!(
                deadline.elapsed().as_secs() < 2,
                "encoder did not publish its PID"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let start = std::time::Instant::now();
        if clear {
            e.execute("clear w;").unwrap();
        }
        drop(e);
        assert!(start.elapsed().as_secs() < 2);
        gone(pid);
        no_temp(directory.path());
        assert!(!path.exists());
        std::fs::remove_file(&pidfile).unwrap();
    }
}

fn vfr_fixture(directory: &std::path::Path, offset: bool) -> std::path::PathBuf {
    // Exact codex finding-3 generator; capture pipe:1 into a temporary Matroska.
    let result = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-i",
        ])
        .arg(fixture("ffmpeg-lossless.mp4"))
        .args([
            "-vf",
            if offset {
                "setpts='if(lt(N,5),N+20,N+30)/(10*TB)'"
            } else {
                "setpts='if(lt(N,5),N,N+10)/(10*TB)'"
            },
            "-c:v",
            "ffv1",
            "-threads",
            "1",
            "-fps_mode",
            "passthrough",
            "-f",
            "matroska",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let path = directory.join(if offset { "offset.mkv" } else { "vfr.mkv" });
    std::fs::write(&path, result.stdout).unwrap();
    path
}
fn assert_ordinal_reads(e: &mut Engine, path: &std::path::Path) {
    e.execute(&format!("v=VideoReader('{}'); all=read(v); seventh=read(v,7); last=read(v,Inf); range=read(v,[6 8]); first=read(v,1);", quote(path))).unwrap();
    let all = interleaved(e, "all");
    let size = 64 * 48 * 3;
    assert_eq!(all.len(), size * 10);
    assert_eq!(interleaved(e, "seventh"), all[6 * size..7 * size]);
    assert_eq!(interleaved(e, "last"), all[9 * size..]);
    assert_eq!(interleaved(e, "range"), all[5 * size..8 * size]);
    assert_eq!(interleaved(e, "first"), all[..size]);
    // Viewer fetches ordinal frames through the same read(v,k) entry point.
    for i in [10, 2, 8, 4, 6, 9, 3, 5, 1, 7] {
        e.execute(&format!("selected=read(v,{i});")).unwrap();
        assert_eq!(
            interleaved(e, "selected"),
            all[(i - 1) * size..i * size],
            "frame {i}"
        );
    }
}
#[test]
fn video_ffmpeg_vfr_ordinal_reads() {
    if !available() {
        return;
    }
    let directory = dir();
    for offset in [false, true] {
        let path = vfr_fixture(directory.path(), offset);
        assert_ordinal_reads(&mut Engine::default(), &path);
    }
}

#[cfg(unix)]
fn real_tool(name: &str) -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|p| p.is_absolute())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
        .unwrap()
}
#[test]
#[cfg(unix)]
fn video_ffmpeg_timestamp_fallback_and_cache() {
    if !available() {
        return;
    }
    let directory = dir();
    let path = vfr_fixture(directory.path(), false);
    let real = real_tool("ffprobe");
    for (case, response) in [
        (
            "valid",
            format!("os.execv({0:?}, [{0:?}]+sys.argv[1:])", real),
        ),
        ("missing", "print(json.dumps({'frames':[{}]*10}))".into()),
        (
            "mismatch",
            "print(json.dumps({'frames':[{'pts_time':'0.0'}]}))".into(),
        ),
        (
            "duplicate",
            "print(json.dumps({'frames':[{'pts_time':'0.0'}]*10}))".into(),
        ),
        (
            "nonmonotonic",
            "print(json.dumps({'frames':[{'pts_time':str(9-i)} for i in range(10)]}))".into(),
        ),
        ("cap", "sys.stdout.write(' '* (1024*1024+1))".into()),
    ] {
        let count = directory.path().join(format!("{case}.count"));
        let tool = fake(
            directory.path(),
            case,
            &format!(
                "if '-show_frames' in sys.argv:\n open({count:?},'a').write('probe\\n')\n {response}\nelse:\n os.execv({real:?}, [{real:?}]+sys.argv[1:])"
            ),
        );
        let mut e = Engine::default();
        override_tool(&mut e, "MATRUST_FFPROBE", &tool);
        assert_ordinal_reads(&mut e, &path);
        assert_eq!(
            std::fs::read_to_string(count).unwrap(),
            "probe\n",
            "{case}: index built once per reader"
        );
    }
}

#[test]
fn video_ffmpeg_relative_overrides_rejected() {
    for (key, value) in [
        ("MATRUST_FFMPEG", "ffmpeg"),
        ("MATRUST_FFMPEG", "./ffmpeg"),
        ("MATRUST_FFPROBE", "ffprobe"),
        ("MATRUST_FFPROBE", "tools/ffprobe"),
    ] {
        let mut e = Engine::default();
        e.execute("setenv('MATRUST_FFMPEG'); setenv('MATRUST_FFPROBE');")
            .unwrap();
        e.execute(&format!("setenv('{key}','{value}');")).unwrap();
        for command in [
            format!("VideoReader('{}');", fixture("ffmpeg-lossless.mp4")),
            "VideoWriter('missing.mp4','MPEG-4');".into(),
        ] {
            let error = e.execute(&command).unwrap_err();
            assert!(
                error.contains(&format!("{key} must be an absolute path")),
                "{error}"
            );
        }
    }
}

#[test]
#[cfg(unix)]
fn video_ffmpeg_path_never_searches_engine_folder() {
    let directory = dir();
    let empty = directory.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let marker = directory.path().join("PLANTED-RAN");
    for name in ["ffmpeg", "ffprobe"] {
        fake(
            directory.path(),
            name,
            &format!("open({marker:?},'a').write(sys.argv[0]); print('{{\"streams\":[]}}')"),
        );
    }
    std::fs::write(directory.path().join("x.mp4"), b"").unwrap();
    let fresh = || {
        let mut e = Engine::default();
        e.set_current_folder(directory.path()).unwrap();
        e.execute("setenv('MATRUST_FFMPEG'); setenv('MATRUST_FFPROBE');")
            .unwrap();
        e
    };
    // Every unsafe PATH row from grok #1, plus explicit relative entries.
    for setup in [
        "setenv('PATH');".into(),
        "setenv('PATH','');".into(),
        format!("setenv('PATH','{}:');", quote(&empty)),
        "setenv('PATH','.');".into(),
        "setenv('PATH','empty:..');".into(),
        format!("setenv('PATH','{}');", quote(&empty)),
    ] {
        let mut e = fresh();
        e.execute(&setup).unwrap();
        let error = e.execute("VideoReader('x.mp4');").unwrap_err();
        assert!(
            error.contains("needs ffmpeg on PATH (not found)"),
            "{setup}: {error}"
        );
        assert!(!marker.exists(), "{setup}: planted tool executed");
    }
    for key in ["MATRUST_FFMPEG", "MATRUST_FFPROBE"] {
        let mut e = fresh();
        e.execute(&format!(
            "setenv('PATH','/usr/bin:/bin'); setenv('{key}','{}');",
            if key == "MATRUST_FFMPEG" {
                "ffmpeg"
            } else {
                "ffprobe"
            }
        ))
        .unwrap();
        let error = e.execute("VideoReader('x.mp4');").unwrap_err();
        assert!(
            error.contains(&format!("{key} must be an absolute path")),
            "{error}"
        );
        assert!(!marker.exists());
    }
    if !available() {
        return;
    }
    let search = format!(
        "{}:{}",
        quote(real_tool("ffmpeg").parent().unwrap()),
        quote(real_tool("ffprobe").parent().unwrap())
    );
    let mut e = fresh();
    // A cached miss must be invalidated when PATH becomes usable.
    e.execute("setenv('PATH');").unwrap();
    assert!(
        e.execute("VideoReader('x.mp4');")
            .unwrap_err()
            .contains("needs ffmpeg on PATH (not found)")
    );
    e.execute(&format!("setenv('PATH','{search}');")).unwrap();
    let error = e.execute("VideoReader('x.mp4');").unwrap_err();
    assert!(
        !error.contains("needs ffmpeg on PATH (not found)") && !error.contains("no video stream"),
        "{error}"
    );
    e.execute(&format!(
        "v=VideoReader('{}');",
        fixture("ffmpeg-lossless.mp4")
    ))
    .unwrap();
    // A cached success must also be invalidated when PATH is cleared.
    e.execute("setenv('PATH','');").unwrap();
    assert!(
        e.execute("VideoReader('x.mp4');")
            .unwrap_err()
            .contains("needs ffmpeg on PATH (not found)")
    );
    // An absolute directory followed by an empty entry still uses the real tools.
    e.execute(&format!(
        "setenv('PATH','{search}:'); v=VideoReader('{}');",
        fixture("ffmpeg-lossless.mp4")
    ))
    .unwrap();
    assert!(!marker.exists(), "planted tool executed");
}

#[test]
fn video_ffmpeg_fractional_b_frames_ordinal_reads() {
    if !available() {
        return;
    }
    let directory = dir();
    let path = directory.path().join("reordered.mp4");
    let result = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-r",
            "30000/1001",
            "-i",
        ])
        .arg(fixture("ffmpeg-lossless.mp4"))
        .args([
            "-c:v",
            "mpeg4",
            "-bf",
            "2",
            "-threads",
            "1",
            "-fps_mode",
            "passthrough",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_ordinal_reads(&mut Engine::default(), &path);
}
