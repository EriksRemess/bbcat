//! ANSI and plain DOS text decoding.
//!
//! ANSI art is a byte stream played into a virtual text terminal. Printable
//! bytes become CP437 character cells, while escape sequences move the cursor,
//! erase cells, or change the active colors. A DIZ/plain-text file follows the
//! same path but normally contains no escape sequences.

/// One character and color pair in a decoded text-art screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
    /// CP437 byte value, or 0..=511 for an XBin font with 512 glyphs.
    pub character: u16,
    /// Index into the screen's 16-color palette.
    pub foreground: u8,
    /// Index into the screen's 16-color palette.
    pub background: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: b' ' as u16,
            foreground: 7,
            background: 0,
        }
    }
}

/// A decoded character grid or indexed raster shared by all input formats.
#[derive(Clone, Debug)]
pub struct Screen {
    /// Character-grid dimensions used by text formats and terminal placement.
    pub width: usize,
    /// Number of character rows in the screen.
    pub height: usize,
    /// Row-major character cells. Raster formats leave this empty.
    pub cells: Vec<Cell>,
    // Graphical writers use these format-specific additions when the default
    // 8x16 VGA font and palette are not sufficient.
    pub(crate) glyph_width: usize,
    pub(crate) glyph_height: usize,
    pub(crate) font: Option<Vec<u8>>,
    pub(crate) palette: Option<[[u8; 3]; 16]>,
    pub(crate) utf8_supported: bool,
    pub(crate) raster: Option<Raster>,
}

impl Screen {
    /// Returns the cell at zero-based `column` and `row` coordinates.
    pub fn cell(&self, column: usize, row: usize) -> Option<&Cell> {
        let index = row.checked_mul(self.width)?.checked_add(column)?;
        (column < self.width && row < self.height)
            .then(|| self.cells.get(index))
            .flatten()
    }

    /// Returns the width and height of one character glyph in pixels.
    pub fn glyph_dimensions(&self) -> (usize, usize) {
        (self.glyph_width, self.glyph_height)
    }

    /// Returns the complete rendered width and height in pixels.
    pub fn pixel_dimensions(&self) -> Option<(usize, usize)> {
        if let Some(raster) = &self.raster {
            Some((raster.width, raster.height))
        } else {
            Some((
                self.width.checked_mul(self.glyph_width)?,
                self.height.checked_mul(self.glyph_height)?,
            ))
        }
    }

    /// Returns the 16-color RGB palette used by graphical output.
    pub fn palette(&self) -> [[u8; 3]; 16] {
        self.palette.unwrap_or(crate::VGA_PALETTE)
    }

    /// Returns glyph-major bitmap font data for character artwork.
    ///
    /// Each glyph contains [`Screen::glyph_dimensions`]'s height in bytes. The
    /// most-significant bit is the leftmost pixel. RIPscrip raster screens do
    /// not use a character font and return `None`.
    pub fn font(&self) -> Option<&[u8]> {
        if self.raster.is_some() {
            None
        } else if let Some(font) = &self.font {
            Some(font)
        } else {
            Some(crate::font::glyphs())
        }
    }

    /// Returns the indexed pixel raster for RIPscrip artwork.
    pub fn raster(&self) -> Option<&Raster> {
        self.raster.as_ref()
    }
}

/// An indexed-color pixel canvas produced by a raster format such as RIPscrip.
#[derive(Clone, Debug)]
pub struct Raster {
    /// Pixel width of the canvas.
    pub width: usize,
    /// Pixel height of the canvas.
    pub height: usize,
    /// Row-major palette indexes, one byte per pixel.
    pub pixels: Vec<u8>,
}

pub(crate) const MAX_CELLS: usize = 10_000_000;

pub(crate) struct ParsedAnsi {
    pub screen: Screen,
    pub frames: Vec<ParsedFrame>,
    pub clear_on_finish: bool,
}

