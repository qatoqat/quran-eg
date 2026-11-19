//! Plain text of a verse: what "Copy" puts on the clipboard, and translation
//! text without its markup.

use crate::api::Verse;

/// Plain-text form of a verse for the clipboard.
pub fn copy_text(verse: &Verse) -> String {
    let arabic = verse
        .text_uthmani
        .clone()
        .or_else(|| verse.text_indopak.clone())
        .or_else(|| verse.text_imlaei.clone())
        .unwrap_or_else(|| {
            verse
                .words
                .iter()
                .filter(|w| !w.is_end_marker())
                .filter_map(|w| {
                    w.text_uthmani.clone().or_else(|| w.text_indopak.clone()).or_else(|| w.text.clone())
                })
                .collect::<Vec<_>>()
                .join(" ")
        });
    let mut out = arabic.trim().to_string();
    for t in &verse.translations {
        out.push_str("\n\n");
        out.push_str(&strip_html(&t.text));
        if let Some(name) = t.resource_name.as_deref().filter(|n| !n.is_empty()) {
            out.push_str(" — ");
            out.push_str(name);
        }
    }
    out.push_str(&format!("\n\n({})", verse.verse_key));
    out
}

/// Remove HTML tags from translation text. Footnote markers
/// (`<sup foot_note=1>1</sup>`) are dropped together with their content.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut sup_depth = 0usize;
    while let Some(lt) = rest.find('<') {
        if sup_depth == 0 {
            out.push_str(&rest[..lt]);
        }
        let Some(gt) = rest[lt..].find('>') else {
            // Unterminated tag: keep the text as-is.
            if sup_depth == 0 {
                out.push_str(&rest[lt..]);
            }
            rest = "";
            break;
        };
        let tag = rest[lt + 1..lt + gt].trim().to_ascii_lowercase();
        if tag == "sup" || tag.starts_with("sup ") {
            sup_depth += 1;
        } else if tag == "/sup" {
            sup_depth = sup_depth.saturating_sub(1);
        }
        rest = &rest[lt + gt + 1..];
    }
    if sup_depth == 0 {
        out.push_str(rest);
    }
    decode_entities(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_footnotes() {
        assert_eq!(
            strip_html("Allah<sup foot_note=123>1</sup> — there is no deity <i>except</i> Him"),
            "Allah — there is no deity except Him"
        );
        assert_eq!(strip_html("a &amp; b&nbsp;c"), "a & b c");
        assert_eq!(strip_html("plain"), "plain");
        assert_eq!(strip_html("x < y"), "x < y");
        assert_eq!(strip_html("<p>one</p>\n<p>two</p>"), "one two");
    }

    #[test]
    fn copy_text_format() {
        let verse: Verse = serde_json::from_str(
            r#"{"id":1,"verse_number":1,"verse_key":"1:1","text_uthmani":"بِسْمِ",
                "translations":[{"resource_id":131,"text":"In the Name<sup foot_note=1>1</sup>",
                "resource_name":"Dr. Mustafa Khattab"}]}"#,
        )
        .unwrap();
        assert_eq!(copy_text(&verse), "بِسْمِ\n\nIn the Name — Dr. Mustafa Khattab\n\n(1:1)");
    }

    #[test]
    fn copy_text_from_words() {
        let verse: Verse = serde_json::from_str(
            r#"{"id":1,"verse_number":1,"verse_key":"1:1","words":[
                {"id":1,"position":1,"char_type_name":"word","text_uthmani":"a"},
                {"id":2,"position":2,"char_type_name":"word","text_uthmani":"b"},
                {"id":3,"position":3,"char_type_name":"end","text_uthmani":"١"}]}"#,
        )
        .unwrap();
        assert_eq!(copy_text(&verse), "a b\n\n(1:1)");
    }
}
