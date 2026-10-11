#!/usr/bin/env python3
"""Check the interface translation tables against the source.

Interface strings are written inline as `t("English", "Français")`; Spanish,
German and Italian are looked up by the English text in
`crates/app/src/i18n_tables.rs`. This script extracts the English text of
every `t(…)` call in `crates/app/src/*.rs`, parses the three tables, and
reports for each language:

  * missing keys    - used in the source, absent from the table (shown in English)
  * unused keys     - in the table, used nowhere (dead or misspelled entries)
  * duplicate keys  - listed more than once in the table

Exit status is 1 when anything is missing or duplicated (or when a `t(…)`
call cannot be read), 0 otherwise. Unused keys and the style notes (lost
ellipsis, surrounding spaces, shortcut symbols…) are reported but not fatal.

    python3 scripts/check_i18n.py           # coverage report
    python3 scripts/check_i18n.py --list    # unique English strings, as JSON strings
    python3 scripts/check_i18n.py --pairs   # ["English", "Français", "file:line"] per string

Standard library only.
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "crates" / "app" / "src"
TABLES_FILE = SRC_DIR / "i18n_tables.rs"
SKIP = {"i18n.rs", "i18n_tables.rs"}
LANGS = (("es", "ES"), ("de", "DE"), ("it", "IT"))

SIMPLE_ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", "0": "\0", '"': '"', "'": "'"}


class ParseError(Exception):
    pass


def tokenize(text, path):
    """Rust source -> list of (kind, value, line).

    kind is "ident", "str" (value = the unescaped contents) or "punct".
    Comments, char literals, lifetimes and numbers are dropped, which is all
    the precision needed to find calls and table entries reliably without
    being fooled by a `t(` inside a comment or a string.
    """
    tokens = []
    i, n, line = 0, len(text), 1

    def fail(message):
        raise ParseError(f"{path}:{line}: {message}")

    def cooked_string(start):
        """Read a "…" literal whose opening quote is at `start`."""
        nonlocal line
        out = []
        j = start + 1
        while True:
            if j >= n:
                fail("unterminated string literal")
            c = text[j]
            if c == '"':
                return "".join(out), j + 1
            if c == "\n":
                line += 1
            if c != "\\":
                out.append(c)
                j += 1
                continue
            j += 1
            if j >= n:
                fail("unterminated string literal")
            e = text[j]
            if e in SIMPLE_ESCAPES:
                out.append(SIMPLE_ESCAPES[e])
                j += 1
            elif e == "x":
                out.append(chr(int(text[j + 1 : j + 3], 16)))
                j += 3
            elif e == "u":
                end = text.index("}", j)
                out.append(chr(int(text[j + 2 : end].replace("_", ""), 16)))
                j = end + 1
            elif e in "\r\n":
                # Line continuation: the newline and the next line's
                # leading whitespace are not part of the string.
                while j < n and text[j] in " \t\r\n":
                    if text[j] == "\n":
                        line += 1
                    j += 1
            else:
                fail(f"unknown string escape \\{e}")

    def raw_string(start):
        """Read r"…" / r#"…"# whose first `#` or quote is at `start`."""
        nonlocal line
        hashes = 0
        j = start
        while j < n and text[j] == "#":
            hashes += 1
            j += 1
        closing = '"' + "#" * hashes
        end = text.find(closing, j + 1)
        if end < 0:
            fail("unterminated raw string literal")
        value = text[j + 1 : end]
        line += value.count("\n")
        return value, end + len(closing)

    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif c in " \t\r":
            i += 1
        elif text.startswith("//", i):
            end = text.find("\n", i)
            i = n if end < 0 else end
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while depth and j < n:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    if text[j] == "\n":
                        line += 1
                    j += 1
            i = j
        elif c == '"':
            start_line = line
            value, i = cooked_string(i)
            tokens.append(("str", value, start_line))
        elif c == "'":
            if i + 1 < n and text[i + 1] == "\\":
                # Escaped char literal: '\n', '\'', '\u{1F600}'.
                j = i + 3
                while j < n and text[j] != "'":
                    j += 1
                i = j + 1
            elif i + 2 < n and text[i + 2] == "'":
                i += 3  # plain char literal
            else:
                i += 1  # lifetime or loop label
        elif c.isalpha() or c == "_":
            j = i
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            word = text[i:j]
            nxt = text[j] if j < n else ""
            start_line = line
            if word in ("r", "br", "cr") and nxt in ('"', "#"):
                k = j
                while k < n and text[k] == "#":
                    k += 1
                if k < n and text[k] == '"':
                    value, i = raw_string(j)
                    tokens.append(("str", value, start_line))
                    continue
            if word in ("b", "c") and nxt == '"':
                value, i = cooked_string(j)
                tokens.append(("str", value, start_line))
                continue
            if word == "b" and nxt == "'":
                i = j  # byte char literal: let the quote branch consume it
                continue
            tokens.append(("ident", word, start_line))
            i = j
        elif c.isdigit():
            j = i
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            i = j
        else:
            tokens.append(("punct", c, line))
            i += 1
    return tokens


