# bbcat

Dependency-free Rust library and terminal viewer for CP437 ANSI and ansimation,
ASCIImation text streams, DarkDraw DDW, DIZ, ADF, RIPscrip, and XBin art. It
writes colored UTF-8 by default, with optional Kitty graphics plus PNG, APNG,
and GIF output. The CLI can also preview artwork embedded in ZIP packs.

Browse and download BBS art packs at [16colo.rs](https://16colo.rs/).

## Install

```console
cargo install bbcat
```

To install from a source checkout, run:

```console
cargo install --path .
```

## Usage

```console
bbcat [OPTIONS] [FILE]...
```

Pass one or more files, use `-` for standard input, or omit the filename to
read standard input. Use `--` before a filename that begins with `-`.

```console
bbcat art.ans FILE_ID.DIZ
bbcat mist0526.zip
bbcat < art.ans
bbcat --kitty scene.xb
bbcat --output preview.png art.adf
bbcat --output - art.ans > preview.png
bbcat --apng animation.png animation.ans
bbcat --gif animation.gif animation.ans
bbcat --2x --kitty art.ans
bbcat --sauce art.ans
bbcat --baud 4x animation.ans
bbcat --asciimation ~/Downloads/starwars.txt
```

ZIP input is detected by content or a `.zip` extension. bbcat previews the
archive description when one is present, preferring `FILE_ID.ANS` and then
`FILE_ID.DIZ`; otherwise it uses the first supported ANSI/BBS art entry. Stored
and ordinary Deflate entries are read directly without an external unzip tool
or an added library dependency. Encrypted, multi-disk, and ZIP64 archives are
not supported.

## Library use

Add bbcat to another Rust application to detect and decode supported BBS art
formats into a common `Document` and `Screen` model:

```console
cargo add bbcat
```

```rust
use bbcat::{DecodeOptions, Format};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read("art/demo.ans")?;
    let document = bbcat::decode_with_options(
        &data,
        DecodeOptions {
            file_name: Some(Path::new("art/demo.ans")),
            width: None,
        },
    )?;

    println!(
        "{}: {}x{} cells",
        document.format, document.screen.width, document.screen.height
    );
    assert_eq!(document.format, Format::AnsiText);

    std::fs::write("preview.png", document.encode_png(1)?)?;
    Ok(())
}
```

Use `decode` for content-based detection with inferred dimensions, or
`decode_with_options` to supply a filename hint and width override. A decoded
`Document` reports its `Format`, final `Screen`, optional `Sauce`, and optional
`Animation`. Character screens expose cells, glyph and pixel dimensions,
palette and font data; RIPscrip screens expose their indexed raster. Documents
can be encoded directly to PNG, APNG, or GIF, while the lower-level text and
Kitty writers remain available for streaming output. ASCIImation has no format
signature; decode it explicitly with `decode_asciimation`.

See the complete [library API documentation](https://docs.rs/bbcat/) on docs.rs.

For a GUI application example, see
[`bbcat-gtk`](https://github.com/EriksRemess/bbcat-gtk), a small GTK4 demo that
uses the library to open and render static and animated ANSI/BBS artwork. It
also demonstrates file handling, responsive image display, and presenting
SAUCE metadata in a desktop application.

## Output modes

| Mode | Option | Behavior |
| --- | --- | --- |
| UTF-8 | default | Converts CP437 characters to Unicode and emits 24-bit ANSI colors. |
| Kitty | `--kitty` | Renders bitmap glyphs through Kitty graphics, cropping at the terminal's right edge. |
| PNG | `-o FILE`, `--output FILE` | Writes one indexed-color PNG; use `-` to write it to standard output. |
| APNG | `--apng FILE` | Writes a looping indexed-color animated PNG from an ANSI or DDW animation. |
| GIF | `--gif FILE` | Writes a looping indexed-color animated GIF from an ANSI or DDW animation. |

Kitty mode requires terminal stdout and a terminal that answers bbcat's Kitty
graphics protocol probe. [Kitty](https://sw.kovidgoyal.net/kitty/) and
[Ghostty](https://ghostty.org/) are supported on Linux and macOS, along with
[iTerm2](https://iterm2.com/) on macOS. Long images are emitted in strips so
they remain available in terminal scrollback. Add `--fit` to scale the complete
image to the terminal width instead of cropping. If that would make the image
shorter than one terminal row, bbcat reports the minimum required terminal
width.

PNG, APNG, and GIF modes require exactly one input file. Use `-` as their
output path to write the image to standard output. Choose only one image output
mode; none can be combined with `--kitty`, `--slow`, `--delay`, or `--sauce`.
Static PNG output cannot use `--baud`; APNG and GIF use it for frame timing.

UTF-8 output is intended for character art using the standard CP437 glyph set.
Use Kitty or PNG output for RIPscrip, XBin 512-character fonts, or artwork whose
embedded bitmap font must be reproduced exactly.

## Animation

bbcat recognizes ansimation from repeated ANSI screen rewrites and DarkDraw
DDW animation frames, then plays them automatically when standard output is a
terminal. Playback defaults to `1X`. Use `--baud RATE` to choose an animation
frame rate or set the row-reveal speed for static ANSI and text:

```console
bbcat --baud 2400 animation.ans
bbcat --baud 57600 animation.ans
bbcat --baud 2x animation.ans
bbcat --baud 4x animation.ans
```

`2400`, `9600`, `14400`, `28800`, `38400`, `57600`, `115200` (`1X`), `2X`
(230400), and `4X` (460800) are suggested familiar rates; run `bbcat --baud`
to print the list. Any positive numeric rate is accepted, as is an `Nx`
multiplier of 115200 (for example, `3X` is 345600). Animated playback uses each
frame's source byte count to determine how long it remains visible. DDW uses
its source-defined per-frame duration at `1X`, scaled proportionally by the
selected rate. Static art uses the same smooth row reveal as `--slow`: `1X` is
25 milliseconds per row, with lower rates slower and higher rates faster.
Terminals with
synchronized-output support reveal each redraw atomically to avoid visible
row-by-row tearing. Animation playback preserves CP437 text and source-defined
16-color, 256-color, and true-color SGR sequences. An animation that explicitly
homes to the terminal's top-left clears the canvas before playback; relative
animations leave the terminal intact. Both retain their final frame and return
the shell prompt below it.

Redirected UTF-8 output and Kitty or PNG output render the last visible state
instead of replaying the animation. `--apng` and `--gif` retain every detected
frame and loop forever. Their frame delays use the same source-byte timing as
terminal ansimation, or DDW's native durations, with `--baud` scaling either
one. Supplying `--baud` explicitly replays an animation or uses the static row
reveal even when UTF-8 output is redirected. `--baud` cannot be combined with
Kitty, static PNG, `--slow`, or `--delay`.

### ASCIImation text streams

The plain-text streams published by [asciimation.co.nz](https://asciimation.co.nz/)
do not contain terminal control sequences, so bbcat does not try to detect them
automatically. Use `--asciimation` to play one explicitly:

```console
bbcat --asciimation ~/Downloads/starwars.txt
```

The format stores a duration in 100 ms ticks followed by thirteen complete ASCII
rows per frame. Playback requires terminal stdout and exactly one input, and
redraws each frame atomically. It cannot be combined with Kitty/fit, image
output, width, scaling, speed, or SAUCE flags.

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

Slow mode is not supported with ANSI or DDW animation, image output, or
RIPscrip raster graphics. Use `--baud` for baud-paced ANSI/text output. In
Kitty slow mode, `--chunk-lines` has no effect.

## SAUCE metadata

Use `--sauce` to show an artwork's available SAUCE title, author, group, and
creation date as a compact gallery-style caption below the rendered art. Files
without descriptive SAUCE metadata render without a caption. The option works
with UTF-8 and Kitty output, including multiple input files, but cannot be
combined with image output.

## 2x scaling

```console
bbcat --2x --kitty scene.xb
bbcat --kitty --fit wide.ans
bbcat --2x --output large.png drawing.rip
bbcat --2x --apng large.png animation.ans
bbcat --2x --kitty --slow art.ans
```

`--2x` doubles both graphical output dimensions. Kitty mode crops the doubled
bitmap at the terminal width unless `--fit` is present; image output writes an
image with twice the width and height. It works with ANSI/text, DDW, XBin, ADF,
and RIPscrip, and can be combined with Kitty slow mode. ASCIImation playback
does not support scaling. Slow-mode delays remain per original artwork row.

Scaling is intentionally unavailable in UTF-8 mode: repeating text characters
would change strings and distort line art. Use `--kitty`, `--output FILE`,
`--apng FILE`, or `--gif FILE` with `--2x`.

When UTF-8 output goes directly to a terminal, rows wider than the terminal are
cropped to its current column count. Redirected or piped UTF-8 output preserves
the full artwork width. Kitty crops at terminal width by default; `--fit` scales
the complete image down when at least one terminal row remains. Image output
always retains its full dimensions.

## Formats

- ANSI and plain CP437 text, including `.ANS`, `.DIZ`, `.ASC`, `.NFO`, `.MEM`,
  and `.TXT`. ANSI cursor movement, erasing, standard and bright colors,
  inverse video, blink/iCE colors, xterm-256 colors, wrapping, SAUCE dimensions,
  and baud-paced ansimation are handled. Ansimation playback also preserves
  true-color SGR sequences.
- ASCIImation text streams containing a duration in 100 ms ticks followed by
  thirteen ASCII rows per frame. Because the format has no signature, use
  `--asciimation` in the CLI or `decode_asciimation` from Rust.
- DarkDraw (`.DDW`) UTF-8 JSON Lines text art and animation. Base and
  frame-specific objects are painted in source order; reusable group references
  are expanded recursively at their positioned frame. Each DDW frame uses its
  declared duration. A `Dimensions` metadata record is used when present;
  otherwise bbcat infers the canvas from the positioned text. Terminal playback
  preserves Unicode glyphs and 16- and 256-color styles; graphical output uses
  the same color indexes with a CP437 glyph approximation where necessary.
  APNG and GIF retain every frame.
- XBin (`.XB`) with embedded palettes, 8- or 16-color backgrounds, embedded
  fonts up to 32 pixels high, 256- and 512-character modes, and XBin RLE.
- ArtWorx Data Format (`.ADF`) version 1 with its embedded palette and 8x16
  font. ADF is fixed at 80 columns.
- RIPscrip (`.RIP`) level-one vector graphics, including its bitmap and
  proportional BGI stroke fonts. RIPscrip is rendered to a 640x350 canvas and
  requires Kitty or PNG output.
- ZIP art packs using stored or Deflate compression. The CLI selects the pack's
  `FILE_ID.ANS`, `FILE_ID.DIZ`, or first supported artwork as a preview.

SAUCE metadata is used for content length, canvas dimensions, iCE color mode,
8- or 9-pixel VGA letter spacing, and named IBM VGA50, Amiga MicroKnight,
Amiga Topaz 2+, and Empathy bitmap fonts when present. Kitty and image output
reproduce their exact glyph shapes; UTF-8 output remains a terminal-font
approximation. A 9-pixel Kitty raster reserves its full pixel width, so it may
occupy more terminal columns than the source character grid. A DOS EOF marker
terminates plain ANSI/text input.

Common raster image inputs such as PNG, GIF, JPEG, WebP, TIFF, ICO, BMP, and
QOI are rejected by content with an explanatory error instead of being parsed
as character art. Malformed, truncated, oversized, or unsupported BBS inputs
also produce a non-zero exit status and a filename-scoped error. With multiple
inputs, bbcat reports a rejected file and continues with the remaining files.

## Options

| Option | Description |
| --- | --- |
| `-w COLS`, `--width COLS` | Override text width. ANSI/text accepts declared widths through 10,000 columns; untagged plain text is inferred through 1,000. DDW and XBin must match their declared width; an untagged DDW may be widened but not narrowed. ADF must be 80, and RIPscrip must be 640. |
| `--chunk-lines ROWS` | Set the number of character rows in each Kitty image strip, from 1 through 256. The default is `$LINES - 1`, clamped to 1 through 64, or 24 when `$LINES` is unavailable. |
| `--kitty` | Use Kitty graphics instead of colored UTF-8. |
| `--fit` | Scale complete Kitty output to terminal width instead of cropping. Errors if the result would be shorter than one terminal row. |
| `--slow` | Reveal one character row at a time using a 25 ms delay. |
| `--delay MS` | Enable slow mode with a delay from 1 through 10,000 ms per row. |
| `--baud RATE` | Play ANSI animation by source-byte timing or DDW animation by native frame timing, set static ANSI/text row-reveal speed, or control APNG/GIF frame timing. `1X` is the same 25 ms/row as `--slow`; rates scale it proportionally. Run `--baud` alone for familiar suggested values. |
| `--2x` | Double Kitty or image output width and height. Requires `--kitty`, `--output FILE`, `--apng FILE`, or `--gif FILE`. |
| `--sauce` | Show the available SAUCE title, author, group, and creation date below the artwork. |
| `-o FILE`, `--output FILE` | Write an indexed-color PNG. Use `-` for standard output; requires exactly one input. |
| `--apng FILE` | Write a looping indexed-color animated PNG from ANSI or DDW animation frames. Use `-` for standard output; requires exactly one input. `--baud` controls timing. |
| `--gif FILE` | Write a looping indexed-color animated GIF from ANSI or DDW animation frames. Use `-` for standard output; requires exactly one input. `--baud` controls timing. |
| `--asciimation` | Play one explicit asciimation.co.nz-style text stream: a duration plus thirteen ASCII rows per frame. Requires exactly one input and terminal stdout; cannot be combined with Kitty/fit, image output, width, scaling, speed, or SAUCE options. |
| `-h`, `--help` | Print command help. |
| `-V`, `--version` | Print the bbcat version. |

bbcat has no runtime dependencies and does not require a particular Rust
version beyond what is needed to compile the current Rust edition.
