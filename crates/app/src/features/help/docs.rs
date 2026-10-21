//! What the Help pane shows: OpenSheet's own documentation, read from the
//! project's French guide `docs/fonctionnalites.md` at build time (its
//! sections "Ce qui fonctionne" and "Ce qui manque"), and what a search
//! finds in it. Nothing here is written twice: when the guide changes, so
//! does the help.

/// The guide of the project, as it was when the application was built.
const README: &str = include_str!("../../../../../docs/fonctionnalites.md");

/// The sections of the README that are the help, as their headings read.
pub(crate) const SECTIONS: [&str; 2] = ["Ce qui fonctionne", "Ce qui manque"];

/// One line of a topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub text: String,
    /// An item of a list (drawn with its dot), not a paragraph.
    pub bullet: bool,
    /// For a keyboard shortcut, its keys ("⌘C"), shown before the text.
    pub keys: Option<String>,
}

impl Line {
    fn new(text: &str, bullet: bool) -> Line {
        Line {
            text: plain(text),
            bullet,
            keys: None,
        }
    }
}

/// A page of the help: a group of the README ("Calcul"), or the list of
/// keyboard shortcuts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Topic {
    /// The heading it stands under in the list of topics.
    pub section: String,
    pub title: String,
    pub lines: Vec<Line>,
}

/// A piece of Markdown as plain text: bold and code marks taken off.
fn plain(text: &str) -> String {
    text.replace("**", "").replace('`', "").trim().to_string()
}

/// The topics of the sections `wanted` of a Markdown document. A topic
/// starts at a line that begins in bold (`**Calcul**`, with whatever
/// follows on the line as its first paragraph) or at a `###` heading;
/// list items are its bullets; pictures are left out; the rows of a table
/// read "cell — cell".
pub(crate) fn topics(markdown: &str, wanted: &[&str]) -> Vec<Topic> {
    let mut out: Vec<Topic> = Vec::new();
    let mut section: Option<String> = None;
    let mut open = false;
    for raw in markdown.lines() {
        let line = raw.trim();
        if let Some(heading) = line.strip_prefix("## ") {
            let heading = plain(heading);
            section = wanted.contains(&heading.as_str()).then_some(heading);
            open = false;
            continue;
        }
        let Some(section) = &section else {
            continue;
        };
        let mut start = |title: &str| {
            out.push(Topic {
                section: section.clone(),
                title: plain(title),
                lines: Vec::new(),
            });
        };
        if line.is_empty() || line.starts_with("![") || line.starts_with("```") {
            continue;
        }
        if let Some(title) = line.strip_prefix("### ") {
            start(title);
            open = true;
            continue;
        }
        let bold = line
            .strip_prefix("**")
            .and_then(|rest| rest.split_once("**"));
        if let Some((title, rest)) = bold {
            start(title);
            open = true;
            let rest = rest.trim().trim_start_matches(':').trim();
            if !rest.is_empty() {
                out.last_mut()
                    .expect("a topic")
                    .lines
                    .push(Line::new(rest, false));
            }
            continue;
        }
        // Text before the first group of a section is a topic by itself.
        if !open {
            start(section);
            open = true;
        }
        let topic = out.last_mut().expect("a topic");
        if let Some(item) = line.strip_prefix("- ") {
            topic.lines.push(Line::new(item, true));
        } else if line.starts_with('|') {
            let cells: Vec<String> = line.trim_matches('|').split('|').map(plain).collect();
            // The rule under the heads of a table says nothing.
            if !cells
                .iter()
                .all(|cell| cell.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
            {
                topic.lines.push(Line::new(&cells.join(" — "), false));
            }
        } else {
            topic.lines.push(Line::new(line, false));
        }
    }
    out
}

/// The help the application was built with.
pub(crate) fn readme_topics() -> Vec<Topic> {
    topics(README, &SECTIONS)
}

/// A text as a search compares it: small letters, accents taken off.
pub(crate) fn folded(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|ch| match ch {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            '’' => '\'',
            other => other,
        })
        .collect()
}

