//! Minimal read-only ZIP support for archive descriptions.
//!
//! Art packs commonly carry a `FILE_ID.ANS` or `FILE_ID.DIZ` alongside images,
//! music, and the full-size artwork. Pulling that one description out needs the ZIP
//! central directory plus the two baseline compression methods: stored and
//! Deflate. Keeping those pieces here preserves bbcat's dependency-free build.

const MAX_ENTRY_SIZE: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

#[derive(Debug)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
}

#[derive(Clone)]
struct CentralEntry {
    name: String,
    name_bytes: Vec<u8>,
    flags: u16,
    method: u16,
    crc: u32,
    compressed_size: usize,
    uncompressed_size: usize,
    local_offset: usize,
}

pub(crate) fn is_zip(data: &[u8]) -> bool {
    matches!(
        data.get(..4),
        Some(b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x07\x08")
    )
}

pub(crate) fn extract_preview(data: &[u8]) -> Result<Entry, String> {
    let entries = central_directory(data)?;
    let selected = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| preview_rank(&entry.name).map(|rank| (rank, index, entry)))
        .min_by_key(|&(rank, index, _)| (rank, index))
        .map(|(_, _, entry)| entry)
        .ok_or("ZIP archive contains no supported ANSI or BBS artwork")?;
    extract(data, selected)
}

fn preview_rank(name: &str) -> Option<u8> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let upper = base.to_ascii_uppercase();
    if upper == "FILE_ID.ANS" {
        return Some(0);
    }
    if upper == "FILE_ID.DIZ" {
        return Some(1);
    }
    if upper.starts_with("LICENSE") || upper.starts_with("README") {
        return None;
    }
    let extension = upper.rsplit_once('.')?.1;
    match extension {
        "DIZ" => Some(2),
        "NFO" | "ANS" | "ASC" | "MEM" => Some(3),
        "DDW" | "ADF" | "RIP" | "XB" => Some(4),
        "TXT" => Some(5),
        _ => None,
    }
}

