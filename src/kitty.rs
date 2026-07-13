use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use crate::{Screen, png};

const INPUT_CHUNK: usize = 3072;

pub fn write_screen<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
) -> io::Result<()> {
    write_screen_scaled(output, screen, chunk_lines, 1)
}

pub fn write_screen_scaled<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    scale: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, chunk_lines, None, scale)
}

pub fn write_screen_slow<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
) -> io::Result<()> {
    write_screen_slow_scaled(output, screen, delay, 1)
}

pub fn write_screen_slow_scaled<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    scale: usize,
) -> io::Result<()> {
    if screen.raster.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "slow mode is not supported for RIPscrip graphics",
        ));
    }
    write_screen_inner(output, screen, 1, Some(delay), scale)
}

fn write_screen_inner<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    delay: Option<Duration>,
    scale: usize,
) -> io::Result<()> {
    if scale == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output scale must be non-zero",
        ));
    }
    if screen.raster.is_some() {
        let image =
            png::encode_screen_scaled(screen, 0, screen.height, scale).map_err(io::Error::other)?;
        let columns = scaled(screen.width, scale)?;
        write_image(output, &image, columns)?;
        output.write_all(b"\r")?;
        return output.flush();
    }
    let chunk_lines = chunk_lines.max(1);
    for first_row in (0..screen.height).step_by(chunk_lines) {
        let rows = chunk_lines.min(screen.height - first_row);
        let image =
            png::encode_screen_scaled(screen, first_row, rows, scale).map_err(io::Error::other)?;
        let columns = scaled(screen.width, scale)?;
        write_image(output, &image, columns)?;
        output.write_all(b"\r")?;
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

fn write_image<W: Write>(output: &mut W, image: &[u8], columns: usize) -> io::Result<()> {
    let chunks = image.len().div_ceil(INPUT_CHUNK);
    for (index, bytes) in image.chunks(INPUT_CHUNK).enumerate() {
        let more = u8::from(index + 1 < chunks);
        if index == 0 {
            write!(output, "\x1b_Ga=T,f=100,c={columns},q=2,m={more};")?;
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
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=1,q=2,m=0;"));
        assert!(output.ends_with(b"\x1b\\\r"));
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
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=2,q=2,m=0;"));
        assert!(output.ends_with(b"\x1b\\\r"));
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
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=1,q=2,m=0;"));
        assert!(!output.windows(3).any(|window| window == b",r="));
        assert!(output.ends_with(b"\x1b\\\r"));
    }
}