/// The lines a search finds: for each topic that has any, its index and
/// those of its lines that hold every word typed (in the line itself, its
/// keys, or the title of its topic).
pub(crate) fn search(topics: &[Topic], query: &str) -> Vec<(usize, Vec<usize>)> {
    let query = folded(query);
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    topics
        .iter()
        .enumerate()
        .filter_map(|(index, topic)| {
            let title = folded(&topic.title);
            let found: Vec<usize> = topic
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    let text = format!(
                        "{} {} {title}",
                        folded(&line.text),
                        folded(line.keys.as_deref().unwrap_or(""))
                    );
                    words.iter().all(|word| text.contains(word))
                })
                .map(|(line, _)| line)
                .collect();
            (!found.is_empty()).then_some((index, found))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Projet\n\nIntro.\n\n## Lancer\n\n- pas de l'aide\n\n## Ce qui fonctionne\n\n**Interface**\n- Barre de titre, `ruban` à onglets.\n- Grille de **1 048 576** lignes.\n\n**Calcul**\n- 337 fonctions.\n\n![Image](docs/x.jpg)\n\nUn classeur d'exemple :\n\n```bash\ncargo run\n```\n\n## Ce qui manque\n\n**Hors périmètre** : les macros VBA.\n\n**Partiels**\n- Commentaires : pas de filtres.\n\n### Vitesse du recalcul\n\n| Modification | Durée |\n|---|---|\n| Une cellule | 0,04 ms |\n\n## Architecture\n\n- rien de l'aide non plus\n";

    #[test]
    fn the_readme_becomes_topics() {
        let topics = topics(SAMPLE, &SECTIONS);
        let shape: Vec<(&str, &str, usize)> = topics
            .iter()
            .map(|topic| {
                (
                    topic.section.as_str(),
                    topic.title.as_str(),
                    topic.lines.len(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("Ce qui fonctionne", "Interface", 2),
                ("Ce qui fonctionne", "Calcul", 3),
                ("Ce qui manque", "Hors périmètre", 1),
                ("Ce qui manque", "Partiels", 1),
                ("Ce qui manque", "Vitesse du recalcul", 2),
            ]
        );
        // Marks are taken off; bullets are told from paragraphs; pictures
        // are left out, the lines of a code block are kept.
        assert_eq!(
            topics[0].lines[0],
            Line::new("Barre de titre, ruban à onglets.", true)
        );
        assert_eq!(topics[0].lines[1].text, "Grille de 1 048 576 lignes.");
        assert_eq!(
            topics[1].lines[1],
            Line::new("Un classeur d'exemple :", false)
        );
        assert_eq!(topics[1].lines[2].text, "cargo run");
        assert_eq!(topics[2].lines[0], Line::new("les macros VBA.", false));
        assert_eq!(topics[4].lines[0].text, "Modification — Durée");
        assert_eq!(topics[4].lines[1].text, "Une cellule — 0,04 ms");
    }

    #[test]
    fn the_real_readme_has_both_sections() {
        let topics = readme_topics();
        for section in SECTIONS {
            let count = topics
                .iter()
                .filter(|topic| topic.section == section)
                .count();
            assert!(count >= 2, "{section}: {count} topics");
        }
        assert!(
            topics
                .iter()
                .all(|topic| !topic.title.is_empty() && !topic.lines.is_empty())
        );
        // Something every version of the README says.
        assert!(!search(&topics, "fonctions").is_empty());
    }

    #[test]
    fn a_search_ignores_case_and_accents_and_wants_every_word() {
        let topics = topics(SAMPLE, &SECTIONS);
        assert_eq!(search(&topics, "GRILLE"), [(0, vec![1])]);
        assert_eq!(search(&topics, "perimetre"), [(2, vec![0])]);
        assert_eq!(search(&topics, "onglets barre"), [(0, vec![0])]);
        assert_eq!(search(&topics, "onglets fonctions"), []);
        // The title of a topic counts for each of its lines.
        assert_eq!(search(&topics, "calcul 337"), [(1, vec![0])]);
        assert_eq!(search(&topics, "interface"), [(0, vec![0, 1])]);
        assert_eq!(search(&topics, "   "), []);
    }
}
