//! DarkDraw `.ddw` animation decoding.
//!
//! A DDW file is UTF-8 JSON Lines. Each record is either metadata, a frame, or
//! a positioned text object. Objects without a `frame` field form the base
//! image; frame objects are painted on top when their space-separated frame
//! list contains the current frame id.

use std::{collections::BTreeMap, time::Duration};

use crate::{Cell, Sauce, Screen, ansi::MAX_CELLS, text::CP437};

pub(crate) struct ParsedDdw {
    pub screen: Screen,
    pub sauce: Option<Sauce>,
    pub frames: Vec<ParsedFrame>,
}

pub(crate) struct ParsedFrame {
    pub screen: Screen,
    pub duration: Duration,
    pub data: Vec<u8>,
}

pub(crate) fn is_ddw(data: &[u8]) -> bool {
    data.windows(b"\"duration_ms\"".len())
        .any(|window| window == b"\"duration_ms\"")
        && data
            .windows(b"\"frame\"".len())
            .any(|window| window == b"\"frame\"")
}

pub(crate) fn parse(data: &[u8], width_override: Option<usize>) -> Result<ParsedDdw, String> {
    let text = std::str::from_utf8(data).map_err(|_| "DDW input must be UTF-8".to_owned())?;
    let mut items = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        items.push(Item::parse(line, line_number + 1)?);
    }
    if items.is_empty() {
        return Err("DDW input contains no JSON records".to_owned());
    }

    let metadata = Metadata::from_items(&items);
    let elements = expand_elements(&items)?;
    let (inferred_width, height) = metadata
        .dimensions
        .map(Ok)
        .unwrap_or_else(|| infer_dimensions(&elements))?;
    let width = match (metadata.dimensions, width_override) {
        (Some((declared_width, _)), Some(override_width)) if override_width != declared_width => {
            return Err(format!(
                "DDW canvas width is {declared_width}; --width must match the declared width"
            ));
        }
        (None, Some(override_width)) if override_width < inferred_width => {
            return Err(format!(
                "DDW canvas needs at least {inferred_width} columns; --width is too small"
            ));
        }
        (_, Some(override_width)) => override_width,
        (_, None) => inferred_width,
    };
    let cell_count = width
        .checked_mul(height)
        .ok_or_else(|| "DDW canvas is too large".to_owned())?;
    if cell_count == 0 || cell_count > MAX_CELLS {
        return Err(format!(
            "DDW canvas has {cell_count} cells; expected 1..={MAX_CELLS}"
        ));
    }

    let frames = items
        .iter()
        .filter(|item| item.kind.as_deref() == Some("frame"))
        .map(Frame::from_item)
        .collect::<Result<Vec<_>, _>>()?;
    let mut ids = std::collections::BTreeSet::new();
    for frame in &frames {
        if !ids.insert(&frame.id) {
            return Err(format!("DDW repeats frame id: {}", frame.id));
        }
    }

    let sauce = Sauce::from_text_metadata(
        metadata.title,
        metadata.author,
        metadata.group,
        metadata.date,
        width,
        height,
        metadata.font,
    );
    if frames.is_empty() {
        let (screen, _, _) = paint_frame(&elements, None, width, height)?;
        return Ok(ParsedDdw {
            screen,
            sauce,
            frames: Vec::new(),
        });
    }

    let mut parsed_frames = Vec::with_capacity(frames.len());
    for frame in &frames {
        let (screen, characters, styles) = paint_frame(&elements, Some(&frame.id), width, height)?;
        parsed_frames.push(ParsedFrame {
            data: encode_frame(&screen, &characters, &styles),
            screen,
            duration: frame.duration,
        });
    }
    Ok(ParsedDdw {
        screen: parsed_frames
            .last()
            .expect("validated non-empty frame list")
            .screen
            .clone(),
        sauce,
        frames: parsed_frames,
    })
}

#[derive(Default)]
struct Metadata {
    title: String,
    author: String,
    group: String,
    date: String,
    font: String,
    dimensions: Option<(usize, usize)>,
}

