use matrust::Engine;

#[test]
fn video_write_exact_layout() {
    let dir =
        std::env::temp_dir().join(format!("matrust-video-write-layout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("three.avi");
    let mut e = Engine::default();
    e.execute(&format!("w=VideoWriter('{}','Uncompressed AVI'); open(w); writeVideo(w,uint8(zeros(24,32,3,3))); close(w);", path.display())).unwrap();
    // RIFF(12) + hdrl(200) + movi(12) + 3*(chunk(8)+32*24*3)
    // + idx1(8+3*16) = 7216 bytes. No optional chunks.
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 7216);
    std::fs::remove_dir_all(dir).unwrap();
}

use matrust::{Array, Matrix, NumericClass, TypedArray};
use std::{path::Path, process::Command};

fn num(e: &Engine, name: &str) -> f64 {
    match e.array(name).unwrap() {
        Array::Numeric(m) => m.data()[0],
        Array::Logical(m) => f64::from(m.data()[0]),
        other => panic!("missing scalar {name}: {other:?}"),
    }
}
fn bytes(e: &Engine, name: &str) -> Vec<u8> {
    let Some(Array::Typed(t)) = e.array(name) else {
        panic!("missing {name}")
    };
    (0..t.len()).map(|i| t.int(i) as u8).collect()
}
fn insert_rgb(e: &mut Engine, name: &str, rgb: &[u8], w: usize, h: usize) {
    let mut col = vec![0; rgb.len()];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                col[y + x * h + c * w * h] = rgb[(y * w + x) * 3 + c];
            }
        }
    }
    e.insert_array(
        name,
        Array::Typed(
            TypedArray::from_ints(
                NumericClass::Uint8,
                h,
                w * 3,
                col.into_iter().map(i128::from),
            )
            .unwrap()
            .with_dims(&[h, w, 3])
            .unwrap(),
        ),
    )
    .unwrap();
}
fn interleaved(e: &Engine, name: &str, w: usize, h: usize, channels: usize) -> Vec<u8> {
    let col = bytes(e, name);
    let mut rgb = vec![0; col.len()];
    for f in 0..col.len() / (w * h * channels) {
        for y in 0..h {
            for x in 0..w {
                for c in 0..channels {
                    rgb[f * w * h * channels + (y * w + x) * channels + c] =
                        col[f * w * h * channels + y + x * h + c * w * h];
                }
            }
        }
    }
    rgb
}
fn no_temp(dir: &Path) {
    for file in std::fs::read_dir(dir).unwrap() {
        assert!(
            !file
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".matrust-video-")
        );
    }
}
fn oracle(
    path: &Path,
    codec: &str,
    shape: (usize, usize, usize),
    native: &[u8],
    channels: usize,
    tolerance: u8,
) {
    let (w, h, frames) = shape;
    let probe = match Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,r_frame_rate,nb_frames",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIP ffprobe oracle: ffprobe is not on PATH");
            return;
        }
        Err(e) => panic!("ffprobe: {e}"),
    };
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    let s = &value["streams"][0];
    assert_eq!(s["codec_name"], codec);
    assert_eq!(s["width"], w);
    assert_eq!(s["height"], h);
    assert_eq!(s["r_frame_rate"], "7/1");
    assert_eq!(s["nb_frames"], frames.to_string());
    eprintln!(
        "ffprobe {}: {}",
        path.file_name().unwrap().to_string_lossy(),
        s
    );
    let decoded = match Command::new("ffmpeg")
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(path)
        .args([
            "-sws_flags",
            "neighbor+accurate_rnd+bitexact",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIP decode oracle: ffmpeg is not on PATH");
            return;
        }
        Err(e) => panic!("ffmpeg: {e}"),
    };
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    // Compare palette video as RGB. Asking swscale for gray introduces a
    // separate RGB-to-luma rounding step (observed error 1 for identity gray).
    let expanded;
    let native = if channels == 1 {
        expanded = native.iter().flat_map(|&v| [v; 3]).collect::<Vec<_>>();
        expanded.as_slice()
    } else {
        native
    };
    assert_eq!(decoded.stdout.len(), native.len());
    let max = decoded
        .stdout
        .iter()
        .zip(native)
        .map(|(&a, &b)| a.abs_diff(b))
        .max()
        .unwrap();
    eprintln!("ffmpeg decode max diff {max}");
    assert!(max <= tolerance, "max diff {max}");
}

