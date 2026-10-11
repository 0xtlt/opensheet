//! Checks on the localized function-name tables.

use super::{Locale, names_de, names_es, names_fr, names_it};
use crate::ast::Expr;
use crate::functions;
use crate::parser::parse_formula;
use crate::value::ErrorKind;
use std::collections::{BTreeMap, BTreeSet};

struct Table {
    locale: &'static Locale,
    /// (canonical English name, localized name).
    functions: &'static [(&'static str, &'static str)],
    /// Registered functions deliberately left out of `functions`.
    same_as_english: &'static [&'static str],
}

fn tables() -> [Table; 4] {
    [
        Table {
            locale: Locale::fr(),
            functions: names_fr::FUNCTIONS,
            same_as_english: names_fr::SAME_AS_ENGLISH,
        },
        Table {
            locale: Locale::es(),
            functions: names_es::FUNCTIONS,
            same_as_english: names_es::SAME_AS_ENGLISH,
        },
        Table {
            locale: Locale::de(),
            functions: names_de::FUNCTIONS,
            same_as_english: names_de::SAME_AS_ENGLISH,
        },
        Table {
            locale: Locale::it(),
            functions: names_it::FUNCTIONS,
            same_as_english: names_it::SAME_AS_ENGLISH,
        },
    ]
}

/// Localized names that are also the English name of a *different* function:
/// (language, canonical function, its localized name). Excel itself has these
/// overlaps, and resolves the name the way the language's users expect (in
/// French, `TRIM` is the modified internal rate of return). Each is only
/// tolerable because the function whose English name is taken has a localized
/// name of its own; `localized_names_do_not_hide_other_functions` enforces it.
const SHADOWS: &[(&str, &str, &str)] = &[
    ("fr", "MIRR", "TRIM"),
    ("fr", "VARP", "VAR.P"),
    ("es", "FIXED", "DECIMAL"),
];

#[test]
fn the_tables_are_the_ones_the_locales_use() {
    for t in tables() {
        let id = t.locale.id;
        assert!(!t.functions.is_empty(), "{id}: empty table");
        for (canon, local) in t.functions {
            assert_eq!(t.locale.fn_to_local(canon), *local, "{id}: {canon}");
        }
    }
    assert_eq!(Locale::en().fn_to_local("SUM"), "SUM");
}

#[test]
fn canonical_names_are_registered() {
    for t in tables() {
        let id = t.locale.id;
        let mut seen = BTreeSet::new();
        for (canon, local) in t.functions {
            assert!(
                functions::lookup(canon).is_some(),
                "{id}: {canon} (\"{local}\") is not a registered function"
            );
            assert!(seen.insert(*canon), "{id}: {canon} is listed twice");
            assert_ne!(
                canon, local,
                "{id}: {canon} is spelled as in English and belongs in SAME_AS_ENGLISH"
            );
        }
        let mut same = BTreeSet::new();
        for canon in t.same_as_english {
            assert!(
                functions::lookup(canon).is_some(),
                "{id}: SAME_AS_ENGLISH entry {canon} is not a registered function"
            );
            assert!(
                same.insert(*canon),
                "{id}: {canon} is listed twice in SAME_AS_ENGLISH"
            );
        }
    }
}

#[test]
fn localized_names_are_unique() {
    for t in tables() {
        let id = t.locale.id;
        let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
        for (canon, local) in t.functions {
            if let Some(other) = owner.insert(*local, *canon) {
                panic!("{id}: {other} and {canon} are both named {local}");
            }
        }
    }
}

#[test]
fn localized_names_do_not_hide_other_functions() {
    for t in tables() {
        let id = t.locale.id;
        let translated: BTreeSet<&str> = t.functions.iter().map(|(canon, _)| *canon).collect();
        let mut found = BTreeSet::new();
        for (canon, local) in t.functions {
            // `local` differs from `canon` (checked above), so a registered
            // function called `local` is a different one.
            if functions::lookup(local).is_none() {
                continue;
            }
            assert!(
                SHADOWS.contains(&(id, *canon, *local)),
                "{id}: {local}, the name of {canon}, is the English name of another function"
            );
            assert!(
                translated.contains(*local),
                "{id}: {local} would mean both {canon} and the function called {local} in English"
            );
            found.insert((id, *canon, *local));
        }
        for shadow in SHADOWS.iter().filter(|s| s.0 == id) {
            assert!(found.contains(shadow), "stale SHADOWS entry {shadow:?}");
        }
    }
    let known: Vec<&str> = tables().iter().map(|t| t.locale.id).collect();
    for shadow in SHADOWS {
        assert!(
            known.contains(&shadow.0),
            "SHADOWS entry for unknown language {shadow:?}"
        );
    }
}

