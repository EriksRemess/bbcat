//! UTF-8 terminal output for character-based art.
//!
//! Each CP437 cell becomes its Unicode equivalent, preceded by a true-color ANSI
//! escape whenever its foreground/background pair changes. Embedded fonts and
//! 512-glyph XBin screens cannot be represented faithfully as Unicode, and
//! RIPscrip has no character cells at all, so those require graphical output.
//! ANSI animation commits each detected screen state atomically, with its
//! encoded byte count determining baud-paced frame timing. Formats with native
//! timing, such as DDW, use generated UTF-8 frame data directly.

use std::{
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};

use crate::{Animation, Screen, png::VGA_PALETTE};

/// Default ANSI playback rate and the baseline for native frame timing.
pub const DEFAULT_ANIMATION_BAUD: u64 = 115_200;

pub(crate) const CP437: [char; 256] = [
    '\u{20}', '\u{263a}', '\u{263b}', '\u{2665}', '\u{2666}', '\u{2663}', '\u{2660}', '\u{2022}',
    '\u{25d8}', '\u{25cb}', '\u{25d9}', '\u{2642}', '\u{2640}', '\u{266a}', '\u{266b}', '\u{263c}',
    '\u{25ba}', '\u{25c4}', '\u{2195}', '\u{203c}', '\u{b6}', '\u{a7}', '\u{25ac}', '\u{21a8}',
    '\u{2191}', '\u{2193}', '\u{2192}', '\u{2190}', '\u{221f}', '\u{2194}', '\u{25b2}', '\u{25bc}',
    '\u{20}', '\u{21}', '\u{22}', '\u{23}', '\u{24}', '\u{25}', '\u{26}', '\u{27}', '\u{28}',
    '\u{29}', '\u{2a}', '\u{2b}', '\u{2c}', '\u{2d}', '\u{2e}', '\u{2f}', '\u{30}', '\u{31}',
    '\u{32}', '\u{33}', '\u{34}', '\u{35}', '\u{36}', '\u{37}', '\u{38}', '\u{39}', '\u{3a}',
    '\u{3b}', '\u{3c}', '\u{3d}', '\u{3e}', '\u{3f}', '\u{40}', '\u{41}', '\u{42}', '\u{43}',
    '\u{44}', '\u{45}', '\u{46}', '\u{47}', '\u{48}', '\u{49}', '\u{4a}', '\u{4b}', '\u{4c}',
    '\u{4d}', '\u{4e}', '\u{4f}', '\u{50}', '\u{51}', '\u{52}', '\u{53}', '\u{54}', '\u{55}',
    '\u{56}', '\u{57}', '\u{58}', '\u{59}', '\u{5a}', '\u{5b}', '\u{5c}', '\u{5d}', '\u{5e}',
    '\u{5f}', '\u{60}', '\u{61}', '\u{62}', '\u{63}', '\u{64}', '\u{65}', '\u{66}', '\u{67}',
    '\u{68}', '\u{69}', '\u{6a}', '\u{6b}', '\u{6c}', '\u{6d}', '\u{6e}', '\u{6f}', '\u{70}',
    '\u{71}', '\u{72}', '\u{73}', '\u{74}', '\u{75}', '\u{76}', '\u{77}', '\u{78}', '\u{79}',
    '\u{7a}', '\u{7b}', '\u{7c}', '\u{7d}', '\u{7e}', '\u{2302}', '\u{c7}', '\u{fc}', '\u{e9}',
    '\u{e2}', '\u{e4}', '\u{e0}', '\u{e5}', '\u{e7}', '\u{ea}', '\u{eb}', '\u{e8}', '\u{ef}',
    '\u{ee}', '\u{ec}', '\u{c4}', '\u{c5}', '\u{c9}', '\u{e6}', '\u{c6}', '\u{f4}', '\u{f6}',
    '\u{f2}', '\u{fb}', '\u{f9}', '\u{ff}', '\u{d6}', '\u{dc}', '\u{a2}', '\u{a3}', '\u{a5}',
    '\u{20a7}', '\u{192}', '\u{e1}', '\u{ed}', '\u{f3}', '\u{fa}', '\u{f1}', '\u{d1}', '\u{aa}',
    '\u{ba}', '\u{bf}', '\u{2310}', '\u{ac}', '\u{bd}', '\u{bc}', '\u{a1}', '\u{ab}', '\u{bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{3b1}', '\u{df}', '\u{393}', '\u{3c0}', '\u{3a3}', '\u{3c3}', '\u{b5}', '\u{3c4}',
    '\u{3a6}', '\u{398}', '\u{3a9}', '\u{3b4}', '\u{221e}', '\u{3c6}', '\u{3b5}', '\u{2229}',
    '\u{2261}', '\u{b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{f7}', '\u{2248}',
    '\u{b0}', '\u{2219}', '\u{b7}', '\u{221a}', '\u{207f}', '\u{b2}', '\u{25a0}', '\u{a0}',
];

