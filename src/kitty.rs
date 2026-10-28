//! Kitty graphics protocol output.
//!
//! Kitty does not receive the character grid directly. This module first asks
//! the PNG backend for an image, base64-encodes that PNG, and wraps it in Kitty
//! APC escape sequences. Static artwork is split vertically (separate PNGs).
//! Animations use a single canvas with terminal-driven playback. All image
//! payloads are split into protocol-sized transport chunks.

use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use crate::{Animation, Screen, animation_image::frame_duration, png};

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

/// Uploads a looping Kitty animation at the requested baud rate and pixel scale.
///
/// Playback is terminal-driven and continues after this function returns. Each
/// frame must have the same pixel dimensions. The cursor advances below the
/// canvas once, and `Animation::clear_on_finish` is ignored because playback
/// loops indefinitely. Requires Kitty animation support (e.g. Ghostty 1.4+).
pub fn write_animation_scaled<W: Write>(
    output: &mut W,
    animation: &Animation,
    baud: u64,
    scale: usize,
) -> io::Result<()> {
    write_animation_inner(output, animation, baud, scale, WidthMode::Full)
}

/// Uploads a looping Kitty animation cropped to a terminal column count.
pub fn write_animation_scaled_cropped<W: Write>(
    output: &mut W,
    animation: &Animation,
    baud: u64,
    scale: usize,
    columns: usize,
) -> io::Result<()> {
    write_animation_inner(output, animation, baud, scale, WidthMode::Crop(columns))
}

/// Uploads a looping Kitty animation fitted to a terminal column count.
pub fn write_animation_scaled_fit<W: Write>(
    output: &mut W,
    animation: &Animation,
    baud: u64,
    scale: usize,
    columns: usize,
) -> io::Result<()> {
    write_animation_inner(output, animation, baud, scale, WidthMode::Fit(columns))
}

fn write_animation_inner<W: Write>(
    output: &mut W,
    animation: &Animation,
    baud: u64,
    scale: usize,
    width_mode: WidthMode,
) -> io::Result<()> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
    if baud == 0 {
        return Err(invalid("animation baud rate must be non-zero"));
    }
    if scale == 0 {
        return Err(invalid("output scale must be non-zero"));
    }
    if matches!(width_mode, WidthMode::Crop(0) | WidthMode::Fit(0)) {
        return Err(invalid("Kitty placement width must be non-zero"));
    }
    let first = animation
        .frames
        .first()
        .ok_or_else(|| invalid("animation contains no frames"))?;
    for frame in &animation.frames {
        frame
            .screen
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if frame.screen.pixel_dimensions() != first.screen.pixel_dimensions() {
            return Err(invalid("animation frames must share pixel dimensions"));
        }
        if let WidthMode::Fit(columns) = width_mode {
            validate_fit_height(&frame.screen, scale, columns, frame.screen.height)?;
        }
    }

    // Image numbers (I), unlike explicit IDs (i), allocate a new image on each
    // upload. Repeated calls therefore preserve older animations in scrollback.
    // All following commands address the newest image with this number.
    const NUMBER: u32 = 1;
    let image = encode_image(&first.screen, 0, first.screen.height, scale, width_mode)?;
    let columns = image_columns(&image)?;
    let rows = image_rows(&image)?;
    write_image_data(
        output,
        &image,
        &format!("a=T,f=100,I={NUMBER},c={columns},r={rows},C=1"),
        false,
    )?;
    // The root frame's gap must be set separately from its image upload.
    write!(
        output,
        "\x1b_Ga=a,I={NUMBER},r=1,z={},q=2;\x1b\\",
        frame_gap(first, baud)
    )?;
    for frame in animation.frames.iter().skip(1) {
        let image = encode_image(&frame.screen, 0, frame.screen.height, scale, width_mode)?;
        write_image_data(
            output,
            &image,
            &format!("a=f,f=100,I={NUMBER},X=1,z={}", frame_gap(frame, baud)),
            true,
        )?;
    }
    // Start only once every frame is uploaded; v=1 means loop forever.
    write!(output, "\x1b_Ga=a,I={NUMBER},s=3,v=1,c=1,q=2;\x1b\\")?;
    advance_rows(output, rows)?;
    output.flush()
}