/// A function added to the registry must be given a name in every language,
/// or be recorded as keeping its English one.
#[test]
fn every_function_is_accounted_for() {
    let registered: BTreeSet<&str> = functions::all().iter().map(|d| d.name).collect();
    for t in tables() {
        let id = t.locale.id;
        let translated: BTreeSet<&str> = t.functions.iter().map(|(canon, _)| *canon).collect();
        let untranslated: BTreeSet<&str> = registered.difference(&translated).copied().collect();
        let same: BTreeSet<&str> = t.same_as_english.iter().copied().collect();
        let unaccounted: Vec<&str> = untranslated.difference(&same).copied().collect();
        let stale: Vec<&str> = same.difference(&untranslated).copied().collect();
        assert!(
            unaccounted.is_empty(),
            "{id}: no localized name and not in SAME_AS_ENGLISH: {unaccounted:?}"
        );
        assert!(
            stale.is_empty(),
            "{id}: in SAME_AS_ENGLISH but translated or not registered: {stale:?}"
        );
    }
}

/// The parser uppercases what the user typed before looking the name up, and
/// only reads letters, digits, `.` and `_` as part of a function name.
#[test]
fn localized_names_are_typeable() {
    for t in tables() {
        let id = t.locale.id;
        for (canon, local) in t.functions {
            assert_eq!(
                local.to_uppercase(),
                *local,
                "{id}: {canon} is not uppercase"
            );
            assert!(
                local.chars().next().is_some_and(char::is_alphabetic),
                "{id}: {local} does not start with a letter"
            );
            assert!(
                local
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '.' || c == '_'),
                "{id}: {local} has a character the parser does not read as part of a name"
            );
        }
    }
}

/// Every registered function, in every language: the localized spelling parses
/// to the canonical function (whatever the case), and prints back unchanged.
#[test]
fn every_name_round_trips() {
    let en = Locale::en();
    for t in tables() {
        let id = t.locale.id;
        for def in functions::all() {
            let local = t.locale.fn_to_local(def.name);
            let text = format!("{local}(A1)");
            for typed in [text.clone(), text.to_lowercase()] {
                let expr = parse_formula(&typed, t.locale)
                    .unwrap_or_else(|e| panic!("{id}: {typed}: {e:?}"));
                match &expr {
                    Expr::Func(name, args) => {
                        assert_eq!(&**name, def.name, "{id}: {typed}");
                        assert_eq!(args.len(), 1, "{id}: {typed}");
                    }
                    other => panic!("{id}: {typed} parsed as {other:?}"),
                }
                assert_eq!(expr.to_text(t.locale), text, "{id}: {typed}");
                assert_eq!(expr.to_text(en), format!("{}(A1)", def.name), "{id}");
            }
        }
    }
}

/// `local` (typed in `locale`) and `canonical` (typed in English) are the same
/// formula, each printing as the other.
#[track_caller]
fn assert_round_trip(locale: &Locale, local: &str, canonical: &str) {
    let en = Locale::en();
    let id = locale.id;
    let from_local =
        parse_formula(local, locale).unwrap_or_else(|e| panic!("{id}: {local}: {e:?}"));
    assert_eq!(from_local.to_text(en), canonical, "{id}: {local} → English");
    assert_eq!(from_local.to_text(locale), local, "{id}: {local} → itself");
    let from_canonical =
        parse_formula(canonical, en).unwrap_or_else(|e| panic!("en: {canonical}: {e:?}"));
    assert_eq!(
        from_canonical.to_text(locale),
        local,
        "{id}: {canonical} → {id}"
    );
    assert_eq!(from_canonical, from_local, "{id}: {local} / {canonical}");
}

