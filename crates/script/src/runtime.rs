//! Running the JavaScript of a script: a QuickJS runtime that lives for
//! one run, knows nothing but the language and the ExcelScript model,
//! and is stopped when its time or its memory is up.

use std::cell::RefCell;
use std::rc::Rc;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use rquickjs::context::EvalOptions;
use rquickjs::{Context as JsContext, Ctx, Error as JsError, Function, Object, Runtime, Value as JsValue};
use serde_json::Value as J;

use opensheet_engine::recalc::CalcMode;
use opensheet_engine::workbook::Workbook;

use crate::api::{API, Call, ENUMS, Ret};
use crate::compile::Compiled;
use crate::host::{Host, MAX_LOGS};
use crate::{Context, ErrorKind, Log, LogLevel, Outcome, ScriptError};

/// What a script may hold in memory, values read from the workbook
/// included.
const MEMORY_LIMIT: usize = 512 << 20;
/// How deep calls may nest, as stack of the thread the script runs on.
const SCRIPT_STACK: usize = 4 << 20;
/// The stack of that thread: the script's, and room for the engine under
/// the deepest call.
const THREAD_STACK: usize = 32 << 20;
/// The members that change nothing in the workbook though their name does
/// not start with `get`.
const READ_ONLY: [&str; 6] = ["find", "findAll", "isFiltered", "activate", "select", "calculate"];
/// The longest text of one log line, in bytes.
const MAX_LOG_TEXT: usize = 20_000;

const PRELUDE: &str = include_str!("prelude.js");
/// The names the two pieces of code run under: they tell the frames of
/// the script from ours in a stack.
const SCRIPT_NAME: &str = "script.ts";
const PRELUDE_NAME: &str = "opensheet.js";

/// Run a compiled script on its own thread, whose stack is known to be
/// large enough whatever the caller runs on.
pub(crate) fn run(compiled: Compiled, wb: &mut Workbook, context: &Context) -> Outcome {
    let lost = |what: &str| Outcome {
        error: Some(ScriptError {
            kind: ErrorKind::Runtime,
            message: format!("The script could not be run: {what}."),
            line: None,
            column: None,
        }),
        ..Outcome::default()
    };
    if compiled.main_params > 1 {
        return Outcome {
            error: Some(ScriptError {
                kind: ErrorKind::Unsupported,
                message: "Parameters of main other than the workbook (scripts called from Power Automate) are not supported in OpenSheet".to_string(),
                line: Some(compiled.main_at.0),
                column: Some(compiled.main_at.1),
            }),
            ..Outcome::default()
        };
    }
    // What is changed on the sheets directly is undone from this
    // snapshot; the caller has normally taken it already.
    wb.snapshot();
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("office script".to_string())
            .stack_size(THREAD_STACK)
            .spawn_scoped(scope, || execute(compiled, wb, context));
        match spawned {
            Ok(thread) => thread.join().unwrap_or_else(|_| lost("the script engine failed")),
            Err(_) => lost("no thread could be started"),
        }
    })
}

/// How a run ended when it did not reach its end.
struct Thrown {
    kind: ErrorKind,
    message: String,
    /// Line and column in the JavaScript.
    at: Option<(u32, u32)>,
}

