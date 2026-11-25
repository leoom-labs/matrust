use matrust::{Engine, help};

#[test]
fn audio_help_is_available() {
    for name in [
        "sound",
        "soundsc",
        "audioplayer",
        "audiorecorder",
        "play",
        "playblocking",
        "pause",
        "resume",
        "stop",
        "isplaying",
        "get",
        "set",
        "record",
        "recordblocking",
        "getaudiodata",
        "getplayer",
    ] {
        assert!(help::page(name).unwrap().contains("Syntax:"), "{name}");
        assert!(help::is_supported(name), "{name}");
    }
}

#[cfg(not(all(feature = "audio", not(target_arch = "wasm32"))))]
#[test]
fn audio_disabled_warns_and_returns() {
    let mut engine = Engine::default();
    let output = engine
        .execute("sound(0); soundsc(0); p = audioplayer(0,8000); r = audiorecorder;")
        .unwrap();
    assert_eq!(
        output
            .output
            .join("\n")
            .matches("Audio output is not available in this build.")
            .count(),
        4
    );
    assert!(engine.array("p").unwrap().is_empty());
    assert!(engine.array("r").unwrap().is_empty());
}

#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
mod audio_enabled {
    use super::Engine;
    use matrust::engine::audio_output_sample;
    use std::time::{Duration, Instant};

    const NO_OUTPUT: &str = "No audio output device found; sound is not played.";

    fn null_engine() -> Engine {
        let mut engine = Engine::default();
        // Engine-local environment prevents races with other tests in this suite.
        engine
            .execute("setenv('MATRUST_AUDIO_DEVICE','null');")
            .unwrap();
        engine
    }

    fn assert_script(engine: &mut Engine, code: &str) {
        engine
            .execute(code)
            .unwrap_or_else(|error| panic!("{error}\n{code}"));
    }

    #[test]
    fn asynchronous_sound_warns_once_per_session() {
        let mut engine = null_engine();
        assert_script(&mut engine, "y=ones(1,8192);");
        let start = Instant::now();
        let output = engine.execute("sound(y);").unwrap();
        assert!(start.elapsed() < Duration::from_millis(400));
        assert!(output.output.join("\n").contains(NO_OUTPUT));
        let output = engine
            .execute("p=audioplayer(y,8192,8,-1); play(p); soundsc(y,8192,[-1 1]);")
            .unwrap();
        assert!(!output.output.join("\n").contains(NO_OUTPUT));
        assert!(
            null_engine()
                .execute("sound(0);")
                .unwrap()
                .output
                .join("\n")
                .contains(NO_OUTPUT)
        );
    }

    #[test]
    fn soundsc_scales_and_sound_clips() {
        let mut engine = null_engine();
        let samples = |engine: &Engine, count, channel| {
            (0..count)
                .map(|frame| audio_output_sample(engine, frame, channel).unwrap())
                .collect::<Vec<_>>()
        };
        let reference: std::collections::BTreeMap<String, Vec<f32>> =
            serde_json::from_str(include_str!("fixtures/audio/scale.json")).unwrap();
        assert_script(&mut engine, "soundsc([2 10; 6 4],8000);");
        let stereo: Vec<_> = [samples(&engine, 2, 0), samples(&engine, 2, 1)].concat();
        assert_eq!(stereo, reference["stereo"]);
        assert_script(&mut engine, "soundsc([-5; 1; 2; 3; 9],[1 3]);");
        assert_eq!(samples(&engine, 5, 0), reference["clip_range"]);
        assert_script(&mut engine, "soundsc([3 3 3],8000,24);");
        assert_eq!(samples(&engine, 3, 0), reference["constant"]);
        assert_script(&mut engine, "soundsc([-realmax, 0, realmax]);");
        assert_eq!(samples(&engine, 3, 0), [-1., 0., 1.]);
        assert_script(&mut engine, "sound([-2 NaN 0.25 3],8000,8);");
        assert_eq!(samples(&engine, 4, 0), [-1., 0., 0.25, 1.]);
        assert_eq!(audio_output_sample(&engine, 0, 1), None);
    }

