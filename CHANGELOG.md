# Changelog

What changed in each version of OpenSheet, for someone who uses it. The section of a version
is what its [GitHub release](https://github.com/0xtlt/opensheet/releases) says.

## 0.2.3 - 2026-10-22

- **Security update**: the dependencies are updated, which fixes the rustls and rustls-webpki
  advisories, and OpenSheet builds with Rust 1.99.1.

## 0.2.2 - 2026-10-15

- **The icon turns green**: a forest-green tile with a yellow selection.

## 0.2.1 - 2026-10-14

- **A merged cell keeps its fill**: the white that hides the gridlines inside a merged area was
  painted over its fill, so a white title on a coloured merged row could not be seen.
- A workbook whose A1 is merged opens with the whole merged area selected, as in Excel.
- **The icon turns indigo**, with a cyan selection.

## 0.2.0 - 2026-10-14

- **The language follows the system**: the interface and the formulas take the first of your
  preferred languages that OpenSheet speaks (else English); the language entries of the
  **File** menu are gone. On macOS, OpenSheet can have a language of its own in System
  Settings ▸ General ▸ Language & Region ▸ Applications. `OPENSHEET_LANG` still forces one.
- **Open from the Finder**: on macOS, double-clicking a spreadsheet (once OpenSheet is its
  default app), *Open With* or a drop on the Dock icon opens it, whether OpenSheet is running
  or not. `.xlsx`, `.xlsm`, `.xltx`, `.xltm`, `.csv` and `.tsv` files are declared to the
  system, which knows them even on a Mac without Excel.
- **One window per workbook**, as in Excel: a file opens in the untouched Book1 when there is
  one, otherwise in a new window brought to the front; a file already open brings its window
  forward. On Windows and Linux, several files given on the command line open in a window
  each. Quitting asks about every window with unsaved changes.
- **Installers**: a disk image on macOS, signed and notarized by Apple, so OpenSheet opens
  with a double-click; a setup program on Windows, for your account without administrator
  rights, which adds OpenSheet to the Start menu and offers to open spreadsheets with it;
  `.deb` and `.rpm` packages on Linux with a menu entry and the file types. The portable
  archives stay. Every file comes with a build provenance attestation
  (`gh attestation verify <file> --repo 0xtlt/opensheet`).
- **Homebrew**: `brew install 0xtlt/tap/opensheet` installs the app, and the `opensheet`
  command for the terminal.
- **An icon**: a sheet of cells with a selected cell, drawn for the Liquid Glass of macOS 26
  and for earlier macOS, Windows and Linux.
- On Windows, no console window opens behind OpenSheet any more.

## 0.1.0 - 2026-10-14

The first public version: an open-source spreadsheet with the interface and behaviour of
Microsoft Excel, written in Rust with GPUI, in English, French, Spanish, German and Italian.

### Install

- **macOS**: unzip `opensheet-aarch64-apple-darwin.zip` (Apple silicon) or
  `opensheet-x86_64-apple-darwin.zip` (Intel) and move `OpenSheet.app` to Applications. The
  app is not signed yet: right-click it ▸ *Open* the first time, or run
  `xattr -dr com.apple.quarantine OpenSheet.app`.
- **Windows**: unzip `opensheet-x86_64-pc-windows-msvc.zip` (or the `aarch64` one for ARM)
  and run `opensheet.exe`. SmartScreen may warn about an unknown publisher.
- **Linux**: unpack `opensheet-x86_64-unknown-linux-gnu.tar.gz` (or the `aarch64` one) and run
  `./opensheet`. It needs X11 or Wayland and Vulkan drivers.
- `SHA256SUMS` lists the checksum of every archive.

OpenSheet is developed and tested on macOS. The Windows and Linux builds come from the same
code and pass the same build, but have not been tried by hand yet: reports are welcome.

### What it does

- **Excel's interface**: the ribbon with its contextual tabs, folding as Excel's does when the
  window narrows, Excel's dialogs, side panes, context menus, status bar and shortcuts.
- **Calculation**: 337 functions, dynamic arrays, defined names, 3-D and structured
  references, recalculation of only what a change reaches, manual mode, iterative calculation
  and the formula auditing tools.
- **Data**: tables, sort and filter, conditional formatting, data validation, PivotTables
  with slicers, timelines and PivotCharts, What-If Analysis, Flash Fill, Text to Columns,
  Consolidate, outline and subtotals, Get Data from text files, the web, workbooks, JSON and
  XML.
- **Charts and objects**: charts and their formatting, sparklines, pictures with styles,
  effects and artistic effects, shapes, WordArt with Excel's 40 text transforms, SmartArt,
  icons, equations, ink, and form controls.
- **Review and sharing**: comments with @mentions, notes, spelling, thesaurus, translation,
  the accessibility checker, and the protection of sheets, ranges and workbooks.
- **Automation**: Office Scripts in TypeScript, with a Code Editor that completes and checks
  the `ExcelScript` object model and an action recorder. The VBA projects of `.xlsm` files are
  kept byte for byte and can be read, never run.
- **Files**: `.xlsx`, `.xlsm`, `.xltx`, `.xltm`, `.csv` and `.tsv`; a file saved by OpenSheet
  opens in Excel without repair. Printing to PDF, to a printer, or to Preview on macOS.
- **Five languages** for the interface and the formulas: `=SUM(A1:A3,1.5)` in English is
  `=SOMME(A1:A3;1,5)` in French, and a workbook typed in one language opens in another.

What is partial or missing is listed, feature by feature, in the
[feature guide](https://github.com/0xtlt/opensheet/blob/main/docs/fonctionnalites.md#ce-qui-manque)
(French).
