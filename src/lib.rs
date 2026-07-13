use std::path::Path;

mod adf;
mod ansi;
mod bgi_font;
mod font;
mod kitty;
mod png;
mod rip;
mod sauce;
mod text;
mod xbin;

pub use ansi::{Cell, Screen};
pub use kitty::{
    write_screen, write_screen_cropped, write_screen_fit, write_screen_scaled,
    write_screen_scaled_cropped, write_screen_scaled_fit, write_screen_slow,
    write_screen_slow_cropped, write_screen_slow_fit, write_screen_slow_scaled,
    write_screen_slow_scaled_cropped, write_screen_slow_scaled_fit,
};
pub use png::{encode_screen, encode_screen_scaled};
pub use sauce::Sauce;
pub use text::{
    write_screen as write_text, write_screen_cropped as write_text_cropped,
    write_screen_slow as write_text_slow, write_screen_slow_cropped as write_text_slow_cropped,
};

const MAX_ANSI_WIDTH: usize = 10_000;
const MAX_INFERRED_WIDTH: usize = 1_000;

#[derive(Debug)]
pub struct Document {
    pub screen: Screen,
    pub sauce: Option<Sauce>,
}

pub fn render(data: &[u8], width_override: Option<usize>) -> Result<Document, String> {
    render_inner(data, width_override, false, false)
}

pub fn render_named(
    data: &[u8],
    width_override: Option<usize>,
    name: &str,
) -> Result<Document, String> {
    let adf_hint = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("adf"));
    let rip_hint = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rip"));
    render_inner(data, width_override, adf_hint, rip_hint)
}

