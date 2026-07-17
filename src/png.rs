//! Dependency-free indexed PNG creation.
//!
//! Character screens are rasterized through an 8-pixel-wide bitmap font. Each
//! set glyph bit selects the cell foreground; each clear bit selects its
//! background. SAUCE can request a nine-pixel VGA cell, whose extra column
//! reproduces the hardware's line-graphics extension. RIPscrip already supplies
//! indexed pixels and skips that step.
//! Both paths produce a 4-bit indexed PNG with a 16-color palette.

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
    encode_screen_scaled(screen, first_row, rows, 1)
}

pub fn encode_screen_scaled(
    screen: &Screen,
    first_row: usize,
    rows: usize,
    scale: usize,
) -> Result<Vec<u8>, String> {
    if scale == 0 {
        return Err("PNG scale must be non-zero".to_owned());
    }
    if rows == 0
        || first_row
            .checked_add(rows)
            .is_none_or(|end| end > screen.height)
    {
        return Err("PNG row range is outside the rendered screen".to_owned());
    }
    if let Some(raster) = &screen.raster {
        if first_row != 0 || rows != screen.height {
            return Err("PNG row ranges are not supported for raster art".to_owned());
        }
        return encode_indexed_scaled(
            raster.width,
            raster.height,
            &raster.pixels,
            screen.palette.unwrap_or(VGA_PALETTE),
            scale,
        );
    }
    let width = screen
        .width
        .checked_mul(screen.glyph_width)
        .and_then(|width| width.checked_mul(scale))
        .ok_or("PNG width overflow")?;
    let height = rows
        .checked_mul(screen.glyph_height)
        .and_then(|height| height.checked_mul(scale))
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
            for _ in 0..scale {
                pixels.push(0); // PNG filter: None
                let mut high_nibble = None;
                for cell in
                    &screen.cells[character_row * screen.width..(character_row + 1) * screen.width]
                {
                    // A font is stored glyph-major: `glyph_height` bytes per
                    // character, with one byte holding each eight-pixel row.
                    let glyph = usize::from(cell.character)
                        .checked_mul(screen.glyph_height)
                        .and_then(|offset| offset.checked_add(glyph_row))
                        .ok_or("font glyph index overflow")?;
                    let bits = *glyphs
                        .get(glyph)
                        .ok_or("character references a missing font glyph")?;
                    for pixel in 0..screen.glyph_width {
                        let color = if glyph_pixel(bits, cell.character, pixel) {
                            cell.foreground
                        } else {
                            cell.background
                        } & 0x0f;
                        for _ in 0..scale {
                            push_color(&mut pixels, &mut high_nibble, color);
                        }
                    }
                }
                if let Some(high) = high_nibble {
                    pixels.push(high << 4);
                }
            }
        }
    }

    // PNG is a signature followed by length/type/data/CRC chunks. IHDR declares
    // 4-bit indexed color, PLTE supplies RGB values, and IDAT holds scanlines.
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

pub(crate) fn encode_screen_scaled_fit(
    screen: &Screen,
    first_row: usize,
    rows: usize,
    scale: usize,
    maximum_width: usize,
) -> Result<Vec<u8>, String> {
    encode_screen_scaled_width(screen, first_row, rows, scale, maximum_width, true)
}

pub(crate) fn encode_screen_scaled_crop(
    screen: &Screen,
    first_row: usize,
    rows: usize,
    scale: usize,
    maximum_width: usize,
) -> Result<Vec<u8>, String> {
    encode_screen_scaled_width(screen, first_row, rows, scale, maximum_width, false)
}

