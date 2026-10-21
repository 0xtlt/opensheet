//! The printers of the system, through the printing commands of macOS
//! (CUPS): `lpstat` lists them, `lpoptions` describes one, `lp` takes a
//! job.
//!
//! Every command is started by [`Tool::run`] and by nothing else. The
//! program each tool runs can be replaced through an environment variable
//! (`OPENSHEET_LP`, `OPENSHEET_LPSTAT`, `OPENSHEET_LPOPTIONS`,
//! `OPENSHEET_OPEN`): a copy of the application that is only being tried
//! out is pointed at stand-ins, so that no job reaches a real printer. In
//! the tests the names come from [`stand_in`] instead (the environment
//! belongs to the whole process, and the tests run side by side); a tool
//! without a stand-in is not run at all there.
//!
//! What the commands print is read by the functions below. Only what CUPS
//! does not translate is relied on: the names `lpstat -e` lists, the
//! `key=value` pairs of `lpoptions`, the keywords of a printer's options.

use std::ffi::OsString;
use std::io;
use std::process::{Command, Output};

/// A program of the system the Print screen runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tool {
    /// Sends a file to a printer.
    Lp,
    /// Lists the printers and names the default one.
    Lpstat,
    /// Describes a printer: its name for people, what it can do.
    Lpoptions,
    /// Opens a file in an application, or a pane of System Settings.
    Open,
}

impl Tool {
    /// The environment variable that names the program to run instead.
    pub(crate) fn variable(self) -> &'static str {
        match self {
            Tool::Lp => "OPENSHEET_LP",
            Tool::Lpstat => "OPENSHEET_LPSTAT",
            Tool::Lpoptions => "OPENSHEET_LPOPTIONS",
            Tool::Open => "OPENSHEET_OPEN",
        }
    }

    fn usual(self) -> &'static str {
        match self {
            Tool::Lp => "lp",
            Tool::Lpstat => "lpstat",
            Tool::Lpoptions => "lpoptions",
            Tool::Open => "open",
        }
    }

    /// The program this tool runs now. It is looked up when asked, on the
    /// thread that asks, so that work handed to another thread runs what
    /// was chosen here.
    pub(crate) fn program(self) -> Option<OsString> {
        #[cfg(test)]
        {
            STAND_INS.with(|map| map.borrow().get(&self).cloned())
        }
        #[cfg(not(test))]
        {
            Some(chosen(self.usual(), std::env::var_os(self.variable())))
        }
    }

    /// Run the tool and wait for it: the one place a printing command is
    /// started from.
    pub(crate) fn run(self, arguments: &[String]) -> io::Result<Output> {
        run(self, self.program(), arguments)
    }
}

/// Run `program` for a tool (see [`Tool::program`]).
pub(crate) fn run(
    tool: Tool,
    program: Option<OsString>,
    arguments: &[String],
) -> io::Result<Output> {
    let Some(program) = program else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{} is not run here ({} names no stand-in)",
                tool.usual(),
                tool.variable()
            ),
        ));
    };
    Command::new(program).args(arguments).output()
}

/// The program to run: the one the variable names, else the usual one.
pub(crate) fn chosen(usual: &str, named: Option<OsString>) -> OsString {
    named
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| usual.into())
}

#[cfg(test)]
thread_local! {
    static STAND_INS: std::cell::RefCell<std::collections::HashMap<Tool, OsString>> = Default::default();
}

/// In a test, the program a tool runs (what the environment variable does
/// for the application).
#[cfg(test)]
pub(crate) fn stand_in(tool: Tool, program: impl Into<OsString>) {
    STAND_INS.with(|map| map.borrow_mut().insert(tool, program.into()));
}

// ------------------------------------------------------------ the printers

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Printer {
    /// The name of the queue, as the commands want it.
    pub name: String,
    /// The name people gave it ("HP LaserJet Pro").
    pub title: String,
}

/// How a job uses the two sides of the paper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Sides {
    #[default]
    One,
    /// Pages turn like those of a book.
    LongEdge,
    /// Pages turn like those of a notepad.
    ShortEdge,
}

impl Sides {
    pub(crate) const ALL: [Sides; 3] = [Sides::One, Sides::LongEdge, Sides::ShortEdge];

    fn keyword(self) -> &'static str {
        match self {
            Sides::One => "one-sided",
            Sides::LongEdge => "two-sided-long-edge",
            Sides::ShortEdge => "two-sided-short-edge",
        }
    }
}

