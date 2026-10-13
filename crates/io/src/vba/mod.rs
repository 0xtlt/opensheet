//! Reading the VBA project of a macro-enabled workbook (MS-OVBA): the
//! list of its modules, their source and the macros they hold.
//!
//! The project (`xl/vbaProject.bin`) is a compound file. Its `PROJECT`
//! stream is a text listing the modules by kind; `VBA/dir`, compressed,
//! gives for each module the stream holding it and where in that stream
//! its compressed source starts (compiled code comes before).
//!
//! OpenSheet shows this source and never changes a project: the bytes
//! read from a file are the bytes written back. [`build_project`] makes a
//! project from source text; it exists for the tests and the sample
//! files, which need a project and have none written by Excel.

mod cfb;
mod compress;

use cfb::{Compound, Node};

/// What kind of module this is, as the Visual Basic editor groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleKind {
    /// The module of the workbook or of a sheet ("Microsoft Excel
    /// Objects").
    Document,
    /// A standard module ("Modules").
    Standard,
    /// A class module.
    Class,
    /// The code behind a user form.
    Form,
}

/// One module of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub kind: ModuleKind,
    /// The source as stored, with the `Attribute` lines the editor hides.
    pub source: String,
}

impl Module {
    /// The source as the Visual Basic editor shows it: without the header
    /// of a class and without the `Attribute` lines that open a module.
    pub fn code(&self) -> String {
        let mut lines = self.source.lines().peekable();
        // VERSION 1.0 CLASS / BEGIN … END, the header of a class or form.
        if lines.peek().is_some_and(|line| line.trim_start().to_ascii_uppercase().starts_with("VERSION ")) {
            lines.next();
            if lines.peek().is_some_and(|line| line.trim_start().to_ascii_uppercase().starts_with("BEGIN")) {
                for line in lines.by_ref() {
                    if line.trim().eq_ignore_ascii_case("END") {
                        break;
                    }
                }
            }
        }
        while lines.peek().is_some_and(|line| line.trim_start().to_ascii_lowercase().starts_with("attribute vb_")) {
            lines.next();
        }
        let mut code = String::new();
        for line in lines {
            code.push_str(line);
            code.push('\n');
        }
        code
    }

    /// The macros of the module: its public `Sub`s without arguments,
    /// which is what Excel lists in its Macro dialog. A class or a form
    /// has none, nor has a module marked `Option Private Module`.
    pub fn macros(&self) -> Vec<String> {
        if matches!(self.kind, ModuleKind::Class | ModuleKind::Form) {
            return Vec::new();
        }
        let mut found = Vec::new();
        for line in logical_lines(&self.code()) {
            let words: Vec<&str> = line.split_whitespace().collect();
            let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
            if lower.len() >= 3 && lower[..3] == ["option", "private", "module"] {
                return Vec::new();
            }
            let mut at = 0;
            if lower.first().is_some_and(|w| w == "public") {
                at += 1;
            }
            if lower.get(at).is_some_and(|w| w == "static") {
                at += 1;
            }
            if lower.get(at).is_none_or(|w| w != "sub") {
                continue;
            }
            // "Name()" or "Name ( )" or just "Name": no argument.
            let rest = words[at + 1..].join(" ");
            let (name, arguments) = match rest.split_once('(') {
                Some((name, after)) => (name.trim(), after.split_once(')').map_or("?", |(inside, _)| inside.trim())),
                None => (rest.split_whitespace().next().unwrap_or(""), ""),
            };
            let named = name.chars().next().is_some_and(char::is_alphabetic) && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            if named && arguments.is_empty() {
                found.push(name.to_string());
            }
        }
        found
    }
}

/// The statements of a source: lines without their comment, those ending
/// in ` _` joined to the next.
fn logical_lines(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = String::new();
    for raw in code.lines() {
        // A quote starts a comment unless it is inside a string.
        let mut in_string = false;
        let mut end = raw.len();
        for (at, ch) in raw.char_indices() {
            match ch {
                '"' => in_string = !in_string,
                '\'' if !in_string => {
                    end = at;
                    break;
                }
                _ => {}
            }
        }
        let line = raw[..end].trim_end();
        match line.strip_suffix(" _") {
            Some(start) => {
                pending.push_str(start);
                pending.push(' ');
            }
            None => {
                pending.push_str(line);
                out.push(std::mem::take(&mut pending));
            }
        }
    }
    if !pending.is_empty() {
        out.push(pending);
    }
    out
}

