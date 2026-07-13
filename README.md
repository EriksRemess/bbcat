# bbcat

Dependency-free terminal viewer for CP437 ANSI, DIZ, ADF, RIPscrip, and XBin
art. It writes colored UTF-8 by default, with optional Kitty graphics and PNG
output.

```console
cargo install bbcat
bbcat art.ans
bbcat FILE_ID.DIZ
bbcat scene.xb
bbcat --kitty art.ans
bbcat --kitty art.adf
bbcat --kitty drawing.rip
```

Use `--kitty` in compatible terminals such as Kitty and Ghostty, or
`--output preview.png` to write a PNG. XBin palettes, fonts, and RLE compression
are supported; custom embedded ADF/XBin fonts require a graphical output mode.
RIPscrip is rendered at 640×350 and requires `--kitty` or `--output FILE`;
its bitmap and proportional BGI stroke fonts are supported. UTF-8 mode is
intended for character art. Unsupported RIPscrip commands are reported instead
of being silently ignored. Long terminal output is placed in scrollback.
