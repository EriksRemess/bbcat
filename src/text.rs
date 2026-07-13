use std::io::{self, Write};

use crate::{Screen, png::VGA_PALETTE};

const CP437: [char; 256] = [
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

pub fn write_screen<W: Write>(output: &mut W, screen: &Screen) -> io::Result<()> {
    if screen.cells.iter().any(|cell| cell.character > 255) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "UTF-8 output cannot represent XBin 512-character font glyphs",
        ));
    }
    let palette = screen.palette.unwrap_or(VGA_PALETTE);

    for row in screen.cells.chunks_exact(screen.width) {
        let mut active_colors = None;
        for cell in row {
            let colors = (cell.foreground & 0x0f, cell.background & 0x0f);
            if active_colors != Some(colors) {
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
            let mut encoded = [0_u8; 4];
            let character = CP437[usize::from(cell.character)].encode_utf8(&mut encoded);
            output.write_all(character.as_bytes())?;
        }
        output.write_all(b"\x1b[0m\r\n")?;
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cell;

    fn screen(cells: Vec<Cell>) -> Screen {
        Screen {
            width: cells.len(),
            height: 1,
            cells,
            glyph_height: 16,
            font: None,
            palette: None,
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
}