def extract_calls(path):
    """Every `t("en", "fr")` call of a file: (calls, problems)."""
    tokens = tokenize(path.read_text(encoding="utf-8"), path)
    calls, problems = [], []
    for idx, (kind, value, line) in enumerate(tokens):
        if kind != "ident" or value != "t":
            continue
        if idx + 1 >= len(tokens) or tokens[idx + 1][:2] != ("punct", "("):
            continue
        if idx > 0:
            prev = tokens[idx - 1]
            if prev[:2] == ("ident", "fn"):
                continue  # a definition
            if prev[:2] == ("punct", "."):
                continue  # a method named `t`, not the lookup function
        where = f"{path.relative_to(ROOT)}:{line}"
        args = tokens[idx + 2 : idx + 7]
        kinds = [a[:2] if a[0] == "punct" else (a[0],) for a in args]
        if (
            len(kinds) >= 4
            and kinds[0] == ("str",)
            and kinds[1] == ("punct", ",")
            and kinds[2] == ("str",)
            and (
                kinds[3] == ("punct", ")")
                or (len(kinds) == 5 and kinds[3] == ("punct", ",") and kinds[4] == ("punct", ")"))
            )
        ):
            calls.append((args[0][1], args[2][1], where))
        else:
            problems.append(f"{where}: t(…) call whose arguments are not two string literals")
    return calls, problems


def source_strings():
    """All calls of the app sources, in file then line order."""
    calls, problems = [], []
    for path in sorted(SRC_DIR.glob("*.rs")):
        if path.name in SKIP:
            continue
        file_calls, file_problems = extract_calls(path)
        calls += file_calls
        problems += file_problems
    return calls, problems


def parse_tables(path):
    """{"ES": [(key, value, line), …], …} from the `pub static` slices."""
    tokens = tokenize(path.read_text(encoding="utf-8"), path)
    tables = {}
    idx = 0
    while idx < len(tokens):
        if tokens[idx][:2] != ("ident", "static") or idx + 1 >= len(tokens):
            idx += 1
            continue
        name = tokens[idx + 1][1]
        # Skip the type (`: &[(&str, &str)]`) up to the initializer.
        while idx < len(tokens) and tokens[idx][:2] != ("punct", "="):
            idx += 1
        while idx < len(tokens) and tokens[idx][:2] != ("punct", "["):
            idx += 1
        idx += 1
        entries = []
        while idx < len(tokens) and tokens[idx][:2] != ("punct", "]"):
            if tokens[idx][:2] == ("punct", ","):
                idx += 1
                continue
            shape = [tok[0] if tok[0] == "str" else tok[1] for tok in tokens[idx : idx + 5]]
            if shape[:4] == ["(", "str", ",", "str"] and shape[4:] == [")"]:
                entries.append((tokens[idx + 1][1], tokens[idx + 3][1], tokens[idx + 1][2]))
                idx += 5
            elif shape[:5] == ["(", "str", ",", "str", ","] and tokens[idx + 5][:2] == ("punct", ")"):
                entries.append((tokens[idx + 1][1], tokens[idx + 3][1], tokens[idx + 1][2]))
                idx += 6
            else:
                raise ParseError(
                    f"{path}:{tokens[idx][2]}: entry of {name} is not a (\"key\", \"value\") pair"
                )
        tables[name] = entries
    return tables


# Symbols that must survive translation unchanged: shortcut glyphs, arrows…
KEEP_SYMBOLS = "⌘⇧⌥⌃⏎↩⌫⎋⇥←→↑↓▾▸✓"


