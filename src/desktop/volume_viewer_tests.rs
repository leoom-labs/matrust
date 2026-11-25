use super::*;
use matrust::{Matrix, engine::volume_viewer_app_hooks};
use std::{
    collections::BTreeMap,
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

#[path = "volume_viewer_label_export_tests.rs"]
mod label_export;

#[path = "volume_viewer_frame_tests.rs"]
mod frames;

fn host() -> Host<VolumeViewer> {
    Host::new(VolumeViewer::default())
}

fn frame(
    h: &mut Host<VolumeViewer>,
    ws: &Snapshot,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<Action>) {
    let mut action = None;
    let output = crate::headless::run(
        ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 850.0),
            )),
            events,
            ..Default::default()
        },
        |ui| show(h, ui, ws, false, &mut action),
    );
    (output, action)
}

fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn click(
    h: &mut Host<VolumeViewer>,
    ws: &Snapshot,
    ctx: &egui::Context,
    name: &str,
) -> Option<Action> {
    let pos = h
        .app
        .controls
        .get(name)
        .unwrap_or_else(|| panic!("Missing control {name}"))
        .center();
    frame(
        h,
        ws,
        ctx,
        vec![egui::Event::PointerMoved(pos), pointer(pos, true)],
    );
    let action = frame(h, ws, ctx, vec![pointer(pos, false)]).1;
    action.or_else(|| frame(h, ws, ctx, vec![]).1)
}

fn wait(h: &mut Host<VolumeViewer>, ws: &Snapshot, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        frame(h, ws, ctx, vec![]);
        if !h.running() && h.app.pending.is_none() {
            assert!(h.run_error.is_none(), "{:?}", h.run_error);
            assert!(h.app.import_error.is_none(), "{:?}", h.app.import_error);
            return;
        }
        assert!(Instant::now() < deadline, "Volume Viewer timed out");
        std::thread::sleep(Duration::from_millis(3));
    }
}

fn sample(h: &mut Host<VolumeViewer>, ws: &Snapshot, ctx: &egui::Context) {
    frame(h, ws, ctx, vec![]);
    click(h, ws, ctx, "Use sample data");
    wait(h, ws, ctx);
    assert_eq!(h.app.loaded.as_ref().unwrap().volume.dims, [96; 3]);
}

