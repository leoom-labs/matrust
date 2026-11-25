use super::*;

// Use a fixed viewport for the lifetime of each interaction. Keep the image
// uploads a native renderer would consume so pixels can be joined to paint by ID.
struct Viewer {
    h: Host<VolumeViewer>,
    ctx: egui::Context,
    ws: Snapshot,
    screen: egui::Rect,
    shapes: Vec<egui::epaint::ClippedShape>,
    images: BTreeMap<egui::TextureId, Arc<egui::ColorImage>>,
}

impl Viewer {
    fn new(width: f32) -> Self {
        let mut viewer = Self {
            h: host(),
            ctx: egui::Context::default(),
            ws: Snapshot::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 850.0)),
            shapes: Vec::new(),
            images: BTreeMap::new(),
        };
        crate::theme::configure(&viewer.ctx);
        for _ in 0..3 {
            viewer.frame(vec![]);
        }
        viewer
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let mut input = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        crate::input::sanitize(&mut input);
        let mut out = self.ctx.run_ui(input, |ui| {
            show(&mut self.h, ui, &self.ws, false, &mut None)
        });
        self.shapes = out.shapes;
        for (id, deltas) in out.textures_delta.set.drain() {
            if id != egui::TextureId::default() {
                let [delta] = deltas.as_slice() else {
                    panic!("expected a single upload for {id:?}")
                };
                let egui::ImageData::Color(image) = &delta.image;
                assert!(delta.pos.is_none(), "expected a whole viewer image");
                self.images.insert(id, image.clone());
            }
        }
        for id in out.textures_delta.free.drain() {
            self.images.remove(&id);
        }
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.frame(vec![]);
            if !self.h.running() && self.h.app.pending.is_none() {
                assert!(self.h.run_error.is_none(), "{:?}", self.h.run_error);
                return;
            }
            assert!(Instant::now() < deadline, "viewer worker did not settle");
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn visible_text(&self, label: &str) -> Option<egui::Rect> {
        self.shapes.iter().rev().find_map(|s| {
            let egui::Shape::Text(text) = &s.shape else {
                return None;
            };
            let ink = text.visual_bounding_rect();
            let clip = s.clip_rect.intersect(self.screen);
            (text.galley.text() == label
                && !text.galley.elided
                && ink.is_positive()
                && clip.contains_rect(ink))
            .then_some(ink)
        })
    }

    fn text(&self, label: &str) -> egui::Rect {
        self.visible_text(label).unwrap_or_else(|| {
            let painted = self
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some((t.visual_bounding_rect(), s.clip_rect.intersect(self.screen)))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            panic!(
                "no visible caption {label:?} at {:?}; (ink, clip) {painted:?}",
                self.screen.size()
            )
        })
    }

    fn click_at(&mut self, pos: egui::Pos2) {
        assert!(self.screen.contains(pos));
        self.frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        self.frame(vec![pointer(pos, false)]);
        assert!(
            self.ctx
                .interaction_snapshot(|s| s.clicked.is_some() || s.drag_stopped.is_some()),
            "no pointer hit at {pos:?}"
        );
        self.frame(vec![]);
    }

    fn click(&mut self, label: &str) {
        self.click_at(self.text(label).center());
    }

    fn assert_render(&self, style: RenderingStyle) -> egui::TextureId {
        let app = &self.h.app;
        let render = app.current_render().expect("current render");
        assert_eq!(render.settings.style, style);
        let id = app.texture.as_ref().unwrap().id();
        let image = &self.images[&id];
        assert_eq!(image.size, render.viewport.size);
        assert!(
            image
                .pixels
                .iter()
                .map(|p| p.to_array())
                .eq(render.pixels.iter().copied())
        );
        assert!(
            image
                .pixels
                .iter()
                .any(|p| p.to_array()[..3] != app.view.background)
        );
        assert!(
            self.shapes.iter().any(|s| {
                let rect = s.shape.visual_bounding_rect();
                s.shape.texture_id() == id
                    && rect.is_positive()
                    && app.controls["3-D view"].contains_rect(rect)
                    && s.clip_rect.intersect(self.screen).contains_rect(rect)
            }),
            "current 3-D texture {id:?} has no visible paint"
        );
        id
    }

    fn wheel(&mut self, pos: egui::Pos2, delta: f32) {
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., delta),
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        // Flush egui's smoothed wheel delta before hit-testing the new position.
        for _ in 0..24 {
            self.frame(vec![]);
        }
    }

    fn scroll_to(&mut self, label: &str, pos: egui::Pos2, delta: f32) {
        for _ in 0..24 {
            if self.visible_text(label).is_some() {
                return;
            }
            self.wheel(pos, delta);
        }
        self.text(label);
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
    }

    fn load_fixture(&mut self, code: &str, name: &str) {
        let mut engine = Engine::default();
        engine.execute(code).unwrap();
        self.ws.sync(&engine.workspace().into_iter().collect());
        submit_array(
            &mut self.h,
            &self.ctx,
            engine.array(name).unwrap(),
            name.into(),
        );
        self.settle();
    }

    fn slice_rail(&self, plane: &str) -> egui::Rect {
        let control = self.h.app.controls[plane];
        // Use the recorded control only to identify its painted rail. The
        // entire rail must fit in the viewport and the scroll area's clip.
        self.text(plane);
        self.shapes
            .iter()
            .find_map(|s| {
                let egui::Shape::Rect(r) = &s.shape else {
                    return None;
                };
                (control.contains_rect(r.rect)
                    && r.rect.width() > 4. * r.rect.height()
                    && (r.rect.left() - control.left()).abs() < 1.
                    && s.clip_rect.intersect(self.screen).contains_rect(r.rect))
                .then_some(r.rect)
            })
            .unwrap_or_else(|| {
                panic!(
                    "no fully visible {plane} rail in {control:?} at {:?}",
                    self.screen
                )
            })
    }

    fn slice_value(&self, plane: &str, value: usize) {
        let control = self.h.app.controls[plane];
        assert!(
            self.shapes.iter().any(|s| {
                let egui::Shape::Text(t) = &s.shape else {
                    return false;
                };
                let ink = t.visual_bounding_rect();
                t.galley.text() == value.to_string()
                    && control.contains(ink.center())
                    && s.clip_rect.intersect(self.screen).contains_rect(ink)
            }),
            "{plane} value {value} outside visible slider {control:?}"
        );
    }

    fn last_slice(&mut self, plane: &str, value: usize) {
        let rail = self.slice_rail(plane);
        self.click_at(rail.right_center() - egui::vec2(1., 0.));
        self.settle();
        self.slice_value(plane, value);
    }
}

