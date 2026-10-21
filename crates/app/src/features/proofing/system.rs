//! What the proofing tools ask of macOS, each behind a small trait so the
//! interface tests run with a stand-in:
//!
//! * spelling: `NSSpellChecker` (AppKit), the checker of every Mac
//!   application, with the words the user taught it;
//! * synonyms: the thesaurus dictionaries of the Dictionary application
//!   that are installed on this Mac, read through DictionaryServices;
//! * translation: Apple's Translation framework, which only Swift can
//!   call: a small helper program (`helpers/translate.swift`, built by
//!   `scripts/build_translate_helper.sh`) is run for each request.
//!
//! Nothing here reaches a network service of a third party.

use crate::app::ExcelApp;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

// ----------------------------------------------------------------- spelling

/// A spelling checker. Offsets are bytes of the text given.
pub trait SpellChecker {
    /// The languages it can check, as it names them ("en", "en_GB", "fr").
    fn languages(&self) -> Vec<String>;

    /// The first misspelled word at or after `from`.
    fn next_misspelling(&self, text: &str, from: usize, language: &str) -> Option<Range<usize>>;

    /// What the word at `range` may have been meant to be, best first.
    fn guesses(&self, text: &str, range: Range<usize>, language: &str) -> Vec<String>;

    /// Pass over a word until the checker is dropped (Ignore All).
    fn ignore(&self, word: &str);

    /// Stop passing over a word.
    fn unignore(&self, word: &str);

    /// Add a word to the user's dictionary, for every application.
    fn learn(&self, word: &str);

    /// Take a word out of the user's dictionary again.
    fn unlearn(&self, word: &str);
}

// ----------------------------------------------------------------- synonyms

/// One meaning of a word in a thesaurus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sense {
    /// "noun", "verb", "adjective"... as the dictionary writes it.
    pub part_of_speech: String,
    /// A phrase showing the word in this meaning; may be empty.
    pub example: String,
    pub synonyms: Vec<String>,
    pub antonyms: Vec<String>,
}

/// What a thesaurus says of a word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Synonyms {
    /// The entry found (its headword: "run" for "ran") and its meanings.
    Found {
        headword: String,
        senses: Vec<Sense>,
    },
    /// The thesauruses installed do not have the word.
    NotFound,
    /// No thesaurus is installed on this Mac.
    NoThesaurus,
}

pub trait Thesaurus {
    /// The names of the thesaurus dictionaries that are installed.
    fn sources(&self) -> Vec<String>;

    fn lookup(&self, word: &str) -> Synonyms;
}

// -------------------------------------------------------------- translation

/// Why a text could not be translated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranslateError {
    /// The helper program is not next to the application.
    NoHelper,
    /// This version of macOS has no translation an application can call.
    UnsupportedSystem,
    /// The languages exist but were not downloaded in System Settings.
    NotInstalled,
    /// macOS does not translate between these two languages.
    UnsupportedPair,
    /// The language of the text could not be told.
    UnknownLanguage,
    /// Anything else, in the system's words.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translation {
    pub text: String,
    /// The language the text was translated from (the one detected when
    /// none was given).
    pub source: String,
}

/// A translator. Called away from the main thread: a translation takes
/// its time.
pub trait Translator: Send + Sync {
    /// The languages it knows, as BCP 47 codes ("en", "fr", "pt-BR").
    fn languages(&self) -> Result<Vec<String>, TranslateError>;

    /// Translate `text` into `to`, from `from` or from the language the
    /// text is found to be in.
    fn translate(
        &self,
        text: &str,
        from: Option<&str>,
        to: &str,
    ) -> Result<Translation, TranslateError>;
}

// ----------------------------------------------------------------- services

/// The services of this workbook's window; tests put stand-ins here.
#[derive(Default)]
pub struct Services {
    pub spelling: Option<Rc<dyn SpellChecker>>,
    pub thesaurus: Option<Rc<dyn Thesaurus>>,
    pub translator: Option<Arc<dyn Translator>>,
}

/// The checker of a spelling session: the words it is told to ignore are
/// forgotten with it.
pub fn spell_checker(app: &ExcelApp) -> Rc<dyn SpellChecker> {
    match app
        .ext::<Services>()
        .and_then(|services| services.spelling.clone())
    {
        Some(checker) => checker,
        None => default_spell_checker(),
    }
}

pub fn thesaurus(app: &ExcelApp) -> Rc<dyn Thesaurus> {
    match app
        .ext::<Services>()
        .and_then(|services| services.thesaurus.clone())
    {
        Some(thesaurus) => thesaurus,
        None => default_thesaurus(),
    }
}

