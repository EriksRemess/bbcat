use crate::ansi::{Cell, MAX_CELLS, Screen};

const VERSION_LENGTH: usize = 1;
const PALETTE_LENGTH: usize = 64 * 3;
const FONT_LENGTH: usize = 256 * 16;
const HEADER_LENGTH: usize = VERSION_LENGTH + PALETTE_LENGTH + FONT_LENGTH;
const WIDTH: usize = 80;
const ROW_LENGTH: usize = WIDTH * 2;
const COLOR_INDICES: [usize; 16] = [0, 1, 2, 3, 4, 5, 20, 7, 56, 57, 58, 59, 60, 61, 62, 63];

pub fn is_adf(data: &[u8]) -> bool {
    data.len() >= HEADER_LENGTH + ROW_LENGTH
        && data[0] == 1
        && data[1..VERSION_LENGTH + PALETTE_LENGTH]
            .iter()
            .all(|&component| component <= 63)
        && (data.len() - HEADER_LENGTH).is_multiple_of(ROW_LENGTH)
}

pub fn parse(data: &[u8], width_override: Option<usize>) -> Result<Screen, String> {
    if data.len() < HEADER_LENGTH {
        return Err("truncated ADF header".to_owned());
    }
    if data[0] != 1 {
        return Err(format!("unsupported ADF version {}", data[0]));
    }
    if let Some(width) = width_override
        && width != WIDTH
    {
        return Err(format!(
            "width override {width} does not match the ADF width {WIDTH}"
        ));
    }

    let palette_data = &data[VERSION_LENGTH..VERSION_LENGTH + PALETTE_LENGTH];
    if let Some(&component) = palette_data.iter().find(|&&component| component > 63) {
        return Err(format!(
            "invalid ADF palette component {component}; expected a 6-bit value"
        ));
    }
    let mut palette = [[0_u8; 3]; 16];
    for (color, &index) in palette.iter_mut().zip(&COLOR_INDICES) {
        let offset = index * 3;
        for (output, &component) in color.iter_mut().zip(&palette_data[offset..offset + 3]) {
            *output = (component << 2) | (component >> 4);
        }
    }

    let font_start = VERSION_LENGTH + PALETTE_LENGTH;
    let font = data[font_start..HEADER_LENGTH].to_vec();
    let utf8_supported = font.as_slice() == crate::font::glyphs();
    let screen_data = &data[HEADER_LENGTH..];
    if screen_data.is_empty() {
        return Err("ADF contains no screen data".to_owned());
    }
    if !screen_data.len().is_multiple_of(ROW_LENGTH) {
        return Err(format!(
            "invalid ADF screen data length {}; expected complete 80-column rows",
            screen_data.len()
        ));
    }
    let cell_count = screen_data.len() / 2;
    if cell_count > MAX_CELLS {
        return Err(format!(
            "ADF canvas exceeds the {MAX_CELLS} cell safety limit"
        ));
    }
    let cells = screen_data
        .chunks_exact(2)
        .map(|pair| Cell {
            character: u16::from(pair[0]),
            foreground: pair[1] & 0x0f,
            background: pair[1] >> 4,
        })
        .collect();

    Ok(Screen {
        width: WIDTH,
        height: cell_count / WIDTH,
        cells,
        glyph_height: 16,
        font: Some(font),
        palette: Some(palette),
        utf8_supported,
        raster: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut data = vec![0_u8; HEADER_LENGTH + ROW_LENGTH];
        data[0] = 1;
        data[1 + 63 * 3..1 + 64 * 3].copy_from_slice(&[63, 42, 21]);
        data[HEADER_LENGTH] = 0xdb;
        data[HEADER_LENGTH + 1] = 0xf1;
        data
    }

    #[test]
    fn decodes_palette_font_and_cells() {
        let data = fixture();
        assert!(is_adf(&data));
        let screen = parse(&data, None).unwrap();
        assert_eq!((screen.width, screen.height), (80, 1));
        assert_eq!(screen.font.as_ref().unwrap().len(), FONT_LENGTH);
        assert!(!screen.utf8_supported);
        assert_eq!(screen.palette.unwrap()[15], [255, 170, 85]);
        assert_eq!(screen.cells[0].character, 0xdb);
        assert_eq!(screen.cells[0].foreground, 1);
        assert_eq!(screen.cells[0].background, 15);
    }

    #[test]
    fn rejects_invalid_fields() {
        let mut data = fixture();
        data[0] = 2;
        assert!(parse(&data, None).unwrap_err().contains("version 2"));

        data[0] = 1;
        data[1] = 64;
        assert!(parse(&data, None).unwrap_err().contains("palette"));

        assert!(parse(&fixture(), Some(79)).unwrap_err().contains("width"));
        assert!(
            parse(&fixture()[..HEADER_LENGTH], None)
                .unwrap_err()
                .contains("no screen data")
        );
        assert!(
            parse(&fixture()[..HEADER_LENGTH + 2], None)
                .unwrap_err()
                .contains("complete 80-column rows")
        );
    }
}
