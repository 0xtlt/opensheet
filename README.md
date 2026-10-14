# OpenSheet

An open-source spreadsheet with the interface and behaviour of Microsoft Excel, written in Rust with [GPUI](https://gpui.rs/).

[![CI](https://github.com/0xtlt/opensheet/actions/workflows/ci.yml/badge.svg)](https://github.com/0xtlt/opensheet/actions/workflows/ci.yml)
[![Release](https://github.com/0xtlt/opensheet/actions/workflows/release.yml/badge.svg)](https://github.com/0xtlt/opensheet/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

![OpenSheet](docs/screenshot.png)

> **Status: v0.1, under active development.** Most of Excel's major features are in place and files round-trip with Excel without repair. Some features are partial or missing — see the [detailed feature guide (French)](docs/fonctionnalites.md#ce-qui-manque). OpenSheet is developed and tested on macOS; the Windows and Linux builds compile from the same code but have not been tested by hand yet.

## Highlights

- **Excel's interface**: title bar, ribbon with its contextual tabs and Excel's folding as the window narrows, dialogs, side panes, context menus, status bar, keyboard shortcuts.
- **Calculation**: 337 functions, dynamic arrays, structured references, dependency-graph recalculation, manual and automatic modes, iterative calculation, formula auditing.
- **Data**: tables, sorting and filters, conditional formatting, data validation, PivotTables with slicers, timelines and PivotCharts, What-If Analysis, Get Data from text, the web, workbooks, JSON and XML.
- **Charts and objects**: charts and their formatting, pictures with styles and artistic effects, shapes, WordArt with the 40 text transforms, SmartArt, icons, equations, ink, form controls, sparklines.
- **Review and sharing**: comments with @mentions, notes, spelling, thesaurus, translation, accessibility checker, protection of sheets, ranges and workbooks.
- **Automation**: Office Scripts in TypeScript (`ExcelScript` object model, Code Editor with completion, action recorder). VBA projects of `.xlsm` files are kept byte for byte and shown read-only; they are never run.
- **Files**: `.xlsx`, `.xlsm`, `.xltx`, `.xltm`, `.csv`, `.tsv`; files written by OpenSheet open in Excel without repair. Printing to PDF, to a printer or to Preview.
- **Five languages** for the interface **and** the formulas: English, French, Spanish, German, Italian (`=SUM(A1:A3,1.5)` / `=SOMME(A1:A3;1,5)` / `=SUMME(…)` …).

The complete inventory, feature by feature, with what is partial or missing, is in [docs/fonctionnalites.md](docs/fonctionnalites.md) (French). It is also what the in-app Help pane shows.

## Download

Each [release](https://github.com/0xtlt/opensheet/releases) has an installer per platform, portable archives, their checksums in `SHA256SUMS`, and what changed in that version ([CHANGELOG.md](CHANGELOG.md)).

| System | Installer | Portable |
| --- | --- | --- |
| macOS, Apple silicon | `opensheet-aarch64-apple-darwin.dmg` | |
| macOS, Intel | `opensheet-x86_64-apple-darwin.dmg` | |
| Windows, x86-64 | `opensheet-x86_64-pc-windows-msvc-setup.exe` | `opensheet-x86_64-pc-windows-msvc.zip` |
| Windows, ARM | `opensheet-aarch64-pc-windows-msvc-setup.exe` | `opensheet-aarch64-pc-windows-msvc.zip` |
| Linux, x86-64 | `opensheet-x86_64-unknown-linux-gnu.deb` / `.rpm` | `opensheet-x86_64-unknown-linux-gnu.tar.gz` |
| Linux, ARM | `opensheet-aarch64-unknown-linux-gnu.deb` / `.rpm` | `opensheet-aarch64-unknown-linux-gnu.tar.gz` |

- **macOS**: open the disk image and drag OpenSheet onto Applications. Until the app is signed by a Developer ID, right-click it ▸ *Open* the first time.
- **Windows**: run the setup program. It installs for your account without administrator rights (or for all users if you choose), adds OpenSheet to the Start menu, and offers to open spreadsheets with it. SmartScreen may warn about an unknown publisher until the installer is signed.
- **Linux**: `sudo apt install ./opensheet-…-linux-gnu.deb` (Debian, Ubuntu) or `sudo dnf install ./opensheet-…-linux-gnu.rpm` (Fedora, openSUSE). It needs X11 or Wayland and Vulkan drivers.

Every file comes with a build provenance attestation: `gh attestation verify <file> --repo 0xtlt/opensheet` checks that it was built by this repository's release workflow.

### Opening spreadsheets with OpenSheet

The installers declare the files OpenSheet opens: `.xlsx`, `.xlsm`, `.xltx`, `.xltm`, `.csv` and `.tsv`.

- **macOS**: select a file in the Finder ▸ *File* ▸ *Get Info* ▸ *Open with*: OpenSheet ▸ *Change All…*.
- **Windows**: the installer's "Open spreadsheets with OpenSheet" option; otherwise right-click a file ▸ *Open with* ▸ *Choose another app* ▸ OpenSheet ▸ *Always*.
- **Linux**: your file manager's *Open With* ▸ *Set as default*, or `xdg-mime default org.opensheet.OpenSheet.desktop text/csv`.

## Build from source

Requirements: the [Rust](https://rustup.rs/) toolchain pinned in `mise.toml` (with [mise](https://mise.jdx.dev/): `mise install`, then `mise run run`, `mise run test`…).

- **macOS**: Xcode or its Command Line Tools (GPUI compiles its Metal shaders with `xcrun metal`).
- **Linux** (Debian/Ubuntu): `sudo apt install pkg-config clang libfontconfig-dev libfreetype-dev libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libvulkan-dev libssl-dev libzstd-dev libasound2-dev`
- **Windows**: the MSVC toolchain (Visual Studio Build Tools).

```bash
cargo run -p opensheet --release                 # empty workbook
cargo run -p opensheet --release -- --demo       # sample data
cargo run -p opensheet --release -- book.xlsx    # open a file
cargo run -p opensheet --release -- book.xlsx --export book.csv   # convert without a window
```

On macOS, `cargo build -p opensheet --release && scripts/bundle.sh release` makes `target/release/OpenSheet.app`. The icon's sources are in `packaging/icon/` (`packaging/icon/build.sh` regenerates the files of every platform).

### Language and fonts

- The interface starts in the system's language; change it at the bottom of the **File** menu, or force it with `OPENSHEET_LANG=en` (`fr`, `es`, `de`, `it`).
- Excel's default font is Aptos Narrow, which is not a system font. Without it OpenSheet uses a close system font. Install Aptos (a free download from Microsoft) for the exact look, or, if Microsoft Office is installed, launch with `OPENSHEET_OFFICE_FONTS=1` to load its fonts at run time (nothing is copied or redistributed).

## Architecture

```
crates/
  engine/   calculation engine, no UI dependency: values, localized formula parser,
            evaluator, 337 functions, number formats, workbook model, undo/redo,
            one module per feature (filters, tables, pivots, charts, drawings…)
  io/       reading and writing .xlsx / .xlsm / .csv, VBA projects kept as they are
  print/    pagination and PDF output
  script/   Office Scripts: TypeScript compiled by oxc, run by QuickJS in a sandbox,
            the ExcelScript object model bound to the engine's operations
  app/      the GPUI application: ribbon, directly painted grid, cell editor,
            one module per feature in src/features/
```

Formulas are stored in canonical (English) form and translated for display and input, so a workbook typed in French opens unchanged in German.

## Tests

```bash
cargo test -p opensheet_engine -p opensheet_io -p opensheet_print -p opensheet_script -p opensheet
python3 scripts/check_i18n.py    # every interface string is translated in es, de and it
```

About 2,000 tests, including headless interface tests that drive the real window (keys, clicks, dialogs) and round-trip tests of every file format feature.

## Releases

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`: it checks that the tag is the crate's version and that `CHANGELOG.md` has a section for it, runs the tests, builds every platform and its installers (installing them where the runner can), and publishes the release with that changelog section. With the Apple secrets listed at the top of the workflow, the macOS app is signed and notarized. Running the workflow by hand does the same without publishing. `mise run release:notes 0.1.0` prints what a release will say.

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

OpenSheet is an independent project, not affiliated with, endorsed or sponsored by Microsoft. Microsoft and Excel are trademarks of Microsoft Corporation.
