//! Markdown till HTML för systemdokumentationen. Bara det som dokumentet
//! använder: rubriker, stycken, punktlistor i två nivåer, tabeller, `kod`
//! och **fetstil**. All text escapas; rå HTML i texten visas som text.

use crate::html::esc;

/// Text med `kod` och **fetstil**.
fn inline(text: &str) -> String {
    let mut out = String::new();
    let parts: Vec<&str> = text.split('`').collect();
    // Med ett udda antal backticks stänger den sista ingen kod.
    let unpaired = parts.len().is_multiple_of(2);
    for (i, part) in parts.iter().enumerate() {
        if unpaired && i == parts.len() - 1 {
            out.push('`');
        } else if i % 2 == 1 {
            out.push_str("<code>");
            out.push_str(&esc(part));
            out.push_str("</code>");
            continue;
        }
        let pieces: Vec<&str> = part.split("**").collect();
        let pairs = (pieces.len() - 1) / 2 * 2;
        for (j, piece) in pieces.iter().enumerate() {
            if j > 0 {
                out.push_str(match j {
                    j if j > pairs => "**",
                    j if j % 2 == 1 => "<strong>",
                    _ => "</strong>",
                });
            }
            out.push_str(&esc(piece));
        }
    }
    out
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if (1..=3).contains(&level) {
        line[level..].strip_prefix(' ').map(|t| (level, t))
    } else {
        None
    }
}

/// Cellerna i en tabellrad. `\|` är ett lodstreck i texten.
fn cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = line.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    cells.push(cur);
    // Raden börjar och slutar med `|`, så första och sista cellen är tomma.
    if cells.first().is_some_and(|c| c.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|c| c.trim().is_empty()) {
        cells.pop();
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

fn is_separator(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':')))
}

fn table(rows: &[&str], out: &mut String) {
    out.push_str("<div class=\"rull\"><table>\n");
    for (i, row) in rows.iter().enumerate() {
        let cells = cells(row);
        if i == 1 && is_separator(&cells) {
            continue;
        }
        let tag = if i == 0 { "th" } else { "td" };
        out.push_str("<tr>");
        for c in &cells {
            out.push_str(&format!("<{tag}>{}</{tag}>", inline(c)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table></div>\n");
}

fn list(items: &[(usize, &str)], out: &mut String) {
    let base = items.iter().map(|(i, _)| *i).min().unwrap_or(0);
    out.push_str("<ul>\n");
    let mut k = 0;
    while k < items.len() {
        let (_, text) = items[k];
        out.push_str("<li>");
        out.push_str(&inline(text));
        let start = k + 1;
        let mut end = start;
        while end < items.len() && items[end].0 > base {
            end += 1;
        }
        if end > start {
            out.push('\n');
            list(&items[start..end], out);
        }
        out.push_str("</li>\n");
        k = end;
    }
    out.push_str("</ul>\n");
}

pub(crate) fn to_html(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = String::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut String| {
        if !para.is_empty() {
            out.push_str(&format!("<p>{}</p>\n", inline(&para.join(" "))));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if t.is_empty() {
            flush(&mut para, &mut out);
            i += 1;
        } else if let Some((level, text)) = heading(t) {
            flush(&mut para, &mut out);
            out.push_str(&format!("<h{level}>{}</h{level}>\n", inline(text)));
            i += 1;
        } else if t.starts_with('|') {
            flush(&mut para, &mut out);
            let start = i;
            while i < lines.len() && lines[i].trim().starts_with('|') {
                i += 1;
            }
            table(&lines[start..i], &mut out);
        } else if t.starts_with("- ") {
            flush(&mut para, &mut out);
            let mut items = Vec::new();
            while i < lines.len() {
                let l = lines[i];
                let Some(text) = l.trim_start().strip_prefix("- ") else {
                    break;
                };
                items.push((l.len() - l.trim_start().len(), text));
                i += 1;
            }
            list(&items, &mut out);
        } else {
            para.push(t);
            i += 1;
        }
    }
    flush(&mut para, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_subset_and_escapes_the_rest() {
        let md = "# Rubrik\n\nEtt stycke med `kod` och **fet** text\nsom fortsätter.\n\n\
                  | A | B |\n| --- | --- |\n| `x` | a \\| b <script> |\n\n- ett\n  - under\n- två\n";
        let html = to_html(md);
        assert!(html.contains("<h1>Rubrik</h1>"));
        assert!(html.contains(
            "<p>Ett stycke med <code>kod</code> och <strong>fet</strong> text som fortsätter.</p>"
        ));
        assert!(html.contains("<tr><th>A</th><th>B</th></tr>"));
        assert!(html.contains("<td>a | b &lt;script&gt;</td>"));
        assert!(!html.contains("---"));
        assert!(html.contains("<li>ett\n<ul>\n<li>under</li>\n</ul>\n</li>\n<li>två</li>"));
    }

    #[test]
    fn unpaired_markers_stay_text() {
        assert_eq!(inline("a ** b"), "a ** b");
        assert_eq!(inline("**a** **b"), "<strong>a</strong> **b");
        assert_eq!(inline("a `b` c `d"), "a <code>b</code> c `d");
    }
}