#[test]
fn video_write_rgb_roundtrip_and_ffprobe() {
    for (w, h) in [(32, 24), (33, 25), (1, 1)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw.avi");
        let mut e = Engine::default();
        e.execute(&format!("w=VideoWriter('{}','Uncompressed AVI'); w.FrameRate=7; a=uint8(reshape(mod(0:{},256),{h},{w},3,3)); w.open(); w.writeVideo(a); w.close(); r=VideoReader('{}'); b=read(r); same=isequal(a,b); n=w.FrameCount; d=w.Duration; width=w.Width; height=w.Height;",path.display(),w*h*9-1,path.display())).unwrap();
        assert_eq!(num(&e, "same"), 1.);
        assert_eq!(num(&e, "n"), 3.);
        assert_eq!(num(&e, "d"), 3. / 7.);
        assert_eq!(num(&e, "width"), w as f64);
        assert_eq!(num(&e, "height"), h as f64);
        oracle(
            &path,
            "rawvideo",
            (w, h, 3),
            &interleaved(&e, "b", w, h, 3),
            3,
            0,
        );
        no_temp(dir.path());
    }
}

#[test]
fn video_write_gray_indexed_roundtrip() {
    for profile in ["Grayscale AVI", "Indexed AVI"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("palette.avi");
        let mut e = Engine::default();
        let map = if profile == "Indexed AVI" {
            "w.Colormap=[(0:255)'/255 zeros(256,1) (255:-1:0)'/255];"
        } else {
            ""
        };
        e.execute(&format!("w=VideoWriter('{}','{profile}'); w.FrameRate=7; {map} a=uint8(reshape(mod(0:494,256),9,11,1,5)); open(w); writeVideo(w,a); close(w); r=VideoReader('{}'); movie=read(r,'native'); b=movie(1).cdata; same=isequal(a(:,:,1,1),b); full=read(r);",path.display(),path.display())).unwrap();
        assert_eq!(num(&e, "same"), 1.);
        for i in 1..=5 {
            e.execute(&format!("same=isequal(a(:,:,1,{i}),movie({i}).cdata);"))
                .unwrap();
            assert_eq!(num(&e, "same"), 1.);
        }
        let c = if profile == "Grayscale AVI" { 1 } else { 3 };
        oracle(
            &path,
            "rawvideo",
            (11, 9, 5),
            &interleaved(&e, "full", 11, 9, c),
            c,
            0,
        );
        no_temp(dir.path());
    }
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mse = a
        .iter()
        .zip(b)
        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum::<f64>()
        / a.len() as f64;
    10. * (255. * 255. / mse).log10()
}

