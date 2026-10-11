//! File input / output for OpenSheet: `.xlsx` workbooks and delimited text
//! (`.csv`, `.tsv`, `.txt`), to and from the engine's [`Workbook`] model.
//!
//! Files hold formulas and number formats in canonical (English / US)
//! syntax; the locale passed to the loaders only decides how the workbook
//! displays and parses what the user types afterwards (and, for text files,
//! how numbers and dates in the file are read).
//!
//! Loaders return a workbook that is recalculated, unmodified and has no
//! undo history.

mod csv;
mod xlsx;

pub use csv::{
    decode_text, default_delimiter, detect_delimiter, load_csv, parse_delimited, save_csv,
    save_delimited, sheet_to_text, workbook_from_text,
};

use opensheet_engine::{Locale, Workbook};
use std::fmt;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

/// Why a file could not be read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum IoError {
    /// The file system refused.
    Io(std::io::Error),
    /// The file is not a readable zip archive (every `.xlsx` is one).
    Zip(String),
    /// The archive is not a workbook, or it is damaged.
    Invalid(String),
    /// A file type this crate does not handle.
    Unsupported(String),
    /// A sheet index outside the workbook.
    NoSuchSheet(usize),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::Io(e) => write!(f, "{e}"),
            IoError::Zip(msg) => write!(f, "not a valid .xlsx file: {msg}"),
            IoError::Invalid(msg) => write!(f, "invalid workbook: {msg}"),
            IoError::Unsupported(msg) => write!(f, "unsupported file: {msg}"),
            IoError::NoSuchSheet(index) => {
                write!(f, "the workbook has no sheet number {}", index + 1)
            }
        }
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IoError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> IoError {
        IoError::Io(e)
    }
}

impl From<zip::result::ZipError> for IoError {
    fn from(e: zip::result::ZipError) -> IoError {
        match e {
            zip::result::ZipError::Io(e) => IoError::Io(e),
            other => IoError::Zip(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, IoError>;

/// Lower-cased file extension ("" when there is none).
pub(crate) fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub(crate) fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|pair| {
        if little_endian {
            u16::from_le_bytes(*pair)
        } else {
            u16::from_be_bytes(*pair)
        }
    });
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Replace the file at `path` with `bytes`. The data goes to a temporary
/// sibling first and is moved into place, so a failed save does not leave a
/// truncated document behind. A read-only document is not replaced.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    let existing = std::fs::metadata(path).ok();
    if existing
        .as_ref()
        .is_some_and(|meta| meta.permissions().readonly())
    {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into());
    }
    let staged = || -> std::io::Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| std::io::Error::other("no file name"))?;
        let unique = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(name);
        tmp_name.push(format!(".{}-{unique}.tmp", std::process::id()));
        let tmp = path.with_file_name(tmp_name);
        let result = std::fs::File::create(&tmp)
            .and_then(|mut file| {
                file.write_all(bytes)?;
                file.sync_all()
            })
            .and_then(|()| {
                // The replacement keeps the access rights of the original.
                if let Some(meta) = &existing {
                    let _ = std::fs::set_permissions(&tmp, meta.permissions());
                }
                std::fs::rename(&tmp, path)
            });
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    };
    if staged().is_ok() {
        return Ok(());
    }
    // The directory may not allow new files while the document itself is
    // writable; write in place and report whatever that gives.
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Read an `.xlsx` workbook from any seekable source.
pub fn read_xlsx<R: Read + Seek>(reader: R, locale: &'static Locale) -> Result<Workbook> {
    xlsx::read::read_workbook(reader, locale)
}

/// Write a workbook as `.xlsx` to any seekable sink, which is handed back.
pub fn write_xlsx<W: Write + Seek>(wb: &Workbook, writer: W) -> Result<W> {
    xlsx::write::write_workbook(wb, writer, false)
}

/// Load an `.xlsx` (or `.xlsm`) file.
///
/// Formulas the engine cannot parse are replaced by their last calculated
/// value. Content the engine has no model for is dropped.
pub fn load_xlsx(path: &Path, locale: &'static Locale) -> Result<Workbook> {
    let file = std::fs::File::open(path)?;
    read_xlsx(std::io::BufReader::new(file), locale)
}

/// Save a workbook as `.xlsx`. Values calculated by the last `recalc` are
/// stored next to the formulas.
pub fn save_xlsx(wb: &Workbook, path: &Path) -> Result<()> {
    // The declared content type has to match the extension for Excel to
    // open the file.
    let macro_enabled = extension(path) == "xlsm";
    let bytes =
        xlsx::write::write_workbook(wb, Cursor::new(Vec::new()), macro_enabled)?.into_inner();
    write_file(path, &bytes)
}

/// Load a file, choosing the format from its extension: `.xlsx` / `.xlsm`
/// (also templates), `.csv`, `.tsv`, `.txt`. Other files are accepted when
/// their content is a zip archive, i.e. a workbook under another name.
pub fn load(path: &Path, locale: &'static Locale) -> Result<Workbook> {
    match extension(path).as_str() {
        "xlsx" | "xlsm" | "xltx" | "xltm" => load_xlsx(path, locale),
        "csv" | "tsv" | "tab" | "txt" => load_csv(path, locale),
        other => {
            let mut magic = [0u8; 4];
            let is_zip = std::fs::File::open(path)?.read_exact(&mut magic).is_ok()
                && magic == *b"PK\x03\x04";
            if is_zip {
                load_xlsx(path, locale)
            } else {
                Err(IoError::Unsupported(format!(
                    "cannot open \".{other}\" files"
                )))
            }
        }
    }
}

/// Save a workbook, choosing the format from the extension. `.xlsx` /
/// `.xlsm` hold the whole workbook; `.csv` (locale delimiter) and `.tsv` /
/// `.txt` (tab-delimited) hold the displayed values of sheet `sheet` only.
pub fn save(wb: &Workbook, sheet: usize, path: &Path) -> Result<()> {
    match extension(path).as_str() {
        "xlsx" | "xlsm" => save_xlsx(wb, path),
        "csv" => save_csv(wb, sheet, path),
        "tsv" | "tab" | "txt" => save_delimited(wb, sheet, path, '\t'),
        other => Err(IoError::Unsupported(format!("cannot save as \".{other}\""))),
    }
}