/// Writes a complete character screen as UTF-8 with ANSI colors.
pub fn write_screen<W: Write>(output: &mut W, screen: &Screen) -> io::Result<()> {
    write_screen_inner(output, screen, None, screen.width)
}

/// Plays an animation at [`DEFAULT_ANIMATION_BAUD`].
pub fn write_animation<W: Write>(output: &mut W, animation: &Animation) -> io::Result<()> {
    write_animation_at_baud(output, animation, DEFAULT_ANIMATION_BAUD)
}

/// Plays an animation at the requested source-byte rate.
pub fn write_animation_at_baud<W: Write>(
    output: &mut W,
    animation: &Animation,
    baud: u64,
) -> io::Result<()> {
    write_animation_at_baud_inner(output, animation, baud)
}

fn write_animation_at_baud_inner<W: Write>(
    output: &mut W,
    animation: &Animation,
    baud: u64,
) -> io::Result<()> {
    if baud == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "animation baud rate must be non-zero",
        ));
    }
    if animation.frames.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "animation contains no frames",
        ));
    }

    // A home/full-clear boundary commits the preceding ANSI updates as one
    // frame. The selected viewer rate controls its duration from the encoded
    // byte count; synchronized-output terminals never expose a half-drawn
    // frame.
    let started = Instant::now();
    let mut source_bytes = 0_usize;
    let mut elapsed = Duration::ZERO;
    let mut buffer = Vec::new();
    // Keep the classic ANSI style between delta frames.  Once an xterm colour
    // escape is seen it remains terminal-managed until an SGR reset returns us
    // to a known VGA style.
    let mut style = Some(AnimationStyle::default());
    let clears_before_playback = animation
        .frames
        .first()
        .is_some_and(|frame| source_uses_absolute_top(&frame.data));
    for (frame_index, frame) in animation.frames.iter().enumerate() {
        buffer.clear();
        buffer.extend_from_slice(b"\x1b[?2026h");
        if frame_index == 0 {
            if clears_before_playback {
                // An absolute top-left animation owns the terminal canvas.
                // Clear it atomically before its first frame is revealed.
                buffer.extend_from_slice(b"\x1b[2J\x1b[H\x1b[0m");
            } else {
                // Preserve the existing terminal for animations that draw
                // relative to their insertion point.
                output.write_all(b"\x1b[0m\r\n")?;
                output.flush()?;
                buffer.extend_from_slice(b"\x1b[0m");
            }
        }
        if frame.utf8 {
            buffer.extend_from_slice(&frame.data);
        } else {
            transcode_frame(&frame.data, &mut buffer, &mut style);
        }
        buffer.extend_from_slice(b"\x1b[?2026l");
        output.write_all(&buffer)?;
        output.flush()?;

        if let Some(duration) = frame.duration {
            elapsed = elapsed.saturating_add(scale_duration(duration, baud));
            sleep_until(started, elapsed);
        } else {
            source_bytes = source_bytes.saturating_add(frame.source_bytes);
            sleep_until(started, transmission_time(source_bytes, baud));
        }
    }

    // Preserve the final frame even when the source ends with a clear-screen
    // command. An absolute canvas needs an absolute prompt position; a
    // relative animation can continue naturally from its final cursor.
    if clears_before_playback {
        let final_row = animation
            .frames
            .last()
            .expect("validated non-empty animation")
            .screen
            .height;
        write!(output, "\x1b[0m\x1b[{final_row};1H\r\n")?;
    } else {
        output.write_all(b"\x1b[0m\r\n")?;
    }
    output.flush()
}

fn source_uses_absolute_top(input: &[u8]) -> bool {
    let mut index = 0;
    while index + 2 < input.len() {
        if input[index] != 0x1b || input[index + 1] != b'[' {
            index += 1;
            continue;
        }
        let Some(relative_end) = input[index + 2..]
            .iter()
            .position(|byte| (0x40..=0x7e).contains(byte))
        else {
            break;
        };
        let end = index + relative_end + 2;
        if matches!(input[end], b'H' | b'f') && cursor_is_absolute_top(&input[index + 2..end]) {
            return true;
        }
        index = end + 1;
    }
    false
}