pub fn translator(app: &ExcelApp) -> Arc<dyn Translator> {
    match app
        .ext::<Services>()
        .and_then(|services| services.translator.clone())
    {
        Some(translator) => translator,
        None => default_translator(),
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn default_spell_checker() -> Rc<dyn SpellChecker> {
    Rc::new(mac::SystemSpellChecker::new())
}

#[cfg(all(target_os = "macos", not(test)))]
fn default_thesaurus() -> Rc<dyn Thesaurus> {
    Rc::new(mac::SystemThesaurus)
}

#[cfg(not(test))]
fn default_translator() -> Arc<dyn Translator> {
    Arc::new(HelperTranslator::beside_the_application())
}

// The tests never call macOS unless they say so: a window opened by a
// test gets services that know nothing.
#[cfg(any(not(target_os = "macos"), test))]
fn default_spell_checker() -> Rc<dyn SpellChecker> {
    Rc::new(fake::Words::default())
}

#[cfg(any(not(target_os = "macos"), test))]
fn default_thesaurus() -> Rc<dyn Thesaurus> {
    Rc::new(fake::NoBooks)
}

#[cfg(test)]
fn default_translator() -> Arc<dyn Translator> {
    Arc::new(HelperTranslator { helper: None })
}

/// The name of a language in the language of the interface ("anglais
/// (Royaume-Uni)" for "en_GB").
pub fn language_name(code: &str) -> String {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        mac::language_name(code, crate::i18n::lang().code()).unwrap_or_else(|| code.to_string())
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        code.to_string()
    }
}

/// The language among `available` that is the interface's: the one the
/// proofing tools start with.
pub fn interface_language(available: &[String]) -> Option<usize> {
    let code = crate::i18n::lang().code();
    available
        .iter()
        .position(|language| language == code)
        .or_else(|| {
            available
                .iter()
                .position(|language| language.starts_with(code))
        })
}

// ------------------------------------------------------ translation helper

/// The translator that runs the Swift helper: one process per request,
/// the text on its input, the answer on its output.
pub struct HelperTranslator {
    helper: Option<std::path::PathBuf>,
}

/// The name of the helper program, next to the `opensheet` binary.
pub const HELPER_NAME: &str = "opensheet-translate";

impl HelperTranslator {
    /// The helper next to the running program (in the bundle or in
    /// `target/`), or the one `OPENSHEET_TRANSLATE_HELPER` names.
    #[cfg(not(test))]
    pub fn beside_the_application() -> HelperTranslator {
        let named = std::env::var_os("OPENSHEET_TRANSLATE_HELPER").map(std::path::PathBuf::from);
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join(HELPER_NAME)));
        HelperTranslator {
            helper: named.or(beside).filter(|path| path.is_file()),
        }
    }

    #[cfg(test)]
    pub fn at(path: std::path::PathBuf) -> HelperTranslator {
        HelperTranslator {
            helper: Some(path).filter(|path| path.is_file()),
        }
    }

    #[cfg(test)]
    pub fn available(&self) -> bool {
        self.helper.is_some()
    }

    /// Run the helper with `args`, `input` on its standard input; its
    /// standard output.
    fn run(&self, args: &[&str], input: &str) -> Result<String, TranslateError> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let helper = self.helper.as_ref().ok_or(TranslateError::NoHelper)?;
        let failed = |error: std::io::Error| TranslateError::Failed(error.to_string());
        let mut child = Command::new(helper)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(failed)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes()).map_err(failed)?;
        }
        let output = child.wait_with_output().map_err(failed)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Read an answer of the helper: a first line `ok <detail>` or
/// `error <code> <detail>`, then the text.
pub fn parse_helper_answer(answer: &str) -> Result<(String, String), TranslateError> {
    let (first, rest) = answer.split_once('\n').unwrap_or((answer, ""));
    let mut words = first.splitn(3, ' ');
    match (words.next(), words.next()) {
        (Some("ok"), detail) => Ok((
            detail.unwrap_or_default().to_string(),
            rest.strip_suffix('\n').unwrap_or(rest).to_string(),
        )),
        (Some("error"), Some(code)) => Err(match code {
            "unsupported-system" => TranslateError::UnsupportedSystem,
            "not-installed" => TranslateError::NotInstalled,
            "unsupported-pair" => TranslateError::UnsupportedPair,
            "unknown-language" => TranslateError::UnknownLanguage,
            _ => TranslateError::Failed(words.next().unwrap_or(code).to_string()),
        }),
        // The helper stopped without a word (killed, or not a helper).
        _ => Err(TranslateError::Failed(first.trim().to_string())),
    }
}

