use crate::{
    ansi::{Cell, MAX_CELLS, Screen},
    png::VGA_PALETTE,
};

const HEADER_LENGTH: usize = 11;
const PALETTE_LENGTH: usize = 16 * 3;
const FLAG_PALETTE: u8 = 0x01;
const FLAG_FONT: u8 = 0x02;
const FLAG_COMPRESSED: u8 = 0x04;
const FLAG_NON_BLINK: u8 = 0x08;
const FLAG_FONT_512: u8 = 0x10;

pub fn parse(data: &[u8], width_override: Option<usize>) -> Result<Screen, String> {
    if data.len() < HEADER_LENGTH {
        return Err("truncated XBin header".to_owned());
    }
    if !data.starts_with(b"XBIN\x1a") {
        return Err("invalid XBin signature".to_owned());
    }

    let width = usize::from(u16::from_le_bytes([data[5], data[6]]));
    let height = usize::from(u16::from_le_bytes([data[7], data[8]]));
    let mut glyph_height = usize::from(data[9]);
    let flags = data[10];

    if width == 0 || height == 0 {
        return Err("XBin width and height must be non-zero".to_owned());
    }
    if width > 1000 {
        return Err(format!("invalid XBin width {width}; expected 1..=1000"));
    }
    if let Some(override_width) = width_override
        && override_width != width
    {
        return Err(format!(
            "width override {override_width} does not match the XBin header width {width}"
        ));
    }
    let cell_count = width.checked_mul(height).ok_or_else(canvas_too_large)?;
    if cell_count > MAX_CELLS {
        return Err(canvas_too_large());
    }
    if glyph_height == 0 {
        glyph_height = 16;
    }
    if glyph_height > 32 {
        return Err(format!(
            "invalid XBin font height {glyph_height}; expected 1..=32"
        ));
    }
    if flags & 0xe0 != 0 {
        return Err(format!("unsupported XBin flags 0x{:02x}", flags & 0xe0));
    }

    let mut offset = HEADER_LENGTH;
    let palette = if flags & FLAG_PALETTE != 0 {
        let bytes = take(data, &mut offset, PALETTE_LENGTH, "palette")?;
        let mut palette = [[0_u8; 3]; 16];
        for (color, values) in palette.iter_mut().zip(bytes.chunks_exact(3)) {
            for (component, &value) in color.iter_mut().zip(values) {
                if value > 63 {
                    return Err(format!(
                        "invalid XBin palette component {value}; expected a 6-bit value"
                    ));
                }
                *component = (value << 2) | (value >> 4);
            }
        }
        palette
    } else {
        xbin_default_palette()
    };

    let font = if flags & FLAG_FONT != 0 {
        let glyph_count = if flags & FLAG_FONT_512 != 0 { 512 } else { 256 };
        let length = glyph_height
            .checked_mul(glyph_count)
            .ok_or_else(|| "XBin font size overflow".to_owned())?;
        Some(take(data, &mut offset, length, "font")?.to_vec())
    } else {
        if flags & FLAG_FONT_512 != 0 {
            return Err("XBin 512-character mode requires an embedded font".to_owned());
        }
        glyph_height = 16;
        None
    };

    let encoded = &data[offset..];
    let pairs = if flags & FLAG_COMPRESSED != 0 {
        decompress(encoded, cell_count)?
    } else {
        let expected = cell_count
            .checked_mul(2)
            .ok_or_else(|| "XBin cell data size overflow".to_owned())?;
        if encoded.len() != expected {
            return Err(format!(
                "invalid XBin cell data length {}; expected {expected}",
                encoded.len()
            ));
        }
        encoded
            .chunks_exact(2)
            .map(|pair| (pair[0], pair[1]))
            .collect()
    };

    let non_blink = flags & FLAG_NON_BLINK != 0;
    let font_512 = flags & FLAG_FONT_512 != 0;
    let cells = pairs
        .into_iter()
        .map(|(character, attribute)| {
            let high_font = font_512 && attribute & 0x08 != 0;
            Cell {
                character: u16::from(character) + if high_font { 256 } else { 0 },
                foreground: attribute & 0x0f,
                background: if non_blink {
                    attribute >> 4
                } else {
                    (attribute >> 4) & 0x07
                },
            }
        })
        .collect();

    Ok(Screen {
        width,
        height,
        cells,
        glyph_height,
        font,
        palette: Some(palette),
        utf8_supported: !font_512,
    })
}

fn xbin_default_palette() -> [[u8; 3]; 16] {
    // XBin attributes use VGA hardware color order, while ANSI SGR uses a
    // red-first order. Reorder the same canonical colors for binary cells.
    [
        VGA_PALETTE[0],
        VGA_PALETTE[4],
        VGA_PALETTE[2],
        VGA_PALETTE[6],
        VGA_PALETTE[1],
        VGA_PALETTE[5],
        VGA_PALETTE[3],
        VGA_PALETTE[7],
        VGA_PALETTE[8],
        VGA_PALETTE[12],
        VGA_PALETTE[10],
        VGA_PALETTE[14],
        VGA_PALETTE[9],
        VGA_PALETTE[13],
        VGA_PALETTE[11],
        VGA_PALETTE[15],
    ]
}

fn take<'a>(
    data: &'a [u8],
    offset: &mut usize,
    length: usize,
    field: &str,
) -> Result<&'a [u8], String> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("XBin {field} offset overflow"))?;
    let bytes = data
        .get(*offset..end)
        .ok_or_else(|| format!("truncated XBin {field}"))?;
    *offset = end;
    Ok(bytes)
}

