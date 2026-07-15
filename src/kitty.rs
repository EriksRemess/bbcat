//! Kitty graphics protocol output.
//!
//! Kitty does not receive the character grid directly. This module first asks
//! the PNG backend for an image, base64-encodes that PNG, and wraps it in Kitty
//! APC escape sequences. Large artwork is split both vertically (separate PNGs)
//! and into protocol-sized transport chunks.

use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use crate::{Screen, png};

const INPUT_CHUNK: usize = 3072;

#[derive(Clone, Copy)]
enum WidthMode {
    /// Preserve all pixels and their requested scale.
    Full,
    /// Keep the height but discard pixels past the terminal's right edge.
    Crop(usize),
    /// Resize both axes so the complete image preserves its aspect ratio.
    Fit(usize),
}

pub fn write_screen<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, chunk_lines, None, 1, WidthMode::Full)
}

pub fn write_screen_cropped<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(
        output,
        screen,
        chunk_lines,
        None,
        1,
        WidthMode::Crop(columns),
    )
}

pub fn write_screen_fit<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(
        output,
        screen,
        chunk_lines,
        None,
        1,
        WidthMode::Fit(columns),
    )
}

pub fn write_screen_scaled<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    scale: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, chunk_lines, None, scale, WidthMode::Full)
}

pub fn write_screen_scaled_cropped<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    scale: usize,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(
        output,
        screen,
        chunk_lines,
        None,
        scale,
        WidthMode::Crop(columns),
    )
}

pub fn write_screen_scaled_fit<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    scale: usize,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(
        output,
        screen,
        chunk_lines,
        None,
        scale,
        WidthMode::Fit(columns),
    )
}

pub fn write_screen_slow<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), 1, WidthMode::Full)
}

pub fn write_screen_slow_cropped<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), 1, WidthMode::Crop(columns))
}

pub fn write_screen_slow_fit<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), 1, WidthMode::Fit(columns))
}

pub fn write_screen_slow_scaled<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    scale: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), scale, WidthMode::Full)
}

pub fn write_screen_slow_scaled_cropped<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    scale: usize,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(
        output,
        screen,
        1,
        Some(delay),
        scale,
        WidthMode::Crop(columns),
    )
}

pub fn write_screen_slow_scaled_fit<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    scale: usize,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(
        output,
        screen,
        1,
        Some(delay),
        scale,
        WidthMode::Fit(columns),
    )
}

fn write_screen_inner<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    delay: Option<Duration>,
    scale: usize,
    width_mode: WidthMode,
) -> io::Result<()> {
    if scale == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output scale must be non-zero",
        ));
    }
    if matches!(width_mode, WidthMode::Crop(0) | WidthMode::Fit(0)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Kitty placement width must be non-zero",
        ));
    }
    if delay.is_some() && screen.raster.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "slow mode is not supported for RIPscrip graphics",
        ));
    }
    if let WidthMode::Fit(columns) = width_mode {
        let rows = if delay.is_some() { 1 } else { screen.height };
        validate_fit_height(screen, scale, columns, rows)?;
    }
    // RIPscrip is already one indivisible raster, unlike character rows that can
    // be rendered as several shorter images to avoid tall terminal placements.
    if screen.raster.is_some() {
        let image = encode_image(screen, 0, screen.height, scale, width_mode)?;
        let columns = output_columns(screen.width, scale, width_mode)?;
        let placement_rows = image_rows(&image)?;
        write_image(output, &image, columns, placement_rows)?;
        advance_rows(output, placement_rows)?;
        return output.flush();
    }
    // Fit needs the whole image to calculate one aspect ratio. Other modes can
    // limit each PNG to a manageable number of character rows.
    let chunk_lines = if matches!(width_mode, WidthMode::Fit(_)) {
        screen.height
    } else {
        chunk_lines.max(1)
    };
    for first_row in (0..screen.height).step_by(chunk_lines) {
        let rows = chunk_lines.min(screen.height - first_row);
        let image = encode_image(screen, first_row, rows, scale, width_mode)?;
        let columns = output_columns(screen.width, scale, width_mode)?;
        let placement_rows = image_rows(&image)?;
        write_image(output, &image, columns, placement_rows)?;
        advance_rows(output, placement_rows)?;
        if first_row + rows < screen.height
            && let Some(delay) = delay
        {
            output.flush()?;
            thread::sleep(delay);
        }
    }
    output.flush()
}

