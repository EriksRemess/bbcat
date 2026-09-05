//! TundraDraw TUNDRA24 decoding.
//!
//! TundraDraw stores CP437 character cells in a stream. Ordinary bytes use the
//! active colors, while control records move the cursor or replace the active
//! 24-bit foreground and background colors.

use std::mem::size_of;

use crate::{Cell, Screen, ansi::MAX_CELLS};

const SIGNATURE: &[u8; 9] = b"\x18TUNDRA24";
const POSITION: u8 = 1;
const FOREGROUND: u8 = 2;
const BACKGROUND: u8 = 4;
const BOTH_COLORS: u8 = 6;
const DEFAULT_WIDTH: usize = 80;
const TUNDRA_CELL_BYTES: usize = size_of::<Cell>() + size_of::<([u8; 3], [u8; 3])>();
// Keep TundraDraw's two parallel cell buffers within the same approximate
// memory budget as MAX_CELLS classic ANSI cells.
const MAX_TUNDRA_CELLS: usize = MAX_CELLS * size_of::<Cell>() / TUNDRA_CELL_BYTES;

pub(crate) fn is_tundra(data: &[u8]) -> bool {
    data.starts_with(SIGNATURE)
}

pub(crate) fn parse(data: &[u8], width: Option<usize>) -> Result<Screen, String> {
    if !is_tundra(data) {
        return Err("invalid TundraDraw TUNDRA24 header".to_owned());
    }
    let width = width.unwrap_or(DEFAULT_WIDTH);
    if width == 0 {
        return Err("TundraDraw canvas width must be non-zero".to_owned());
    }
    if width > MAX_TUNDRA_CELLS {
        return Err(format!(
            "TundraDraw canvas exceeds the {MAX_TUNDRA_CELLS}-cell safety limit"
        ));
    }

    let mut cells = vec![Cell::default(); width];
    let mut colors = vec![([0; 3], [0; 3]); width];
    let (mut row, mut column) = (0_usize, 0_usize);
    let (mut foreground, mut background) = ([0_u8; 3], [0_u8; 3]);
    let mut offset = SIGNATURE.len();

    while offset < data.len() {
        if column == width {
            row = row
                .checked_add(1)
                .ok_or_else(|| "TundraDraw row coordinate overflow".to_owned())?;
            column = 0;
        } else if column > width {
            return Err(format!(
                "TundraDraw column {column} is outside the {width}-column canvas"
            ));
        }

        let control = data[offset];
        if control == POSITION {
            let record = data
                .get(offset + 1..offset + 9)
                .ok_or_else(|| "truncated TundraDraw position record".to_owned())?;
            row = u32::from_be_bytes(record[..4].try_into().unwrap()) as usize;
            column = u32::from_be_bytes(record[4..].try_into().unwrap()) as usize;
            validate_position(row, column, width)?;
            offset += 9;
            continue;
        }

        let character = match control {
            FOREGROUND | BACKGROUND => {
                let record = data
                    .get(offset + 1..offset + 6)
                    .ok_or_else(|| "truncated TundraDraw color record".to_owned())?;
                let color = [record[2], record[3], record[4]];
                if control == FOREGROUND {
                    foreground = color;
                } else {
                    background = color;
                }
                offset += 6;
                record[0]
            }
            BOTH_COLORS => {
                let record = data
                    .get(offset + 1..offset + 10)
                    .ok_or_else(|| "truncated TundraDraw dual-color record".to_owned())?;
                foreground = [record[2], record[3], record[4]];
                background = [record[6], record[7], record[8]];
                offset += 10;
                record[0]
            }
            character => {
                offset += 1;
                character
            }
        };

        let required = row
            .checked_add(1)
            .and_then(|rows| rows.checked_mul(width))
            .ok_or_else(|| "TundraDraw canvas dimensions overflow".to_owned())?;
        if required > MAX_TUNDRA_CELLS {
            return Err(format!(
                "TundraDraw canvas exceeds the {MAX_TUNDRA_CELLS}-cell safety limit"
            ));
        }
        if required > cells.len() {
            cells.resize(required, Cell::default());
            colors.resize(required, ([0; 3], [0; 3]));
        }
        let index = row * width + column;
        // These byte values identify records when they occur at stream level.
        // TundraDraw's reference rendering leaves them blank as glyph values.
        if !matches!(character, POSITION | FOREGROUND | BACKGROUND | BOTH_COLORS) {
            cells[index].character = u16::from(character);
            colors[index] = (foreground, background);
            column += 1;
        }
    }

    let height = cells.len() / width;
    Ok(Screen {
        width,
        height,
        cells,
        glyph_width: 8,
        glyph_height: 16,
        font: None,
        palette: None,
        true_colors: Some(colors),
        utf8_supported: true,
        raster: None,
    })
}

