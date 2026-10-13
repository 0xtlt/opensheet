//! The Print screen driven in GPUI's headless window. The printing
//! commands are stand-in scripts written to a temporary folder: each one
//! notes how it was called, and the tests read those notes. No job goes
//! to a printer.

use super::print::{PrintDialog, Used};
use super::printer::{self, Sides, Tool};
use super::Cmd;
use crate::app::ExcelApp;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A folder of stand-ins for `lp`, `lpstat`, `lpoptions` and `open`, and
/// the notes they take. With `printers`, `lpstat` knows two of them, the
/// second being the default one and the first printing on both sides.
struct StandIns {
    dir: PathBuf,
}

impl StandIns {
    fn new(printers: bool) -> StandIns {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "opensheet-print-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let note = r#"echo "$(basename "$0") $*" >> "$(dirname "$0")/log""#;
        let lpstat = if printers {
            // The sentence of `-d` as a French system words it.
            "case \"$1\" in\n  -e) printf 'HP_LaserJet_Pro\\nCanon_TS5000\\n';;\n  -d) echo 'destination système par défaut : Canon_TS5000';;\nesac"
        } else {
            "echo 'lpstat: Aucune destination ajoutée.' >&2\nexit 1"
        };
        let lpoptions = "case \"$*\" in\n  *'HP_LaserJet_Pro -l') echo 'Duplex/Two-Sided: None *DuplexNoTumble DuplexTumble';;\n  *' -l') echo 'PageSize/Media Size: *A4 Letter';;\n  *HP_LaserJet_Pro*) echo \"copies=1 printer-info='HP LaserJet Pro' printer-is-shared=false\";;\n  *) echo 'copies=1 printer-info=Canon';;\nesac";
        let scripts = [
            (Tool::Lp, "lp", "echo 'request id is HP_LaserJet_Pro-1 (1 file(s))'"),
            (Tool::Lpstat, "lpstat", lpstat),
            (Tool::Lpoptions, "lpoptions", lpoptions),
            (Tool::Open, "open", "true"),
        ];
        for (tool, name, body) in scripts {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{note}\n{body}\n")).unwrap();
            make_runnable(&path);
            printer::stand_in(tool, &path);
        }
        StandIns { dir }
    }

    /// How a stand-in was called, once for each call: its arguments.
    fn calls(&self, name: &str) -> Vec<String> {
        let log = std::fs::read_to_string(self.dir.join("log")).unwrap_or_default();
        log.lines().filter_map(|line| line.strip_prefix(name)?.strip_prefix(' ')).map(str::to_string).collect()
    }
}

