//! Safe rendering of API search snippets.
//!
//! The API wraps matches in `<em>...</em>` and translation text may contain other
//! markup (e.g. `<sup foot_note=1>1</sup>`). We never inject API HTML; instead this
//! tokenizer turns it into plain-text segments flagged as highlighted or not.
//! Unknown tags are dropped (their text kept); `<sup>`, `<script>` and `<style>`
//! are dropped along with their content. Basic entities are decoded.

use crate::html::{decode_entities, is_html_tag};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub highlighted: bool,
}

/// Tags that mark a highlighted match.
const HIGHLIGHT_TAGS: &[&str] = &["em", "mark", "b", "strong"];
/// Tags whose content is dropped entirely.
const SKIP_TAGS: &[&str] = &["sup", "script", "style"];

struct Tag {
    name: String,
    closing: bool,
    self_closing: bool,
}

/// Parse the inside of `<...>` (without the angle brackets).
fn parse_tag(inner: &str) -> Option<Tag> {
    let inner = inner.trim();
    let (closing, rest) = match inner.strip_prefix('/') {
        Some(r) => (true, r.trim_start()),
        None => (false, inner),
    };
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if name.is_empty() || !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(Tag { name, closing, self_closing: inner.ends_with('/') })
}

/// Byte index of the `>` closing a tag that starts at `start` (the `<`),
/// honouring quoted attribute values.
fn tag_end(s: &str, start: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s[start + 1..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(start + 1 + i),
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

fn push(segments: &mut Vec<Segment>, raw: &str, highlighted: bool) {
    if raw.is_empty() {
        return;
    }
    let text = decode_entities(raw);
    match segments.last_mut() {
        Some(last) if last.highlighted == highlighted => last.text.push_str(&text),
        _ => segments.push(Segment { text, highlighted }),
    }
}

/// Split API HTML into plain-text segments.
pub fn segments(html: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut highlight_depth = 0usize;
    // Name of the tag whose content we are skipping, and its nesting depth.
    let mut skipping: Option<(String, usize)> = None;
    let mut i = 0;
    let mut text_start = 0;

    while let Some(rel) = html[i..].find('<') {
        let lt = i + rel;
        let Some(gt) = tag_end(html, lt) else {
            // A lone `<` is literal text.
            i = lt + 1;
            continue;
        };
        // Only real HTML element names are markup; `<ye>`-style bracketed words in a
        // translation stay as text.
        let Some(tag) = parse_tag(&html[lt + 1..gt]).filter(|t| is_html_tag(&t.name)) else {
            // Comments, doctypes or `a < b > c` text: drop `<!...>`, keep the rest as text.
            if html[lt + 1..].starts_with('!') {
                if skipping.is_none() {
                    push(&mut out, &html[text_start..lt], highlight_depth > 0);
                }
                text_start = gt + 1;
            }
            i = lt + 1;
            continue;
        };
        if skipping.is_none() {
            push(&mut out, &html[text_start..lt], highlight_depth > 0);
        }
        text_start = gt + 1;
        i = gt + 1;

        if let Some((name, depth)) = skipping.as_mut() {
            if *name == tag.name && !tag.self_closing {
                if tag.closing {
                    *depth -= 1;
                    if *depth == 0 {
                        skipping = None;
                    }
                } else {
                    *depth += 1;
                }
            }
            continue;
        }

        let name = tag.name.as_str();
        if SKIP_TAGS.contains(&name) {
            if !tag.closing && !tag.self_closing {
                skipping = Some((tag.name, 1));
            }
        } else if HIGHLIGHT_TAGS.contains(&name) {
            if tag.closing {
                highlight_depth = highlight_depth.saturating_sub(1);
            } else if !tag.self_closing {
                highlight_depth += 1;
            }
        } else if matches!(name, "br" | "p" | "div" | "li") {
            // Block-ish boundaries become a space so words don't run together.
            push(&mut out, " ", highlight_depth > 0);
        }
    }
    if skipping.is_none() {
        push(&mut out, &html[text_start..], highlight_depth > 0);
    }
    out
}

/// The words of a snippet (split at white space), each flagged if any part of
/// it is highlighted: what a renderer that draws word by word needs.
pub fn words(html: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut open = false;
    for segment in segments(html) {
        for c in segment.text.chars() {
            if c.is_whitespace() {
                open = false;
            } else if open && let Some((word, lit)) = out.last_mut() {
                word.push(c);
                *lit |= segment.highlighted;
            } else {
                out.push((c.to_string(), segment.highlighted));
                open = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, highlighted: bool) -> Segment {
        Segment { text: text.into(), highlighted }
    }

    #[test]
    fn plain() {
        assert_eq!(segments("hello world"), vec![seg("hello world", false)]);
        assert_eq!(segments(""), vec![]);
    }

    #[test]
    fn highlights() {
        assert_eq!(
            segments("the <em>light</em> of <em>Allah</em>"),
            vec![seg("the ", false), seg("light", true), seg(" of ", false), seg("Allah", true),]
        );
        assert_eq!(
            segments("<em class=\"hlt1\">نور</em> السماوات"),
            vec![seg("نور", true), seg(" السماوات", false)]
        );
        assert_eq!(segments("<EM>x</EM>"), vec![seg("x", true)]);
    }

    #[test]
    fn drops_footnotes_with_content() {
        assert_eq!(
            segments(
                "Allah<sup foot_note=123>1</sup> is the <em>Light</em><sup foot_note=\"9\">2</sup>."
            ),
            vec![seg("Allah is the ", false), seg("Light", true), seg(".", false)]
        );
        // nested sup
        assert_eq!(segments("a<sup>1<sup>2</sup>3</sup>b"), vec![seg("ab", false)]);
    }

    #[test]
    fn drops_unknown_tags_keeps_text() {
        assert_eq!(
            segments("<span class=\"x\">say <i>He</i> is</span> One"),
            vec![seg("say He is One", false)]
        );
        assert_eq!(segments("<a href=\"javascript:alert(1)\">x</a>"), vec![seg("x", false)]);
        assert_eq!(segments("<script>alert(1)</script>ok"), vec![seg("ok", false)]);
        assert_eq!(segments("a<!-- c -->b"), vec![seg("ab", false)]);
        assert_eq!(segments("one<br/>two"), vec![seg("one two", false)]);
    }

    #[test]
    fn attribute_with_gt() {
        assert_eq!(segments("<span title=\"a>b\">x</span>"), vec![seg("x", false)]);
    }

    #[test]
    fn literal_angle_brackets() {
        assert_eq!(segments("1 < 2 and 3 > 2"), vec![seg("1 < 2 and 3 > 2", false)]);
        assert_eq!(segments("a <"), vec![seg("a <", false)]);
    }

    #[test]
    fn unbalanced() {
        assert_eq!(segments("<em>open"), vec![seg("open", true)]);
        assert_eq!(segments("close</em> x"), vec![seg("close x", false)]);
        assert_eq!(segments("a<sup>never closed"), vec![seg("a", false)]);
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#x27;"),
            "a & b <c> \"d\" 'e'"
        );
        assert_eq!(decode_entities("&nbsp;"), "\u{a0}");
        assert_eq!(decode_entities("AT&T &unknown; & &#0; &#xZZ;"), "AT&T &unknown; & &#0; &#xZZ;");
        assert_eq!(decode_entities("&#1606;"), "ن");
        assert_eq!(
            segments("&lt;em&gt; is <em>&amp;</em>"),
            vec![seg("<em> is ", false), seg("&", true)]
        );
    }

    #[test]
    fn bracketed_words_kept() {
        assert_eq!(segments("O you <ye> who <em>believe</em>"), vec![seg("O you <ye> who ", false), seg("believe", true)]);
    }

    #[test]
    fn words_carry_their_highlight() {
        let w = words("one <em>two</em> thr<em>ee</em>  four<sup>1</sup>");
        let expect = [("one", false), ("two", true), ("three", true), ("four", false)];
        assert_eq!(w, expect.map(|(t, lit)| (t.to_string(), lit)));
        assert!(words("  ").is_empty());
        assert_eq!(words("<em>رَحْمَةً</em> لِّلْعَٰلَمِينَ").iter().map(|w| w.1).collect::<Vec<_>>(), [true, false]);
    }
}
