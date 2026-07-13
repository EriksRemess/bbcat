use std::sync::OnceLock;

const ENCODED: &str = include_str!("font_data.b64");

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

fn decode(input: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    for group in input.as_bytes().chunks(4) {
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
    }
}
