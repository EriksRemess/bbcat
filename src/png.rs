//! Dependency-free PNG creation.
//!
//! Character screens are rasterized through an 8-pixel-wide bitmap font. Each
//! set glyph bit selects the cell foreground; each clear bit selects its
//! background. SAUCE can request a nine-pixel VGA cell, whose extra column
//! reproduces the hardware's line-graphics extension. RIPscrip already supplies
//! indexed pixels and skips that step.
//! Classic artwork produces a 4-bit indexed PNG. ANSI and DDW screens that use
//! xterm-256 colors produce an 8-bit indexed PNG with the complete palette.
//! TundraDraw's per-cell 24-bit colors use a lossless adaptive palette when the
//! image contains at most 256 colors, and an RGB PNG otherwise.

use std::collections::HashMap;

use crate::{Screen, font};

/// The default 16-color VGA palette as RGB triples.
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

/// The xterm 256-color palette, using VGA-compatible values for its first 16
/// entries and the standard color cube and grayscale ramp for the remainder.
pub const XTERM_256_PALETTE: [[u8; 3]; 256] = xterm_256_palette();

const fn xterm_256_palette() -> [[u8; 3]; 256] {
    let mut palette = [[0_u8; 3]; 256];
    let mut index = 0;
    while index < 16 {
        palette[index] = VGA_PALETTE[index];
        index += 1;
    }
    while index < 232 {
        let value = index - 16;
        palette[index] = [
            cube_level(value / 36),
            cube_level((value / 6) % 6),
            cube_level(value % 6),
        ];
        index += 1;
    }
    while index < 256 {
        let level = 8 + ((index - 232) as u8) * 10;
        palette[index] = [level, level, level];
        index += 1;
    }
    palette
}

const fn cube_level(value: usize) -> u8 {
    match value {
        0 => 0,
        1 => 95,
        2 => 135,
        3 => 175,
        4 => 215,
        _ => 255,
    }
}

const MAX_PNG_PIXELS: usize = 100_000_000;
const MAX_PNG_BUFFER_BYTES: usize = 100_000_000;

/// Encodes a contiguous range of character rows as a PNG.
pub fn encode_screen(screen: &Screen, first_row: usize, rows: usize) -> Result<Vec<u8>, String> {
    encode_screen_scaled(screen, first_row, rows, 1)
}

/// Encodes character rows as a PNG at an integer scale.
pub fn encode_screen_scaled(
    screen: &Screen,
    first_row: usize,
    rows: usize,
    scale: usize,
) -> Result<Vec<u8>, String> {
    encode_screen_scaled_with_depth(screen, first_row, rows, scale, false)
}

/// Encodes the complete screen within a pixel bounding box while preserving
/// its aspect ratio. Screens already inside the bounds are not enlarged.
pub fn encode_screen_fit(
    screen: &Screen,
    maximum_width: usize,
    maximum_height: usize,
) -> Result<Vec<u8>, String> {
    screen.validate()?;
    if maximum_width == 0 || maximum_height == 0 {
        return Err("PNG fit dimensions must be non-zero".to_owned());
    }
    let (source_width, source_height) =
        screen.pixel_dimensions().ok_or("PNG dimensions overflow")?;
    let height_limited_width =
        usize::try_from(source_width as u128 * maximum_height as u128 / source_height as u128)
            .unwrap_or(usize::MAX);
    let fitted_width = source_width.min(maximum_width).min(height_limited_width);
    if fitted_width == 0 {
        return Err(format!(
            "PNG cannot preserve its aspect ratio within {maximum_width}x{maximum_height} pixels"
        ));
    }
    encode_screen_scaled_fit(screen, 0, screen.height, 1, fitted_width)
}