#[test]
fn german_formulas_round_trip() {
    let de = Locale::de();
    assert_round_trip(
        de,
        "ZÄHLENWENN(A1:A10;\">1,5\")+SUMMEWENNS(C:C;A:A;\"x\")",
        "COUNTIF(A1:A10,\">1,5\")+SUMIFS(C:C,A:A,\"x\")",
    );
    assert_round_trip(
        de,
        "WENN(UND(A1>0,5;NICHT(B1));WAHR;FALSCH)",
        "IF(AND(A1>0.5,NOT(B1)),TRUE,FALSE)",
    );
    assert_round_trip(
        de,
        "WENNFEHLER(SVERWEIS(A1;B:C;2;FALSCH);#NV)",
        "IFERROR(VLOOKUP(A1,B:C,2,FALSE),#N/A)",
    );
    assert_round_trip(
        de,
        "KGRÖSSTE(A:A;2)+BEREICH.VERSCHIEBEN(A1;1;1)+LÄNGE(B1)",
        "LARGE(A:A,2)+OFFSET(A1,1,1)+LEN(B1)",
    );
    assert_round_trip(
        de,
        "BOGENMASS(180)+OBERGRENZE.MATHEMATIK(A1)+GDA2(A1;A2;A3;1)",
        "RADIANS(180)+CEILING.MATH(A1)+DB(A1,A2,A3,1)",
    );
    assert_round_trip(de, "SUMME({1.2.3;4,5.5.6})", "SUM({1,2,3;4.5,5,6})");
    assert_round_trip(
        de,
        "INDEX({\"a\".WAHR;#WERT!.FALSCH};2;1)",
        "INDEX({\"a\",TRUE;#VALUE!,FALSE},2,1)",
    );
    assert_round_trip(de, "#BEZUG!+#ZAHL!+#DIV/0!", "#REF!+#NUM!+#DIV/0!");
    assert_round_trip(
        de,
        "XVERWEIS(A1;B:B;C:C)&TEXTVERKETTEN(\";\";WAHR;D1:D3)",
        "XLOOKUP(A1,B:B,C:C)&TEXTJOIN(\";\",TRUE,D1:D3)",
    );
}

#[test]
fn spanish_formulas_round_trip() {
    let es = Locale::es();
    assert_round_trip(
        es,
        "SI.ERROR(BUSCARV(A1;B:C;2;FALSO);#¡VALOR!)",
        "IFERROR(VLOOKUP(A1,B:C,2,FALSE),#VALUE!)",
    );
    assert_round_trip(
        es,
        "AÑO(HOY())+FRAC.AÑO(A1;A2)+DIA(A1)",
        "YEAR(TODAY())+YEARFRAC(A1,A2)+DAY(A1)",
    );
    assert_round_trip(
        es,
        "SI(Y(A1>0,5;NO(B1));VERDADERO;FALSO)",
        "IF(AND(A1>0.5,NOT(B1)),TRUE,FALSE)",
    );
    assert_round_trip(
        es,
        "CONTAR.SI.CONJUNTO(A:A;\">0\")+M.C.D(4;6)+COEFICIENTE.R2(A1:A3;B1:B3)",
        "COUNTIFS(A:A,\">0\")+GCD(4,6)+RSQ(A1:A3,B1:B3)",
    );
    assert_round_trip(es, "SUMA({1\\2\\3;4,5\\5\\6})", "SUM({1,2,3;4.5,5,6})");
    assert_round_trip(
        es,
        "INDICE({\"a\"\\VERDADERO;#¿NOMBRE?\\FALSO};2;1)",
        "INDEX({\"a\",TRUE;#NAME?,FALSE},2,1)",
    );
    assert_round_trip(es, "#¡REF!+#¡NUM!+#¡DIV/0!", "#REF!+#NUM!+#DIV/0!");
    // `DECIMAL` is Spanish for FIXED; the English DECIMAL is `CONV.DECIMAL`.
    assert_round_trip(
        es,
        "DECIMAL(A1;2)&CONV.DECIMAL(\"FF\";16)",
        "FIXED(A1,2)&DECIMAL(\"FF\",16)",
    );
    // Excel's Spanish keeps COVARIANCE.P in English but translates its sibling.
    assert_round_trip(
        es,
        "COVARIANCE.P(A1:A3;B1:B3)+COVARIANZA.M(A1:A3;B1:B3)",
        "COVARIANCE.P(A1:A3,B1:B3)+COVARIANCE.S(A1:A3,B1:B3)",
    );
}