    #[test]
    #[ignore = "optimized soundsc benchmark"]
    fn soundsc_benchmark() {
        let workers = std::env::var("MATRUST_AUDIO_BENCH_WORKERS")
            .unwrap()
            .parse()
            .unwrap();
        matrust::initialize_worker_pool(workers).unwrap();
        let mut engine = null_engine();
        for samples in [8192, 8 * 1024 * 1024] {
            let frames = samples / 2;
            let values = (0..samples).map(|i| i as f64 / samples as f64).collect();
            let clip = matrust::Matrix::from_column_vec(frames, 2, values).unwrap();
            engine.variables.insert("y".into(), clip);
            assert_script(&mut engine, "soundsc(y);");
            let mut scan = [Duration::ZERO; 5];
            let mut convert = [Duration::ZERO; 5];
            for i in 0..scan.len() {
                let start = Instant::now();
                assert_script(&mut engine, "soundsc(y);");
                scan[i] = start.elapsed();
                let start = Instant::now();
                for channel in 0..2 {
                    for frame in 0..frames {
                        std::hint::black_box(audio_output_sample(&engine, frame, channel).unwrap());
                    }
                }
                convert[i] = start.elapsed();
            }
            scan.sort_unstable();
            convert.sort_unstable();
            println!(
                "{workers} workers, {samples} samples: scan {:?}, conversion {:?}",
                scan[2], convert[2]
            );
        }
    }

    #[test]
    fn playblocking_observes_duration_and_clears_running() {
        let mut engine = null_engine();
        assert_script(&mut engine, "p=audioplayer(zeros(3200,2),8000);");
        let start = Instant::now();
        assert_script(&mut engine, "playblocking(p,[801 2400]);");
        let elapsed = start.elapsed();
        assert!(
            (Duration::from_millis(100)..=Duration::from_millis(300)).contains(&elapsed),
            "elapsed {elapsed:?}"
        );
        assert_script(
            &mut engine,
            "assert(~isplaying(p)); assert(strcmp(p.Running,'off')); assert(p.CurrentSample==1);",
        );
    }

    #[test]
    fn player_methods_share_state_and_properties() {
        let mut engine = null_engine();
        assert_script(
            &mut engine,
            "p=audioplayer(zeros(16000,2),8000,24); q=p; play(p,801); assert(isplaying(q));",
        );
        std::thread::sleep(Duration::from_millis(35));
        assert_script(
            &mut engine,
            "assert(q.CurrentSample>801); c={q}; pause(c{1}); n=get(q,'CurrentSample'); assert(~isplaying(p));",
        );
        std::thread::sleep(Duration::from_millis(20));
        assert_script(
            &mut engine,
            "assert(p.CurrentSample==n); q.resume(); assert(isplaying(p)); stop(q); assert(~isplaying(p)); assert(p.CurrentSample==1);",
        );
        assert_script(
            &mut engine,
            "set(q,'SampleRate',16000,'Tag','clip','UserData',[3 4]); q.Tag='alias'; a=get(p); assert(a.SampleRate==16000); assert(a.NumChannels==2); assert(a.BitsPerSample==24); assert(a.TotalSamples==16000); assert(a.DeviceID==-1); assert(strcmp(a.Tag,'alias')); assert(isequal(p.UserData,[3 4]));",
        );
        let output = engine.execute("disp(p);").unwrap().output.join("\n");
        assert!(output.contains("audioplayer with properties:"));
        assert!(output.contains("CurrentSample: 1"));
        assert!(
            engine
                .execute("q.TotalSamples=9;")
                .unwrap_err()
                .contains("read-only")
        );
        assert!(engine.execute("play(p,[100 99]);").is_err());
        assert_script(&mut engine, "play(p); delete(q); assert(~isvalid(p));");
    }

    #[test]
    fn recorder_reports_no_input_on_record() {
        let mut engine = null_engine();
        assert_script(
            &mut engine,
            "r=audiorecorder; assert(r.SampleRate==8000); assert(r.BitsPerSample==8); s=audiorecorder(16000,24,2); assert(s.NumChannels==2); y=getaudiodata(s,'single'); assert(isa(y,'single')); assert(isempty(y)); p=getplayer(s); assert(p.NumChannels==2); assert(p.TotalSamples==0); stop(s);",
        );
        for command in ["record(r);", "r.record(0.1);", "recordblocking(r,0.1);"] {
            assert!(
                engine
                    .execute(command)
                    .unwrap_err()
                    .contains("No audio input device found.")
            );
        }
    }

