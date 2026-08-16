use std::{
    env, fs,
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

mod terminal;

const DEFAULT_SLOW_DELAY_MS: u64 = 35;
const DEFAULT_BAUD_ROW_DELAY_MS: u64 = 25;
const MAX_DELAY_MS: u64 = 10_000;
const SUGGESTED_ANIMATION_RATES: [u64; 7] = [2_400, 9_600, 14_400, 28_800, 38_400, 57_600, 115_200];

struct Options {
    width: Option<usize>,
    chunk_lines: usize,
    output: Option<PathBuf>,
    apng: Option<PathBuf>,
    gif: Option<PathBuf>,
    asciimation: bool,
    kitty: bool,
    fit: bool,
    delay: Option<Duration>,
    baud: Option<u64>,
    scale: usize,
    sauce: bool,
    info: bool,
    list_archive: bool,
    files: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(message) => {
            eprintln!("bbcat: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let Some(mut options) = parse_args()? else {
        return Ok(ExitCode::SUCCESS);
    };
    if options.files.is_empty() {
        if io::stdin().is_terminal() {
            print_help();
            return Ok(ExitCode::SUCCESS);
        }
        options.files.push("-".to_owned());
    }
    if options.list_archive {
        if options.files.len() != 1 {
            return Err("--list-archive requires exactly one ZIP archive".to_owned());
        }
        if options.width.is_some()
            || options.output.is_some()
            || options.apng.is_some()
            || options.gif.is_some()
            || options.asciimation
            || options.kitty
            || options.fit
            || options.delay.is_some()
            || options.baud.is_some()
            || options.scale > 1
            || options.sauce
            || options.info
        {
            return Err(
                "--list-archive cannot be combined with rendering or output options".to_owned(),
            );
        }
        let data = read_file(&options.files[0])?;
        if !bbcat::is_zip(&data) {
            return Err(format!("{}: not a ZIP archive", options.files[0]));
        }
        let entries = bbcat::list_archive_artworks(&data)
            .map_err(|error| format!("{}: {error}", options.files[0]))?;
        let mut stdout = io::stdout().lock();
        for entry in entries {
            use std::io::Write;
            writeln!(stdout, "{entry}").map_err(|error| format!("stdout: {error}"))?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    let image_output = options.output.is_some() || options.apng.is_some() || options.gif.is_some();
    if [
        options.output.as_ref(),
        options.apng.as_ref(),
        options.gif.as_ref(),
    ]
    .into_iter()
    .flatten()
    .count()
        > 1
    {
        return Err("choose only one of --output, --apng, or --gif".to_owned());
    }
    if image_output && options.files.len() != 1 {
        return Err("image output requires exactly one input file".to_owned());
    }
    if options.asciimation && options.files.len() != 1 {
        return Err("--asciimation requires exactly one input file".to_owned());
    }
    if options.info && options.files.len() != 1 {
        return Err("--info requires exactly one input file".to_owned());
    }
    if options.info
        && (image_output
            || options.kitty
            || options.fit
            || options.delay.is_some()
            || options.baud.is_some()
            || options.scale > 1
            || options.sauce)
    {
        return Err("--info cannot be combined with rendering or output options".to_owned());
    }
    if options.asciimation
        && (image_output
            || options.kitty
            || options.fit
            || options.delay.is_some()
            || options.baud.is_some()
            || options.scale > 1
            || options.sauce
            || options.width.is_some())
    {
        return Err("--asciimation cannot be combined with rendering or output options".to_owned());
    }
    if image_output && options.delay.is_some() {
        return Err("--slow/--delay cannot be used with image output".to_owned());
    }
    if options.output.is_some() && options.baud.is_some() {
        return Err("--baud cannot be used with --output".to_owned());
    }
    if image_output && options.kitty {
        return Err("image output and --kitty cannot be used together".to_owned());
    }
    if options.kitty && options.baud.is_some() {
        return Err("--baud cannot be used with --kitty".to_owned());
    }
    if options.delay.is_some() && options.baud.is_some() {
        return Err("--slow/--delay and --baud cannot be used together".to_owned());
    }
    if image_output && options.sauce {
        return Err("--sauce cannot be used with image output".to_owned());
    }
    if options.fit && !options.kitty {
        return Err("--fit requires --kitty".to_owned());
    }
    if options.scale > 1 && !image_output && !options.kitty {
        return Err("--2x requires --kitty, --output FILE, --apng FILE, or --gif FILE".to_owned());
    }
    let stdout_is_terminal = io::stdout().is_terminal();
    if options.asciimation && !options.info && !stdout_is_terminal {
        return Err("--asciimation requires terminal stdout".to_owned());
    }
    if options.kitty && !stdout_is_terminal {
        return Err("--kitty requires terminal stdout".to_owned());
    }
    if options.kitty && !terminal::supports_kitty()? {
        return Err(
            "terminal does not support the Kitty graphics protocol; omit --kitty for UTF-8 output"
                .to_owned(),
        );
    }

    let terminal_columns = stdout_is_terminal.then(terminal::width).flatten();
    if options.kitty && terminal_columns.is_none() {
        return Err("cannot determine terminal width for Kitty output".to_owned());
    }
    let mut stdout = io::stdout().lock();
    if options.asciimation {
        let input = read_input(&options.files[0])?;
        let file = input.label;
        let data = input.data;
        let animation =
            bbcat::decode_asciimation(&data).map_err(|error| format!("{file}: {error}"))?;
        if options.info {
            write_asciimation_info(&mut stdout, &file, data.len(), &animation)?;
            return Ok(ExitCode::SUCCESS);
        }
        if let Some(columns) = terminal_columns
            && animation.width > columns
        {
            return Err(format!(
                "{file}: asciimation requires at least {} terminal columns",
                animation.width
            ));
        }
        bbcat::write_asciimation(&mut stdout, &animation)
            .map_err(|error| format!("{file}: {error}"))?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut input_error = false;
    for path in &options.files {
        let input = match read_input(path) {
            Ok(input) => input,
            Err(error) => {
                eprintln!("bbcat: {error}");
                input_error = true;
                continue;
            }
        };
        let file = input.label;
        let name = input.name;
        let data = input.data;
        // All input formats become the same Screen here. The remaining branches
        // differ only in how that screen is serialized for the requested target.
        let document = match bbcat::decode_with_options(
            &data,
            bbcat::DecodeOptions {
                file_name: Some(Path::new(&name)),
                width: options.width,
            },
        ) {
            Ok(document) => document,
            Err(error) => {
                eprintln!("bbcat: {file}: {error}");
                input_error = true;
                continue;
            }
        };
        if options.info {
            write_document_info(&mut stdout, &file, data.len(), &document)?;
            continue;
        }
        if options.delay.is_some() && document.animation.is_some() {
            eprintln!(
                "bbcat: {file}: use --baud to control animation speed; --slow/--delay reveal static art by row"
            );
            input_error = true;
            continue;
        }
        if let Some(path) = &options.apng {
            let animation = document
                .animation
                .as_ref()
                .ok_or_else(|| format!("{file}: --apng requires an animated ANSI or DDW input"))?;
            let apng = bbcat::encode_animation_apng(
                animation,
                options.baud.unwrap_or(bbcat::DEFAULT_ANIMATION_BAUD),
                options.scale,
            )
            .map_err(|error| format!("{file}: {error}"))?;
            write_png(&mut stdout, path, &apng)?;
        } else if let Some(path) = &options.gif {
            let animation = document
                .animation
                .as_ref()
                .ok_or_else(|| format!("{file}: --gif requires an animated ANSI or DDW input"))?;
            let gif = bbcat::encode_animation_gif(
                animation,
                options.baud.unwrap_or(bbcat::DEFAULT_ANIMATION_BAUD),
                options.scale,
            )
            .map_err(|error| format!("{file}: {error}"))?;
            write_png(&mut stdout, path, &gif)?;
        } else if let Some(path) = &options.output {
            // File output is a single PNG containing the complete screen.
            let png = bbcat::encode_screen_scaled(
                &document.screen,
                0,
                document.screen.height,
                options.scale,
            )
            .map_err(|error| format!("{file}: {error}"))?;
            write_png(&mut stdout, path, &png)?;
        } else if options.kitty {
            // Kitty output transports PNG chunks inside terminal escape sequences;
            // fit/crop, reveal delay, and scale select the appropriate wrapper.
            if let Some(delay) = options.delay {
                if let Some(columns) = terminal_columns {
                    if options.fit {
                        bbcat::write_screen_slow_scaled_fit(
                            &mut stdout,
                            &document.screen,
                            delay,
                            options.scale,
                            columns,
                        )
                        .map_err(|error| format!("{file}: {error}"))?;
                    } else {
                        bbcat::write_screen_slow_scaled_cropped(
                            &mut stdout,
                            &document.screen,
                            delay,
                            options.scale,
                            columns,
                        )
                        .map_err(|error| format!("{file}: {error}"))?;
                    }
                } else {
                    bbcat::write_screen_slow_scaled(
                        &mut stdout,
                        &document.screen,
                        delay,
                        options.scale,
                    )
                    .map_err(|error| format!("{file}: {error}"))?;
                }
            } else if let Some(columns) = terminal_columns {
                if options.fit {
                    bbcat::write_screen_scaled_fit(
                        &mut stdout,
                        &document.screen,
                        options.chunk_lines,
                        options.scale,
                        columns,
                    )
                    .map_err(|error| format!("{file}: {error}"))?;
                } else {
                    bbcat::write_screen_scaled_cropped(
                        &mut stdout,
                        &document.screen,
                        options.chunk_lines,
                        options.scale,
                        columns,
                    )
                    .map_err(|error| format!("{file}: {error}"))?;
                }
            } else {
                bbcat::write_screen_scaled(
                    &mut stdout,
                    &document.screen,
                    options.chunk_lines,
                    options.scale,
                )
                .map_err(|error| format!("{file}: {error}"))?;
            }
        } else if let Some(animation) = &document.animation
            && (stdout_is_terminal || options.baud.is_some())
        {
            let baud = options.baud.unwrap_or(bbcat::DEFAULT_ANIMATION_BAUD);
            bbcat::write_animation_at_baud(&mut stdout, animation, baud)
                .map_err(|error| format!("{file}: {error}"))?;
        } else if let Some(baud) = options.baud {
            // Static art has no frame timing. Reuse the smooth row reveal,
            // scaling its delay from the same 1X baud baseline as animation.
            let delay = baud_row_delay(baud);
            if let Some(columns) = terminal_columns {
                bbcat::write_text_slow_cropped(&mut stdout, &document.screen, delay, columns)
                    .map_err(|error| format!("{file}: {error}"))?;
            } else {
                bbcat::write_text_slow(&mut stdout, &document.screen, delay)
                    .map_err(|error| format!("{file}: {error}"))?;
            }
        } else if let Some(delay) = options.delay {
            // Plain terminal output maps CP437 cells to Unicode plus ANSI colors.
            if let Some(columns) = terminal_columns {
                bbcat::write_text_slow_cropped(&mut stdout, &document.screen, delay, columns)
                    .map_err(|error| format!("{file}: {error}"))?;
            } else {
                bbcat::write_text_slow(&mut stdout, &document.screen, delay)
                    .map_err(|error| format!("{file}: {error}"))?;
            }
        } else if let Some(columns) = terminal_columns {
            bbcat::write_text_cropped(&mut stdout, &document.screen, columns)
                .map_err(|error| format!("{file}: {error}"))?;
        } else {
            bbcat::write_text(&mut stdout, &document.screen)
                .map_err(|error| format!("{file}: {error}"))?;
        }
        if options.sauce {
            write_sauce(&mut stdout, document.sauce.as_ref())?;
        }
    }
    Ok(if input_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

struct Input {
    label: String,
    name: String,
    data: Vec<u8>,
}

fn read_input(path: &str) -> Result<Input, String> {
    let (archive_path, entry_name) = split_archive_spec(path);
    let data = read_file(archive_path)?;
    let zip_name = Path::new(archive_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"));
    if entry_name.is_some() && !bbcat::is_zip(&data) {
        return Err(format!("{archive_path}: not a ZIP archive"));
    }
    if bbcat::is_zip(&data) || zip_name {
        let entry = if let Some(name) = entry_name {
            if name.is_empty() {
                return Err(format!("{archive_path}: ZIP entry name is empty"));
            }
            bbcat::extract_archive_artwork(&data, name)
        } else {
            bbcat::extract_archive_preview(&data)
        }
        .map_err(|error| format!("{archive_path}: {error}"))?;
        return Ok(Input {
            label: format!("{archive_path}:{}", entry.name),
            name: entry.name,
            data: entry.data,
        });
    }
    Ok(Input {
        label: path.to_owned(),
        name: path.to_owned(),
        data,
    })
}

fn read_file(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        let mut data = Vec::new();
        io::stdin()
            .read_to_end(&mut data)
            .map_err(|error| format!("stdin: {error}"))?;
        Ok(data)
    } else {
        fs::read(path).map_err(|error| format!("{path}: {error}"))
    }
}

fn split_archive_spec(path: &str) -> (&str, Option<&str>) {
    let lowercase = path.to_ascii_lowercase();
    let Some(separator) = lowercase.find(".zip:") else {
        return (path, None);
    };
    let archive_end = separator + ".zip".len();
    (&path[..archive_end], Some(&path[archive_end + 1..]))
}

fn write_document_info<W: io::Write>(
    output: &mut W,
    file: &str,
    source_bytes: usize,
    document: &bbcat::Document,
) -> Result<(), String> {
    let info = document.info();
    let (pixel_width, pixel_height) = info
        .pixel_dimensions
        .map_or(("null".to_owned(), "null".to_owned()), |(width, height)| {
            (width.to_string(), height.to_string())
        });
    let frames = document
        .animation
        .as_ref()
        .map(|animation| {
            animation
                .frames
                .iter()
                .map(|frame| {
                    let duration = frame.duration.map_or("null".to_owned(), |duration| {
                        duration.as_millis().to_string()
                    });
                    format!(
                        "{{\"sourceBytes\":{},\"durationMs\":{duration}}}",
                        frame.source_bytes
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    let clear_on_finish = document
        .animation
        .as_ref()
        .is_some_and(|animation| animation.clear_on_finish);
    let sauce = sauce_json(info.sauce.as_ref());
    writeln!(
        output,
        concat!(
            "{{\n",
            "  \"file\": {},\n",
            "  \"sourceBytes\": {},\n",
            "  \"format\": {},\n",
            "  \"formatName\": {},\n",
            "  \"dimensions\": {{\"columns\":{},\"rows\":{},\"pixelWidth\":{},\"pixelHeight\":{},\"glyphWidth\":{},\"glyphHeight\":{}}},\n",
            "  \"raster\": {},\n",
            "  \"utf8Supported\": {},\n",
            "  \"embeddedFont\": {},\n",
            "  \"animation\": {{\"animated\":{},\"frameCount\":{},\"clearOnFinish\":{},\"frames\":[{}]}},\n",
            "  \"sauce\": {}\n",
            "}}"
        ),
        json_string(file),
        source_bytes,
        json_string(format_id(info.format)),
        json_string(&info.format.to_string()),
        info.columns,
        info.rows,
        pixel_width,
        pixel_height,
        info.glyph_dimensions.0,
        info.glyph_dimensions.1,
        info.raster,
        info.utf8_supported,
        info.embedded_font,
        info.animated,
        info.frame_count,
        clear_on_finish,
        frames,
        sauce,
    )
    .map_err(|error| format!("stdout: {error}"))
}

fn write_asciimation_info<W: io::Write>(
    output: &mut W,
    file: &str,
    source_bytes: usize,
    animation: &bbcat::Asciimation,
) -> Result<(), String> {
    let frames = animation
        .frames
        .iter()
        .map(|frame| {
            format!(
                "{{\"sourceBytes\":null,\"durationMs\":{}}}",
                frame.duration().as_millis()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    writeln!(
        output,
        concat!(
            "{{\n",
            "  \"file\": {},\n",
            "  \"sourceBytes\": {},\n",
            "  \"format\": \"asciimation\",\n",
            "  \"formatName\": \"ASCIImation\",\n",
            "  \"dimensions\": {{\"columns\":{},\"rows\":13,\"pixelWidth\":null,\"pixelHeight\":null,\"glyphWidth\":null,\"glyphHeight\":null}},\n",
            "  \"raster\": false,\n",
            "  \"utf8Supported\": true,\n",
            "  \"embeddedFont\": false,\n",
            "  \"animation\": {{\"animated\":true,\"frameCount\":{},\"clearOnFinish\":true,\"frames\":[{}]}},\n",
            "  \"sauce\": null\n",
            "}}"
        ),
        json_string(file),
        source_bytes,
        animation.width,
        animation.frames.len(),
        frames,
    )
    .map_err(|error| format!("stdout: {error}"))
}

fn sauce_json(sauce: Option<&bbcat::Sauce>) -> String {
    let Some(sauce) = sauce else {
        return "null".to_owned();
    };
    let letter_spacing = match sauce.letter_spacing {
        Some(bbcat::LetterSpacing::EightPixels) => "8",
        Some(bbcat::LetterSpacing::NinePixels) => "9",
        None => "null",
    };
    format!(
        concat!(
            "{{\"title\":{},\"author\":{},\"group\":{},\"date\":{},",
            "\"width\":{},\"height\":{},\"iceColors\":{},",
            "\"letterSpacing\":{},\"fontName\":{}}}"
        ),
        json_string(&sauce.title),
        json_string(&sauce.author),
        json_string(&sauce.group),
        json_string(&sauce.date),
        sauce.width,
        sauce.height,
        sauce.ice_colors,
        letter_spacing,
        json_string(&sauce.font_name),
    )
}

fn format_id(format: bbcat::Format) -> &'static str {
    match format {
        bbcat::Format::AnsiText => "ansi-text",
        bbcat::Format::DarkDraw => "darkdraw",
        bbcat::Format::ArtWorx => "artworx",
        bbcat::Format::Ripscrip => "ripscrip",
        bbcat::Format::TundraDraw => "tundradraw",
        bbcat::Format::XBin => "xbin",
        _ => "unknown",
    }
}

fn json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0c}' => encoded.push_str("\\f"),
            character if character <= '\u{1f}' => {
                encoded.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

fn write_png<W: io::Write>(output: &mut W, path: &Path, png: &[u8]) -> Result<(), String> {
    if path == Path::new("-") {
        output
            .write_all(png)
            .and_then(|()| output.flush())
            .map_err(|error| format!("stdout: {error}"))
    } else {
        fs::write(path, png).map_err(|error| format!("{}: {error}", path.display()))
    }
}

fn write_sauce<W: io::Write>(output: &mut W, sauce: Option<&bbcat::Sauce>) -> Result<(), String> {
    let Some(sauce) = sauce else {
        return Ok(());
    };

    let date = sauce_date(&sauce.date);
    let mut details = Vec::new();
    if !sauce.author.is_empty() {
        details.push(format!("by {}", sauce.author));
    }
    if !sauce.group.is_empty() {
        details.push(sauce.group.clone());
    }
    if !date.is_empty() {
        details.push(date);
    }
    if sauce.title.is_empty() && details.is_empty() {
        return Ok(());
    }

    writeln!(output).map_err(|error| format!("stdout: {error}"))?;
    if !sauce.title.is_empty() {
        writeln!(output, "\x1b[1m{}\x1b[0m", sauce.title)
            .map_err(|error| format!("stdout: {error}"))?;
    }
    if !details.is_empty() {
        writeln!(output, "{}", details.join(" · ")).map_err(|error| format!("stdout: {error}"))?;
    }
    writeln!(output).map_err(|error| format!("stdout: {error}"))
}

fn sauce_date(date: &str) -> String {
    if date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..])
    } else {
        date.to_owned()
    }
}

fn parse_args() -> Result<Option<Options>, String> {
    let mut width = None;
    let mut chunk_lines = env::var("LINES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map_or(24, |lines| lines.saturating_sub(1).clamp(1, 64));
    let mut output = None;
    let mut apng = None;
    let mut gif = None;
    let mut asciimation = false;
    let mut kitty = false;
    let mut fit = false;
    let mut delay = None;
    let mut baud = None;
    let mut scale = 1;
    let mut sauce = false;
    let mut info = false;
    let mut list_archive = false;
    let mut files = Vec::new();
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("bbcat {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-w" | "--width" => width = Some(number(&argument, arguments.next())?),
            "--chunk-lines" => chunk_lines = number(&argument, arguments.next())?,
            "-o" | "--output" => {
                output = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| format!("{argument} requires a path"))?,
                ));
            }
            "--apng" => {
                apng = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| format!("{argument} requires a path"))?,
                ));
            }
            "--gif" => {
                gif = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| format!("{argument} requires a path"))?,
                ));
            }
            "--asciimation" => asciimation = true,
            "--kitty" => kitty = true,
            "--fit" => fit = true,
            "--slow" => {
                delay.get_or_insert(Duration::from_millis(DEFAULT_SLOW_DELAY_MS));
            }
            "--delay" => {
                delay = Some(Duration::from_millis(milliseconds(
                    &argument,
                    arguments.next(),
                )?));
            }
            "--baud" => baud = Some(baud_rate(&argument, arguments.next())?),
            "--2x" => scale = 2,
            "--sauce" => sauce = true,
            "--info" => info = true,
            "--list-archive" => list_archive = true,
            "--" => {
                files.extend(arguments);
                break;
            }
            "-" => files.push(argument),
            _ if argument.starts_with('-') => return Err(format!("unknown option: {argument}")),
            _ => files.push(argument),
        }
    }
    if chunk_lines == 0 || chunk_lines > 256 {
        return Err("--chunk-lines must be between 1 and 256".to_owned());
    }
    Ok(Some(Options {
        width,
        chunk_lines,
        output,
        apng,
        gif,
        asciimation,
        kitty,
        fit,
        delay,
        baud,
        scale,
        sauce,
        info,
        list_archive,
        files,
    }))
}

fn number(option: &str, value: Option<String>) -> Result<usize, String> {
    value
        .ok_or_else(|| format!("{option} requires a number"))?
        .parse()
        .map_err(|_| format!("{option} requires a positive integer"))
}

fn milliseconds(option: &str, value: Option<String>) -> Result<u64, String> {
    let value = value
        .ok_or_else(|| format!("{option} requires milliseconds between 1 and {MAX_DELAY_MS}"))?
        .parse::<u64>()
        .map_err(|_| format!("{option} requires milliseconds between 1 and {MAX_DELAY_MS}"))?;
    if !(1..=MAX_DELAY_MS).contains(&value) {
        return Err(format!(
            "{option} requires milliseconds between 1 and {MAX_DELAY_MS}"
        ));
    }
    Ok(value)
}

fn baud_rate(option: &str, value: Option<String>) -> Result<u64, String> {
    let value = value.ok_or_else(|| baud_suggestions(option))?;
    if let Some(multiplier) = value.strip_suffix('x').or_else(|| value.strip_suffix('X')) {
        return multiplier
            .parse::<u64>()
            .ok()
            .filter(|&multiplier| multiplier > 0)
            .and_then(|multiplier| 115_200_u64.checked_mul(multiplier))
            .ok_or_else(|| baud_suggestions(option));
    }

    value
        .parse::<u64>()
        .ok()
        .filter(|&rate| rate > 0)
        .ok_or_else(|| baud_suggestions(option))
}

fn baud_row_delay(baud: u64) -> Duration {
    let nanoseconds = u128::from(DEFAULT_BAUD_ROW_DELAY_MS)
        * 1_000_000
        * u128::from(bbcat::DEFAULT_ANIMATION_BAUD)
        / u128::from(baud);
    Duration::from_nanos(u64::try_from(nanoseconds).unwrap_or(u64::MAX))
}

fn baud_suggestions(option: &str) -> String {
    let suggestions = SUGGESTED_ANIMATION_RATES
        .iter()
        .map(u64::to_string)
        .chain(["1X".to_owned(), "2X".to_owned(), "4X".to_owned()])
        .collect::<Vec<_>>()
        .join(", ");
    format!("{option} requires a positive rate or Nx multiplier; try: {suggestions}")
}

fn print_help() {
    println!(
        r#"bbcat {}
Render character art, play terminal animation, or write Kitty, PNG, APNG, and GIF images.

Usage: bbcat [OPTIONS] [FILE]...

Arguments:
  [FILE]...                 Art files, .ZIP packs, or ARCHIVE.zip:ENTRY; use - for stdin

Options:
  -w, --width COLS          Override text width; must match fixed binary/vector widths
      --chunk-lines ROWS    Kitty image height (default: LINES - 1, or 24)
      --kitty               Use Kitty graphics instead of UTF-8 text
      --fit                 Scale complete Kitty art to terminal width instead of cropping
      --slow                Reveal character art one row at a time (35 ms/row)
      --delay MS            Set the slow-mode row delay (1..=10000)
      --baud RATE           Animation speed or static row-reveal speed: positive RATE or Nx (try --baud for suggestions; 1X is 25 ms/row)
      --2x                  Double Kitty or image output dimensions
      --sauce               Show a SAUCE caption below the artwork
      --info                Print detected artwork information as JSON
      --list-archive        List supported artwork entry names in one ZIP archive
  -o, --output FILE         Write a PNG file; use - for stdout
      --apng FILE           Write an animated PNG; use - for stdout
      --gif FILE            Write an animated GIF; use - for stdout
      --asciimation         Interpret a 13-row asciimation.co.nz frame stream
  -h, --help                Print help
  -V, --version             Print version"#,
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_explicit_zip_entry_specs() {
        assert_eq!(
            split_archive_spec("pack.zip:art/demo.ans"),
            ("pack.zip", Some("art/demo.ans"))
        );
        assert_eq!(
            split_archive_spec("PACK.ZIP:FILE_ID.ANS"),
            ("PACK.ZIP", Some("FILE_ID.ANS"))
        );
        assert_eq!(split_archive_spec("art.ans"), ("art.ans", None));
        assert_eq!(split_archive_spec("odd:name.ans"), ("odd:name.ans", None));
    }

    #[test]
    fn validates_slow_mode_delays() {
        assert_eq!(milliseconds("--delay", Some("25".to_owned())), Ok(25));
        for value in [
            None,
            Some("0".to_owned()),
            Some("10001".to_owned()),
            Some("fast".to_owned()),
        ] {
            assert!(milliseconds("--delay", value).is_err());
        }
    }

    #[test]
    fn validates_animation_baud_rates() {
        for rate in SUGGESTED_ANIMATION_RATES {
            assert_eq!(baud_rate("--baud", Some(rate.to_string())), Ok(rate));
        }
        assert_eq!(baud_rate("--baud", Some("2x".to_owned())), Ok(230_400));
        assert_eq!(baud_rate("--baud", Some("4X".to_owned())), Ok(460_800));
        assert_eq!(baud_rate("--baud", Some("3X".to_owned())), Ok(345_600));
        assert_eq!(
            baud_rate("--baud", Some("10000000".to_owned())),
            Ok(10_000_000)
        );
        for value in [
            None,
            Some("0".to_owned()),
            Some("0x".to_owned()),
            Some("fast".to_owned()),
        ] {
            assert!(baud_rate("--baud", value).is_err());
        }
    }

    #[test]
    fn missing_baud_shows_the_suggested_rates() {
        let error = baud_rate("--baud", None).unwrap_err();
        assert!(error.contains("try: 2400, 9600, 14400"));
        assert!(error.contains("1X, 2X, 4X"));
    }

    #[test]
    fn baud_scales_the_static_row_delay_from_1x() {
        assert_eq!(
            baud_row_delay(bbcat::DEFAULT_ANIMATION_BAUD),
            Duration::from_millis(DEFAULT_BAUD_ROW_DELAY_MS)
        );
        assert_eq!(baud_row_delay(57_600), Duration::from_millis(50));
        assert_eq!(baud_row_delay(30_000), Duration::from_millis(96));
        assert_eq!(baud_row_delay(230_400), Duration::from_micros(12_500));
    }

    #[test]
    fn dash_output_writes_png_to_stdout() {
        let mut output = Vec::new();
        write_png(&mut output, Path::new("-"), b"PNG").unwrap();
        assert_eq!(output, b"PNG");
    }

    #[test]
    fn writes_document_information_as_json() {
        let document = bbcat::decode(b"A").unwrap();
        let mut output = Vec::new();

        write_document_info(&mut output, "quote\".ans", 1, &document).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("{\n"));
        assert!(output.contains("\"file\": \"quote\\\".ans\""));
        assert!(output.contains("\"sourceBytes\": 1"));
        assert!(output.contains("\"format\": \"ansi-text\""));
        assert!(output.contains("\"animated\":false"));
        assert!(output.contains("\"sauce\": null"));
    }

    #[test]
    fn writes_explicit_asciimation_information_as_json() {
        let mut input = String::from("2\n");
        for _ in 0..13 {
            input.push_str("x\n");
        }
        let animation = bbcat::decode_asciimation(input.as_bytes()).unwrap();
        let mut output = Vec::new();

        write_asciimation_info(&mut output, "demo.txt", input.len(), &animation).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\"format\": \"asciimation\""));
        assert!(output.contains("\"frameCount\":1"));
        assert!(output.contains("\"durationMs\":200"));
    }

    #[test]
    fn writes_gallery_style_sauce_caption() {
        let mut data = [0_u8; 128];
        data[..7].copy_from_slice(b"SAUCE00");
        data[7..11].copy_from_slice(b"Demo");
        data[42..48].copy_from_slice(b"Artist");
        data[62..67].copy_from_slice(b"Group");
        data[82..90].copy_from_slice(b"19940630");
        let sauce = bbcat::Sauce::parse(&data).unwrap();
        let mut output = Vec::new();

        write_sauce(&mut output, Some(&sauce)).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\n\x1b[1mDemo\x1b[0m\nby Artist · Group · 1994-06-30\n\n"
        );
    }

    #[test]
    fn omits_caption_without_sauce() {
        let mut output = Vec::new();
        write_sauce(&mut output, None).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn preserves_nonstandard_sauce_dates() {
        assert_eq!(sauce_date("19940630"), "1994-06-30");
        assert_eq!(sauce_date("SUMMER94"), "SUMMER94");
    }
}