pub(crate) struct ParsedFrame {
    pub screen: Screen,
    pub source_bytes: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum EscapeEvent {
    None,
    Home,
    ClearDisplay,
    Erase,
}

#[derive(Clone, Copy)]
struct Style {
    foreground: u8,
    background: u8,
    bold: bool,
    blink: bool,
    inverse: bool,
}

impl Default for Style {
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

struct Parser {
    width: usize,
    cells: Vec<Cell>,
    x: usize,
    y: usize,
    saved: (usize, usize),
    style: Style,
    ice_colors: bool,
    auto_wrap: bool,
    // Real terminals defer wrapping until the next printable byte. Keeping this
    // sentinel prevents a CR/LF immediately after the last column from wrapping
    // once implicitly and once explicitly.
    pending_wrap: bool,
    max_written_row: Option<usize>,
}

#[cfg(test)]
pub fn parse(
    bytes: &[u8],
    width: usize,
    declared_height: Option<usize>,
    ice_colors: bool,
) -> Result<Screen, String> {
    Ok(parse_with_animation(bytes, width, declared_height, ice_colors)?.screen)
}

pub(crate) fn parse_with_animation(
    bytes: &[u8],
    width: usize,
    declared_height: Option<usize>,
    ice_colors: bool,
) -> Result<ParsedAnsi, String> {
    // A SAUCE height is useful for preserving intentionally blank rows. Without
    // one, the backing grid grows only when cursor movement or output reaches it.
    let initial_cells = width
        .checked_mul(declared_height.unwrap_or(1).max(1))
        .ok_or_else(canvas_too_large)?;
    if initial_cells > MAX_CELLS {
        return Err(canvas_too_large());
    }
    let mut parser = Parser {
        width,
        cells: vec![Cell::default(); initial_cells],
        x: 0,
        y: 0,
        saved: (0, 0),
        style: Style::default(),
        ice_colors,
        auto_wrap: true,
        pending_wrap: false,
        max_written_row: None,
    };

    let mut frames = Vec::new();
    let mut stored_frame_cells = 0_usize;
    let mut frame_start = 0_usize;
    let mut dirty = false;
    let mut clear_on_finish = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                let event = escape_event(&bytes[index..]);
                if matches!(event, EscapeEvent::Home | EscapeEvent::ClearDisplay) && dirty {
                    let screen = parser.snapshot(declared_height)?;
                    stored_frame_cells = stored_frame_cells
                        .checked_add(screen.cells.len())
                        .ok_or_else(animation_too_large)?;
                    if stored_frame_cells > MAX_CELLS {
                        return Err(animation_too_large());
                    }
                    frames.push(ParsedFrame {
                        screen,
                        source_bytes: index.saturating_sub(frame_start).max(1),
                        data: bytes[frame_start..index].to_vec(),
                    });
                    frame_start = index;
                    dirty = false;
                }
                index += parser.escape(&bytes[index..]);
                match event {
                    EscapeEvent::Erase => {
                        dirty = true;
                        clear_on_finish = false;
                    }
                    EscapeEvent::None | EscapeEvent::Home => {}
                    EscapeEvent::ClearDisplay => clear_on_finish = true,
                }
            }
            b'\r' => {
                parser.x = 0;
                parser.pending_wrap = false;
                index += 1;
            }
            b'\n' => {
                parser.y = parser.y.saturating_add(1);
                parser.pending_wrap = false;
                index += 1;
            }
            b'\t' => {
                let next = ((parser.x / 8) + 1) * 8;
                parser.x = next.min(width.saturating_sub(1));
                parser.pending_wrap = false;
                index += 1;
            }
            0x1a => index += 1,
            character => {
                parser.put(character)?;
                dirty = true;
                clear_on_finish = false;
                index += 1;
            }
        }
    }

    if dirty {
        let screen = parser.snapshot(declared_height)?;
        stored_frame_cells = stored_frame_cells
            .checked_add(screen.cells.len())
            .ok_or_else(animation_too_large)?;
        if stored_frame_cells > MAX_CELLS {
            return Err(animation_too_large());
        }
        frames.push(ParsedFrame {
            screen,
            source_bytes: bytes.len().saturating_sub(frame_start).max(1),
            data: bytes[frame_start..].to_vec(),
        });
    }

    // Repeated homes/full clears delimit ansimation frames. A lone snapshot is
    // merely the ordinary final state of a static ANSI file and is discarded.
    if frames.len() >= 2 {
        Ok(ParsedAnsi {
            screen: frames.last().unwrap().screen.clone(),
            frames,
            clear_on_finish,
        })
    } else {
        Ok(ParsedAnsi {
            screen: parser.snapshot(declared_height)?,
            frames: Vec::new(),
            clear_on_finish: false,
        })
    }
}