#[test]
fn volume_viewer_launch_and_close_roundtrip() {
    let no_desktop = Engine::default().execute("volumeViewer").unwrap_err();
    assert!(no_desktop.contains("requires the desktop"), "{no_desktop}");
    let launched = Arc::new(Mutex::new(Vec::new()));
    let sink = launched.clone();
    let _presentation = volume_viewer_app_hooks::present(move |launch| {
        sink.lock().unwrap().push(launch);
        Ok(())
    });
    let mut engine = Engine::default();
    engine.execute("volumeViewer; V = reshape(uint16(1:6), 1, 2, 3); volumeViewer(V, 'VolumeType', 'Volume'); volumeViewer(logical(V)); volumeViewer close").unwrap();
    let list = launched.lock().unwrap();
    assert!(
        matches!(&list[..], [Launch::Empty, Launch::Volume { name: Some(name), array }, Launch::Volume { array: Array::Logical(_), .. }, Launch::Close]
        if name == "V" && array.class() == "uint16" && array.dims().as_ref() == [1, 2, 3])
    );
    for code in [
        "volumeViewer(ones(2,3))",
        "volumeViewer(ones(2,3,4,5))",
        "volumeViewer(V, 'VolumeType', 'Labels')",
        "volumeViewer(complex(V))",
    ] {
        let error = engine.execute(code).unwrap_err();
        assert_eq!(error.lines().count(), 1, "{error}");
    }
    assert_eq!(list.len(), 4);
    drop(list);

    let ctx = egui::Context::default();
    let mut workspace = crate::Workspace::with_context(ctx, true);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for (code, open) in [
        ("volumeViewer(ones(1,1,5));", true),
        ("volumeViewer close", false),
    ] {
        workspace.submit(crate::worker::Job::ScriptInFolder(
            code.to_owned(),
            dir.clone(),
        ));
        let deadline = Instant::now() + Duration::from_secs(15);
        while workspace.busy {
            workspace.receive();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(workspace.toolkits.apps.is_open(AppKind::VolumeViewer), open);
    }
    assert_eq!(
        AppKind::from_tab(matrust::session::ToolTab::VolumeViewer),
        Some(AppKind::VolumeViewer)
    );
}

#[test]
fn volume_viewer_sample_all_styles_and_export() {
    let mut h = host();
    let ctx = egui::Context::default();
    let ws = Snapshot::default();
    sample(&mut h, &ws, &ctx);
    let loaded = h.app.loaded.as_ref().unwrap();
    assert_eq!(loaded.array.class(), "double");
    // Values computed by tool/refs/volume_viewer/phantom.py.
    for ([row, col, page], expected) in [
        ([0, 0, 0], 0.0),
        ([48, 48, 48], 1.02),
        ([48, 80, 48], 2.0),
        ([48, 58, 36], 1.0),
        ([64, 48, 36], 1.04),
        ([17, 44, 36], 1.03),
        ([43, 50, 77], 1.04),
        ([52, 48, 77], 1.0),
    ] {
        let value = loaded.volume.data[row + 96 * (col + 96 * page)];
        assert!(
            (value - expected).abs() < 1e-12,
            "{row}, {col}, {page}: {value}"
        );
    }
    for (style, _) in STYLES {
        h.app.view.style = style;
        wait(&mut h, &ws, &ctx);
        let render = h.app.render.as_ref().unwrap();
        let [width, height] = render.viewport.size;
        let center = render.pixels[(height / 2) * width + width / 2];
        assert_ne!(center[..3], h.app.view.background, "{style:?}");
    }

    h.app.view.style = RenderingStyle::VolumeRendering;
    h.app.view.cmap = "hot";
    h.app.view.scale = [1.25, 0.75, 1.5];
    h.app.view.angles = [18.0, 55.0];
    h.app.view.zoom = 1.28;
    h.app.view.background = [18, 39, 61];
    h.app.export_name = "myVolumeSettings".into();
    wait(&mut h, &ws, &ctx);
    let Some(Action::ExportScript(settings)) =
        click(&mut h, &ws, &ctx, "Export rendering settings")
    else {
        panic!("Export did not request a workspace script");
    };
    let Some(Action::SaveAppScript(AppKind::VolumeViewer, script)) =
        click(&mut h, &ws, &ctx, "Generate script")
    else {
        panic!("Generate script did not request a save");
    };
    assert!(script.contains("function [V, L] = vv_shepp_logan()"));
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut fresh = Engine::with_limits(matrust::engine::Limits::local());
    fresh.set_file_directory(dir.path()).unwrap();
    crate::apps::execute_export(&mut fresh, &settings).unwrap();
    let Some(Array::Struct(exported)) = fresh.array("myVolumeSettings") else {
        panic!("Settings were not exported as a struct");
    };
    for name in [
        "RenderingStyle",
        "Colormap",
        "Alphamap",
        "IsosurfaceValue",
        "ScaleFactors",
        "CameraPosition",
    ] {
        assert!(exported.field_index(name).is_some(), "{name}");
    }
    fresh.execute("assert(strcmp(myVolumeSettings.RenderingStyle,'VolumeRendering')); assert(isequal(myVolumeSettings.Colormap,hot(256))); assert(isequal(myVolumeSettings.Alphamap,linspace(0,1,256))); assert(isequal(myVolumeSettings.ScaleFactors,[1.25 .75 1.5])); assert(isequal(myVolumeSettings.CameraPosition,[18 55]));").unwrap();
    assert_replay(&h, &script, dir.path(), None);
}

fn replay_png(script: &str, dir: &Path, document: Option<&Path>) -> image::RgbaImage {
    let mut fresh = Engine::with_limits(matrust::engine::Limits::local());
    fresh.set_file_directory(dir).unwrap();
    match document {
        Some(path) => fresh.execute_document(script, path),
        None => fresh.execute(script),
    }
    .unwrap();
    image::open(dir.join("volume_viewer.png"))
        .unwrap()
        .into_rgba8()
}

fn assert_replay(h: &Host<VolumeViewer>, script: &str, dir: &Path, document: Option<&Path>) {
    let pixels = replay_png(script, dir, document);
    let render = h.app.render.as_ref().unwrap();
    assert_eq!(
        pixels.dimensions(),
        (
            render.viewport.size[0] as u32,
            render.viewport.size[1] as u32
        )
    );
    for (actual, expected) in pixels.pixels().zip(&render.pixels) {
        assert!(
            actual
                .0
                .iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(*b) <= 2),
            "{actual:?} != {expected:?}"
        );
    }
}

