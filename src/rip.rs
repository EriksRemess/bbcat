use std::f64::consts::PI;

use crate::{
    ansi::{Raster, Screen},
    bgi_font,
};

const WIDTH: usize = 640;
const HEIGHT: usize = 350;
const MAX_COMMANDS: usize = 1_000_000;
const LINE_PATTERNS: [u16; 5] = [0xffff, 0xcccc, 0xf878, 0xf8f8, 0];
const FILL_PATTERNS: [[u8; 8]; 13] = [
    [0x00; 8],
    [0xff; 8],
    [0xff, 0xff, 0, 0, 0, 0, 0, 0],
    [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80],
    [0xe0, 0xc1, 0x83, 0x07, 0x0e, 0x1c, 0x38, 0x70],
    [0xf0, 0x78, 0x3c, 0x1e, 0x0f, 0x87, 0xc3, 0xe1],
    [0xa5, 0xd2, 0x69, 0xb4, 0x5a, 0x2d, 0x96, 0x4b],
    [0xff, 0x88, 0x88, 0x88, 0xff, 0x88, 0x88, 0x88],
    [0x81, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x81],
    [0xcc, 0x33, 0xcc, 0x33, 0xcc, 0x33, 0xcc, 0x33],
    [0x80, 0x00, 0x08, 0x00, 0x80, 0x00, 0x08, 0x00],
    [0x88, 0x00, 0x22, 0x00, 0x88, 0x00, 0x22, 0x00],
    [0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55],
];

pub fn is_rip(data: &[u8]) -> bool {
    data.windows(2).take(80).any(|window| window == b"!|") || data.starts_with(b"|1\x1b")
}

pub fn parse(data: &[u8], width_override: Option<usize>) -> Result<Screen, String> {
    if let Some(width) = width_override
        && width != WIDTH
    {
        return Err(format!(
            "width override {width} does not match the RIPscrip width {WIDTH}"
        ));
    }
    if !is_rip(data) {
        return Err("invalid RIPscrip header; expected a !| command introducer".to_owned());
    }

    let mut parser = Parser::new(data);
    parser.run()?;
    let canvas = parser.canvas;
    Ok(Screen {
        width: WIDTH / 8,
        height: HEIGHT.div_ceil(16),
        cells: Vec::new(),
        glyph_height: 16,
        font: None,
        palette: Some(canvas.palette),
        utf8_supported: true,
        raster: Some(Raster {
            width: WIDTH,
            height: HEIGHT,
            pixels: canvas.pixels,
        }),
    })
}

