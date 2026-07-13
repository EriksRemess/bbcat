use crate::{Screen, font};

pub const VGA_PALETTE: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xaa, 0x00, 0x00],
    [0x00, 0xaa, 0x00],
    [0xaa, 0x55, 0x00],
    [0x00, 0x00, 0xaa],
    [0xaa, 0x00, 0xaa],
    [0x00, 0xaa, 0xaa],
    [0xaa, 0xaa, 0xaa],
    [0x55, 0x55, 0x55],
    [0xff, 0x55, 0x55],
    [0x55, 0xff, 0x55],
    [0xff, 0xff, 0x55],
    [0x55, 0x55, 0xff],
    [0xff, 0x55, 0xff],
    [0x55, 0xff, 0xff],
    [0xff, 0xff, 0xff],
];

const MAX_PNG_PIXELS: usize = 100_000_000;

pub fn encode_screen(screen: &Screen, first_row: usize, rows: usize) -> Result<Vec<u8>, String> {
    if rows == 0
        || first_row
            .checked_add(rows)
            .is_none_or(|end| end > screen.height)
    {
        return Err("PNG row range is outside the rendered screen".to_owned());
    }
    let width = screen.width.checked_mul(8).ok_or("PNG width overflow")?;
    let height = rows
        .checked_mul(screen.glyph_height)
        .ok_or("PNG height overflow")?;
    let pixel_count = width
        .checked_mul(height)
        .ok_or("PNG pixel count overflow")?;
    if pixel_count > MAX_PNG_PIXELS {
        return Err(format!(
            "PNG output exceeds the {MAX_PNG_PIXELS} pixel safety limit"
        ));
    }
    let scanline_bytes = 1 + width.div_ceil(2);
    let capacity = scanline_bytes
        .checked_mul(height)
        .ok_or("PNG buffer size overflow")?;
    let mut pixels = Vec::with_capacity(capacity);
    let glyphs: &[u8] = match &screen.font {
        Some(font) => font,
        None => font::glyphs(),
    };

    for character_row in first_row..first_row + rows {
        for glyph_row in 0..screen.glyph_height {
            pixels.push(0); // PNG filter: None
            let mut high_nibble = None;
            for cell in
                &screen.cells[character_row * screen.width..(character_row + 1) * screen.width]
            {
                let glyph = usize::from(cell.character)
                    .checked_mul(screen.glyph_height)
                    .and_then(|offset| offset.checked_add(glyph_row))
                    .ok_or("font glyph index overflow")?;
                let bits = *glyphs
                    .get(glyph)
                    .ok_or("character references a missing font glyph")?;
                for bit in 0..8 {
                    let color = if bits & (0x80 >> bit) != 0 {
                        cell.foreground
                    } else {
                        cell.background
                    } & 0x0f;
                    if let Some(high) = high_nibble.take() {
                        pixels.push((high << 4) | color);
                    } else {
                        high_nibble = Some(color);
                    }
                }
            }
            if let Some(high) = high_nibble {
                pixels.push(high << 4);
            }
        }
    }

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[4, 3, 0, 0, 0]); // 4-bit indexed color
    chunk(&mut png, b"IHDR", &ihdr);

    let palette: Vec<u8> = screen
        .palette
        .unwrap_or(VGA_PALETTE)
        .into_iter()
        .flatten()
        .collect();
    chunk(&mut png, b"PLTE", &palette);
    chunk(&mut png, b"IDAT", &zlib_store(&pixels));
    chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + data.len() / 65_535 * 5 + 11);
    output.extend_from_slice(&[0x78, 0x01]);
    if data.is_empty() {
        output.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    } else {
        for (index, block) in data.chunks(65_535).enumerate() {
            let final_block = index + 1 == data.len().div_ceil(65_535);
            output.push(u8::from(final_block));
            let length = block.len() as u16;
            output.extend_from_slice(&length.to_le_bytes());
            output.extend_from_slice(&(!length).to_le_bytes());
            output.extend_from_slice(block);
        }
    }
    output.extend_from_slice(&adler32(data).to_be_bytes());
    output
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    output.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cell;

    #[test]
    fn writes_indexed_png() {
        let screen = Screen {
            width: 1,
            height: 1,
            cells: vec![Cell::default()],
            glyph_height: 16,
            font: None,
            palette: None,
        };
        let png = encode_screen(&screen, 0, 1).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[24..29], &[4, 3, 0, 0, 0]);
        assert!(png.windows(4).any(|window| window == b"IEND"));
    }
}
