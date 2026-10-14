# Contributing to OpenSheet

Thanks for your interest. Issues and pull requests are welcome: bug reports with a file that shows the problem, missing Excel behaviour, translations, and code.

## The rule of the project

OpenSheet follows **Excel for Mac**: its menus, dialogs, wording, keyboard shortcuts and file format. When in doubt, do what Excel does, and write to `.xlsx` what Excel itself writes (attributes it always writes, element order, extension lists), not the minimum the schema allows. A file saved by OpenSheet must open in Excel without a repair prompt and look the same.

Never copy Microsoft assets into the repository (fonts, icons, images, documentation). Interface strings follow Excel's terms in each language.

## Getting started

```bash
cargo run -p opensheet --release -- --demo
```

See the [README](README.md#build-from-source) for the platform requirements.

## Before opening a pull request

1. The build has no warnings: `cargo build -p opensheet`.
2. All tests pass:
   ```bash
   cargo test -p opensheet_engine -p opensheet_io -p opensheet_print -p opensheet_script -p opensheet
   ```
3. Every new interface string is translated: write it as `t("English", "Français")` in the code, add the Spanish, German and Italian entries to `crates/app/src/i18n_tables.rs`, and check with `python3 scripts/check_i18n.py`.
4. New behaviour comes with tests: engine tests for the model, round-trip tests in `crates/io/tests/` for what is saved, and headless interface tests (see `crates/app/src/features/*/ui_tests.rs`) for dialogs and commands, including undo.
5. Every change to a workbook is one undo step and respects sheet and workbook protection.

## Where things are

- `crates/engine`: the workbook model and calculation, without any UI dependency. A new function that reads anything other than its reference arguments must be declared in `deps.rs`.
- `crates/io`: `.xlsx` / `.xlsm` / `.csv` reading and writing.
- `crates/print`: pagination and PDF.
- `crates/script`: the Office Scripts engine.
- `crates/app`: the GPUI application; each feature has its own module in `src/features/`.
- `docs/fonctionnalites.md`: the feature guide (French), also shown by the in-app Help pane. Update it when a feature is added or a limit is lifted.

## Commits

Small, focused commits with a message that says what changes for the user. Keep the code style of the file you edit.

## License

By contributing, you agree that your contributions are licensed under MIT OR Apache-2.0, as the rest of the project.
