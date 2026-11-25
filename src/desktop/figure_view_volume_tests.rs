use super::*;
use crate::figure_volumes::BEFORE_RENDER;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);

fn paint(ctx: &egui::Context, figure: &Figure) -> egui::FullOutput {
    use crate::figure_geometry::Geometry;
    let mut geometry = Geometry::prepare(1, figure, &AtomicBool::new(false)).unwrap();
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640., 480.),
            )),
            ..Default::default()
        },
        |ui| match &mut geometry {
            Geometry::Ordinary(data) => {
                show_interactive_prepared(ui, figure, data.view(), true, None);
            }
            Geometry::Subplots(canvas) => {
                let _ = canvas.show_prepared(ui, 1, figure, true, None).unwrap();
            }
        },
    )
}

fn block_next_render() -> (mpsc::Receiver<Arc<AtomicBool>>, mpsc::Sender<()>) {
    let (started, worker) = mpsc::channel();
    let (resume, release) = mpsc::channel();
    BEFORE_RENDER.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |cancel| {
            if started.send(cancel.clone()).is_ok() {
                let _ = release.recv_timeout(TIMEOUT);
            }
        }));
    });
    (worker, resume)
}

fn raster_upload(
    output: &egui::FullOutput,
    side: usize,
) -> Option<(egui::TextureId, egui::Color32)> {
    let (id, image) = plotted_raster(output, [side; 2])?;
    Some((id, image[(side / 2, side / 2)]))
}

fn plotted_raster(
    frame: &egui::FullOutput,
    dims: [usize; 2],
) -> Option<(egui::TextureId, Arc<egui::ColorImage>)> {
    for (id, updates) in &frame.textures_delta.set {
        for update in updates {
            let egui::ImageData::Color(image) = &update.image;
            if *id != egui::TextureId::default() && update.pos.is_none() && image.size == dims {
                return Some((*id, image.clone()));
            }
        }
    }
    None
}

fn uploaded_raster(output: &egui::FullOutput) -> Option<egui::TextureId> {
    raster_upload(output, 16).map(|(id, _)| id)
}

fn paints(ctx: &egui::Context, output: egui::FullOutput, id: egui::TextureId) -> bool {
    ctx.tessellate(output.shapes, output.pixels_per_point)
        .iter()
        .any(|p| {
            matches!(&p.primitive, egui::epaint::Primitive::Mesh(mesh)
            if mesh.texture_id == id && !mesh.indices.is_empty()
                && mesh.vertices.iter().any(|vertex| vertex.color == egui::Color32::WHITE))
        })
}

fn repaint_channel() -> (egui::Context, mpsc::Receiver<egui::ViewportId>) {
    let ctx = egui::Context::default();
    let (notify, wake) = mpsc::channel();
    let ui_thread = std::thread::current().id();
    ctx.set_request_repaint_callback(move |request| {
        if std::thread::current().id() != ui_thread {
            let _ = notify.send(request.viewport_id);
        }
    });
    (ctx, wake)
}

// egui coalesces repaint requests. Drain already scheduled frames while the
// worker is held so its completion will deliver a distinct wake-up.
fn drain_repaints(ctx: &egui::Context, figure: &Figure) {
    for _ in 0..3 {
        if !ctx.has_requested_repaint() {
            break;
        }
        let frame = paint(ctx, figure);
        assert!(
            frame
                .textures_delta
                .set
                .iter()
                .all(|(id, _)| *id == egui::TextureId::default()),
            "a held render uploaded a texture"
        );
    }
    assert!(!ctx.has_requested_repaint());
}

fn finish_render(resume: mpsc::Sender<()>, wake: &mpsc::Receiver<egui::ViewportId>) {
    resume.send(()).unwrap();
    assert_eq!(wake.recv_timeout(TIMEOUT).unwrap(), egui::ViewportId::ROOT);
}

fn settled_frame(
    ctx: &egui::Context,
    figure: &Figure,
    wake: &mpsc::Receiver<egui::ViewportId>,
) -> egui::FullOutput {
    let (worker, resume) = block_next_render();
    let frame = paint(ctx, figure);
    if BEFORE_RENDER.with(|hook| hook.borrow_mut().take().is_some()) {
        return frame; // Cache hit. The frame never submitted a worker.
    }
    worker.recv_timeout(TIMEOUT).unwrap();
    drain_repaints(ctx, figure);
    finish_render(resume, wake);
    paint(ctx, figure)
}

