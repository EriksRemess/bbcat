use std::{error::Error as StdError, path::Path, time::Duration};

use bbcat::{DecodeOptions, Format};

#[test]
fn decodes_and_encodes_through_the_public_api() -> Result<(), Box<dyn StdError>> {
    let document = bbcat::decode_with_options(
        b"\x1b[31mHi",
        DecodeOptions {
            file_name: Some(Path::new("welcome.ans")),
            width: Some(4),
        },
    )?;

    assert_eq!(document.format, Format::AnsiText);
    assert_eq!((document.screen.width, document.screen.height), (4, 1));
    assert_eq!(
        document.screen.cell(0, 0).unwrap().character,
        u16::from(b'H')
    );
    assert_eq!(document.screen.glyph_dimensions(), (8, 16));
    assert_eq!(document.screen.pixel_dimensions(), Some((32, 16)));
    assert_eq!(document.screen.palette(), bbcat::VGA_PALETTE);
    assert_eq!(document.screen.font().unwrap().len(), 256 * 16);
    assert!(document.screen.raster().is_none());

    let png = document.encode_png(1)?;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    Ok(())
}

#[test]
fn reports_the_detected_format() {
    let error = bbcat::decode(b"XBIN\x1a").unwrap_err();
    assert!(error.message().contains("truncated XBin header"));

    let data = concat!(
        r#"{"type":"Dimensions","text":"1x1","frame":"SAUCE_record"}"#,
        "\n",
        r#"{"id":"1","type":"frame","duration_ms":25}"#,
        "\n",
        r#"{"x":0,"y":0,"text":"X","color":"15","frame":"1"}"#,
    );
    let document = bbcat::decode(data.as_bytes()).unwrap();
    assert_eq!(document.format, Format::DarkDraw);
}

#[test]
fn exposes_character_and_raster_formats() -> Result<(), Box<dyn StdError>> {
    let xbin = [
        b'X', b'B', b'I', b'N', 0x1a, 1, 0, 1, 0, 0, 0x08, b'A', 0x1e,
    ];
    let document = bbcat::decode(&xbin)?;
    assert_eq!(document.format, Format::XBin);
    assert_eq!(document.screen.cell(0, 0).unwrap().foreground, 14);

    let document = bbcat::decode(b"!|*|c0F|L00000A0A|#")?;
    assert_eq!(document.format, Format::Ripscrip);
    let raster = document.screen.raster().unwrap();
    assert_eq!((raster.width, raster.height), (640, 350));
    assert_eq!(raster.pixels[0], 15);
    Ok(())
}

#[test]
fn exposes_animation_encoders_and_explicit_asciimation() -> Result<(), Box<dyn StdError>> {
    let ansi = b"\x1b[2J\x1b[H\x1b[1;1HA\x1b[1;1HB\x1b[2J";
    let document = bbcat::decode_with_options(
        ansi,
        DecodeOptions {
            file_name: Some(Path::new("demo.ans")),
            width: Some(1),
        },
    )?;
    assert!(document.animation.is_some());
    assert!(
        document
            .encode_apng(bbcat::DEFAULT_ANIMATION_BAUD, 1)?
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    assert!(
        document
            .encode_gif(bbcat::DEFAULT_ANIMATION_BAUD, 1)?
            .starts_with(b"GIF89a")
    );

    let mut stream = String::from("2\n");
    for _ in 0..13 {
        stream.push_str("x\n");
    }
    let animation = bbcat::decode_asciimation(stream.as_bytes())?;
    assert_eq!(animation.frames[0].duration(), Duration::from_millis(200));
    assert_eq!(animation.frames[0].rows().len(), 13);
    Ok(())
}
