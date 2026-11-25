use super::*;

const BLACK: [u8; 4] = [0, 0, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

fn private_output() -> tempfile::TempDir {
    let mut dir = tempfile::Builder::new();
    dir.prefix("volume-label-export-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        dir.permissions(std::fs::Permissions::from_mode(0o700));
    }
    dir.tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap()
}

fn assert_mask_pixels(path: &Path, expected: &[[u8; 4]]) {
    let image = image::open(path).unwrap().into_rgba8();
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!(expected.len(), 4);
    for (i, (pixel, wanted)) in image.pixels().zip(expected).enumerate() {
        assert!(
            pixel.0.iter().zip(wanted).all(|(a, b)| a.abs_diff(*b) <= 2),
            "pixel {i}: {pixel:?} != {wanted:?}"
        );
    }
}

// Codes 1 and 3 are occupied. Code 2 is unused, and zero is undefined.
fn load_mask(
    h: &mut Host<VolumeViewer>,
    engine: &mut Engine,
    ctx: &egui::Context,
    red_voxel: usize,
) -> Snapshot {
    engine
        .execute(&format!(
            "V = reshape(1:8,2,2,2); M = zeros(2,2,2); M({red_voxel}) = 91; M(4) = 7; \
         C = categorical(M,[7 23 91],{{'z hidden','unused','red''s voxel'}},'Ordinal',true);"
        ))
        .unwrap();
    let mut ws = Snapshot::default();
    ws.sync(&engine.workspace().into_iter().collect::<BTreeMap<_, _>>());
    if h.app.loaded.is_none() {
        submit_array(h, ctx, engine.array("V").unwrap(), "V".into());
        wait(h, &ws, ctx);
    }
    load_workspace_labels(h, &ws, ctx, "C");
    wait(h, &ws, ctx);
    let entries = &mut h.app.labels.as_mut().unwrap().entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(
        (entries[0].value, entries[0].name.as_str()),
        (1, "z hidden")
    );
    assert_eq!(
        (entries[1].value, entries[1].name.as_str()),
        (3, "red's voxel")
    );
    entries[0].visible = false;
    entries[1].color = [1.0, 0.0, 0.0];
    entries[1].opacity = 1.0;
    h.app.labels_only = true;
    h.app.view.angles = [0.0, 90.0];
    h.app.view.background = [0; 3];
    wait(h, &ws, ctx);
    ws
}

#[test]
fn categorical_generate_script_replays_full_viewport() {
    let dir = private_output();
    let mut h = host();
    let ctx = egui::Context::default();
    let mut engine = Engine::default();
    let ws = load_mask(&mut h, &mut engine, &ctx, 6);
    assert_eq!(
        h.app.render.as_ref().unwrap().pixels,
        [BLACK, BLACK, RED, BLACK]
    );
    let Some(Action::SaveVolumeScript(script, volume, labels)) =
        click(&mut h, &ws, &ctx, "Generate script")
    else {
        panic!("Generate script did not request a companion");
    };
    let path = dir.path().join("ordinal_mask_view.m");
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
    // Inject a rejected MAT variable name while replacing a working script.
    let error = crate::designer_output::save(
        crate::designer_output::Save {
            path: path.clone(),
            script: "invalid replacement".into(),
            volume: None,
            labels: Some(("invalid name".into(), engine.array("C").unwrap())),
        },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.contains("Invalid variable name"), "{error}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), script);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);

    let mut fresh = Engine::default();
    fresh.set_file_directory(dir.path()).unwrap();
    assert_eq!(fresh.workspace_count(), 0);
    fresh.execute_document(&script, &path).unwrap();
    fresh.execute(
        "assert(isequal(vv_settings.OverlayCategoryNames,{'z hidden';'unused';'red''s voxel'})); \
         assert(vv_settings.OverlayOrdinal); \
         assert(isequal(vv_object.OverlayData,reshape(uint32([0 0 0 1 0 3 0 0]),2,2,2))); \
         assert(isequal(vv_settings.OverlayAlphamap,[0 0 1]));"
    ).unwrap();
    assert_mask_pixels(
        &dir.path().join("volume_viewer.png"),
        &h.app.render.as_ref().unwrap().pixels,
    );
}

#[test]
fn workspace_settings_replay_both_masks_after_the_source_is_cleared() {
    let dir = private_output();
    let mut engine = Engine::default();
    engine.set_file_directory(dir.path()).unwrap();
    let mut h = host();
    let ctx = egui::Context::default();
    let mut views = Vec::new();
    for (name, red_voxel, expected) in [
        ("firstView", 6, [BLACK, BLACK, RED, BLACK]),
        ("secondView", 7, [BLACK, RED, BLACK, BLACK]),
    ] {
        let ws = load_mask(&mut h, &mut engine, &ctx, red_voxel);
        let pixels = h.app.render.as_ref().unwrap().pixels.clone();
        assert_eq!(pixels, expected);
        h.app.export_name = name.into();
        frame(&mut h, &ws, &ctx, vec![]);
        let Some(Action::ExportVolumeSettings(settings)) =
            click(&mut h, &ws, &ctx, "Export rendering settings")
        else {
            panic!("Expected an export with the displayed mask");
        };
        let source = engine.array("C").unwrap();
        engine.execute("clear C M").unwrap();
        crate::worker::perform(
            &mut engine,
            crate::worker::Job::ExportVolumeSettings(settings, dir.path().to_owned()),
        )
        .unwrap();
        let Some(Array::Struct(exported)) = engine.array(name) else {
            panic!("Missing settings");
        };
        let mask = exported
            .get(exported.field_index("OverlayData").unwrap(), 0)
            .unwrap();
        assert!(mask.same_snapshot(&source));
        views.push((name, pixels));
    }
    engine
        .execute(
            "assert(isequal(firstView.OverlayColormap,secondView.OverlayColormap)); \
        assert(isequal(firstView.OverlayAlphamap,secondView.OverlayAlphamap));",
        )
        .unwrap();

    for (name, pixels) in views {
        engine.execute(&format!(
            "S = {name}; figure; obj = volshow(V, 'RenderingStyle', S.RenderingStyle, \
             'Colormap', S.Colormap, 'Alphamap', S.Alphamap, 'IsosurfaceValue', S.IsosurfaceValue, \
             'Transformation', diag([S.ScaleFactors 1]), 'OverlayData', S.OverlayData, \
             'OverlayRenderingStyle', S.OverlayRenderingStyle, 'OverlayColormap', S.OverlayColormap, \
             'OverlayAlphamap', S.OverlayAlphamap); \
             rgb = builtin('volumeViewer','replayViewport',obj,[S.CameraPosition S.Zoom],[2 2],S.BackgroundColor); \
             imwrite(rgb,'{name}.png');"
        )).unwrap();
        assert_mask_pixels(&dir.path().join(format!("{name}.png")), &pixels);
    }
}

#[test]
fn failed_overlay_export_preserves_the_previous_settings() {
    let mut h = host();
    let ctx = egui::Context::default();
    let ws = load_mask(&mut h, &mut Engine::default(), &ctx, 6);
    let Some(Action::ExportVolumeSettings(labeled)) =
        click(&mut h, &ws, &ctx, "Export rendering settings")
    else {
        panic!("Expected the captured mask");
    };
    click(&mut h, &ws, &ctx, "Clear labels");
    wait(&mut h, &ws, &ctx);
    let Some(Action::ExportScript(plain)) = click(&mut h, &ws, &ctx, "Export rendering settings")
    else {
        panic!("Expected settings without a mask");
    };
    let mut destination = Engine::default();
    crate::apps::execute_export(&mut destination, &plain).unwrap();
    let good = destination.array("renderingSettings").unwrap();
    assert!(matches!(&good, Array::Struct(s) if s.field_index("OverlayData").is_none()));

    // The budget admits the palette and the transaction's scalar completion
    // mark, but cannot hold the struct with its additional OverlayData field.
    let mut preview = destination.clone();
    crate::apps::execute_export(&mut preview, &labeled.script).unwrap();
    destination.limits_set(matrust::engine::Limits {
        memory_bytes: preview.array("renderingSettings").unwrap().bytes() + 8,
        ..destination.limits_active()
    });
    crate::apps::execute_export(&mut destination.clone(), &labeled.script).unwrap();
    let error = labeled.execute(&mut destination).unwrap_err();
    assert!(error.contains("Workspace exceeds"), "{error}");
    assert!(
        destination
            .array("renderingSettings")
            .unwrap()
            .same_snapshot(&good)
    );
}
