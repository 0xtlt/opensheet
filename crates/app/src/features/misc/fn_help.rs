//! What the Insert Function and Function Arguments dialogs say about a
//! function: how its arguments are called and what they are for.
//!
//! The engine's registry knows the name of a function, its category and
//! how many arguments it takes, nothing more. Microsoft Excel carries the
//! names of the arguments and a description of each function and argument
//! in its resources, in every language of the interface. Those texts are
//! Microsoft's and OpenSheet holds no copy of them: when Excel is installed
//! on the machine they are read from its bundle (as `load_office_fonts`
//! does for the Office fonts); otherwise the dialogs show the syntax with
//! numbered arguments and no description.

use crate::i18n::{Lang, t};
use opensheet_engine::Locale;
use opensheet_engine::functions::{self, FunctionDef, VAR};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What is known about a function besides what the registry says.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FunctionHelp {
    /// Names of the arguments, in order.
    pub args: Vec<String>,
    /// The last names stand for as many arguments as the user gives
    /// (`number1,number2,...`).
    pub repeats: bool,
    pub description: String,
    /// One per argument; may be shorter than `args`.
    pub arg_descriptions: Vec<String>,
}

/// Where Excel keeps its resources. `OPENSHEET_EXCEL_RESOURCES` names
/// another folder (or none at all, to do without).
fn resources() -> PathBuf {
    match std::env::var_os("OPENSHEET_EXCEL_RESOURCES") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from("/Applications/Microsoft Excel.app/Contents/Resources"),
    }
}

// ----------------------------------------------------- binary property lists

/// A big-endian unsigned integer of 1 to 8 bytes.
fn be_uint(bytes: &[u8]) -> Option<u64> {
    (!bytes.is_empty() && bytes.len() <= 8)
        .then(|| bytes.iter().fold(0u64, |n, b| (n << 8) | *b as u64))
}

/// Reader of the objects of a binary property list.
struct Plist<'a> {
    bytes: &'a [u8],
    offset_size: usize,
    ref_size: usize,
    count: usize,
    table: usize,
}

impl<'a> Plist<'a> {
    fn open(bytes: &'a [u8]) -> Option<(Plist<'a>, usize)> {
        if !bytes.starts_with(b"bplist00") || bytes.len() < 40 {
            return None;
        }
        let trailer = &bytes[bytes.len() - 32..];
        let plist = Plist {
            bytes,
            offset_size: trailer[6] as usize,
            ref_size: trailer[7] as usize,
            count: be_uint(&trailer[8..16])? as usize,
            table: be_uint(&trailer[24..32])? as usize,
        };
        let top = be_uint(&trailer[16..24])? as usize;
        (plist.offset_size >= 1 && plist.ref_size >= 1).then_some((plist, top))
    }

    /// Where object number `index` starts.
    fn offset(&self, index: usize) -> Option<usize> {
        if index >= self.count {
            return None;
        }
        let at = self
            .table
            .checked_add(index.checked_mul(self.offset_size)?)?;
        Some(be_uint(self.bytes.get(at..at.checked_add(self.offset_size)?)?)? as usize)
    }

    /// The count held in the low half of a marker byte, or in the integer
    /// that follows it; and where the content starts.
    fn counted(&self, at: usize) -> Option<(usize, usize)> {
        let low = (*self.bytes.get(at)? & 0x0F) as usize;
        if low != 0x0F {
            return Some((low, at + 1));
        }
        let marker = *self.bytes.get(at + 1)?;
        if marker >> 4 != 0x1 {
            return None;
        }
        let width = 1usize << (marker & 0x0F);
        let value = be_uint(self.bytes.get(at + 2..(at + 2).checked_add(width)?)?)? as usize;
        Some((value, at + 2 + width))
    }

    fn string(&self, index: usize) -> Option<String> {
        let at = self.offset(index)?;
        let (len, start) = self.counted(at)?;
        match *self.bytes.get(at)? >> 4 {
            // ASCII, UTF-8.
            0x5 | 0x7 => {
                String::from_utf8(self.bytes.get(start..start.checked_add(len)?)?.to_vec()).ok()
            }
            // UTF-16, big-endian.
            0x6 => {
                let raw = self
                    .bytes
                    .get(start..start.checked_add(len.checked_mul(2)?)?)?;
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16(&units).ok()
            }
            _ => None,
        }
    }

