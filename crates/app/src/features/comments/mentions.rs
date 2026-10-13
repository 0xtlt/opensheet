//! Naming people in a comment: `@` opens the list of the people the
//! workbook knows, and the one chosen stands in the text as one piece.
//!
//! Excel takes the people it offers from the directory of the user's
//! organisation. There is none here: the list holds the people the
//! comments of the workbook already know and the user, and a full e-mail
//! address typed after the `@` names someone new. Nobody is sent a
//! message.

use crate::widgets::text_area::TextArea;
use opensheet_engine::comment::{Mention, Person, Post};
use std::ops::Range;

/// The colour of a mention, in the text being typed and in a posted
/// comment.
pub(super) const COLOR: u32 = 0x0F6CBD;
/// How many people the list shows at most.
const MAX_SHOWN: usize = 6;
/// A name being typed is given up beyond this many bytes.
const MAX_QUERY: usize = 64;

/// The `@` being typed at the caret: where it stands, and what was typed
/// after it. An `@` counts when it starts a word (the one inside an
/// address does not) and is not part of a mention already made; what
/// follows it stays on its line.
pub(super) fn query_at<'a>(text: &'a str, caret: usize, tokens: &[Range<usize>]) -> Option<(usize, &'a str)> {
    let before = text.get(..caret)?;
    let line = before.rfind('\n').map_or(0, |index| index + 1);
    let mut end = before.len();
    while let Some(found) = before[line..end].rfind('@') {
        let at = line + found;
        let starts_word = before[..at].chars().next_back().is_none_or(|ch| !ch.is_alphanumeric() && ch != '@' && ch != '.');
        let query = &before[at + 1..];
        if starts_word {
            let taken = tokens.iter().any(|token| token.start < caret && at < token.end);
            let fits = query.len() <= MAX_QUERY && !query.starts_with(char::is_whitespace);
            return (!taken && fits).then_some((at, query));
        }
        end = at;
    }
    None
}

/// Whether a text is a whole e-mail address: someone@domain.tld, without
/// blanks.
pub(super) fn is_email(text: &str) -> bool {
    let Some((name, domain)) = text.split_once('@') else {
        return false;
    };
    let plain = |part: &str| !part.is_empty() && !part.contains(|ch: char| ch.is_whitespace() || ch == '@');
    let labels: Vec<&str> = domain.split('.').collect();
    plain(name) && plain(domain) && labels.len() >= 2 && labels.iter().all(|label| !label.is_empty())
        && labels.last().is_some_and(|last| last.chars().count() >= 2)
}

/// The address of a person, when their account is one.
pub(super) fn address(person: &Person) -> Option<&str> {
    Some(person.user_id.as_str()).filter(|id| is_email(id) && *id != person.name)
}

/// The people offered for what was typed after the `@`: those of the
/// workbook and the user whose name or address has a word starting with
/// it, in the order the workbook knows them; then, when it is a whole
/// address nobody has, the person of that address.
pub(super) fn candidates(known: &[Person], user: &str, query: &str) -> Vec<Person> {
    let wanted = query.trim_end().to_lowercase();
    let starts = |text: &str| {
        let text = text.to_lowercase();
        text.starts_with(&wanted)
            || text
                .char_indices()
                .any(|(index, ch)| !ch.is_alphanumeric() && text[index + ch.len_utf8()..].starts_with(&wanted))
    };
    let mut people: Vec<Person> = Vec::new();
    for person in known.iter().cloned().chain((!user.trim().is_empty()).then(|| Person::new(user))) {
        // One line per name: the same person signed in two ways is one.
        if !person.name.trim().is_empty() && !people.iter().any(|seen| seen.name == person.name) {
            people.push(person);
        }
    }
    let mut out: Vec<Person> = people
        .into_iter()
        .filter(|person| starts(&person.name) || address(person).is_some_and(|address| starts(address)))
        .take(MAX_SHOWN)
        .collect();
    let typed = query.trim();
    if is_email(typed) && !out.iter().any(|person| person.user_id.eq_ignore_ascii_case(typed)) {
        out.push(Person::mentioned(typed, typed));
    }
    out
}

/// How a mention of someone is written in the text.
pub(super) fn token(person: &Person) -> String {
    format!("@{}", person.name)
}

/// The people named in a box, with the words that name them: a box only
/// knows where its mentions are, the card remembers who they are.
#[derive(Default)]
pub(super) struct Named(Vec<(String, Person)>);

impl Named {
    pub fn of(post: &Post) -> Named {
        Named(post.pieces().into_iter().filter_map(|(text, person)| Some((text.to_string(), person?.clone()))).collect())
    }