fn cursor_is_absolute_top(parameters: &[u8]) -> bool {
    let mut parameters = parameters.split(|&byte| byte == b';');
    let row = cursor_parameter(parameters.next());
    let column = cursor_parameter(parameters.next());
    row == Some(1) && column == Some(1)
}

fn cursor_parameter(parameter: Option<&[u8]>) -> Option<usize> {
    let parameter = parameter.unwrap_or_default();
    if parameter.is_empty() {
        Some(1)
    } else {
        std::str::from_utf8(parameter)
            .ok()?
            .parse::<usize>()
            .ok()
            .map(|value| value.max(1))
    }
}

fn transcode_frame(input: &[u8], output: &mut Vec<u8>, style: &mut Option<AnimationStyle>) {
    let mut index = 0_usize;
    while index < input.len() {
        let consumed = match input[index] {
            0x1a => break,
            0x1b => transcode_escape(&input[index..], output, style),
            b'\r' | b'\n' | b'\t' => {
                output.push(input[index]);
                1
            }
            character => {
                let mut encoded = [0_u8; 4];
                output.extend_from_slice(
                    CP437[usize::from(character)]
                        .encode_utf8(&mut encoded)
                        .as_bytes(),
                );
                1
            }
        };
        index += consumed;
    }
}

fn transcode_escape(
    input: &[u8],
    output: &mut Vec<u8>,
    style: &mut Option<AnimationStyle>,
) -> usize {
    let Some(&second) = input.get(1) else {
        return 1;
    };
    if second != b'[' {
        match second {
            b'7' | b'8' => output.extend_from_slice(&input[..2]),
            // Do not forward a destructive full terminal reset. This is the
            // cursor/style subset implemented by bbcat's ANSI state machine.
            b'c' => output.extend_from_slice(b"\x1b[0m\x1b[H"),
            _ => {}
        }
        return 2;
    }

    let Some(relative_end) = input[2..]
        .iter()
        .position(|byte| (0x40..=0x7e).contains(byte))
    else {
        return input.len();
    };
    let end = relative_end + 2;
    let command = input[end];
    let raw = &input[2..end];
    let standard_parameters = raw
        .iter()
        .all(|byte| byte.is_ascii_digit() || *byte == b';');
    let supported = standard_parameters
        && matches!(
            command,
            b'm' | b'A'
                | b'B'
                | b'C'
                | b'D'
                | b'E'
                | b'F'
                | b'G'
                | b'`'
                | b'd'
                | b'H'
                | b'f'
                | b'J'
                | b'K'
                | b's'
                | b'u'
        )
        || matches!(command, b'h' | b'l') && raw == b"?7";
    if supported {
        match command {
            b'm' => transcode_sgr(raw, &input[..=end], output, style),
            // Erase operations use the terminal's current background and
            // extend to its edge, not the source art's edge.  The parser has
            // already included the actual canvas cells in each frame, so
            // replaying these would leak a frame's background into the rest
            // of a wide terminal window.
            b'J' | b'K' => {}
            _ => output.extend_from_slice(&input[..=end]),
        }
    }
    end + 1
}

#[derive(Clone, Copy)]
struct AnimationStyle {
    foreground: u8,
    background: u8,
    bold: bool,
    blink: bool,
    inverse: bool,
}

impl Default for AnimationStyle {
    fn default() -> Self {
        Self {
            foreground: 7,
            background: 0,
            bold: false,
            blink: false,
            inverse: false,
        }
    }
}