#[test]
fn italian_formulas_round_trip() {
    let it = Locale::it();
    assert_round_trip(
        it,
        "SE.ERRORE(CERCA.VERT(A1;B:C;2;FALSO);#N/D)",
        "IFERROR(VLOOKUP(A1,B:C,2,FALSE),#N/A)",
    );
    assert_round_trip(
        it,
        "PIÙ.SE(A1>0,5;\"a\";VERO;\"b\")+SOMMA.PIÙ.SE(C:C;A:A;\"x\")",
        "IFS(A1>0.5,\"a\",TRUE,\"b\")+SUMIFS(C:C,A:A,\"x\")",
    );
    // The Italian IFNA really ends with a dot.
    assert_round_trip(
        it,
        "SE.NON.DISP.(CONFRONTA(A1;B:B;0);0)",
        "IFNA(MATCH(A1,B:B,0),0)",
    );
    assert_round_trip(
        it,
        "SE(E(A1>0;NON(B1));VERO;FALSO)+O(A1;B1)",
        "IF(AND(A1>0,NOT(B1)),TRUE,FALSE)+OR(A1,B1)",
    );
    assert_round_trip(
        it,
        "ARCTAN.2(1;1)+PI.GRECO()+NUM(A1)+MATR.SOMMA.PRODOTTO(A1:A3;B1:B3)",
        "ATAN2(1,1)+PI()+N(A1)+SUMPRODUCT(A1:A3,B1:B3)",
    );
    // Columns are separated by `\`, rows by `.`.
    assert_round_trip(it, "SOMMA({1\\2\\3.4,5\\5\\6})", "SUM({1,2,3;4.5,5,6})");
    assert_round_trip(
        it,
        "INDICE({\"a\"\\VERO.#VALORE!\\FALSO};2;1)",
        "INDEX({\"a\",TRUE;#VALUE!,FALSE},2,1)",
    );
    assert_round_trip(it, "GRANDE(A1:A9;{1.2.3})", "LARGE(A1:A9,{1;2;3})");
    assert_round_trip(it, "#RIF!+#NOME?+#DIV/0!", "#REF!+#NAME?+#DIV/0!");
}

#[test]
fn french_formulas_round_trip() {
    let fr = Locale::fr();
    assert_round_trip(
        fr,
        "SIERREUR(RECHERCHEV(A1;B:C;2;FAUX);#N/A)",
        "IFERROR(VLOOKUP(A1,B:C,2,FALSE),#N/A)",
    );
    assert_round_trip(
        fr,
        "SI(ET(A1>0,5;NON(B1));VRAI;FAUX)+NB.SI.ENS(A:A;\">0\")",
        "IF(AND(A1>0.5,NOT(B1)),TRUE,FALSE)+COUNTIFS(A:A,\">0\")",
    );
    assert_round_trip(
        fr,
        "LIEN_HYPERTEXTE(\"x\")&MAX.SI.ENS(A:A;B:B;1)&ARRONDI.AU.MULTIPLE(A1;5)",
        "HYPERLINK(\"x\")&MAXIFS(A:A,B:B,1)&MROUND(A1,5)",
    );
    assert_round_trip(fr, "SOMME({1.2.3;4,5.5.6})", "SUM({1,2,3;4.5,5,6})");
    assert_round_trip(
        fr,
        "INDEX({\"a\".VRAI;#VALEUR!.FAUX};2;1)",
        "INDEX({\"a\",TRUE;#VALUE!,FALSE},2,1)",
    );
    assert_round_trip(fr, "#NOM?+#NOMBRE!+#NUL!", "#NAME?+#NUM!+#NULL!");
    // `TRIM` and `VAR.P` are French names of other functions than the English
    // ones spelled that way.
    assert_round_trip(
        fr,
        "TRIM(A1:A3;0,1;0,2)&SUPPRESPACE(\" a \")",
        "MIRR(A1:A3,0.1,0.2)&TRIM(\" a \")",
    );
    assert_round_trip(
        fr,
        "VAR.P(A1:A3)+VAR.P.N(A1:A3)",
        "VARP(A1:A3)+VAR.P(A1:A3)",
    );
}