#[test]
fn video_write_mjpeg_quality_and_ffprobe() {
    let reference = std::fs::read(format!(
        "{}/tests/fixtures/video/bgr24.rgb",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let rgb = &reference[..64 * 48 * 3];
    let mut previous = 0.;
    for quality in [75, 95, 100] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mjpeg.avi");
        let mut e = Engine::default();
        insert_rgb(&mut e, "a", rgb, 64, 48);
        e.execute(&format!("w=VideoWriter('{}'); w.FrameRate=7; w.Quality={quality}; writeVideo(w,a); close(w); r=VideoReader('{}'); b=readFrame(r);",path.display(),path.display())).unwrap();
        let actual = interleaved(&e, "b", 64, 48, 3);
        let score = psnr(rgb, &actual);
        eprintln!("Quality {quality}: PSNR {score:.5} dB");
        assert!(score >= 35., "PSNR {score}");
        assert!(score >= previous);
        previous = score;
        let file = std::fs::read(&path).unwrap();
        assert!(file.windows(5).any(|b| b == b"JFIF\0"));
        assert!(file.windows(2).any(|b| b == [255, 196]));
        oracle(&path, "mjpeg", (64, 48, 1), &actual, 3, 2);
        no_temp(dir.path());
    }
}

#[test]
fn video_write_float_scaling_and_index_base() {
    for class in ["double", "single"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scale.avi");
        let mut e = Engine::default();
        e.execute(&format!("w=VideoWriter('{}','Grayscale AVI'); writeVideo(w,{class}([-1 0 0.5 1 2])); close(w); r=VideoReader('{}'); b=readFrame(r);",path.display(),path.display())).unwrap();
        assert_eq!(bytes(&e, "b"), vec![0, 0, 128, 255, 255]);
        let path = dir.path().join("index.avi");
        e.execute(&format!("w=VideoWriter('{}','Indexed AVI'); w.Colormap=[1 0 0;0 1 0;0 0 1]; writeVideo(w,{class}([1 2 3])); close(w); r=VideoReader('{}'); m=readFrame(r,'native'); b=m.cdata;",path.display(),path.display())).unwrap();
        assert_eq!(bytes(&e, "b"), vec![0, 1, 2]);
    }
}

#[test]
fn video_write_movie_struct_array_and_palette() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    for profile in ["Uncompressed AVI", "Grayscale AVI", "Indexed AVI"] {
        let path = dir.path().join(format!("{profile}.avi"));
        let (cdata, map) = if profile == "Uncompressed AVI" {
            ("uint8(ones(7,9,3)*42)", "[]")
        } else if profile == "Grayscale AVI" {
            ("uint8(ones(7,9)*42)", "[]")
        } else {
            ("uint8(zeros(7,9))", "[1 0 0;0 1 0]")
        };
        e.execute(&format!("w=VideoWriter('{}','{profile}'); f=struct(); f.cdata={cdata}; f.colormap={map}; movie=[f f f]; open(w); writeVideo(w,movie); close(w); n=w.FrameCount; r=VideoReader('{}'); m=read(r,2,'native'); b=m;",path.display(),path.display())).unwrap();
        assert_eq!(num(&e, "n"), 3.);
        let actual = if profile == "Uncompressed AVI" {
            "m"
        } else {
            "m.cdata"
        };
        e.execute(&format!("same=isequal(f.cdata,{actual});"))
            .unwrap();
        assert_eq!(num(&e, "same"), 1.);
        no_temp(dir.path());
    }
    let path = dir.path().join("expand.avi");
    e.execute(&format!("w=VideoWriter('{}','Uncompressed AVI'); f=struct('cdata',uint8([0 1]),'colormap',[1 0 0;0 1 0]); writeVideo(w,f); close(w); r=VideoReader('{}'); b=readFrame(r);",path.display(),path.display())).unwrap();
    assert_eq!(bytes(&e, "b"), vec![255, 0, 0, 255, 0, 0]);
}