#[test]
fn volume_viewer_workspace_import_mip_and_ortho() {
    let mut h = host();
    let ctx = egui::Context::default();
    let mut engine = Engine::default();
    engine
        .execute(
            "V = reshape(uint16(1:120), 6, 5, 4); flat = zeros(2,2); logicalVolume = logical(V);",
        )
        .unwrap();
    let mut ws = Snapshot::default();
    ws.sync(&engine.workspace().into_iter().collect::<BTreeMap<_, _>>());
    assert_eq!(
        workspace_volumes(&ws)
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["V", "logicalVolume"]
    );
    frame(&mut h, &ws, &ctx, vec![]);
    click(&mut h, &ws, &ctx, "Import");
    click(&mut h, &ws, &ctx, "Import from workspace");
    assert!(h.app.import == Import::Workspace);
    h.app.workspace_name = "V".into();
    frame(&mut h, &ws, &ctx, vec![]);
    click(&mut h, &ws, &ctx, "Load workspace volume");
    wait(&mut h, &ws, &ctx);
    assert_eq!(h.app.loaded.as_ref().unwrap().array.class(), "uint16");

    h.app.view.style = RenderingStyle::MaximumIntensityProjection;
    h.app.view.angles = [0.0, 90.0];
    h.app.view.cmap = "jet";
    h.app.numbers[0] = 2;
    wait(&mut h, &ws, &ctx);
    engine.execute("maximum = double(max(double(V),[],3)); o = orthosliceViewer(V, 'Colormap', 'jet', 'SliceNumbers', [3 3 2], 'CrosshairEnabled', false);").unwrap();
    let maximum = engine.array("maximum").unwrap();
    let maximum = maximum.as_numeric().unwrap();
    let volume = &h.app.loaded.as_ref().unwrap().volume;
    let map = colormap::named("jet", 256).unwrap().unwrap();
    let [lo, hi] = volume.data_limits;
    let render = h.app.render.as_ref().unwrap();
    assert_eq!(render.viewport.size, [5, 6]);
    for row in 0..6 {
        for col in 0..5 {
            let i = (((maximum.get(row, col) - lo) / (hi - lo)) * 255.0).round() as usize;
            let expected = map[i].map(|v| (v * 255.0).round() as u8);
            assert_eq!(render.pixels[row * 5 + col][..3], expected);
        }
    }

    let mut volume = volume.clone();
    volume.colormap = Arc::new(map);
    let slice = slice_pixels(&volume, h.app.slice_request(), 0);
    let figure = engine.figures().next().unwrap().1;
    let mut expected_axes = matrust::plot::Axes::default();
    figure.orthoslice_viewers[0].sync_pane(0, &mut expected_axes);
    let expected = &expected_axes.extras.images[0];
    assert_eq!([expected.cols, expected.rows], slice.size);
    assert_eq!(
        expected.rgb(
            &expected_axes.extras.effective_colormap(),
            expected_axes.extras.color_limits()
        ),
        slice.rgb
    );

    let Some(Action::SaveVolumeScript(code, array, labels)) =
        click(&mut h, &ws, &ctx, "Generate script")
    else {
        panic!("Workspace export needs a data companion");
    };
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let path = dir.path().join("workspace_view.m");
    crate::designer_output::save(
        crate::designer_output::Save {
            script: code,
            path: path.clone(),
            volume: array,
            labels,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let script = std::fs::read_to_string(&path).unwrap();
    assert_replay(&h, &script, dir.path(), Some(&path));
}

#[test]
fn volume_viewer_dicom_series_order_and_voxel_spacing() {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    for (file, k) in [5, 2, 4, 1, 3].into_iter().enumerate() {
        engine.execute(&format!(
            "M = struct('SOPClassUID','1.2.840.10008.5.1.4.1.1.7', 'SOPInstanceUID','2.25.9{k}', \
             'StudyInstanceUID','2.25.100', 'SeriesInstanceUID','2.25.200', 'InstanceNumber',{k}, \
             'PixelSpacing',[0.7; 0.4], 'SliceThickness',2, 'ImagePositionPatient',[0; 0; {}], \
             'ImageOrientationPatient',[1; 0; 0; 0; 1; 0]); \
             dicomwrite(uint16(ones(3,4)*{k}), 'slice_{file}.dcm', M, 'CreateMode', 'copy');",
            3.5 * f64::from(k),
        )).unwrap();
    }
    let mut h = host();
    let ctx = egui::Context::default();
    let ws = Snapshot::default();
    frame(&mut h, &ws, &ctx, vec![]);
    click(&mut h, &ws, &ctx, "Import");
    click(&mut h, &ws, &ctx, "Import DICOM folder");
    assert!(h.app.import == Import::Dicom);
    h.app.folder = dir.path().to_string_lossy().into_owned();
    frame(&mut h, &ws, &ctx, vec![]);
    click(&mut h, &ws, &ctx, "Scan folder");
    wait(&mut h, &ws, &ctx);
    assert_eq!(h.app.series.as_ref().unwrap().rows.len(), 1);
    click(&mut h, &ws, &ctx, "Load DICOM series");
    wait(&mut h, &ws, &ctx);
    let volume = &h.app.loaded.as_ref().unwrap().volume;
    assert_eq!(volume.dims, [3, 4, 5]);
    assert_eq!(h.app.view.scale, [0.4, 0.7, 3.5]);
    for (page, slice) in volume.data.as_chunks::<12>().0.iter().enumerate() {
        assert!(
            slice.iter().all(|&value| value == (page + 1) as f64),
            "{slice:?}"
        );
    }
}

#[test]
fn volume_viewer_render_worker_is_cancellable_without_blocking_frames() {
    let ctx = egui::Context::default();
    let ws = Snapshot::default();
    let mut h = host();
    let array = Array::Numeric(
        Matrix::filled(1, 12, 1.0)
            .unwrap()
            .with_dims(&[1, 1, 12])
            .unwrap(),
    );
    submit_array(&mut h, &ctx, array, "Single voxel column".into());
    wait(&mut h, &ws, &ctx);
    let (start, started) = std::sync::mpsc::channel();
    let observed = Arc::new(AtomicBool::new(false));
    let signal = observed.clone();
    let ui_thread = std::thread::current().id();
    submit(&mut h, &ctx, JobKind::Render, move |e| {
        start.send(std::thread::current().id()).unwrap();
        let cancel = e.cancellation();
        let end = Instant::now() + Duration::from_secs(3);
        while !cancel.load(Ordering::Relaxed) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1));
        }
        signal.store(cancel.load(Ordering::Relaxed), Ordering::Relaxed);
        Err("Retired render".into())
    });
    assert_ne!(
        started.recv_timeout(Duration::from_secs(3)).unwrap(),
        ui_thread
    );
    let start = Instant::now();
    let (output, _) = frame(&mut h, &ws, &ctx, vec![]);
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(h.running());
    assert!(output.shapes.iter().any(|shape| matches!(
        &shape.shape, egui::Shape::Text(text) if text.galley.text() == "Rendering…"
    )));
    h.app.view.angles = [10.0, 50.0];
    wait(&mut h, &ws, &ctx);
    assert!(observed.load(Ordering::Relaxed));
    assert_eq!(h.app.render.as_ref().unwrap().viewport.view, [10.0, 50.0]);
    assert_eq!(h.app.loaded.as_ref().unwrap().volume.dims, [1, 1, 12]);
}