pub(crate) fn encode_screen_scaled_with_depth(
    screen: &Screen,
    first_row: usize,
    rows: usize,
    scale: usize,
    force_8_bit: bool,
) -> Result<Vec<u8>, String> {
    screen.validate()?;
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
    let true_color = screen.true_colors.is_some();
    let bit_depth = if force_8_bit || uses_xterm_256(screen) {
        8
    } else {
        4
    };
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
    let capacity = if true_color {
        rgb_capacity(width, height)?
    } else {
        (1 + row_bytes(width, bit_depth))
            .checked_mul(height)
            .ok_or("PNG buffer size overflow")?
    };
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
                for (column, cell) in screen.cells
                    [character_row * screen.width..(character_row + 1) * screen.width]
                    .iter()
                    .enumerate()
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
                    let cell_index = character_row * screen.width + column;
                    for pixel in 0..screen.glyph_width {
                        let foreground = glyph_pixel(bits, cell.character, pixel);
                        let indexed_color = if foreground {
                            cell.foreground
                        } else {
                            cell.background
                        };
                        let rgb_color = true_color.then(|| {
                            let colors = screen.cell_colors_at(cell_index, cell);
                            if foreground { colors.0 } else { colors.1 }
                        });
                        for _ in 0..scale {
                            if let Some(color) = rgb_color {
                                pixels.extend_from_slice(&color);
                            } else {
                                push_color(&mut pixels, &mut high_nibble, indexed_color, bit_depth);
                            }
                        }
                    }
                }
                if let Some(high) = high_nibble {
                    pixels.push(high << 4);
                }
            }
        }
    }

    if true_color {
        Ok(adaptive_color_png(width, height, pixels))
    } else {
        Ok(indexed_png(
            width,
            height,
            &pixels,
            &palette_bytes(screen, bit_depth),
            bit_depth,
        ))
    }
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
    let true_color = screen.true_colors.is_some();
    let bit_depth = if uses_xterm_256(screen) { 8 } else { 4 };
    let mut pixels = if true_color {
        rgb_buffer(width, height)?
    } else {
        packed_buffer(width, height, bit_depth)?
    };
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
            let cell_index = character_row * screen.width + source_x / screen.glyph_width;
            let cell = &screen.cells[cell_index];
            let glyph = usize::from(cell.character)
                .checked_mul(screen.glyph_height)
                .and_then(|offset| offset.checked_add(glyph_row))
                .ok_or("font glyph index overflow")?;
            let bits = *glyphs
                .get(glyph)
                .ok_or("character references a missing font glyph")?;
            let foreground = glyph_pixel(bits, cell.character, source_x % screen.glyph_width);
            let color = if foreground {
                cell.foreground
            } else {
                cell.background
            };
            if true_color {
                let colors = screen.cell_colors_at(cell_index, cell);
                pixels.extend_from_slice(if foreground { &colors.0 } else { &colors.1 });
            } else {
                push_color(&mut pixels, &mut high_nibble, color, bit_depth);
            }
        }
        if let Some(high) = high_nibble {
            pixels.push(high << 4);
        }
    }

    if true_color {
        Ok(adaptive_color_png(width, height, pixels))
    } else {
        Ok(indexed_png(
            width,
            height,
            &pixels,
            &palette_bytes(screen, bit_depth),
            bit_depth,
        ))
    }
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
    let mut pixels = packed_buffer(width, height, 4)?;
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
                4,
            );
        }
        if let Some(high) = high_nibble {
            pixels.push(high << 4);
        }
    }
    Ok(indexed_png(
        width,
        height,
        &pixels,
        &palette.into_iter().flatten().collect::<Vec<_>>(),
        4,
    ))
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

fn packed_buffer(width: usize, height: usize, bit_depth: u8) -> Result<Vec<u8>, String> {
    let pixel_count = width
        .checked_mul(height)
        .ok_or("PNG pixel count overflow")?;
    if pixel_count > MAX_PNG_PIXELS {
        return Err(format!(
            "PNG output exceeds the {MAX_PNG_PIXELS} pixel safety limit"
        ));
    }
    let capacity = (1 + row_bytes(width, bit_depth))
        .checked_mul(height)
        .ok_or("PNG buffer size overflow")?;
    Ok(Vec::with_capacity(capacity))
}

fn rgb_buffer(width: usize, height: usize) -> Result<Vec<u8>, String> {
    let pixel_count = width
        .checked_mul(height)
        .ok_or("PNG pixel count overflow")?;
    if pixel_count > MAX_PNG_PIXELS {
        return Err(format!(
            "PNG output exceeds the {MAX_PNG_PIXELS} pixel safety limit"
        ));
    }
    Ok(Vec::with_capacity(rgb_capacity(width, height)?))
}

fn rgb_capacity(width: usize, height: usize) -> Result<usize, String> {
    let capacity = width
        .checked_mul(3)
        .and_then(|row| row.checked_add(1))
        .and_then(|row| row.checked_mul(height))
        .ok_or("PNG buffer size overflow")?;
    if capacity > MAX_PNG_BUFFER_BYTES {
        return Err(format!(
            "RGB PNG scanline buffer exceeds the {MAX_PNG_BUFFER_BYTES}-byte safety limit"
        ));
    }
    Ok(capacity)
}