#[test]
fn background_picker_has_the_widget_border_in_each_theme() {
    let mut v = Viewer::new(1400.);
    v.click("Use sample data");
    v.settle();
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        v.ctx.set_theme(theme);
        v.frame(vec![]);
        let background = v.text("Background");
        let border = crate::tokens::palette(&v.ctx).border;
        assert!(v.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(r)
            if (0.0..20.).contains(&(r.rect.left() - background.right())) && r.rect.y_range().contains(background.center().y)
            && r.stroke == egui::Stroke::new(1., border) && r.fill == egui::Color32::TRANSPARENT
            && r.stroke_kind == egui::StrokeKind::Inside && shape.clip_rect.contains_rect(r.rect)
        )), "{theme:?}");
    }
}

#[test]
fn narrow_sample_and_style_by_pointer() {
    let mut v = Viewer::new(390.0);
    v.click("Use sample data");
    v.settle();
    assert!(v.h.app.loaded.as_ref().unwrap().sample);
    let old = v.assert_render(RenderingStyle::VolumeRendering);
    v.click("Volume rendering");
    v.click("Maximum intensity (MIP)");
    v.settle();
    v.text("Maximum intensity (MIP)");
    assert_ne!(
        old,
        v.assert_render(RenderingStyle::MaximumIntensityProjection)
    );
}