fn black_mip() -> matrust::Engine {
    let mut engine = matrust::Engine::default();
    engine.execute(
        "v=volshow(zeros(2,2,2), RenderingStyle='MaximumIntensityProjection', \
         OverlayColormap=[1 0 0; 0 1 0], OverlayAlphamap=1, OverlayDisplayRange=[2 3]); view(0,90);"
    ).unwrap();
    engine
}

fn plotted_mesh(
    frame: &egui::FullOutput,
    id: egui::TextureId,
) -> (usize, &egui::epaint::ClippedShape, &egui::epaint::Mesh) {
    frame
        .shapes
        .iter()
        .enumerate()
        .find_map(|(i, shape)| match &shape.shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == id => Some((i, shape, mesh.as_ref())),
            _ => None,
        })
        .expect("painted volume mesh")
}

#[test]
fn voxel_and_marker_share_desktop_data_coordinates() {
    for (dims, row, col) in [([3, 3], 1, 2), ([2, 4], 2, 1)] {
        let [ny, nx] = dims;
        let (x_max, y_max) = (nx as f64 + 0.5, ny as f64 + 0.5);
        let mut engine = matrust::Engine::default();
        engine
            .execute(&format!(
                "V=zeros({ny},{nx},2); V({row},{col},1)=1; \
                 volshow(V, RenderingStyle='MaximumIntensityProjection', Colormap='gray'); \
                 view(0,90); hold on; plot({col},{row},'ro'); \
                 axis([.5 {x_max} .5 {y_max}]); axis off;"
            ))
            .unwrap();
        let (ctx, wake) = repaint_channel();
        let first = settled_frame(&ctx, engine.figure.as_ref().unwrap(), &wake);
        let (id, raster) =
            plotted_raster(&first, [nx * 4, ny * 4]).expect("uploaded volume raster");
        engine.execute("axis ij;").unwrap();
        let reverse = settled_frame(&ctx, engine.figure.as_ref().unwrap(), &wake);
        assert!(
            reverse.textures_delta.set.iter().all(|(tex, _)| *tex != id),
            "YDir should reuse the volume texture"
        );
        for (direction, frame) in [("normal", first), ("reverse", reverse)] {
            let (_, image, mesh) = plotted_mesh(&frame, id);
            let mut uv = egui::Vec2::ZERO;
            let mut weight = 0.;
            for (i, &pixel) in raster.pixels.iter().enumerate() {
                let light = f32::from(pixel.r());
                uv += egui::vec2(
                    (i % raster.size[0]) as f32 + 0.5,
                    (i / raster.size[0]) as f32 + 0.5,
                ) * light;
                weight += light;
            }
            assert!(weight > 0.);
            uv.x /= weight * raster.size[0] as f32;
            uv.y /= weight * raster.size[1] as f32;
            let at = |u, v| {
                mesh.vertices
                    .iter()
                    .find(|p| p.uv == egui::pos2(u, v))
                    .unwrap()
                    .pos
            };
            let origin = at(0., 0.);
            let dx = at(1., 0.) - origin;
            let dy = at(0., 1.) - origin;
            let voxel = origin + dx * uv.x + dy * uv.y;
            let (marker, clip) = frame
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) if circle.stroke.color == Color32::RED => {
                        Some((circle.center, shape.clip_rect))
                    }
                    _ => None,
                })
                .expect("painted red circle marker");
            eprintln!("desktop {ny}x{nx} {direction}: voxel={voxel:.2?}, marker={marker:.2?}");
            assert!(image.clip_rect.contains(voxel) && clip.contains(marker));
            assert!((voxel.x - marker.x).abs() <= dx.length() / raster.size[0] as f32);
            assert!((voxel.y - marker.y).abs() <= dy.length() / raster.size[1] as f32);
        }
    }
}