fn indexed_png(
    width: usize,
    height: usize,
    pixels: &[u8],
    palette: &[u8],
    bit_depth: u8,
) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[bit_depth, 3, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"PLTE", palette);
    compressed_chunk(&mut png, b"IDAT", pixels);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn adaptive_color_png(width: usize, height: usize, mut pixels: Vec<u8>) -> Vec<u8> {
    if let Some((mut indexes, palette)) = rgb_to_indexed(width, height, &pixels) {
        drop(pixels);
        if palette.len() <= 16 * 3 {
            let mut packed = pack_four_bit_indexes(width, height, &indexes);
            drop(indexes);
            apply_sub_filter(&mut packed, width.div_ceil(2), 1);
            indexed_png(width, height, &packed, &palette, 4)
        } else {
            apply_sub_filter(&mut indexes, width, 1);
            indexed_png(width, height, &indexes, &palette, 8)
        }
    } else {
        apply_sub_filter(&mut pixels, width, 3);
        rgb_png(width, height, &pixels)
    }
}

fn pack_four_bit_indexes(width: usize, height: usize, indexes: &[u8]) -> Vec<u8> {
    let mut packed = Vec::with_capacity((1 + width.div_ceil(2)) * height);
    for row in indexes.chunks_exact(width + 1) {
        packed.push(0);
        for pair in row[1..].chunks(2) {
            packed.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
        }
    }
    packed
}

fn apply_sub_filter(pixels: &mut [u8], width: usize, bytes_per_pixel: usize) {
    let row_bytes = 1 + width * bytes_per_pixel;
    for row in pixels.chunks_exact_mut(row_bytes) {
        row[0] = 1;
        for index in (bytes_per_pixel..width * bytes_per_pixel).rev() {
            row[index + 1] = row[index + 1].wrapping_sub(row[index + 1 - bytes_per_pixel]);
        }
    }
}

fn rgb_to_indexed(width: usize, height: usize, pixels: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let row_bytes = width.checked_mul(3)?.checked_add(1)?;
    if pixels.len() != row_bytes.checked_mul(height)? {
        return None;
    }

    let mut colors = HashMap::<[u8; 3], u8>::new();
    let mut palette = Vec::new();
    let mut indexes = Vec::with_capacity((width + 1).checked_mul(height)?);
    for row in pixels.chunks_exact(row_bytes) {
        if row[0] != 0 {
            return None;
        }
        indexes.push(0);
        for rgb in row[1..].as_chunks::<3>().0 {
            let color = [rgb[0], rgb[1], rgb[2]];
            let index = if let Some(&index) = colors.get(&color) {
                index
            } else {
                let index = u8::try_from(colors.len()).ok()?;
                colors.insert(color, index);
                palette.extend_from_slice(&color);
                index
            };
            indexes.push(index);
        }
    }
    Some((indexes, palette))
}

fn rgb_png(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    compressed_chunk(&mut png, b"IDAT", pixels);
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
                    push_color(&mut pixels, &mut high_nibble, color & 0x0f, 4);
                }
            }
            if let Some(high) = high_nibble {
                pixels.push(high << 4);
            }
        }
    }

    Ok(indexed_png(
        width,
        height,
        &pixels,
        &palette.into_iter().flatten().collect::<Vec<_>>(),
        4,
    ))
}

fn push_color(pixels: &mut Vec<u8>, high_nibble: &mut Option<u8>, color: u8, bit_depth: u8) {
    if bit_depth == 8 {
        pixels.push(color);
        return;
    }
    // At four bits per pixel, two palette indices share each scanline byte.
    if let Some(high) = high_nibble.take() {
        pixels.push((high << 4) | color);
    } else {
        *high_nibble = Some(color);
    }
}

fn row_bytes(width: usize, bit_depth: u8) -> usize {
    if bit_depth == 8 {
        width
    } else {
        width.div_ceil(2)
    }
}

pub(crate) fn uses_xterm_256(screen: &Screen) -> bool {
    screen
        .cells
        .iter()
        .any(|cell| cell.foreground >= 16 || cell.background >= 16)
}

fn palette_bytes(screen: &Screen, bit_depth: u8) -> Vec<u8> {
    if bit_depth == 8 {
        screen.palette_256().into_iter().flatten().collect()
    } else {
        screen.palette().into_iter().flatten().collect()
    }
}