fn encode_screen_scaled_width(
    screen: &Screen,
    first_row: usize,
    rows: usize,
    scale: usize,
    maximum_width: usize,
    fit_height: bool,
) -> Result<Vec<u8>, String> {
    if maximum_width == 0 {
        return Err("PNG output width must be non-zero".to_owned());
    }
    let source_width = screen
        .raster
        .as_ref()
        .map_or_else(
            || screen.width.checked_mul(screen.glyph_width),
            |raster| Some(raster.width),
        )
        .ok_or("PNG width overflow")?;
    let requested_width = source_width
        .checked_mul(scale)
        .ok_or("PNG width overflow")?;
    if requested_width <= maximum_width {
        return encode_screen_scaled(screen, first_row, rows, scale);
    }
    if scale == 0 {
        return Err("PNG scale must be non-zero".to_owned());
    }
    if rows == 0
        || first_row
            .checked_add(rows)
            .is_none_or(|end| end > screen.height)
    {
        return Err("PNG row range is outside the rendered screen".to_owned());
    }

    if let Some(raster) = &screen.raster {
        if first_row != 0 || rows != screen.height {
            return Err("PNG row ranges are not supported for raster art".to_owned());
        }
        return encode_indexed_width(
            raster.width,
            raster.height,
            &raster.pixels,
            screen.palette.unwrap_or(VGA_PALETTE),
            scale,
            maximum_width,
            fit_height,
        );
    }

    let requested_height = rows
        .checked_mul(screen.glyph_height)
        .and_then(|height| height.checked_mul(scale))
        .ok_or("PNG height overflow")?;
    let width = maximum_width;
    let height = if fit_height {
        fitted_height(requested_width, requested_height, width)?
    } else {
        requested_height
    };
    let mut pixels = packed_buffer(width, height)?;
    let glyphs: &[u8] = match &screen.font {
        Some(font) => font,
        None => font::glyphs(),
    };

    for y in 0..height {
        pixels.push(0);
        let mut high_nibble = None;
        let source_y = scaled_coordinate(y, requested_height, height) / scale;
        let character_row = first_row + source_y / screen.glyph_height;
        let glyph_row = source_y % screen.glyph_height;
        for x in 0..width {
            let source_x = if fit_height {
                scaled_coordinate(x, requested_width, width)
            } else {
                x
            } / scale;
            let cell = &screen.cells[character_row * screen.width + source_x / screen.glyph_width];
            let glyph = usize::from(cell.character)
                .checked_mul(screen.glyph_height)
                .and_then(|offset| offset.checked_add(glyph_row))
                .ok_or("font glyph index overflow")?;
            let bits = *glyphs
                .get(glyph)
                .ok_or("character references a missing font glyph")?;
            let color = if glyph_pixel(bits, cell.character, source_x % screen.glyph_width) {
                cell.foreground
            } else {
                cell.background
            } & 0x0f;
            push_color(&mut pixels, &mut high_nibble, color);
        }
        if let Some(high) = high_nibble {
            pixels.push(high << 4);
        }
    }

    Ok(indexed_png(
        width,
        height,
        &pixels,
        screen.palette.unwrap_or(VGA_PALETTE),
    ))
}

fn glyph_pixel(bits: u8, character: u16, pixel: usize) -> bool {
    match pixel {
        0..=7 => bits & (0x80 >> pixel) != 0,
        // The VGA's 9th pixel column duplicated column eight only for this
        // line-graphics range. Shade blocks deliberately retain a blank gap.
        8 if (0xc0..=0xdf).contains(&character) => bits & 1 != 0,
        _ => false,
    }
}

fn encode_indexed_width(
    source_width: usize,
    source_height: usize,
    colors: &[u8],
    palette: [[u8; 3]; 16],
    scale: usize,
    maximum_width: usize,
    fit_height: bool,
) -> Result<Vec<u8>, String> {
    if colors.len()
        != source_width
            .checked_mul(source_height)
            .ok_or("PNG pixel count overflow")?
    {
        return Err("raster pixel buffer does not match its dimensions".to_owned());
    }
    let requested_width = source_width
        .checked_mul(scale)
        .ok_or("PNG width overflow")?;
    let requested_height = source_height
        .checked_mul(scale)
        .ok_or("PNG height overflow")?;
    let width = requested_width.min(maximum_width);
    let height = if fit_height {
        fitted_height(requested_width, requested_height, width)?
    } else {
        requested_height
    };
    let mut pixels = packed_buffer(width, height)?;
    for y in 0..height {
        pixels.push(0);
        let mut high_nibble = None;
        let source_y = scaled_coordinate(y, requested_height, height) / scale;
        for x in 0..width {
            let source_x = if fit_height {
                scaled_coordinate(x, requested_width, width)
            } else {
                x
            } / scale;
            push_color(
                &mut pixels,
                &mut high_nibble,
                colors[source_y * source_width + source_x] & 0x0f,
            );
        }
        if let Some(high) = high_nibble {
            pixels.push(high << 4);
        }
    }
    Ok(indexed_png(width, height, &pixels, palette))
}

fn fitted_height(width: usize, height: usize, fitted_width: usize) -> Result<usize, String> {
    height
        .checked_mul(fitted_width)
        .map(|area| area.div_ceil(width).max(1))
        .ok_or_else(|| "PNG fitted height overflow".to_owned())
}

fn scaled_coordinate(position: usize, source_length: usize, target_length: usize) -> usize {
    ((position as u128 * source_length as u128) / target_length as u128) as usize
}

