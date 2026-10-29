#[allow(dead_code)]
#[path = "../src/desktop/image_load.rs"]
mod after;
#[allow(dead_code)]
#[path = "baselines/image-preview/image_load.rs"]
mod before;
#[allow(dead_code)]
#[path = "../src/desktop/path_label.rs"]
mod path_label;
use std::{path::Path, sync::atomic::AtomicBool};
fn main() {
    let cancel = AtomicBool::new(false);
    for name in [
        "fixture.png",
        "fixture.jpg",
        "fixture.bmp",
        "fixture.tiff",
        "large.bmp",
    ] {
        let path = Path::new("target/evidence/plots/image-preview").join(name);
        for side in [1024, 4096] {
            let old = before::load(&path, side, &cancel).unwrap();
            let new = after::load(&path, side, &cancel).unwrap();
            assert_eq!(old.original_size, new.original_size);
            assert_eq!(old.format, new.format);
            assert_eq!(old.pixels.size, new.pixels.size);
            assert_eq!(old.pixels.pixels, new.pixels.pixels);
            println!(
                "{name}: {side}-pixel texture limit, {} pixels agree exactly",
                new.pixels.pixels.len()
            );
        }
    }
}