fn scaled(value: usize, scale: usize) -> io::Result<usize> {
    value
        .checked_mul(scale)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output dimensions overflow"))
}

fn validate_fit_height(
    screen: &Screen,
    scale: usize,
    columns: usize,
    rows: usize,
) -> io::Result<()> {
    let (source_width, source_height) = if let Some(raster) = &screen.raster {
        (raster.width, raster.height)
    } else {
        (
            screen.width.checked_mul(8).ok_or_else(dimension_error)?,
            rows.checked_mul(screen.glyph_height)
                .ok_or_else(dimension_error)?,
        )
    };
    let requested_width = source_width
        .checked_mul(scale)
        .ok_or_else(dimension_error)?;
    let requested_height = source_height
        .checked_mul(scale)
        .ok_or_else(dimension_error)?;
    let maximum_width = columns.checked_mul(8).ok_or_else(dimension_error)?;
    if requested_width <= maximum_width {
        return Ok(());
    }

    let fitted_height = div_ceil(
        requested_height as u128 * maximum_width as u128,
        requested_width as u128,
    );
    if fitted_height >= 16 {
        return Ok(());
    }
    let minimum_pixels = div_ceil(16_u128 * requested_width as u128, requested_height as u128);
    let minimum_columns = div_ceil(minimum_pixels, 8) as usize;
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "--fit at {columns} columns would be less than one terminal row; terminal must be at least {minimum_columns} columns wide"
        ),
    ))
}

fn div_ceil(numerator: u128, denominator: u128) -> u128 {
    numerator.div_ceil(denominator)
}

fn dimension_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "output dimensions overflow")
}

fn image_rows(image: &[u8]) -> io::Result<usize> {
    // IHDR starts at byte 8; its big-endian height occupies bytes 20..24. Kitty
    // placement uses terminal rows, approximated here as 16 image pixels each.
    let height = image
        .get(20..24)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PNG dimensions"))?;
    Ok((height as usize).div_ceil(16).max(1))
}

fn advance_rows<W: Write>(output: &mut W, rows: usize) -> io::Result<()> {
    for _ in 0..rows {
        output.write_all(b"\r\n")?;
    }
    Ok(())
}

fn output_columns(width: usize, scale: usize, mode: WidthMode) -> io::Result<usize> {
    let columns = scaled(width, scale)?;
    Ok(match mode {
        WidthMode::Full => columns,
        WidthMode::Crop(maximum) | WidthMode::Fit(maximum) => columns.min(maximum),
    })
}

fn encode_image(
    screen: &Screen,
    first_row: usize,
    rows: usize,
    scale: usize,
    width_mode: WidthMode,
) -> io::Result<Vec<u8>> {
    let maximum_width = |columns: usize| {
        columns.checked_mul(8).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "output dimensions overflow")
        })
    };
    let image = match width_mode {
        WidthMode::Full => png::encode_screen_scaled(screen, first_row, rows, scale),
        WidthMode::Crop(columns) => {
            png::encode_screen_scaled_crop(screen, first_row, rows, scale, maximum_width(columns)?)
        }
        WidthMode::Fit(columns) => {
            png::encode_screen_scaled_fit(screen, first_row, rows, scale, maximum_width(columns)?)
        }
    };
    image.map_err(io::Error::other)
}

fn write_image<W: Write>(
    output: &mut W,
    image: &[u8],
    columns: usize,
    rows: usize,
) -> io::Result<()> {
    // Kitty's direct-data transmission (`a=T`, `f=100`) carries a PNG. `m=1`
    // announces another transport chunk and `m=0` finishes the image. APC is
    // introduced by ESC _ G and terminated by ST (ESC backslash).
    let chunks = image.len().div_ceil(INPUT_CHUNK);
    for (index, bytes) in image.chunks(INPUT_CHUNK).enumerate() {
        let more = u8::from(index + 1 < chunks);
        if index == 0 {
            write!(
                output,
                "\x1b_Ga=T,f=100,c={columns},r={rows},C=1,q=2,m={more};"
            )?;
        } else {
            write!(output, "\x1b_Gq=2,m={more};")?;
        }
        base64(output, bytes)?;
        output.write_all(b"\x1b\\")?;
    }
    Ok(())
}

