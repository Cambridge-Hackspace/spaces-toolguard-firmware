//! The pin table in `docs/INSTALLING.md` is checked against the firmware.
//!
//! That document is what somebody follows with a soldering iron, and a pin
//! table is exactly the kind of thing that drifts: the source moves a signal,
//! the table does not, and the next person wires a relay to the I²C clock. The
//! failure is expensive and silent until the smoke.
//!
//! So the mapping is written twice on purpose -- once in the firmware as a
//! `cfg`-gated tuple, once in the document as a table -- and this compares
//! them. Deriving the table from the source would make the check agree with
//! itself; the duplication **is** the check.
//!
//! **Why it lives in `toolguard-core` and reads `toolguard-fw` as text.** The
//! pins are concrete `esp-idf-hal` types behind plain trait bounds, so the
//! firmware crate only compiles for an ESP target and nothing that runs on a
//! host can link against it. A text comparison is what is available, and it is
//! enough: the question is whether two written-down mappings agree.

use std::collections::BTreeMap;
use std::path::PathBuf;

const CHIPS: [&str; 3] = ["esp32c6", "esp32c3", "esp32s3"];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is this package; the repository is its parent.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("toolguard-core must have a parent directory")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// `signal -> pin number`, from the `cfg`-gated tuple for one chip.
///
/// Anchored on the `#[cfg(<chip>)]` attribute and closed at the `);` that ends
/// the value list, so a second tuple elsewhere in the file cannot be read by
/// accident.
fn from_source(src: &str, chip: &str) -> BTreeMap<String, u32> {
    let attr = format!("#[cfg({chip})]");
    let start = src
        .find(&attr)
        .unwrap_or_else(|| panic!("no `{attr}` in the firmware; the pin map moved"));
    let rest = &src[start..];
    let end = rest
        .find(");")
        .unwrap_or_else(|| panic!("the tuple after `{attr}` is unterminated"));
    let block = &rest[..end];

    let names_open = block.find("let (").expect("no `let (` after the cfg") + "let (".len();
    let names_close = block[names_open..]
        .find(')')
        .expect("the name list is unterminated")
        + names_open;
    let names: Vec<String> = block[names_open..names_close]
        .split(',')
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        // The firmware calls two of them `*_pin`; the document drops the
        // suffix, because a table column headed "light_pin" reads oddly to
        // somebody holding a board.
        .map(|n| n.trim_end_matches("_pin").to_string())
        .collect();

    let values: Vec<u32> = block[names_close..]
        .split("pins.gpio")
        .skip(1)
        .map(|chunk| {
            let digits: String = chunk.chars().take_while(char::is_ascii_digit).collect();
            digits
                .parse()
                .unwrap_or_else(|_| panic!("could not read a pin number from {chunk:?}"))
        })
        .collect();

    assert_eq!(
        names.len(),
        values.len(),
        "{chip}: {} names but {} pins -- the tuple and its value list disagree, \
         which is a firmware bug and not a documentation one",
        names.len(),
        values.len()
    );
    assert!(
        names.len() >= 9,
        "{chip}: only {} pins parsed; the tuple shape changed and this parser \
         is now reading something else",
        names.len()
    );
    names.into_iter().zip(values).collect()
}

/// `signal -> pin number`, from the markdown table for one chip.
fn from_doc(doc: &str, chip: &str) -> BTreeMap<String, u32> {
    let marker = "<!-- PINMAP:";
    let start = doc
        .find(marker)
        .expect("the pin table's PINMAP marker is gone; this check cannot find its table");
    let table = &doc[start..];

    let header = table
        .lines()
        .find(|l| l.starts_with('|') && l.contains("signal"))
        .expect("no header row after the PINMAP marker");
    let column = header
        .split('|')
        .map(str::trim)
        .position(|c| c == chip)
        .unwrap_or_else(|| panic!("the table has no `{chip}` column"));

    let mut out = BTreeMap::new();
    for line in table.lines().skip_while(|l| !std::ptr::eq(*l, header)) {
        if !line.starts_with('|') {
            // Past the end of the table.
            if !out.is_empty() {
                break;
            }
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let Some(signal) = cells.get(1) else { continue };
        // Skip the header and the `|---|` separator.
        if *signal == "signal" || signal.starts_with("---") || signal.is_empty() {
            continue;
        }
        let Some(cell) = cells.get(column) else {
            continue;
        };
        if let Ok(pin) = cell.parse::<u32>() {
            out.insert(signal.to_string(), pin);
        }
    }
    out
}

#[test]
fn the_documented_pin_table_matches_the_firmware() {
    let src = read("toolguard-fw/src/main.rs");
    let doc = read("docs/INSTALLING.md");

    for chip in CHIPS {
        let from_code = from_source(&src, chip);
        let documented = from_doc(&doc, chip);
        assert!(
            !documented.is_empty(),
            "{chip}: parsed no pins out of the document, so the comparison \
             below would pass over an empty set"
        );
        assert_eq!(
            documented, from_code,
            "{chip}: the pin table in docs/INSTALLING.md and the tuple in \
             toolguard-fw/src/main.rs disagree.\n  documented: {documented:?}\n  \
             firmware:   {from_code:?}\n\nSomebody follows that table with a \
             soldering iron."
        );
    }
}

/// Both parsers must be capable of disagreeing, or the comparison above is two
/// empty maps matching.
#[test]
fn the_parsers_report_only_what_they_are_given() {
    let src = "\
    #[cfg(esp32c6)]
    let (light_pin, setup_pin, sclk) = (
        pins.gpio8,
        pins.gpio9,
        pins.gpio0,
    );";
    // Nine is the real floor; this fixture is deliberately shorter, so the
    // shape assertion has to be the thing that rejects it.
    let short = std::panic::catch_unwind(|| from_source(src, "esp32c6"));
    assert!(
        short.is_err(),
        "a three-pin tuple was accepted, so a parser that found only part of \
         the real map would pass silently"
    );

    let doc = "\
<!-- PINMAP: x -->

| signal | esp32c6 | esp32c3 | esp32s3 |
|---|---|---|---|
| light | 8 | 7 | 48 |
| relay | 19 | 3 | 16 |
";
    let c6 = from_doc(doc, "esp32c6");
    assert_eq!(c6.get("light"), Some(&8));
    assert_eq!(c6.get("relay"), Some(&19));
    // And it reads the right COLUMN, which a parser that always took the first
    // number would get wrong on exactly one chip.
    assert_eq!(
        from_doc(doc, "esp32c3").get("light"),
        Some(&7),
        "the column lookup is not per chip"
    );
    assert_eq!(from_doc(doc, "esp32s3").get("light"), Some(&48));
}

/// And a transposed pin must fail, which is the defect this exists for.
#[test]
fn a_transposed_pin_is_caught() {
    let src = read("toolguard-fw/src/main.rs");
    let from_code = from_source(&src, "esp32c6");
    let doc = read("docs/INSTALLING.md");
    let mut documented = from_doc(&doc, "esp32c6");

    let sda = *documented.get("sda").expect("sda should be documented");
    let scl = *documented.get("scl").expect("scl should be documented");
    documented.insert("sda".into(), scl);
    documented.insert("scl".into(), sda);
    assert_ne!(
        documented, from_code,
        "swapping SDA and SCL in the documented table still compared equal, so \
         this check would not notice the table drifting"
    );
}
