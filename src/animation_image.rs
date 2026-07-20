//! Dependency-free APNG and GIF encoders for rendered animation frames.
//!
//! Frames first use bbcat's indexed PNG rasterizer. APNG can retain those
//! compressed IDAT streams directly; GIF unpacks the same stored-zlib rows and
//! writes a simple 16-color LZW stream. Keeping one raster path means terminal,
//! PNG, APNG, and GIF agree on fonts, palettes, and SAUCE letter spacing.

use std::{collections::HashMap, time::Duration};

use crate::{Animation, AnimationFrame, DEFAULT_ANIMATION_BAUD, png};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

struct FrameImage {
    ihdr: Vec<u8>,
    palette: Vec<u8>,
    idat: Vec<u8>,
    width: u32,
    height: u32,
}

/// Encodes decoded animation frames as a looping indexed-color APNG.
pub fn encode_animation_apng(
    animation: &Animation,
    baud: u64,
    scale: usize,
) -> Result<Vec<u8>, String> {
    let images = rendered_frames(animation, baud, scale)?;
    let first = &images[0].0;
    let frame_count = u32::try_from(images.len()).map_err(|_| "animation has too many frames")?;
    let mut output = PNG_SIGNATURE.to_vec();
    png_chunk(&mut output, b"IHDR", &first.ihdr);
    let mut control = Vec::with_capacity(8);
    control.extend_from_slice(&frame_count.to_be_bytes());
    control.extend_from_slice(&0_u32.to_be_bytes()); // loop forever
    png_chunk(&mut output, b"acTL", &control);
    png_chunk(&mut output, b"PLTE", &first.palette);

    let mut sequence = 0_u32;
    for (index, (image, duration)) in images.iter().enumerate() {
        frame_control(&mut output, &mut sequence, image, *duration)?;
        if index == 0 {
            png_chunk(&mut output, b"IDAT", &image.idat);
        } else {
            let mut data = Vec::with_capacity(4 + image.idat.len());
            data.extend_from_slice(&next_sequence(&mut sequence)?.to_be_bytes());
            data.extend_from_slice(&image.idat);
            png_chunk(&mut output, b"fdAT", &data);
        }
    }
    png_chunk(&mut output, b"IEND", &[]);
    Ok(output)
}

/// Encodes decoded animation frames as a looping 16-color GIF.
pub fn encode_animation_gif(
    animation: &Animation,
    baud: u64,
    scale: usize,
) -> Result<Vec<u8>, String> {
    let images = rendered_frames(animation, baud, scale)?;
    let first = &images[0].0;
    let width = u16::try_from(first.width).map_err(|_| "GIF width exceeds 65535 pixels")?;
    let height = u16::try_from(first.height).map_err(|_| "GIF height exceeds 65535 pixels")?;
    if first.palette.len() != 16 * 3 {
        return Err("GIF output requires a 16-color palette".to_owned());
    }

    let mut output = b"GIF89a".to_vec();
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.extend_from_slice(&[0xb3, 0, 0]); // 16-color global table, 4-bit resolution
    output.extend_from_slice(&first.palette);
    output.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00");

    for (image, duration) in &images {
        let pixels = png_pixels(image)?;
        let delay = gif_delay(*duration);
        output.extend_from_slice(&[0x21, 0xf9, 4, 4]); // preserve the prior full frame
        output.extend_from_slice(&delay.to_le_bytes());
        output.extend_from_slice(&[0, 0]);
        output.push(0x2c);
        output.extend_from_slice(&[0, 0, 0, 0]);
        output.extend_from_slice(&width.to_le_bytes());
        output.extend_from_slice(&height.to_le_bytes());
        output.push(0); // use the global palette
        output.push(4); // LZW minimum code size
        gif_sub_blocks(&mut output, &gif_lzw(&pixels));
    }
    output.push(0x3b);
    Ok(output)
}