    fn reference(&self, at: usize) -> Option<usize> {
        Some(be_uint(self.bytes.get(at..at.checked_add(self.ref_size)?)?)? as usize)
    }
}

/// The text → text pairs of a `.strings` file in binary form (what the
/// bundles of macOS applications hold). `None` for anything else.
pub(crate) fn read_strings(bytes: &[u8]) -> Option<HashMap<String, String>> {
    let (plist, top) = Plist::open(bytes)?;
    let at = plist.offset(top)?;
    if *bytes.get(at)? >> 4 != 0xD {
        return None;
    }
    let (count, keys) = plist.counted(at)?;
    let values = keys.checked_add(count.checked_mul(plist.ref_size)?)?;
    // A count the file has no room for is a damaged file.
    if values.checked_add(count.checked_mul(plist.ref_size)?)? > bytes.len() {
        return None;
    }
    let mut out = HashMap::with_capacity(count);
    for i in 0..count {
        let key = plist.reference(keys + i * plist.ref_size)?;
        let value = plist.reference(values + i * plist.ref_size)?;
        if let (Some(key), Some(value)) = (plist.string(key), plist.string(value)) {
            out.insert(key, value);
        }
    }
    Some(out)
}

fn read_strings_file(path: &Path) -> Option<HashMap<String, String>> {
    read_strings(&std::fs::read(path).ok()?)
}

// ------------------------------------------------------ Excel's descriptions

/// Read one entry of Excel's function table:
/// `!arg,arg,...!other form!!description!first argument!second!…!`.
/// `None` when it says nothing (the macro functions have no description).
pub(crate) fn parse_entry(entry: &str) -> Option<FunctionHelp> {
    let mut parts = entry.strip_prefix('!')?.split('!');
    // The forms of the function; the first is the one shown.
    let mut forms: Vec<&str> = Vec::new();
    for part in parts.by_ref() {
        if part.is_empty() {
            break;
        }
        forms.push(part);
    }
    let description = parts.next()?.trim().to_string();
    if description.is_empty() {
        return None;
    }
    let mut args: Vec<String> = forms
        .first()
        .map(|form| form.split(',').map(|arg| arg.trim().to_string()).collect())
        .unwrap_or_default();
    let repeats = args.last().is_some_and(|last| last == "...");
    if repeats {
        args.pop();
    }
    let arg_descriptions: Vec<String> = parts
        .filter(|part| !part.is_empty())
        .take(args.len())
        .map(|part| part.trim().to_string())
        .collect();
    Some(FunctionHelp {
        args,
        repeats,
        description,
        arg_descriptions,
    })
}

/// Excel's texts for the functions of the engine in one language, by
/// canonical name. Empty when Excel is not installed.
fn load(dir: &Path, lang: Lang) -> HashMap<&'static str, FunctionHelp> {
    let mut out = HashMap::new();
    // Functions are numbered: one file gives the (English) name of each
    // number, three others the texts of numbers 0-255, 256-511, 512-767.
    let Some(names) = read_strings_file(&dir.join("en.lproj/strman_0408.strings")) else {
        return out;
    };
    let pages: Vec<HashMap<String, String>> = ["strman_0061", "strman_0062", "strman_0063"]
        .iter()
        .map(|file| {
            read_strings_file(&dir.join(format!("{}.lproj/{file}.strings", lang.code())))
                .unwrap_or_default()
        })
        .collect();
    for (key, name) in &names {
        let Ok(number) = usize::from_str_radix(key, 16) else {
            continue;
        };
        let Some(def) = functions::lookup(name) else {
            continue;
        };
        let entry = pages
            .get(number / 256)
            .and_then(|page| page.get(&format!("{:x}", number % 256)))
            .and_then(|entry| parse_entry(entry));
        if let Some(help) = entry {
            out.insert(def.name, help);
        }
    }
    out
}

/// What Excel says about a function in the language of the interface, if
/// Excel is installed on this machine.
pub(crate) fn help(function: &str) -> Option<&'static FunctionHelp> {
    static TABLES: [OnceLock<HashMap<&'static str, FunctionHelp>>; 5] = [
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
    ];
    let lang = crate::i18n::lang();
    TABLES[lang as usize % TABLES.len()]
        .get_or_init(|| load(&resources(), lang))
        .get(function)
}