#[test]
fn video_write_error_finalization_and_size_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("partial.avi");
    let mut e = Engine::default();
    e.execute(&format!(
        "w=VideoWriter('{}','Uncompressed AVI'); open(w); writeVideo(w,uint8(zeros(3,5,3)));",
        path.display()
    ))
    .unwrap();
    assert!(!path.exists());
    let err = e.execute("writeVideo(w,uint8(zeros(4,5,3)));").unwrap_err();
    assert!(err.contains("first frame"), "{err}");
    no_temp(dir.path());
    e.execute(&format!(
        "r=VideoReader('{}'); n=r.NumFrames;",
        path.display()
    ))
    .unwrap();
    assert_eq!(num(&e, "n"), 1.);
    for property in ["FrameRate", "Quality", "Colormap"] {
        let path = dir.path().join(format!("{property}.avi"));
        e.execute(&format!("w=VideoWriter('{}'); open(w);", path.display()))
            .unwrap();
        let err = e.execute(&format!("w.{property}=42;")).unwrap_err();
        assert!(err.contains("after calling open"), "{err}");
        no_temp(dir.path());
    }
    for code in [
        "w.Filename='no';",
        "w.Width=4;",
        "w.FrameRate=0;",
        "w.Quality=101;",
    ] {
        e.execute(&format!("w=VideoWriter('{}');", path.display()))
            .unwrap();
        assert!(e.execute(code).is_err());
    }
}

#[test]
fn video_write_handle_collection_delete_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    for end in ["clear w;", "delete(w);", ""] {
        let path = dir.path().join(format!("drop{}.avi", end.len()));
        {
            let mut e = Engine::default();
            e.execute(&format!(
                "w=VideoWriter('{}','Uncompressed AVI'); writeVideo(w,uint8(zeros(2,3,3))); {end}",
                path.display()
            ))
            .unwrap();
        }
        no_temp(dir.path());
        let mut e = Engine::default();
        e.execute(&format!(
            "r=VideoReader('{}'); n=r.NumFrames;",
            path.display()
        ))
        .unwrap();
        assert_eq!(num(&e, "n"), 1.);
    }
    let path = dir.path().join("shared.avi");
    let mut e = Engine::default();
    e.execute(&format!("w=VideoWriter('{}','Uncompressed AVI'); p=w; writeVideo(p,uint8(zeros(2,3,3))); n=w.FrameCount; close(p); open(w); writeVideo(w,uint8(ones(3,4,3))); close(w); r=VideoReader('{}'); h=r.Height; count=r.NumFrames;",path.display(),path.display())).unwrap();
    assert_eq!(num(&e, "n"), 1.);
    assert_eq!(num(&e, "h"), 3.);
    assert_eq!(num(&e, "count"), 1.);
    no_temp(dir.path());
}

#[test]
fn video_write_profiles_paths_and_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no-extension");
    let mut e = Engine::default();
    let absent = dir
        .path()
        .join("nonexistent")
        .to_string_lossy()
        .replace('\'', "''");
    e.execute(&format!("setenv('MATRUST_FFMPEG','{absent}'); w=VideoWriter(\"{}\"); name=w.Filename; rate=w.FrameRate; quality=w.Quality; channels=w.ColorChannels; bits=w.VideoBitsPerPixel; format=w.FileFormat; compression=w.VideoCompressionMethod; p=VideoWriter.getProfiles(); count=numel(p); writeVideo(w,uint8(zeros(2,3,3))); close(w);",path.display())).unwrap();
    assert_eq!(num(&e, "rate"), 30.);
    assert_eq!(num(&e, "quality"), 75.);
    assert_eq!(num(&e, "channels"), 3.);
    assert_eq!(num(&e, "bits"), 24.);
    assert_eq!(num(&e, "count"), 4.);
    assert!(path.with_extension("avi").exists());
    for profile in ["Motion JPEG 2000", "Archival", "MPEG-4"] {
        let err = e
            .execute(&format!("VideoWriter('{}','{profile}');", path.display()))
            .unwrap_err();
        assert!(err.contains("needs ffmpeg on PATH (not found)"), "{err}");
    }
    assert!(
        e.execute(&format!("VideoWriter('{}','unknown');", path.display()))
            .is_err()
    );
}