#[test]
fn volume_viewer_reopen_drops_stale_render() {
    let ctx = egui::Context::default();
    let ws = Snapshot::default();
    let mut workspace = crate::Workspace::with_context(ctx.clone(), true);
    let kind = AppKind::VolumeViewer;
    let tool = crate::toolkits::Tool::App(kind);
    let open = |workspace: &mut crate::Workspace, name: &str, volume| {
        *workspace.worker.volume_viewer.lock().unwrap() = Some(Launch::Volume {
            name: Some(name.into()),
            array: Array::Numeric(volume),
        });
        receive_open(workspace);
        assert!(workspace.toolkits.apps.is_open(kind));
    };
    let shows = |output: &egui::FullOutput, label: &str| {
        output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label),
        )
    };

    let a = Matrix::filled(2, 4, 1.0)
        .unwrap()
        .with_dims(&[2, 2, 2])
        .unwrap();
    open(&mut workspace, "Volume A", a);
    let h = &mut workspace.toolkits.apps.volume_viewer;
    wait(h, &ws, &ctx);
    let old_texture = h.app.texture.as_ref().unwrap().id();

    let (start, started) = mpsc::channel();
    let (observe_cancel, cancelled) = mpsc::channel();
    let timeout = Duration::from_secs(10);
    const LATE_ERROR: &str = "Late render error from Volume A";
    submit(h, &ctx, JobKind::Render, move |engine| {
        let cancel = engine.cancellation();
        start.send(std::thread::current().id()).unwrap();
        // Closing retires the job. Dropping the runner also joins it, so
        // cancellation alone must let this worker return.
        while !cancel.load(Ordering::Relaxed) {
            std::thread::yield_now();
        }
        let _ = observe_cancel.send(());
        Err(LATE_ERROR.into())
    });
    assert_ne!(
        started
            .recv_timeout(timeout)
            .expect("render A did not start"),
        std::thread::current().id()
    );
    let (output, _) = frame(h, &ws, &ctx, vec![]);
    assert!(h.running());
    assert!(shows(&output, "Rendering…"));

    workspace.close_tool_tab(tool);
    assert!(!workspace.toolkits.apps.is_open(kind));
    let b = Matrix::from_column_slice(3, 8, &[[0.0; 12], [1.0; 12]].concat())
        .unwrap()
        .with_dims(&[3, 4, 2])
        .unwrap();
    open(&mut workspace, "Volume B", b);
    // Await close's cancellation before any B frame can submit a job and
    // cancel A itself. The first B frame may now collect A's late error.
    cancelled
        .recv_timeout(timeout)
        .expect("close did not cancel render A");

    let h = &mut workspace.toolkits.apps.volume_viewer;
    assert!(h.app.loaded.is_none());
    assert!(h.app.render.is_none());
    assert!(h.app.texture.is_none());
    h.app.view.style = RenderingStyle::MaximumIntensityProjection;
    h.app.view.cmap = "jet";
    let (output, _) = frame(h, &ws, &ctx, vec![]);
    assert!(!shows(&output, LATE_ERROR));
    wait(h, &ws, &ctx);
    h.app.view.angles = [0.0, 90.0];
    wait(h, &ws, &ctx);
    let (output, _) = frame(h, &ws, &ctx, vec![]);

    let loaded = h.app.loaded.as_ref().unwrap();
    assert_eq!(loaded.name, "Volume B");
    assert_eq!(loaded.volume.dims, [3, 4, 2]);
    let render = h.app.render.as_ref().unwrap();
    assert_eq!(render.viewport.size, [4, 3]);
    // Every axial ray hits B's unit-valued back slice, the red end of jet.
    assert_eq!(render.pixels, vec![[128, 0, 0, 255]; 12]);
    assert!(h.app.status_text().is_none());
    assert!(h.run_error.is_none());
    assert!(h.app.import_error.is_none());
    assert!(shows(&output, "3 × 4 × 2  double"));
    assert!(!shows(&output, LATE_ERROR));
    let texture = h.app.texture.as_ref().unwrap().id();
    assert_ne!(texture, old_texture);
    assert!(ctx.tessellate(output.shapes, output.pixels_per_point).iter().any(|p| {
        matches!(&p.primitive, egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture)
    }));
}