fn compressed_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let length_offset = output.len();
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(kind);
    let data_offset = output.len();
    zlib_compress(output, data);
    let length = output.len() - data_offset;
    output[length_offset..length_offset + 4].copy_from_slice(&(length as u32).to_be_bytes());
    let crc = crc32_parts(kind, &output[data_offset..]);
    output.extend_from_slice(&crc.to_be_bytes());
}

fn zlib_compress(output: &mut Vec<u8>, data: &[u8]) {
    // Keep tiny images byte-for-byte simple. Larger images use a dependency-free
    // fixed-Huffman DEFLATE stream with greedy LZ77 matching.
    output.extend_from_slice(&[0x78, 0x01]);
    if data.len() < 1024 {
        deflate_store(output, data);
    } else {
        deflate_fixed(output, data);
    }
    output.extend_from_slice(&adler32(data).to_be_bytes());
}

fn deflate_store(output: &mut Vec<u8>, data: &[u8]) {
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
}

fn deflate_fixed(output: &mut Vec<u8>, data: &[u8]) {
    let mut writer = BitWriter::new(output);
    writer.write_bits(1, 1); // final block
    writer.write_bits(1, 2); // fixed Huffman block

    let mut previous = vec![[usize::MAX; 4]; 1 << 16];
    let mut position = 0;
    while position < data.len() {
        let mut match_length = 0;
        let mut match_distance = 0;
        if position + 2 < data.len() {
            let hash = hash3(&data[position..]);
            let candidates = previous[hash];
            remember_position(&mut previous[hash], position);
            for candidate in candidates {
                if candidate == usize::MAX || position - candidate > 32_768 {
                    continue;
                }
                let maximum = 258.min(data.len() - position);
                let mut candidate_length = 0;
                while candidate_length < maximum
                    && data[candidate + candidate_length] == data[position + candidate_length]
                {
                    candidate_length += 1;
                }
                if candidate_length >= 3 && candidate_length > match_length {
                    match_length = candidate_length;
                    match_distance = position - candidate;
                    if match_length == maximum {
                        break;
                    }
                }
            }
        }

        if match_length >= 3 {
            write_length_distance(&mut writer, match_length, match_distance);
            let end = position + match_length;
            position += 1;
            while position < end {
                if position + 2 < data.len() {
                    let hash = hash3(&data[position..]);
                    remember_position(&mut previous[hash], position);
                }
                position += 1;
            }
        } else {
            write_fixed_symbol(&mut writer, u16::from(data[position]));
            position += 1;
        }
    }
    write_fixed_symbol(&mut writer, 256);
    writer.finish();
}

fn remember_position(previous: &mut [usize; 4], position: usize) {
    previous.copy_within(..3, 1);
    previous[0] = position;
}

fn hash3(data: &[u8]) -> usize {
    ((usize::from(data[0]) * 251 + usize::from(data[1])) * 251 + usize::from(data[2])) & 0xffff
}

const LENGTH_BASES: [usize; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRAS: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASES: [usize; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12_289, 16_385, 24_577,
];
const DISTANCE_EXTRAS: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn write_length_distance(writer: &mut BitWriter<'_>, length: usize, distance: usize) {
    let length_index = LENGTH_BASES
        .iter()
        .zip(LENGTH_EXTRAS)
        .position(|(&base, extra)| length <= base + ((1_usize << extra) - 1))
        .expect("DEFLATE match length is bounded");
    write_fixed_symbol(writer, 257 + length_index as u16);
    writer.write_bits(
        (length - LENGTH_BASES[length_index]) as u32,
        LENGTH_EXTRAS[length_index],
    );

    let distance_index = DISTANCE_BASES
        .iter()
        .zip(DISTANCE_EXTRAS)
        .position(|(&base, extra)| distance <= base + ((1_usize << extra) - 1))
        .expect("DEFLATE match distance is bounded");
    writer.write_bits(reverse_bits(distance_index as u16, 5) as u32, 5);
    writer.write_bits(
        (distance - DISTANCE_BASES[distance_index]) as u32,
        DISTANCE_EXTRAS[distance_index],
    );
}

fn write_fixed_symbol(writer: &mut BitWriter<'_>, symbol: u16) {
    let (code, bits) = match symbol {
        0..=143 => (0x30 + symbol, 8),
        144..=255 => (0x190 + symbol - 144, 9),
        256..=279 => (symbol - 256, 7),
        280..=287 => (0xc0 + symbol - 280, 8),
        _ => unreachable!("invalid fixed Huffman symbol"),
    };
    writer.write_bits(reverse_bits(code, bits) as u32, bits);
}

fn reverse_bits(value: u16, bits: u8) -> u16 {
    value.reverse_bits() >> (u16::BITS as u8 - bits)
}

struct BitWriter<'a> {
    output: &'a mut Vec<u8>,
    pending: u64,
    bits: u8,
}

