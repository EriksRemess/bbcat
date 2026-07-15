//! Embedded bitmap fonts used while turning character cells into pixels.
//!
//! Each glyph is eight pixels wide and stored as one byte per scanline, most
//! significant bit first. The base64 source files keep binary font data easy to
//! include in the crate; [`OnceLock`] decodes each font only on first use.

use std::sync::OnceLock;

const ENCODED: &str = include_str!("font_data.b64");
const ENCODED_8X8: &str = include_str!("bgi_fonts/cp437_f08.b64");
const AMIGA_MICROKNIGHT: &str = include_str!("fonts/amiga_microknight.b64");
const AMIGA_TOPAZ_2_PLUS: &str = include_str!("fonts/amiga_topaz_2_plus.b64");
const EMPATHY_BY_SKABOY: &str = include_str!("fonts/empathy_by_skaboy.b64");

pub(crate) struct SauceFont {
    pub(crate) glyphs: &'static [u8],
    pub(crate) glyph_height: usize,
}

pub fn glyphs() -> &'static [u8] {
    static GLYPHS: OnceLock<Vec<u8>> = OnceLock::new();
    GLYPHS.get_or_init(|| {
        let bytes = decode(ENCODED.trim());
        assert_eq!(
            bytes.len(),
            256 * 16,
            "embedded VGA font has the wrong size"
        );
        bytes
    })
}

pub(crate) fn glyphs_8x8() -> &'static [u8] {
    static GLYPHS: OnceLock<Vec<u8>> = OnceLock::new();
    GLYPHS.get_or_init(|| {
        let bytes = decode(ENCODED_8X8.trim());
        assert_eq!(
            bytes.len(),
            256 * 8,
            "embedded VGA 8x8 font has the wrong size"
        );
        bytes
    })
}

fn amiga_microknight() -> &'static [u8] {
    static GLYPHS: OnceLock<Vec<u8>> = OnceLock::new();
    GLYPHS.get_or_init(|| decode_8x16(AMIGA_MICROKNIGHT, "Amiga MicroKnight"))
}

fn amiga_topaz_2_plus() -> &'static [u8] {
    static GLYPHS: OnceLock<Vec<u8>> = OnceLock::new();
    GLYPHS.get_or_init(|| decode_8x16(AMIGA_TOPAZ_2_PLUS, "Amiga Topaz 2+"))
}

fn empathy_by_skaboy() -> &'static [u8] {
    static GLYPHS: OnceLock<Vec<u8>> = OnceLock::new();
    GLYPHS.get_or_init(|| decode_8x16(EMPATHY_BY_SKABOY, "Empathy by Skaboy"))
}

fn decode_8x16(encoded: &str, name: &str) -> Vec<u8> {
    let bytes = decode(encoded.trim());
    assert_eq!(
        bytes.len(),
        256 * 16,
        "embedded {name} font has the wrong size"
    );
    bytes
}

pub(crate) fn sauce_font(name: &str) -> Option<SauceFont> {
    let (glyphs, glyph_height): (&[u8], usize) = if name.eq_ignore_ascii_case("IBM VGA50") {
        (glyphs_8x8(), 8)
    } else if name.eq_ignore_ascii_case("Amiga MicroKnight") {
        (amiga_microknight(), 16)
    } else if name.eq_ignore_ascii_case("Amiga Topaz 2+") {
        (amiga_topaz_2_plus(), 16)
    } else if name.eq_ignore_ascii_case("Empathy by Skaboy") {
        (empathy_by_skaboy(), 16)
    } else {
        return None;
    };
    Some(SauceFont {
        glyphs,
        glyph_height,
    })
}

pub(crate) fn decode(input: &str) -> Vec<u8> {
    let encoded: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let mut output = Vec::with_capacity(encoded.len() / 4 * 3);
    for group in encoded.chunks(4) {
        if group.len() != 4 {
            break;
        }
        let value = (u32::from(six_bits(group[0])) << 18)
            | (u32::from(six_bits(group[1])) << 12)
            | (u32::from(six_bits(group[2])) << 6)
            | u32::from(six_bits(group[3]));
        output.push((value >> 16) as u8);
        if group[2] != b'=' {
            output.push((value >> 8) as u8);
        }
        if group[3] != b'=' {
            output.push(value as u8);
        }
    }
    output
}

fn six_bits(byte: u8) -> u8 {
    match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        b'=' => 0,
        _ => panic!("invalid base64 in embedded VGA font"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_has_all_cp437_glyphs() {
        assert_eq!(glyphs().len(), 4096);
        assert!(
            glyphs()[0xdb * 16..0xdc * 16]
                .iter()
                .all(|&row| row == 0xff)
        );
        assert_eq!(glyphs_8x8().len(), 2048);
        assert!(
            glyphs_8x8()[0xdb * 8..0xdc * 8]
                .iter()
                .all(|&row| row == 0xff)
        );
    }

    #[test]
    fn sauce_fonts_have_complete_bitmap_tables() {
        for name in ["Amiga MicroKnight", "Amiga Topaz 2+", "Empathy by Skaboy"] {
            let font = sauce_font(name).unwrap();
            assert_eq!(font.glyphs.len(), 4096, "{name}");
            assert_eq!(font.glyph_height, 16, "{name}");
        }
    }

    #[test]
    fn sauce_font_names_are_case_insensitive() {
        assert!(sauce_font("amiga microknight").is_some());
        assert!(sauce_font("AMIGA TOPAZ 2+").is_some());
        assert!(sauce_font("empathy BY skaboy").is_some());
        assert!(sauce_font("unknown").is_none());
    }
}