fn base64<W: Write>(output: &mut W, input: &[u8]) -> io::Result<()> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for bytes in input.chunks(3) {
        let value = (u32::from(bytes[0]) << 16)
            | (u32::from(*bytes.get(1).unwrap_or(&0)) << 8)
            | u32::from(*bytes.get(2).unwrap_or(&0));
        let encoded = [
            TABLE[((value >> 18) & 63) as usize],
            TABLE[((value >> 12) & 63) as usize],
            if bytes.len() > 1 {
                TABLE[((value >> 6) & 63) as usize]
            } else {
                b'='
            },
            if bytes.len() > 2 {
                TABLE[(value & 63) as usize]
            } else {
                b'='
            },
        ];
        output.write_all(&encoded)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cell;

    #[test]
    fn base64_matches_rfc4648_examples() {
        for (plain, encoded) in [(b"f".as_slice(), b"Zg==".as_slice()), (b"foo", b"Zm9v")] {
            let mut output = Vec::new();
            base64(&mut output, plain).unwrap();
            assert_eq!(output, encoded);
        }
    }

    #[test]
    fn screen_output_uses_kitty_apc_and_advances_lines() {
        let screen = Screen {
            width: 1,
            height: 1,
            cells: vec![Cell::default()],
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen(&mut output, &screen, 24).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=1,r=1,C=1,q=2,m=0;"));
        assert!(output.ends_with(b"\x1b\\\r\n"));
    }

    #[test]
    fn slow_output_uses_one_image_per_character_row() {
        let screen = Screen {
            width: 1,
            height: 2,
            cells: vec![Cell::default(); 2],
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen_slow(&mut output, &screen, Duration::ZERO).unwrap();
        assert_eq!(
            output
                .windows(b"\x1b_Ga=T".len())
                .filter(|window| *window == b"\x1b_Ga=T")
                .count(),
            2
        );
        assert_eq!(
            output
                .windows(b",r=1,C=1".len())
                .filter(|window| *window == b",r=1,C=1")
                .count(),
            2
        );
        assert_eq!(
            output
                .windows(2)
                .filter(|window| *window == b"\r\n")
                .count(),
            2
        );
    }

    #[test]
    fn scaled_output_uses_twice_the_terminal_footprint() {
        let screen = Screen {
            width: 1,
            height: 1,
            cells: vec![Cell::default()],
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen_scaled(&mut output, &screen, 24, 2).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=2,r=2,C=1,q=2,m=0;"));
        assert!(output.ends_with(b"\x1b\\\r\n\r\n"));
    }

    #[test]
    fn fit_output_caps_the_terminal_footprint_without_cropping_pixels() {
        let screen = Screen {
            width: 4,
            height: 2,
            cells: vec![Cell::default(); 8],
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen_scaled_fit(&mut output, &screen, 24, 2, 3).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=3,r=2,C=1,q=2,m="));
    }

    #[test]
    fn cropped_output_keeps_height_and_caps_width() {
        let screen = Screen {
            width: 4,
            height: 1,
            cells: vec![Cell::default(); 4],
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen_scaled_cropped(&mut output, &screen, 24, 1, 3).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=3,r=1,C=1,q=2,m="));
    }

    #[test]
    fn fit_reports_the_minimum_width_for_one_terminal_row() {
        let screen = Screen {
            width: 1_750,
            height: 25,
            cells: vec![Cell::default(); 1_750 * 25],
            glyph_height: 8,
            font: Some(crate::font::glyphs_8x8().to_vec()),
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let error = validate_fit_height(&screen, 1, 120, screen.height).unwrap_err();
        assert!(error.to_string().contains("at least 140 columns"));
        assert!(validate_fit_height(&screen, 1, 140, screen.height).is_ok());

        let slow_error = validate_fit_height(&screen, 1, 120, 1).unwrap_err();
        assert!(slow_error.to_string().contains("at least 3500 columns"));
    }

    #[test]
    fn eight_pixel_font_preserves_its_graphical_aspect() {
        let screen = Screen {
            width: 1,
            height: 19,
            cells: vec![Cell::default(); 19],
            glyph_height: 8,
            font: Some(crate::font::glyphs_8x8().to_vec()),
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen(&mut output, &screen, 24).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=1,r=10,C=1,q=2,m=0;"));
        assert!(output.ends_with(b"\r\n"));
        assert_eq!(
            output
                .windows(2)
                .filter(|window| *window == b"\r\n")
                .count(),
            10
        );
    }
}