impl<'a> BitWriter<'a> {
    fn new(output: &'a mut Vec<u8>) -> Self {
        Self {
            output,
            pending: 0,
            bits: 0,
        }
    }

    fn write_bits(&mut self, value: u32, bits: u8) {
        self.pending |= u64::from(value) << self.bits;
        self.bits += bits;
        while self.bits >= 8 {
            self.output.push(self.pending as u8);
            self.pending >>= 8;
            self.bits -= 8;
        }
    }

    fn finish(self) {
        if self.bits > 0 {
            self.output.push(self.pending as u8);
        }
    }
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
    output.extend_from_slice(&crc32_parts(kind, data).to_be_bytes());
}

fn crc32_parts(first: &[u8], second: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in first.iter().chain(second) {
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
            true_colors: None,
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
    fn writes_xterm_256_indexes_as_eight_bit_png_pixels() {
        let screen = Screen {
            width: 1,
            height: 1,
            cells: vec![Cell {
                character: u16::from(b'X'),
                foreground: 196,
                background: 235,
            }],
            glyph_width: 8,
            glyph_height: 16,
            font: Some(vec![0xff; 256 * 16]),
            palette: None,
            true_colors: None,
            utf8_supported: true,
            raster: None,
        };
        let png = encode_screen(&screen, 0, 1).unwrap();

        assert_eq!(&png[24..29], &[8, 3, 0, 0, 0]);
        let palette = chunk_payload(&png, b"PLTE");
        assert_eq!(palette.len(), 256 * 3);
        assert_eq!(&palette[196 * 3..196 * 3 + 3], &[255, 0, 0]);

        let idat = chunk_payload(&png, b"IDAT");
        let scanlines = &idat[7..idat.len() - 4];
        assert_eq!(scanlines[0], 0);
        assert_eq!(&scanlines[1..9], &[196; 8]);
    }

    fn chunk_payload<'a>(png: &'a [u8], expected: &[u8; 4]) -> &'a [u8] {
        let mut offset = 8;
        loop {
            let length = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
            if &png[offset + 4..offset + 8] == expected {
                return &png[offset + 8..offset + 8 + length];
            }
            offset += 12 + length;
        }
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
            true_colors: None,
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
            true_colors: None,
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
            true_colors: None,
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
            true_colors: None,
            utf8_supported: true,
            raster: None,
        };
        let png = encode_screen_scaled_crop(&screen, 0, 1, 1, 8).unwrap();
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 8);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 16);
    }

    #[test]
    fn rejects_oversized_rgb_scanline_buffers_before_allocating() {
        let error = rgb_capacity(MAX_PNG_BUFFER_BYTES / 3 + 1, 1).unwrap_err();
        assert!(error.contains("byte safety limit"));
    }

    #[test]
    fn uses_rgb_when_true_color_art_exceeds_256_colors() {
        let mut colors = Vec::new();
        for index in 0..257_u16 {
            colors.push(([index as u8, (index >> 8) as u8, 0], [0, 0, 0]));
        }
        let screen = Screen {
            width: colors.len(),
            height: 1,
            cells: vec![
                Cell {
                    character: 0,
                    foreground: 0,
                    background: 0,
                };
                colors.len()
            ],
            glyph_width: 1,
            glyph_height: 1,
            font: Some(vec![0xff; 256]),
            palette: None,
            true_colors: Some(colors),
            utf8_supported: true,
            raster: None,
        };

        let png = encode_screen(&screen, 0, 1).unwrap();
        assert_eq!(&png[24..26], &[8, 2]);
    }

    #[test]
    fn compresses_large_repetitive_scanlines() {
        let data = vec![0; 10_000];
        let mut compressed = Vec::new();
        zlib_compress(&mut compressed, &data);
        assert!(compressed.len() < 100);
    }
}