impl Parser {
    fn snapshot(&self, declared_height: Option<usize>) -> Result<Screen, String> {
        let measured = self.max_written_row.map_or(1, |row| row + 1);
        let height = declared_height.unwrap_or(measured).max(measured).max(1);
        let required = self
            .width
            .checked_mul(height)
            .ok_or_else(canvas_too_large)?;
        if required > MAX_CELLS {
            return Err(canvas_too_large());
        }
        let mut cells = self.cells.clone();
        cells.resize(required, Cell::default());
        cells.truncate(required);
        Ok(Screen {
            width: self.width,
            height,
            cells,
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        })
    }

    fn ensure_row(&mut self, row: usize) -> Result<(), String> {
        let required = row
            .checked_add(1)
            .and_then(|rows| rows.checked_mul(self.width))
            .ok_or_else(canvas_too_large)?;
        if required > MAX_CELLS {
            return Err(canvas_too_large());
        }
        if self.cells.len() < required {
            self.cells.resize(required, Cell::default());
        }
        Ok(())
    }

    fn put(&mut self, character: u8) -> Result<(), String> {
        if self.pending_wrap {
            self.x = 0;
            self.y = self.y.saturating_add(1);
            self.pending_wrap = false;
        }
        self.ensure_row(self.y)?;
        // ANSI stores effects as state, but Screen stores the final color pair
        // on every cell. In iCE mode the blink bit is repurposed as background
        // intensity, matching how late DOS art commonly used 16 backgrounds.
        let (mut foreground, mut background) = (self.style.foreground, self.style.background);
        if self.style.bold && foreground < 8 {
            foreground += 8;
        }
        if self.style.blink && self.ice_colors && background < 8 {
            background += 8;
        }
        if self.style.inverse {
            (foreground, background) = (background, foreground);
        }
        self.cells[self.y * self.width + self.x] = Cell {
            character: u16::from(character),
            foreground,
            background,
        };
        self.max_written_row = Some(self.max_written_row.map_or(self.y, |row| row.max(self.y)));

        if self.x + 1 == self.width {
            self.pending_wrap = self.auto_wrap;
        } else {
            self.x += 1;
        }
        Ok(())
    }

    fn escape(&mut self, bytes: &[u8]) -> usize {
        if bytes.len() < 2 {
            return 1;
        }
        match bytes[1] {
            b'[' => {
                // A Control Sequence Introducer ends at its first final byte
                // (0x40..=0x7e); bytes before it are parameters/intermediates.
                let Some(relative_end) = bytes[2..]
                    .iter()
                    .position(|byte| (0x40..=0x7e).contains(byte))
                else {
                    return bytes.len();
                };
                let end = relative_end + 2;
                self.csi(&bytes[2..end], bytes[end]);
                end + 1
            }
            b'7' => {
                self.saved = (self.x, self.y);
                2
            }
            b'8' => {
                (self.x, self.y) = self.saved;
                self.pending_wrap = false;
                2
            }
            b'c' => {
                self.x = 0;
                self.y = 0;
                self.style = Style::default();
                self.pending_wrap = false;
                2
            }
            _ => 2,
        }
    }