fn rendered_frames(
    animation: &Animation,
    baud: u64,
    scale: usize,
) -> Result<Vec<(FrameImage, Duration)>, String> {
    if baud == 0 {
        return Err("animation baud rate must be non-zero".to_owned());
    }
    if animation.frames.is_empty() {
        return Err("animation contains no frames".to_owned());
    }
    let mut result = Vec::with_capacity(animation.frames.len());
    let mut expected = None;
    for frame in &animation.frames {
        let png = png::encode_screen_scaled(&frame.screen, 0, frame.screen.height, scale)?;
        let image = parse_png(&png)?;
        if let Some((ihdr, palette)) = expected.as_ref() {
            if image.ihdr != *ihdr || image.palette != *palette {
                return Err("animation frames must share dimensions and palette".to_owned());
            }
        } else {
            expected = Some((image.ihdr.clone(), image.palette.clone()));
        }
        result.push((image, frame_duration(frame, baud)));
    }
    Ok(result)
}

fn frame_duration(frame: &AnimationFrame, baud: u64) -> Duration {
    match frame.duration {
        Some(duration) => scale_duration(duration, baud),
        None => transmission_time(frame.source_bytes, baud),
    }
}

fn transmission_time(bytes: usize, baud: u64) -> Duration {
    let nanoseconds = bytes as u128 * 1_000_000_000 / u128::from(baud);
    Duration::from_nanos(u64::try_from(nanoseconds).unwrap_or(u64::MAX))
}

fn scale_duration(duration: Duration, baud: u64) -> Duration {
    let nanoseconds = duration.as_nanos() * u128::from(DEFAULT_ANIMATION_BAUD) / u128::from(baud);
    Duration::from_nanos(u64::try_from(nanoseconds).unwrap_or(u64::MAX))
}

fn parse_png(data: &[u8]) -> Result<FrameImage, String> {
    if !data.starts_with(PNG_SIGNATURE) {
        return Err("internal PNG encoder returned an invalid signature".to_owned());
    }
    let mut offset = PNG_SIGNATURE.len();
    let mut ihdr = None;
    let mut palette = None;
    let mut idat = Vec::new();
    while offset < data.len() {
        let length = read_u32(data, offset)? as usize;
        let chunk_start = offset.checked_add(8).ok_or("PNG chunk offset overflow")?;
        let chunk_end = chunk_start
            .checked_add(length)
            .ok_or("PNG chunk length overflow")?;
        let crc_end = chunk_end
            .checked_add(4)
            .ok_or("PNG chunk length overflow")?;
        let kind = data
            .get(offset + 4..offset + 8)
            .ok_or("truncated PNG chunk type")?;
        let payload = data
            .get(chunk_start..chunk_end)
            .ok_or("truncated PNG chunk data")?;
        if data.get(chunk_end..crc_end).is_none() {
            return Err("truncated PNG chunk CRC".to_owned());
        }
        match kind {
            b"IHDR" => ihdr = Some(payload.to_vec()),
            b"PLTE" => palette = Some(payload.to_vec()),
            b"IDAT" => idat.extend_from_slice(payload),
            b"IEND" => break,
            _ => {}
        }
        offset = crc_end;
    }
    let ihdr = ihdr.ok_or("internal PNG is missing IHDR")?;
    if ihdr.len() != 13 || ihdr[8..] != [4, 3, 0, 0, 0] {
        return Err("internal PNG is not 4-bit indexed color".to_owned());
    }
    let width = u32::from_be_bytes(ihdr[0..4].try_into().unwrap());
    let height = u32::from_be_bytes(ihdr[4..8].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err("internal PNG has zero dimensions".to_owned());
    }
    Ok(FrameImage {
        ihdr,
        palette: palette.ok_or("internal PNG is missing PLTE")?,
        idat,
        width,
        height,
    })
}