fn packed_buffer(width: usize, height: usize) -> Result<Vec<u8>, String> {
    let pixel_count = width
        .checked_mul(height)
        .ok_or("PNG pixel count overflow")?;
    if pixel_count > MAX_PNG_PIXELS {
        return Err(format!(
            "PNG output exceeds the {MAX_PNG_PIXELS} pixel safety limit"
        ));
    }
    let capacity = (1 + width.div_ceil(2))
        .checked_mul(height)
        .ok_or("PNG buffer size overflow")?;
    Ok(Vec::with_capacity(capacity))
}

fn indexed_png(width: usize, height: usize, pixels: &[u8], palette: [[u8; 3]; 16]) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[4, 3, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(
        &mut png,
        b"PLTE",
        &palette.into_iter().flatten().collect::<Vec<_>>(),
    );
    chunk(&mut png, b"IDAT", &zlib_store(pixels));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn encode_indexed_scaled(
    width: usize,
    height: usize,
    colors: &[u8],
    palette: [[u8; 3]; 16],
    scale: usize,
) -> Result<Vec<u8>, String> {
    let source_pixel_count = width
        .checked_mul(height)
        .ok_or("PNG pixel count overflow")?;
    if colors.len() != source_pixel_count {
        return Err("raster pixel buffer does not match its dimensions".to_owned());
    }
    let width = width.checked_mul(scale).ok_or("PNG width overflow")?;
    let height = height.checked_mul(scale).ok_or("PNG height overflow")?;
    let pixel_count = width
        .checked_mul(height)
        .ok_or("PNG pixel count overflow")?;
    if pixel_count > MAX_PNG_PIXELS {
        return Err(format!(
            "PNG output exceeds the {MAX_PNG_PIXELS} pixel safety limit"
        ));
    }

    let capacity = (1 + width.div_ceil(2))
        .checked_mul(height)
        .ok_or("PNG buffer size overflow")?;
    let mut pixels = Vec::with_capacity(capacity);
    for row in colors.chunks_exact(width / scale) {
        for _ in 0..scale {
            pixels.push(0);
            let mut high_nibble = None;
            for &color in row {
                for _ in 0..scale {
                    push_color(&mut pixels, &mut high_nibble, color & 0x0f);
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
    ihdr.extend_from_slice(&[4, 3, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(
        &mut png,
        b"PLTE",
        &palette.into_iter().flatten().collect::<Vec<_>>(),
    );
    chunk(&mut png, b"IDAT", &zlib_store(&pixels));
    chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

fn push_color(pixels: &mut Vec<u8>, high_nibble: &mut Option<u8>, color: u8) {
    // At four bits per pixel, two palette indices share each scanline byte.
    if let Some(high) = high_nibble.take() {
        pixels.push((high << 4) | color);
    } else {
        *high_nibble = Some(color);
    }
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    // IDAT requires a zlib stream. DEFLATE "stored" blocks add framing and
    // checksums without compression, keeping this encoder dependency-free.
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
    // The PNG CRC covers the four-byte chunk type and its data, but not length.
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
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let png = encode_screen(&screen, 0, 1).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[24..29], &[4, 3, 0, 0, 0]);
        assert!(png.windows(4).any(|window| window == b"IEND"));
    }

    #[test]
    fn scales_png_dimensions_by_two() {
        let screen = Screen {
            width: 1,
            height: 1,
            cells: vec![Cell::default()],
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let png = encode_screen_scaled(&screen, 0, 1, 2).unwrap();
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 16);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 32);
    }

    #[test]
    fn nine_pixel_vga_spacing_widens_line_graphics() {
        let screen = Screen {
            width: 1,
            height: 1,
            cells: vec![Cell {
                character: 0xc4,
                foreground: 7,
                background: 0,
            }],
            glyph_width: 9,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let png = encode_screen(&screen, 0, 1).unwrap();

        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 9);
        assert!(glyph_pixel(0x01, 0xc4, 8));
        assert!(!glyph_pixel(0x01, 0xb0, 8));
    }

    #[test]
    fn fits_extra_wide_pngs_before_kitty_transport() {
        let screen = Screen {
            width: 4,
            height: 1,
            cells: vec![Cell::default(); 4],
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let png = encode_screen_scaled_fit(&screen, 0, 1, 1, 8).unwrap();
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 8);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 4);

        let full = encode_screen_scaled_fit(&screen, 0, 1, 1, 64).unwrap();
        assert_eq!(u32::from_be_bytes(full[16..20].try_into().unwrap()), 32);
        assert_eq!(u32::from_be_bytes(full[20..24].try_into().unwrap()), 16);
    }

    #[test]
    fn crops_extra_wide_pngs_without_scaling_their_height() {
        let screen = Screen {
            width: 4,
            height: 1,
            cells: vec![Cell::default(); 4],
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let png = encode_screen_scaled_crop(&screen, 0, 1, 1, 8).unwrap();
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 8);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 16);
    }
}
