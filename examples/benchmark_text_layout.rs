//! Warm CPU layout costs; excludes GPU rendering, startup and file I/O.
use eframe::egui::{self, Color32, FontId};
use std::{hint::black_box, time::Instant};

fn main() {
    println!("case,mode,utf8_bytes,repetitions,iterations,median_us,min_us,max_us");
    for (name, text) in [
        ("ascii_label", "Signal amplitude (mV)".to_owned()),
        (
            "unicode_label",
            "Cafe\u{301} a\u{301} e\u{302} \u{3b1} \u{3a9} \u{221e}".to_owned(),
        ),
        ("ascii_editor", "x = sin(t); % signal\n".repeat(256)),
        (
            "unicode_editor",
            "% Cafe\u{301} a\u{301} e\u{302} \u{3b1} \u{3a9} \u{221e}\nx = sin(t);\n".repeat(128),
        ),
    ] {
        for cached in [false, true] {
            let ctx = egui::Context::default();
            let mut initial = ctx.run_ui(Default::default(), |_| {});
            initial.textures_delta.clear();
            let font = if name.ends_with("editor") {
                FontId::monospace(14.)
            } else {
                FontId::proportional(14.)
            };
            let mut samples = Vec::new();
            let iterations = if cached { 100 } else { 10 };
            for sample in 0..14 {
                let start = Instant::now();
                for iteration in 0..iterations {
                    // Unique suffix bypasses the layout cache for uncached cases.
                    let input = if cached {
                        text.clone()
                    } else {
                        format!("{text} [{sample}:{iteration}]")
                    };
                    black_box(ctx.fonts_mut(|fonts| {
                        fonts.layout(input, font.clone(), Color32::BLACK, 800.)
                    }));
                }
                let us = start.elapsed().as_secs_f64() * 1e6 / iterations as f64;
                if sample >= 3 {
                    samples.push(us);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{name},{},{},11,{iterations},{:.6},{:.6},{:.6}",
                if cached { "cached" } else { "uncached" },
                text.len(),
                samples[5],
                samples[0],
                samples[10]
            );
        }
    }
}