fn frame_gap(frame: &crate::AnimationFrame, baud: u64) -> u32 {
    // Zero means "unspecified" in Kitty, and negative gaps skip the frame.
    // Clamp to a visible millisecond and to the protocol's signed 32-bit limit.
    frame_duration(frame, baud)
        .as_millis()
        .clamp(1, i32::MAX as u128) as u32
}

/// Writes a screen through the Kitty graphics protocol in vertical chunks.
pub fn write_screen<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, chunk_lines, None, 1, WidthMode::Full)
}

/// Writes Kitty graphics cropped to a terminal column count.
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

/// Fits the complete screen within a terminal column count and writes it.
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

/// Writes Kitty graphics at an integer pixel scale.
pub fn write_screen_scaled<W: Write>(
    output: &mut W,
    screen: &Screen,
    chunk_lines: usize,
    scale: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, chunk_lines, None, scale, WidthMode::Full)
}

/// Writes scaled Kitty graphics cropped to a terminal column count.
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

/// Fits and writes the complete screen after applying an integer scale.
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

/// Writes one Kitty image per character row with a delay between rows.
pub fn write_screen_slow<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), 1, WidthMode::Full)
}

/// Slowly writes Kitty graphics cropped to a terminal column count.
pub fn write_screen_slow_cropped<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), 1, WidthMode::Crop(columns))
}

/// Slowly writes Kitty graphics fitted to a terminal column count.
pub fn write_screen_slow_fit<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), 1, WidthMode::Fit(columns))
}

/// Slowly writes Kitty graphics at an integer pixel scale.
pub fn write_screen_slow_scaled<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    scale: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, 1, Some(delay), scale, WidthMode::Full)
}

/// Slowly writes scaled Kitty graphics cropped to terminal columns.
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