#[test]
fn mip_and_line_stack_in_desktop_child_order_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    for volume_on_top in [true, false] {
        let mut engine = matrust::Engine::default();
        engine.set_file_directory(directory.path()).unwrap();
        let line = "h=plot([-1 5],[2 2],'r-','LineWidth',3);";
        let volume = "v=volshow(zeros(3,3,2), RenderingStyle='MaximumIntensityProjection', Colormap='gray');";
        let (first, last, children) = if volume_on_top {
            (line, volume, "[v;h]")
        } else {
            (volume, line, "[h;v]")
        };
        engine
            .execute(&format!(
                "subplot(1,2,1); plot([0 1],[0 1],'b-'); \
                 ax=subplot(1,2,2); hold on; {first} {last} \
                 view(0,90); axis([-2 6 .5 3.5]); axis off; \
                 assert(isequal(ax.Children,{children})); savefig('stack.mrfig');"
            ))
            .unwrap();
        for reopened in [false, true] {
            if reopened {
                engine
                    .execute(&format!(
                        "close all; openfig('stack.mrfig'); ax=subplot(1,2,2); \
                         h=findobj(ax,'Type','line'); v=findobj(ax,'Type','volume'); \
                         assert(isequal(ax.Children,{children}));"
                    ))
                    .unwrap();
            }
            let (ctx, wake) = repaint_channel();
            let frame = settled_frame(&ctx, engine.figure.as_ref().unwrap(), &wake);
            let (id, raster) = plotted_raster(&frame, [12, 12]).expect("uploaded volume raster");
            assert!(raster.pixels.iter().all(|&p| p == Color32::BLACK));
            let (image_index, image, mesh) = plotted_mesh(&frame, id);
            let rect = mesh.calc_bounds();
            assert!(mesh.vertices.iter().all(|v| v.color == Color32::WHITE));
            let (line_index, clip, segment) = frame
                .shapes
                .iter()
                .enumerate()
                .find_map(|(i, shape)| match &shape.shape {
                    egui::Shape::Path(path)
                        if path.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED) =>
                    {
                        Some((i, shape.clip_rect, path.points.as_slice()))
                    }
                    _ => None,
                })
                .expect("painted red line");
            let center = rect.center();
            let outside = rect.width() * 0.75;
            assert_eq!(segment.len(), 2);
            assert!(segment.iter().all(|p| (p.y - center.y).abs() < 0.1));
            for x in [center.x - outside, center.x, center.x + outside] {
                assert!(segment[0].x < x && segment[1].x > x);
                assert!(clip.contains(egui::pos2(x, center.y)));
            }
            assert!(image.clip_rect.contains(center));
            assert!(!rect.contains(center - egui::vec2(outside, 0.)));
            assert!(!rect.contains(center + egui::vec2(outside, 0.)));
            eprintln!(
                "desktop volume_on_top={volume_on_top}, reopened={reopened}: image={image_index}, line={line_index}, center={center:.2?}"
            );
            assert_eq!(image_index > line_index, volume_on_top);
        }
    }
}

#[test]
fn overlay_setters_repaint_the_volume_pixel() {
    use egui::Color32;
    let (ctx, wake) = repaint_channel();
    let mut engine = black_mip();
    let axes = &engine.figure.as_ref().unwrap().axes;
    let volume = &axes.extras.volumes[0];
    let (identity, base, view) = (volume.identity.clone(), volume.data.clone(), axes.view);
    let mut last = None;
    for (edit, expected) in [
        ("", Color32::BLACK),
        ("v.OverlayData=uint8(ones(2,2,2));", Color32::RED),
        ("", Color32::RED),
        ("v.OverlayData=uint8(2*ones(2,2,2));", Color32::GREEN),
        ("v.OverlayColormap=[1 0 0; 0 0 1];", Color32::BLUE),
        ("v.OverlayAlphamap=0;", Color32::BLACK),
        ("set(v,'OverlayAlphamap',1);", Color32::BLUE),
        ("v.OverlayRenderingStyle='VolumeOverlay';", Color32::RED),
        ("v.OverlayDisplayRange=[1 2];", Color32::BLUE),
        ("v.OverlayData=[];", Color32::BLACK),
    ] {
        if !edit.is_empty() {
            engine.execute(edit).unwrap();
        }
        let figure = engine.figure.as_ref().unwrap();
        let volume = &figure.axes.extras.volumes[0];
        assert!(volume.identity.same_object(&identity));
        assert!(Arc::ptr_eq(&volume.data, &base));
        assert_eq!(figure.axes.view, view);
        let frame = settled_frame(&ctx, figure, &wake);
        let upload = raster_upload(&frame, 8);
        if edit.is_empty() && last.is_some() {
            assert!(upload.is_none(), "unchanged frame reuploaded the raster");
        }
        let (id, pixel) = upload.or(last).expect("initial volume texture");
        assert!(paints(&ctx, frame, id));
        assert_eq!(pixel, expected, "painted center pixel after {edit:?}");
        last = Some((id, pixel));
    }
}