fn execute(compiled: Compiled, wb: &mut Workbook, context: &Context) -> Outcome {
    // The runtime keeps what it calls for as long as it lives, so the
    // workbook moves into the host for the time of the run and comes
    // back whatever happens.
    let empty = Workbook::new(wb.locale);
    let mode = wb.calc_mode();
    let front = context.active_sheet.min(wb.sheets.len().saturating_sub(1));
    let host = Rc::new(RefCell::new(Host::new(std::mem::replace(wb, empty), context)));
    let compiled = Rc::new(compiled);
    let timed_out = Arc::new(AtomicBool::new(false));
    let armed = Arc::new(AtomicBool::new(true));
    let ended = catch_unwind(AssertUnwindSafe(|| match start(context, &timed_out, &armed) {
        Ok((runtime, js)) => {
            let ended = js.with(|ctx| drive(&ctx, compiled.clone(), host.clone(), armed.clone()));
            drop(js);
            drop(runtime);
            ended
        }
        Err(message) => Err(Thrown {
            kind: ErrorKind::Runtime,
            message,
            at: None,
        }),
    }))
    .unwrap_or_else(|_| {
        Err(Thrown {
            kind: ErrorKind::Runtime,
            message: "The script could not be run: the script engine failed.".to_string(),
            at: None,
        })
    });
    // Whatever happens while the run is closed, the workbook goes back
    // to its owner.
    let settled = catch_unwind(AssertUnwindSafe(|| settle(&mut host.borrow_mut(), ended, timed_out.load(Ordering::Relaxed), &compiled, context, mode, front)));
    std::mem::swap(wb, &mut host.borrow_mut().wb);
    settled.unwrap_or_else(|_| Outcome {
        error: Some(ScriptError {
            kind: ErrorKind::Runtime,
            message: "The script could not be run: its changes could not be undone.".to_string(),
            line: None,
            column: None,
        }),
        ..Outcome::default()
    })
}

/// Close a run: what it reports when it reached its end, the workbook as
/// it was when it did not.
fn settle(host: &mut Host, ended: Result<Option<String>, Thrown>, timed_out: bool, compiled: &Compiled, context: &Context, mode: CalcMode, front: usize) -> Outcome {
    let ended = match (host.fatal.take(), timed_out, ended) {
        (Some(message), _, _) => Err(Thrown {
            kind: ErrorKind::Runtime,
            message,
            at: None,
        }),
        (None, true, Err(thrown)) => Err(Thrown {
            kind: ErrorKind::Timeout,
            message: format!("The script was stopped: it ran longer than {}.", seconds(context)),
            at: thrown.at,
        }),
        (None, _, ended) => ended,
    };
    let logs = std::mem::take(&mut host.logs);
    match ended {
        Ok(returned) => {
            if !host.edited {
                // A script that only read leaves no step to undo: the
                // snapshot goes, and the caller has nothing to commit.
                host.wb.rollback();
            }
            Outcome {
                logs,
                error: None,
                returned,
                // The sheet in front may have moved without being
                // activated: a sheet added, deleted or moved before it.
                activated: (host.activated || host.active_sheet() != front).then(|| host.active_sheet()),
                selected: host.selected(),
                autofit: host.autofits(),
            }
        }
        Err(thrown) => {
            // A script that fails changes nothing: its changes go here,
            // and the results of formulas calculated on the way with them.
            host.wb.rollback();
            // The calculation mode is not part of what is undone.
            if host.wb.calc_mode() != mode {
                host.wb.set_calc_mode(mode);
            }
            if host.calculated {
                host.wb.calculate_now();
            }
            let origin = thrown.at.and_then(|(line, column)| compiled.origin(line, column));
            Outcome {
                logs,
                error: Some(ScriptError {
                    kind: thrown.kind,
                    message: thrown.message,
                    line: origin.map(|o| o.0),
                    column: origin.map(|o| o.1),
                }),
                ..Outcome::default()
            }
        }
    }
}

fn seconds(context: &Context) -> String {
    let limit = context.time_limit.as_secs_f64();
    if limit == 1.0 {
        "1 second".to_string()
    } else if limit.fract() == 0.0 {
        format!("{limit} seconds")
    } else {
        format!("{limit:.1} seconds")
    }
}

/// A runtime with its limits.
fn start(context: &Context, timed_out: &Arc<AtomicBool>, armed: &Arc<AtomicBool>) -> Result<(Runtime, JsContext), String> {
    let failed = |_| "The script engine could not be started.".to_string();
    let runtime = Runtime::new().map_err(failed)?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(SCRIPT_STACK);
    let deadline = Instant::now() + context.time_limit;
    let (timed_out, armed) = (timed_out.clone(), armed.clone());
    // QuickJS asks this every few thousand jumps and calls: answering
    // yes stops the script with an error it cannot catch.
    runtime.set_interrupt_handler(Some(Box::new(move || {
        if armed.load(Ordering::Relaxed) && Instant::now() >= deadline {
            timed_out.store(true, Ordering::Relaxed);
            return true;
        }
        false
    })));
    let js = JsContext::full(&runtime).map_err(failed)?;
    Ok((runtime, js))
}