#[test]
fn wide_style_choices_paint_the_selected_render() {
    let mut v = Viewer::new(1400.);
    v.click("Use sample data");
    v.settle();
    let mut previous = v.assert_render(RenderingStyle::VolumeRendering);
    let mut caption = "Volume rendering";
    for (style, next) in [
        (
            RenderingStyle::MaximumIntensityProjection,
            "Maximum intensity (MIP)",
        ),
        (
            RenderingStyle::MinimumIntensityProjection,
            "Minimum intensity (MinIP)",
        ),
        (RenderingStyle::Isosurface, "Isosurface"),
        (RenderingStyle::SlicePlanes, "Slice planes"),
        (RenderingStyle::VolumeRendering, "Volume rendering"),
    ] {
        v.click(caption);
        assert!(egui::Popup::is_any_open(&v.ctx));
        v.click(next);
        v.settle();
        assert!(!egui::Popup::is_any_open(&v.ctx));
        v.text(next);
        let current = v.assert_render(style);
        assert_ne!(current, previous);
        previous = current;
        caption = next;
    }
}

#[test]
fn narrow_output_scroll_reaches_last_slice_value() {
    let mut v = Viewer::new(390.);
    v.click("Use sample data");
    v.settle();
    let output = v.text("OUTPUT").center();
    let before = v.h.app.controls["XZ"].center().y;
    v.wheel(output, -280.);
    assert!(
        v.h.app.controls["XZ"].center().y < before,
        "Output did not scroll"
    );
    v.last_slice("XZ", 96);
}

fn assert_plane(
    v: &Viewer,
    plane: &str,
    next_caption: Option<&str>,
    size: [usize; 2],
    pixels: &[u8],
) {
    let caption = v.text(plane);
    let top = v.h.app.controls[plane].bottom();
    let bottom = next_caption.map_or(v.text("Use sample data").top(), |name| v.text(name).top());
    assert!(caption.bottom() <= top);
    // Locate the painted quad in the caption's column and vertical band. No
    // use of the app's slice texture handles, pane order, or slice extractor.
    let mut quads = v.shapes.iter().filter_map(|s| {
        let egui::Shape::Mesh(mesh) = &s.shape else {
            return None;
        };
        let rect = mesh.calc_bounds();
        (v.images.contains_key(&mesh.texture_id)
            && rect.left() >= caption.left() - 2.
            && rect.top() >= top
            && rect.bottom() < bottom
            && s.clip_rect.intersect(v.screen).contains_rect(rect))
        .then_some((mesh, rect))
    });
    let (mesh, rect) = quads
        .next()
        .unwrap_or_else(|| panic!("no image below {plane}"));
    assert!(quads.next().is_none(), "ambiguous {plane} image");
    assert!(
        (rect.width() / rect.height() - size[0] as f32 / size[1] as f32).abs() < 0.01,
        "{plane} {rect:?}"
    );
    // Texture coordinates must put the uploaded first row at the screen top.
    assert_eq!(mesh.vertices.len(), 4);
    for vertex in &mesh.vertices {
        let uv = egui::pos2(
            if vertex.pos.x == rect.left() { 0. } else { 1. },
            if vertex.pos.y == rect.top() { 0. } else { 1. },
        );
        assert_eq!(vertex.uv, uv, "{plane}");
        assert_eq!(vertex.color, egui::Color32::WHITE);
    }
    let image = &v.images[&mesh.texture_id];
    assert_eq!(image.size, size, "{plane}");
    assert_eq!(
        image.pixels,
        pixels
            .iter()
            .map(|&gray| egui::Color32::from_gray(gray))
            .collect::<Vec<_>>(),
        "{plane}"
    );
}

