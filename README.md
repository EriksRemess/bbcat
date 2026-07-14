# bbcat

Dependency-free terminal viewer for CP437 ANSI, DIZ, ADF, RIPscrip, and XBin
art. It writes colored UTF-8 by default, with optional Kitty graphics and PNG
output.

Browse and download BBS art packs at [16colo.rs](https://16colo.rs/).

## Install

```console
cargo install bbcat
```

To build from source, run `cargo build --release`.

## Usage

```console
bbcat [OPTIONS] [FILE]...
```

Pass one or more files, use `-` for standard input, or omit the filename to
read standard input. Use `--` before a filename that begins with `-`.

```console
bbcat art.ans FILE_ID.DIZ
bbcat < art.ans
bbcat --kitty scene.xb
bbcat --output preview.png art.adf
bbcat --output - art.ans > preview.png
bbcat --2x --kitty art.ans
```

## Output modes

| Mode | Option | Behavior |
| --- | --- | --- |
| UTF-8 | default | Converts CP437 characters to Unicode and emits 24-bit ANSI colors. |
| Kitty | `--kitty` | Renders bitmap glyphs through Kitty graphics, cropping at the terminal's right edge. |
| PNG | `-o FILE`, `--output FILE` | Writes one indexed-color PNG; use `-` to write it to standard output. |

Kitty mode requires terminal stdout and a terminal that answers bbcat's Kitty
graphics protocol probe. [Kitty](https://sw.kovidgoyal.net/kitty/) and
[Ghostty](https://ghostty.org/) are supported on Linux. Long images are emitted
in strips so they remain available in terminal scrollback. Add
`--fit` to scale the complete image to the terminal width instead of cropping.
If that would make the image shorter than one terminal row, bbcat reports the
minimum required terminal width.

PNG mode requires exactly one input file. Use `--output -` to pipe or redirect
the PNG from standard output. PNG mode cannot be combined with `--kitty`,
`--slow`, or `--delay`.

UTF-8 output is intended for character art using the standard CP437 glyph set.
Use Kitty or PNG output for RIPscrip, XBin 512-character fonts, or artwork whose
embedded bitmap font must be reproduced exactly.

## Slow mode

```console
bbcat --slow art.ans
bbcat --delay 50 art.ans
bbcat --kitty --slow art.ans
```

`--slow` reveals one character row at a time with a 25 millisecond delay.
`--delay MS` enables slow mode with a custom delay from 1 through 10,000
milliseconds. Both UTF-8 and Kitty modes flush each row before waiting; Kitty
mode automatically uses one image strip per character row.

Slow mode is not supported with PNG output or RIPscrip raster graphics. In
Kitty slow mode, `--chunk-lines` has no effect.

## 2x scaling

```console
bbcat --2x --kitty scene.xb
bbcat --kitty --fit wide.ans
bbcat --2x --output large.png drawing.rip
bbcat --2x --kitty --slow art.ans
```

`--2x` doubles both graphical output dimensions. Kitty mode crops the doubled
bitmap at the terminal width unless `--fit` is present; PNG mode writes an image
with twice the width and height. It works with every supported input format and
can be combined with Kitty slow mode. Slow-mode delays remain per original
artwork row.

Scaling is intentionally unavailable in UTF-8 mode: repeating text characters
would change strings and distort line art. Use `--kitty` or `--output FILE` with
`--2x`.

When UTF-8 output goes directly to a terminal, rows wider than the terminal are
cropped to its current column count. Redirected or piped UTF-8 output preserves
the full artwork width. Kitty crops at terminal width by default; `--fit` scales
the complete image down when at least one terminal row remains. PNG output
always retains its full dimensions.

## Formats

- ANSI and plain CP437 text, including `.ANS`, `.DIZ`, `.ASC`, `.NFO`, `.MEM`,
  and `.TXT`. ANSI cursor movement, erasing, standard and bright colors,
  inverse video, blink/iCE colors, wrapping, and SAUCE dimensions are handled.
- XBin (`.XB`) with embedded palettes, 8- or 16-color backgrounds, embedded
  fonts up to 32 pixels high, 256- and 512-character modes, and XBin RLE.
- ArtWorx Data Format (`.ADF`) version 1 with its embedded palette and 8x16
  font. ADF is fixed at 80 columns.
- RIPscrip (`.RIP`) level-one vector graphics, including its bitmap and
  proportional BGI stroke fonts. RIPscrip is rendered to a 640x350 canvas and
  requires Kitty or PNG output.

SAUCE metadata is used for content length, canvas dimensions, iCE color mode,
and named IBM VGA50, Amiga MicroKnight, Amiga Topaz 2+, and Empathy bitmap
fonts when present. Kitty and PNG output reproduce their exact glyph shapes;
UTF-8 output remains a terminal-font approximation. A DOS EOF marker terminates
plain ANSI/text input.

Common raster image inputs such as PNG, GIF, JPEG, WebP, TIFF, ICO, BMP, and
QOI are rejected by content with an explanatory error instead of being parsed
as character art. Malformed, truncated, oversized, or unsupported BBS inputs
also produce a non-zero exit status and a filename-scoped error.

## Options

| Option | Description |
| --- | --- |
| `-w COLS`, `--width COLS` | Override text width. ANSI/text accepts declared widths through 10,000 columns; untagged plain text is inferred through 1,000. XBin must match its header, ADF must be 80, and RIPscrip must be 640. |
| `--chunk-lines ROWS` | Set the number of character rows in each Kitty image strip, from 1 through 256. The default is `$LINES - 1`, clamped to 1 through 64, or 24 when `$LINES` is unavailable. |
| `--kitty` | Use Kitty graphics instead of colored UTF-8. |
| `--fit` | Scale complete Kitty output to terminal width instead of cropping. Errors if the result would be shorter than one terminal row. |
| `--slow` | Reveal one character row at a time using a 25 ms delay. |
| `--delay MS` | Enable slow mode with a delay from 1 through 10,000 ms per row. |
| `--2x` | Double Kitty or PNG output width and height. Requires `--kitty` or `--output FILE`. |
| `-o FILE`, `--output FILE` | Write an indexed-color PNG. Use `-` for standard output; requires exactly one input. |
| `-h`, `--help` | Print command help. |
| `-V`, `--version` | Print the bbcat version. |

bbcat has no runtime dependencies and does not require a particular Rust
version beyond what is needed to compile the current Rust edition.
