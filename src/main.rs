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

struct Options {
    width: Option<usize>,
    chunk_lines: usize,
    output: Option<PathBuf>,
    kitty: bool,
    delay: Option<Duration>,
    scale: usize,
    files: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("bbcat: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(options) = parse_args()? else {
        return Ok(());
    };
    if options.output.is_some() && options.files.len() != 1 {
        return Err("--output requires exactly one input file".to_owned());
    }
    if options.output.is_some() && options.delay.is_some() {
        return Err("--slow/--delay cannot be used with --output".to_owned());
    }
    if options.output.is_some() && options.kitty {
        return Err("--output and --kitty cannot be used together".to_owned());
    }
    if options.scale > 1 && options.output.is_none() && !options.kitty {
        return Err("--2x requires --kitty or --output FILE".to_owned());
    }
    if options.kitty && !io::stdout().is_terminal() {
        return Err("--kitty requires terminal stdout".to_owned());
    }
    if options.kitty && !terminal::supports_kitty()? {
        return Err(
            "terminal does not support the Kitty graphics protocol; omit --kitty for UTF-8 output"
                .to_owned(),
        );
    }

    let mut stdout = io::stdout().lock();
    for file in &options.files {
        let data = read(file)?;
        let document = bbcat::render_named(&data, options.width, file)
            .map_err(|error| format!("{file}: {error}"))?;
        if let Some(path) = &options.output {
            let png = bbcat::encode_screen_scaled(
                &document.screen,
                0,
                document.screen.height,
                options.scale,
            )?;
            write_png(&mut stdout, path, &png)?;
        } else if options.kitty {
            if let Some(delay) = options.delay {
                bbcat::write_screen_slow_scaled(
                    &mut stdout,
                    &document.screen,
                    delay,
                    options.scale,
                )
                .map_err(|error| format!("{file}: {error}"))?;
            } else {
                bbcat::write_screen_scaled(
                    &mut stdout,
                    &document.screen,
                    options.chunk_lines,
                    options.scale,
                )
                .map_err(|error| format!("{file}: {error}"))?;
            }
        } else if let Some(delay) = options.delay {
            bbcat::write_text_slow(&mut stdout, &document.screen, delay)
                .map_err(|error| format!("{file}: {error}"))?;
        } else {
            bbcat::write_text(&mut stdout, &document.screen)
                .map_err(|error| format!("{file}: {error}"))?;
        }
    }
    Ok(())
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

fn parse_args() -> Result<Option<Options>, String> {
    let mut width = None;
    let mut chunk_lines = env::var("LINES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map_or(24, |lines| lines.saturating_sub(1).clamp(1, 64));
    let mut output = None;
    let mut kitty = false;
    let mut delay = None;
    let mut scale = 1;
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
            "--slow" => {
                delay.get_or_insert(Duration::from_millis(DEFAULT_DELAY_MS));
            }
            "--delay" => {
                delay = Some(Duration::from_millis(milliseconds(
                    &argument,
                    arguments.next(),
                )?));
            }
            "--2x" => scale = 2,
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
        delay,
        scale,
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

fn print_help() {
    println!(
        r#"bbcat {}
Render character art as UTF-8, or supported BBS art as Kitty graphics or PNG.

Usage: bbcat [OPTIONS] [FILE]...

Arguments:
  [FILE]...                 .ANS/.DIZ/.ADF/.RIP/.XB files; use - or omit for stdin

Options:
  -w, --width COLS          Override text width; must match fixed binary/vector widths
      --chunk-lines ROWS    Kitty image height (default: LINES - 1, or 24)
      --kitty               Use Kitty graphics instead of UTF-8 text
      --slow                Reveal character art one row at a time (25 ms/row)
      --delay MS            Set the slow-mode row delay (1..=10000)
      --2x                  Double Kitty or PNG output dimensions
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
    fn dash_output_writes_png_to_stdout() {
        let mut output = Vec::new();
        write_png(&mut output, Path::new("-"), b"PNG").unwrap();
        assert_eq!(output, b"PNG");
    }
}