    fn csi(&mut self, raw: &[u8], command: u8) {
        let private = raw.first() == Some(&b'?');
        let raw = if private { &raw[1..] } else { raw };
        // Parameters are decimal and semicolon-separated. Empty/default values
        // are interpreted by the individual command below.
        let parameters: Vec<usize> = if raw.is_empty() {
            Vec::new()
        } else {
            raw.split(|&byte| byte == b';')
                .map(|part| {
                    part.iter().fold(0_usize, |value, &digit| {
                        if digit.is_ascii_digit() {
                            value
                                .saturating_mul(10)
                                .saturating_add((digit - b'0') as usize)
                        } else {
                            value
                        }
                    })
                })
                .collect()
        };
        let amount = || parameters.first().copied().unwrap_or(1).max(1);

        if command != b'm' && self.pending_wrap {
            self.x = 0;
            self.y = self.y.saturating_add(1);
        }
        if command != b'm' {
            self.pending_wrap = false;
        }
        match command {
            b'm' => self.sgr(&parameters),
            b'A' => self.y = self.y.saturating_sub(amount()),
            b'B' => self.y = self.y.saturating_add(amount()),
            b'C' => {
                let column = self.x.saturating_add(amount());
                if column >= self.width {
                    self.x = self.width - 1;
                    self.pending_wrap = self.auto_wrap;
                } else {
                    self.x = column;
                }
            }
            b'D' => self.x = self.x.saturating_sub(amount()),
            b'E' => {
                self.y = self.y.saturating_add(amount());
                self.x = 0;
            }
            b'F' => {
                self.y = self.y.saturating_sub(amount());
                self.x = 0;
            }
            b'G' | b'`' => self.x = amount().saturating_sub(1).min(self.width - 1),
            b'd' => self.y = amount().saturating_sub(1),
            b'H' | b'f' => {
                self.y = parameters.first().copied().unwrap_or(1).max(1) - 1;
                self.x = parameters.get(1).copied().unwrap_or(1).max(1) - 1;
                self.x = self.x.min(self.width - 1);
            }
            b'J' => self.erase_display(parameters.first().copied().unwrap_or(0)),
            b'K' => self.erase_line(parameters.first().copied().unwrap_or(0)),
            b's' => self.saved = (self.x, self.y),
            b'u' => (self.x, self.y) = self.saved,
            b'h' if private && parameters.contains(&7) => self.auto_wrap = true,
            b'l' if private && parameters.contains(&7) => self.auto_wrap = false,
            _ => {}
        }
    }

    fn sgr(&mut self, parameters: &[usize]) {
        // SGR (the `m` command) changes style only; a later printable byte turns
        // that style into the concrete foreground/background stored in a Cell.
        let parameters = if parameters.is_empty() {
            &[0][..]
        } else {
            parameters
        };
        for &parameter in parameters {
            match parameter {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                5 | 6 => self.style.blink = true,
                7 => self.style.inverse = true,
                22 => self.style.bold = false,
                25 => self.style.blink = false,
                27 => self.style.inverse = false,
                30..=37 => self.style.foreground = (parameter - 30) as u8,
                39 => self.style.foreground = 7,
                40..=47 => self.style.background = (parameter - 40) as u8,
                49 => self.style.background = 0,
                90..=97 => {
                    self.style.foreground = (parameter - 90 + 8) as u8;
                    self.style.bold = false;
                }
                100..=107 => self.style.background = (parameter - 100 + 8) as u8,
                _ => {}
            }
        }
    }

    fn erase_line(&mut self, mode: usize) {
        if self.ensure_row(self.y).is_err() {
            return;
        }
        let (start, end) = match mode {
            1 => (0, self.x + 1),
            2 => (0, self.width),
            _ => (self.x, self.width),
        };
        let row = self.y * self.width;
        let blank = self.blank_cell();
        self.cells[row + start..row + end].fill(blank);
    }

    fn erase_display(&mut self, mode: usize) {
        if self.ensure_row(self.y).is_err() {
            return;
        }
        let cursor = self.y * self.width + self.x;
        let blank = self.blank_cell();
        match mode {
            1 => self.cells[..=cursor].fill(blank),
            2 | 3 => self.cells.fill(blank),
            _ => self.cells[cursor..].fill(blank),
        }
    }

    fn blank_cell(&self) -> Cell {
        let (mut foreground, mut background) = (self.style.foreground, self.style.background);
        if self.style.bold && foreground < 8 {
            foreground += 8;
        }
        if self.style.blink && self.ice_colors && background < 8 {
            background += 8;
        }
        if self.style.inverse {
            (foreground, background) = (background, foreground);
        }
        Cell {
            character: b' ' as u16,
            foreground,
            background,
        }
    }
}