struct Parser<'a> {
    data: &'a [u8],
    index: usize,
    commands: usize,
    terminated: bool,
    canvas: Canvas,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            index: 0,
            commands: 0,
            terminated: false,
            canvas: Canvas::new(),
        }
    }

    fn run(&mut self) -> Result<(), String> {
        while self.index < self.data.len() {
            if self.data[self.index] != b'|' {
                self.index += 1;
                continue;
            }
            self.index += 1;
            let first = self.byte("opcode")?;
            let second = if first == b'1' {
                Some(self.byte("level-one opcode")?)
            } else {
                None
            };
            self.commands += 1;
            if self.commands > MAX_COMMANDS {
                return Err(format!(
                    "RIPscrip exceeds the {MAX_COMMANDS} command safety limit"
                ));
            }

            match (first, second) {
                (b'#', None) => {
                    self.terminated = true;
                    break;
                }
                (b'*', None) => self.canvas.reset(),
                (b'W', None) => self.canvas.write_mode = self.word("write mode")? as u8,
                (b'Q', None) => {
                    let mut indices = [0_u8; 16];
                    for index in &mut indices {
                        let value = self.word("palette entry")?;
                        if value > 63 {
                            return Err(format!(
                                "invalid RIPscrip palette entry {value}; expected 0..=63"
                            ));
                        }
                        *index = value as u8;
                    }
                    self.canvas.set_palette(indices);
                }
                (b'a', None) => {
                    let color = self.word("palette color")? as usize;
                    let palette = self.word("palette entry")?;
                    if color >= 16 || palette > 63 {
                        return Err(format!(
                            "invalid RIPscrip palette assignment {color}={palette}; expected color 0..=15 and entry 0..=63"
                        ));
                    }
                    self.canvas.palette[color] = ega_color(palette as u8);
                }
                (b'=', None) => {
                    let style = self.word("line style")? as usize;
                    let pattern = self.integer("line pattern")? as u16;
                    let thickness = self.word("line thickness")? as usize;
                    if style >= LINE_PATTERNS.len() {
                        return Err(format!("unsupported RIPscrip line style {style}"));
                    }
                    if !(1..=64).contains(&thickness) {
                        return Err(format!("invalid RIPscrip line thickness {thickness}"));
                    }
                    self.canvas.line_pattern = if style == 4 {
                        pattern
                    } else {
                        LINE_PATTERNS[style]
                    };
                    self.canvas.line_thickness = thickness;
                }
                (b'c', None) => self.canvas.color = (self.word("color")? % 16) as u8,
                (b'S', None) => {
                    let style = self.word("fill style")? as usize;
                    let color = self.word("fill color")? as u8 % 16;
                    if style >= FILL_PATTERNS.len() {
                        return Err(format!("unsupported RIPscrip fill style {style}"));
                    }
                    self.canvas.fill_style = style;
                    self.canvas.fill_color = color;
                }
                (b's', None) => {
                    let mut pattern = [0_u8; 8];
                    for row in &mut pattern {
                        let value = self.word("user fill pattern row")?;
                        if value > 255 {
                            return Err(format!(
                                "invalid RIPscrip user fill pattern row {value}; expected 0..=255"
                            ));
                        }
                        *row = value as u8;
                    }
                    self.canvas.user_fill_pattern = pattern;
                    self.canvas.fill_style = 12;
                    self.canvas.fill_color = (self.word("user fill color")? % 16) as u8;
                }
                (b'L', None) => {
                    let start = self.point("line start")?;
                    let end = self.point("line end")?;
                    self.canvas.line(start, end);
                }
                (b'l', None) => {
                    let points = self.points("polyline")?;
                    self.canvas.polyline(&points);
                }
                (b'P', None) => {
                    let points = self.points("polygon")?;
                    self.canvas.polygon(&points);
                }
                (b'p', None) => {
                    let points = self.points("filled polygon")?;
                    self.canvas.fill_polygon(&points);
                }
                (b'F', None) => {
                    let point = self.point("flood fill position")?;
                    let border = self.word("flood fill border color")? as u8 % 16;
                    self.canvas.flood_fill(point, border);
                }
                (b'X', None) => {
                    let point = self.point("pixel")?;
                    self.canvas.put(point.0, point.1, self.canvas.color);
                }
                (b'B', None) => {
                    let rectangle = self.rectangle("bar")?;
                    self.canvas.bar(rectangle);
                }
                (b'R', None) => {
                    let rectangle = self.rectangle("rectangle")?;
                    self.canvas.rectangle_outline(rectangle);
                }
                (b'o', None) => {
                    let center = self.point("filled oval center")?;
                    let radius = self.point("filled oval radius")?;
                    self.canvas.fill_ellipse(center, radius);
                }
                (b'V', None) => {
                    let center = self.point("oval arc center")?;
                    let start = self.word("oval arc start angle")? as i32;
                    let end = self.word("oval arc end angle")? as i32;
                    let radius = self.point("oval arc radius")?;
                    self.canvas.ellipse(center, start, end, radius);
                }
                (b'I', None) => {
                    let center = self.point("pie center")?;
                    let start = self.word("pie start angle")? as i32;
                    let end = self.word("pie end angle")? as i32;
                    let radius = self.word("pie radius")? as i32;
                    self.canvas.pie(center, start, end, radius);
                }
                (b'Z', None) => {
                    let mut points = [(0_i32, 0_i32); 4];
                    for point in &mut points {
                        *point = self.point("Bezier point")?;
                    }
                    let segments = self.word("Bezier segment count")? as usize;
                    if !(1..=4096).contains(&segments) {
                        return Err(format!("invalid RIPscrip Bezier segment count {segments}"));
                    }
                    self.canvas.bezier(points, segments);
                }
                (b'Y', None) => {
                    self.canvas.font = self.word("font")? as u8;
                    self.canvas.text_direction = self.word("text direction")? as u8;
                    self.canvas.character_size = self.word("character size")? as usize;
                    let _reserved = self.word("font reserved value")?;
                    if self.canvas.font > 10 {
                        return Err(format!("unsupported RIPscrip font {}", self.canvas.font));
                    }
                    if self.canvas.text_direction > 1 {
                        return Err(format!(
                            "unsupported RIPscrip text direction {}",
                            self.canvas.text_direction
                        ));
                    }
                    if !(1..=10).contains(&self.canvas.character_size) {
                        return Err(format!(
                            "invalid RIPscrip character size {}",
                            self.canvas.character_size
                        ));
                    }
                }
                (b'@', None) => {
                    let point = self.point("text position")?;
                    let text = self.string()?;
                    self.canvas.text(point, &text);
                }
                (b'w', None) => {}
                (b'1', Some(b'K' | 0x1b)) => {}
                (b'1', Some(b'C')) => {
                    let rectangle = self.rectangle("captured image")?;
                    let _reserved = self.number("captured image reserved value")?;
                    self.canvas.capture(rectangle);
                }
                (b'1', Some(b'P')) => {
                    let point = self.point("image position")?;
                    let mode = self.word("image write mode")? as u8;
                    let _reserved = self.number("image reserved value")?;
                    if mode > 4 {
                        return Err(format!("unsupported RIPscrip image write mode {mode}"));
                    }
                    self.canvas.paste(point, mode)?;
                }
                _ => {
                    let opcode = match second {
                        Some(value) if value.is_ascii_graphic() => {
                            format!("{}{}", char::from(first), char::from(value))
                        }
                        Some(value) => format!("{}\\x{value:02X}", char::from(first)),
                        None => char::from(first).to_string(),
                    };
                    return Err(format!("unsupported RIPscrip opcode {opcode}"));
                }
            }
        }
        if !self.terminated {
            return Err("truncated RIPscrip input; missing |# terminator".to_owned());
        }
        Ok(())
    }

    fn byte(&mut self, field: &str) -> Result<u8, String> {
        let mut byte = *self
            .data
            .get(self.index)
            .ok_or_else(|| format!("truncated RIPscrip {field}"))?;
        self.index += 1;
        if byte == b'\\' {
            byte = *self
                .data
                .get(self.index)
                .ok_or_else(|| format!("truncated RIPscrip {field}"))?;
            self.index += 1;
            while matches!(byte, b'\r' | b'\n') {
                byte = *self
                    .data
                    .get(self.index)
                    .ok_or_else(|| format!("truncated RIPscrip {field}"))?;
                self.index += 1;
            }
        }
        Ok(byte)
    }

    fn number(&mut self, field: &str) -> Result<u16, String> {
        let byte = self.byte(field)?;
        match byte {
            b'0'..=b'9' => Ok(u16::from(byte - b'0')),
            b'A'..=b'Z' => Ok(u16::from(byte - b'A' + 10)),
            _ => Err(format!(
                "invalid RIPscrip {field} digit 0x{byte:02x}; expected base-36"
            )),
        }
    }

    fn word(&mut self, field: &str) -> Result<u16, String> {
        Ok(self.number(field)? * 36 + self.number(field)?)
    }

    fn integer(&mut self, field: &str) -> Result<u32, String> {
        Ok(u32::from(self.word(field)?) * 1296 + u32::from(self.word(field)?))
    }

    fn point(&mut self, field: &str) -> Result<(i32, i32), String> {
        Ok((i32::from(self.word(field)?), i32::from(self.word(field)?)))
    }

    fn rectangle(&mut self, field: &str) -> Result<(i32, i32, i32, i32), String> {
        let start = self.point(field)?;
        let end = self.point(field)?;
        Ok((start.0, start.1, end.0, end.1))
    }

    fn points(&mut self, field: &str) -> Result<Vec<(i32, i32)>, String> {
        let count = self.word(&format!("{field} point count"))? as usize;
        if count > 4096 {
            return Err(format!(
                "invalid RIPscrip {field} point count {count}; expected 0..=4096"
            ));
        }
        (0..count).map(|_| self.point(field)).collect()
    }

    fn string(&mut self) -> Result<Vec<u8>, String> {
        let mut output = Vec::new();
        while self.index < self.data.len() && !matches!(self.data[self.index], b'|' | b'\r' | b'\n')
        {
            if output.len() >= 4096 {
                return Err("RIPscrip text exceeds the 4096-byte safety limit".to_owned());
            }
            output.push(self.byte("text")?);
        }
        while self.index < self.data.len() && matches!(self.data[self.index], b'\r' | b'\n') {
            self.index += 1;
        }
        Ok(output)
    }
}

