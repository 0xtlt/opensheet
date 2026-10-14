OpenSheet, an open-source spreadsheet with the interface and behaviour of Microsoft Excel, written in Rust with GPUI.

## Downloads

| System | File |
| --- | --- |
| macOS (Apple silicon) | `OpenSheet-…-macos-arm64.zip` |
| Windows (x86-64) | `OpenSheet-…-windows-x86_64.zip` |
| Linux (x86-64) | `OpenSheet-…-linux-x86_64.tar.gz` |

- **macOS**: the app is not signed yet. Right-click `OpenSheet.app` ▸ *Open* the first time, or run `xattr -dr com.apple.quarantine OpenSheet.app`.
- **Windows**: SmartScreen may warn about an unknown publisher.
- **Linux**: needs X11 or Wayland and Vulkan drivers.

OpenSheet is developed and tested on macOS. The Windows and Linux builds come from the same code but have not been tested by hand yet: reports are welcome.

See the [README](https://github.com/0xtlt/opensheet#readme) for what works, and the [feature guide](https://github.com/0xtlt/opensheet/blob/main/docs/fonctionnalites.md) (French) for every feature and limit.