/// The names `lpstat -e` lists, one printer on each line.
pub(crate) fn parse_names(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// The default printer in what `lpstat -d` says. The sentence is
/// translated by the system ("system default destination: X",
/// "destination système par défaut : X"), so only its end is read, and
/// only a printer that exists is believed.
pub(crate) fn parse_default(output: &str, names: &[String]) -> Option<String> {
    let line = output.lines().find(|line| line.contains(':'))?;
    let name = line.rsplit(':').next()?.trim();
    names.iter().find(|known| known.as_str() == name).cloned()
}

/// The value of `key` among the `key=value` pairs `lpoptions -p` prints
/// (values with spaces are quoted).
pub(crate) fn option_value(output: &str, key: &str) -> Option<String> {
    let mut rest = output;
    while let Some(at) = rest.find(key) {
        let before_ok = at == 0 || rest[..at].ends_with(char::is_whitespace);
        let after = &rest[at + key.len()..];
        if let (true, Some(value)) = (before_ok, after.strip_prefix('=')) {
            let text = match value.strip_prefix('\'') {
                Some(quoted) => quoted.split('\'').next().unwrap_or_default(),
                None => value.split_whitespace().next().unwrap_or_default(),
            };
            return Some(text.to_string());
        }
        rest = after;
    }
    None
}

/// Whether a printer prints on both sides, from the options
/// `lpoptions -p … -l` lists: a `Duplex` option (or the `sides` of a
/// printer without a driver) with a choice other than one side.
pub(crate) fn parse_two_sided(output: &str) -> bool {
    output.lines().any(|line| {
        let Some((option, choices)) = line.split_once(':') else {
            return false;
        };
        let keyword = option.split('/').next().unwrap_or_default().trim();
        (keyword.eq_ignore_ascii_case("Duplex") || keyword.eq_ignore_ascii_case("sides"))
            && choices.split_whitespace().any(|choice| {
                let choice = choice.trim_start_matches('*');
                choice.starts_with("Duplex") || choice.starts_with("two-sided")
            })
    })
}

fn text_of(result: io::Result<Output>) -> String {
    result
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

/// The printers of the system and the default one; none at all when the
/// system has none or cannot say.
pub(crate) fn printers() -> (Vec<Printer>, Option<String>) {
    let names = parse_names(&text_of(Tool::Lpstat.run(&["-e".to_string()])));
    if names.is_empty() {
        return (Vec::new(), None);
    }
    let default = parse_default(&text_of(Tool::Lpstat.run(&["-d".to_string()])), &names);
    let printers = names
        .into_iter()
        .map(|name| {
            let described = text_of(Tool::Lpoptions.run(&["-p".to_string(), name.clone()]));
            let title = option_value(&described, "printer-info")
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| name.replace('_', " "));
            Printer { name, title }
        })
        .collect();
    (printers, default)
}

/// Whether a printer prints on both sides. `program` is what
/// `Tool::Lpoptions.program()` gave: the question is asked away from the
/// window, since a printer on the network may take its time to answer.
pub(crate) fn two_sided(program: Option<OsString>, printer: &str) -> bool {
    let arguments = ["-p".to_string(), printer.to_string(), "-l".to_string()];
    parse_two_sided(&text_of(run(Tool::Lpoptions, program, &arguments)))
}

/// A job as `lp` is told about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ticket<'a> {
    pub printer: &'a str,
    pub copies: u32,
    pub collate: bool,
    /// `None` when the printer has one side only: nothing is asked of it.
    pub sides: Option<Sides>,
    pub title: &'a str,
    pub file: &'a str,
}

