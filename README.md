# bbcat

Dependency-free terminal viewer for CP437 ANSI, DIZ, ADF, and XBin art. It writes
colored UTF-8 text by default, with optional Kitty graphics and PNG output.

```console
cargo install bbcat
bbcat art.ans
bbcat FILE_ID.DIZ
bbcat scene.xb
bbcat --kitty art.ans
bbcat --kitty art.adf
```

Use `--kitty` in compatible terminals such as Kitty and Ghostty, or
`--output preview.png` to write a PNG. XBin palettes, fonts, and RLE compression
are supported; custom embedded ADF/XBin fonts require a graphical output mode.
Long terminal output is placed in scrollback.