/// A macro as Excel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Macro {
    /// The name in the list: `Macro1`, or `Sheet1.Macro1` for a macro of
    /// the module of a sheet or of the workbook.
    pub name: String,
    /// The module it is in (an index into [`Project::modules`]).
    pub module: usize,
}

/// A VBA project, read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    /// The modules, in the order of the project.
    pub modules: Vec<Module>,
}

impl Project {
    /// The macros of the project, in the alphabetical order of the Macro
    /// dialog.
    pub fn macros(&self) -> Vec<Macro> {
        let mut found = Vec::new();
        for (index, module) in self.modules.iter().enumerate() {
            for name in module.macros() {
                let name = if module.kind == ModuleKind::Document { format!("{}.{name}", module.name) } else { name };
                found.push(Macro { name, module: index });
            }
        }
        found.sort_by_key(|m| m.name.to_lowercase());
        found
    }
}

// ----------------------------------------------------------------- reading

/// Bytes in a Windows or Mac code page → text. Western European pages
/// are known; text in another page keeps its ASCII and shows the rest as
/// Windows-1252 would.
fn decode(bytes: &[u8], code_page: u16) -> String {
    const CP1252_HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”',
        '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    const MAC_ROMAN_HIGH: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{A0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{F8FF}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";
    match code_page {
        65001 => String::from_utf8_lossy(bytes).into_owned(),
        10000 => {
            let high: Vec<char> = MAC_ROMAN_HIGH.chars().collect();
            bytes.iter().map(|b| if *b < 0x80 { *b as char } else { high[(*b - 0x80) as usize] }).collect()
        }
        _ => bytes
            .iter()
            .map(|b| match *b {
                0x80..=0x9F => CP1252_HIGH[(*b - 0x80) as usize],
                other => other as char,
            })
            .collect(),
    }
}

/// Text → bytes of Windows-1252; what the page lacks becomes `?`.
fn encode_1252(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| match ch as u32 {
            code @ (0..=0x7F | 0xA0..=0xFF) => code as u8,
            _ => (0x80u8..=0x9F).find(|b| decode(&[*b], 1252).starts_with(ch)).unwrap_or(b'?'),
        })
        .collect()
}

struct DirModule {
    name: String,
    stream: String,
    offset: u32,
    procedural: bool,
}

