# bbcat

Dependency-free terminal viewer for CP437 ANSI, DIZ, and XBin art. It renders
through the Kitty graphics protocol and works in Kitty and Ghostty.

```console
cargo install bbcat
bbcat art.ans
bbcat FILE_ID.DIZ
bbcat scene.xb
```

Use `--output preview.png` to write a PNG instead. XBin palettes, fonts, and RLE
compression are supported; long terminal output is placed in scrollback.