/// English names stay usable in a localized formula (unless the language
/// gives that spelling to another function), and are printed localized.
#[test]
fn english_names_are_accepted_everywhere() {
    for (locale, typed, printed) in [
        (
            Locale::de(),
            "SUM(A1;VLOOKUP(A2;B:C;2))",
            "SUMME(A1;SVERWEIS(A2;B:C;2))",
        ),
        (
            Locale::es(),
            "sum(A1;iferror(A2;0))",
            "SUMA(A1;SI.ERROR(A2;0))",
        ),
        (
            Locale::it(),
            "Sum(A1;IfNa(A2;0))",
            "SOMMA(A1;SE.NON.DISP.(A2;0))",
        ),
        (
            Locale::fr(),
            "SUM(A1;XLOOKUP(A2;B:B;C:C))",
            "SOMME(A1;RECHERCHEX(A2;B:B;C:C))",
        ),
    ] {
        let expr = parse_formula(typed, locale).unwrap();
        assert_eq!(expr.to_text(locale), printed, "{}", locale.id);
    }
}

/// Names are matched whatever their case, non-ASCII letters included: `ß`
/// uppercases to the `SS` Excel spells German names with.
#[test]
fn typed_names_ignore_case() {
    for (locale, typed, printed) in [
        (
            Locale::de(),
            "kgrößte(a1:a9;2)+Bogenmaß(180)+länge(groß(b1))",
            "KGRÖSSTE(A1:A9;2)+BOGENMASS(180)+LÄNGE(GROSS(B1))",
        ),
        (
            Locale::es(),
            "año(hoy())+Frac.Año(a1;a2)",
            "AÑO(HOY())+FRAC.AÑO(A1;A2)",
        ),
        (
            Locale::it(),
            "più.se(a1;1;vero;2)+conta.più.se(a:a;1)",
            "PIÙ.SE(A1;1;VERO;2)+CONTA.PIÙ.SE(A:A;1)",
        ),
        (
            Locale::fr(),
            "lien_hypertexte(\"x\")&Nb.Si(a:a;vrai)",
            "LIEN_HYPERTEXTE(\"x\")&NB.SI(A:A;VRAI)",
        ),
    ] {
        let expr = parse_formula(typed, locale).unwrap();
        assert_eq!(expr.to_text(locale), printed, "{}", locale.id);
    }
}

/// Locale data the formula parser depends on.
#[test]
fn locale_literals_are_consistent() {
    for locale in Locale::all() {
        let id = locale.id;
        let mut seen = BTreeSet::new();
        for (i, name) in locale.errors.iter().enumerate() {
            assert!(name.starts_with('#'), "{id}: {name}");
            assert_eq!(name.to_uppercase(), *name, "{id}: {name} is not uppercase");
            assert!(seen.insert(*name), "{id}: {name} names two errors");
            assert_eq!(
                locale.parse_error(name),
                Some(ErrorKind::ALL[i]),
                "{id}: {name}"
            );
            assert_eq!(
                locale.parse_error(&name.to_lowercase()),
                Some(ErrorKind::ALL[i])
            );
            // The English spelling is accepted too, and must not mean another
            // error in this language.
            assert_eq!(
                locale.parse_error(ErrorKind::ALL[i].as_str()),
                Some(ErrorKind::ALL[i]),
                "{id}: {name}"
            );
        }
        assert_eq!(locale.parse_bool(locale.true_name), Some(true), "{id}");
        assert_eq!(locale.parse_bool(locale.false_name), Some(false), "{id}");
        assert_eq!(locale.true_name.to_uppercase(), locale.true_name, "{id}");
        assert_eq!(locale.false_name.to_uppercase(), locale.false_name, "{id}");
        // TRUE() / FALSE() carry the same names as the literals.
        assert_eq!(locale.fn_to_local("TRUE"), locale.true_name, "{id}");
        assert_eq!(locale.fn_to_local("FALSE"), locale.false_name, "{id}");

        // Separators the parser must be able to tell apart.
        assert_ne!(locale.arg_sep, locale.decimal_sep, "{id}");
        assert_ne!(locale.array_col_sep, locale.array_row_sep, "{id}");
        assert_ne!(locale.array_col_sep, locale.decimal_sep, "{id}");
        assert_ne!(locale.array_row_sep, locale.decimal_sep, "{id}");
        assert_ne!(locale.thousands_sep, locale.decimal_sep, "{id}");
        assert_ne!(locale.fmt_day, locale.fmt_year, "{id}");
    }
}