fn transcode_sgr(
    raw: &[u8],
    original: &[u8],
    output: &mut Vec<u8>,
    style: &mut Option<AnimationStyle>,
) {
    let Some(parameters) = sgr_parameters(raw) else {
        // Colons and private SGR extensions are terminal-specific.  Preserve
        // them, but do not apply subsequent classic colours against a style we
        // can no longer know.
        output.extend_from_slice(original);
        *style = None;
        return;
    };

    if !parameters.iter().copied().all(is_classic_sgr) {
        // xterm 256/true-colour ANSI is already expressed in the terminal's
        // colour space.  Forward it intact (rick.txt relies on this), then wait
        // for a reset before resuming exact VGA palette conversion.
        output.extend_from_slice(original);
        *style = None;
        return;
    }

    if let Some(active_style) = style.as_mut() {
        for &parameter in &parameters {
            active_style.apply(parameter);
        }
        write_vga_style(output, *active_style);
        return;
    }

    let Some(reset) = parameters.iter().rposition(|&parameter| parameter == 0) else {
        output.extend_from_slice(original);
        return;
    };
    let mut restored = AnimationStyle::default();
    for &parameter in &parameters[reset + 1..] {
        restored.apply(parameter);
    }
    write_vga_style(output, restored);
    *style = Some(restored);
}

fn sgr_parameters(raw: &[u8]) -> Option<Vec<usize>> {
    if raw.is_empty() {
        return Some(vec![0]);
    }
    raw.split(|&byte| byte == b';')
        .map(|part| {
            if part.is_empty() {
                return Some(0);
            }
            std::str::from_utf8(part).ok()?.parse().ok()
        })
        .collect()
}

fn is_classic_sgr(parameter: usize) -> bool {
    matches!(
        parameter,
        0 | 1 | 5 | 6 | 7 | 22 | 25 | 27 | 30..=37 | 39 | 40..=47 | 49 | 90..=97 | 100..=107
    )
}

impl AnimationStyle {
    fn apply(&mut self, parameter: usize) {
        match parameter {
            0 => *self = Self::default(),
            1 => self.bold = true,
            5 | 6 => self.blink = true,
            7 => self.inverse = true,
            22 => self.bold = false,
            25 => self.blink = false,
            27 => self.inverse = false,
            30..=37 => self.foreground = (parameter - 30) as u8,
            39 => self.foreground = 7,
            40..=47 => self.background = (parameter - 40) as u8,
            49 => self.background = 0,
            90..=97 => {
                self.foreground = (parameter - 90 + 8) as u8;
                self.bold = false;
            }
            100..=107 => self.background = (parameter - 100 + 8) as u8,
            _ => unreachable!("classic SGR parameters were validated"),
        }
    }
}

fn write_vga_style(output: &mut Vec<u8>, style: AnimationStyle) {
    let mut foreground = style.foreground;
    let background = style.background;
    if style.bold && foreground < 8 {
        foreground += 8;
    }
    let (foreground, background) = if style.inverse {
        (background, foreground)
    } else {
        (foreground, background)
    };
    let foreground = VGA_PALETTE[usize::from(foreground)];
    let background = VGA_PALETTE[usize::from(background)];
    write!(
        output,
        "\x1b[38;2;{};{};{};48;2;{};{};{}m",
        foreground[0], foreground[1], foreground[2], background[0], background[1], background[2]
    )
    .expect("writing to a byte buffer cannot fail");
}

fn transmission_time(bytes: usize, baud: u64) -> Duration {
    // Match ANSI viewers that treat their labelled baud setting as playback
    // throughput. This makes 2X and 4X exact multiples of the 115200 preset.
    let nanoseconds = bytes as u128 * 1_000_000_000 / u128::from(baud);
    Duration::new(
        (nanoseconds / 1_000_000_000) as u64,
        (nanoseconds % 1_000_000_000) as u32,
    )
}

fn scale_duration(duration: Duration, baud: u64) -> Duration {
    let nanoseconds = duration
        .as_nanos()
        .saturating_mul(u128::from(DEFAULT_ANIMATION_BAUD))
        / u128::from(baud);
    let seconds = nanoseconds / 1_000_000_000;
    if seconds > u128::from(u64::MAX) {
        Duration::MAX
    } else {
        Duration::new(seconds as u64, (nanoseconds % 1_000_000_000) as u32)
    }
}

fn sleep_until(started: Instant, target: Duration) {
    if let Some(remaining) = target.checked_sub(started.elapsed()) {
        thread::sleep(remaining);
    }
}

/// Writes UTF-8 text cropped to a maximum character-column count.
pub fn write_screen_cropped<W: Write>(
    output: &mut W,
    screen: &Screen,
    columns: usize,
) -> io::Result<()> {
    write_screen_inner(output, screen, None, columns)
}

/// Writes UTF-8 text one character row at a time with a delay.
pub fn write_screen_slow<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
) -> io::Result<()> {
    if screen.raster.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "slow mode is not supported for RIPscrip graphics",
        ));
    }
    write_screen_inner(output, screen, Some(delay), screen.width)
}