#[test]
fn volume_viewer_large_input_camera_drag_and_wheel() {
    let ctx = egui::Context::default();
    let ws = Snapshot::default();
    let mut h = host();
    let dims = [256; 3];
    let array = Array::Numeric(
        Matrix::filled(256, 256 * 256, 1.0)
            .unwrap()
            .with_dims(&dims)
            .unwrap(),
    );
    submit_array(&mut h, &ctx, array, "Large volume".into());
    wait(&mut h, &ws, &ctx);
    assert_eq!(h.app.loaded.as_ref().unwrap().volume.dims, dims);
    let angles = h.app.view.angles;
    let initial = h.app.controls["3-D view"].center();
    let moved = initial + egui::vec2(32.0, 16.0);
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![egui::Event::PointerMoved(initial), pointer(initial, true)],
    );
    frame(&mut h, &ws, &ctx, vec![egui::Event::PointerMoved(moved)]);
    frame(&mut h, &ws, &ctx, vec![pointer(moved, false)]);
    assert!(h.app.view.angles[0] < angles[0] && h.app.view.angles[1] > angles[1]);
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 50.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        }],
    );
    assert!(h.app.view.zoom > 1.0);
    wait(&mut h, &ws, &ctx);
    assert_eq!(
        h.app.render.as_ref().unwrap().viewport.view,
        h.app.view.angles
    );
}