fn render_inner(
    data: &[u8],
    width_override: Option<usize>,
    adf_hint: bool,
    rip_hint: bool,
) -> Result<Document, String> {
    let is_xbin = data.starts_with(b"XBIN\x1a");
    if !is_xbin && let Some(format) = unsupported_format(data) {
        return Err(format!(
            "{format} input is not supported; expected ANSI, DIZ, ADF, RIPscrip, or XBin art"
        ));
    }

    let sauce = Sauce::parse(data);
    let binary_content = sauce.as_ref().map_or(data, |sauce| sauce.content(data));
    if rip_hint || rip::is_rip(binary_content) {
        let screen = rip::parse(binary_content, width_override)?;
        return Ok(Document { screen, sauce });
    }
    if !is_xbin && (adf_hint || adf::is_adf(binary_content)) {
        let screen = adf::parse(binary_content, width_override)?;
        return Ok(Document { screen, sauce });
    }
    let content = if is_xbin {
        binary_content
    } else {
        sauce
            .as_ref()
            .map_or_else(|| strip_dos_eof(data), |s| s.content(data))
    };
    if is_xbin {
        let screen = xbin::parse(content, width_override)?;
        return Ok(Document { screen, sauce });
    }
    let declared_width = width_override.or_else(|| {
        sauce
            .as_ref()
            .and_then(|s| (s.width > 0).then_some(s.width))
    });
    let width = declared_width
        .or_else(|| {
            (!content.contains(&0x1b))
                .then(|| plain_text_width(content))
                .flatten()
        })
        .unwrap_or(80);

    let maximum_width = if declared_width.is_some() {
        MAX_ANSI_WIDTH
    } else {
        MAX_INFERRED_WIDTH
    };
    if !(1..=maximum_width).contains(&width) {
        return Err(format!(
            "invalid canvas width {width}; expected 1..={maximum_width}"
        ));
    }

    let declared_height = sauce
        .as_ref()
        .and_then(|s| (s.height > 0).then_some(s.height));
    let ice_colors = sauce.as_ref().is_some_and(|s| s.ice_colors);
    let mut screen = ansi::parse(content, width, declared_height, ice_colors)?;
    if let Some(selected) = sauce
        .as_ref()
        .and_then(|sauce| font::sauce_font(&sauce.font_name))
    {
        screen.glyph_height = selected.glyph_height;
        screen.font = Some(selected.glyphs.to_vec());
    }
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
            "PNG image input is not supported; expected ANSI, DIZ, ADF, RIPscrip, or XBin art"
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

    #[test]
    fn named_adf_inputs_are_validated_as_adf() {
        let error = render_named(b"not an ADF", None, "broken.ADF").unwrap_err();
        assert!(error.contains("truncated ADF header"));
    }

    #[test]
    fn signed_binary_formats_take_precedence_over_an_adf_extension() {
        let error = render_named(b"XBIN\x1a", None, "misnamed.adf").unwrap_err();
        assert!(error.contains("truncated XBin header"));
    }

    #[test]
    fn named_rip_inputs_are_validated_as_ripscrip() {
        let error = render_named(b"not RIPscrip", None, "broken.rip").unwrap_err();
        assert!(error.contains("RIPscrip header"));
    }

    #[test]
    fn image_signatures_take_precedence_over_a_rip_extension() {
        let error = render_named(b"GIF89a...", None, "misnamed.rip").unwrap_err();
        assert!(error.contains("GIF image input is not supported"));
    }

    #[test]
    fn adf_detection_honors_a_sauce_content_length() {
        let content_len = 1 + 192 + 4096 + 160;
        let mut data = vec![0_u8; content_len];
        data[0] = 1;
        data.push(0x1a);
        let mut record = [0_u8; 128];
        record[..7].copy_from_slice(b"SAUCE00");
        record[90..94].copy_from_slice(&(content_len as u32).to_le_bytes());
        data.extend(record);

        let document = render(&data, None).unwrap();
        assert_eq!((document.screen.width, document.screen.height), (80, 1));
        assert!(document.sauce.is_some());
    }

    #[test]
    fn sauce_vga50_selects_the_8x8_font() {
        let content = b"A";
        let mut data = content.to_vec();
        data.push(0x1a);
        let mut record = [0_u8; 128];
        record[..7].copy_from_slice(b"SAUCE00");
        record[90..94].copy_from_slice(&(content.len() as u32).to_le_bytes());
        record[96..98].copy_from_slice(&80_u16.to_le_bytes());
        record[98..100].copy_from_slice(&1_u16.to_le_bytes());
        record[106..115].copy_from_slice(b"IBM VGA50");
        data.extend(record);

        let document = render(&data, None).unwrap();
        assert_eq!(document.screen.glyph_height, 8);
        assert_eq!(document.screen.font.as_deref(), Some(font::glyphs_8x8()));
    }

    #[test]
    fn sauce_selects_named_custom_fonts() {
        for name in ["Amiga MicroKnight", "Amiga Topaz 2+", "Empathy by Skaboy"] {
            let content = b"A";
            let mut data = content.to_vec();
            data.push(0x1a);
            let mut record = [0_u8; 128];
            record[..7].copy_from_slice(b"SAUCE00");
            record[90..94].copy_from_slice(&(content.len() as u32).to_le_bytes());
            record[96..98].copy_from_slice(&80_u16.to_le_bytes());
            record[98..100].copy_from_slice(&1_u16.to_le_bytes());
            record[106..106 + name.len()].copy_from_slice(name.as_bytes());
            data.extend(record);

            let document = render(&data, None).unwrap();
            assert_eq!(document.screen.glyph_height, 16, "{name}");
            assert_eq!(document.screen.font.as_ref().unwrap().len(), 4096, "{name}");
            assert!(document.screen.utf8_supported, "{name}");
        }
    }

    #[test]
    fn accepts_extra_wide_ansi_within_the_cell_limit() {
        let document = render(b"wide", Some(1_750)).unwrap();
        assert_eq!((document.screen.width, document.screen.height), (1_750, 1));
    }

    #[test]
    fn rejects_excessive_ansi_widths() {
        let error = render(b"wide", Some(MAX_ANSI_WIDTH + 1)).unwrap_err();
        assert!(error.contains("invalid canvas width"));
        assert!(error.contains("10000"));
    }

    #[test]
    fn rejects_excessive_inferred_plain_text_widths() {
        let error = render(&vec![b'x'; MAX_INFERRED_WIDTH + 1], None).unwrap_err();
        assert!(error.contains("invalid canvas width"));
        assert!(error.contains("1000"));
    }
}