#[test]
fn three_captioned_slice_textures_follow_their_own_sliders() {
    let mut v = Viewer::new(1400.);
    v.load_fixture("V = reshape(11*(0:23),2,3,4); V(end) = 255;", "V");
    // Hand-enumerated rows from V(y,x,z) with a [0,255] gray scale.
    let mut xy: &[u8] = &[66, 88, 110, 77, 99, 121]; // z = 2
    let mut yz: &[u8] = &[22, 88, 154, 220, 33, 99, 165, 231]; // x = 2
    let mut xz: &[u8] = &[0, 22, 44, 66, 88, 110, 132, 154, 176, 198, 220, 242]; // y = 1
    for step in 0..4 {
        match step {
            1 => {
                v.last_slice("XY", 4);
                xy = &[198, 220, 242, 209, 231, 255];
            }
            2 => {
                v.last_slice("YZ", 3);
                yz = &[44, 110, 176, 242, 55, 121, 187, 255];
            }
            3 => {
                v.last_slice("XZ", 2);
                xz = &[11, 33, 55, 77, 99, 121, 143, 165, 187, 209, 231, 255];
            }
            _ => {}
        }
        assert_plane(&v, "XY", Some("YZ"), [3, 2], xy);
        assert_plane(&v, "YZ", Some("XZ"), [4, 2], yz);
        assert_plane(&v, "XZ", None, [3, 4], xz);
    }
}

#[test]
fn cancelled_picker_and_keyboard_scan_recover_to_a_current_sample() {
    let mut v = Viewer::new(390.);
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut directory = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        directory.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let dir = directory.tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    v.ws.set_folder(Some(dir.path().into()));
    v.load_fixture("V = reshape(1:24,2,3,4);", "V");
    // Make Backspace meaningful: the selected point is removable.
    let editor = v.h.app.controls["Alphamap editor"];
    assert!(v.screen.contains_rect(editor));
    assert!(
        v.shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Rect(r)
        if r.rect == editor && s.clip_rect.intersect(v.screen).contains_rect(editor)))
    );
    v.click_at(editor.center());
    v.settle();
    assert_eq!(v.h.app.alpha_selected, Some(1));
    let points = v.h.app.alpha_points.clone();
    assert_eq!(points.len(), 3);

    let old = v.assert_render(RenderingStyle::VolumeRendering);
    let serial = v.h.app.serial;
    v.click("Import");
    v.click("Import DICOM folder"); // test picker returns None
    v.settle();
    assert!(v.h.app.import == Import::Dicom && !v.h.app.browse_requested);
    assert!(v.h.app.folder.is_empty());
    assert_eq!(v.h.app.serial, serial);
    v.text("V");
    assert_eq!(v.assert_render(RenderingStyle::VolumeRendering), old);
    let folder = v.text("DICOM folder");
    let scan = v.text("Scan folder");
    v.click_at(egui::pos2(
        folder.center().x,
        (folder.bottom() + scan.top()) / 2.,
    ));
    let field = v.ctx.memory(|m| m.focused()).expect("folder field focus");
    v.frame(vec![egui::Event::Text("missingx".into())]);
    v.text("missingx");
    v.key(egui::Key::Tab, egui::Modifiers::NONE);
    let scan_id = v.ctx.memory(|m| m.focused()).expect("Scan button focus");
    assert!(
        v.ctx
            .read_response(scan_id)
            .unwrap()
            .interact_rect
            .contains(v.text("Scan folder").center())
    );
    v.key(egui::Key::Tab, egui::Modifiers::SHIFT);
    assert_eq!(v.ctx.memory(|m| m.focused()), Some(field));
    v.key(egui::Key::End, egui::Modifiers::NONE);
    v.key(egui::Key::Backspace, egui::Modifiers::NONE);
    v.text("missing");
    assert_eq!(v.h.app.folder, "missing");
    assert_eq!(v.h.app.alpha_points, points);
    assert_eq!(v.h.app.alpha_selected, Some(1));

    let cause = std::fs::metadata(dir.path().join("missing")).unwrap_err();
    assert_eq!(cause.kind(), std::io::ErrorKind::NotFound);
    v.key(egui::Key::Tab, egui::Modifiers::NONE);
    assert_eq!(v.ctx.memory(|m| m.focused()), Some(scan_id));
    v.key(egui::Key::Enter, egui::Modifiers::NONE);
    v.settle();
    let error = v.h.app.import_error.clone().expect("current scan error");
    assert!(
        error.contains(&format!("dicomCollection: {cause}")),
        "{error}"
    );
    v.text(&error);
    assert_eq!(v.assert_render(RenderingStyle::VolumeRendering), old);
    v.click("Use sample data");
    v.settle();
    assert!(v.h.app.loaded.as_ref().unwrap().sample);
    assert!(v.h.app.import_error.is_none() && v.visible_text(&error).is_none());
    v.text("96 × 96 × 96  double");
    assert_ne!(v.assert_render(RenderingStyle::VolumeRendering), old);
}