fn frame_control(
    output: &mut Vec<u8>,
    sequence: &mut u32,
    image: &FrameImage,
    duration: Duration,
) -> Result<(), String> {
    let (numerator, denominator) = apng_delay(duration);
    let mut control = Vec::with_capacity(26);
    control.extend_from_slice(&next_sequence(sequence)?.to_be_bytes());
    control.extend_from_slice(&image.width.to_be_bytes());
    control.extend_from_slice(&image.height.to_be_bytes());
    control.extend_from_slice(&[0; 8]); // x/y offset
    control.extend_from_slice(&numerator.to_be_bytes());
    control.extend_from_slice(&denominator.to_be_bytes());
    control.extend_from_slice(&[0, 0]); // no disposal; replace the full canvas
    png_chunk(output, b"fcTL", &control);
    Ok(())
}

fn apng_delay(duration: Duration) -> (u16, u16) {
    let milliseconds = duration.as_millis().max(1);
    if milliseconds <= u128::from(u16::MAX) {
        (milliseconds as u16, 1_000)
    } else {
        (
            milliseconds.div_ceil(10).min(u128::from(u16::MAX)) as u16,
            100,
        )
    }
}

fn gif_delay(duration: Duration) -> u16 {
    duration
        .as_millis()
        .div_ceil(10)
        .clamp(1, u128::from(u16::MAX)) as u16
}

fn next_sequence(sequence: &mut u32) -> Result<u32, String> {
    let current = *sequence;
    *sequence = sequence
        .checked_add(1)
        .ok_or("APNG sequence number overflow")?;
    Ok(current)
}

fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(payload);
    let mut crc_data = Vec::with_capacity(4 + payload.len());
    crc_data.extend_from_slice(kind);
    crc_data.extend_from_slice(payload);
    output.extend_from_slice(&crc32(&crc_data).to_be_bytes());
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

fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_be_bytes(
        data.get(offset..offset + 4)
            .ok_or("truncated PNG chunk length")?
            .try_into()
            .unwrap(),
    ))
}

fn png_pixels(image: &FrameImage) -> Result<Vec<u8>, String> {
    let raw = zlib_store_decode(&image.idat)?;
    let width = image.width as usize;
    let height = image.height as usize;
    let row_bytes = width.div_ceil(2);
    if raw.len()
        != height
            .checked_mul(row_bytes + 1)
            .ok_or("PNG row size overflow")?
    {
        return Err("internal PNG scanlines have an unexpected length".to_owned());
    }
    let mut pixels = Vec::with_capacity(width * height);
    for row in raw.chunks_exact(row_bytes + 1) {
        if row[0] != 0 {
            return Err("internal PNG uses an unsupported row filter".to_owned());
        }
        for pixel in 0..width {
            let packed = row[1 + pixel / 2];
            pixels.push(if pixel % 2 == 0 {
                packed >> 4
            } else {
                packed & 0x0f
            });
        }
    }
    Ok(pixels)
}

fn zlib_store_decode(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.get(..2) != Some(&[0x78, 0x01]) {
        return Err("internal PNG uses an unexpected zlib stream".to_owned());
    }
    let mut offset = 2;
    let mut output = Vec::new();
    loop {
        let header = *data.get(offset).ok_or("truncated zlib block")?;
        offset += 1;
        if header & 0b110 != 0 {
            return Err("internal PNG uses compressed DEFLATE blocks".to_owned());
        }
        let length = u16::from_le_bytes(
            data.get(offset..offset + 2)
                .ok_or("truncated zlib block length")?
                .try_into()
                .unwrap(),
        ) as usize;
        let complement = u16::from_le_bytes(
            data.get(offset + 2..offset + 4)
                .ok_or("truncated zlib block length")?
                .try_into()
                .unwrap(),
        );
        if complement != !(length as u16) {
            return Err("invalid zlib stored block length".to_owned());
        }
        offset += 4;
        let end = offset
            .checked_add(length)
            .ok_or("zlib block length overflow")?;
        output.extend_from_slice(data.get(offset..end).ok_or("truncated zlib block data")?);
        offset = end;
        if header & 1 != 0 {
            break;
        }
    }
    if data.len() != offset + 4 {
        return Err("internal PNG has an unexpected zlib trailer".to_owned());
    }
    Ok(output)
}

