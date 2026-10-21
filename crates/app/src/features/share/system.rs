//! What sharing asks of macOS, behind a trait: the application talks to
//! `system()`, which is the real thing in the application and a stand-in
//! that only records what it was asked in the tests, so that no test ever
//! opens a share sheet or sends anything.

use gpui::{App, Bounds, Pixels, Window};
use std::path::PathBuf;
use std::rc::Rc;

/// Something handed to the share sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A file, sent as an attachment.
    File(PathBuf),
    /// Plain text (the body of a message).
    Text(String),
}

pub trait System {
    /// Show the share sheet of macOS (Mail, Messages, AirDrop…) for
    /// `items`, hanging under the box `under` of the window. The user
    /// chooses there what happens next; nothing is sent by this call.
    fn share(
        &self,
        items: &[Item],
        under: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String>;

    /// The version of macOS, as "macOS 15.1".
    fn os_version(&self) -> String;
}

#[cfg(all(target_os = "macos", not(test)))]
pub fn system() -> Rc<dyn System> {
    Rc::new(mac::Mac)
}

#[cfg(all(not(target_os = "macos"), not(test)))]
pub fn system() -> Rc<dyn System> {
    Rc::new(other::Other)
}

#[cfg(test)]
pub fn system() -> Rc<dyn System> {
    Rc::new(fake::Fake)
}

/// Systems without a share sheet: the copy is made, and the message says
/// where it is so that it can be attached by hand.
#[cfg(all(not(target_os = "macos"), not(test)))]
mod other {
    use super::{Item, System};
    use gpui::{App, Bounds, Pixels, Window};

    pub struct Other;

    impl System for Other {
        fn share(
            &self,
            items: &[Item],
            _: Bounds<Pixels>,
            _: &mut Window,
            _: &mut App,
        ) -> Result<(), String> {
            let files: Vec<String> = items
                .iter()
                .filter_map(|item| match item {
                    Item::File(path) => Some(path.display().to_string()),
                    Item::Text(_) => None,
                })
                .collect();
            Err(format!(
                "This system has no share sheet. The copy is here: {}",
                files.join(", ")
            ))
        }

        fn os_version(&self) -> String {
            std::env::consts::OS.to_string()
        }
    }
}

// The macros of `objc` test a `cfg` this crate does not declare.
#[cfg(all(target_os = "macos", not(test)))]
#[allow(unexpected_cfgs)]
mod mac {
    use super::{Item, System};
    use gpui::{App, Bounds, Pixels, Window};
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub struct Mac;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NsPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NsSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NsRect {
        origin: NsPoint,
        size: NsSize,
    }

    /// `NSUTF8StringEncoding`.
    const UTF8: usize = 4;
    /// `NSRectEdgeMinY`: the bottom edge of a box in a view whose y axis
    /// points up, as the view of the window does.
    const BELOW: usize = 1;

    /// An autoreleased `NSString`.
    unsafe fn ns_string(text: &str) -> *mut Object {
        unsafe {
            let string: *mut Object = msg_send![class!(NSString), alloc];
            let string: *mut Object =
                msg_send![string, initWithBytes: text.as_ptr() length: text.len() encoding: UTF8];
            msg_send![string, autorelease]
        }
    }

    /// Show the picker. Called from a task of its own: the sheet runs an
    /// event loop of its own, during which the window must be free to
    /// handle its events.
    unsafe fn show(view: *mut Object, items: &[Item], under: (f64, f64, f64, f64)) {
        unsafe {
            let list: *mut Object = msg_send![class!(NSMutableArray), array];
            for item in items {
                let object: *mut Object = match item {
                    Item::File(path) => {
                        let path = ns_string(&path.to_string_lossy());
                        msg_send![class!(NSURL), fileURLWithPath: path]
                    }
                    Item::Text(text) => ns_string(text),
                };
                if !object.is_null() {
                    let () = msg_send![list, addObject: object];
                }
            }
            // The view counts from its bottom-left corner, the window of
            // the application from its top-left one.
            let bounds: NsRect = msg_send![view, bounds];
            let (left, top, width, height) = under;
            let rect = NsRect {
                origin: NsPoint {
                    x: left,
                    y: bounds.size.height - top - height,
                },
                size: NsSize { width, height },
            };
            let picker: *mut Object = msg_send![class!(NSSharingServicePicker), alloc];
            let picker: *mut Object = msg_send![picker, initWithItems: list];
            if picker.is_null() {
                return;
            }
            let () = msg_send![picker, showRelativeToRect: rect ofView: view preferredEdge: BELOW];
            let () = msg_send![picker, autorelease];
        }
    }

    impl System for Mac {
        fn share(
            &self,
            items: &[Item],
            under: Bounds<Pixels>,
            window: &mut Window,
            cx: &mut App,
        ) -> Result<(), String> {
            let view = match window.window_handle().map(|handle| handle.as_raw()) {
                Ok(RawWindowHandle::AppKit(handle)) => handle.ns_view.as_ptr() as *mut Object,
                _ => return Err("no window".to_string()),
            };
            let items = items.to_vec();
            let under = (
                f64::from(f32::from(under.left())),
                f64::from(f32::from(under.top())),
                f64::from(f32::from(under.size.width)),
                f64::from(f32::from(under.size.height)),
            );
            cx.spawn(async move |_| unsafe { show(view, &items, under) })
                .detach();
            Ok(())
        }

        fn os_version(&self) -> String {
            let version = std::process::Command::new("sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .unwrap_or_default();
            format!("macOS {version}").trim().to_string()
        }
    }
}

/// The stand-in of the tests: it notes what was shared and where.
#[cfg(test)]
pub mod fake {
    use super::{Item, System};
    use gpui::{App, Bounds, Pixels, Window};
    use std::cell::{Cell, RefCell};

    thread_local! {
        static SHARED: RefCell<Vec<(Vec<Item>, Bounds<Pixels>)>> = const { RefCell::new(Vec::new()) };
        static FAILING: Cell<bool> = const { Cell::new(false) };
    }

    pub struct Fake;

    /// What was handed to the share sheet since the last call, oldest
    /// first, with the box each sheet hung from.
    pub fn taken() -> Vec<(Vec<Item>, Bounds<Pixels>)> {
        SHARED.with(|shared| std::mem::take(&mut *shared.borrow_mut()))
    }

    /// Make the share sheet fail (or work again).
    pub fn fail(on: bool) {
        FAILING.with(|failing| failing.set(on));
    }

    impl System for Fake {
        fn share(
            &self,
            items: &[Item],
            under: Bounds<Pixels>,
            _: &mut Window,
            _: &mut App,
        ) -> Result<(), String> {
            if FAILING.with(Cell::get) {
                return Err("the share sheet is not available".to_string());
            }
            SHARED.with(|shared| shared.borrow_mut().push((items.to_vec(), under)));
            Ok(())
        }

        fn os_version(&self) -> String {
            "macOS 0.0".to_string()
        }
    }
}