impl Drop for StandIns {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn make_runnable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    app.update(cx, |app, cx| {
        for (row, text) in ["Item", "Tea", "Coffee"].into_iter().enumerate() {
            app.wb.set_cell_text(0, row as u32, 0, text).unwrap();
        }
        app.commit("setup", cx);
    });
    redraw(cx);
    (app, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

/// File ▸ Print: the Print screen.
fn print_screen(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<PrintDialog> {
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::Print, window, cx));
    redraw(cx);
    app.update(cx, |app, _| app.front_modal_view().expect("the Print screen").downcast::<PrintDialog>())
        .ok()
        .expect("the Print screen")
}

fn dialogs(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
    app.update(cx, |app, _| app.modals.len())
}

fn press_print(cx: &mut VisualTestContext) {
    let at = cx.debug_bounds("print-button").expect("the Print button").center();
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

#[gpui::test]
fn a_job_goes_to_the_printer_chosen(cx: &mut TestAppContext) {
    let tools = StandIns::new(true);
    let (app, cx) = open(cx);
    let screen = print_screen(&app, cx);
    // The printers of the system under their names for people; the
    // default one is chosen.
    assert_eq!(screen.update(cx, |screen, _| screen.printer_titles()), vec!["HP LaserJet Pro", "Canon"]);
    assert_eq!(screen.update(cx, |screen, _| screen.chosen()).as_deref(), Some("Canon_TS5000"));
    // That one prints on one side: nothing about two sides is offered.
    assert!(!screen.update(cx, |screen, _| screen.offers_two_sided()));
    assert!(tools.calls("lpoptions").contains(&"-p Canon_TS5000 -l".to_string()));

    // The other one prints on both.
    screen.update(cx, |screen, cx| screen.choose_printer(0, cx));
    redraw(cx);
    assert!(screen.update(cx, |screen, _| screen.offers_two_sided()));
    screen.update(cx, |screen, cx| {
        screen.set_sides(Sides::LongEdge, cx);
        screen.set_job(2, Some((1, 1)), cx);
    });
    redraw(cx);
    assert!(tools.calls("lp").is_empty(), "nothing is sent before Print");
    press_print(cx);

    // One call of lp, with the printer, the copies, the sides, the title
    // of the workbook and the PDF.
    let calls = tools.calls("lp");
    assert_eq!(calls.len(), 1, "{calls:?}");
    let call = &calls[0];
    let expected = "-d HP_LaserJet_Pro -n 2 -o collate=true -o sides=two-sided-long-edge -t ";
    assert!(call.starts_with(expected), "{call}");
    let file = call.rsplit(' ').next().unwrap();
    assert!(file.ends_with(".pdf"), "{call}");
    assert!(std::fs::read(file).unwrap().starts_with(b"%PDF-"));
    // The screen closes and the status bar names the printer.
    assert_eq!(dialogs(&app, cx), 0);
    let status = app.update(cx, |app, _| app.status_message.clone().unwrap_or_default());
    assert!(status.to_string().contains("HP LaserJet Pro"), "{status}");

    // Next time: the same printer, and the settings of that job as a
    // preset.
    let screen = print_screen(&app, cx);
    assert_eq!(screen.update(cx, |screen, _| screen.chosen()).as_deref(), Some("HP_LaserJet_Pro"));
    assert!(screen.update(cx, |screen, _| screen.has_last_used()));
    let default = screen.update(cx, |screen, cx| screen.settings_now(cx));
    assert_eq!(default, Used::default());
    redraw(cx);
    screen.update(cx, |screen, cx| screen.apply_preset(1, cx));
    let last = screen.update(cx, |screen, cx| screen.settings_now(cx));
    assert_eq!(
        last,
        Used {
            copies: 2,
            collate: true,
            sides: Sides::LongEdge
        }
    );
    screen.update(cx, |screen, cx| screen.apply_preset(0, cx));
    assert_eq!(screen.update(cx, |screen, cx| screen.settings_now(cx)), Used::default());
}

#[gpui::test]
fn without_a_printer_the_system_says_what_to_do(cx: &mut TestAppContext) {
    let tools = StandIns::new(false);
    let (app, cx) = open(cx);
    let screen = print_screen(&app, cx);
    assert!(screen.update(cx, |screen, _| screen.printer_titles()).is_empty());
    assert_eq!(screen.update(cx, |screen, _| screen.chosen()), None);
    press_print(cx);
    // A message over the Print screen, and no job.
    assert_eq!(dialogs(&app, cx), 2);
    assert!(tools.calls("lp").is_empty());
    assert!(tools.calls("lpoptions").is_empty(), "no printer to ask");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(dialogs(&app, cx), 1);
    // The PDF can still be looked at in Preview.
    screen.update(cx, |screen, cx| screen.preview_it(cx));
    let calls = tools.calls("open");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("-a Preview ") && calls[0].ends_with(".pdf"), "{calls:?}");
}

#[gpui::test]
fn a_test_without_stand_ins_runs_no_command(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let screen = print_screen(&app, cx);
    assert!(screen.update(cx, |screen, _| screen.printer_titles()).is_empty());
    press_print(cx);
    assert_eq!(dialogs(&app, cx), 2, "the message about setting a printer up");
}
