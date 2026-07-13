mod ansi;
mod font;
mod kitty;
mod png;
mod sauce;
mod text;
mod xbin;

pub use ansi::{Cell, Screen};
pub use kitty::write_screen;
pub use png::encode_screen;
pub use sauce::Sauce;
pub use text::write_screen as write_text;

#[derive(Debug)]
pub struct Document {
    pub screen: Screen,
    pub sauce: Option<Sauce>,
}

pub fn render(data: &[u8], width_override: Option<usize>) -> Result<Document, String> {
    let is_xbin = data.starts_with(b"XBIN\x1a");
    if !is_xbin && let Some(format) = unsupported_format(data) {
        return Err(format!(
            "{format} input is not supported; expected CP437 ANSI, DIZ, or XBin art"
        ));
    }

    let sauce = Sauce::parse(data);
    let content = if is_xbin {
        sauce.as_ref().map_or(data, |s| s.content(data))
    } else {
        sauce
            .as_ref()
            .map_or_else(|| strip_dos_eof(data), |s| s.content(data))
    };
    if is_xbin {
        let screen = xbin::parse(content, width_override)?;
        return Ok(Document { screen, sauce });
    }
    let width = width_override
        .or_else(|| {
            sauce
                .as_ref()
                .and_then(|s| (s.width > 0).then_some(s.width))
        })
        .or_else(|| {
            (!content.contains(&0x1b))
                .then(|| plain_text_width(content))
                .flatten()
        })
        .unwrap_or(80);

    if !(1..=1000).contains(&width) {
        return Err(format!("invalid canvas width {width}; expected 1..=1000"));
    }

    let declared_height = sauce
        .as_ref()
        .and_then(|s| (s.height > 0).then_some(s.height));
    let ice_colors = sauce.as_ref().is_some_and(|s| s.ice_colors);
    let screen = ansi::parse(content, width, declared_height, ice_colors)?;
    Ok(Document { screen, sauce })
}

fn unsupported_format(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("PNG image")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("GIF image")
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("JPEG image")
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        Some("WebP image")
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        Some("TIFF image")
    } else if data.starts_with(&[0, 0, 1, 0]) {
        Some("ICO image")
    } else if is_bmp(data) {
        Some("BMP image")
    } else if data.starts_with(b"qoif") {
        Some("QOI image")
    } else {
        None
    }
}

fn is_bmp(data: &[u8]) -> bool {
    if data.len() < 14 || !data.starts_with(b"BM") || data[6..10] != [0; 4] {
        return false;
    }
    let pixel_offset = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    pixel_offset >= 14
}

fn strip_dos_eof(data: &[u8]) -> &[u8] {
    match data.iter().position(|&byte| byte == 0x1a) {
        Some(end) => &data[..end],
        None => data,
    }
}

fn plain_text_width(data: &[u8]) -> Option<usize> {
    let (mut column, mut widest) = (0_usize, 0_usize);
    for &byte in data {
        match byte {
            b'\r' => column = 0,
            b'\n' => {
                widest = widest.max(column);
                column = 0;
            }
            b'\t' => column = ((column / 8) + 1) * 8,
            0x1a => {}
            _ => column += 1,
        }
        widest = widest.max(column);
    }
    (widest > 0).then_some(widest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_diz_uses_cp437_bytes() {
        let doc = render(b"hello\r\n\xdb", Some(8)).unwrap();
        assert_eq!(doc.screen.width, 8);
        assert_eq!(doc.screen.height, 2);
        assert_eq!(doc.screen.cells[8].character, 0xdb);
    }

    #[test]
    fn plain_diz_uses_its_content_width() {
        let doc = render(b"FILE_ID.DIZ\r\nhello", None).unwrap();
        assert_eq!(doc.screen.width, 11);
    }

    #[test]
    fn plain_diz_counts_cp437_control_range_glyphs() {
        let doc = render(b"\x03\x16", None).unwrap();
        assert_eq!(doc.screen.width, 2);
        assert_eq!(doc.screen.cells[0].character, 0x03);
        assert_eq!(doc.screen.cells[1].character, 0x16);
    }

    #[test]
    fn dos_eof_hides_trailing_bytes() {
        let doc = render(b"ok\x1aignored", Some(8)).unwrap();
        assert_eq!(doc.screen.height, 1);
        assert_eq!(doc.screen.cells[0].character, u16::from(b'o'));
        assert_eq!(doc.screen.cells[2].character, u16::from(b' '));
    }

    #[test]
    fn rejects_png_by_content() {
        let error = render(b"\x89PNG\r\n\x1a\nrest", None).unwrap_err();
        assert_eq!(
            error,
            "PNG image input is not supported; expected CP437 ANSI, DIZ, or XBin art"
        );
    }

    #[test]
    fn rejects_gif_by_content() {
        let error = render(b"GIF89a...", None).unwrap_err();
        assert!(error.starts_with("GIF image input is not supported"));
    }

    #[test]
    fn image_detection_does_not_rely_on_the_filename() {
        for data in [
            b"\xff\xd8\xffjpeg".as_slice(),
            b"RIFF\x01\x00\x00\x00WEBP".as_slice(),
            b"II*\0tiff".as_slice(),
            b"qoifdata".as_slice(),
        ] {
            assert!(render(data, None).is_err());
        }
    }
}