fn escape_event(bytes: &[u8]) -> EscapeEvent {
    if bytes.get(1) != Some(&b'[') {
        return EscapeEvent::None;
    }
    let Some(relative_end) = bytes[2..]
        .iter()
        .position(|byte| (0x40..=0x7e).contains(byte))
    else {
        return EscapeEvent::None;
    };
    let end = relative_end + 2;
    let command = bytes[end];
    let raw = bytes[2..end].strip_prefix(b"?").unwrap_or(&bytes[2..end]);
    let parameters: Vec<usize> = raw
        .split(|&byte| byte == b';')
        .map(|part| {
            part.iter().fold(0_usize, |value, &digit| {
                if digit.is_ascii_digit() {
                    value
                        .saturating_mul(10)
                        .saturating_add((digit - b'0') as usize)
                } else {
                    value
                }
            })
        })
        .collect();

    match command {
        b'H' | b'f'
            if parameters.first().copied().unwrap_or(1).max(1) == 1
                && parameters.get(1).copied().unwrap_or(1).max(1) == 1 =>
        {
            EscapeEvent::Home
        }
        b'J' if matches!(parameters.first().copied().unwrap_or(0), 2 | 3) => {
            EscapeEvent::ClearDisplay
        }
        b'J' | b'K' => EscapeEvent::Erase,
        _ => EscapeEvent::None,
    }
}

fn canvas_too_large() -> String {
    format!("ANSI canvas exceeds the {MAX_CELLS} cell safety limit")
}

fn animation_too_large() -> String {
    format!("ANSI animation exceeds the {MAX_CELLS} stored-cell safety limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_after_last_column_does_not_double_wrap() {
        let screen = parse(b"1234\r\nX", 4, None, false).unwrap();
        assert_eq!(screen.height, 2);
        assert_eq!(screen.cells[4].character, u16::from(b'X'));
    }

    #[test]
    fn sgr_preserves_a_pending_wrap() {
        let screen = parse(b"1234\x1b[31mX", 4, None, false).unwrap();
        assert_eq!(screen.height, 2);
        assert_eq!(screen.cells[4].character, u16::from(b'X'));
        assert_eq!(screen.cells[4].foreground, 1);
    }

    #[test]
    fn cursor_forward_can_reach_the_wrap_sentinel() {
        let screen = parse(b"\x1b[4CX", 4, None, false).unwrap();
        assert_eq!(screen.height, 2);
        assert_eq!(screen.cells[4].character, u16::from(b'X'));
    }

    #[test]
    fn cursor_forward_is_applied_after_a_pending_wrap() {
        let screen = parse(b"1234\x1b[2CX", 4, None, false).unwrap();
        assert_eq!(screen.height, 2);
        assert_eq!(screen.cells[6].character, u16::from(b'X'));
    }

    #[test]
    fn bold_selects_bright_foreground() {
        let screen = parse(b"\x1b[1;31mX", 1, None, false).unwrap();
        assert_eq!(screen.cells[0].foreground, 9);
    }

    #[test]
    fn sauce_ice_mode_turns_blink_into_bright_background() {
        let screen = parse(b"\x1b[5;44mX", 1, None, true).unwrap();
        assert_eq!(screen.cells[0].background, 12);
    }

    #[test]
    fn cursor_position_overwrites_cells() {
        let screen = parse(b"abc\x1b[1;2HZ", 4, None, false).unwrap();
        assert_eq!(screen.cells[1].character, u16::from(b'Z'));
    }

    #[test]
    fn cp437_house_is_printable() {
        let screen = parse(b"\x7f", 1, None, false).unwrap();
        assert_eq!(screen.cells[0].character, 0x7f);
    }

    #[test]
    fn cp437_control_range_glyphs_are_printable() {
        let screen = parse(b"\x03\x16", 2, None, false).unwrap();
        assert_eq!(screen.cells[0].character, 0x03);
        assert_eq!(screen.cells[1].character, 0x16);
    }

    #[test]
    fn erase_uses_the_active_background() {
        let screen = parse(b"abc\x1b[44m\x1b[2K", 4, None, false).unwrap();
        assert!(screen.cells.iter().all(|cell| cell.background == 4));
    }

    #[test]
    fn rejects_excessive_declared_dimensions_before_allocating() {
        let error = parse(b"", 1000, Some(65_535), false).unwrap_err();
        assert!(error.contains("safety limit"));
    }

    #[test]
    fn rejects_cursor_movement_beyond_the_canvas_limit() {
        let error = parse(b"\x1b[99999999BX", 80, None, false).unwrap_err();
        assert!(error.contains("safety limit"));
    }
}