/// Read a VBA project (the bytes of `xl/vbaProject.bin`).
pub fn read_project(bytes: &[u8]) -> Result<Project, String> {
    let file = Compound::parse(bytes)?;
    let dir = file.stream(&["VBA", "dir"]).ok_or("the project has no VBA/dir stream")?;
    let dir = compress::decompress(&dir).ok_or("the VBA/dir stream is damaged")?;

    let mut code_page = 1252u16;
    let mut project = Project::default();
    let mut modules: Vec<DirModule> = Vec::new();
    let mut at = 0;
    while at + 6 <= dir.len() {
        let id = u16::from_le_bytes([dir[at], dir[at + 1]]);
        let mut size = u32::from_le_bytes([dir[at + 2], dir[at + 3], dir[at + 4], dir[at + 5]]) as usize;
        if id == 0x0009 {
            // The version record says 4 and holds 6 bytes.
            size = 6;
        }
        let Some(data) = dir.get(at + 6..at + 6 + size) else {
            break;
        };
        at += 6 + size;
        let utf16 = || {
            let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        };
        match id {
            0x0003 if size >= 2 => code_page = u16::from_le_bytes([data[0], data[1]]),
            0x0004 => project.name = decode(data, code_page),
            // A module starts with its name.
            0x0019 => modules.push(DirModule {
                name: decode(data, code_page),
                stream: decode(data, code_page),
                offset: 0,
                procedural: true,
            }),
            0x0010 => break,
            _ => {
                let Some(module) = modules.last_mut() else {
                    continue;
                };
                match id {
                    0x0047 if size > 0 => module.name = utf16(),
                    0x001A => module.stream = decode(data, code_page),
                    0x0032 if size > 0 => module.stream = utf16(),
                    0x0031 if size >= 4 => module.offset = u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
                    0x0022 => module.procedural = false,
                    _ => {}
                }
            }
        }
    }

    // The PROJECT stream tells documents, classes and forms apart.
    let listing = file.stream(&["PROJECT"]).map(|bytes| decode(&bytes, code_page)).unwrap_or_default();
    let mut kinds: Vec<(String, ModuleKind)> = Vec::new();
    for line in listing.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let kind = match key.trim() {
            "Document" => ModuleKind::Document,
            "Module" => ModuleKind::Standard,
            "Class" => ModuleKind::Class,
            "BaseClass" => ModuleKind::Form,
            _ => continue,
        };
        // "ThisWorkbook/&H00000000"
        let name = value.split('/').next().unwrap_or(value).trim();
        kinds.push((name.to_string(), kind));
    }

    for module in modules {
        let stream = file.stream(&["VBA", &module.stream]).ok_or_else(|| format!("the module {} has no stream", module.name))?;
        let packed = stream.get(module.offset as usize..).ok_or_else(|| format!("the module {} is damaged", module.name))?;
        let source = compress::decompress(packed).ok_or_else(|| format!("the source of the module {} is damaged", module.name))?;
        let kind = kinds
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&module.name))
            .map(|(_, kind)| *kind)
            .unwrap_or(if module.procedural { ModuleKind::Standard } else { ModuleKind::Class });
        project.modules.push(Module {
            name: module.name,
            kind,
            source: decode(&source, code_page).replace("\r\n", "\n"),
        });
    }
    Ok(project)
}

// ----------------------------------------------------------------- writing

/// The `Attribute` lines Excel puts at the head of a module.
fn header(name: &str, kind: ModuleKind) -> String {
    let mut text = match kind {
        ModuleKind::Class | ModuleKind::Form => "VERSION 1.0 CLASS\r\nBEGIN\r\n  MultiUse = -1  'True\r\nEND\r\n".to_string(),
        _ => String::new(),
    };
    text.push_str(&format!("Attribute VB_Name = \"{name}\"\r\n"));
    let (base, predeclared, exposed, customizable) = match kind {
        ModuleKind::Standard => return text,
        ModuleKind::Document if name == "ThisWorkbook" => ("0{00020819-0000-0000-C000-000000000046}", true, true, true),
        ModuleKind::Document => ("0{00020820-0000-0000-C000-000000000046}", true, true, true),
        ModuleKind::Class | ModuleKind::Form => ("0{FCFB3D2A-A0FA-1068-A738-08002B3371B5}", false, false, false),
    };
    let flag = |on: bool| if on { "True" } else { "False" };
    text.push_str(&format!("Attribute VB_Base = \"{base}\"\r\n"));
    text.push_str("Attribute VB_GlobalNameSpace = False\r\nAttribute VB_Creatable = False\r\n");
    text.push_str(&format!("Attribute VB_PredeclaredId = {}\r\n", flag(predeclared)));
    text.push_str(&format!("Attribute VB_Exposed = {}\r\n", flag(exposed)));
    text.push_str("Attribute VB_TemplateDerived = False\r\n");
    text.push_str(&format!("Attribute VB_Customizable = {}\r\n", flag(customizable)));
    text
}