fn gif_lzw(pixels: &[u8]) -> Vec<u8> {
    let clear = 16_u16;
    let end = 17_u16;
    let mut bits = BitWriter::default();
    let mut dictionary = HashMap::new();
    let mut code_size = 5_u8;
    let mut next_code = 18_u16;
    bits.push(clear, code_size);
    let Some((&first, rest)) = pixels.split_first() else {
        bits.push(end, code_size);
        return bits.finish();
    };
    let mut current = u16::from(first);
    for &pixel in rest {
        if let Some(&code) = dictionary.get(&(current, pixel)) {
            current = code;
            continue;
        }
        bits.push(current, code_size);
        if next_code < 4096 {
            dictionary.insert((current, pixel), next_code);
            next_code += 1;
            // The decoder adds this dictionary entry only after it consumes
            // the *next* code. Keep the old width for that code, then grow
            // the encoder one entry later so both bitstreams stay aligned.
            if next_code > (1 << code_size) && code_size < 12 {
                code_size += 1;
            }
        } else {
            bits.push(clear, code_size);
            dictionary.clear();
            code_size = 5;
            next_code = 18;
        }
        current = u16::from(pixel);
    }
    bits.push(current, code_size);
    bits.push(end, code_size);
    bits.finish()
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    buffer: u32,
    bits: u8,
}

impl BitWriter {
    fn push(&mut self, code: u16, width: u8) {
        self.buffer |= u32::from(code) << self.bits;
        self.bits += width;
        while self.bits >= 8 {
            self.bytes.push(self.buffer as u8);
            self.buffer >>= 8;
            self.bits -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits != 0 {
            self.bytes.push(self.buffer as u8);
        }
        self.bytes
    }
}

fn gif_sub_blocks(output: &mut Vec<u8>, data: &[u8]) {
    for block in data.chunks(255) {
        output.push(block.len() as u8);
        output.extend_from_slice(block);
    }
    output.push(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cell, Screen};

    fn animation() -> Animation {
        let screen = |character| Screen {
            width: 1,
            height: 1,
            cells: vec![Cell {
                character,
                foreground: 15,
                background: 0,
            }],
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        };
        Animation {
            frames: vec![
                AnimationFrame {
                    screen: screen(u16::from(b'A')),
                    source_bytes: 1152,
                    duration: None,
                    utf8: false,
                    data: vec![],
                },
                AnimationFrame {
                    screen: screen(u16::from(b'B')),
                    source_bytes: 1,
                    duration: Some(Duration::from_millis(20)),
                    utf8: false,
                    data: vec![],
                },
            ],
            clear_on_finish: false,
        }
    }

    #[test]
    fn writes_looping_apng_with_a_control_for_each_frame() {
        let apng = encode_animation_apng(&animation(), DEFAULT_ANIMATION_BAUD, 1).unwrap();
        assert_eq!(&apng[..8], PNG_SIGNATURE);
        assert_eq!(apng.windows(4).filter(|chunk| *chunk == b"acTL").count(), 1);
        assert_eq!(apng.windows(4).filter(|chunk| *chunk == b"fcTL").count(), 2);
        assert!(apng.windows(4).any(|chunk| chunk == b"fdAT"));
    }

    #[test]
    fn writes_looping_gif_with_two_images() {
        let gif = encode_animation_gif(&animation(), DEFAULT_ANIMATION_BAUD, 1).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        assert_eq!(
            gif.windows(2).filter(|chunk| *chunk == b"\x21\xf9").count(),
            2
        );
        assert_eq!(gif.last(), Some(&0x3b));
    }

    #[test]
    fn frame_duration_matches_terminal_baud_timing() {
        let frame = &animation().frames[0];
        assert_eq!(
            frame_duration(frame, DEFAULT_ANIMATION_BAUD),
            Duration::from_millis(10)
        );
        assert_eq!(frame_duration(frame, 57_600), Duration::from_millis(20));
    }
}
