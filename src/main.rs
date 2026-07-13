use std::{
    env, fs,
    io::{self, IsTerminal, Read},
    path::PathBuf,
    process::ExitCode,
};

mod terminal;

struct Options {
    width: Option<usize>,
    chunk_lines: usize,
    output: Option<PathBuf>,
    kitty: bool,
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
    if options.output.is_some() && options.kitty {
        return Err("--output and --kitty cannot be used together".to_owned());
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
        let document =
            bbcat::render(&data, options.width).map_err(|error| format!("{file}: {error}"))?;
        if let Some(path) = &options.output {
            let png = bbcat::encode_screen(&document.screen, 0, document.screen.height)?;
            fs::write(path, png).map_err(|error| format!("{}: {error}", path.display()))?;
        } else if options.kitty {
            bbcat::write_screen(&mut stdout, &document.screen, options.chunk_lines)
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

fn parse_args() -> Result<Option<Options>, String> {
    let mut width = None;
    let mut chunk_lines = env::var("LINES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map_or(24, |lines| lines.saturating_sub(1).clamp(1, 64));
    let mut output = None;
    let mut kitty = false;
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
        files,
    }))
}

fn number(option: &str, value: Option<String>) -> Result<usize, String> {
    value
        .ok_or_else(|| format!("{option} requires a number"))?
        .parse()
        .map_err(|_| format!("{option} requires a positive integer"))
}

fn print_help() {
    println!(
        r#"bbcat {}
Render CP437 ANSI, DIZ, and XBin art as UTF-8 text, Kitty graphics, or PNG.

Usage: bbcat [OPTIONS] [FILE]...

Arguments:
  [FILE]...                 .ANS/.DIZ/.XB files; use - or omit for stdin

Options:
  -w, --width COLS          Override ANSI/DIZ width; must match an XBin header
      --chunk-lines ROWS    Kitty image height (default: LINES - 1, or 24)
      --kitty               Use Kitty graphics instead of UTF-8 text
  -o, --output FILE         Write a PNG file
  -h, --help                Print help
  -V, --version             Print version"#,
        env!("CARGO_PKG_VERSION")
    );
}
