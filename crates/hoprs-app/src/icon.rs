//! The app icon and the app id the window manager groups windows by.

/// Wayland app id and X11 class of the window. The installed `.desktop` file mirrors it
/// (file name and `StartupWMClass`), which is how the shell ties the window to its icon.
pub const APP_ID: &str = "io.github.engviniciusbs.hoprs";

/// The 256 px icon, embedded.
const ICON_256: &[u8] = include_bytes!("../assets/icon/hoprs-256.png");

/// The window icon, or `None` if the embedded PNG cannot be decoded (the window then
/// opens with the platform's default icon).
pub fn window_icon() -> Option<iced::window::Icon> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(ICON_256))
        .read_info()
        .ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    iced::window::icon::from_rgba(buf, info.width, info.height).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_icon_decodes_256() {
        let icon = window_icon().expect("the embedded icon decodes");
        let (rgba, size) = icon.into_raw();
        assert_eq!((size.width, size.height), (256, 256));
        assert_eq!(rgba.len(), 256 * 256 * 4);
        assert!(rgba.chunks(4).any(|p| p[3] > 0), "not fully transparent");
    }

    #[test]
    fn icon_pngs_match_their_names() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon");
        for n in [16u32, 24, 32, 48, 64, 128, 256, 512] {
            let bytes = std::fs::read(dir.join(format!("hoprs-{n}.png"))).expect("icon exists");
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{n}: PNG signature");
            // IHDR: length(4) "IHDR"(4) width(4) height(4), right after the signature.
            assert_eq!(&bytes[12..16], b"IHDR");
            let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
            let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
            assert_eq!((w, h), (n, n), "hoprs-{n}.png");
        }
    }

    #[test]
    fn app_id_is_the_reverse_dns_name() {
        assert_eq!(APP_ID, "io.github.engviniciusbs.hoprs");
    }
}