/// One record of the `dir` stream.
fn record(out: &mut Vec<u8>, id: u16, data: &[u8]) {
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// A value of the `PROJECT` stream that Office keeps scrambled (MS-OVBA
/// 2.4.3): the protection state, the password and the visibility of the
/// project. `data` is the plain value; the result is its hexadecimal text.
fn scramble(project_id: &str, data: &[u8]) -> String {
    const SEED: u8 = 0x5A;
    const VERSION: u8 = 2;
    let key = project_id.bytes().fold(0u8, u8::wrapping_add);
    let mut out = vec![SEED, SEED ^ VERSION, SEED ^ key];
    let (mut plain_before, mut enc_before, mut enc_before_that) = (key, SEED ^ key, SEED ^ VERSION);
    let mut push = |byte: u8, out: &mut Vec<u8>| {
        let enc = byte ^ enc_before_that.wrapping_add(plain_before);
        out.push(enc);
        enc_before_that = enc_before;
        enc_before = enc;
        plain_before = byte;
    };
    // Bytes that mean nothing, as many as the seed says.
    for _ in 0..(SEED & 6) / 2 {
        push(0x07, &mut out);
    }
    for byte in (data.len() as u32).to_le_bytes() {
        push(byte, &mut out);
    }
    for byte in data {
        push(*byte, &mut out);
    }
    out.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// A library a project refers to: its name and its registered identifier.
const REFERENCES: [(&str, &str); 4] = [
    ("VBA", "*\\G{000204EF-0000-0000-C000-000000000046}#4.2#9#C:\\Program Files\\Common Files\\Microsoft Shared\\VBA\\VBA7.1\\VBE7.DLL#Visual Basic For Applications"),
    ("Excel", "*\\G{00020813-0000-0000-C000-000000000046}#1.9#0#C:\\Program Files\\Microsoft Office\\root\\Office16\\EXCEL.EXE#Microsoft Excel 16.0 Object Library"),
    ("stdole", "*\\G{00020430-0000-0000-C000-000000000046}#2.0#0#C:\\Windows\\System32\\stdole2.tlb#OLE Automation"),
    ("Office", "*\\G{2DF8D04C-5BFA-101B-BDE5-00AA0044DE52}#2.8#0#C:\\Program Files\\Common Files\\Microsoft Shared\\OFFICE16\\MSO.DLL#Microsoft Office 16.0 Object Library"),
];

/// Make a VBA project from source text: for each module its name, its
/// kind and its code (without `Attribute` lines). The project holds no
/// compiled code; an application that runs VBA compiles the source when
/// it opens the project.
pub fn build_project(modules: &[(&str, ModuleKind, &str)]) -> Vec<u8> {
    const PROJECT_ID: &str = "{8A5B3E21-6C4D-4F7A-9B1E-2D3C4F5A6B7C}";
    let mut dir = Vec::new();
    // Information about the project.
    record(&mut dir, 0x0001, &3u32.to_le_bytes());
    record(&mut dir, 0x0002, &0x0409u32.to_le_bytes());
    record(&mut dir, 0x0014, &0x0409u32.to_le_bytes());
    record(&mut dir, 0x0003, &1252u16.to_le_bytes());
    record(&mut dir, 0x0004, b"VBAProject");
    record(&mut dir, 0x0005, b"");
    record(&mut dir, 0x0040, b"");
    record(&mut dir, 0x0006, b"");
    record(&mut dir, 0x003D, b"");
    record(&mut dir, 0x0007, &0u32.to_le_bytes());
    record(&mut dir, 0x0008, &0u32.to_le_bytes());
    // The version: its size field says 4, six bytes follow.
    dir.extend_from_slice(&0x0009u16.to_le_bytes());
    dir.extend_from_slice(&4u32.to_le_bytes());
    dir.extend_from_slice(&1u32.to_le_bytes());
    dir.extend_from_slice(&0u16.to_le_bytes());
    record(&mut dir, 0x000C, b"");
    record(&mut dir, 0x003C, b"");
    // The libraries.
    for (name, libid) in REFERENCES {
        record(&mut dir, 0x0016, name.as_bytes());
        record(&mut dir, 0x003E, &utf16(name));
        let mut registered = (libid.len() as u32).to_le_bytes().to_vec();
        registered.extend_from_slice(libid.as_bytes());
        registered.extend_from_slice(&[0; 6]);
        record(&mut dir, 0x000D, &registered);
    }
    // The modules.
    record(&mut dir, 0x000F, &(modules.len() as u16).to_le_bytes());
    record(&mut dir, 0x0013, &0xFFFFu16.to_le_bytes());
    for (name, kind, _) in modules {
        let ansi = encode_1252(name);
        record(&mut dir, 0x0019, &ansi);
        record(&mut dir, 0x0047, &utf16(name));
        record(&mut dir, 0x001A, &ansi);
        record(&mut dir, 0x0032, &utf16(name));
        record(&mut dir, 0x001C, b"");
        record(&mut dir, 0x0048, b"");
        record(&mut dir, 0x0031, &0u32.to_le_bytes());
        record(&mut dir, 0x001E, &0u32.to_le_bytes());
        record(&mut dir, 0x002C, &0xFFFFu16.to_le_bytes());
        record(&mut dir, if *kind == ModuleKind::Standard { 0x0021 } else { 0x0022 }, b"");
        record(&mut dir, 0x002B, b"");
    }
    record(&mut dir, 0x0010, b"");

    let mut listing = format!("ID=\"{PROJECT_ID}\"\r\n");
    let mut names = Vec::new();
    let mut vba = vec![
        // No compiled code: the version that makes the reader compile.
        Node::Stream("_VBA_PROJECT".to_string(), vec![0xCC, 0x61, 0xFF, 0xFF, 0x00, 0x00, 0x00]),
        Node::Stream("dir".to_string(), compress::compress(&dir)),
    ];
    for (name, kind, code) in modules {
        listing.push_str(&match kind {
            ModuleKind::Document => format!("Document={name}/&H00000000\r\n"),
            ModuleKind::Standard => format!("Module={name}\r\n"),
            ModuleKind::Class => format!("Class={name}\r\n"),
            ModuleKind::Form => format!("BaseClass={name}\r\n"),
        });
        names.extend_from_slice(&encode_1252(name));
        names.push(0);
        names.extend_from_slice(&utf16(name));
        names.extend_from_slice(&[0, 0]);
        let source = format!("{}{}", header(name, *kind), code.replace("\r\n", "\n").replace('\n', "\r\n"));
        vba.push(Node::Stream(name.to_string(), compress::compress(&encode_1252(&source))));
    }
    names.extend_from_slice(&[0, 0]);
    listing.push_str("Name=\"VBAProject\"\r\nHelpContextID=\"0\"\r\nVersionCompatible32=\"393222000\"\r\n");
    listing.push_str(&format!("CMG=\"{}\"\r\n", scramble(PROJECT_ID, &[0, 0, 0, 0])));
    listing.push_str(&format!("DPB=\"{}\"\r\n", scramble(PROJECT_ID, &[0])));
    listing.push_str(&format!("GC=\"{}\"\r\n", scramble(PROJECT_ID, &[0xFF])));
    listing.push_str("\r\n[Host Extender Info]\r\n&H00000001={3832D640-CF90-11CF-8E43-00A0C911005A};VBE;&H00000000\r\n");
    listing.push_str("\r\n[Workspace]\r\n");
    for (name, _, _) in modules {
        listing.push_str(&format!("{name}=0, 0, 0, 0, C\r\n"));
    }
    cfb::write(vec![
        Node::Stream("PROJECT".to_string(), encode_1252(&listing)),
        Node::Stream("PROJECTwm".to_string(), names),
        Node::Storage("VBA".to_string(), vba),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: &str = "Option Explicit\n\nSub Macro1()\n    Range(\"A1\").Value = \"Café\"\nEnd Sub\n\nPublic Sub Total_2 ( )\nEnd Sub\n\nPrivate Sub Hidden()\nEnd Sub\n\nSub WithArgument(ByVal n As Long)\nEnd Sub\n\nFunction Twice(x)\n    Twice = 2 * x\nEnd Function\n";

    #[test]
    fn a_project_is_read_back() {
        let bytes = build_project(&[
            ("ThisWorkbook", ModuleKind::Document, "Private Sub Workbook_Open()\nEnd Sub\n"),
            ("Sheet1", ModuleKind::Document, "Sub Refresh()\n    Me.Calculate\nEnd Sub\n"),
            ("Module1", ModuleKind::Standard, MODULE),
            ("Class1", ModuleKind::Class, "Public Name As String\n\nPublic Sub Reset()\nEnd Sub\n"),
        ]);
        let project = read_project(&bytes).expect("a project");
        assert_eq!(project.name, "VBAProject");
        let listed: Vec<(&str, ModuleKind)> = project.modules.iter().map(|m| (m.name.as_str(), m.kind)).collect();
        assert_eq!(
            listed,
            [
                ("ThisWorkbook", ModuleKind::Document),
                ("Sheet1", ModuleKind::Document),
                ("Module1", ModuleKind::Standard),
                ("Class1", ModuleKind::Class)
            ]
        );
        // The code is what was given; the stored source has the hidden
        // lines before it.
        assert_eq!(project.modules[2].code(), MODULE);
        assert!(project.modules[2].source.starts_with("Attribute VB_Name = \"Module1\"\n"));
        assert_eq!(project.modules[3].code(), "Public Name As String\n\nPublic Sub Reset()\nEnd Sub\n");
        assert!(project.modules[3].source.starts_with("VERSION 1.0 CLASS\n"));
        let macros: Vec<(String, usize)> = project.macros().into_iter().map(|m| (m.name, m.module)).collect();
        assert_eq!(macros, [("Macro1".to_string(), 2), ("Sheet1.Refresh".to_string(), 1), ("Total_2".to_string(), 2)]);
    }

    #[test]
    fn macros_are_the_public_subs_without_arguments() {
        let module = |code: &str| Module {
            name: "M".to_string(),
            kind: ModuleKind::Standard,
            source: code.to_string(),
        };
        let code = "Sub A\nEnd Sub\nsub b()\nEnd Sub\nPublic Static Sub C()\nEnd Sub\nFriend Sub D()\nEnd Sub\n' Sub E()\nSub F(Optional x)\nEnd Sub\nSub G( _\n)\nEnd Sub\nx = \"Sub H()\"\nSub I() ' Sub J()\nEnd Sub\n";
        assert_eq!(module(code).macros(), ["A", "b", "C", "G", "I"]);
        // A private module shows none of its macros.
        assert!(module("Option Private Module\nSub A()\nEnd Sub\n").macros().is_empty());
        let class = Module {
            kind: ModuleKind::Class,
            ..module("Sub A()\nEnd Sub\n")
        };
        assert!(class.macros().is_empty());
    }

    #[test]
    fn text_is_decoded_in_the_page_of_the_project() {
        assert_eq!(decode(&[0x43, 0x61, 0x66, 0xE9, 0x80, 0x99], 1252), "Café€™");
        assert_eq!(decode(&[0x43, 0x61, 0x66, 0x8E, 0xDB], 10000), "Café€");
        assert_eq!(decode("né".as_bytes(), 65001), "né");
        assert_eq!(encode_1252("Café€™ ✓"), [0x43, 0x61, 0x66, 0xE9, 0x80, 0x99, 0x20, b'?']);
    }

    #[test]
    fn what_is_not_a_project_is_refused() {
        assert!(read_project(b"").is_err());
        assert!(read_project(b"not a project at all").is_err());
        // A compound file without a VBA storage.
        let other = cfb::write(vec![Node::Stream("Workbook".to_string(), vec![1, 2, 3])]);
        assert!(read_project(&other).is_err());
        // Damage anywhere: an error or a project, never a panic.
        let good = build_project(&[("Module1", ModuleKind::Standard, MODULE)]);
        for at in (0..good.len()).step_by(3) {
            let mut bad = good.clone();
            bad[at] = bad[at].wrapping_add(1);
            let _ = read_project(&bad);
        }
    }

    /// The Solver add-in that comes with Excel holds a project Excel
    /// itself wrote. It is read where it is, when it is there.
    #[test]
    fn a_project_written_by_excel_is_read() {
        let path = "/Applications/Microsoft Excel.app/Contents/Resources/en.lproj/Add-Ins/Solver.xlam";
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        let mut zip = zip::ZipArchive::new(file).expect("a package");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name("xl/vbaProject.bin").expect("a project"), &mut bytes).unwrap();
        let project = read_project(&bytes).expect("Excel's own project");
        assert!(project.modules.len() > 3, "{:?}", project.modules.iter().map(|m| &m.name).collect::<Vec<_>>());
        assert!(project.modules.iter().any(|m| m.name == "ThisWorkbook" && m.kind == ModuleKind::Document));
        for module in &project.modules {
            assert!(module.source.contains("Attribute VB_Name"), "{}", module.name);
        }
        assert!(project.modules.iter().any(|m| m.kind == ModuleKind::Standard && m.code().contains("Sub ")));
    }
}
