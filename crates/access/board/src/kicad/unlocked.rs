//! The unlocked wiring of a KiCad board file, which a router may route again.

use std::ops::Range;

/// The board file without its unlocked tracks, arcs and vias: what is left
/// for a router to route again. Zones and the copper the designer locked
/// stay; the rest of the text is unchanged.
pub fn without_unlocked_wiring(board: &str) -> String {
    let mut kept = String::with_capacity(board.len());
    let mut from = 0;
    for item in elements(board) {
        if is_unlocked_copper(&board[item.clone()]) {
            let indented = board[..item.start].trim_end_matches([' ', '\t']).len();
            kept.push_str(&board[from..indented.max(from)]);
            let rest = &board[item.end..];
            let line_end = ["\r\n", "\n"]
                .into_iter()
                .find(|end| rest.starts_with(end))
                .map_or(0, str::len);
            from = item.end + line_end;
        }
    }
    kept.push_str(&board[from..]);
    kept
}

fn is_unlocked_copper(item: &str) -> bool {
    let parts = elements(item);
    let keyword = parts.first().map(|k| &item[k.clone()]);
    matches!(keyword, Some("segment" | "arc" | "via"))
        && !parts.iter().any(|part| is_locked(&item[part.clone()]))
}

/// `locked`, as KiCad wrote it before version 7, or `(locked yes)`.
fn is_locked(part: &str) -> bool {
    if part == "locked" {
        return true;
    }
    let words: Vec<&str> = elements(part).into_iter().map(|w| &part[w]).collect();
    matches!(words.as_slice(), ["locked"] | ["locked", "yes"])
}

/// The spans of the elements directly inside the list that `text` starts
/// with: atoms, quoted strings and lists.
fn elements(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let (mut depth, mut list_start, mut atom_start) = (0usize, 0, None);
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let ends_atom = byte == b'(' || byte == b')' || byte == b'"' || byte.is_ascii_whitespace();
        if ends_atom && let Some(start) = atom_start.take() {
            spans.push(start..i);
        }
        match byte {
            b'"' => {
                let end = closing_quote(bytes, i);
                if depth == 1 {
                    spans.push(i..end + 1);
                }
                i = end;
            }
            b'(' => {
                depth += 1;
                if depth == 2 {
                    list_start = i;
                }
            }
            b')' => {
                if depth == 2 {
                    spans.push(list_start..i + 1);
                }
                if depth <= 1 {
                    break;
                }
                depth -= 1;
            }
            _ if depth == 1 && !ends_atom && atom_start.is_none() => atom_start = Some(i),
            _ => {}
        }
        i += 1;
    }
    spans
}

/// The index of the quote that closes the string opened at `open`.
fn closing_quote(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < bytes.len() && bytes[i] != b'"' {
        i += if bytes[i] == b'\\' { 2 } else { 1 };
    }
    i.min(bytes.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTED: &str = "(kicad_pcb\n\t(version 20260206)\n\
        \t(footprint \"SW\" (property \"Value\" \"(segment)\" (unlocked yes)) (pad \"1\" smd))\n\
        \t(segment\n\t\t(start 1 2)\n\t\t(end 3 4)\n\t\t(net \"COL1\")\n\t)\n\
        \t(segment (start 1 2) (end 3 4) (locked yes) (net \"COL1\"))\n\
        \t(arc (start 1 2) (mid 2 3) (end 3 4) (net \"locked\"))\n\
        \t(via locked (at 1 2) (net \"ROW1\"))\n\
        \t(via (at 5 6) (net \"ROW1\"))\n\
        \t(zone (net \"GND\") (polygon (pts (xy 0 0) (xy 9 0) (xy 9 9))))\n)\n";

    #[test]
    fn removes_unlocked_tracks_arcs_and_vias() {
        let stripped = without_unlocked_wiring(ROUTED);
        assert!(!stripped.contains("(end 3 4)\n"), "{stripped}");
        assert!(!stripped.contains("(arc"), "{stripped}");
        assert!(!stripped.contains("(at 5 6)"), "{stripped}");
    }

    #[test]
    fn keeps_locked_copper_zones_and_footprints_as_they_were() {
        assert_eq!(
            without_unlocked_wiring(ROUTED),
            "(kicad_pcb\n\t(version 20260206)\n\
            \t(footprint \"SW\" (property \"Value\" \"(segment)\" (unlocked yes)) (pad \"1\" smd))\n\
            \t(segment (start 1 2) (end 3 4) (locked yes) (net \"COL1\"))\n\
            \t(via locked (at 1 2) (net \"ROW1\"))\n\
            \t(zone (net \"GND\") (polygon (pts (xy 0 0) (xy 9 0) (xy 9 9))))\n)\n"
        );
    }

    #[test]
    fn a_board_without_unlocked_wiring_stays_as_it_was() {
        let board = "(kicad_pcb (version 1) (net \"a\\\"(\"))";
        assert_eq!(without_unlocked_wiring(board), board);
    }
}