impl Translator for HelperTranslator {
    fn languages(&self) -> Result<Vec<String>, TranslateError> {
        let (_, list) = parse_helper_answer(&self.run(&["languages"], "")?)?;
        Ok(list
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn translate(
        &self,
        text: &str,
        from: Option<&str>,
        to: &str,
    ) -> Result<Translation, TranslateError> {
        let answer = self.run(&["translate", from.unwrap_or("auto"), to], text)?;
        let (source, text) = parse_helper_answer(&answer)?;
        Ok(Translation { text, source })
    }
}

// --------------------------------------------------------------------- text

/// Byte offset of a UTF-16 offset of `text` (clamped to its end).
pub fn byte_of_utf16(text: &str, utf16: usize) -> usize {
    let mut seen = 0;
    for (byte, ch) in text.char_indices() {
        if seen >= utf16 {
            return byte;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

/// UTF-16 offset of a byte offset of `text`.
pub fn utf16_of_byte(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].encode_utf16().count()
}

// -------------------------------------------------------------------- macOS

// (The `objc` macros test a configuration flag this crate does not know.)
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub mod mac {
    //! The calls into AppKit and DictionaryServices.

    use super::{SpellChecker, Synonyms, Thesaurus, byte_of_utf16, utf16_of_byte};
    use objc::runtime::{BOOL, NO, Object};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::{CStr, c_char, c_void};
    use std::ops::Range;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
        fn CFSetGetCount(set: *const c_void) -> isize;
        fn CFSetGetValues(set: *const c_void, values: *mut *const c_void);
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    }

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    type Id = *mut Object;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRange {
        location: usize,
        length: usize,
    }

    /// `NSNotFound`.
    const NOT_FOUND: usize = isize::MAX as usize;
    const UTF8: usize = 4;

    /// Objects made inside `f` and left to the pool are released when it
    /// returns.
    fn with_pool<R>(f: impl FnOnce() -> R) -> R {
        unsafe {
            let pool: Id = msg_send![class!(NSAutoreleasePool), new];
            let result = f();
            let _: () = msg_send![pool, drain];
            result
        }
    }

    /// An autoreleased `NSString`.
    unsafe fn ns_string(text: &str) -> Id {
        unsafe {
            let string: Id = msg_send![class!(NSString), alloc];
            let string: Id =
                msg_send![string, initWithBytes: text.as_ptr() length: text.len() encoding: UTF8];
            msg_send![string, autorelease]
        }
    }

    unsafe fn rust_string(string: Id) -> String {
        if string.is_null() {
            return String::new();
        }
        unsafe {
            let bytes: *const c_char = msg_send![string, UTF8String];
            if bytes.is_null() {
                String::new()
            } else {
                CStr::from_ptr(bytes).to_string_lossy().into_owned()
            }
        }
    }

    unsafe fn strings_of(array: Id) -> Vec<String> {
        if array.is_null() {
            return Vec::new();
        }
        unsafe {
            let count: usize = msg_send![array, count];
            (0..count)
                .map(|index| {
                    let item: Id = msg_send![array, objectAtIndex: index];
                    rust_string(item)
                })
                .collect()
        }
    }

    /// "anglais (Royaume-Uni)": a language named in another.
    pub fn language_name(code: &str, in_language: &str) -> Option<String> {
        with_pool(|| unsafe {
            let locale: Id =
                msg_send![class!(NSLocale), localeWithLocaleIdentifier: ns_string(in_language)];
            let name: Id = msg_send![locale, localizedStringForLocaleIdentifier: ns_string(code)];
            Some(rust_string(name)).filter(|name| !name.is_empty())
        })
    }

    /// The system's spelling checker. The words it is told to ignore
    /// belong to its "document", closed when it is dropped.
    pub struct SystemSpellChecker {
        tag: isize,
    }

    impl SystemSpellChecker {
        pub fn new() -> SystemSpellChecker {
            let tag = unsafe { msg_send![class!(NSSpellChecker), uniqueSpellDocumentTag] };
            SystemSpellChecker { tag }
        }

        fn shared() -> Id {
            unsafe { msg_send![class!(NSSpellChecker), sharedSpellChecker] }
        }
    }

    impl Drop for SystemSpellChecker {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![Self::shared(), closeSpellDocumentWithTag: self.tag];
            }
        }
    }

    impl SpellChecker for SystemSpellChecker {
        fn languages(&self) -> Vec<String> {
            with_pool(|| unsafe {
                let languages: Id = msg_send![Self::shared(), availableLanguages];
                strings_of(languages)
            })
        }

        fn next_misspelling(
            &self,
            text: &str,
            from: usize,
            language: &str,
        ) -> Option<Range<usize>> {
            with_pool(|| unsafe {
                let start = utf16_of_byte(text, from) as isize;
                let wrap: BOOL = NO;
                let range: NSRange = msg_send![Self::shared(),
                    checkSpellingOfString: ns_string(text)
                    startingAt: start
                    language: ns_string(language)
                    wrap: wrap
                    inSpellDocumentWithTag: self.tag
                    wordCount: std::ptr::null_mut::<isize>()];
                if range.location == NOT_FOUND || range.length == 0 {
                    return None;
                }
                let found = byte_of_utf16(text, range.location)
                    ..byte_of_utf16(text, range.location + range.length);
                // The checker is not asked to wrap; a word before `from`
                // would send the caller round in circles.
                (found.start >= from).then_some(found)
            })
        }

        fn guesses(&self, text: &str, range: Range<usize>, language: &str) -> Vec<String> {
            with_pool(|| unsafe {
                let start = utf16_of_byte(text, range.start);
                let word = NSRange {
                    location: start,
                    length: utf16_of_byte(text, range.end) - start,
                };
                let guesses: Id = msg_send![Self::shared(),
                    guessesForWordRange: word
                    inString: ns_string(text)
                    language: ns_string(language)
                    inSpellDocumentWithTag: self.tag];
                strings_of(guesses)
            })
        }

        fn ignore(&self, word: &str) {
            with_pool(|| unsafe {
                let _: () = msg_send![Self::shared(), ignoreWord: ns_string(word) inSpellDocumentWithTag: self.tag];
            })
        }

        fn unignore(&self, word: &str) {
            with_pool(|| unsafe {
                let ignored: Id =
                    msg_send![Self::shared(), ignoredWordsInSpellDocumentWithTag: self.tag];
                let kept: Id = msg_send![class!(NSMutableArray), array];
                for other in strings_of(ignored)
                    .into_iter()
                    .filter(|other| other != word)
                {
                    let _: () = msg_send![kept, addObject: ns_string(&other)];
                }
                let _: () = msg_send![Self::shared(), setIgnoredWords: kept inSpellDocumentWithTag: self.tag];
            })
        }

        fn learn(&self, word: &str) {
            with_pool(|| unsafe {
                let _: () = msg_send![Self::shared(), learnWord: ns_string(word)];
            })
        }

        fn unlearn(&self, word: &str) {
            with_pool(|| unsafe {
                let _: () = msg_send![Self::shared(), unlearnWord: ns_string(word)];
            })
        }
    }

    // ---- DictionaryServices
    //
    // The framework's public function only gives the definition of a word
    // as one string, from the dictionaries ticked in the Dictionary
    // application. Reading one dictionary's entry takes the functions the
    // Dictionary application itself uses; they are not in the headers, so
    // they are looked up by name when needed and the thesaurus is simply
    // absent should a future macOS drop them.

    type CopyDictionaries = unsafe extern "C" fn() -> *const c_void;
    type DictionaryText = unsafe extern "C" fn(*const c_void) -> *const c_void;
    type CopyRecords =
        unsafe extern "C" fn(*const c_void, *const c_void, isize, isize) -> *const c_void;
    type RecordData = unsafe extern "C" fn(*const c_void, isize) -> *const c_void;

    struct Dcs {
        dictionaries: CopyDictionaries,
        identifier: DictionaryText,
        name: DictionaryText,
        url: DictionaryText,
        records: CopyRecords,
        data: RecordData,
    }

    impl Dcs {
        fn load() -> Option<Dcs> {
            // RTLD_DEFAULT: every image the process has loaded.
            let find = |name: &CStr| {
                let symbol = unsafe { dlsym(-2isize as *mut c_void, name.as_ptr()) };
                (!symbol.is_null()).then_some(symbol)
            };
            unsafe {
                Some(Dcs {
                    dictionaries: std::mem::transmute::<*mut c_void, CopyDictionaries>(find(
                        c"DCSCopyAvailableDictionaries",
                    )?),
                    identifier: std::mem::transmute::<*mut c_void, DictionaryText>(find(
                        c"DCSDictionaryGetIdentifier",
                    )?),
                    name: std::mem::transmute::<*mut c_void, DictionaryText>(find(
                        c"DCSDictionaryGetName",
                    )?),
                    url: std::mem::transmute::<*mut c_void, DictionaryText>(find(
                        c"DCSDictionaryGetURL",
                    )?),
                    records: std::mem::transmute::<*mut c_void, CopyRecords>(find(
                        c"DCSCopyRecordsForSearchString",
                    )?),
                    data: std::mem::transmute::<*mut c_void, RecordData>(find(
                        c"DCSRecordCopyData",
                    )?),
                })
            }
        }

        /// Run `f` on each thesaurus that is installed (a dictionary of
        /// the catalogue that was not downloaded has no place on disk),
        /// until it answers.
        fn each_thesaurus<R>(
            &self,
            mut f: impl FnMut(*const c_void, String) -> Option<R>,
        ) -> Option<R> {
            unsafe {
                let set = (self.dictionaries)();
                if set.is_null() {
                    return None;
                }
                let count = CFSetGetCount(set).max(0) as usize;
                let mut all: Vec<*const c_void> = vec![std::ptr::null(); count];
                CFSetGetValues(set, all.as_mut_ptr());
                // A CFString is an NSString.
                let text = |string: *const c_void| rust_string(string as Id);
                let mut found: Vec<(String, *const c_void, String)> = all
                    .into_iter()
                    .filter(|dictionary| !(self.url)(*dictionary).is_null())
                    .map(|dictionary| {
                        (
                            text((self.identifier)(dictionary)),
                            dictionary,
                            text((self.name)(dictionary)),
                        )
                    })
                    .filter(|(identifier, _, name)| super::is_thesaurus(identifier, name))
                    .collect();
                found.sort();
                let mut result = None;
                for (_, dictionary, name) in found {
                    result = f(dictionary, name);
                    if result.is_some() {
                        break;
                    }
                }
                CFRelease(set);
                result
            }
        }
    }

    /// The thesaurus dictionaries installed on this Mac.
    pub struct SystemThesaurus;

    impl Thesaurus for SystemThesaurus {
        fn sources(&self) -> Vec<String> {
            let mut names = Vec::new();
            if let Some(dcs) = Dcs::load() {
                with_pool(|| {
                    dcs.each_thesaurus(|_, name| {
                        names.push(name);
                        None::<()>
                    })
                });
            }
            names
        }

        fn lookup(&self, word: &str) -> Synonyms {
            let Some(dcs) = Dcs::load() else {
                return Synonyms::NoThesaurus;
            };
            with_pool(|| unsafe {
                let mut any = false;
                let found = dcs.each_thesaurus(|dictionary, _| {
                    any = true;
                    let records = (dcs.records)(dictionary, ns_string(word) as *const c_void, 0, 0);
                    if records.is_null() {
                        return None;
                    }
                    let mut entry = None;
                    for index in 0..CFArrayGetCount(records) {
                        // Version 0: the entry as the dictionary stores
                        // it, without the style sheet.
                        let data = (dcs.data)(CFArrayGetValueAtIndex(records, index), 0);
                        if data.is_null() {
                            continue;
                        }
                        let xml = rust_string(data as Id);
                        CFRelease(data);
                        let (headword, senses) = super::parse_thesaurus_entry(&xml);
                        if !senses.is_empty() {
                            entry = Some(Synonyms::Found { headword, senses });
                            break;
                        }
                    }
                    CFRelease(records);
                    entry
                });
                match found {
                    Some(entry) => entry,
                    None if any => Synonyms::NotFound,
                    None => Synonyms::NoThesaurus,
                }
            })
        }
    }
}

/// Whether a dictionary of the Dictionary application is a thesaurus:
/// Apple ships the Oxford Thesaurus of English, the Oxford American
/// Writer's Thesaurus and a Chinese one, and names them so.
pub fn is_thesaurus(identifier: &str, name: &str) -> bool {
    let (identifier, name) = (identifier.to_lowercase(), name.to_lowercase());
    identifier.ends_with(".ote")
        || identifier.ends_with(".oawt")
        || identifier.ends_with(".thes")
        || name.contains("thesaurus")
}

/// Read an entry of one of Apple's thesaurus dictionaries (XHTML whose
/// `class` attributes say what each piece is): the headword and the
/// meanings, without the phrases listed after them.
pub fn parse_thesaurus_entry(xml: &str) -> (String, Vec<Sense>) {
    /// What the text inside an element is.
    #[derive(Clone, Copy, PartialEq)]
    enum Part {
        Other,
        Skip,
        Pos,
        Example,
        Synonym,
        Antonym,
        /// Punctuation and labels the dictionary adds between words.
        Glue,
    }
    let mut headword = String::new();
    let mut senses: Vec<Sense> = Vec::new();
    let mut pos = String::new();
    // The elements being read, innermost last: what each is, the text
    // gathered in it so far, and whether it is a piece of the one around
    // it (its text then goes to that one).
    let mut stack: Vec<(Part, String, bool)> = Vec::new();
    let is_word = |part: Part| {
        matches!(
            part,
            Part::Pos | Part::Example | Part::Synonym | Part::Antonym
        )
    };
    let mut rest = xml;
    // Only the body is the entry.
    if let Some(body) = rest.find("<body") {
        rest = &rest[body..];
    }
    while let Some(open) = rest.find('<') {
        let text = &rest[..open];
        if let Some((part, gathered, _)) = stack.last_mut() {
            if is_word(*part) {
                gathered.push_str(&unescape(text));
            }
        }
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if tag.starts_with('/') {
            let Some((part, gathered, piece)) = stack.pop() else {
                continue;
            };
            if piece {
                if let Some((_, around, _)) = stack.last_mut() {
                    around.push_str(&gathered);
                }
                continue;
            }
            let word = gathered.split_whitespace().collect::<Vec<_>>().join(" ");
            match part {
                Part::Pos => pos = word,
                Part::Example => {
                    if let Some(sense) = senses.last_mut() {
                        if sense.example.is_empty() {
                            sense.example = word;
                        }
                    }
                }
                Part::Synonym | Part::Antonym if !word.is_empty() => {
                    if let Some(sense) = senses.last_mut() {
                        let list = if part == Part::Synonym {
                            &mut sense.synonyms
                        } else {
                            &mut sense.antonyms
                        };
                        if !list.contains(&word) {
                            list.push(word);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let class = attribute(tag, "class").unwrap_or_default();
        let first = class.split_whitespace().next().unwrap_or_default();
        let inside = stack.last().map_or(Part::Other, |(part, _, _)| *part);
        let (part, piece) = if inside == Part::Skip {
            (Part::Skip, false)
        } else {
            match first {
                // Phrases built on the word and the notes on usage are
                // not meanings of the word.
                "subEntryBlock" | "feature" | "note" => (Part::Skip, false),
                "pos" => (Part::Pos, false),
                "ex" => (Part::Example, false),
                "syn" => (Part::Synonym, false),
                "ant" => (Part::Antonym, false),
                // The name of a part of speech is written in such a
                // span; elsewhere it holds the commas between words.
                "gp" if inside == Part::Pos => (Part::Pos, true),
                "gp" | "lg" | "reg" | "xrg" => (Part::Glue, false),
                // Whatever else is inside a word belongs to it.
                _ if is_word(inside) => (inside, true),
                _ => (Part::Other, false),
            }
        };
        if tag.contains("d:entry") && headword.is_empty() {
            headword = unescape(&attribute(tag, "d:title").unwrap_or_default());
        }
        if first == "msThes" && part != Part::Skip {
            senses.push(Sense {
                part_of_speech: pos.clone(),
                ..Sense::default()
            });
        }
        if !tag.ends_with('/') {
            stack.push((part, String::new(), piece));
        }
    }
    senses.retain(|sense| !sense.synonyms.is_empty() || !sense.antonyms.is_empty());
    (headword, senses)
}

/// The value of an attribute of a start tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let at = tag.match_indices(name).find(|(index, _)| {
        tag[..*index].ends_with(char::is_whitespace) && tag[index + name.len()..].starts_with("=\"")
    })?;
    let value = &tag[at.0 + name.len() + 2..];
    Some(value[..value.find('"')?].to_string())
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
}

// -------------------------------------------------------------- stand-ins

#[cfg(any(not(target_os = "macos"), test))]
pub mod fake {
    //! Services that never leave the process: what a window gets in a
    //! test, and what a system other than macOS has.

    #[cfg(test)]
    use super::{Sense, TranslateError, Translation, Translator};
    use super::{SpellChecker, Synonyms, Thesaurus};
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::ops::Range;

    /// A checker that knows the words it was given (compared without
    /// case), and guesses what it was told to guess.
    #[derive(Default)]
    pub struct Words {
        pub known: RefCell<BTreeSet<String>>,
        pub guesses: BTreeMap<String, Vec<String>>,
        pub learned: RefCell<Vec<String>>,
        pub ignored: RefCell<BTreeSet<String>>,
        /// Every word is right when the list of known words is empty.
        pub strict: bool,
    }

    impl Words {
        #[cfg(test)]
        pub fn knowing(words: &[&str], guesses: &[(&str, &[&str])]) -> Words {
            Words {
                known: RefCell::new(words.iter().map(|word| word.to_lowercase()).collect()),
                guesses: guesses
                    .iter()
                    .map(|(word, list)| {
                        (
                            word.to_string(),
                            list.iter().map(|guess| guess.to_string()).collect(),
                        )
                    })
                    .collect(),
                strict: true,
                ..Words::default()
            }
        }

        fn accepts(&self, word: &str) -> bool {
            !self.strict
                || self.known.borrow().contains(&word.to_lowercase())
                || self.ignored.borrow().contains(word)
                || self.learned.borrow().iter().any(|learned| learned == word)
        }
    }

    impl SpellChecker for Words {
        fn languages(&self) -> Vec<String> {
            vec!["en".to_string(), "fr".to_string()]
        }

        fn next_misspelling(
            &self,
            text: &str,
            from: usize,
            _language: &str,
        ) -> Option<Range<usize>> {
            let mut start = None;
            let from = from.min(text.len());
            let ends = text[from..]
                .char_indices()
                .map(|(at, ch)| (from + at, Some(ch)))
                .chain([(text.len(), None)]);
            for (at, ch) in ends {
                let in_word = ch.is_some_and(|ch| ch.is_alphanumeric() || ch == '\'');
                match (start, in_word) {
                    (None, true) => start = Some(at),
                    (Some(first), false) => {
                        if !self.accepts(&text[first..at]) {
                            return Some(first..at);
                        }
                        start = None;
                    }
                    _ => {}
                }
            }
            None
        }

        fn guesses(&self, text: &str, range: Range<usize>, _language: &str) -> Vec<String> {
            self.guesses.get(&text[range]).cloned().unwrap_or_default()
        }

        fn ignore(&self, word: &str) {
            self.ignored.borrow_mut().insert(word.to_string());
        }

        fn unignore(&self, word: &str) {
            self.ignored.borrow_mut().remove(word);
        }

        fn learn(&self, word: &str) {
            self.learned.borrow_mut().push(word.to_string());
        }

        fn unlearn(&self, word: &str) {
            self.learned.borrow_mut().retain(|learned| learned != word);
        }
    }

    /// A Mac without any thesaurus.
    pub struct NoBooks;

    impl Thesaurus for NoBooks {
        fn sources(&self) -> Vec<String> {
            Vec::new()
        }

        fn lookup(&self, _word: &str) -> Synonyms {
            Synonyms::NoThesaurus
        }
    }

    /// A thesaurus of a few words.
    #[cfg(test)]
    pub struct Book(pub BTreeMap<String, Vec<Sense>>);

    #[cfg(test)]
    impl Thesaurus for Book {
        fn sources(&self) -> Vec<String> {
            vec!["Test Thesaurus".to_string()]
        }

        fn lookup(&self, word: &str) -> Synonyms {
            match self.0.get(&word.to_lowercase()) {
                Some(senses) => Synonyms::Found {
                    headword: word.to_lowercase(),
                    senses: senses.clone(),
                },
                None => Synonyms::NotFound,
            }
        }
    }

    /// A translator that answers at once: the text in capitals between
    /// brackets naming the target, or the error it was made with.
    #[cfg(test)]
    pub struct Echo(pub Option<TranslateError>);

    #[cfg(test)]
    impl Translator for Echo {
        fn languages(&self) -> Result<Vec<String>, TranslateError> {
            match &self.0 {
                Some(error) => Err(error.clone()),
                None => Ok(vec![
                    "de".to_string(),
                    "en".to_string(),
                    "es".to_string(),
                    "fr".to_string(),
                    "it".to_string(),
                ]),
            }
        }

        fn translate(
            &self,
            text: &str,
            from: Option<&str>,
            to: &str,
        ) -> Result<Translation, TranslateError> {
            match &self.0 {
                Some(error) => Err(error.clone()),
                None => Ok(Translation {
                    text: format!("[{to}] {}", text.to_uppercase()),
                    source: from.unwrap_or("en").to_string(),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_convert_between_bytes_and_utf16() {
        let text = "é𝄞a";
        assert_eq!(
            [0, 1, 3, 4].map(|utf16| byte_of_utf16(text, utf16)),
            [0, 2, 6, 7]
        );
        assert_eq!(
            [0, 2, 6, 7].map(|byte| utf16_of_byte(text, byte)),
            [0, 1, 3, 4]
        );
        assert_eq!(byte_of_utf16(text, 99), text.len());
    }

    #[test]
    fn helper_answers_are_read() {
        assert_eq!(
            parse_helper_answer("ok en\nBonjour\nle monde\n"),
            Ok(("en".to_string(), "Bonjour\nle monde".to_string()))
        );
        assert_eq!(
            parse_helper_answer("ok\nde\nfr\n"),
            Ok((String::new(), "de\nfr".to_string()))
        );
        assert_eq!(
            parse_helper_answer("error not-installed\n"),
            Err(TranslateError::NotInstalled)
        );
        assert_eq!(
            parse_helper_answer("error unsupported-system\n"),
            Err(TranslateError::UnsupportedSystem)
        );
        assert_eq!(
            parse_helper_answer("error unsupported-pair en xx\n"),
            Err(TranslateError::UnsupportedPair)
        );
        assert_eq!(
            parse_helper_answer("error unknown-language\n"),
            Err(TranslateError::UnknownLanguage)
        );
        assert_eq!(
            parse_helper_answer("error failed The model is busy\n"),
            Err(TranslateError::Failed("The model is busy".to_string()))
        );
        assert_eq!(
            parse_helper_answer(""),
            Err(TranslateError::Failed(String::new()))
        );
        // No helper: said without running anything.
        let none = HelperTranslator { helper: None };
        assert!(!none.available());
        assert_eq!(none.languages(), Err(TranslateError::NoHelper));
        assert_eq!(
            none.translate("a", None, "fr"),
            Err(TranslateError::NoHelper)
        );
    }

    /// An entry shaped like those of the Oxford Thesaurus of English.
    const ENTRY: &str = r#"<?xml version="1.0"?><html xmlns:d="x"><head><style>.syn{}</style></head><body><d:entry id="t1" d:title="happy" class="entry"><span class="hg x_xh0"><span role="text" class="hw">happy </span></span><span class="sg"><span class="se1 x_xd0"><span role="text" class="posg x_xdh"><span d:pos="1" class="pos"><span class="gp tg_pos">adjective </span><d:pos></d:pos></span></span><span class="se2 x_xd1 hasSn"><span class="gp x_xdh sn ty_label tg_se2"> 1 </span><span id="t1.001" class="msThes x_xd1sub t_core"><span role="text" class="eg"><span class="ex">Melissa looked happy &amp; excited</span><span class="gp tg_eg">. </span></span><span role="text" class="synList"><span class="synGroup"><span d:def="1" class="syn t_core">contented<d:def></d:def><span class="gp tg_syn">, </span></span><span class="syn">cheerful<span class="gp tg_syn">, </span></span><span class="syn">in good spirits</span><span class="gp tg_synGroup">; </span></span><span class="synGroup"><span class="lg"><span class="reg">informal </span></span><span class="syn">chirpy</span></span></span><span class="antList"><span class="gp ty_label tg_antList">ANTONYMS </span><span class="antGroup"><span class="ant">sad</span></span></span></span></span><span class="se2 x_xd1 hasSn"><span class="gp sn"> 2 </span><span class="msThes x_xd1sub t_core"><span class="synList"><span class="synGroup"><span class="syn">willing</span><span class="syn">glad</span></span></span></span></span></span><span class="se1 x_xd0"><span class="posg"><span class="pos"><span class="gp tg_pos">noun </span></span></span><span class="se2"><span class="msThes"><span class="synList"><span class="syn">joy</span></span></span></span></span><span class="subEntryBlock x_xo0 t_phrases"><span class="subEntry"><span class="l">happy medium</span><span class="msThes"><span class="syn">compromise</span></span></span></span></span></d:entry></body></html>"#;

    #[test]
    fn a_thesaurus_entry_is_read() {
        let (headword, senses) = parse_thesaurus_entry(ENTRY);
        assert_eq!(headword, "happy");
        let sense = |pos: &str, example: &str, synonyms: &[&str], antonyms: &[&str]| Sense {
            part_of_speech: pos.to_string(),
            example: example.to_string(),
            synonyms: synonyms.iter().map(|word| word.to_string()).collect(),
            antonyms: antonyms.iter().map(|word| word.to_string()).collect(),
        };
        assert_eq!(
            senses,
            vec![
                sense(
                    "adjective",
                    "Melissa looked happy & excited",
                    &["contented", "cheerful", "in good spirits", "chirpy"],
                    &["sad"]
                ),
                sense("adjective", "", &["willing", "glad"], &[]),
                sense("noun", "", &["joy"], &[]),
            ]
        );
        // Anything else has no meanings.
        assert_eq!(
            parse_thesaurus_entry("<html><body><p>nothing</p></body></html>").1,
            Vec::new()
        );
        assert_eq!(parse_thesaurus_entry("").1, Vec::new());
        assert_eq!(
            parse_thesaurus_entry("<span class=\"syn\">cut off").1,
            Vec::new()
        );
    }

    #[test]
    fn thesauruses_are_told_from_dictionaries() {
        assert!(is_thesaurus(
            "com.apple.dictionary.OTE",
            "Oxford Thesaurus of English"
        ));
        assert!(is_thesaurus(
            "com.apple.dictionary.OAWT",
            "Oxford American Writer’s Thesaurus"
        ));
        assert!(is_thesaurus(
            "com.apple.dictionary.zh_CN.thes",
            "现代汉语同义词典"
        ));
        assert!(!is_thesaurus(
            "com.apple.dictionary.NOAD",
            "New Oxford American Dictionary"
        ));
        assert!(!is_thesaurus(
            "com.apple.dictionary.fr.Multi",
            "Multidictionnaire de la langue française"
        ));
    }

    #[test]
    fn the_stand_in_checker_finds_unknown_words() {
        let checker = fake::Words::knowing(&["the", "cat"], &[("catt", &["cat", "cart"])]);
        let text = "The catt, the Cat";
        assert_eq!(checker.next_misspelling(text, 0, "en"), Some(4..8));
        assert_eq!(checker.guesses(text, 4..8, "en"), vec!["cat", "cart"]);
        assert_eq!(checker.next_misspelling(text, 8, "en"), None);
        checker.ignore("catt");
        assert_eq!(checker.next_misspelling(text, 0, "en"), None);
        // One that knows nothing in particular accepts everything.
        assert_eq!(
            fake::Words::default().next_misspelling("zzzz", 0, "en"),
            None
        );
    }

    // ---- the real system. These call macOS; each is skipped, saying so,
    // where the Mac running the tests lacks what it needs.

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_checker_checks_spelling() {
        let checker = mac::SystemSpellChecker::new();
        let languages = checker.languages();
        let Some(english) = languages.iter().find(|language| language.starts_with("en")) else {
            eprintln!("skipped: the system checker has no English ({languages:?})");
            return;
        };
        let text = "This is a tset of speling.";
        let Some(first) = checker.next_misspelling(text, 0, english) else {
            eprintln!(
                "skipped: the system checker found nothing (no spelling service in this session)"
            );
            return;
        };
        assert_eq!(&text[first.clone()], "tset");
        assert!(
            checker
                .guesses(text, first.clone(), english)
                .iter()
                .any(|guess| guess == "test")
        );
        let second = checker
            .next_misspelling(text, first.end, english)
            .expect("a second word");
        assert_eq!(&text[second.clone()], "speling");
        assert_eq!(checker.next_misspelling(text, second.end, english), None);
        // Ignored words are passed over, until the checker goes.
        checker.ignore("tset");
        assert_eq!(checker.next_misspelling(text, 0, english), Some(second));
        checker.unignore("tset");
        assert_eq!(
            checker.next_misspelling(text, 0, english),
            Some(first.clone())
        );
        checker.ignore("tset");
        drop(checker);
        let fresh = mac::SystemSpellChecker::new();
        assert_eq!(fresh.next_misspelling(text, 0, english), Some(first));
        // Offsets are bytes, whatever comes before the word.
        let accented = "Café 𝄞 speling";
        let found = fresh
            .next_misspelling(accented, 0, english)
            .map(|range| &accented[range]);
        assert_eq!(found, Some("speling"));
        assert!(
            mac::language_name("en_GB", "fr")
                .is_some_and(|name| name.to_lowercase().contains("anglais"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_thesaurus_gives_synonyms() {
        let thesaurus = mac::SystemThesaurus;
        let sources = thesaurus.sources();
        if sources.is_empty() {
            eprintln!("skipped: no thesaurus dictionary is installed on this Mac");
            assert_eq!(thesaurus.lookup("happy"), Synonyms::NoThesaurus);
            return;
        }
        if !sources
            .iter()
            .any(|name| name.contains("English") || name.contains("American"))
        {
            eprintln!("skipped: no English thesaurus among {sources:?}");
            return;
        }
        let Synonyms::Found { headword, senses } = thesaurus.lookup("happy") else {
            panic!("no entry for \"happy\" in {sources:?}");
        };
        assert_eq!(headword, "happy");
        assert!(
            senses[0].part_of_speech.starts_with("adj"),
            "{:?}",
            senses[0]
        );
        assert!(
            senses
                .iter()
                .any(|sense| sense.synonyms.iter().any(|word| word == "cheerful"))
        );
        assert!(
            senses
                .iter()
                .any(|sense| sense.antonyms.iter().any(|word| word == "sad"))
        );
        assert_eq!(thesaurus.lookup("zzzqxv"), Synonyms::NotFound);
    }

    /// The helper, when `scripts/build_translate_helper.sh` built it for
    /// this profile.
    fn built_helper() -> Option<HelperTranslator> {
        let dir = std::env::current_exe()
            .ok()?
            .parent()?
            .parent()?
            .to_path_buf();
        Some(HelperTranslator::at(dir.join(HELPER_NAME))).filter(HelperTranslator::available)
    }

    #[test]
    fn the_system_translator_translates() {
        let Some(translator) = built_helper() else {
            eprintln!(
                "skipped: the translation helper is not built (scripts/build_translate_helper.sh)"
            );
            return;
        };
        let languages = match translator.languages() {
            Ok(languages) => languages,
            Err(error) => {
                eprintln!("skipped: macOS has no translation here ({error:?})");
                return;
            }
        };
        assert!(
            languages.iter().any(|language| language == "fr"),
            "{languages:?}"
        );
        match translator.translate("Good morning", Some("en"), "fr") {
            Ok(translation) => {
                assert_eq!(translation.source, "en");
                assert!(
                    translation.text.to_lowercase().contains("bonjour"),
                    "{translation:?}"
                );
            }
            Err(TranslateError::NotInstalled) => {
                eprintln!("skipped: English and French are not downloaded on this Mac")
            }
            Err(error) => panic!("{error:?}"),
        }
        // The language is found when none is given.
        match translator.translate("Où est la gare ?", None, "en") {
            Ok(translation) => assert_eq!(translation.source, "fr"),
            Err(TranslateError::NotInstalled) => {
                eprintln!("skipped: French and English are not downloaded on this Mac")
            }
            Err(error) => panic!("{error:?}"),
        }
        assert_eq!(
            translator.translate("Good morning", Some("en"), "xx-nowhere"),
            Err(TranslateError::UnsupportedPair)
        );
    }
}
