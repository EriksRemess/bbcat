//! Playback for the line-framed format used by asciimation.co.nz.
//!
//! Each frame record starts with a positive duration measured in 100 ms ticks,
//! followed by exactly thirteen complete rows of ASCII art. It has no magic
//! header, so callers must opt in explicitly rather than guessing from a plain
//! text file.

use std::{
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};

const FRAME_ROWS: usize = 13;
const TICK: Duration = Duration::from_millis(100);

/// A parsed asciimation.co.nz-style frame stream.
#[derive(Clone, Debug)]
pub struct Asciimation {
    /// Frames in playback order.
    pub frames: Vec<AsciimationFrame>,
    /// Widest frame row, in terminal columns.
    pub width: usize,
}

/// One frame from an [`Asciimation`] stream.
#[derive(Clone, Debug)]
pub struct AsciimationFrame {
    duration: Duration,
    rows: Vec<String>,
}

impl AsciimationFrame {
    /// Returns how long the frame should remain visible.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the frame's thirteen display rows.
    pub fn rows(&self) -> &[String] {
        &self.rows
    }
}

/// Parses an explicit asciimation.co.nz-style text stream.
pub fn parse(data: &[u8]) -> Result<Asciimation, String> {
    let input =
        std::str::from_utf8(data).map_err(|_| "asciimation input must be UTF-8 or ASCII text")?;
    let mut lines: Vec<&str> = input
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let record_lines = FRAME_ROWS + 1;
    if lines.is_empty() || !lines.len().is_multiple_of(record_lines) {
        return Err(format!(
            "asciimation input must contain a duration and {FRAME_ROWS} rows per frame"
        ));
    }

    let mut width = 0;
    let mut frames = Vec::with_capacity(lines.len() / record_lines);
    for (index, record) in lines.chunks_exact(record_lines).enumerate() {
        let ticks = record[0]
            .parse::<u64>()
            .ok()
            .filter(|&ticks| ticks > 0)
            .ok_or_else(|| {
                format!(
                    "asciimation frame {} has an invalid duration {:?}",
                    index + 1,
                    record[0]
                )
            })?;
        let ticks = u32::try_from(ticks)
            .map_err(|_| format!("asciimation frame {} duration is too large", index + 1))?;
        let duration = TICK
            .checked_mul(ticks)
            .ok_or_else(|| format!("asciimation frame {} duration is too large", index + 1))?;
        let rows = record[1..]
            .iter()
            .map(|row| {
                if !row
                    .bytes()
                    .all(|byte| byte.is_ascii() && !byte.is_ascii_control())
                {
                    return Err(format!(
                        "asciimation frame {} contains a non-ASCII display character",
                        index + 1
                    ));
                }
                width = width.max(row.len());
                Ok((*row).to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        frames.push(AsciimationFrame { duration, rows });
    }
    Ok(Asciimation { frames, width })
}

/// Plays an asciimation stream to terminal output using its native timing.
pub fn write<W: Write>(output: &mut W, animation: &Asciimation) -> io::Result<()> {
    let started = Instant::now();
    let mut elapsed = Duration::ZERO;
    let mut buffer = Vec::new();
    for frame in &animation.frames {
        buffer.clear();
        // Keep each complete 13-row frame atomic. Clearing inside the
        // synchronized update prevents shorter subsequent rows leaving ghosts.
        buffer.extend_from_slice(b"\x1b[?2026h\x1b[2J\x1b[H\x1b[0m");
        for row in &frame.rows {
            buffer.extend_from_slice(row.as_bytes());
            buffer.extend_from_slice(b"\r\n");
        }
        buffer.extend_from_slice(b"\x1b[?2026l");
        output.write_all(&buffer)?;
        output.flush()?;
        elapsed = elapsed.saturating_add(frame.duration);
        let remaining = elapsed.saturating_sub(started.elapsed());
        if !remaining.is_zero() {
            thread::sleep(remaining);
        }
    }
    write!(output, "\x1b[0m\x1b[{};1H\r\n", FRAME_ROWS + 1)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thirteen_rows_and_duration_ticks() {
        let mut input = String::from("2\n");
        for _ in 0..FRAME_ROWS {
            input.push_str("x\n");
        }
        let animation = parse(input.as_bytes()).unwrap();
        assert_eq!(animation.frames.len(), 1);
        assert_eq!(animation.width, 1);
        assert_eq!(animation.frames[0].duration, Duration::from_millis(200));
    }

    #[test]
    fn rejects_malformed_records() {
        assert!(parse(b"1\nonly one row\n").is_err());
        let invalid_duration = format!("zero\n{}", "\n".repeat(FRAME_ROWS));
        assert!(parse(invalid_duration.as_bytes()).is_err());
    }
}