fn central_directory(data: &[u8]) -> Result<Vec<CentralEntry>, String> {
    let eocd = find_eocd(data).ok_or("invalid ZIP archive: end record not found")?;
    let disk = le_u16(data, eocd + 4, "ZIP disk number")?;
    let central_disk = le_u16(data, eocd + 6, "ZIP central-directory disk")?;
    let disk_entries = usize::from(le_u16(data, eocd + 8, "ZIP disk entry count")?);
    let entry_count = usize::from(le_u16(data, eocd + 10, "ZIP entry count")?);
    let central_size = usize::try_from(le_u32(data, eocd + 12, "ZIP central-directory size")?)
        .map_err(|_| "ZIP central-directory size is too large")?;
    let central_offset = usize::try_from(le_u32(data, eocd + 16, "ZIP central-directory offset")?)
        .map_err(|_| "ZIP central-directory offset is too large")?;

    if disk != 0 || central_disk != 0 || disk_entries != entry_count {
        return Err("multi-disk ZIP archives are not supported".to_owned());
    }
    if entry_count == usize::from(u16::MAX)
        || central_size == u32::MAX as usize
        || central_offset == u32::MAX as usize
    {
        return Err("ZIP64 archives are not supported".to_owned());
    }
    if entry_count > MAX_ENTRIES {
        return Err(format!(
            "ZIP archive exceeds the {MAX_ENTRIES} entry safety limit"
        ));
    }
    let central_end = central_offset
        .checked_add(central_size)
        .filter(|&end| end <= eocd)
        .ok_or("invalid ZIP central-directory bounds")?;

    let mut entries = Vec::with_capacity(entry_count);
    let mut offset = central_offset;
    for _ in 0..entry_count {
        let header = data
            .get(offset..)
            .and_then(|tail| tail.get(..46))
            .ok_or("truncated ZIP central-directory entry")?;
        if header.get(..4) != Some(b"PK\x01\x02") {
            return Err("invalid ZIP central-directory entry".to_owned());
        }
        let flags = le_u16(header, 8, "ZIP entry flags")?;
        let method = le_u16(header, 10, "ZIP compression method")?;
        let crc = le_u32(header, 16, "ZIP entry CRC")?;
        let compressed_size = usize::try_from(le_u32(header, 20, "ZIP compressed size")?)
            .map_err(|_| "ZIP compressed size is too large")?;
        let uncompressed_size = usize::try_from(le_u32(header, 24, "ZIP uncompressed size")?)
            .map_err(|_| "ZIP uncompressed size is too large")?;
        let name_length = usize::from(le_u16(header, 28, "ZIP name length")?);
        let extra_length = usize::from(le_u16(header, 30, "ZIP extra length")?);
        let comment_length = usize::from(le_u16(header, 32, "ZIP comment length")?);
        let start_disk = le_u16(header, 34, "ZIP entry disk")?;
        let local_offset = usize::try_from(le_u32(header, 42, "ZIP local-header offset")?)
            .map_err(|_| "ZIP local-header offset is too large")?;
        if start_disk != 0 {
            return Err("multi-disk ZIP archives are not supported".to_owned());
        }
        if compressed_size == u32::MAX as usize
            || uncompressed_size == u32::MAX as usize
            || local_offset == u32::MAX as usize
        {
            return Err("ZIP64 entries are not supported".to_owned());
        }
        let name_start = offset
            .checked_add(46)
            .ok_or("ZIP central-directory offset overflow")?;
        let name_end = name_start
            .checked_add(name_length)
            .ok_or("ZIP entry name overflow")?;
        let next = name_end
            .checked_add(extra_length)
            .and_then(|end| end.checked_add(comment_length))
            .filter(|&end| end <= central_end)
            .ok_or("truncated ZIP central-directory entry")?;
        let name_bytes = data
            .get(name_start..name_end)
            .ok_or("truncated ZIP entry name")?
            .to_vec();
        let name = String::from_utf8_lossy(&name_bytes).into_owned();
        if !name.ends_with('/') && !name.ends_with('\\') {
            entries.push(CentralEntry {
                name,
                name_bytes,
                flags,
                method,
                crc,
                compressed_size,
                uncompressed_size,
                local_offset,
            });
        }
        offset = next;
    }
    if offset != central_end {
        return Err("ZIP central-directory size does not match its entries".to_owned());
    }
    Ok(entries)
}

fn find_eocd(data: &[u8]) -> Option<usize> {
    let last = data.len().checked_sub(22)?;
    let first = data.len().saturating_sub(22 + usize::from(u16::MAX));
    (first..=last).rev().find(|&offset| {
        data.get(offset..offset + 4) == Some(b"PK\x05\x06")
            && le_u16(data, offset + 20, "ZIP comment length")
                .ok()
                .is_some_and(|length| offset + 22 + usize::from(length) == data.len())
    })
}