/// The arguments of `lp` for a job.
pub(crate) fn lp_arguments(ticket: &Ticket) -> Vec<String> {
    let mut args = vec![
        "-d".to_string(),
        ticket.printer.to_string(),
        "-n".to_string(),
        ticket.copies.max(1).to_string(),
    ];
    if ticket.copies > 1 {
        args.extend(["-o".to_string(), format!("collate={}", ticket.collate)]);
    }
    if let Some(sides) = ticket.sides {
        args.extend(["-o".to_string(), format!("sides={}", sides.keyword())]);
    }
    args.extend([
        "-t".to_string(),
        ticket.title.to_string(),
        ticket.file.to_string(),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_names_the_program() {
        assert_eq!(chosen("lp", None), OsString::from("lp"));
        assert_eq!(chosen("lp", Some("".into())), OsString::from("lp"));
        assert_eq!(
            chosen("lp", Some("/tmp/stand-in/lp".into())),
            OsString::from("/tmp/stand-in/lp")
        );
        assert_eq!(Tool::Lp.variable(), "OPENSHEET_LP");
        assert_eq!(Tool::Lpstat.variable(), "OPENSHEET_LPSTAT");
        assert_eq!(Tool::Lpoptions.variable(), "OPENSHEET_LPOPTIONS");
    }

    #[test]
    fn a_test_never_runs_the_real_commands() {
        // No stand-in was named on this thread: nothing is started.
        for tool in [Tool::Lp, Tool::Lpstat, Tool::Lpoptions, Tool::Open] {
            assert!(tool.program().is_none());
            assert!(tool.run(&[]).is_err());
        }
        assert_eq!(printers(), (Vec::new(), None));
    }

    #[test]
    fn printers_are_read_whatever_the_language_of_the_system() {
        let names = parse_names("HP_LaserJet_Pro\nCanon_TS5000\n\n");
        assert_eq!(names, vec!["HP_LaserJet_Pro", "Canon_TS5000"]);
        assert_eq!(
            parse_default("system default destination: Canon_TS5000\n", &names).as_deref(),
            Some("Canon_TS5000")
        );
        assert_eq!(
            parse_default("destination système par défaut : HP_LaserJet_Pro\n", &names).as_deref(),
            Some("HP_LaserJet_Pro")
        );
        assert_eq!(
            parse_default("no system default destination\n", &names),
            None
        );
        assert_eq!(
            parse_default("aucune destination système par défaut\n", &names),
            None
        );
        // A printer that was removed is not a default.
        assert_eq!(
            parse_default("system default destination: Gone\n", &names),
            None
        );
        assert!(parse_names("").is_empty());
    }

    #[test]
    fn a_printer_is_described_by_lpoptions() {
        let described = "copies=1 device-uri=ipp://hp.local/ipp/print finishings=3 printer-info='HP LaserJet Pro' \
                         printer-is-shared=false printer-location='2nd floor' printer-make-and-model='HP LaserJet'";
        assert_eq!(
            option_value(described, "printer-info").as_deref(),
            Some("HP LaserJet Pro")
        );
        assert_eq!(option_value(described, "copies").as_deref(), Some("1"));
        assert_eq!(
            option_value(described, "printer-is-shared").as_deref(),
            Some("false")
        );
        assert_eq!(option_value(described, "info"), None);
        assert_eq!(option_value("", "printer-info"), None);
    }

    #[test]
    fn two_sided_printing_is_read_from_the_options() {
        let laser = "PageSize/Media Size: *A4 Letter Legal\nDuplex/Two-Sided: None *DuplexNoTumble DuplexTumble\nColorModel/Color Mode: *Gray\n";
        assert!(parse_two_sided(laser));
        assert!(parse_two_sided(
            "sides/Sides: *one-sided two-sided-long-edge two-sided-short-edge\n"
        ));
        assert!(!parse_two_sided(
            "PageSize/Media Size: *A4 Letter\nDuplex/Two-Sided: *None\n"
        ));
        assert!(!parse_two_sided("PageSize/Media Size: *A4 Letter\n"));
        assert!(!parse_two_sided(""));
    }

    #[test]
    fn lp_gets_the_job() {
        let ticket = Ticket {
            printer: "HP_LaserJet_Pro",
            copies: 1,
            collate: true,
            sides: None,
            title: "Budget",
            file: "/tmp/Budget.pdf",
        };
        assert_eq!(
            lp_arguments(&ticket),
            vec![
                "-d",
                "HP_LaserJet_Pro",
                "-n",
                "1",
                "-t",
                "Budget",
                "/tmp/Budget.pdf"
            ]
        );
        let several = Ticket {
            printer: "P",
            copies: 3,
            collate: false,
            sides: Some(Sides::ShortEdge),
            title: "B",
            file: "f.pdf",
        };
        assert_eq!(
            lp_arguments(&several),
            vec![
                "-d",
                "P",
                "-n",
                "3",
                "-o",
                "collate=false",
                "-o",
                "sides=two-sided-short-edge",
                "-t",
                "B",
                "f.pdf"
            ]
        );
        let one_side = Ticket {
            sides: Some(Sides::One),
            ..ticket
        };
        assert!(lp_arguments(&one_side).contains(&"sides=one-sided".to_string()));
    }
}