struct Capture {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

struct Canvas {
    pixels: Vec<u8>,
    palette: [[u8; 3]; 16],
    color: u8,
    background: u8,
    fill_color: u8,
    fill_style: usize,
    user_fill_pattern: [u8; 8],
    line_pattern: u16,
    line_thickness: usize,
    write_mode: u8,
    font: u8,
    text_direction: u8,
    character_size: usize,
    captured: Option<Capture>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT],
            palette: default_palette(),
            color: 7,
            background: 0,
            fill_color: 0,
            fill_style: 1,
            user_fill_pattern: [0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55],
            line_pattern: 0xffff,
            line_thickness: 1,
            write_mode: 0,
            font: 2,
            text_direction: 0,
            character_size: 4,
            captured: None,
        }
    }

    fn reset(&mut self) {
        self.pixels.fill(0);
        self.color = 7;
        self.background = 0;
        self.fill_color = 0;
        self.fill_style = 1;
        self.user_fill_pattern = [0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55];
        self.line_pattern = 0xffff;
        self.line_thickness = 1;
        self.write_mode = 0;
        self.palette = default_palette();
    }

    fn set_palette(&mut self, indices: [u8; 16]) {
        for (output, index) in self.palette.iter_mut().zip(indices) {
            *output = ega_color(index);
        }
    }

    fn put(&mut self, x: i32, y: i32, color: u8) {
        self.put_mode(x, y, color, self.write_mode);
    }

    fn put_mode(&mut self, x: i32, y: i32, color: u8, mode: u8) {
        if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
            return;
        }
        let pixel = &mut self.pixels[y as usize * WIDTH + x as usize];
        let color = color & 0x0f;
        *pixel = match mode {
            0 => color,
            1 => *pixel ^ color,
            2 => *pixel | color,
            3 => *pixel & color,
            4 => *pixel & !color,
            _ => *pixel,
        } & 0x0f;
    }

    fn pattern_bit(&self, offset: i32) -> bool {
        let index = offset.unsigned_abs() as usize % 16;
        let byte = if index < 8 {
            (self.line_pattern >> 8) as u8
        } else {
            self.line_pattern as u8
        };
        byte & (1 << (index % 8)) != 0
    }

    fn line(&mut self, start: (i32, i32), end: (i32, i32)) {
        let y_delta = (end.1 - start.1).abs();
        let x_delta = (end.0 - start.0).abs();
        let mut offset = 0;
        if x_delta == 0 {
            self.fill_y(start.0, start.1.min(end.1), y_delta + 1, &mut offset);
        } else if y_delta == 0 {
            self.fill_x(start.1, start.0.min(end.0), x_delta + 1, &mut offset);
        } else if x_delta >= y_delta {
            let (mut position, step) = if start.1 < end.1 {
                (start, if start.0 > end.0 { -1 } else { 1 })
            } else {
                (end, if end.0 > start.0 { -1 } else { 1 })
            };
            let whole_step = x_delta / y_delta * step;
            let mut adjust_up = x_delta % y_delta;
            let adjust_down = y_delta * 2;
            let mut error = adjust_up - adjust_down;
            adjust_up *= 2;
            let mut start_length = whole_step / 2 + step;
            let end_length = start_length;
            if adjust_up == 0 && whole_step & 1 == 0 {
                start_length -= step;
            }
            if whole_step & 1 != 0 {
                error += y_delta;
            }
            self.fill_x(position.1, position.0, start_length, &mut offset);
            position.0 += start_length;
            position.1 += 1;
            for _ in 0..y_delta - 1 {
                let mut run_length = whole_step;
                error += adjust_up;
                if error > 0 {
                    run_length += step;
                    error -= adjust_down;
                }
                self.fill_x(position.1, position.0, run_length, &mut offset);
                position.0 += run_length;
                position.1 += 1;
            }
            self.fill_x(position.1, position.0, end_length, &mut offset);
        } else {
            let (mut position, advance) = if start.1 < end.1 {
                (start, if start.0 > end.0 { -1 } else { 1 })
            } else {
                (end, if end.0 > start.0 { -1 } else { 1 })
            };
            let whole_step = y_delta / x_delta;
            let mut adjust_up = y_delta % x_delta;
            let adjust_down = x_delta * 2;
            let mut error = adjust_up - adjust_down;
            adjust_up *= 2;
            let mut start_length = whole_step / 2 + 1;
            let end_length = start_length;
            if adjust_up == 0 && whole_step & 1 == 0 {
                start_length -= 1;
            }
            if whole_step & 1 != 0 {
                error += x_delta;
            }
            self.fill_y(position.0, position.1, start_length, &mut offset);
            position.1 += start_length;
            position.0 += advance;
            for _ in 0..x_delta - 1 {
                let mut run_length = whole_step;
                error += adjust_up;
                if error > 0 {
                    run_length += 1;
                    error -= adjust_down;
                }
                self.fill_y(position.0, position.1, run_length, &mut offset);
                position.1 += run_length;
                position.0 += advance;
            }
            self.fill_y(position.0, position.1, end_length, &mut offset);
        }
    }

    fn polyline(&mut self, points: &[(i32, i32)]) {
        for pair in points.windows(2) {
            self.line(pair[0], pair[1]);
        }
    }

    fn polygon(&mut self, points: &[(i32, i32)]) {
        self.polyline(points);
        if points.len() > 1 {
            self.line(points[points.len() - 1], points[0]);
        }
    }

    fn fill_polygon(&mut self, points: &[(i32, i32)]) {
        if points.len() < 3 {
            self.polyline(points);
            return;
        }
        let top = points.iter().map(|point| point.1).min().unwrap_or(0).max(0);
        let bottom = points
            .iter()
            .map(|point| point.1)
            .max()
            .unwrap_or(0)
            .min(HEIGHT as i32 - 1);
        for y in top..=bottom {
            let mut intersections = Vec::new();
            for index in 0..points.len() {
                let first = points[index];
                let second = points[(index + 1) % points.len()];
                if (first.1 <= y && second.1 > y) || (second.1 <= y && first.1 > y) {
                    let x = first.0 + (y - first.1) * (second.0 - first.0) / (second.1 - first.1);
                    intersections.push(x);
                }
            }
            intersections.sort_unstable();
            for pair in intersections.chunks_exact(2) {
                for x in pair[0]..=pair[1] {
                    self.fill_pixel(x, y);
                }
            }
        }
        self.polygon(points);
    }

    fn flood_fill(&mut self, start: (i32, i32), border: u8) {
        if start.0 < 0 || start.1 < 0 || start.0 >= WIDTH as i32 || start.1 >= HEIGHT as i32 {
            return;
        }
        let mut pending = std::collections::VecDeque::from([start]);
        let mut visited = vec![false; WIDTH * HEIGHT];
        while let Some((x, y)) = pending.pop_front() {
            if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
                continue;
            }
            let index = y as usize * WIDTH + x as usize;
            if visited[index] || self.pixels[index] == border {
                continue;
            }
            visited[index] = true;
            self.fill_pixel(x, y);
            pending.extend([(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]);
        }
    }

    fn fill_x(&mut self, y: i32, start_x: i32, count: i32, offset: &mut i32) {
        let start_y = y - self.line_thickness as i32 / 2;
        let end_y = start_y + self.line_thickness as i32 - 1;
        let mut end_x = start_x + count;
        if count > 0 {
            end_x -= 1;
        } else {
            end_x += 1;
            *offset -= count;
        }
        let increment = if count >= 0 { 1 } else { -1 };
        let (left, right) = ordered(start_x, end_x);
        for x in left..=right {
            if self.pattern_bit(*offset) {
                for current_y in start_y..=end_y {
                    self.put(x, current_y, self.color);
                }
            }
            *offset += increment;
        }
        if count < 0 {
            *offset -= count;
        }
    }

    fn fill_y(&mut self, x: i32, start_y: i32, count: i32, offset: &mut i32) {
        let start_x = x - self.line_thickness as i32 / 2;
        let end_x = start_x + self.line_thickness as i32 - 1;
        let mut end_y = start_y + count;
        if count > 0 {
            end_y -= 1;
        } else {
            end_y += 1;
            *offset += count;
        }
        let (top, bottom) = ordered(start_y, end_y);
        for y in top..=bottom {
            if self.pattern_bit(*offset) {
                for current_x in start_x..=end_x {
                    self.put(current_x, y, self.color);
                }
            }
            *offset += 1;
        }
        if count < 0 {
            *offset += count;
        }
    }

    fn fill_pixel(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
            return;
        }
        let pattern = if self.fill_style == 12 {
            self.user_fill_pattern
        } else {
            FILL_PATTERNS[self.fill_style]
        };
        let mask = 0x80 >> (x as usize % 8);
        let color = if pattern[y as usize % 8] & mask != 0 {
            self.fill_color
        } else {
            self.background
        };
        self.pixels[y as usize * WIDTH + x as usize] = color;
    }

    fn bar(&mut self, rectangle: (i32, i32, i32, i32)) {
        let (left, right) = ordered(rectangle.0, rectangle.2);
        let (top, bottom) = ordered(rectangle.1, rectangle.3);
        for y in top.max(0)..=bottom.min(HEIGHT as i32 - 1) {
            for x in left.max(0)..=right.min(WIDTH as i32 - 1) {
                self.fill_pixel(x, y);
            }
        }
    }

    fn rectangle_outline(&mut self, rectangle: (i32, i32, i32, i32)) {
        let (left, right) = ordered(rectangle.0, rectangle.2);
        let (top, bottom) = ordered(rectangle.1, rectangle.3);
        self.line((left, top), (right, top));
        self.line((right, top), (right, bottom));
        self.line((right, bottom), (left, bottom));
        self.line((left, bottom), (left, top));
    }

    fn scan_ellipse(
        &self,
        center: (i32, i32),
        mut start: i32,
        mut end: i32,
        radius: (i32, i32),
    ) -> Vec<Vec<i32>> {
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        let radius_x = radius.0.max(1);
        let radius_y = radius.1.max(1);
        let diameter_x = i64::from(radius_x * 2);
        let diameter_y = i64::from(radius_y * 2);
        let b1 = diameter_y & 1;
        let mut stop_x = 4 * (1 - diameter_x) * diameter_y * diameter_y;
        let mut stop_y = 4 * (b1 + 1) * diameter_x * diameter_x;
        let mut error = stop_x + stop_y + b1 * diameter_x * diameter_x;
        let mut x_offset = radius_x;
        let mut y_offset = 0_i32;
        let increment_x = 8 * diameter_x * diameter_x;
        let increment_y = 8 * diameter_y * diameter_y;
        let aspect = f64::from(radius_x) / f64::from(radius_y);
        let horizontal_angle = if radius_x < radius_y {
            90.0 - 45.0 * aspect
        } else {
            45.0 / aspect
        };
        let mut rows = vec![Vec::new(); HEIGHT + 2];

        loop {
            let twice = 2 * error;
            let angle = (f64::from(y_offset) * aspect / f64::from(x_offset)).atan() * 180.0 / PI;
            self.symmetry_scan(
                center,
                start,
                end,
                (x_offset, y_offset),
                angle.round() as i32,
                angle <= horizontal_angle,
                &mut rows,
            );
            if (angle - horizontal_angle).abs() < 1.0 {
                self.symmetry_scan(
                    center,
                    start,
                    end,
                    (x_offset, y_offset),
                    angle.round() as i32,
                    angle > horizontal_angle,
                    &mut rows,
                );
            }
            if twice <= stop_y {
                y_offset += 1;
                stop_y += increment_x;
                error += stop_y;
            }
            if twice >= stop_x {
                x_offset -= 1;
                stop_x += increment_y;
                error += stop_x;
            }
            if x_offset < 0 {
                break;
            }
        }

        x_offset += 1;
        while y_offset < radius_y {
            let angle = (f64::from(y_offset) * aspect / f64::from(x_offset)).atan() * 180.0 / PI;
            self.symmetry_scan(
                center,
                start,
                end,
                (x_offset, y_offset),
                angle.round() as i32,
                angle <= horizontal_angle,
                &mut rows,
            );
            y_offset += 1;
        }
        rows
    }

    #[allow(clippy::too_many_arguments)]
    fn symmetry_scan(
        &self,
        center: (i32, i32),
        start: i32,
        end: i32,
        offset: (i32, i32),
        angle: i32,
        horizontal: bool,
        rows: &mut [Vec<i32>],
    ) {
        let candidates = [
            (angle, center.0 + offset.0, center.1 - offset.1),
            (180 - angle, center.0 - offset.0, center.1 - offset.1),
            (180 + angle, center.0 - offset.0, center.1 + offset.1),
            (360 - angle, center.0 + offset.0, center.1 + offset.1),
        ];
        let before = (self.line_thickness / 2) as i32;
        for (candidate_angle, x, y) in candidates {
            if candidate_angle < start || candidate_angle > end {
                continue;
            }
            if self.line_thickness == 1 {
                add_scan(rows, x, y);
            } else {
                for thickness in 0..self.line_thickness as i32 {
                    if horizontal {
                        add_scan(rows, x + thickness - before, y);
                    } else {
                        add_scan(rows, x, y + thickness - before);
                    }
                }
            }
        }
    }

    fn draw_scan(&mut self, rows: &[Vec<i32>]) {
        for (index, row) in rows.iter().enumerate() {
            let y = index as i32 - 1;
            let mut points = row.clone();
            points.sort_unstable();
            points.dedup();
            for x in points {
                self.put(x, y, self.color);
            }
        }
    }

    fn fill_scan(&mut self, rows: &[Vec<i32>]) {
        for y in 0..HEIGHT as i32 {
            let row = &rows[y as usize + 1];
            if let (Some(left), Some(right)) = (row.iter().min(), row.iter().max()) {
                for x in *left..=*right {
                    self.fill_pixel(x, y);
                }
            }
        }
    }

    fn ellipse(&mut self, center: (i32, i32), start: i32, end: i32, radius: (i32, i32)) {
        if start > end {
            let mut rows = self.scan_ellipse(center, 0, end, radius);
            let other = self.scan_ellipse(center, start, 360, radius);
            for (row, other) in rows.iter_mut().zip(other) {
                row.extend(other);
            }
            self.draw_scan(&rows);
        } else {
            let rows = self.scan_ellipse(center, start, end, radius);
            self.draw_scan(&rows);
        }
    }

    fn fill_ellipse(&mut self, center: (i32, i32), radius: (i32, i32)) {
        let rows = self.scan_ellipse(center, 0, 360, radius);
        self.fill_scan(&rows);
        self.draw_scan(&rows);
    }

    fn pie(&mut self, center: (i32, i32), start: i32, end: i32, radius: i32) {
        let radius_y = (f64::from(radius) * (350.0 / 480.0 * 1.06)).trunc() as i32;
        let mut rows = if start > end {
            let mut first = self.scan_ellipse(center, 0, end, (radius, radius_y));
            let second = self.scan_ellipse(center, start, 360, (radius, radius_y));
            for (row, second) in first.iter_mut().zip(second) {
                row.extend(second);
            }
            first
        } else {
            self.scan_ellipse(center, start, end, (radius, radius_y))
        };
        let start_point = angle_point(center, start, (radius, radius_y));
        let end_point = angle_point(center, end, (radius, radius_y));
        scan_line(center, start_point, &mut rows);
        scan_line(center, end_point, &mut rows);
        if self.fill_style != 0 {
            self.fill_scan(&rows);
        }
        self.draw_scan(&rows);
        self.line(center, start_point);
        self.line(center, end_point);
    }

    fn bezier(&mut self, points: [(i32, i32); 4], segments: usize) {
        let mut last = points[0];
        for segment in 0..segments {
            let t = segment as f64 / segments as f64;
            let inverse = 1.0 - t;
            let x = (inverse.powi(3) * f64::from(points[0].0)
                + 3.0 * t * inverse.powi(2) * f64::from(points[1].0)
                + 3.0 * t * t * inverse * f64::from(points[2].0)
                + t.powi(3) * f64::from(points[3].0)) as i32;
            let y = (inverse.powi(3) * f64::from(points[0].1)
                + 3.0 * t * inverse.powi(2) * f64::from(points[1].1)
                + 3.0 * t * t * inverse * f64::from(points[2].1)
                + t.powi(3) * f64::from(points[3].1)) as i32;
            self.line(last, (x, y));
            last = (x, y);
        }
        self.line(last, points[3]);
    }

    fn capture(&mut self, rectangle: (i32, i32, i32, i32)) {
        let (left, right) = ordered(rectangle.0, rectangle.2);
        let (top, bottom) = ordered(rectangle.1, rectangle.3);
        let width = (right - left + 1).max(0) as usize;
        let height = (bottom - top + 1).max(0) as usize;
        let mut pixels = Vec::with_capacity(width.saturating_mul(height));
        for y in top..=bottom {
            for x in left..=right {
                let color = if x >= 0 && y >= 0 && x < WIDTH as i32 && y < HEIGHT as i32 {
                    self.pixels[y as usize * WIDTH + x as usize]
                } else {
                    0
                };
                pixels.push(color);
            }
        }
        self.captured = Some(Capture {
            width,
            height,
            pixels,
        });
    }

    fn paste(&mut self, point: (i32, i32), mode: u8) -> Result<(), String> {
        let Some(captured) = self.captured.take() else {
            return Err("RIPscrip image paste appeared before image capture".to_owned());
        };
        for y in 0..captured.height {
            for x in 0..captured.width {
                self.put_mode(
                    point.0 + x as i32,
                    point.1 + y as i32,
                    captured.pixels[y * captured.width + x],
                    mode,
                );
            }
        }
        self.captured = Some(captured);
        Ok(())
    }

    fn text(&mut self, point: (i32, i32), text: &[u8]) {
        let old_pattern = self.line_pattern;
        let old_thickness = self.line_thickness;
        self.line_pattern = 0xffff;
        self.line_thickness = 1;

        let mut cursor = point;
        if self.text_direction == 1 {
            cursor.1 += self.text_width(text);
        }
        for &character in text {
            let width = if self.font == 0 {
                self.bitmap_character(cursor, character)
            } else {
                self.stroke_character(cursor, character)
            };
            if self.text_direction == 0 {
                cursor.0 += width;
            } else {
                cursor.1 -= width;
            }
        }

        self.line_pattern = old_pattern;
        self.line_thickness = old_thickness;
    }

    fn text_width(&self, text: &[u8]) -> i32 {
        if self.font == 0 {
            return text.len() as i32 * 8 * self.character_size as i32;
        }
        let font = bgi_font::stroke_font(self.font).expect("validated RIP font index");
        let width: i32 = text
            .iter()
            .filter_map(|&character| font.glyph(character))
            .map(|glyph| glyph.width)
            .sum();
        bgi_font::scale(width, self.character_size)
    }

    fn bitmap_character(&mut self, point: (i32, i32), character: u8) -> i32 {
        let glyphs = bgi_font::bitmap();
        let offset = usize::from(character) * 8;
        let scale = self.character_size as i32;
        for row in 0..8_i32 {
            let bits = glyphs[offset + row as usize];
            for column in 0..8_i32 {
                if bits & (0x80 >> column) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let (x, y) = if self.text_direction == 0 {
                            (point.0 + column * scale + sx, point.1 + row * scale + sy)
                        } else {
                            (point.0 + row * scale + sx, point.1 - column * scale - sy)
                        };
                        self.put(x, y, self.color);
                    }
                }
            }
        }
        8 * scale
    }

    fn stroke_character(&mut self, point: (i32, i32), character: u8) -> i32 {
        let font = bgi_font::stroke_font(self.font).expect("validated RIP font index");
        let Some(glyph) = font.glyph(character) else {
            return 0;
        };
        let mut current = None;
        for stroke in &glyph.strokes {
            let position = if self.text_direction == 0 {
                (
                    point.0 + bgi_font::scale(stroke.x, self.character_size),
                    point.1 + bgi_font::scale(stroke.y, self.character_size),
                )
            } else {
                (
                    point.0 + bgi_font::scale(stroke.y, self.character_size),
                    point.1 - bgi_font::scale(stroke.x, self.character_size),
                )
            };
            match stroke.kind {
                bgi_font::StrokeKind::Move => current = Some(position),
                bgi_font::StrokeKind::Line => {
                    if let Some(start) = current {
                        self.line(start, position);
                    }
                    current = Some(position);
                }
            }
        }
        bgi_font::scale(glyph.width, self.character_size)
    }
}