fn extract(data: &[u8], entry: &CentralEntry) -> Result<Entry, String> {
    if entry.flags & 1 != 0 {
        return Err(format!(
            "{}: encrypted ZIP entries are not supported",
            entry.name
        ));
    }
    if entry.uncompressed_size > MAX_ENTRY_SIZE {
        return Err(format!(
            "{}: ZIP entry exceeds the {} MiB safety limit",
            entry.name,
            MAX_ENTRY_SIZE / 1024 / 1024
        ));
    }
    let offset = entry.local_offset;
    let header = data
        .get(offset..)
        .and_then(|tail| tail.get(..30))
        .ok_or_else(|| format!("{}: truncated ZIP local header", entry.name))?;
    if header.get(..4) != Some(b"PK\x03\x04") {
        return Err(format!("{}: invalid ZIP local header", entry.name));
    }
    let local_flags = le_u16(header, 6, "ZIP local flags")?;
    let local_method = le_u16(header, 8, "ZIP local compression method")?;
    let name_length = usize::from(le_u16(header, 26, "ZIP local name length")?);
    let extra_length = usize::from(le_u16(header, 28, "ZIP local extra length")?);
    if local_flags != entry.flags || local_method != entry.method {
        return Err(format!("{}: ZIP headers disagree", entry.name));
    }
    let name_start = offset
        .checked_add(30)
        .ok_or("ZIP local-header offset overflow")?;
    let name_end = name_start
        .checked_add(name_length)
        .ok_or("ZIP local name overflow")?;
    if data.get(name_start..name_end) != Some(entry.name_bytes.as_slice()) {
        return Err(format!("{}: ZIP headers have different names", entry.name));
    }
    let content_start = name_end
        .checked_add(extra_length)
        .ok_or("ZIP entry offset overflow")?;
    let content_end = content_start
        .checked_add(entry.compressed_size)
        .ok_or("ZIP entry size overflow")?;
    let compressed = data
        .get(content_start..content_end)
        .ok_or_else(|| format!("{}: truncated ZIP entry data", entry.name))?;
    let output = match entry.method {
        0 => {
            if entry.compressed_size != entry.uncompressed_size {
                return Err(format!("{}: invalid stored ZIP entry size", entry.name));
            }
            compressed.to_vec()
        }
        8 => inflate(compressed, entry.uncompressed_size)
            .map_err(|error| format!("{}: {error}", entry.name))?,
        method => {
            return Err(format!(
                "{}: ZIP compression method {method} is not supported",
                entry.name
            ));
        }
    };
    if crc32(&output) != entry.crc {
        return Err(format!("{}: ZIP CRC check failed", entry.name));
    }
    Ok(Entry {
        name: entry.name.clone(),
        data: output,
    })
}

struct Bits<'a> {
    data: &'a [u8],
    offset: usize,
}

impl Bits<'_> {
    fn read(&mut self, count: usize) -> Result<u32, String> {
        let end = self
            .offset
            .checked_add(count)
            .filter(|&end| end <= self.data.len().saturating_mul(8))
            .ok_or("truncated Deflate stream")?;
        let mut value = 0_u32;
        for bit in 0..count {
            value |= u32::from((self.data[self.offset / 8] >> (self.offset % 8)) & 1) << bit;
            self.offset += 1;
        }
        debug_assert_eq!(self.offset, end);
        Ok(value)
    }

    fn align_byte(&mut self) {
        self.offset = (self.offset + 7) & !7;
    }
}

struct Huffman {
    by_length: Vec<Vec<(u16, u16)>>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Option<Self>, String> {
        let mut counts = [0_u16; 16];
        for &length in lengths {
            if length > 15 {
                return Err("invalid Deflate Huffman code length".to_owned());
            }
            counts[usize::from(length)] += 1;
        }
        if counts[1..].iter().all(|&count| count == 0) {
            return Ok(None);
        }
        let mut remaining = 1_i32;
        for &count in &counts[1..] {
            remaining = (remaining << 1) - i32::from(count);
            if remaining < 0 {
                return Err("oversubscribed Deflate Huffman tree".to_owned());
            }
        }

        let mut next = [0_u16; 16];
        let mut code = 0_u16;
        for length in 1..=15 {
            code = (code + counts[length - 1]) << 1;
            next[length] = code;
        }
        let mut by_length = vec![Vec::new(); 16];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let canonical = next[usize::from(length)];
            next[usize::from(length)] += 1;
            let reversed = canonical.reverse_bits() >> (16 - length);
            by_length[usize::from(length)].push((reversed, symbol as u16));
        }
        Ok(Some(Self { by_length }))
    }

    fn symbol(&self, bits: &mut Bits<'_>) -> Result<u16, String> {
        let mut code = 0_u16;
        for length in 1..self.by_length.len() {
            code |= (bits.read(1)? as u16) << (length - 1);
            if let Some((_, symbol)) = self.by_length[length]
                .iter()
                .find(|&&(candidate, _)| candidate == code)
            {
                return Ok(*symbol);
            }
        }
        Err("invalid Deflate Huffman code".to_owned())
    }
}