// ------------------------------------------------------------------- syntax

/// One argument as the dialogs present it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Arg {
    pub name: String,
    pub description: String,
    /// The function works without it.
    pub optional: bool,
}

/// A name with its trailing number replaced: `number2` → `number5`. A name
/// without a number gets one.
fn renumbered(name: &str, number: usize) -> String {
    let stem = name.trim_end_matches(|ch: char| ch.is_ascii_digit());
    format!("{stem}{number}")
}

/// The first `count` arguments of a function: Excel's names when they are
/// known, "argument1", "argument2"… otherwise. Functions taking any number
/// of arguments go on with the names of their last ones, renumbered.
pub(crate) fn arguments(def: &FunctionDef, help: Option<&FunctionHelp>, count: usize) -> Vec<Arg> {
    let min = def.min_args as usize;
    let fallback = |index: usize| Arg {
        name: format!("{}{}", t("argument", "argument"), index + 1),
        description: String::new(),
        optional: index >= min,
    };
    let Some(help) = help.filter(|help| !help.args.is_empty()) else {
        return (0..count).map(fallback).collect();
    };
    let named = help.args.len();
    // The names that repeat: the last one when it is numbered
    // (`number1,number2,...`), the last two otherwise (`range,criteria,...`).
    let numbered = help.args[named - 1].ends_with(|ch: char| ch.is_ascii_digit());
    let group = if numbered || named < 2 { 1 } else { 2 };
    (0..count)
        .map(|index| {
            if index < named {
                return Arg {
                    name: help.args[index].clone(),
                    description: help
                        .arg_descriptions
                        .get(index)
                        .cloned()
                        .unwrap_or_default(),
                    optional: index >= min,
                };
            }
            if !help.repeats {
                return fallback(index);
            }
            // Which name of the group, and how many times it came before.
            let beyond = index - named;
            let source = named - group + beyond % group;
            let first_number: usize = help.args[source]
                .rsplit(|ch: char| !ch.is_ascii_digit())
                .next()
                .and_then(|digits| digits.parse().ok())
                .unwrap_or(1);
            Arg {
                name: renumbered(&help.args[source], first_number + 1 + beyond / group),
                description: help
                    .arg_descriptions
                    .get(source)
                    .cloned()
                    .unwrap_or_default(),
                optional: true,
            }
        })
        .collect()
}

/// How many argument boxes the Function Arguments dialog shows for a
/// function of which `filled` arguments are typed: the named ones, and
/// always an empty one to go on with when the function takes more.
pub(crate) fn shown_arguments(
    def: &FunctionDef,
    help: Option<&FunctionHelp>,
    filled: usize,
) -> usize {
    let max = if def.max_args == VAR {
        255
    } else {
        def.max_args as usize
    };
    let named = help.map(|help| help.args.len()).unwrap_or(0);
    let base = if def.max_args == VAR {
        named.max(def.min_args as usize).max(1)
    } else {
        max
    };
    base.max(filled + 1).min(max).max(filled)
}