fn options(name: &str) -> EvalOptions {
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = true;
    options.filename = Some(name.to_string());
    options
}

/// The classes and the enums, as the prelude builds them.
fn spec() -> (String, String) {
    let classes: Vec<J> = API
        .iter()
        .map(|class| {
            let members: Vec<&str> = class.members.iter().map(|m| m.name).collect();
            serde_json::json!({ "name": class.name, "members": members })
        })
        .collect();
    let enums: Vec<J> = ENUMS
        .iter()
        .map(|e| serde_json::json!({ "name": e.name, "members": e.members }))
        .collect();
    (J::from(classes).to_string(), J::from(enums).to_string())
}

/// Load the model and the script, call `main` and wait for it. Gives
/// what `main` returned as JSON text.
fn drive<'js>(ctx: &Ctx<'js>, compiled: Rc<Compiled>, host: Rc<RefCell<Host>>, armed: Arc<AtomicBool>) -> Result<Option<String>, Thrown> {
    // Something of ours failing is not the script's doing.
    let internal = |_: JsError| Thrown {
        kind: ErrorKind::Runtime,
        message: "The script engine could not be started.".to_string(),
        at: None,
    };
    let calls = host.clone();
    let op = Function::new(ctx.clone(), move |class: usize, member: usize, payload: String| dispatch(&calls, class, member, &payload))
        .map_err(internal)?;
    let places = compiled.clone();
    let log = Function::new(ctx.clone(), move |level: i32, text: String, stack: String| {
        let mut host = host.borrow_mut();
        if host.logs.len() > MAX_LOGS {
            return;
        }
        if host.logs.len() == MAX_LOGS {
            host.logs.push(Log {
                level: LogLevel::Warn,
                text: format!("The log is full: what the script wrote after {MAX_LOGS} lines is not kept."),
                line: None,
            });
            return;
        }
        let mut text = text;
        if text.len() > MAX_LOG_TEXT {
            let mut cut = MAX_LOG_TEXT;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        host.logs.push(Log {
            level: match level {
                1 => LogLevel::Warn,
                2 => LogLevel::Error,
                _ => LogLevel::Log,
            },
            text,
            line: frame(&stack).and_then(|(line, column)| places.origin(line, column)).map(|o| o.0),
        });
    })
    .map_err(internal)?;
    let prelude: Function = ctx.eval_with_options(PRELUDE, options(PRELUDE_NAME)).map_err(internal)?;
    let (classes, enums) = spec();
    let tools: Object = prelude.call((op, log, classes, enums)).map_err(internal)?;
    let run: Function = tools.get("run").map_err(internal)?;
    let describe: Function = tools.get("describe").map_err(internal)?;

    // What the script threw, read once nothing can stop us reading it.
    let thrown = |error: JsError, value: Option<JsValue<'js>>| -> Thrown {
        armed.store(false, Ordering::Relaxed);
        let value = match (value, &error) {
            (Some(value), _) => value,
            (None, JsError::Exception) => ctx.catch(),
            (None, other) => {
                return Thrown {
                    kind: ErrorKind::Runtime,
                    message: other.to_string(),
                    at: None,
                };
            }
        };
        let text: String = describe.call((value,)).unwrap_or_default();
        let told: J = serde_json::from_str(&text).unwrap_or(J::Null);
        let field = |key: &str| told.get(key).and_then(J::as_str).unwrap_or_default().to_string();
        let (name, message, stack) = (field("name"), field("message"), field("stack"));
        let kind = match (field("kind").as_str(), name.as_str()) {
            ("Unsupported", _) => ErrorKind::Unsupported,
            (_, "SyntaxError") => ErrorKind::Syntax,
            _ => ErrorKind::Runtime,
        };
        // Errors of the language say what they are (TypeError: ...);
        // those of the model already do.
        let message = match name.as_str() {
            "" | "Error" => message,
            _ if kind == ErrorKind::Unsupported => message,
            _ => format!("{name}: {message}"),
        };
        Thrown {
            kind,
            message,
            at: frame(&stack),
        }
    };

    let source = format!("{}\n;typeof main === \"function\" ? main : undefined", compiled.js);
    let main: JsValue = ctx.eval_with_options(source, options(SCRIPT_NAME)).map_err(|e| thrown(e, None))?;
    let state: Object = run.call((main,)).map_err(|e| thrown(e, None))?;
    while ctx.execute_pending_job() {}
    if ctx.has_exception() {
        return Err(thrown(JsError::Exception, None));
    }
    let flag = |key: &str| state.get::<_, bool>(key).unwrap_or(false);
    if !flag("done") {
        return Err(Thrown {
            kind: ErrorKind::Runtime,
            message: "main did not finish: it waits for something that never comes.".to_string(),
            at: None,
        });
    }
    if flag("failed") {
        let error: JsValue = state.get("error").map_err(|e| thrown(e, None))?;
        return Err(thrown(JsError::Exception, Some(error)));
    }
    Ok(state.get::<_, Option<String>>("json").unwrap_or(None))
}