fn inflate(data: &[u8], expected_size: usize) -> Result<Vec<u8>, String> {
    let mut bits = Bits { data, offset: 0 };
    let mut output = Vec::with_capacity(expected_size);
    loop {
        let final_block = bits.read(1)? != 0;
        match bits.read(2)? {
            0 => stored_block(&mut bits, &mut output, expected_size)?,
            1 => {
                let literal_lengths = (0..288)
                    .map(|symbol| match symbol {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    })
                    .collect::<Vec<_>>();
                let literals =
                    Huffman::new(&literal_lengths)?.ok_or("invalid fixed Deflate literal tree")?;
                let distances =
                    Huffman::new(&[5; 32])?.ok_or("invalid fixed Deflate distance tree")?;
                compressed_block(
                    &mut bits,
                    &mut output,
                    expected_size,
                    &literals,
                    Some(&distances),
                )?;
            }
            2 => dynamic_block(&mut bits, &mut output, expected_size)?,
            _ => return Err("invalid reserved Deflate block type".to_owned()),
        }
        if final_block {
            break;
        }
    }
    if bits.offset.div_ceil(8) != data.len() {
        return Err("Deflate stream has trailing data".to_owned());
    }
    if output.len() != expected_size {
        return Err(format!(
            "Deflate size mismatch: expected {expected_size} bytes, decoded {}",
            output.len()
        ));
    }
    Ok(output)
}

fn stored_block(
    bits: &mut Bits<'_>,
    output: &mut Vec<u8>,
    expected_size: usize,
) -> Result<(), String> {
    bits.align_byte();
    let length = bits.read(16)? as u16;
    let complement = bits.read(16)? as u16;
    if length != !complement {
        return Err("invalid stored Deflate block length".to_owned());
    }
    let length = usize::from(length);
    if output.len().saturating_add(length) > expected_size {
        return Err("Deflate output exceeds the declared ZIP size".to_owned());
    }
    for _ in 0..length {
        output.push(bits.read(8)? as u8);
    }
    Ok(())
}

fn dynamic_block(
    bits: &mut Bits<'_>,
    output: &mut Vec<u8>,
    expected_size: usize,
) -> Result<(), String> {
    let literal_count = bits.read(5)? as usize + 257;
    let distance_count = bits.read(5)? as usize + 1;
    let code_count = bits.read(4)? as usize + 4;
    if literal_count > 286 || distance_count > 32 {
        return Err("invalid dynamic Deflate tree dimensions".to_owned());
    }
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let mut code_lengths = [0_u8; 19];
    for &symbol in &ORDER[..code_count] {
        code_lengths[symbol] = bits.read(3)? as u8;
    }
    let code_tree =
        Huffman::new(&code_lengths)?.ok_or("dynamic Deflate block has no code-length tree")?;
    let total = literal_count + distance_count;
    let mut lengths = Vec::with_capacity(total);
    while lengths.len() < total {
        match code_tree.symbol(bits)? {
            value @ 0..=15 => lengths.push(value as u8),
            16 => {
                let previous = *lengths
                    .last()
                    .ok_or("Deflate repeat code has no previous length")?;
                let count = bits.read(2)? as usize + 3;
                append_lengths(&mut lengths, total, previous, count)?;
            }
            17 => {
                let count = bits.read(3)? as usize + 3;
                append_lengths(&mut lengths, total, 0, count)?;
            }
            18 => {
                let count = bits.read(7)? as usize + 11;
                append_lengths(&mut lengths, total, 0, count)?;
            }
            _ => return Err("invalid Deflate code-length symbol".to_owned()),
        }
    }
    if lengths[256] == 0 {
        return Err("Deflate literal tree has no end marker".to_owned());
    }
    let literals = Huffman::new(&lengths[..literal_count])?
        .ok_or("dynamic Deflate block has no literal tree")?;
    let distances = Huffman::new(&lengths[literal_count..])?;
    compressed_block(bits, output, expected_size, &literals, distances.as_ref())
}

fn append_lengths(
    lengths: &mut Vec<u8>,
    total: usize,
    value: u8,
    count: usize,
) -> Result<(), String> {
    if lengths.len().saturating_add(count) > total {
        return Err("Deflate code-length repeat exceeds the tree".to_owned());
    }
    lengths.extend(std::iter::repeat_n(value, count));
    Ok(())
}