impl Metadata {
    fn from_items(items: &[Item]) -> Self {
        let mut metadata = Self::default();
        for item in items {
            if item.frame.as_deref() != Some("SAUCE_record") {
                continue;
            }
            match item.kind.as_deref() {
                Some("Title") => metadata.title = item.text.clone(),
                Some("Author") => metadata.author = item.text.clone(),
                Some("Group") => metadata.group = item.text.clone(),
                Some("Date") => metadata.date = item.text.clone(),
                Some("Font") => metadata.font = item.text.clone(),
                Some("Dimensions") => metadata.dimensions = parse_dimensions(&item.text),
                _ => {}
            }
        }
        metadata
    }
}

fn parse_dimensions(value: &str) -> Option<(usize, usize)> {
    let (width, height) = value.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

fn infer_dimensions(elements: &[Element]) -> Result<(usize, usize), String> {
    let mut width = 0_usize;
    let mut height = 0_usize;
    for element in elements {
        let right = element
            .x
            .checked_add(element.text.chars().count())
            .ok_or_else(|| format!("DDW element on line {} is outside its canvas", element.line))?;
        let bottom = element
            .y
            .checked_add(1)
            .ok_or_else(|| format!("DDW element on line {} is outside its canvas", element.line))?;
        width = width.max(right);
        height = height.max(bottom);
    }
    if width == 0 || height == 0 {
        Err("DDW input has no positioned text elements to size its canvas".to_owned())
    } else {
        Ok((width, height))
    }
}

struct Frame {
    id: String,
    duration: Duration,
}

impl Frame {
    fn from_item(item: &Item) -> Result<Self, String> {
        let id = item
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("DDW frame on line {} has no id", item.line))?
            .to_owned();
        let milliseconds = item.duration_ms.unwrap_or(100);
        Ok(Self {
            id,
            duration: Duration::from_millis(milliseconds),
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Style {
    foreground: u8,
    background: u8,
    bold: bool,
    dim: bool,
    underline: bool,
    reverse: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            foreground: 7,
            background: 0,
            bold: false,
            dim: false,
            underline: false,
            reverse: false,
        }
    }
}

impl Style {
    fn parse(value: &str, line: usize) -> Result<Self, String> {
        let mut style = Self::default();
        let mut background = false;
        for token in value.split_whitespace() {
            match token {
                "on" | "bg" => background = true,
                "fg" => background = false,
                "bold" => style.bold = true,
                "dim" => style.dim = true,
                "underline" => style.underline = true,
                "reverse" => style.reverse = true,
                "blink" => {}
                _ => {
                    let color = color(token).ok_or_else(|| {
                        format!("DDW line {line} has unsupported color token: {token}")
                    })?;
                    if background {
                        style.background = color;
                    } else {
                        style.foreground = color;
                    }
                }
            }
        }
        Ok(style)
    }

    fn screen_colors(self) -> (u8, u8) {
        let mut colors = (self.foreground, self.background);
        if self.reverse {
            colors = (colors.1, colors.0);
        }
        colors
    }

    fn encode(self, output: &mut Vec<u8>) {
        let mut parameters = vec!["0".to_owned()];
        if self.bold {
            parameters.push("1".to_owned());
        }
        if self.dim {
            parameters.push("2".to_owned());
        }
        if self.underline {
            parameters.push("4".to_owned());
        }
        if self.reverse {
            parameters.push("7".to_owned());
        }
        parameters.push(format!("38;5;{}", self.foreground));
        parameters.push(format!("48;5;{}", self.background));
        output.extend_from_slice(b"\x1b[");
        output.extend_from_slice(parameters.join(";").as_bytes());
        output.push(b'm');
    }
}

fn color(value: &str) -> Option<u8> {
    value.parse::<u8>().ok().or_else(|| {
        Some(match value {
            "black" => 0,
            "red" => 1,
            "green" => 2,
            "yellow" => 3,
            "blue" => 4,
            "magenta" => 5,
            "cyan" => 6,
            "white" => 7,
            _ => return None,
        })
    })
}

fn paint_frame(
    elements: &[Element],
    frame: Option<&str>,
    width: usize,
    height: usize,
) -> Result<(Screen, Vec<char>, Vec<Style>), String> {
    let mut cells = vec![Cell::default(); width * height];
    let mut characters = vec![' '; cells.len()];
    let mut styles = vec![Style::default(); cells.len()];
    for item in elements {
        if !item.in_frame(frame) {
            continue;
        }
        let style = Style::parse(&item.color, item.line)?;
        let (foreground, background) = style.screen_colors();
        for (offset, character) in item.text.chars().enumerate() {
            let column = item.x.checked_add(offset).ok_or_else(|| {
                format!("DDW element on line {} is outside its canvas", item.line)
            })?;
            if column >= width || item.y >= height {
                return Err(format!(
                    "DDW element on line {} is outside its canvas",
                    item.line
                ));
            }
            let index = item.y * width + column;
            cells[index] = Cell {
                // The bitmap/PNG fallback only has CP437 glyphs. Terminal
                // animation still receives the original Unicode below.
                character: u16::from(cp437(character).unwrap_or(b'?')),
                foreground,
                background,
            };
            characters[index] = character;
            styles[index] = style;
        }
    }
    Ok((
        Screen {
            width,
            height,
            cells,
            glyph_width: 8,
            glyph_height: 16,
            font: None,
            palette: None,
            utf8_supported: true,
            raster: None,
        },
        characters,
        styles,
    ))
}

fn cp437(character: char) -> Option<u8> {
    if character == ' ' {
        return Some(b' ');
    }
    CP437
        .iter()
        .position(|&candidate| candidate == character)
        .and_then(|index| u8::try_from(index).ok())
}

fn encode_frame(screen: &Screen, characters: &[char], styles: &[Style]) -> Vec<u8> {
    let mut output = b"\x1b[H".to_vec();
    for row in 0..screen.height {
        output.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
        let mut active_style = None;
        for column in 0..screen.width {
            let index = row * screen.width + column;
            if active_style != Some(styles[index]) {
                styles[index].encode(&mut output);
                active_style = Some(styles[index]);
            }
            let mut encoded = [0_u8; 4];
            output.extend_from_slice(characters[index].encode_utf8(&mut encoded).as_bytes());
        }
    }
    output
}

struct Item {
    line: usize,
    id: Option<String>,
    kind: Option<String>,
    x: Option<usize>,
    y: Option<usize>,
    text: String,
    color: String,
    frame: Option<String>,
    duration_ms: Option<u64>,
    reference: Option<String>,
    rows: Vec<Item>,
}

impl Item {
    fn parse(line: &str, line_number: usize) -> Result<Self, String> {
        let values = JsonReader::new(line.as_bytes())
            .object()
            .map_err(|error| format!("DDW line {line_number}: {error}"))?;
        Self::from_values(&values, line_number)
    }

    fn from_values(values: &[(String, Value)], line_number: usize) -> Result<Self, String> {
        let rows = match values
            .iter()
            .find_map(|(name, value)| match (name.as_str(), value) {
                ("rows", Value::Array(rows)) => Some(rows),
                _ => None,
            }) {
            Some(rows) => rows
                .iter()
                .map(|row| match row {
                    Value::Object(values) => Self::from_values(values, line_number),
                    _ => Err(format!(
                        "DDW group on line {line_number} has a non-object row"
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        Ok(Self {
            line: line_number,
            id: string(values, "id"),
            kind: string(values, "type"),
            x: number(values, "x")
                .map(usize::try_from)
                .transpose()
                .map_err(|_| format!("DDW line {line_number} has an invalid x position"))?,
            y: number(values, "y")
                .map(usize::try_from)
                .transpose()
                .map_err(|_| format!("DDW line {line_number} has an invalid y position"))?,
            text: string(values, "text").unwrap_or_default(),
            color: string(values, "color")
                .or_else(|| number(values, "color").map(|value| value.to_string()))
                .unwrap_or_default(),
            frame: string(values, "frame"),
            duration_ms: number(values, "duration_ms")
                .map(u64::try_from)
                .transpose()
                .map_err(|_| format!("DDW line {line_number} has an invalid frame duration"))?,
            reference: string(values, "ref"),
            rows,
        })
    }

    fn is_element(&self) -> bool {
        self.kind.as_deref().is_none_or(str::is_empty)
    }
}

struct Element {
    line: usize,
    x: usize,
    y: usize,
    text: String,
    color: String,
    frames: Vec<String>,
}

impl Element {
    fn in_frame(&self, frame: Option<&str>) -> bool {
        self.frames.iter().all(|frames| {
            frame.is_some_and(|frame| frames.split_whitespace().any(|id| id == frame))
        })
    }
}

fn expand_elements(items: &[Item]) -> Result<Vec<Element>, String> {
    let mut groups = BTreeMap::new();
    for item in items
        .iter()
        .filter(|item| item.kind.as_deref() == Some("group"))
    {
        let id = item
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("DDW group on line {} has no id", item.line))?;
        if groups.insert(id, item).is_some() {
            return Err(format!("DDW repeats group id: {id}"));
        }
    }

    let mut elements = Vec::new();
    let mut group_stack = Vec::new();
    for item in items {
        expand_item(item, (0, 0), &[], &groups, &mut elements, &mut group_stack)?;
    }
    Ok(elements)
}

fn expand_item(
    item: &Item,
    offset: (usize, usize),
    inherited_frames: &[String],
    groups: &BTreeMap<&str, &Item>,
    elements: &mut Vec<Element>,
    group_stack: &mut Vec<String>,
) -> Result<(), String> {
    match item.kind.as_deref() {
        Some("group") => Ok(()),
        Some("ref") => {
            let reference = item
                .reference
                .as_deref()
                .filter(|reference| !reference.is_empty())
                .ok_or_else(|| format!("DDW ref on line {} has no target group", item.line))?;
            let group = groups.get(reference).ok_or_else(|| {
                format!(
                    "DDW ref on line {} points to missing group: {reference}",
                    item.line
                )
            })?;
            if group_stack.iter().any(|id| id == reference) {
                return Err(format!("DDW group reference cycle includes: {reference}"));
            }
            let position = positioned(item, offset)?;
            let group_offset = add_position(
                position,
                (group.x.unwrap_or(0), group.y.unwrap_or(0)),
                item.line,
            )?;
            let frames = append_frame(inherited_frames, item.frame.as_deref());
            group_stack.push(reference.to_owned());
            for row in &group.rows {
                expand_item(row, group_offset, &frames, groups, elements, group_stack)?;
            }
            group_stack.pop();
            Ok(())
        }
        _ if item.is_element() && !item.text.is_empty() => {
            let (x, y) = positioned(item, offset)?;
            elements.push(Element {
                line: item.line,
                x,
                y,
                text: item.text.clone(),
                color: item.color.clone(),
                frames: append_frame(inherited_frames, item.frame.as_deref()),
            });
            Ok(())
        }
        _ => Ok(()),
    }
}

fn positioned(item: &Item, offset: (usize, usize)) -> Result<(usize, usize), String> {
    let x = item
        .x
        .ok_or_else(|| format!("DDW element on line {} has no x position", item.line))?;
    let y = item
        .y
        .ok_or_else(|| format!("DDW element on line {} has no y position", item.line))?;
    add_position(offset, (x, y), item.line)
}

fn add_position(
    left: (usize, usize),
    right: (usize, usize),
    line: usize,
) -> Result<(usize, usize), String> {
    Ok((
        left.0
            .checked_add(right.0)
            .ok_or_else(|| format!("DDW element on line {line} is outside its canvas"))?,
        left.1
            .checked_add(right.1)
            .ok_or_else(|| format!("DDW element on line {line} is outside its canvas"))?,
    ))
}

fn append_frame(inherited: &[String], frame: Option<&str>) -> Vec<String> {
    let mut frames = inherited.to_vec();
    if let Some(frame) = frame.filter(|frame| !frame.is_empty()) {
        frames.push(frame.to_owned());
    }
    frames
}

fn string(values: &[(String, Value)], key: &str) -> Option<String> {
    values
        .iter()
        .find_map(|(name, value)| match (name == key, value) {
            (true, Value::String(value)) => Some(value.clone()),
            _ => None,
        })
}

fn number(values: &[(String, Value)], key: &str) -> Option<i64> {
    values
        .iter()
        .find_map(|(name, value)| match (name == key, value) {
            (true, Value::Number(value)) => Some(*value),
            _ => None,
        })
}

enum Value {
    String(String),
    Number(i64),
    Null,
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
    Ignored,
}

struct JsonReader<'a> {
    input: &'a [u8],
    index: usize,
}

impl<'a> JsonReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, index: 0 }
    }

    fn object(mut self) -> Result<Vec<(String, Value)>, String> {
        let values = self.parse_object()?;
        self.whitespace();
        if self.index != self.input.len() {
            return Err("trailing content after JSON object".to_owned());
        }
        Ok(values)
    }

    fn parse_object(&mut self) -> Result<Vec<(String, Value)>, String> {
        self.expect(b'{')?;
        let mut values = Vec::new();
        loop {
            self.whitespace();
            if self.consume(b'}') {
                return Ok(values);
            }
            let key = self.parse_string()?;
            self.whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            values.push((key, value));
            self.whitespace();
            if self.consume(b'}') {
                return Ok(values);
            }
            self.expect(b',')?;
        }
    }

    fn parse_array(&mut self) -> Result<Vec<Value>, String> {
        self.expect(b'[')?;
        let mut values = Vec::new();
        loop {
            self.whitespace();
            if self.consume(b']') {
                return Ok(values);
            }
            values.push(self.parse_value()?);
            self.whitespace();
            if self.consume(b']') {
                return Ok(values);
            }
            self.expect(b',')?;
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.whitespace();
        match self.peek().ok_or_else(|| "missing JSON value".to_owned())? {
            b'"' => Ok(Value::String(self.parse_string()?)),
            b'{' => self.parse_object().map(Value::Object),
            b'[' => self.parse_array().map(Value::Array),
            b'n' => {
                self.literal(b"null")?;
                Ok(Value::Null)
            }
            b't' => {
                self.literal(b"true")?;
                Ok(Value::Ignored)
            }
            b'f' => {
                self.literal(b"false")?;
                Ok(Value::Ignored)
            }
            b'-' | b'0'..=b'9' => self.parse_number().map(Value::Number),
            _ => Err("invalid JSON value".to_owned()),
        }
    }

    fn parse_number(&mut self) -> Result<i64, String> {
        let start = self.index;
        self.consume(b'-');
        let digits = self.index;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.index += 1;
        }
        if self.index == digits {
            return Err("invalid JSON number".to_owned());
        }
        if self
            .peek()
            .is_some_and(|byte| matches!(byte, b'.' | b'e' | b'E'))
        {
            return Err("DDW numbers must be integers".to_owned());
        }
        std::str::from_utf8(&self.input[start..self.index])
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| "invalid JSON number".to_owned())
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut output = String::new();
        let mut start = self.index;
        loop {
            let byte = *self
                .input
                .get(self.index)
                .ok_or_else(|| "unterminated JSON string".to_owned())?;
            match byte {
                b'"' => {
                    output.push_str(
                        std::str::from_utf8(&self.input[start..self.index])
                            .map_err(|_| "invalid UTF-8 in JSON string")?,
                    );
                    self.index += 1;
                    return Ok(output);
                }
                b'\\' => {
                    output.push_str(
                        std::str::from_utf8(&self.input[start..self.index])
                            .map_err(|_| "invalid UTF-8 in JSON string")?,
                    );
                    self.index += 1;
                    let escape = self
                        .input
                        .get(self.index)
                        .copied()
                        .ok_or_else(|| "unterminated JSON escape".to_owned())?;
                    self.index += 1;
                    match escape {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{0008}'),
                        b'f' => output.push('\u{000c}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => output.push(self.unicode_escape()?),
                        _ => return Err("invalid JSON escape".to_owned()),
                    }
                    start = self.index;
                }
                0..=0x1f => return Err("unescaped control character in JSON string".to_owned()),
                _ => self.index += 1,
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let value = self.hex_quad()?;
        if !(0xd800..=0xdbff).contains(&value) {
            return char::from_u32(u32::from(value))
                .ok_or_else(|| "invalid Unicode escape".to_owned());
        }
        if self.input.get(self.index..self.index + 2) != Some(b"\\u") {
            return Err("unpaired Unicode surrogate".to_owned());
        }
        self.index += 2;
        let low = self.hex_quad()?;
        if !(0xdc00..=0xdfff).contains(&low) {
            return Err("unpaired Unicode surrogate".to_owned());
        }
        char::from_u32(0x1_0000 + ((u32::from(value) - 0xd800) << 10) + u32::from(low) - 0xdc00)
            .ok_or_else(|| "invalid Unicode escape".to_owned())
    }

    fn hex_quad(&mut self) -> Result<u16, String> {
        let end = self
            .index
            .checked_add(4)
            .ok_or_else(|| "invalid Unicode escape".to_owned())?;
        let digits = self
            .input
            .get(self.index..end)
            .ok_or_else(|| "invalid Unicode escape".to_owned())?;
        self.index = end;
        std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| u16::from_str_radix(digits, 16).ok())
            .ok_or_else(|| "invalid Unicode escape".to_owned())
    }

    fn literal(&mut self, value: &[u8]) -> Result<(), String> {
        if self.input.get(self.index..self.index + value.len()) == Some(value) {
            self.index += value.len();
            Ok(())
        } else {
            Err("invalid JSON literal".to_owned())
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.index += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        self.whitespace();
        if self.consume(expected) {
            Ok(())
        } else {
            Err(format!("expected JSON byte {}", char::from(expected)))
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"id":"-","type":"-","x":0,"y":0,"text":"-","color":null,"tags":null,"group":null,"frame":"-","rows":null,"duration_ms":null}
{"type":"Title","text":"The Door","frame":"SAUCE_record","rows":[]}
{"type":"Author","text":"Artist","frame":"SAUCE_record","rows":[]}
{"type":"Dimensions","text":"3x2","frame":"SAUCE_record","rows":[]}
{"id":"4","type":"frame","duration_ms":200}
{"id":"5","type":"frame","duration_ms":100}
{"x":0,"y":0,"text":"A","color":"15 on 1"}
{"x":1,"y":0,"text":"░","color":"3 dim","frame":"4"}
{"x":1,"y":0,"text":"B","color":"2","frame":"5"}
"#;

    #[test]
    fn decodes_layered_frames_metadata_and_colors() {
        let parsed = parse(SAMPLE.as_bytes(), None).unwrap();

        assert_eq!((parsed.screen.width, parsed.screen.height), (3, 2));
        assert_eq!(parsed.sauce.as_ref().unwrap().author, "Artist");
        assert_eq!(parsed.frames.len(), 2);
        assert_eq!(parsed.frames[0].duration, Duration::from_millis(200));
        assert_eq!(parsed.frames[1].duration, Duration::from_millis(100));
        assert_eq!(parsed.frames[0].screen.cells[1].character, 0xb0);
        assert_eq!(parsed.frames[1].screen.cells[1].character, u16::from(b'B'));
        assert!(parsed.frames[0].data.starts_with(b"\x1b[H"));
        assert!(
            parsed.frames[0]
                .data
                .windows(5)
                .any(|part| part == b"38;5;")
        );
    }

    #[test]
    fn preserves_unicode_glyphs_for_terminal_animation() {
        let sample = SAMPLE.replace("\"░\"", "\"🙂\"");
        let parsed = parse(sample.as_bytes(), None).unwrap();
        assert!(
            String::from_utf8(parsed.frames[0].data.clone())
                .unwrap()
                .contains('🙂')
        );
    }

    #[test]
    fn infers_canvas_dimensions_and_accepts_numeric_colors() {
        let sample = concat!(
            r#"{"id":"0","type":"frame","duration_ms":100}"#,
            "\n",
            r#"{"x":2,"y":1,"text":"██","color":247,"frame":"0"}"#,
        );
        let parsed = parse(sample.as_bytes(), None).unwrap();

        assert_eq!((parsed.screen.width, parsed.screen.height), (4, 2));
        assert!(
            String::from_utf8(parsed.frames[0].data.clone())
                .unwrap()
                .contains("38;5;247")
        );
        assert_eq!(parsed.screen.cells[6].foreground, 247);
    }

    #[test]
    fn expands_nested_group_references_at_their_frame_positions() {
        let sample = concat!(
            r#"{"id":"0","type":"frame","duration_ms":100}"#,
            "\n",
            r#"{"id":"car","type":"group","rows":[{"x":0,"y":0,"text":"X","color":"red"}]}"#,
            "\n",
            r#"{"id":"lit-car","type":"group","rows":[{"x":0,"y":0,"type":"ref","ref":"car"},{"x":0,"y":0,"text":"*","color":"yellow"}]}"#,
            "\n",
            r#"{"x":2,"y":1,"type":"ref","ref":"lit-car","frame":"0"}"#,
        );
        let parsed = parse(sample.as_bytes(), None).unwrap();
        let frame = &parsed.frames[0];

        assert_eq!((frame.screen.width, frame.screen.height), (3, 2));
        assert_eq!(frame.screen.cells[5].character, u16::from(b'*'));
        assert!(
            String::from_utf8(frame.data.clone())
                .unwrap()
                .contains("38;5;3")
        );
    }
}