fn validate_position(row: usize, column: usize, width: usize) -> Result<(), String> {
    if column > width {
        return Err(format!(
            "TundraDraw column {column} is outside the {width}-column canvas"
        ));
    }
    let cells = row
        .checked_mul(width)
        .and_then(|cells| cells.checked_add(column.min(width.saturating_sub(1))))
        .ok_or_else(|| "TundraDraw position overflow".to_owned())?;
    if cells >= MAX_TUNDRA_CELLS {
        return Err(format!(
            "TundraDraw position exceeds the {MAX_TUNDRA_CELLS}-cell safety limit"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_cells_and_colors_when_drawing_on_an_earlier_row() {
        let mut data = SIGNATURE.to_vec();
        for (row, character, rgb) in [(2_u32, b'A', [1, 2, 3]), (0, b'B', [4, 5, 6])] {
            data.push(POSITION);
            data.extend_from_slice(&row.to_be_bytes());
            data.extend_from_slice(&0_u32.to_be_bytes());
            data.extend_from_slice(&[FOREGROUND, character, 0]);
            data.extend_from_slice(&rgb);
        }
        let screen = parse(&data, Some(2)).unwrap();
        assert_eq!(screen.height, 3);
        assert_eq!(screen.cell(0, 0).unwrap().character, u16::from(b'B'));
        assert_eq!(screen.cell(0, 2).unwrap().character, u16::from(b'A'));
        assert_eq!(screen.cell_colors(0, 0), Some(([4, 5, 6], [0; 3])));
        assert_eq!(screen.cell_colors(0, 2), Some(([1, 2, 3], [0; 3])));
    }

    #[test]
    fn decodes_positions_and_inherited_true_colors() {
        let mut data = SIGNATURE.to_vec();
        data.extend_from_slice(&[BOTH_COLORS, b'A', 0, 1, 2, 3, 0, 4, 5, 6, b'B']);
        data.extend_from_slice(&[
            POSITION, 0, 0, 0, 1, 0, 0, 0, 2, FOREGROUND, b'C', 0, 7, 8, 9,
        ]);

        let screen = parse(&data, Some(4)).unwrap();
        assert_eq!((screen.width, screen.height), (4, 2));
        assert_eq!(screen.cell(0, 0).unwrap().character, u16::from(b'A'));
        assert_eq!(screen.cell(1, 0).unwrap().character, u16::from(b'B'));
        assert_eq!(screen.cell(2, 1).unwrap().character, u16::from(b'C'));
        assert_eq!(screen.cell_colors(1, 0), Some(([1, 2, 3], [4, 5, 6])));
        assert_eq!(screen.cell_colors(2, 1), Some(([7, 8, 9], [4, 5, 6])));
    }

    #[test]
    fn rejects_bad_headers_records_and_coordinates() {
        assert!(parse(b"not tundra", None).is_err());
        assert!(parse(b"\x18TUNDRA24\x02", None).is_err());

        let mut position = SIGNATURE.to_vec();
        position.extend_from_slice(&[POSITION, 0, 0, 0, 0, 0, 0, 0, 81]);
        assert!(parse(&position, Some(80)).is_err());
    }

    #[test]
    fn rejects_positions_above_the_cell_budget() {
        let mut data = SIGNATURE.to_vec();
        data.push(POSITION);
        data.extend_from_slice(&(MAX_TUNDRA_CELLS as u32).to_be_bytes());
        data.extend_from_slice(&0_u32.to_be_bytes());
        assert!(parse(&data, Some(1)).is_err());
    }

    #[test]
    fn skipped_control_characters_do_not_advance_the_cursor() {
        let mut data = SIGNATURE.to_vec();
        data.extend_from_slice(&[FOREGROUND, POSITION, 0, 1, 2, 3, b'A']);

        let screen = parse(&data, Some(2)).unwrap();
        assert_eq!(screen.cell(0, 0).unwrap().character, u16::from(b'A'));
        assert_eq!(screen.cell(1, 0).unwrap().character, u16::from(b' '));
    }
}