fn compressed_block(
    bits: &mut Bits<'_>,
    output: &mut Vec<u8>,
    expected_size: usize,
    literals: &Huffman,
    distances: Option<&Huffman>,
) -> Result<(), String> {
    const LENGTH_BASE: [usize; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LENGTH_EXTRA: [usize; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DISTANCE_BASE: [usize; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DISTANCE_EXTRA: [usize; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    loop {
        match literals.symbol(bits)? {
            literal @ 0..=255 => {
                if output.len() == expected_size {
                    return Err("Deflate output exceeds the declared ZIP size".to_owned());
                }
                output.push(literal as u8);
            }
            256 => return Ok(()),
            symbol @ 257..=285 => {
                let index = usize::from(symbol - 257);
                let length = LENGTH_BASE[index] + bits.read(LENGTH_EXTRA[index])? as usize;
                let distance_tree =
                    distances.ok_or("Deflate stream uses a missing distance tree")?;
                let distance_symbol = usize::from(distance_tree.symbol(bits)?);
                if distance_symbol >= DISTANCE_BASE.len() {
                    return Err("invalid Deflate distance symbol".to_owned());
                }
                let distance = DISTANCE_BASE[distance_symbol]
                    + bits.read(DISTANCE_EXTRA[distance_symbol])? as usize;
                if distance == 0 || distance > output.len() {
                    return Err("invalid Deflate back-reference distance".to_owned());
                }
                if output.len().saturating_add(length) > expected_size {
                    return Err("Deflate output exceeds the declared ZIP size".to_owned());
                }
                for _ in 0..length {
                    output.push(output[output.len() - distance]);
                }
            }
            _ => return Err("invalid Deflate literal/length symbol".to_owned()),
        }
    }
}

fn le_u16(data: &[u8], offset: usize, field: &str) -> Result<u16, String> {
    let bytes: [u8; 2] = data
        .get(offset..)
        .and_then(|tail| tail.get(..2))
        .ok_or_else(|| format!("truncated {field}"))?
        .try_into()
        .expect("slice length checked");
    Ok(u16::from_le_bytes(bytes))
}

fn le_u32(data: &[u8], offset: usize, field: &str) -> Result<u32, String> {
    let bytes: [u8; 4] = data
        .get(offset..)
        .and_then(|tail| tail.get(..4))
        .ok_or_else(|| format!("truncated {field}"))?
        .try_into()
        .expect("slice length checked");
    Ok(u32::from_le_bytes(bytes))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEntry<'a> {
        name: &'a str,
        method: u16,
        compressed: &'a [u8],
        plain: &'a [u8],
    }

    fn zip(entries: &[TestEntry<'_>]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut offsets = Vec::new();
        for entry in entries {
            offsets.push(output.len());
            output.extend_from_slice(b"PK\x03\x04");
            output.extend_from_slice(&20_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&entry.method.to_le_bytes());
            output.extend_from_slice(&[0; 4]);
            output.extend_from_slice(&crc32(entry.plain).to_le_bytes());
            output.extend_from_slice(&(entry.compressed.len() as u32).to_le_bytes());
            output.extend_from_slice(&(entry.plain.len() as u32).to_le_bytes());
            output.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(entry.name.as_bytes());
            output.extend_from_slice(entry.compressed);
        }
        let central_offset = output.len();
        for (entry, &local_offset) in entries.iter().zip(&offsets) {
            output.extend_from_slice(b"PK\x01\x02");
            output.extend_from_slice(&20_u16.to_le_bytes());
            output.extend_from_slice(&20_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&entry.method.to_le_bytes());
            output.extend_from_slice(&[0; 4]);
            output.extend_from_slice(&crc32(entry.plain).to_le_bytes());
            output.extend_from_slice(&(entry.compressed.len() as u32).to_le_bytes());
            output.extend_from_slice(&(entry.plain.len() as u32).to_le_bytes());
            output.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            output.extend_from_slice(&[0; 8]);
            output.extend_from_slice(&0_u32.to_le_bytes());
            output.extend_from_slice(&(local_offset as u32).to_le_bytes());
            output.extend_from_slice(entry.name.as_bytes());
        }
        let central_size = output.len() - central_offset;
        output.extend_from_slice(b"PK\x05\x06");
        output.extend_from_slice(&[0; 4]);
        output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        output.extend_from_slice(&(central_size as u32).to_le_bytes());
        output.extend_from_slice(&(central_offset as u32).to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output
    }

    #[test]
    fn chooses_file_id_ans_before_diz_from_a_mixed_archive() {
        let archive = zip(&[
            TestEntry {
                name: "cover.gif",
                method: 0,
                compressed: b"GIF89a",
                plain: b"GIF89a",
            },
            TestEntry {
                name: "art.ans",
                method: 0,
                compressed: b"large art",
                plain: b"large art",
            },
            TestEntry {
                name: "FILE_ID.DIZ",
                method: 0,
                compressed: b"archive description",
                plain: b"archive description",
            },
            TestEntry {
                name: "FILE_ID.ANS",
                method: 0,
                compressed: b"ANSI archive description",
                plain: b"ANSI archive description",
            },
        ]);

        let preview = extract_preview(&archive).unwrap();
        assert_eq!(preview.name, "FILE_ID.ANS");
        assert_eq!(preview.data, b"ANSI archive description");
    }

    #[test]
    fn inflates_fixed_huffman_entries() {
        // Raw Deflate for "hello", produced with the baseline method used by ZIP.
        let compressed = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00];
        let archive = zip(&[TestEntry {
            name: "hello.ans",
            method: 8,
            compressed: &compressed,
            plain: b"hello",
        }]);

        assert_eq!(extract_preview(&archive).unwrap().data, b"hello");
    }

    #[test]
    fn inflates_dynamic_huffman_entries() {
        let plain = b"abcdefghijklmnopqrstuvwxyz".repeat(400);
        let compressed = [
            0xed, 0xc9, 0xb7, 0x01, 0x80, 0x20, 0x00, 0x00, 0xb0, 0x5b, 0xb1, 0x77, 0x11, 0xec,
            0xd7, 0xfb, 0x83, 0x73, 0xb2, 0x26, 0x14, 0x65, 0x55, 0x37, 0x6d, 0xd7, 0x0f, 0xe3,
            0x34, 0x2f, 0x6b, 0xdc, 0x52, 0xde, 0x8f, 0xf3, 0xba, 0x9f, 0x37, 0x18, 0x63, 0x8c,
            0x31, 0xc6, 0x18, 0x63, 0x8c, 0x31, 0xc6, 0x18, 0x63, 0x8c, 0x31, 0xc6, 0x18, 0x63,
            0x8c, 0x31, 0xc6, 0x18, 0x63, 0x8c, 0x31, 0xc6, 0xfc, 0x9a, 0x0f,
        ];
        let archive = zip(&[TestEntry {
            name: "dynamic.ans",
            method: 8,
            compressed: &compressed,
            plain: &plain,
        }]);

        assert_eq!(extract_preview(&archive).unwrap().data, plain);
    }

    #[test]
    fn rejects_bad_crc_and_unsupported_compression() {
        let mut archive = zip(&[TestEntry {
            name: "bad.ans",
            method: 0,
            compressed: b"art",
            plain: b"art",
        }]);
        archive[14] ^= 1;
        let central_crc = archive
            .windows(4)
            .position(|bytes| bytes == b"PK\x01\x02")
            .unwrap()
            + 16;
        archive[central_crc] ^= 1;
        assert!(extract_preview(&archive).unwrap_err().contains("CRC"));

        let archive = zip(&[TestEntry {
            name: "odd.ans",
            method: 12,
            compressed: b"art",
            plain: b"art",
        }]);
        assert!(extract_preview(&archive).unwrap_err().contains("method 12"));
    }
}
