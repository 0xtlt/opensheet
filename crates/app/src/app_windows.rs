//! The windows of the application, one per workbook as in Excel, and the
//! files the system asks to open: the Finder sends a double-clicked file
//! (or one dropped on the Dock) as a `file://` URL, Windows and Linux as an
//! argument of the command line.
//!
//! A file opens in the window of the new workbook nobody has touched yet
//! (Book1 as it came), otherwise in a window of its own brought to the
//! front; a file already open only brings its window to the front.

use crate::app::ExcelApp;
use gpui::{
    App, AppContext, Bounds, Context, TitlebarOptions, Window, WindowBounds, WindowHandle,
    WindowOptions, point, px, size,
};
use std::path::{Path, PathBuf};

/// The identifier of the application: on Linux (Wayland) it ties the
/// windows to `org.opensheet.OpenSheet.desktop` and its icon.
const APP_ID: &str = "org.opensheet.OpenSheet";

/// The size of a new window: 1440 × 900, or `OPENSHEET_WINDOW=1000x700` to
/// look at the ribbon in a narrow window.
fn window_size() -> (f32, f32) {
    std::env::var("OPENSHEET_WINDOW")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
        })
        .filter(|(w, h)| *w >= 640.0 && *h >= 420.0)
        .unwrap_or((1440.0, 900.0))
}

/// Open a window on a new workbook; `setup` runs on it before it is first
/// drawn (a file to load, the demo sheet).
pub fn open_window(
    cx: &mut App,
    setup: impl FnOnce(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>) + 'static,
) -> Option<WindowHandle<ExcelApp>> {
    let (width, height) = window_size();
    let mut bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    // Each window a little lower and to the right of the ones before, so
    // that none hides another exactly.
    let shift = px(24.0 * (cx.windows().len() % 8) as f32);
    bounds.origin.x += shift;
    bounds.origin.y += shift;
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("OpenSheet".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(13.), px(12.))),
            }),
            window_min_size: Some(size(px(640.), px(420.))),
            app_id: Some(APP_ID.to_string()),
            ..Default::default()
        },
        move |window, cx| {
            // What the folding ribbon kept of this window goes with it.
            let id = window.window_handle().window_id();
            cx.new(|cx| {
                cx.on_release(move |_, _| crate::ribbon_fold::forget(id))
                    .detach();
                let mut app = ExcelApp::new(window, cx);
                setup(&mut app, window, cx);
                app
            })
        },
    )
    .ok()
}

/// The file a `file://` URL names, percent-decoded ("file:///Users/a/My%20Book.xlsx"
/// → "/Users/a/My Book.xlsx"); `None` for any other URL.
pub fn path_of_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // "file:///path" or "file://localhost/path"; another host is not a
    // file of this machine.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    // A query or a fragment is no part of the name (a '?' or a '#' of the
    // name itself comes encoded).
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    path_of_bytes(percent_decode(rest))
}

/// "%20" → ' ', and so on; a '%' not followed by two hexadecimal digits
/// stays as it is.
fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(unix)]
fn path_of_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(not(unix))]
fn path_of_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    let text = String::from_utf8(bytes).ok()?;
    // "/C:/Users/…" → "C:/Users/…"
    let text = match text.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => text[1..].to_string(),
        _ => text,
    };
    Some(PathBuf::from(text))
}

/// The files among URLs the system sent.
pub fn paths_of_urls(urls: &[String]) -> Vec<PathBuf> {
    urls.iter().filter_map(|url| path_of_url(url)).collect()
}

/// A window, as the choice of where a file opens sees it.
#[derive(Clone, Debug, Default)]
pub struct Slot {
    /// The file of its workbook (as `canonical` gives it).
    pub path: Option<PathBuf>,
    /// The new workbook as it came: no file, nothing changed, nothing
    /// being typed.
    pub untouched: bool,
    /// The window in front.
    pub active: bool,
}

/// Where a file asked to open goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// It is the workbook of that window already: bring it to the front.
    Activate(usize),
    /// That window holds an untouched new workbook: the file takes its
    /// place.
    Replace(usize),
    /// A window of its own.
    NewWindow,
}

/// Excel's choice for a file (`path` as `canonical` gives it) among the
/// open windows.
pub fn placement(path: &Path, windows: &[Slot]) -> Placement {
    if let Some(index) = windows.iter().position(|w| w.path.as_deref() == Some(path)) {
        return Placement::Activate(index);
    }
    // The untouched workbook in front first, else any other.
    windows
        .iter()
        .position(|w| w.untouched && w.active)
        .or_else(|| windows.iter().position(|w| w.untouched))
        .map_or(Placement::NewWindow, Placement::Replace)
}