    pub fn add(&mut self, person: &Person) {
        let text = token(person);
        self.0.retain(|(known, _)| *known != text);
        self.0.push((text, person.clone()));
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// What a box holds, as a post would keep it: the text without the
    /// blanks around it, and the mentions that are still whole in it.
    pub fn draft(&self, area: &TextArea) -> (String, Vec<Mention>) {
        let text = area.text();
        let trimmed = text.trim();
        let lead = text.len() - text.trim_start().len();
        let mentions = area
            .tokens()
            .iter()
            .filter_map(|range| {
                let (_, person) = self.0.iter().find(|(known, _)| text.get(range.clone()) == Some(known.as_str()))?;
                let start = range.start.checked_sub(lead)?;
                (range.end - lead <= trimmed.len()).then(|| Mention {
                    person: person.clone(),
                    start,
                    len: range.len(),
                })
            })
            .collect();
        (trimmed.to_string(), mentions)
    }
}

/// The list open under a box.
pub(super) struct Suggestions {
    /// Under the box of a post being rewritten, not the box of the card.
    pub in_edit: bool,
    /// Where the `@` stands in the text.
    pub at: usize,
    pub people: Vec<Person>,
    pub selected: usize,
}

impl Suggestions {
    /// Move the selection, round the ends of the list.
    pub fn step(&mut self, down: bool) {
        let count = self.people.len().max(1);
        self.selected = if down { (self.selected + 1) % count } else { (self.selected + count - 1) % count };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_at_sign_being_typed() {
        assert_eq!(query_at("@", 1, &[]), Some((0, "")));
        assert_eq!(query_at("Hi @An", 6, &[]), Some((3, "An")));
        // Names have blanks; the caret says how far the name goes.
        assert_eq!(query_at("Hi @Ann Le, ok", 10, &[]), Some((3, "Ann Le")));
        // An address in running text is not a mention being typed, but an
        // address typed after an `@` is.
        assert_eq!(query_at("write to ann@example.com", 24, &[]), None);
        assert_eq!(query_at("ask @sam@example.com", 20, &[]), Some((4, "sam@example.com")));
        // On its own line only, never across a blank right after the `@`.
        assert_eq!(query_at("@Ann\nLee", 8, &[]), None);
        assert_eq!(query_at("(@Bo", 4, &[]), Some((1, "Bo")));
        assert_eq!(query_at("a @ b", 5, &[]), None);
        // A mention already made is left alone; one typed after it is found.
        assert_eq!(query_at("@Ann Lee", 8, &[0..8]), None);
        assert_eq!(query_at("@Ann Lee and @B", 15, &[0..8]), Some((13, "B")));
        // "é" is two bytes: offsets are bytes.
        assert_eq!(query_at("vérifié @Zo", 13, &[]), Some((10, "Zo")));
        assert_eq!(query_at("no sign", 7, &[]), None);
        assert_eq!(query_at("@x", 9, &[]), None);
    }

    #[test]
    fn addresses() {
        for good in ["a@b.co", "sam.lee+x@mail.example.com", "zoë@exämple.org"] {
            assert!(is_email(good), "{good}");
        }
        for bad in ["", "sam", "sam@", "@b.co", "sam@b", "sam@b.", "sam@.co", "s m@b.co", "a@b@c.co", "sam@b.c"] {
            assert!(!is_email(bad), "{bad}");
        }
    }

    #[test]
    fn who_is_offered() {
        let ann = Person::new("Ann Lee");
        let bob = Person::mentioned("Bob Martin", "robert@example.com");
        let nobody = Person::default();
        let known = [ann.clone(), bob.clone(), nobody, Person::mentioned("Ann Lee", "ann@example.com")];
        let names = |query: &str| -> Vec<String> {
            candidates(&known, "Thomas Tastet", query).into_iter().map(|person| person.name).collect()
        };
        // Everyone with a name, the user last, the same name once.
        assert_eq!(names(""), ["Ann Lee", "Bob Martin", "Thomas Tastet"]);
        // Any word of the name, or the address; capitals do not matter.
        assert_eq!(names("le"), ["Ann Lee"]);
        assert_eq!(names("MAR"), ["Bob Martin"]);
        assert_eq!(names("rob"), ["Bob Martin"]);
        assert_eq!(names("t"), ["Thomas Tastet"]);
        assert_eq!(names("nn"), Vec::<String>::new());
        assert_eq!(names("Ann L"), ["Ann Lee"]);
        // A whole address nobody has names someone new, as Excel records
        // the people of a directory.
        let new = candidates(&known, "Thomas Tastet", "sam@example.com");
        assert_eq!(new, [Person::mentioned("sam@example.com", "sam@example.com")]);
        assert_eq!(names("sam@example"), Vec::<String>::new());
        // An address someone has is that person.
        assert_eq!(candidates(&known, "", "Robert@Example.com"), [bob]);
        // The user is offered once even when the workbook knows them.
        assert_eq!(candidates(&[ann.clone()], "Ann Lee", ""), [ann]);
    }

    #[test]
    fn the_list_goes_round() {
        let mut list = Suggestions {
            in_edit: false,
            at: 0,
            people: vec![Person::new("A"), Person::new("B"), Person::new("C")],
            selected: 0,
        };
        list.step(false);
        assert_eq!(list.selected, 2);
        list.step(true);
        list.step(true);
        assert_eq!(list.selected, 1);
    }
}