#[test]
fn video_write_memory_admission_and_invalid_classes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("limit.avi");
    let mut limits = matrust::engine::Limits::browser();
    limits.memory_bytes = 150_000;
    let mut e = Engine::with_limits(limits);
    e.insert_array("a", Array::Numeric(Matrix::filled(50, 50, 0.).unwrap()))
        .unwrap();
    e.execute(&format!("w=VideoWriter('{}'); open(w);", path.display()))
        .unwrap();
    let err = e.execute("writeVideo(w,a);").unwrap_err();
    assert!(err.contains("memory"), "{err}");
    no_temp(dir.path());
    assert!(!path.exists());
    let mut e = Engine::default();
    for value in [
        "uint16(zeros(2,3))",
        "true(2,3)",
        "[NaN 1]",
        "zeros(2,3,2)",
        "zeros(0,2)",
        "'bad'",
        "\"bad\"",
    ] {
        e.execute(&format!("w=VideoWriter('{}'); open(w);", path.display()))
            .unwrap();
        assert!(
            e.execute(&format!("writeVideo(w,{value});")).is_err(),
            "{value}"
        );
        no_temp(dir.path());
    }
}

#[test]
fn video_write_cancellation_finalizes_completed_frames() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cancel.avi");
    let mut e = Engine::default();
    e.execute(&format!("w=VideoWriter('{}','Uncompressed AVI'); writeVideo(w,uint8(zeros(120,160,3))); a=uint8(zeros(120,160,3,200));",path.display())).unwrap();
    let cancel = e.cancellation();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        cancel.store(true, Ordering::Relaxed);
    });
    let result = e.execute("writeVideo(w,a);");
    worker.join().unwrap();
    assert!(result.unwrap_err().contains("cancelled"));
    e.cancellation().store(false, Ordering::Relaxed);
    no_temp(dir.path());
    e.execute(&format!(
        "r=VideoReader('{}'); n=r.NumFrames;",
        path.display()
    ))
    .unwrap();
    assert!((1. ..201.).contains(&num(&e, "n")));
}

#[test]
fn video_write_release_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.avi");
    let mut limits = matrust::engine::Limits::browser();
    limits.evaluation_units = None;
    let mut e = Engine::with_limits(limits);
    let reference = std::fs::read(format!(
        "{}/tests/fixtures/video/bgr24.rgb",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    insert_rgb(&mut e, "a", &reference[..64 * 48 * 3], 64, 48);
    e.execute(&format!("w=VideoWriter('{}'); open(w);", path.display()))
        .unwrap();
    let start = std::time::Instant::now();
    e.execute("for k=1:1200; writeVideo(w,a); end; close(w);")
        .unwrap();
    eprintln!(
        "writer workers={} 1200 64x48 MJPEG frames {:.3} ms",
        std::env::var("RAYON_NUM_THREADS").unwrap_or_default(),
        start.elapsed().as_secs_f64() * 1000.
    );
    no_temp(dir.path());
}

#[test]
fn video_write_does_not_shadow_figure_close() {
    let mut e = Engine::default();
    e.execute("figure(8); close(8); figure(9); close all;")
        .unwrap();
}

#[test]
fn video_write_collection_preserves_live_alias_and_atomic_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.avi");
    std::fs::write(&path, b"old destination").unwrap();
    let mut e = Engine::default();
    e.execute(&format!("w=VideoWriter('{}','Uncompressed AVI'); alias=w; writeVideo(w,uint8(zeros(2,3,3))); clear w;",path.display())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"old destination");
    e.execute("clear alias;").unwrap();
    no_temp(dir.path());
    e.execute(&format!(
        "r=VideoReader('{}'); n=r.NumFrames;",
        path.display()
    ))
    .unwrap();
    assert_eq!(num(&e, "n"), 1.);
    let path = dir.path().join("unchanged.avi");
    std::fs::write(&path, b"old").unwrap();
    e.execute(&format!(
        "w=VideoWriter('{}'); open(w); close(w);",
        path.display()
    ))
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    no_temp(dir.path());
}

