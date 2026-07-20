//! SAUCE metadata trailer parsing.
//!
//! SAUCE appends a fixed 128-byte record to artwork rather than wrapping it in a
//! container. The record identifies itself with `SAUCE00`, reports where the art
//! bytes end, and may provide dimensions, iCE-color mode, and a font name.

/// Metadata decoded from a SAUCE trailer or equivalent format metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sauce {
    /// Artwork title.
    pub title: String,
    /// Artist or author name.
    pub author: String,
    /// Art group name.
    pub group: String,
    /// Source date, conventionally formatted as `YYYYMMDD`.
    pub date: String,
    /// Declared character width, or zero when unspecified.
    pub width: usize,
    /// Declared character height, or zero when unspecified.
    pub height: usize,
    /// Whether the blink bit selects bright background colors.
    pub ice_colors: bool,
    /// Explicit eight- or nine-pixel VGA character spacing.
    pub letter_spacing: Option<LetterSpacing>,
    /// Named bitmap font requested by the metadata.
    pub font_name: String,
    content_len: usize,
}

/// Horizontal VGA character-cell spacing declared by SAUCE.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LetterSpacing {
    /// Use an eight-pixel-wide character cell.
    EightPixels,
    /// Use a nine-pixel-wide VGA character cell.
    NinePixels,
}

impl LetterSpacing {
    pub(crate) fn glyph_width(self) -> usize {
        match self {
            Self::EightPixels => 8,
            Self::NinePixels => 9,
        }
    }
}

impl Sauce {
    pub(crate) fn from_text_metadata(
        title: String,
        author: String,
        group: String,
        date: String,
        width: usize,
        height: usize,
        font_name: String,
    ) -> Option<Self> {
        if title.is_empty()
            && author.is_empty()
            && group.is_empty()
            && date.is_empty()
            && font_name.is_empty()
        {
            return None;
        }
        Some(Self {
            title,
            author,
            group,
            date,
            width,
            height,
            ice_colors: false,
            letter_spacing: None,
            font_name,
            content_len: 0,
        })
    }

    /// Parses a SAUCE record located exactly 128 bytes from the end of `data`.
    pub fn parse(data: &[u8]) -> Option<Self> {
        // The signature is meaningful only exactly 128 bytes from EOF.
        let start = data.len().checked_sub(128)?;
        let record = &data[start..];
        if &record[..7] != b"SAUCE00" {
            return None;
        }

        let reported_len = u32::from_le_bytes(record[90..94].try_into().ok()?) as usize;
        // Broken producers sometimes report a size beyond the trailer. Clamp to
        // the physical trailer boundary so metadata can never become art data.
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
            letter_spacing: match (record[105] >> 1) & 0b11 {
                1 => Some(LetterSpacing::EightPixels),
                2 => Some(LetterSpacing::NinePixels),
                _ => None,
            },
            font_name: field(&record[106..128]),
            content_len,
        })
    }

    /// Returns the artwork bytes preceding this SAUCE record.
    pub fn content<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[..self.content_len.min(data.len())]
    }
}

fn field(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .rposition(|&byte| byte != 0 && byte != b' ')
        .map_or(0, |position| position + 1);
    bytes[..end]
        .iter()
        .map(|&byte| crate::text::CP437[usize::from(byte)])
        .collect()
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
        record[105] = 0b101; // iCE colors plus a 9-pixel VGA font.
        data.extend(record);

        let sauce = Sauce::parse(&data).unwrap();
        assert_eq!(sauce.title, "Demo");
        assert_eq!((sauce.width, sauce.height), (80, 25));
        assert!(sauce.ice_colors);
        assert_eq!(sauce.letter_spacing, Some(LetterSpacing::NinePixels));
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

    #[test]
    fn decodes_text_fields_as_cp437() {
        let mut data = Vec::new();
        let mut record = [0_u8; 128];
        record[..7].copy_from_slice(b"SAUCE00");
        record[7..11].copy_from_slice(&[b'C', b'a', b'f', 0x82]);
        data.extend(record);

        let sauce = Sauce::parse(&data).unwrap();
        assert_eq!(sauce.title, "Café");
    }
}