/// Slowly writes UTF-8 text cropped to a maximum column count.
pub fn write_screen_slow_cropped<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Duration,
    columns: usize,
) -> io::Result<()> {
    if screen.raster.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "slow mode is not supported for RIPscrip graphics",
        ));
    }
    write_screen_inner(output, screen, Some(delay), columns)
}

fn write_screen_inner<W: Write>(
    output: &mut W,
    screen: &Screen,
    delay: Option<Duration>,
    columns: usize,
) -> io::Result<()> {
    if columns == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "UTF-8 crop width must be non-zero",
        ));
    }
    validate_text_screen(screen)?;
    let palette = screen.palette.unwrap_or(VGA_PALETTE);

    for (row_index, row) in screen.cells.chunks_exact(screen.width).enumerate() {
        let mut active_colors = None;
        for cell in row.iter().take(columns) {
            let colors = (cell.foreground & 0x0f, cell.background & 0x0f);
            if active_colors != Some(colors) {
                // Emit 24-bit SGR colors only when the pair changes, rather than
                // repeating an escape sequence before every character.
                let foreground = palette[usize::from(colors.0)];
                let background = palette[usize::from(colors.1)];
                write!(
                    output,
                    "\x1b[38;2;{};{};{};48;2;{};{};{}m",
                    foreground[0],
                    foreground[1],
                    foreground[2],
                    background[0],
                    background[1],
                    background[2]
                )?;
                active_colors = Some(colors);
            }
            // CP437 is a glyph table, not UTF-8-compatible text. Translate the
            // byte through the explicit table above before encoding as UTF-8.
            let mut encoded = [0_u8; 4];
            let character = CP437[usize::from(cell.character)].encode_utf8(&mut encoded);
            output.write_all(character.as_bytes())?;
        }
        output.write_all(b"\x1b[0m\r\n")?;
        if row_index + 1 < screen.height
            && let Some(delay) = delay
        {
            output.flush()?;
            thread::sleep(delay);
        }
    }
    output.flush()
}

