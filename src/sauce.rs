#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sauce {
    pub title: String,
    pub author: String,
    pub group: String,
    pub date: String,
    pub width: usize,
    pub height: usize,
    pub ice_colors: bool,
    pub font_name: String,
    content_len: usize,
}

impl Sauce {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let start = data.len().checked_sub(128)?;
        let record = &data[start..];
        if &record[..7] != b"SAUCE00" {
            return None;
        }

        let reported_len = u32::from_le_bytes(record[90..94].try_into().ok()?) as usize;
        let content_len = if reported_len <= start {
            reported_len
        } else {
            start
        };
        Some(Self {
            title: field(&record[7..42]),
            author: field(&record[42..62]),
            group: field(&record[62..82]),
            date: field(&record[82..90]),
            width: u16::from_le_bytes(record[96..98].try_into().ok()?) as usize,
            height: u16::from_le_bytes(record[98..100].try_into().ok()?) as usize,
            ice_colors: record[105] & 1 != 0,
            font_name: field(&record[106..128]),
            content_len,
        })
    }

    pub fn content<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[..self.content_len.min(data.len())]
    }
}

fn field(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .rposition(|&byte| byte != 0 && byte != b' ')
        .map_or(0, |position| position + 1);
    bytes[..end].iter().map(|&byte| char::from(byte)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_record_and_content_size() {
        let mut data = vec![b'x'; 3];
        let mut record = [0_u8; 128];
        record[..7].copy_from_slice(b"SAUCE00");
        record[7..11].copy_from_slice(b"Demo");
        record[90..94].copy_from_slice(&3_u32.to_le_bytes());
        record[96..98].copy_from_slice(&80_u16.to_le_bytes());
        record[98..100].copy_from_slice(&25_u16.to_le_bytes());
        record[105] = 1;
        data.extend(record);

        let sauce = Sauce::parse(&data).unwrap();
        assert_eq!(sauce.title, "Demo");
        assert_eq!((sauce.width, sauce.height), (80, 25));
        assert!(sauce.ice_colors);
        assert_eq!(sauce.content(&data), b"xxx");
    }

    #[test]
    fn preserves_a_binary_sub_byte_inside_the_reported_size() {
        let mut data = vec![b'X', 0x1a];
        let mut record = [0_u8; 128];
        record[..7].copy_from_slice(b"SAUCE00");
        record[90..94].copy_from_slice(&2_u32.to_le_bytes());
        record[94] = 6; // XBin
        data.extend(record);

        let sauce = Sauce::parse(&data).unwrap();
        assert_eq!(sauce.content(&data), &[b'X', 0x1a]);
    }
}