#[test]
fn overlay_edit_retires_the_pending_raster_and_paints_its_replacement() {
    for paint_edit_before_completion in [true, false] {
        let (ctx, wake) = repaint_channel();
        let mut engine = black_mip();
        let base = settled_frame(&ctx, engine.figure.as_ref().unwrap(), &wake);
        let (black, pixel) = raster_upload(&base, 8).unwrap();
        assert_eq!(pixel, egui::Color32::BLACK);
        assert!(paints(&ctx, base, black));
        let assert_placeholder = |frame| {
            assert!(
                raster_upload(&frame, 8).is_none(),
                "published a stale raster"
            );
            assert!(paints(&ctx, frame, black));
        };

        engine.execute("v.OverlayData=uint8(ones(2,2,2));").unwrap();
        let (red, resume_red) = block_next_render();
        let figure = engine.figure.as_ref().unwrap();
        assert_placeholder(paint(&ctx, figure));
        let cancel = red.recv_timeout(TIMEOUT).unwrap();
        drain_repaints(&ctx, figure);

        engine.execute("v.OverlayColormap=[0 1 0];").unwrap();
        let figure = engine.figure.as_ref().unwrap();
        if paint_edit_before_completion {
            assert_placeholder(paint(&ctx, figure));
            drain_repaints(&ctx, figure);
        }
        // Exercise both worker outcomes: cancellation, and a successful red
        // image already queued when the UI first observes the palette edit.
        assert_eq!(cancel.load(Ordering::Relaxed), paint_edit_before_completion);
        finish_render(resume_red, &wake);

        let (green, resume_green) = block_next_render();
        assert_placeholder(paint(&ctx, figure));
        let cancel_green = green.recv_timeout(TIMEOUT).expect("replacement started");
        assert!(!cancel_green.load(Ordering::Relaxed));
        drain_repaints(&ctx, figure);
        finish_render(resume_green, &wake);
        let latest = paint(&ctx, figure);
        let (id, pixel) = raster_upload(&latest, 8).expect("replacement uploaded");
        assert_ne!(id, black);
        assert_eq!(pixel, egui::Color32::GREEN);
        assert!(paints(&ctx, latest, id));
    }
}

#[test]
fn paints_asynchronously_and_cancels_stale_volume_rasters() {
    let (ctx, wake) = repaint_channel();
    let mut engine = matrust::Engine::default();
    engine
        .execute("v=volshow(reshape(1:64,4,4,4),RenderingStyle='Isosurface'); view(0,90);")
        .unwrap();

    let (worker, resume) = block_next_render();
    let first = paint(&ctx, engine.figure.as_ref().unwrap());
    let cancel = worker.recv_timeout(TIMEOUT).unwrap();
    assert!(!cancel.load(Ordering::Relaxed));
    assert!(uploaded_raster(&first).is_none());
    drain_repaints(&ctx, engine.figure.as_ref().unwrap());
    finish_render(resume, &wake);
    let finished = paint(&ctx, engine.figure.as_ref().unwrap());
    let old = uploaded_raster(&finished).expect("worker uploaded a volume raster");
    assert!(paints(&ctx, finished, old));

    let (worker, resume) = block_next_render();
    engine.execute("view(90,0);").unwrap();
    let rotating = paint(&ctx, engine.figure.as_ref().unwrap());
    let cancel = worker.recv_timeout(TIMEOUT).unwrap();
    assert!(paints(&ctx, rotating, old));
    engine.execute("v.IsosurfaceColor=[1 0 0];").unwrap();
    let updating = paint(&ctx, engine.figure.as_ref().unwrap());
    assert!(cancel.load(Ordering::Relaxed));
    assert!(paints(&ctx, updating, old));

    let figure = engine.figure.as_ref().unwrap();
    drain_repaints(&ctx, figure);
    finish_render(resume, &wake);
    let (replacement, resume_replacement) = block_next_render();
    let frame = paint(&ctx, figure);
    assert!(uploaded_raster(&frame).is_none());
    assert!(paints(&ctx, frame, old));
    replacement.recv_timeout(TIMEOUT).unwrap();
    drain_repaints(&ctx, figure);
    finish_render(resume_replacement, &wake);
    let frame = paint(&ctx, figure);
    let latest = uploaded_raster(&frame).expect("replacement uploaded");
    assert_ne!(latest, old);
    assert!(paints(&ctx, frame, latest));
}
