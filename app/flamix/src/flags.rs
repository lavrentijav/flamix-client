//! ISO 3166-1 alpha-2 -> flag image, embedded at build time (see `build.rs`).

use slint::Image;
use std::cell::RefCell;
use std::collections::HashMap;

include!(concat!(env!("OUT_DIR"), "/flags.rs"));

thread_local! {
    static CACHE: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
}

/// Return the flag for an ISO code (case-insensitive); unknown codes fall back to `UN`.
pub fn flag(iso: &str) -> Image {
    let key = iso.trim().to_ascii_uppercase();
    CACHE.with(|cache| {
        if let Some(image) = cache.borrow().get(&key) {
            return image.clone();
        }
        let image = Image::load_from_svg_data(svg_for(&key))
            .or_else(|_| Image::load_from_svg_data(svg_for("UN")))
            .unwrap_or_default();
        cache.borrow_mut().insert(key.clone(), image.clone());
        image
    })
}