/// The path that tells one file from another, whatever way it was written
/// ("./a.csv", a link): the file's real path when it exists.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Open files the system or the command line asked for, each as Excel
/// would (see [`placement`]).
pub fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    for path in paths {
        let path = canonical(&path);
        let handles: Vec<WindowHandle<ExcelApp>> = cx
            .windows()
            .into_iter()
            .filter_map(|w| w.downcast())
            .collect();
        let active = cx.active_window().map(|w| w.window_id());
        let slots: Vec<Slot> = handles
            .iter()
            .map(|handle| match handle.read(cx) {
                Ok(app) => Slot {
                    path: app.path.as_deref().map(canonical),
                    untouched: app.path.is_none() && !app.wb.modified && !app.is_editing(),
                    active: active == Some(handle.window_id()),
                },
                Err(_) => Slot::default(),
            })
            .collect();
        match placement(&path, &slots) {
            Placement::Activate(index) => {
                handles[index]
                    .update(cx, |_, window, _| window.activate_window())
                    .ok();
            }
            Placement::Replace(index) => {
                handles[index]
                    .update(cx, |app, window, cx| {
                        app.load_path(path, cx);
                        window.activate_window();
                    })
                    .ok();
            }
            Placement::NewWindow => {
                if let Some(handle) = open_window(cx, move |app, _, cx| app.load_path(path, cx)) {
                    handle
                        .update(cx, |_, window, _| window.activate_window())
                        .ok();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use opensheet_engine::Value;

    #[test]
    fn file_urls_become_paths() {
        assert_eq!(
            path_of_url("file:///Users/a/Book.xlsx"),
            Some(PathBuf::from("/Users/a/Book.xlsx"))
        );
        assert_eq!(
            path_of_url("file:///Users/a/My%20Book%20%C3%A9t%C3%A9.xlsx"),
            Some(PathBuf::from("/Users/a/My Book été.xlsx"))
        );
        assert_eq!(
            path_of_url("file://localhost/tmp/a%23b.csv"),
            Some(PathBuf::from("/tmp/a#b.csv"))
        );
        assert_eq!(
            path_of_url("file:///tmp/100%.csv"),
            Some(PathBuf::from("/tmp/100%.csv"))
        );
        assert_eq!(
            path_of_url("file:///tmp/a.csv?x=1"),
            Some(PathBuf::from("/tmp/a.csv"))
        );
        assert_eq!(path_of_url("file://server/share/a.csv"), None);
        assert_eq!(path_of_url("https://example.com/a.csv"), None);
        assert_eq!(path_of_url("opensheet://open"), None);
        assert_eq!(
            paths_of_urls(&[
                "file:///a.csv".into(),
                "mailto:x@example.com".into(),
                "file:///b.xlsx".into()
            ]),
            [PathBuf::from("/a.csv"), PathBuf::from("/b.xlsx")]
        );
    }

    fn slot(path: Option<&str>, untouched: bool, active: bool) -> Slot {
        Slot {
            path: path.map(PathBuf::from),
            untouched,
            active,
        }
    }

    #[test]
    fn a_file_takes_the_untouched_book_or_a_window_of_its_own() {
        let file = Path::new("/d/a.xlsx");
        // No window yet (the app launched by the file).
        assert_eq!(placement(file, &[]), Placement::NewWindow);
        // Book1 as it came.
        assert_eq!(
            placement(file, &[slot(None, true, true)]),
            Placement::Replace(0)
        );
        // Book1 with something typed, or another file.
        assert_eq!(
            placement(file, &[slot(None, false, true)]),
            Placement::NewWindow
        );
        assert_eq!(
            placement(file, &[slot(Some("/d/b.xlsx"), false, true)]),
            Placement::NewWindow
        );
        // Already open: its window comes to the front, wherever it is.
        let open = [
            slot(None, true, true),
            slot(Some("/d/a.xlsx"), false, false),
        ];
        assert_eq!(placement(file, &open), Placement::Activate(1));
        // Of two untouched books, the one in front.
        let two = [
            slot(None, true, false),
            slot(Some("/d/b.xlsx"), false, false),
            slot(None, true, true),
        ];
        assert_eq!(placement(file, &two), Placement::Replace(2));
        let behind = [
            slot(Some("/d/b.xlsx"), false, true),
            slot(None, true, false),
        ];
        assert_eq!(placement(file, &behind), Placement::Replace(1));
    }

    /// A folder of its own for the files of a test.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("opensheet-windows-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn titles(cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| {
            let mut titles: Vec<String> = cx
                .windows()
                .into_iter()
                .filter_map(|w| w.downcast::<ExcelApp>())
                .filter_map(|w| w.read(cx).ok().map(|app| app.title.clone()))
                .collect();
            titles.sort();
            titles
        })
    }

    fn active_title(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| {
            let window = cx.active_window()?.downcast::<ExcelApp>()?;
            window.read(cx).ok().map(|app| app.title.clone())
        })
    }

    #[gpui::test]
    fn files_open_side_by_side_in_windows_of_their_own(cx: &mut TestAppContext) {
        let dir = folder("side");
        let (a, b) = (dir.join("Alpha.csv"), dir.join("Beta.csv"));
        std::fs::write(&a, "1,2\n3,4\n").unwrap();
        std::fs::write(&b, "x,y\n").unwrap();
        cx.update(|cx| {
            crate::bind_keys(cx);
            open_window(cx, |_, _, _| {});
        });
        cx.run_until_parked();
        assert_eq!(titles(cx), ["Book1"]);

        // The first file takes the place of the untouched Book1, the
        // second gets a window of its own.
        cx.update(|cx| open_paths(vec![a.clone(), b.clone()], cx));
        cx.run_until_parked();
        assert_eq!(titles(cx), ["Alpha", "Beta"]);
        assert_eq!(active_title(cx).as_deref(), Some("Beta"));

        // Asked again (by another spelling of its path), Alpha only comes
        // to the front.
        cx.update(|cx| open_paths(vec![dir.join(".").join("Alpha.csv")], cx));
        cx.run_until_parked();
        assert_eq!(titles(cx), ["Alpha", "Beta"]);
        assert_eq!(active_title(cx).as_deref(), Some("Alpha"));

        // Each window keeps its own workbook and takes its own commands.
        let windows: Vec<WindowHandle<ExcelApp>> = cx.update(|cx| {
            cx.windows()
                .into_iter()
                .filter_map(|w| w.downcast())
                .collect()
        });
        let alpha = windows
            .iter()
            .copied()
            .find(|w| cx.update(|cx| w.read(cx).unwrap().title == "Alpha"))
            .unwrap();
        let beta = windows.iter().copied().find(|w| *w != alpha).unwrap();
        let cell = |w: WindowHandle<ExcelApp>, cx: &mut TestAppContext| {
            cx.update(|cx| w.read(cx).unwrap().wb.sheets[0].value_at(0, 0))
        };
        assert_eq!(cell(alpha, cx), Value::Number(1.0));
        assert_eq!(cell(beta, cx), Value::Text("x".into()));
        cx.update(|cx| {
            beta.update(cx, |_, window, cx| {
                window.dispatch_action(Box::new(crate::app::ZoomIn), cx)
            })
            .unwrap();
        });
        cx.run_until_parked();
        let zoom = |w: WindowHandle<ExcelApp>, cx: &mut TestAppContext| {
            cx.update(|cx| w.read(cx).unwrap().zoom)
        };
        assert!(zoom(beta, cx) > 1.0);
        assert_eq!(zoom(alpha, cx), 1.0);

        // Closing one leaves the other as it was.
        cx.update(|cx| {
            beta.update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        cx.run_until_parked();
        assert_eq!(titles(cx), ["Alpha"]);
        assert_eq!(cell(alpha, cx), Value::Number(1.0));

        // Another file, with Alpha open: a new window.
        cx.update(|cx| open_paths(vec![b.clone()], cx));
        cx.run_until_parked();
        assert_eq!(titles(cx), ["Alpha", "Beta"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui::test]
    fn a_book_with_changes_is_never_replaced(cx: &mut TestAppContext) {
        let dir = folder("changed");
        let a = dir.join("Alpha.csv");
        std::fs::write(&a, "1\n").unwrap();
        let book = cx.update(|cx| {
            crate::bind_keys(cx);
            open_window(cx, |app, _, _| app.wb.modified = true).unwrap()
        });
        cx.run_until_parked();
        cx.update(|cx| open_paths(vec![a.clone()], cx));
        cx.run_until_parked();
        assert_eq!(titles(cx), ["Alpha", "Book1"]);
        assert!(cx.update(|cx| book.read(cx).unwrap().wb.modified));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui::test]
    fn quitting_asks_about_every_window_with_changes(cx: &mut TestAppContext) {
        let changed = |title: &'static str| {
            move |app: &mut ExcelApp, _: &mut Window, _: &mut Context<ExcelApp>| {
                app.title = title.to_string();
                app.wb.modified = true;
            }
        };
        let (alpha, beta) = cx.update(|cx| {
            crate::bind_keys(cx);
            let alpha = open_window(cx, changed("Alpha")).unwrap();
            let beta = open_window(cx, changed("Beta")).unwrap();
            // Nothing to ask about this one.
            open_window(cx, |_, _, _| {}).unwrap();
            (alpha, beta)
        });
        cx.run_until_parked();
        cx.update(|cx| {
            alpha
                .update(cx, |app, window, cx| app.request_quit(window, cx))
                .unwrap()
        });
        cx.run_until_parked();
        let (question, _) = cx.pending_prompt().expect("a question for Alpha");
        assert!(question.contains("Alpha"), "{question}");
        cx.simulate_prompt_answer("Discard");
        cx.run_until_parked();
        // Then Beta, brought to the front; Cancel stops there.
        let (question, _) = cx.pending_prompt().expect("a question for Beta");
        assert!(question.contains("Beta"), "{question}");
        assert_eq!(active_title(cx).as_deref(), Some("Beta"));
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        assert!(cx.update(|cx| beta.read(cx).unwrap().wb.modified));
    }
}