fn validate_text_screen(screen: &Screen) -> io::Result<()> {
    if screen.raster.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "UTF-8 output is not useful for RIPscrip graphics; use --kitty or --output FILE",
        ));
    }
    if screen.cells.iter().any(|cell| cell.character > 255) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "UTF-8 output cannot represent XBin 512-character font glyphs",
        ));
    }
    if !screen.utf8_supported {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "UTF-8 output cannot represent a custom embedded bitmap font; use --kitty or --output FILE",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Animation, AnimationFrame, Cell};

    fn screen(cells: Vec<Cell>) -> Screen {
        Screen {
            width: cells.len(),
            height: 1,
            cells,
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        }
    }

    #[test]
    fn maps_cp437_to_utf8_with_true_color() {
        let mut output = Vec::new();
        write_screen(
            &mut output,
            &screen(vec![
                Cell {
                    character: 0x03,
                    foreground: 1,
                    background: 2,
                },
                Cell {
                    character: 0xdb,
                    foreground: 1,
                    background: 2,
                },
            ]),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\x1b[38;2;170;0;0;48;2;0;170;0m♥█\x1b[0m\r\n"
        );
    }

    #[test]
    fn relative_animation_preserves_the_existing_terminal() {
        let animation = Animation {
            frames: vec![AnimationFrame {
                screen: screen(vec![Cell {
                    character: 0x03,
                    foreground: 7,
                    background: 0,
                }]),
                source_bytes: 1,
                duration: None,
                utf8: false,
                data: b"\x1b[38;5;196m\x03".to_vec(),
            }],
            clear_on_finish: true,
        };
        let mut output = Vec::new();

        write_animation_at_baud(&mut output, &animation, u64::MAX).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("\x1b[0m\r\n\x1b[?2026h\x1b[0m"));
        assert!(output.contains("\x1b[38;5;196m"));
        assert!(output.contains('♥'));
        assert!(output.ends_with("\x1b[0m\r\n"));
        assert!(!output.contains("\x1b[2J"));
    }

    #[test]
    fn absolute_top_animation_clears_and_preserves_the_last_frame() {
        let animation = Animation {
            frames: vec![AnimationFrame {
                screen: screen(vec![Cell {
                    character: u16::from(b'X'),
                    foreground: 7,
                    background: 0,
                }]),
                source_bytes: 1,
                duration: None,
                utf8: false,
                data: b"\x1b[HX".to_vec(),
            }],
            clear_on_finish: true,
        };
        let mut output = Vec::new();

        write_animation_at_baud(&mut output, &animation, u64::MAX).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("\x1b[?2026h\x1b[2J\x1b[H\x1b[0m"));
        assert!(output.contains('X'));
        assert!(output.ends_with("\x1b[0m\x1b[1;1H\r\n"));
    }

    #[test]
    fn animation_maps_classic_sgr_to_the_vga_palette() {
        let animation = Animation {
            frames: vec![AnimationFrame {
                screen: screen(vec![Cell {
                    character: u16::from(b'X'),
                    foreground: 8,
                    background: 0,
                }]),
                source_bytes: 1,
                duration: None,
                utf8: false,
                data: b"\x1b[1;30mX".to_vec(),
            }],
            clear_on_finish: false,
        };
        let mut output = Vec::new();

        write_animation_at_baud(&mut output, &animation, u64::MAX).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\x1b[38;2;85;85;85;48;2;0;0;0mX"));
        assert!(!output.contains("\x1b[1;30m"));
    }

    #[test]
    fn animation_discards_terminal_wide_erase_sequences() {
        let mut output = Vec::new();
        let mut style = Some(AnimationStyle::default());

        transcode_frame(b"\x1b[2J\x1b[KX", &mut output, &mut style);

        assert_eq!(output, b"X");
    }

    #[test]
    fn animation_rates_are_source_bytes_per_second() {
        assert_eq!(transmission_time(1, 2_400), Duration::from_nanos(416_666));
        assert_eq!(transmission_time(4_608, 460_800), Duration::from_millis(10));
    }

    #[test]
    fn native_frame_durations_scale_from_1x() {
        let duration = Duration::from_millis(200);
        assert_eq!(
            scale_duration(duration, DEFAULT_ANIMATION_BAUD),
            Duration::from_millis(200)
        );
        assert_eq!(scale_duration(duration, 57_600), Duration::from_millis(400));
        assert_eq!(
            scale_duration(duration, 230_400),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn rejects_a_512_character_xbin_glyph() {
        let mut output = Vec::new();
        let error = write_screen(
            &mut output,
            &screen(vec![Cell {
                character: 256,
                foreground: 7,
                background: 0,
            }]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("512-character"));
        assert!(output.is_empty());
    }

    #[test]
    fn crops_each_utf8_row_to_the_requested_width() {
        let mut output = Vec::new();
        let screen = screen(
            b"ABCDEF"
                .iter()
                .map(|&character| Cell {
                    character: u16::from(character),
                    foreground: 7,
                    background: 0,
                })
                .collect(),
        );
        write_screen_cropped(&mut output, &screen, 3).unwrap();
        assert!(String::from_utf8(output).unwrap().contains("ABC\x1b[0m"));
    }

    #[test]
    fn rejects_a_zero_utf8_crop_width() {
        let error =
            write_screen_cropped(&mut Vec::new(), &screen(vec![Cell::default()]), 0).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_a_custom_embedded_bitmap_font() {
        let mut screen = screen(vec![Cell::default()]);
        screen.font = Some(vec![0; 256 * 16]);
        screen.utf8_supported = false;
        let mut output = Vec::new();
        let error = write_screen(&mut output, &screen).unwrap_err();
        assert!(error.to_string().contains("--kitty or --output"));
        assert!(output.is_empty());
    }

    #[test]
    fn rejects_raster_art_with_a_graphical_output_hint() {
        let mut screen = screen(Vec::new());
        screen.width = 80;
        screen.height = 22;
        screen.raster = Some(crate::ansi::Raster {
            width: 640,
            height: 350,
            pixels: vec![0; 640 * 350],
        });
        let error = write_screen(&mut Vec::new(), &screen).unwrap_err();
        assert!(error.to_string().contains("RIPscrip"));
        assert!(error.to_string().contains("--kitty or --output"));
    }

    #[test]
    fn slow_mode_rejects_raster_art() {
        let mut screen = screen(Vec::new());
        screen.width = 80;
        screen.height = 22;
        screen.raster = Some(crate::ansi::Raster {
            width: 640,
            height: 350,
            pixels: vec![0; 640 * 350],
        });
        let error = write_screen_slow(&mut Vec::new(), &screen, Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("slow mode"));
        assert!(error.to_string().contains("RIPscrip"));
    }
}
