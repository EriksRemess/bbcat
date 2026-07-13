# bbcat

Dependency-free terminal viewer for CP437 ANSI, DIZ, and XBin art. It writes
colored UTF-8 text by default, with optional Kitty graphics and PNG output.

```console
cargo install bbcat
bbcat art.ans
bbcat FILE_ID.DIZ
bbcat scene.xb
bbcat --kitty art.ans
```

Use `--kitty` in compatible terminals such as Kitty and Ghostty, or
`--output preview.png` to write a PNG. XBin palettes, fonts, and RLE compression
are supported; long terminal output is placed in scrollback.
