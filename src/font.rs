use std::sync::OnceLock;

const ENCODED: &str = include_str!("font_data.b64");
const ENCODED_8X8: &str = include_str!("bgi_fonts/cp437_f08.b64");

pub fn glyphs() -> &'static [u8] {
    static GLYPHS: OnceLock<Vec<u8>> = OnceLock::new();
    GLYPHS.get_or_init(|| {
        let bytes = decode(ENCODED.trim());
        assert_eq!(
            bytes.len(),
            256 * 16,
            "embedded VGA font has the wrong size"
        );
        bytes
    })
}

pub(crate) fn glyphs_8x8() -> &'static [u8] {
    static GLYPHS: OnceLock<Vec<u8>> = OnceLock::new();
    GLYPHS.get_or_init(|| {
        let bytes = decode(ENCODED_8X8.trim());
        assert_eq!(
            bytes.len(),
            256 * 8,
            "embedded VGA 8x8 font has the wrong size"
        );
        bytes
    })
}

pub(crate) fn decode(input: &str) -> Vec<u8> {
    let encoded: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let mut output = Vec::with_capacity(encoded.len() / 4 * 3);
    for group in encoded.chunks(4) {
        if group.len() != 4 {
            break;
        }
        let value = (u32::from(six_bits(group[0])) << 18)
            | (u32::from(six_bits(group[1])) << 12)
            | (u32::from(six_bits(group[2])) << 6)
            | u32::from(six_bits(group[3]));
        output.push((value >> 16) as u8);
        if group[2] != b'=' {
            output.push((value >> 8) as u8);
        }
        if group[3] != b'=' {
            output.push(value as u8);
        }
    }
    output
}

fn six_bits(byte: u8) -> u8 {
    match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        b'=' => 0,
        _ => panic!("invalid base64 in embedded VGA font"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_has_all_cp437_glyphs() {
        assert_eq!(glyphs().len(), 4096);
        assert!(
            glyphs()[0xdb * 16..0xdc * 16]
                .iter()
                .all(|&row| row == 0xff)
        );
        assert_eq!(glyphs_8x8().len(), 2048);
        assert!(
            glyphs_8x8()[0xdb * 8..0xdc * 8]
                .iter()
                .all(|&row| row == 0xff)
        );
    }
}