fn add_scan(rows: &mut [Vec<i32>], x: i32, y: i32) {
    if (-1..=HEIGHT as i32).contains(&y) {
        rows[(y + 1) as usize].push(x);
    }
}

fn scan_line(start: (i32, i32), end: (i32, i32), rows: &mut [Vec<i32>]) {
    let (mut x, mut y) = start;
    let dx = (end.0 - x).abs();
    let sx = if x < end.0 { 1 } else { -1 };
    let dy = -(end.1 - y).abs();
    let sy = if y < end.1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        add_scan(rows, x, y);
        if (x, y) == end {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            x += sx;
        }
        if twice <= dx {
            error += dx;
            y += sy;
        }
    }
}

fn angle_point(center: (i32, i32), angle: i32, radius: (i32, i32)) -> (i32, i32) {
    let radians = f64::from(angle) * PI / 180.0;
    (
        center.0 + (radians.cos() * f64::from(radius.0)).round() as i32,
        center.1 - (radians.sin() * f64::from(radius.1)).round() as i32,
    )
}

fn ordered(left: i32, right: i32) -> (i32, i32) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn ega_color(index: u8) -> [u8; 3] {
    let component = |primary: u8, secondary: u8| {
        u8::from(index & primary != 0) * 0xa8 + u8::from(index & secondary != 0) * 0x54
    };
    [component(4, 32), component(2, 16), component(1, 8)]
}

