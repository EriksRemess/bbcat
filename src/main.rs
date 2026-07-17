use std::{
    env, fs,
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

mod terminal;

const DEFAULT_DELAY_MS: u64 = 25;
const MAX_DELAY_MS: u64 = 10_000;
const SUGGESTED_ANIMATION_RATES: [u64; 7] = [2_400, 9_600, 14_400, 28_800, 38_400, 57_600, 115_200];

struct Options {
    width: Option<usize>,
    chunk_lines: usize,
    output: Option<PathBuf>,
    kitty: bool,
    fit: bool,
    delay: Option<Duration>,
    baud: Option<u64>,
    scale: usize,
    sauce: bool,
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
    let Some(options) = parse_args()? else {
        return Ok(ExitCode::SUCCESS);
    };
    if options.output.is_some() && options.files.len() != 1 {
        return Err("--output requires exactly one input file".to_owned());
    }
    if options.output.is_some() && options.delay.is_some() {
        return Err("--slow/--delay cannot be used with --output".to_owned());
    }
    if options.output.is_some() && options.baud.is_some() {
        return Err("--baud cannot be used with --output".to_owned());
    }
    if options.output.is_some() && options.kitty {
        return Err("--output and --kitty cannot be used together".to_owned());
    }
    if options.kitty && options.baud.is_some() {
        return Err("--baud cannot be used with --kitty".to_owned());
    }
    if options.delay.is_some() && options.baud.is_some() {
        return Err("--slow/--delay and --baud cannot be used together".to_owned());
    }
    if options.output.is_some() && options.sauce {
        return Err("--sauce cannot be used with --output".to_owned());
    }
    if options.fit && !options.kitty {
        return Err("--fit requires --kitty".to_owned());
    }
    if options.scale > 1 && options.output.is_none() && !options.kitty {
        return Err("--2x requires --kitty or --output FILE".to_owned());
    }
    let stdout_is_terminal = io::stdout().is_terminal();
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
    let mut input_error = false;
    for file in &options.files {
        let data = match read(file) {
            Ok(data) => data,
            Err(error) => {
                eprintln!("bbcat: {error}");
                input_error = true;
                continue;
            }
        };
        // All input formats become the same Screen here. The remaining branches
        // differ only in how that screen is serialized for the requested target.
        let document = match bbcat::render_named(&data, options.width, file) {
            Ok(document) => document,
            Err(error) => {
                eprintln!("bbcat: {file}: {error}");
                input_error = true;
                continue;
            }
        };
        if options.delay.is_some() && document.animation.is_some() {
            eprintln!(
                "bbcat: {file}: use --baud to control ANSI animation speed; --slow/--delay reveal static art by row"
            );
            input_error = true;
            continue;
        }
        if let Some(path) = &options.output {
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

fn read(path: &str) -> Result<Vec<u8>, String> {
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
    let mut kitty = false;
    let mut fit = false;
    let mut delay = None;
    let mut baud = None;
    let mut scale = 1;
    let mut sauce = false;
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
            "--kitty" => kitty = true,
            "--fit" => fit = true,
            "--slow" => {
                delay.get_or_insert(Duration::from_millis(DEFAULT_DELAY_MS));
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
            "--" => {
                files.extend(arguments);
                break;
            }
            "-" => files.push(argument),
            _ if argument.starts_with('-') => return Err(format!("unknown option: {argument}")),
            _ => files.push(argument),
        }
    }
    if files.is_empty() {
        files.push("-".to_owned());
    }
    if chunk_lines == 0 || chunk_lines > 256 {
        return Err("--chunk-lines must be between 1 and 256".to_owned());
    }
    Ok(Some(Options {
        width,
        chunk_lines,
        output,
        kitty,
        fit,
        delay,
        baud,
        scale,
        sauce,
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
    let nanoseconds =
        u128::from(DEFAULT_DELAY_MS) * 1_000_000 * u128::from(bbcat::DEFAULT_ANIMATION_BAUD)
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
Render character art, play ANSI animation, or write Kitty graphics and PNG.

Usage: bbcat [OPTIONS] [FILE]...

Arguments:
  [FILE]...                 .ANS/.DIZ/.ADF/.RIP/.XB files; use - or omit for stdin

Options:
  -w, --width COLS          Override text width; must match fixed binary/vector widths
      --chunk-lines ROWS    Kitty image height (default: LINES - 1, or 24)
      --kitty               Use Kitty graphics instead of UTF-8 text
      --fit                 Scale complete Kitty art to terminal width instead of cropping
      --slow                Reveal character art one row at a time (25 ms/row)
      --delay MS            Set the slow-mode row delay (1..=10000)
      --baud RATE           Animation speed or static row-reveal speed: positive RATE or Nx (try --baud for suggestions; 1X is 25 ms/row)
      --2x                  Double Kitty or PNG output dimensions
      --sauce               Show a SAUCE caption below the artwork
  -o, --output FILE         Write a PNG file; use - for stdout
  -h, --help                Print help
  -V, --version             Print version"#,
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Duration::from_millis(DEFAULT_DELAY_MS)
        );
        assert_eq!(baud_row_delay(57_600), Duration::from_millis(50));
        assert_eq!(baud_row_delay(230_400), Duration::from_micros(12_500));
    }

    #[test]
    fn dash_output_writes_png_to_stdout() {
        let mut output = Vec::new();
        write_png(&mut output, Path::new("-"), b"PNG").unwrap();
        assert_eq!(output, b"PNG");
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