fn decompress(data: &[u8], cell_count: usize) -> Result<Vec<(u8, u8)>, String> {
    let mut cells = Vec::with_capacity(cell_count);
    let mut offset = 0;
    while cells.len() < cell_count {
        let control = byte(data, &mut offset, "compression control")?;
        let count = usize::from(control & 0x3f) + 1;
        if count > cell_count - cells.len() {
            return Err("XBin compressed run exceeds the declared dimensions".to_owned());
        }

        match control & 0xc0 {
            0x00 => {
                for _ in 0..count {
                    let character = byte(data, &mut offset, "character")?;
                    let attribute = byte(data, &mut offset, "attribute")?;
                    cells.push((character, attribute));
                }
            }
            0x40 => {
                let character = byte(data, &mut offset, "repeated character")?;
                for _ in 0..count {
                    let attribute = byte(data, &mut offset, "attribute")?;
                    cells.push((character, attribute));
                }
            }
            0x80 => {
                let attribute = byte(data, &mut offset, "repeated attribute")?;
                for _ in 0..count {
                    let character = byte(data, &mut offset, "character")?;
                    cells.push((character, attribute));
                }
            }
            0xc0 => {
                let character = byte(data, &mut offset, "repeated character")?;
                let attribute = byte(data, &mut offset, "repeated attribute")?;
                cells.extend(std::iter::repeat_n((character, attribute), count));
            }
            _ => unreachable!(),
        }
    }

    if offset != data.len() {
        return Err(format!(
            "XBin compressed stream has {} trailing bytes",
            data.len() - offset
        ));
    }
    Ok(cells)
}

fn byte(data: &[u8], offset: &mut usize, field: &str) -> Result<u8, String> {
    let value = data
        .get(*offset)
        .copied()
        .ok_or_else(|| format!("truncated XBin compressed {field}"))?;
    *offset += 1;
    Ok(value)
}

fn canvas_too_large() -> String {
    format!("XBin canvas exceeds the {MAX_CELLS} cell safety limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_screen;

    fn header(width: u16, height: u16, glyph_height: u8, flags: u8) -> Vec<u8> {
        let mut data = b"XBIN\x1a".to_vec();
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(&[glyph_height, flags]);
        data
    }

    #[test]
    fn decodes_uncompressed_cells() {
        let mut data = header(2, 1, 0, FLAG_NON_BLINK);
        data.extend_from_slice(&[b'A', 0x1e, b'B', 0xf4]);
        let screen = parse(&data, None).unwrap();
        assert_eq!(
            (screen.width, screen.height, screen.glyph_height),
            (2, 1, 16)
        );
        assert_eq!(screen.cells[0].character, u16::from(b'A'));
        assert_eq!(
            (screen.cells[0].foreground, screen.cells[0].background),
            (14, 1)
        );
        assert_eq!(screen.cells[1].background, 15);
        assert_eq!(screen.palette.unwrap()[1], [0, 0, 0xaa]);
    }

    #[test]
    fn decodes_all_compression_modes() {
        let mut data = header(8, 1, 16, FLAG_COMPRESSED | FLAG_NON_BLINK);
        data.extend_from_slice(&[
            0x01, b'A', 1, b'B', 2, // no compression
            0x41, b'C', 3, 4, // repeated character
            0x81, 5, b'D', b'E', // repeated attribute
            0xc1, b'F', 6, // repeated character and attribute
        ]);
        let screen = parse(&data, None).unwrap();
        let characters: Vec<u16> = screen.cells.iter().map(|cell| cell.character).collect();
        assert_eq!(characters, (*b"ABCCDEFF").map(u16::from));
        let attributes: Vec<(u8, u8)> = screen
            .cells
            .iter()
            .map(|cell| (cell.foreground, cell.background))
            .collect();
        assert_eq!(
            attributes,
            [
                (1, 0),
                (2, 0),
                (3, 0),
                (4, 0),
                (5, 0),
                (5, 0),
                (6, 0),
                (6, 0)
            ]
        );
    }

    #[test]
    fn uses_embedded_palette_and_font() {
        let mut data = header(1, 1, 2, FLAG_PALETTE | FLAG_FONT | FLAG_NON_BLINK);
        data.extend((0_u8..48).map(|value| value % 64));
        let mut font = vec![0_u8; 256 * 2];
        font[usize::from(b'A') * 2..usize::from(b'A') * 2 + 2].copy_from_slice(&[0x80, 0x40]);
        data.extend(font);
        data.extend_from_slice(&[b'A', 0x1e]);

        let screen = parse(&data, None).unwrap();
        assert_eq!(screen.glyph_height, 2);
        assert_eq!(screen.palette.unwrap()[0], [0, 4, 8]);
        let png = encode_screen(&screen, 0, 1).unwrap();
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 2);
    }

    #[test]
    fn selects_the_high_half_of_a_512_character_font() {
        let mut data = header(1, 1, 1, FLAG_FONT | FLAG_FONT_512);
        data.extend(vec![0_u8; 512]);
        data.extend_from_slice(&[5, 0x0f]);
        let screen = parse(&data, None).unwrap();
        assert_eq!(screen.cells[0].character, 261);
    }

    #[test]
    fn rejects_truncated_and_overlong_compressed_data() {
        let mut truncated = header(1, 1, 16, FLAG_COMPRESSED);
        truncated.extend_from_slice(&[0x00, b'A']);
        assert!(parse(&truncated, None).unwrap_err().contains("truncated"));

        let mut overlong = header(1, 1, 16, FLAG_COMPRESSED);
        overlong.extend_from_slice(&[0x01, b'A', 7, b'B', 7]);
        assert!(parse(&overlong, None).unwrap_err().contains("exceeds"));
    }
}
