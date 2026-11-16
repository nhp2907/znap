# znap

Region screenshot + annotate tool in Rust. Windows first.

## Use

| Key / action | Effect |
|---|---|
| `Ctrl+Alt+S` | Capture the monitor under the cursor, open overlay |
| Drag | Select region (click without drag = whole screen); Rect tool is then active |
| Drag edges/corners | Resize selection |
| Drag inside (no tool) | Move selection |
| Toolbar | Rect, Ellipse, Arrow, Pen, Text, Mosaic, colors, Undo, Save, Copy, Cancel, Done |
| Mouse wheel | Stroke width (font size for Text) |
| `Ctrl+C` / `Enter` / ✓ | Copy region + annotations to clipboard, close |
| `Ctrl+S` | Save as PNG |
| `Ctrl+Z` | Undo |
| `Esc` / right-click | Finish text, clear selection, or close |

Tray icon → Quit. `znap.exe --capture` = open overlay once, exit after (no hotkey/tray).

## Build

Needs `rustup`. `rust-toolchain.toml` selects the GNU toolchain (no Visual Studio needed).
`softbuffer`/`arboard` are pinned: newer versions need `windows-sys` 0.61 (raw-dylib → full MinGW `dlltool`/`as`).

```bash
cargo run              # debug, with console logs
cargo build --release  # target/release/znap.exe, no console
```

## Install (per user)

Download `znap-<ver>-windows-x64.zip` from [Releases](https://github.com/nhp2907/znap/releases), unzip, double-click `install.cmd`. Installs + adds auto-start on login.

- Exe: `%LOCALAPPDATA%\Programs\znap\znap.exe`
- Auto-start: `shell:startup\znap.lnk`

## Layout

- `src/main.rs` – event loop, global hotkey, tray
- `src/capture.rs` – screen grab (Windows GDI; other OS TODO)
- `src/overlay.rs` – overlay window, selection, input, render
- `src/shapes.rs` – annotation shapes
- `src/toolbar.rs` – toolbar layout + icons
- `src/text.rs` – text rendering (system font)
- `src/output.rs` – clipboard, PNG save

## TODO

- macOS / Linux capture (`capture.rs`)
- Span all monitors in one overlay
- Configurable hotkey, auto-start