#[test]
fn volume_viewer_alphamap_presets_edit_and_all_zero() {
    let mut h = host();
    let ctx = egui::Context::default();
    let ws = Snapshot::default();
    sample(&mut h, &ws, &ctx);

    // Preset Ramp: Alphamap(1)=0, (end)=1, and 0.3/0.7 knees within 1/255.
    click(&mut h, &ws, &ctx, "Preset Ramp");
    wait(&mut h, &ws, &ctx);
    let ramp = h.app.effective_view().alphamap;
    assert_eq!(ramp[0], 0.0);
    assert_eq!(ramp[255], 1.0);
    let k03 = (0.3_f64 * 255.0).floor() as usize;
    let k07 = (0.7_f64 * 255.0).ceil() as usize;
    assert!(
        ramp[k03].abs() <= 1.0 / 255.0,
        "ramp[{k03}] = {}",
        ramp[k03]
    );
    assert!(
        (ramp[k07] - 1.0).abs() <= 1.0 / 255.0,
        "ramp[{k07}] = {}",
        ramp[k07]
    );

    let before_drag = h.app.render.as_ref().unwrap().pixels.clone();

    // Drag a control point in the alphamap editor (e.g. the 0.3 knee up toward 0.9) and verify render changes.
    let editor_rect = h.app.controls["Alphamap editor"];
    let plot_rect = editor_rect.shrink(6.0);
    let p_03 = egui::pos2(
        plot_rect.left() + 0.3 * plot_rect.width(),
        plot_rect.bottom(),
    );
    let p_dragged = egui::pos2(
        plot_rect.left() + 0.35 * plot_rect.width(),
        plot_rect.bottom() - 0.8 * plot_rect.height(),
    );
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![egui::Event::PointerMoved(p_03), pointer(p_03, true)],
    );
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![egui::Event::PointerMoved(p_dragged)],
    );
    frame(&mut h, &ws, &ctx, vec![pointer(p_dragged, false)]);
    wait(&mut h, &ws, &ctx);
    let after_drag = &h.app.render.as_ref().unwrap().pixels;
    assert_ne!(before_drag, *after_drag);

    // Click empty space adds a point; Delete key / right-click removes an interior point; ends never removable.
    let count_before_add = h.app.alpha_points.len();
    let empty_pos = egui::pos2(
        plot_rect.left() + 0.5 * plot_rect.width(),
        plot_rect.bottom() - 0.5 * plot_rect.height(),
    );
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![
            egui::Event::PointerMoved(empty_pos),
            pointer(empty_pos, true),
        ],
    );
    frame(&mut h, &ws, &ctx, vec![pointer(empty_pos, false)]);
    assert_eq!(h.app.alpha_points.len(), count_before_add + 1);

    // Remove via Delete key (selected point is the newly added one).
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(h.app.alpha_points.len(), count_before_add);

    // Right-click removes an interior point, but never removes endpoints.
    let end_pos = egui::pos2(plot_rect.left(), plot_rect.bottom());
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![
            egui::Event::PointerMoved(end_pos),
            egui::Event::PointerButton {
                pos: end_pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    frame(
        &mut h,
        &ws,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: end_pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert_eq!(h.app.alpha_points.len(), count_before_add);

    // Alphamap all-zero -> background-only render.
    h.app.alpha_points = vec![[0.0, 0.0], [1.0, 0.0]];
    wait(&mut h, &ws, &ctx);
    let bg = h.app.view.background;
    let render = h.app.render.as_ref().unwrap();
    assert!(render.pixels.iter().all(|px| px[..3] == bg));
}

const LABELED_RAMP: &str =
    "V = reshape(1:8, 2, 2, 2); L = zeros(2, 2, 2, 'uint16'); L(2,1,2) = uint16(1);";

fn load_workspace_labels(
    h: &mut Host<VolumeViewer>,
    ws: &Snapshot,
    ctx: &egui::Context,
    name: &str,
) {
    h.app.label_workspace_name = name.into();
    frame(h, ws, ctx, vec![]);
    click(h, ws, ctx, "Load labels");
}

fn assert_generated_ramp_replay(h: &mut Host<VolumeViewer>, ws: &Snapshot, ctx: &egui::Context) {
    let Some(Action::SaveVolumeScript(script, volume, labels)) =
        click(h, ws, ctx, "Generate script")
    else {
        panic!("Generate script should save the ramp and its labels");
    };
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let path = dir.path().join("ramp.m");
    crate::designer_output::save(
        crate::designer_output::Save {
            script,
            path: path.clone(),
            volume,
            labels,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let script = std::fs::read_to_string(&path).unwrap();
    let image = replay_png(&script, dir.path(), Some(&path));
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!(
        image.as_raw(),
        h.app.render.as_ref().unwrap().pixels.as_flattened()
    );
}

#[test]
fn volume_viewer_labels_only_all_black_and_single_red_voxel() {
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    let mut h = host();
    let ctx = egui::Context::default();
    let mut engine = Engine::default();
    engine.execute(LABELED_RAMP).unwrap();
    let mut ws = Snapshot::default();
    ws.sync(&engine.workspace().into_iter().collect::<BTreeMap<_, _>>());
    submit_array(&mut h, &ctx, engine.array("V").unwrap(), "V".into());
    wait(&mut h, &ws, &ctx);
    h.app.view.angles = [0.0, 90.0];
    h.app.view.background = [0; 3];
    let base_images = STYLES.map(|(style, _)| {
        h.app.view.style = style;
        wait(&mut h, &ws, &ctx);
        h.app.render.as_ref().unwrap().pixels.clone()
    });
    load_workspace_labels(&mut h, &ws, &ctx, "L");
    wait(&mut h, &ws, &ctx);
    let label = &mut h.app.labels.as_mut().unwrap().entries[0];
    label.color = [1.0, 0.0, 0.0];
    label.opacity = 1.0;
    for ((style, _), base) in STYLES.into_iter().zip(base_images) {
        h.app.view.style = style;
        h.app.labels.as_mut().unwrap().entries[0].visible = true;
        wait(&mut h, &ws, &ctx);
        click(&mut h, &ws, &ctx, "Labels only");
        wait(&mut h, &ws, &ctx);
        assert!(h.app.labels_only);
        h.app.labels.as_mut().unwrap().entries[0].visible = false;
        wait(&mut h, &ws, &ctx);
        let render = h.app.render.as_ref().unwrap();
        assert_eq!(render.viewport.size, [2, 2]);
        assert_eq!(render.pixels, [BLACK; 4], "{style:?} with hidden label");
        assert_generated_ramp_replay(&mut h, &ws, &ctx);

        h.app.labels.as_mut().unwrap().entries[0].visible = true;
        wait(&mut h, &ws, &ctx);
        assert_eq!(
            h.app.render.as_ref().unwrap().pixels,
            [BLACK, BLACK, RED, BLACK],
            "{style:?} with label at row 2, column 1"
        );
        assert_generated_ramp_replay(&mut h, &ws, &ctx);

        // Extrema projections still show the base with a manually zeroed
        // opacity curve when Labels only is off.
        if matches!(
            style,
            RenderingStyle::MaximumIntensityProjection | RenderingStyle::MinimumIntensityProjection
        ) {
            h.app.alpha_points = vec![[0.0, 0.0], [1.0, 0.0]];
        }
        click(&mut h, &ws, &ctx, "Labels only");
        wait(&mut h, &ws, &ctx);
        assert!(!h.app.labels_only);
        let mut mixed = base.clone();
        mixed[2] = RED;
        assert_eq!(h.app.render.as_ref().unwrap().pixels, mixed, "{style:?}");
        h.app.labels.as_mut().unwrap().entries[0].visible = false;
        wait(&mut h, &ws, &ctx);
        assert_eq!(h.app.render.as_ref().unwrap().pixels, base, "{style:?}");
        h.app.alpha_points = AlphaPreset::Linear.points();
    }
}

#[test]
fn volume_viewer_failed_label_import_resumes_interrupted_camera() {
    const ERROR: &str = "Label value 4097 exceeds 4096 maximum";
    const TIMEOUT: Duration = Duration::from_secs(10);
    let mut h = host();
    let ctx = egui::Context::default();
    let mut engine = Engine::default();
    engine.execute(LABELED_RAMP).unwrap();
    engine.execute("bad = L; bad(1) = uint16(4097);").unwrap();
    let mut ws = Snapshot::default();
    ws.sync(&engine.workspace().into_iter().collect::<BTreeMap<_, _>>());
    h.app.view.style = RenderingStyle::MaximumIntensityProjection;
    submit_array(&mut h, &ctx, engine.array("V").unwrap(), "V".into());
    wait(&mut h, &ws, &ctx);
    load_workspace_labels(&mut h, &ws, &ctx, "L");
    wait(&mut h, &ws, &ctx);
    let good = h.app.labels.as_mut().unwrap();
    good.entries[0].color = [1.0, 0.0, 0.0];
    good.entries[0].opacity = 1.0;
    let overlay = good.overlay.data.clone();
    wait(&mut h, &ws, &ctx);
    let texture_a = h.app.texture.as_ref().unwrap().id();
    let pixels_a = h.app.render.as_ref().unwrap().pixels.clone();

    // Stop each real B worker just before raycasting so frames can inspect
    // the old texture both before and after the import fails.
    let pause_next_render = |app: &mut VolumeViewer| {
        let (start, started) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        app.before_render = Some(Box::new(move || {
            start.send(()).unwrap();
            resume
                .recv_timeout(TIMEOUT)
                .expect("render was not released");
        }));
        (started, release)
    };
    let (started, release) = pause_next_render(&mut h.app);
    h.app.view.angles = [0.0, 90.0];
    frame(&mut h, &ws, &ctx, vec![]);
    started
        .recv_timeout(TIMEOUT)
        .expect("camera B did not start");
    let (retry_started, release_retry) = pause_next_render(&mut h.app);
    load_workspace_labels(&mut h, &ws, &ctx, "bad");
    assert_eq!(h.app.status_text(), Some("Loading labels…"));
    release.send(()).unwrap();

    let deadline = Instant::now() + TIMEOUT;
    let output = loop {
        let (output, _) = frame(&mut h, &ws, &ctx, vec![]);
        if h.app.label_import_error.is_some() {
            break output;
        }
        assert!(Instant::now() < deadline, "label import never finished");
    };
    assert!(output.shapes.iter().any(
        |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == ERROR)
    ));
    assert!(Arc::ptr_eq(
        &h.app.labels.as_ref().unwrap().overlay.data,
        &overlay
    ));
    assert_eq!(h.app.texture.as_ref().unwrap().id(), texture_a);
    assert!(click(&mut h, &ws, &ctx, "Export rendering settings").is_none());
    assert!(click(&mut h, &ws, &ctx, "Generate script").is_none());
    retry_started
        .recv_timeout(TIMEOUT)
        .expect("camera B was not retried");
    release_retry.send(()).unwrap();
    wait(&mut h, &ws, &ctx);

    let render = h.app.render.as_ref().unwrap();
    assert_eq!(render.viewport.view, [0.0, 90.0]);
    assert_eq!(render.viewport.size, [2, 2]);
    assert_ne!(render.pixels, pixels_a);
    // Axial maxima [5 7; 6 8] on the gray ramp, with L(2,1,2) opaque red.
    assert_eq!(
        render.pixels,
        [
            [146, 146, 146, 255],
            [219, 219, 219, 255],
            [255, 0, 0, 255],
            [255; 4]
        ]
    );
    let texture_b = h.app.texture.as_ref().unwrap().id();
    assert_ne!(texture_b, texture_a);
    let (output, _) = frame(&mut h, &ws, &ctx, vec![]);
    assert!(ctx.tessellate(output.shapes, output.pixels_per_point).iter().any(|p| {
        matches!(&p.primitive, egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture_b)
    }));
    assert_eq!(h.app.label_import_error.as_deref(), Some(ERROR));
    let Some(Action::ExportVolumeSettings(settings)) =
        click(&mut h, &ws, &ctx, "Export rendering settings")
    else {
        panic!("Recovered view should export settings");
    };
    settings.execute(&mut engine).unwrap();
    engine.execute("assert(isequal(renderingSettings.CameraPosition, [0 90])); assert(isequal(renderingSettings.OverlayColormap, [1 0 0]));").unwrap();
    assert_generated_ramp_replay(&mut h, &ws, &ctx);
}

#[test]
fn volume_viewer_labels_import_hide_export_and_replay() {
    let mut h = host();
    let ctx = egui::Context::default();
    let mut engine = Engine::default();
    engine
        .execute(
            "V = reshape(double(1:64), 4, 4, 4); \
             L = zeros(4, 4, 4, 'uint8'); \
             L(1:2, 1:4, 1:2) = uint8(1); \
             L(3:4, 1:4, 1:2) = uint8(2); \
             L(1:2, 1:4, 3:4) = uint8(3); \
             wrongSize = zeros(4, 4, 3, 'uint8'); \
             floatVol = single(L);",
        )
        .unwrap();
    let mut ws = Snapshot::default();
    ws.sync(&engine.workspace().into_iter().collect::<BTreeMap<_, _>>());

    // Non-matching size variable and single-precision float are not listed.
    let listed: Vec<&str> = workspace_label_volumes(&ws, [4, 4, 4])
        .map(|(name, _)| name)
        .collect();
    assert_eq!(listed, vec!["L"]);

    // Load base volume V.
    submit_array(&mut h, &ctx, engine.array("V").unwrap(), "V".into());
    wait(&mut h, &ws, &ctx);

    // Import 3-label uint8 volume L via UI.
    h.app.label_workspace_name = "L".into();
    frame(&mut h, &ws, &ctx, vec![]);
    click(&mut h, &ws, &ctx, "Load labels");
    wait(&mut h, &ws, &ctx);

    let labels = h.app.labels.as_ref().unwrap();
    assert_eq!(labels.entries.len(), 3);
    assert_eq!(labels.entries[0].value, 1);
    assert_eq!(labels.entries[0].count, 16);
    assert_eq!(labels.entries[1].value, 2);
    assert_eq!(labels.entries[1].count, 16);
    assert_eq!(labels.entries[2].value, 3);
    assert_eq!(labels.entries[2].count, 16);

    // Hiding label 2 equals rendering with OverlayAlphamap(2)=0.
    h.app.labels.as_mut().unwrap().entries[1].visible = false;
    wait(&mut h, &ws, &ctx);
    let app_pixels = h.app.render.as_ref().unwrap().pixels.clone();

    let mut vol = h.app.loaded.as_ref().unwrap().volume.clone();
    vol.alphamap = Arc::new(h.app.effective_view().alphamap.to_vec());
    vol.overlay_data = Some(h.app.labels.as_ref().unwrap().overlay.clone());
    vol.overlay_rendering_style = OverlayRenderingStyle::LabelOverlay;
    let mut expected_amap = vec![DEFAULT_OVERLAY_ALPHA; 3];
    expected_amap[1] = 0.0;
    vol.overlay_alphamap = Arc::new(expected_amap);
    let expected_pixels = h
        .app
        .effective_view()
        .viewport(&vol)
        .render(&vol, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(app_pixels, expected_pixels);

    // Exported struct has Alphamap and overlay fields.
    h.app.export_name = "lblSettings".into();
    wait(&mut h, &ws, &ctx);
    let Some(Action::ExportVolumeSettings(settings)) =
        click(&mut h, &ws, &ctx, "Export rendering settings")
    else {
        panic!("Export did not capture the displayed labels");
    };
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut fresh = Engine::with_limits(matrust::engine::Limits::local());
    fresh.set_file_directory(dir.path()).unwrap();
    settings.execute(&mut fresh).unwrap();
    let Some(Array::Struct(exported)) = fresh.array("lblSettings") else {
        panic!("Settings were not exported as a struct");
    };
    for name in [
        "Alphamap",
        "OverlayRenderingStyle",
        "OverlayColormap",
        "OverlayAlphamap",
    ] {
        assert!(exported.field_index(name).is_some(), "{name}");
    }

    // Generated script reproduces the render (2/255 rule).
    let Some(Action::SaveVolumeScript(code, volume, labels_companion)) =
        click(&mut h, &ws, &ctx, "Generate script")
    else {
        panic!("Generate script did not request SaveVolumeScript");
    };
    let path = dir.path().join("labeled_view.m");
    crate::designer_output::save(
        crate::designer_output::Save {
            script: code,
            path: path.clone(),
            volume,
            labels: labels_companion,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let script = std::fs::read_to_string(&path).unwrap();
    assert!(script.contains("'OverlayData', L"));
    assert_replay(&h, &script, dir.path(), Some(&path));
}

#[test]
fn volume_viewer_sample_labels_constant_and_40_labels() {
    let mut h = host();
    let ctx = egui::Context::default();
    let ws = Snapshot::default();
    sample(&mut h, &ws, &ctx);

    // Sample labels and Labels only toggle.
    click(&mut h, &ws, &ctx, "Use sample labels");
    wait(&mut h, &ws, &ctx);
    assert_eq!(h.app.labels.as_ref().unwrap().entries.len(), 10);
    click(&mut h, &ws, &ctx, "Labels only");
    wait(&mut h, &ws, &ctx);
    assert!(h.app.labels_only);
    assert!(h.app.effective_view().alphamap.iter().all(|&a| a == 0.0));
    let Some(Action::SaveAppScript(AppKind::VolumeViewer, script)) =
        click(&mut h, &ws, &ctx, "Generate script")
    else {
        panic!("Sample + sample labels should save standalone app script");
    };
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    assert_replay(&h, &script, dir.path(), None);

    // Constant volume (flat histogram) and 40 labels (table scrolls).
    let mut engine = Engine::default();
    engine
        .execute("C = ones(4, 4, 4); L40 = reshape(uint8(mod(0:63, 40) + 1), 4, 4, 4);")
        .unwrap();
    let mut ws2 = Snapshot::default();
    ws2.sync(&engine.workspace().into_iter().collect::<BTreeMap<_, _>>());
    submit_array(&mut h, &ctx, engine.array("C").unwrap(), "C".into());
    wait(&mut h, &ws2, &ctx);
    let hist = h.app.loaded.as_ref().unwrap().histogram;
    assert!(hist.iter().all(|&v| v == hist[0] && v.is_finite()));
    submit_labels(
        &mut h,
        &ctx,
        engine.array("L40").unwrap(),
        "L40".into(),
        [4, 4, 4],
    );
    wait(&mut h, &ws2, &ctx);
    assert_eq!(h.app.labels.as_ref().unwrap().entries.len(), 40);
}