/// The first place of the script in a stack: 1-based line and column of
/// the JavaScript.
fn frame(stack: &str) -> Option<(u32, u32)> {
    let marker = format!("{SCRIPT_NAME}:");
    stack.lines().find_map(|line| {
        let rest = &line[line.find(&marker)? + marker.len()..];
        let mut numbers = rest.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty());
        let row = numbers.next()?.parse().ok()?;
        let column = numbers.next().and_then(|n| n.parse().ok()).unwrap_or(1);
        Some((row, column))
    })
}

/// One call of a method of the model. The answer starts with a letter:
/// `u` for undefined, `v` and JSON data, `o` and objects of the model,
/// `d` and the milliseconds of a date, `e` and an error.
fn dispatch(host: &RefCell<Host>, class: usize, member: usize, payload: &str) -> String {
    let error = |kind: ErrorKind, message: &str| {
        let kind = if kind == ErrorKind::Unsupported { "Unsupported" } else { "Runtime" };
        format!("e{}", serde_json::json!({ "k": kind, "m": message }))
    };
    let Some((class, member)) = API.get(class).and_then(|c| Some((c, c.members.get(member)?))) else {
        return error(ErrorKind::Runtime, "No such member.");
    };
    let payload: J = serde_json::from_str(payload).unwrap_or(J::Null);
    let part = |index: usize| payload.get(index).and_then(J::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let call = Call {
        class: class.name,
        member: member.name,
        this: part(0),
        args: part(1),
    };
    let mut host = host.borrow_mut();
    if host.fatal.is_some() {
        return error(ErrorKind::Runtime, "The script was stopped.");
    }
    host.edited |= !(member.name.starts_with("get") || READ_ONLY.contains(&member.name));
    // The engine must not take the application down with it: what it
    // could not do ends the run, and the run is undone.
    match catch_unwind(AssertUnwindSafe(|| (member.run)(&mut host, &call))) {
        Ok(Ok(answer)) => {
            crate::api::note_call(class.name, member.name);
            match answer {
                Ret::Nothing => "u".to_string(),
                Ret::Value(value) => format!("v{value}"),
                Ret::Object(value) => format!("o{value}"),
                Ret::Date(milliseconds) => format!("d{milliseconds}"),
            }
        }
        Ok(Err(fail)) => error(fail.kind, &fail.message),
        Err(_) => {
            let message = format!("{}.{} failed inside OpenSheet; the script was stopped.", class.name, member.name);
            host.fatal = Some(message.clone());
            error(ErrorKind::Runtime, &message)
        }
    }
}
