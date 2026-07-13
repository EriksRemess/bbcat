use std::io::{self, Write};

use crate::{Screen, png};

const INPUT_CHUNK: usize = 3072;

pub fn write_screen<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
) -> io::Result<()> {
    let chunk_lines = chunk_lines.max(1);
    for first_row in (0..screen.height).step_by(chunk_lines) {
        let rows = chunk_lines.min(screen.height - first_row);
        let image = png::encode_screen(screen, first_row, rows).map_err(io::Error::other)?;
        write_image(output, &image, screen.width, rows)?;
        for _ in 0..rows {
            output.write_all(b"\r\n")?;
        }
    }
    output.flush()
}

fn write_image<W: Write>(
    output: &mut W,
    image: &[u8],
    columns: usize,
    rows: usize,
) -> io::Result<()> {
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
        };
        let mut output = Vec::new();
        write_screen(&mut output, &screen, 24).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=1,r=1,C=1,q=2,m=0;"));
        assert!(output.ends_with(b"\x1b\\\r\n"));
    }
}
