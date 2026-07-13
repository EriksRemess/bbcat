use std::sync::OnceLock;

use crate::font;

// Converted from the BSD-2-Clause libansilove PC 80x50 font and the Zlib
// bgi-stroked-fonts geometry. See THIRD_PARTY_LICENSES.
const BITMAP_DATA: &str = include_str!("bgi_fonts/cp437_f08.b64");
const STROKE_DATA: &str = include_str!("bgi_fonts/strokes.b64");

pub(crate) const SCALE_UP: [i32; 11] = [1, 6, 2, 3, 1, 4, 5, 2, 5, 3, 4];
pub(crate) const SCALE_DOWN: [i32; 11] = [1, 10, 3, 4, 1, 3, 3, 1, 2, 1, 1];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StrokeKind {
    Move,
    Line,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Stroke {
    pub kind: StrokeKind,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct Glyph {
    pub width: i32,
    pub strokes: Vec<Stroke>,
}

#[derive(Clone, Debug)]
pub(crate) struct StrokeFont {
    pub height: i32,
    glyphs: Vec<Option<Glyph>>,
}

impl StrokeFont {
    pub fn glyph(&self, character: u8) -> Option<&Glyph> {
        self.glyphs[usize::from(character)].as_ref()
    }
}

pub(crate) fn bitmap() -> &'static [u8] {
    static BITMAP: OnceLock<Vec<u8>> = OnceLock::new();
    BITMAP.get_or_init(|| {
        let bytes = font::decode(BITMAP_DATA.trim());
        assert_eq!(bytes.len(), 256 * 8, "embedded BGI bitmap font size");
        bytes
    })
}

pub(crate) fn stroke_font(font_index: u8) -> Option<&'static StrokeFont> {
    if !(1..=10).contains(&font_index) {
        return None;
    }
    static FONTS: OnceLock<Vec<StrokeFont>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        parse(&font::decode(STROKE_DATA.trim())).expect("invalid embedded BGI stroke fonts")
    });
    fonts.get(usize::from(font_index - 1))
}

pub(crate) fn scale(value: i32, size: usize) -> i32 {
    value * SCALE_UP[size] / SCALE_DOWN[size]
}

fn parse(data: &[u8]) -> Result<Vec<StrokeFont>, String> {
    if !data.starts_with(b"BGIS\x01") {
        return Err("invalid embedded stroke font header".to_owned());
    }
    let mut position = 5;
    let mut fonts = Vec::with_capacity(10);
    for _ in 0..10 {
        let height = i32::from(byte(data, &mut position)?);
        let character_count = usize::from(word(data, &mut position)?);
        if character_count > 224 {
            return Err("embedded stroke font has too many glyphs".to_owned());
        }
        let mut glyphs = vec![None; 256];
        for relative in 0..character_count {
            let width = i32::from(byte(data, &mut position)?);
            let point_count = usize::from(word(data, &mut position)?);
            if point_count > 4096 {
                return Err("embedded glyph exceeds stroke safety limit".to_owned());
            }
            let mut strokes = Vec::with_capacity(point_count);
            for _ in 0..point_count {
                let encoded_x = byte(data, &mut position)?;
                let y = i32::from(byte(data, &mut position)? as i8);
                strokes.push(Stroke {
                    kind: if encoded_x & 0x80 == 0 {
                        StrokeKind::Move
                    } else {
                        StrokeKind::Line
                    },
                    x: i32::from(encoded_x & 0x7f) - 64,
                    y,
                });
            }
            glyphs[32 + relative] = Some(Glyph { width, strokes });
        }
        fonts.push(StrokeFont { height, glyphs });
    }
    if position != data.len() {
        return Err("trailing data in embedded stroke fonts".to_owned());
    }
    Ok(fonts)
}

fn byte(data: &[u8], position: &mut usize) -> Result<u8, String> {
    let value = *data
        .get(*position)
        .ok_or("truncated embedded stroke fonts")?;
    *position += 1;
    Ok(value)
}

fn word(data: &[u8], position: &mut usize) -> Result<u16, String> {
    let low = byte(data, position)?;
    let high = byte(data, position)?;
    Ok(u16::from_le_bytes([low, high]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_bitmap_and_all_stroke_fonts() {
        assert_eq!(bitmap().len(), 2048);
        for index in 1..=10 {
            let font = stroke_font(index).unwrap();
            assert!(font.height > 0);
            assert!(font.glyph(b'A').is_some());
        }
    }

    #[test]
    fn embedded_strokes_include_moves_and_lines() {
        let glyph = stroke_font(1).unwrap().glyph(b'A').unwrap();
        assert!(
            glyph
                .strokes
                .iter()
                .any(|stroke| stroke.kind == StrokeKind::Move)
        );
        assert!(
            glyph
                .strokes
                .iter()
                .any(|stroke| stroke.kind == StrokeKind::Line)
        );
    }
}