#[test]
fn narrow_workspace_and_inner_label_table_reach_id_40() {
    const VOLUME: &str = "reconstructed_reference_volume_for_orientation";
    const LABELS: &str = "segmentation_labels_for_the_reference_volume";
    const LAST: &str = "Region 40 with a long posterior compartment name";
    let names = (1..40)
        .map(|n| format!("'Region {n:02}'"))
        .chain(std::iter::once(format!("'{LAST}'")))
        .collect::<Vec<_>>()
        .join(",");
    let mut engine = Engine::default();
    engine.execute(&format!(
        "{VOLUME} = reshape(1:40,4,5,2); {LABELS} = categorical({VOLUME},1:40,{{{names}}},'Ordinal',true);"
    )).unwrap();
    let mut v = Viewer::new(390.);
    v.ws.sync(&engine.workspace().into_iter().collect());
    v.click("Import");
    v.click("Import from workspace");
    v.click("Select variable");
    v.click(&format!("{VOLUME}  4 × 5 × 2  double"));
    v.text(VOLUME);
    v.click("Load workspace volume");
    v.settle();
    assert_eq!(v.h.app.loaded.as_ref().unwrap().name, VOLUME);
    let inputs = v.text("INPUTS").center();
    v.scroll_to("Import labels", inputs, -160.);
    v.click("Import labels");
    v.click(&format!("{LABELS}  categorical"));
    v.scroll_to("Load labels", inputs, -120.);
    v.click("Load labels");
    v.settle();
    assert_eq!(v.h.app.labels.as_ref().unwrap().name, LABELS);
    v.wheel(inputs, 1600.);
    v.text("INPUTS");
    assert!(v.visible_text("Settings variable").is_none());
    v.scroll_to("Settings variable", inputs, -160.);
    v.wheel(inputs, -160.);
    let anchor = v.text("Settings variable");
    let first = v.text("Region 01").center();
    assert!(v.visible_text(LAST).is_none(), "last row initially visible");
    v.scroll_to(LAST, first, -160.);
    assert!(v.visible_text("Region 01").is_none());
    assert!(
        (v.text("Settings variable").top() - anchor.top()).abs() < 1.,
        "outer Inputs pane scrolled with the table"
    );

    let last = v.text(LAST);
    let mark = |v: &Viewer| {
        v.shapes.iter().find_map(|s| {
            let egui::Shape::Path(path) = &s.shape else {
                return None;
            };
            let ink = path.visual_bounding_rect();
            (path.points.len() == 3
                && ink.right() < last.left()
                && last.y_range().contains(ink.center().y)
                && s.clip_rect.intersect(v.screen).contains_rect(ink))
            .then_some(ink)
        })
    };
    let checked = mark(&v).expect("ID 40 checkmark");
    let entries = |v: &Viewer| {
        v.h.app
            .labels
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .map(|e| {
                (
                    e.value,
                    e.name.clone(),
                    e.count,
                    e.color,
                    e.opacity,
                    e.visible,
                )
            })
            .collect::<Vec<_>>()
    };
    let mut expected = entries(&v);
    assert_eq!(expected.len(), 40);
    assert_eq!(
        (expected[39].0, expected[39].1.as_str(), expected[39].5),
        (40, LAST, true)
    );
    expected[39].5 = false;
    v.click_at(checked.center());
    v.settle();
    assert_eq!(entries(&v), expected);
    v.text(LAST);
    assert!(mark(&v).is_none(), "ID 40 still paints its checkmark");
}