fn default_palette() -> [[u8; 3]; 16] {
    [
        [0, 0, 0],
        [0, 0, 171],
        [0, 171, 0],
        [0, 171, 171],
        [171, 0, 0],
        [171, 0, 171],
        [171, 87, 0],
        [171, 171, 171],
        [87, 87, 87],
        [87, 87, 255],
        [87, 255, 87],
        [87, 255, 255],
        [255, 87, 87],
        [255, 87, 255],
        [255, 255, 87],
        [255, 255, 255],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_basic_rip_commands() {
        let screen = parse(b"!|*|c0F|L00000A0A|#", None).unwrap();
        let png = crate::encode_screen(&screen, 0, screen.height).unwrap();
        assert_eq!(&png[16..24], &[0, 0, 2, 128, 0, 0, 1, 94]);
        let raster = screen.raster.unwrap();
        assert_eq!((raster.width, raster.height), (640, 350));
        assert_eq!(raster.pixels[0], 15);
        assert_eq!(raster.pixels[10 * WIDTH + 10], 15);
    }

    #[test]
    fn rejects_truncated_and_unknown_commands() {
        assert!(parse(b"!|c0", None).unwrap_err().contains("truncated"));
        assert!(
            parse(b"!|?00|#", None)
                .unwrap_err()
                .contains("unsupported RIPscrip opcode ?")
        );
    }

    #[test]
    fn renders_polygons_polylines_and_flood_fill() {
        let polygon = parse(b"!|*|S010F|p030A0A0K0A0A0K|#", None).unwrap();
        assert_eq!(polygon.screen_pixel(12, 12), 15, "filled polygon interior");

        let flood = parse(
            b"!|*|c0F|L05050F05|L0F050F0F|L0F0F050F|L050F0505|S010C|F0A0A0F|#",
            None,
        )
        .unwrap();
        assert_eq!(flood.screen_pixel(10, 10), 12, "flood-filled interior");

        let pattern = parse(b"!|*|s73000000000000000F|B00000707|#", None).unwrap();
        assert_eq!(pattern.screen_pixel(0, 0), 15, "set user-pattern bit");
        assert_eq!(pattern.screen_pixel(0, 1), 0, "clear user-pattern bit");

        let rectangle = parse(b"!|*|c0F|R05050F0F|#", None).unwrap();
        assert_eq!(rectangle.screen_pixel(5, 5), 15, "rectangle corner");
        assert_eq!(rectangle.screen_pixel(10, 5), 15, "rectangle edge");
        assert_eq!(rectangle.screen_pixel(10, 10), 0, "rectangle interior");
    }

    #[test]
    fn renders_bitmap_and_proportional_bgi_text() {
        let bitmap = parse(b"!|*|c0F|Y00000100|@0A0AA|#", None).unwrap();
        let triplex = parse(b"!|*|c0F|Y01000100|@0A0AA|#", None).unwrap();
        let lit_pixels = |screen: Screen| {
            screen
                .raster
                .unwrap()
                .pixels
                .into_iter()
                .filter(|&pixel| pixel != 0)
                .count()
        };
        let bitmap_pixels = lit_pixels(bitmap);
        let triplex_pixels = lit_pixels(triplex);
        assert!(bitmap_pixels > 0);
        assert!(triplex_pixels > 0);
        assert_ne!(bitmap_pixels, triplex_pixels);

        let little = parse(b"!|*|c0F|Y02000600|@0A0Aw|#", None).unwrap();
        let lit_rows: Vec<_> = little
            .raster
            .unwrap()
            .pixels
            .chunks_exact(WIDTH)
            .enumerate()
            .filter_map(|(row, pixels)| pixels.iter().any(|&pixel| pixel != 0).then_some(row))
            .collect();
        assert_eq!(lit_rows.first(), Some(&18));
        assert_eq!(lit_rows.last(), Some(&25));
    }

    #[test]
    fn validates_width_and_header() {
        assert!(parse(b"not rip", None).unwrap_err().contains("header"));
        assert!(parse(b"!|#", Some(80)).unwrap_err().contains("width 640"));
    }

    trait ScreenPixel {
        fn screen_pixel(&self, x: usize, y: usize) -> u8;
    }

    impl ScreenPixel for Screen {
        fn screen_pixel(&self, x: usize, y: usize) -> u8 {
            self.raster.as_ref().unwrap().pixels[y * WIDTH + x]
        }
    }
}