/// The syntax line of the Insert Function dialog: `SUM(number1;number2;...)`.
pub(crate) fn syntax(def: &FunctionDef, help: Option<&FunctionHelp>, locale: &Locale) -> String {
    let sep = locale.arg_sep.to_string();
    let listed = match help {
        Some(help) if !help.args.is_empty() => help.args.len(),
        _ if def.max_args == VAR => (def.min_args as usize).max(1),
        _ => def.max_args as usize,
    };
    let mut names: Vec<String> = arguments(def, help, listed)
        .into_iter()
        .map(|arg| arg.name)
        .collect();
    if def.max_args == VAR {
        names.push("...".to_string());
    }
    format!("{}({})", locale.fn_to_local(def.name), names.join(&sep))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.strings` file in binary form holding three pairs (made with
    /// `plutil -convert binary1`): one value in ASCII, one in UTF-16.
    const SAMPLE: [u8; 365] = [
        98, 112, 108, 105, 115, 116, 48, 48, 211, 1, 2, 3, 4, 5, 6, 81, 52, 82, 49, 97, 85, 99,
        111, 117, 110, 116, 95, 16, 95, 33, 102, 105, 114, 115, 116, 44, 115, 101, 99, 111, 110,
        100, 44, 46, 46, 46, 33, 33, 83, 97, 109, 112, 108, 101, 32, 116, 101, 120, 116, 32, 111,
        102, 32, 97, 32, 102, 117, 110, 99, 116, 105, 111, 110, 33, 119, 104, 97, 116, 32, 116,
        104, 101, 32, 102, 105, 114, 115, 116, 32, 111, 110, 101, 32, 105, 115, 32, 102, 111, 114,
        33, 119, 104, 97, 116, 32, 116, 104, 101, 32, 111, 116, 104, 101, 114, 115, 32, 97, 114,
        101, 32, 102, 111, 114, 33, 111, 16, 95, 0, 33, 0, 118, 0, 97, 0, 108, 0, 101, 0, 117, 0,
        114, 0, 33, 0, 33, 0, 84, 0, 101, 0, 120, 0, 116, 0, 101, 0, 32, 0, 97, 0, 99, 0, 99, 0,
        101, 0, 110, 0, 116, 0, 117, 0, 233, 0, 32, 0, 58, 0, 32, 0, 233, 0, 32, 0, 224, 0, 32, 0,
        231, 0, 33, 0, 117, 0, 110, 0, 32, 0, 110, 0, 111, 0, 109, 0, 98, 0, 114, 0, 101, 0, 32, 0,
        114, 0, 233, 0, 101, 0, 108, 0, 32, 0, 115, 0, 117, 0, 102, 0, 102, 0, 105, 0, 115, 0, 97,
        0, 109, 0, 109, 0, 101, 0, 110, 0, 116, 0, 32, 0, 108, 0, 111, 0, 110, 0, 103, 0, 32, 0,
        112, 0, 111, 0, 117, 0, 114, 0, 32, 0, 100, 0, 233, 0, 112, 0, 97, 0, 115, 0, 115, 0, 101,
        0, 114, 0, 32, 0, 113, 0, 117, 0, 97, 0, 116, 0, 111, 0, 114, 0, 122, 0, 101, 0, 32, 0,
        117, 0, 110, 0, 105, 0, 116, 0, 233, 0, 115, 0, 33, 81, 50, 0, 8, 0, 15, 0, 17, 0, 20, 0,
        26, 0, 124, 1, 61, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 1, 63,
    ];

    #[test]
    fn binary_strings_files_are_read() {
        let strings = read_strings(&SAMPLE).unwrap();
        assert_eq!(strings.len(), 3);
        assert_eq!(strings["count"], "2");
        assert!(strings["4"].starts_with("!first,second,...!!Sample text"));
        assert!(strings["1a"].contains("Texte accentué : é à ç"));
    }

    #[test]
    fn damaged_files_are_refused_without_panicking() {
        assert_eq!(read_strings(b""), None);
        assert_eq!(read_strings(b"bplist00"), None);
        assert_eq!(read_strings(b"\"4\" = \"text\";"), None);
        // Every truncation and every single corrupted byte.
        for len in 0..SAMPLE.len() {
            let _ = read_strings(&SAMPLE[..len]);
        }
        for at in 0..SAMPLE.len() {
            let mut bytes = SAMPLE;
            bytes[at] = bytes[at].wrapping_add(0x5B);
            let _ = read_strings(&bytes);
        }
    }

    #[test]
    fn entries_give_names_and_descriptions() {
        let strings = read_strings(&SAMPLE).unwrap();
        let help = parse_entry(&strings["4"]).unwrap();
        assert_eq!(help.args, ["first", "second"]);
        assert!(help.repeats);
        assert_eq!(help.description, "Sample text of a function");
        assert_eq!(
            help.arg_descriptions,
            ["what the first one is for", "what the others are for"]
        );

        // Two forms: the first one is kept, with its descriptions.
        let two =
            parse_entry("!a,b,c!a,d!!Looks things up!first!second!third!other first!other second!")
                .unwrap();
        assert_eq!(two.args, ["a", "b", "c"]);
        assert_eq!(two.arg_descriptions, ["first", "second", "third"]);
        assert!(!two.repeats);

        let none = parse_entry("!!Gives a random number!").unwrap();
        assert!(none.args.is_empty() && none.arg_descriptions.is_empty());
        assert_eq!(none.description, "Gives a random number");

        // Nothing said: macro functions, and what is not an entry.
        assert_eq!(parse_entry("!type_num,match_text!!"), None);
        assert_eq!(parse_entry("!!"), None);
        assert_eq!(parse_entry("SUM"), None);
        assert_eq!(parse_entry(""), None);
    }

    fn named(args: &[&str], repeats: bool) -> FunctionHelp {
        FunctionHelp {
            args: args.iter().map(|arg| arg.to_string()).collect(),
            repeats,
            description: "x".to_string(),
            arg_descriptions: args.iter().map(|arg| format!("about {arg}")).collect(),
        }
    }

    fn names(def: &FunctionDef, help: Option<&FunctionHelp>, count: usize) -> Vec<String> {
        arguments(def, help, count)
            .into_iter()
            .map(|arg| arg.name)
            .collect()
    }

    #[test]
    fn arguments_are_named_and_repeat() {
        let sum = functions::lookup("SUM").unwrap();
        let numbers = named(&["number1", "number2"], true);
        assert_eq!(
            names(sum, Some(&numbers), 4),
            ["number1", "number2", "number3", "number4"]
        );
        let args = arguments(sum, Some(&numbers), 3);
        assert!(!args[0].optional && args[1].optional && args[2].optional);
        assert_eq!(args[2].description, "about number2");
        assert_eq!(
            syntax(sum, Some(&numbers), Locale::en()),
            "SUM(number1,number2,...)"
        );
        assert_eq!(
            syntax(sum, Some(&numbers), Locale::fr()),
            "SOMME(number1;number2;...)"
        );

        // Pairs repeat together.
        let sumifs = functions::lookup("SUMIFS").unwrap();
        let pairs = named(&["sum_range", "criteria_range", "criteria"], true);
        assert_eq!(
            names(sumifs, Some(&pairs), 7),
            [
                "sum_range",
                "criteria_range",
                "criteria",
                "criteria_range2",
                "criteria2",
                "criteria_range3",
                "criteria3"
            ]
        );

        // Without Excel's names: numbered arguments, optional after the
        // ones the function needs.
        let vlookup = functions::lookup("VLOOKUP").unwrap();
        let plain = arguments(vlookup, None, 4);
        assert_eq!(plain[0].name, "argument1");
        assert!(!plain[2].optional && plain[3].optional);
        assert_eq!(
            syntax(vlookup, None, Locale::en()),
            "VLOOKUP(argument1,argument2,argument3,argument4)"
        );
        assert_eq!(syntax(sum, None, Locale::en()), "SUM(argument1,...)");
        assert_eq!(
            syntax(functions::lookup("RAND").unwrap(), None, Locale::en()),
            "RAND()"
        );
    }

    #[test]
    fn argument_boxes_grow_with_what_is_typed() {
        let sum = functions::lookup("SUM").unwrap();
        let numbers = named(&["number1", "number2"], true);
        assert_eq!(shown_arguments(sum, Some(&numbers), 0), 2);
        assert_eq!(shown_arguments(sum, Some(&numbers), 2), 3);
        assert_eq!(shown_arguments(sum, Some(&numbers), 5), 6);
        assert_eq!(shown_arguments(sum, None, 0), 1);
        let vlookup = functions::lookup("VLOOKUP").unwrap();
        assert_eq!(shown_arguments(vlookup, None, 0), 4);
        assert_eq!(shown_arguments(vlookup, None, 4), 4);
        assert_eq!(
            shown_arguments(functions::lookup("RAND").unwrap(), None, 0),
            0
        );
    }

    /// With Excel on the machine, its texts are found for the engine's
    /// functions in the five languages. Nothing to check without it.
    #[test]
    fn excel_resources_are_understood_when_present() {
        let dir = resources();
        if !dir.join("en.lproj/strman_0408.strings").exists() {
            return;
        }
        for lang in Lang::ALL {
            let table = load(&dir, lang);
            assert!(
                table.len() > 250,
                "{}: {} functions",
                lang.code(),
                table.len()
            );
            let sum = &table["SUM"];
            assert_eq!(sum.args.len(), 2, "{}", lang.code());
            assert!(sum.repeats && !sum.description.is_empty());
            let vlookup = &table["VLOOKUP"];
            assert_eq!(
                (vlookup.args.len(), vlookup.arg_descriptions.len()),
                (4, 4),
                "{}",
                lang.code()
            );
            assert!(table["RAND"].args.is_empty());
        }
    }
}