def style_notes(key, value):
    """Non-fatal oddities of one translation."""
    notes = []
    if not value:
        return ["empty translation"]
    if key.strip() and (
        len(key) - len(key.lstrip()) != len(value) - len(value.lstrip())
        or len(key) - len(key.rstrip()) != len(value) - len(value.rstrip())
    ):
        notes.append("leading/trailing whitespace differs from the English text")
    core_key, core_value = key.rstrip(), value.rstrip()
    for mark in ("…", ":", "?"):
        if core_key.endswith(mark) != core_value.endswith(mark):
            notes.append(f"final {mark!r} differs from the English text")
    if "..." in value:
        notes.append("three dots instead of the ellipsis character …")
    for symbol in KEEP_SYMBOLS:
        if key.count(symbol) != value.count(symbol):
            notes.append(f"symbol {symbol!r} differs from the English text")
    for placeholder in ("{}", "%s", "%d"):
        if key.count(placeholder) != value.count(placeholder):
            notes.append(f"placeholder {placeholder!r} differs from the English text")
    if key.count("\n") != value.count("\n"):
        notes.append("number of line breaks differs from the English text")
    return notes


def show(text):
    return json.dumps(text, ensure_ascii=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--list", action="store_true", help="print the unique English strings (JSON strings)")
    parser.add_argument("--pairs", action="store_true", help='print ["English", "Français", "file:line"] per unique string')
    parser.add_argument("--quiet", action="store_true", help="do not list unused keys and style notes")
    args = parser.parse_args()

    try:
        calls, problems = source_strings()
    except ParseError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    first = {}  # English -> (French, where), in order of first use
    french_variants = {}
    for en, fr, where in calls:
        first.setdefault(en, (fr, where))
        french_variants.setdefault(en, {}).setdefault(fr, where)

    if args.list or args.pairs:
        for en, (fr, where) in first.items():
            print(show(en) if args.list else json.dumps([en, fr, where], ensure_ascii=False))
        for problem in problems:
            print(f"warning: {problem}", file=sys.stderr)
        return 0

    try:
        tables = parse_tables(TABLES_FILE)
    except (ParseError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    failed = bool(problems)
    print(f"{len(calls)} t(…) calls, {len(first)} unique English strings in {SRC_DIR.relative_to(ROOT)}")
    for problem in problems:
        print(f"  UNREADABLE {problem}")

    # One English text standing for two French texts is one key for two
    # meanings: the other languages cannot tell them apart.
    ambiguous = {en: frs for en, frs in french_variants.items() if len(frs) > 1}
    if ambiguous and not args.quiet:
        print(f"\nnote: {len(ambiguous)} English string(s) used with several French texts (one key, several meanings):")
        for en, frs in ambiguous.items():
            print(f"  {show(en)}")
            for fr, where in frs.items():
                print(f"      {show(fr)}  ({where})")

    for code, name in LANGS:
        print(f"\n[{code}] ", end="")
        if name not in tables:
            print(f"table {name} not found in {TABLES_FILE.relative_to(ROOT)}")
            failed = True
            continue
        entries = tables[name]
        seen, duplicates = {}, []
        for key, value, line in entries:
            if key in seen:
                duplicates.append((key, seen[key][1], line))
            else:
                seen[key] = (value, line)
        missing = [en for en in first if en not in seen]
        unused = [(key, line) for key, (_, line) in seen.items() if key not in first]
        covered = len(first) - len(missing)
        print(
            f"{len(entries)} entries, {covered}/{len(first)} strings translated: "
            f"{len(missing)} missing, {len(duplicates)} duplicated, {len(unused)} unused"
        )
        for en in missing:
            print(f"  MISSING   {show(en)}  ({first[en][1]})")
        for key, line_a, line_b in duplicates:
            print(f"  DUPLICATE {show(key)}  (lines {line_a} and {line_b})")
        if not args.quiet:
            for key, line in unused:
                print(f"  unused    {show(key)}  (line {line})")
            for key, (value, line) in seen.items():
                for note in style_notes(key, value):
                    print(f"  note      {show(key)} -> {show(value)}: {note}  (line {line})")
        failed = failed or bool(missing) or bool(duplicates)

    print("\n" + ("FAILED: missing or duplicated translations" if failed else "OK: every string is translated in es, de and it"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