/// Slowly writes scaled Kitty graphics fitted to terminal columns.
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
    screen
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
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
        let columns = image_columns(&image)?;
        let placement_rows = image_rows(&image)?;
        write_image(output, &image, columns, placement_rows)?;
        advance_rows(output, placement_rows)?;
        return output.flush();
    }
    // Slow fit uses one source row per strip at the same width ratio. The
    // height check above ensures each strip still occupies a terminal row.
    let chunk_lines = if delay.is_some() {
        1
    } else if matches!(width_mode, WidthMode::Fit(_)) {
        screen.height
    } else {
        chunk_lines.max(1)
    };
    for first_row in (0..screen.height).step_by(chunk_lines) {
        let rows = chunk_lines.min(screen.height - first_row);
        let image = encode_image(screen, first_row, rows, scale, width_mode)?;
        let columns = image_columns(&image)?;
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
            screen
                .width
                .checked_mul(screen.glyph_width)
                .ok_or_else(dimension_error)?,
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

fn image_columns(image: &[u8]) -> io::Result<usize> {
    // The renderer uses an 8-pixel terminal-cell baseline. A 9-pixel VGA
    // raster must therefore reserve more cells instead of being squeezed into
    // the source's character-column count.
    let width = image
        .get(16..20)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PNG dimensions"))?;
    Ok((width as usize).div_ceil(8).max(1))
}

fn advance_rows<W: Write>(output: &mut W, rows: usize) -> io::Result<()> {
    for _ in 0..rows {
        output.write_all(b"\r\n")?;
    }
    Ok(())
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
    write_image_data(
        output,
        image,
        &format!("a=T,f=100,c={columns},r={rows},C=1"),
        false,
    )
}

fn write_image_data<W: Write>(
    output: &mut W,
    image: &[u8],
    control: &str,
    animation_frame: bool,
) -> io::Result<()> {
    // Kitty's direct-data transmission (`a=T`, `f=100`) carries a PNG. `m=1`
    // announces another transport chunk and `m=0` finishes the image. APC is
    // introduced by ESC _ G and terminated by ST (ESC backslash).
    let chunks = image.len().div_ceil(INPUT_CHUNK);
    for (index, bytes) in image.chunks(INPUT_CHUNK).enumerate() {
        let more = u8::from(index + 1 < chunks);
        if index == 0 {
            write!(output, "\x1b_G{control},q=2,m={more};")?;
        } else if animation_frame {
            write!(output, "\x1b_Ga=f,q=2,m={more};")?;
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

    fn animation() -> Animation {
        let screen = crate::decode(b"AB\nCD").unwrap().screen;
        let mut second = screen.clone();
        second.cells[0].character = u16::from(b'Z');
        second.cells[0].foreground = 196;
        Animation {
            frames: vec![
                crate::AnimationFrame {
                    screen,
                    source_bytes: 11_520,
                    duration: None,
                    utf8: false,
                    data: Vec::new(),
                },
                crate::AnimationFrame {
                    screen: second,
                    source_bytes: 0,
                    duration: Some(Duration::from_millis(250)),
                    utf8: true,
                    data: Vec::new(),
                },
            ],
            clear_on_finish: true,
        }
    }

    fn commands(output: &[u8]) -> Vec<(&str, &str)> {
        std::str::from_utf8(output)
            .unwrap()
            .split("\x1b_G")
            .skip(1)
            .map(|part| {
                part.split_once("\x1b\\")
                    .unwrap()
                    .0
                    .split_once(';')
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn animation_uploads_complete_frames_and_starts_a_terminal_loop() {
        let animation = animation();
        let mut output = Vec::new();
        write_animation_scaled(&mut output, &animation, 57_600, 1).unwrap();
        let commands = commands(&output);
        assert!(commands[0].0.starts_with("a=T,f=100,I=1,c=2,r=2,C=1"));
        assert_eq!(commands[1], ("a=a,I=1,r=1,z=200,q=2", ""));
        assert!(commands[2].0.starts_with("a=f,f=100,I=1,X=1,z=500"));
        assert_eq!(commands.last().unwrap(), &("a=a,I=1,s=3,v=1,c=1,q=2", ""));
        assert!(output.ends_with(b"\r\n\r\n"));
        // Each PNG uses its own palette/depth, including the later 256-color frame.
        for (index, frame) in animation.frames.iter().enumerate() {
            let png = encode_image(&frame.screen, 0, 2, 1, WidthMode::Full).unwrap();
            let mut encoded = Vec::new();
            base64(&mut encoded, &png).unwrap();
            assert_eq!(commands[index * 2].1.as_bytes(), encoded);
        }
    }

    #[test]
    fn animation_continuations_keep_the_frame_action_and_transport_limits() {
        let mut animation = animation();
        let mut screen = crate::decode(b"X").unwrap().screen;
        screen.width = 80;
        screen.height = 24;
        screen.cells = vec![Cell::default(); 80 * 24];
        // Vary glyphs and colors so PNG compression cannot collapse the upload
        // into a single transport chunk.
        let mut random = 42_u32;
        for cell in &mut screen.cells {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            cell.character = u16::from((random >> 24) as u8);
            cell.foreground = (random >> 16) as u8;
            cell.background = (random >> 8) as u8;
        }
        for frame in &mut animation.frames {
            frame.screen = screen.clone();
        }
        let mut output = Vec::new();
        write_animation_scaled(&mut output, &animation, 115_200, 1).unwrap();
        let commands = commands(&output);
        let mut payloads = Vec::new();
        let mut current = String::new();
        let mut frame_upload = false;
        let mut continuations = 0;
        for (header, payload) in commands {
            assert!(header.contains("q=2"));
            if payload.is_empty() {
                continue;
            }
            if current.is_empty() {
                frame_upload = header.starts_with("a=f,");
            } else {
                continuations += 1;
                assert_eq!(header.starts_with("a=f,"), frame_upload);
                assert!(!header.contains("I="));
                assert!(!header.contains("f=100"));
            }
            assert!(payload.len() <= 4096);
            assert_eq!(payload.len() % 4, 0);
            current.push_str(payload);
            if header.ends_with("m=0") {
                payloads.push(std::mem::take(&mut current));
            } else {
                assert!(header.ends_with("m=1"));
                assert_eq!(payload.len(), 4096);
            }
        }
        assert!(continuations > 0);
        assert!(current.is_empty());
        assert_eq!(payloads.len(), 2);
        let png = encode_image(&screen, 0, screen.height, 1, WidthMode::Full).unwrap();
        let mut encoded = Vec::new();
        base64(&mut encoded, &png).unwrap();
        for payload in payloads {
            assert_eq!(payload.as_bytes(), encoded);
        }
    }

    #[test]
    fn animation_crop_and_fit_use_one_scaled_canvas() {
        for (mode, expected) in [
            (WidthMode::Crop(2), "c=2,r=4"),
            (WidthMode::Fit(2), "c=2,r=2"),
        ] {
            let mut output = Vec::new();
            let animation = animation();
            write_animation_inner(&mut output, &animation, 115_200, 2, mode).unwrap();
            let commands = commands(&output);
            assert!(commands[0].0.contains(expected));
            // Only one placement is made, regardless of the number of frames.
            assert_eq!(
                commands
                    .iter()
                    .filter(|(header, _)| header.starts_with("a=T,"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn animation_gaps_are_visible_and_fit_the_protocol_integer() {
        let mut frame = animation().frames.remove(0);
        assert_eq!(frame_gap(&frame, 115_200), 100);
        assert_eq!(frame_gap(&frame, 230_400), 50);
        frame.source_bytes = 0;
        assert_eq!(frame_gap(&frame, 115_200), 1);
        frame.duration = Some(Duration::MAX);
        assert_eq!(frame_gap(&frame, 1), i32::MAX as u32);
        frame.duration = Some(Duration::ZERO);
        assert_eq!(frame_gap(&frame, 115_200), 1);
    }

    #[test]
    fn invalid_animations_fail_before_writing_graphics() {
        let mut output = Vec::new();
        let mut animation = animation();
        assert!(write_animation_scaled(&mut output, &animation, 0, 1).is_err());
        assert!(write_animation_scaled(&mut output, &animation, 1, 0).is_err());
        assert!(write_animation_scaled_cropped(&mut output, &animation, 1, 1, 0).is_err());
        assert!(write_animation_scaled_fit(&mut output, &animation, 1, 1, 0).is_err());
        animation.frames[1].screen = crate::decode(b"X").unwrap().screen;
        assert!(write_animation_scaled(&mut output, &animation, 1, 1).is_err());
        animation.frames.clear();
        assert!(write_animation_scaled(&mut output, &animation, 1, 1).is_err());
        assert!(output.is_empty());
    }

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
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            true_colors: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen(&mut output, &screen, 24).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=1,r=1,C=1,q=2,m=0;"));
        assert!(output.ends_with(b"\x1b\\\r\n"));
    }

    #[test]
    fn nine_pixel_raster_uses_its_full_terminal_width() {
        let screen = Screen {
            width: 1,
            height: 1,
            cells: vec![Cell::default()],
            glyph_width: 9,
            glyph_height: 16,
            font: None,
            palette: None,
            true_colors: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen(&mut output, &screen, 24).unwrap();

        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=2,r=1,C=1,q=2,m=0;"));
    }

    #[test]
    fn slow_output_uses_one_image_per_character_row() {
        let screen = Screen {
            width: 1,
            height: 2,
            cells: vec![Cell::default(); 2],
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            true_colors: None,
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
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            true_colors: None,
            utf8_supported: true,
            raster: None,
        };
        let mut output = Vec::new();
        write_screen_scaled(&mut output, &screen, 24, 2).unwrap();
        assert!(output.starts_with(b"\x1b_Ga=T,f=100,c=2,r=2,C=1,q=2,m=0;"));
        assert!(output.ends_with(b"\x1b\\\r\n\r\n"));
    }

    #[test]
    fn slow_fit_reveals_each_row_with_a_delay() {
        let document = crate::decode(b"AB\nCD").unwrap();
        // 2x scaling then fitting into two columns exercises actual downscaling.
        let mut output = Vec::new();
        let delay = Duration::from_millis(10);
        let started = std::time::Instant::now();
        write_screen_slow_scaled_fit(&mut output, &document.screen, delay, 2, 2).unwrap();
        assert_eq!(
            output
                .windows(b"\x1b_Ga=T".len())
                .filter(|window| *window == b"\x1b_Ga=T")
                .count(),
            2
        );
        assert!(started.elapsed() >= delay);
    }

    #[test]
    fn fit_output_caps_the_terminal_footprint_without_cropping_pixels() {
        let screen = Screen {
            width: 4,
            height: 2,
            cells: vec![Cell::default(); 8],
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            true_colors: None,
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
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            true_colors: None,
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
            glyph_width: 8,
            glyph_height: 8,
            font: Some(crate::font::glyphs_8x8().to_vec()),
            palette: None,
            true_colors: None,
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
            glyph_width: 8,
            glyph_height: 8,
            font: Some(crate::font::glyphs_8x8().to_vec()),
            palette: None,
            true_colors: None,
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
