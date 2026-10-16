//! Tray icon loading.
//!
//! The normal icon is the packaged PNG (64 px). The "disconnected" variant is produced at
//! runtime by desaturating the same pixels — no extra asset files are shipped.

/// Packaged 64 px application icon, embedded at compile time so the tray works from any cwd.
const ICON_PNG: &[u8] = include_bytes!("../../assets/app-icon/64.png");

/// A decoded RGBA8 image (row-major, 4 bytes per pixel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Decode a PNG into RGBA8. Returns an error string (never panics) on malformed input.
pub fn decode_png(bytes: &[u8]) -> Result<Rgba, String> {
    let mut decoder = png::Decoder::new(bytes);
    // Expand palette/tRNS and add an alpha channel so the output is always RGBA8.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let bytes = &buf[..info.buffer_size()];
    let (width, height) = (info.width, info.height);
    let data = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".to_string()),
    };
    Ok(Rgba {
        width,
        height,
        data,
    })
}

/// Convert every pixel to its luma in place, keeping the alpha channel (grey variant).
pub fn desaturate_rgba(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let luma = (0.299 * f32::from(px[0]) + 0.587 * f32::from(px[1]) + 0.114 * f32::from(px[2]))
            .round()
            .clamp(0.0, 255.0) as u8;
        px[0] = luma;
        px[1] = luma;
        px[2] = luma;
    }
}

/// The embedded icon, optionally desaturated for the "disconnected" state.
/// `None` only if the bundled PNG cannot be decoded (should never happen).
pub fn load(grey: bool) -> Option<Rgba> {
    let mut rgba = decode_png(ICON_PNG).ok()?;
    if grey {
        desaturate_rgba(&mut rgba.data);
    }
    Some(rgba)
}

/// Convert RGBA8 to the ARGB32 network-byte-order layout used by StatusNotifierItem (ksni).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.rotate_right(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desaturate_makes_pixels_grey_but_keeps_alpha() {
        let mut px = [255u8, 0, 0, 200];
        desaturate_rgba(&mut px);
        assert_eq!(px[0], px[1]);
        assert_eq!(px[1], px[2]);
        assert_eq!(px[3], 200);
    }

    #[test]
    fn desaturate_ignores_trailing_bytes() {
        let mut data = vec![10u8, 20, 30, 40, 99];
        desaturate_rgba(&mut data);
        assert_eq!(data[4], 99);
    }

    #[test]
    fn embedded_icon_decodes_to_rgba() {
        let img = load(false).expect("bundled icon must decode");
        assert!(img.width > 0 && img.height > 0);
        assert_eq!(img.data.len(), (img.width * img.height * 4) as usize);
    }

    #[test]
    fn grey_variant_differs_from_normal() {
        let normal = load(false).unwrap();
        let grey = load(true).unwrap();
        assert_eq!(normal.width, grey.width);
        assert_ne!(normal.data, grey.data);
    }

    #[test]
    fn argb_conversion_rotates_channels() {
        let argb = rgba_to_argb(&[1, 2, 3, 4]);
        assert_eq!(argb, vec![4, 1, 2, 3]);
    }
}