#[test]
fn video_write_rgb_float_scaling_and_mjpeg_odd() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    for class in ["single", "double"] {
        let path = dir.path().join(format!("rgb-{class}.avi"));
        e.execute(&format!("w=VideoWriter('{}','Uncompressed AVI'); a=repmat({class}([-1 0 0.5 1 2]),1,1,3); writeVideo(w,a); close(w); r=VideoReader('{}'); b=readFrame(r);",path.display(),path.display())).unwrap();
        assert_eq!(bytes(&e, "b"), [0, 0, 128, 255, 255].repeat(3));
    }
    let path = dir.path().join("odd-mjpeg.avi");
    e.execute(&format!("w=VideoWriter('{}'); w.FrameRate=7; writeVideo(w,uint8(ones(25,33,3,2)*128)); close(w); r=VideoReader('{}'); b=read(r);",path.display(),path.display())).unwrap();
    assert!(bytes(&e, "b").iter().all(|&v| v.abs_diff(128) <= 1));
    oracle(
        &path,
        "mjpeg",
        (33, 25, 2),
        &interleaved(&e, "b", 33, 25, 3),
        3,
        2,
    );
    no_temp(dir.path());
}

#[test]
fn video_write_palette_error_and_failed_rename_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("palette-error.avi");
    let mut e = Engine::default();
    e.execute(&format!("w=VideoWriter('{}','Indexed AVI'); f=struct('cdata',uint8(zeros(2,3)),'colormap',[1 0 0;0 1 0]); g=f; g.colormap=[0 0 1;1 0 0];",path.display())).unwrap();
    let err = e.execute("writeVideo(w,[f g]);").unwrap_err();
    assert!(err.contains("colormap must match"), "{err}");
    no_temp(dir.path());
    e.execute(&format!(
        "r=VideoReader('{}'); n=r.NumFrames;",
        path.display()
    ))
    .unwrap();
    assert_eq!(num(&e, "n"), 1.);
    let blocked = dir.path().join("directory.avi");
    e.execute(&format!(
        "w=VideoWriter('{}','Uncompressed AVI'); writeVideo(w,uint8(zeros(2,3,3)));",
        blocked.display()
    ))
    .unwrap();
    std::fs::create_dir(&blocked).unwrap();
    let error = e.execute("close(w);").unwrap_err();
    assert!(error.contains("regular file"), "{error}");
    assert!(blocked.is_dir());
    no_temp(dir.path());
}

#[cfg(unix)]
#[test]
fn video_write_rejects_nonregular_destinations() {
    use std::os::unix::{
        fs::{FileTypeExt, symlink},
        net::UnixListener,
    };
    let dir = tempfile::tempdir().unwrap();
    for kind in ["fifo", "socket", "directory", "device"] {
        let path = dir.path().join(format!("{kind}.avi"));
        let _socket = match kind {
            "fifo" => {
                assert!(
                    Command::new("mkfifo")
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
        let error = Engine::default()
            .execute(&format!(
                "w=VideoWriter('{}','Uncompressed AVI'); open(w);",
                path.display()
            ))
            .unwrap_err();
        assert!(error.contains("regular file"), "{kind}: {error}");
        if kind == "fifo" {
            assert!(std::fs::metadata(&path).unwrap().file_type().is_fifo());
        }
        no_temp(dir.path());
    }
    // A path can change after construction, or while frames are being written.
    for phase in ["open", "close"] {
        let path = dir.path().join(format!("changed-{phase}.avi"));
        let mut e = Engine::default();
        e.execute(&format!(
            "w=VideoWriter('{}','Uncompressed AVI');",
            path.display()
        ))
        .unwrap();
        if phase == "close" {
            e.execute("writeVideo(w,uint8(zeros(2,3,3)));").unwrap();
        }
        assert!(
            Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let error = e.execute(&format!("{phase}(w);")).unwrap_err();
        assert!(error.contains("regular file"), "{error}");
        assert!(std::fs::metadata(&path).unwrap().file_type().is_fifo());
        no_temp(dir.path());
    }
}