    #[test]
    fn long_clip_accepted_and_over_limit_refused() {
        let mut engine = null_engine();
        let mut limits = engine.limits_active();
        limits.memory_bytes = 512 * 1024 * 1024;
        limits.evaluation_units = Some(64 * 1024 * 1024);
        engine.limits_set(limits);
        // ~9.45 min of 44.1 kHz stereo = 50,000,000 samples (right below Matrix::MAX_ELEMENTS = 50,331,648,
        // and >6x the removed 8 Mi sample cap).
        assert_script(
            &mut engine,
            "y = zeros(25000000, 2, 'int16'); sound(y, 44100);",
        );
        limits.memory_bytes = 16 * 1024 * 1024;
        engine.limits_set(limits);
        let err = engine.execute("sound(y, 44100);").unwrap_err();
        assert!(err.contains("memory limit"), "unexpected error: {err}");
        let err = engine.execute("p = audioplayer(y, 44100);").unwrap_err();
        assert!(err.contains("memory limit"), "unexpected error: {err}");
    }

    #[test]
    fn unreachable_players_stop_and_free_slots() {
        let mut engine = null_engine();
        assert_script(
            &mut engine,
            "y = zeros(80000, 1); for k = 1:40; p = audioplayer(y, 8000); play(p); end",
        );
        assert_script(
            &mut engine,
            "for k = 1:40; p = audioplayer(y, 8000); play(p); pause(p); end",
        );
        assert_script(
            &mut engine,
            "p = audioplayer(y, 8000); play(p); clear p; c = cell(1, 16); for k = 1:16; c{k} = audioplayer(y, 8000); play(c{k}); end",
        );
        let err = engine
            .execute("extra = audioplayer(y, 8000); play(extra);")
            .unwrap_err();
        assert!(
            err.contains("Too many simultaneous audio streams."),
            "unexpected error: {err}"
        );
        // Dropping the last handle in c{1} frees its stream slot at the statement boundary.
        assert_script(&mut engine, "c{1} = []; play(extra);");
    }

    #[test]
    fn finished_players_release_slots_while_held() {
        let mut engine = null_engine();
        // 80 samples at 8 kHz is 0.01 s. The players stay in the cell, so each
        // slot has to free when that clip's time elapses.
        assert_script(
            &mut engine,
            "c = cell(1, 40); for k = 1:40; c{k} = audioplayer(zeros(80, 1), 8000); play(c{k}); pause(0.05); end",
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn forced_alsa_load_failure_warns_once_and_continues() {
        let mut engine = Engine::default();
        engine
            .execute("setenv('MATRUST_ALSA_LIB','/nonexistent/libasound.so.2');")
            .unwrap();
        let output = engine
            .execute("y = zeros(1, 800); sound(y, 8000); x = 42;")
            .unwrap();
        let text = output.output.join("\n");
        assert_eq!(text.matches(NO_OUTPUT).count(), 1, "output: {text}");
        assert_eq!(engine.array("x").unwrap().to_string().trim(), "42");
        let second = engine
            .execute("sound(y, 8000); p = audioplayer(y, 8000); play(p);")
            .unwrap();
        assert_eq!(
            second.output.join("\n").matches(NO_OUTPUT).count(),
            0,
            "unexpected repeat warning: {:?}",
            second.output
        );
        let err = engine.execute("r = audiorecorder; record(r);").unwrap_err();
        assert!(
            err.contains("No audio input device found."),
            "unexpected recorder error: {err}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batch_runs_with_forced_alsa_load_failure() {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_matrust"))
            .env("MATRUST_ALSA_LIB", "/nonexistent/libasound.so.2")
            .args(["-batch", "disp(1)"])
            .output()
            .expect("spawn matrust");
        assert!(
            output.status.success(),
            "matrust -batch failed: status={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), "1", "stdout={stdout:?}");
    }
}
