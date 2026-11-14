//! The app icon and the app id the window manager groups windows by.

/// Wayland app id and X11 class of the window. The installed `.desktop` file mirrors it
/// (file name and `StartupWMClass`), which is how the shell ties the window to its icon.
pub const APP_ID: &str = "io.github.engviniciusbs.hoprs";

/// The 256 px icon, embedded.
const ICON_256: &[u8] = include_bytes!("../assets/icon/hoprs-256.png");

/// The window icon, or `None` if the embedded PNG cannot be decoded (the window then
/// opens with the platform's default icon; the reason is logged).
pub fn window_icon() -> Option<iced::window::Icon> {
    let (rgba, width, height) = match decode_rgba8(ICON_256) {
        Ok(decoded) => decoded,
        Err(err) => {
            tracing::warn!(%err, "cannot decode the embedded window icon");
            return None;
        }
    };
    match iced::window::icon::from_rgba(rgba, width, height) {
        Ok(icon) => Some(icon),
        Err(err) => {
            tracing::warn!(%err, "the window icon was rejected");
            None
        }
    }
}

/// Decodes any PNG to RGBA8: palette, grayscale, gray-alpha, RGB and 16-bit all end up
/// as 4 bytes per pixel.
fn decode_rgba8(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // EXPAND: palette -> RGB(A), low-bit gray -> 8-bit, tRNS -> alpha channel.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("unexpected bit depth {:?}", info.bit_depth));
    }
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err("palette was not expanded".into()),
    };
    Ok((rgba, info.width, info.height))
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
    fn ico_has_the_header_and_a_256_entry() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon");
        let b = std::fs::read(dir.join("hoprs.ico")).expect("hoprs.ico exists");
        assert_eq!(&b[..4], &[0, 0, 1, 0], "ICONDIR: reserved 0, type 1");
        let count = u16::from_le_bytes([b[4], b[5]]) as usize;
        assert_eq!(count, 7);
        // ICONDIRENTRY is 16 bytes; width/height 0 mean 256.
        let widths: Vec<u32> = (0..count)
            .map(|i| match b[6 + i * 16] {
                0 => 256,
                w => u32::from(w),
            })
            .collect();
        assert_eq!(widths, [16, 24, 32, 48, 64, 128, 256]);
        for i in 0..count {
            let e = &b[6 + i * 16..6 + (i + 1) * 16];
            let size = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
            let off = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
            assert_eq!(&b[off..off + 8], b"\x89PNG\r\n\x1a\n", "frame {i} is a PNG");
            assert!(off + size <= b.len());
        }
    }

    #[test]
    fn icns_length_field_equals_the_file_size() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon");
        let b = std::fs::read(dir.join("hoprs.icns")).expect("hoprs.icns exists");
        assert_eq!(&b[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(b[4..8].try_into().unwrap()) as usize,
            b.len()
        );
        // Walk the chunks: each is type(4) + length(4, includes the header) + PNG.
        let mut types = Vec::new();
        let mut at = 8;
        while at < b.len() {
            let len = u32::from_be_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
            assert_eq!(&b[at + 8..at + 16], b"\x89PNG\r\n\x1a\n");
            types.push(String::from_utf8(b[at..at + 4].to_vec()).unwrap());
            at += len;
        }
        assert_eq!(at, b.len());
        for t in ["ic04", "ic05", "ic07", "ic08", "ic09", "ic10"] {
            assert!(types.contains(&t.to_string()), "{t} present");
        }
    }

    #[test]
    fn app_id_is_the_reverse_dns_name() {
        assert_eq!(APP_ID, "io.github.engviniciusbs.hoprs");
    }

    fn encode(
        color: png::ColorType,
        depth: png::BitDepth,
        palette: Option<Vec<u8>>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 2);
        enc.set_color(color);
        enc.set_depth(depth);
        if let Some(palette) = palette {
            enc.set_palette(palette);
        }
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn palette_png_decodes_to_rgba8() {
        // 8-bit indices into a 2-colour palette: red, blue.
        let png = encode(
            png::ColorType::Indexed,
            png::BitDepth::Eight,
            Some(vec![255, 0, 0, 0, 0, 255]),
            &[0, 1, 1, 0],
        );
        let (rgba, w, h) = decode_rgba8(&png).unwrap();
        assert_eq!((w, h, rgba.len()), (2, 2, 16));
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 0, 255, 255]);
    }

    #[test]
    fn sixteen_bit_png_decodes_to_rgba8() {
        // 16-bit RGB, big endian: every sample 0xFF80 -> 0xFF after stripping.
        let data: Vec<u8> = [0xFF, 0x80].repeat(3 * 4);
        let png = encode(png::ColorType::Rgb, png::BitDepth::Sixteen, None, &data);
        let (rgba, w, h) = decode_rgba8(&png).unwrap();
        assert_eq!((w, h, rgba.len()), (2, 2, 16));
        assert_eq!(&rgba[..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn gray_and_gray_alpha_png_decode_to_rgba8() {
        let gray = encode(
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            None,
            &[10; 4],
        );
        let (rgba, _, _) = decode_rgba8(&gray).unwrap();
        assert_eq!(&rgba[..4], &[10, 10, 10, 255]);
        let ga = encode(
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            None,
            &[10, 20].repeat(4),
        );
        let (rgba, _, _) = decode_rgba8(&ga).unwrap();
        assert_eq!((rgba.len(), &rgba[..4]), (16, &[10, 10, 10, 20][..]));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_rgba8(b"not a png").is_err());
    }

    const DESKTOP: &str =
        include_str!("../../../packaging/linux/io.github.engviniciusbs.hoprs.desktop");

    #[test]
    fn desktop_entry_icon_and_wm_class_match_app_id() {
        let line = |key: &str| {
            DESKTOP
                .lines()
                .find_map(|l| l.strip_prefix(key))
                .map(str::to_owned)
        };
        assert_eq!(line("Icon=").as_deref(), Some(APP_ID));
        assert_eq!(line("StartupWMClass=").as_deref(), Some(APP_ID));
        assert_eq!(line("Exec=").as_deref(), Some("hoprs"));
    }
}
