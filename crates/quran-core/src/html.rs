//! Tiny, safe HTML tokenizer for API-provided translation/footnote text.
//!
//! The API returns snippets such as `He is <i>the</i> One<sup foot_note=77232>1</sup>`.
//! We never inject that HTML into the DOM. Instead it is reduced to plain text
//! segments (optionally italic/bold), footnote markers and line breaks, which are
//! rendered as ordinary escaped text nodes. Unknown tags are dropped but their
//! inner text is kept; `<script>`/`<style>` content is dropped entirely.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextStyle {
    pub italic: bool,
    pub bold: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text { text: String, style: TextStyle },
    /// `<sup foot_note=ID>label</sup>`
    FootNote { id: u32, label: String },
    LineBreak,
}

/// Parsed start/end tag.
#[derive(Debug, PartialEq)]
struct Tag {
    name: String,
    closing: bool,
    self_closing: bool,
    attrs: Vec<(String, String)>,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// HTML element names we treat as markup. Anything else in angle brackets
/// (e.g. `<ye>`, `<<Allah>>`) is ordinary translation text.
pub fn is_html_tag(name: &str) -> bool {
    matches!(
        name,
        "a" | "abbr" | "b" | "bdi" | "bdo" | "big" | "blockquote" | "br" | "cite" | "code" | "dd" | "del"
            | "dfn" | "div" | "dl" | "dt" | "em" | "font" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "hr" | "i" | "img" | "ins" | "kbd" | "li" | "mark" | "ol" | "p" | "pre" | "q" | "s"
            | "samp" | "script" | "small" | "span" | "strike" | "strong" | "style" | "sub" | "sup"
            | "table" | "tbody" | "td" | "th" | "thead" | "tr" | "tt" | "u" | "ul" | "var" | "wbr"
    )
}

/// Split API HTML into safe segments.
pub fn tokenize(input: &str) -> Vec<Segment> {
    let mut out = Builder::default();
    let mut italic = 0u32;
    let mut bold = 0u32;
    // While inside a footnote `<sup>`: (id, label, nested sup depth).
    let mut footnote: Option<(u32, String, u32)> = None;
    // While inside <script>/<style>: name of the tag whose content is skipped.
    let mut skip_until: Option<String> = None;

    let mut rest = input;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            emit_text(&mut out, &mut footnote, &skip_until, rest, italic, bold);
            break;
        };
        if lt > 0 {
            emit_text(&mut out, &mut footnote, &skip_until, &rest[..lt], italic, bold);
            rest = &rest[lt..];
        }
        // `rest` starts with '<'.
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = match after.find("-->") {
                Some(end) => &after[end + 3..],
                None => "",
            };
            continue;
        }
        let Some((tag, len)) = parse_tag(rest).filter(|(t, _)| is_html_tag(&t.name)) else {
            // Not a tag (e.g. "a < b", or bracketed words like "<ye>" in a translation):
            // keep '<' as text so no words of the translation are lost.
            emit_text(&mut out, &mut footnote, &skip_until, "<", italic, bold);
            rest = &rest[1..];
            continue;
        };
        rest = &rest[len..];

        if let Some(skip) = &skip_until {
            if tag.closing && &tag.name == skip {
                skip_until = None;
            }
            continue;
        }
        if let Some((id, label, depth)) = &mut footnote {
            if tag.name == "sup" {
                if tag.closing {
                    if *depth == 0 {
                        let label = label.trim().to_string();
                        out.push(Segment::FootNote { id: *id, label });
                        footnote = None;
                    } else {
                        *depth -= 1;
                    }
                } else if !tag.self_closing {
                    *depth += 1;
                }
            }
            continue;
        }

        match (tag.name.as_str(), tag.closing) {
            ("script" | "style", false) if !tag.self_closing => skip_until = Some(tag.name.clone()),
            ("i" | "em", false) if !tag.self_closing => italic += 1,
            ("i" | "em", true) => italic = italic.saturating_sub(1),
            ("b" | "strong", false) if !tag.self_closing => bold += 1,
            ("b" | "strong", true) => bold = bold.saturating_sub(1),
            ("sup", false) => {
                if let Some(id) = tag.attr("foot_note").and_then(|v| v.trim().parse::<u32>().ok()) {
                    if tag.self_closing {
                        out.push(Segment::FootNote { id, label: "*".into() });
                    } else {
                        footnote = Some((id, String::new(), 0));
                    }
                }
            }
            ("br", _) => out.push(Segment::LineBreak),
            ("p" | "div" | "li" | "tr", true) => out.push(Segment::LineBreak),
            _ => {}
        }
    }
    // Unterminated footnote: still surface it.
    if let Some((id, label, _)) = footnote {
        out.push(Segment::FootNote { id, label: label.trim().to_string() });
    }
    out.finish()
}

/// Like [`tokenize`], but guarantees text never silently disappears: if the
/// tokenizer yields no visible text while the raw input has some, fall back to
/// the raw text with tags stripped.
pub fn display_segments(input: &str) -> Vec<Segment> {
    let segs = tokenize(input);
    let has_text = segs.iter().any(|s| match s {
        Segment::Text { text, .. } => !text.trim().is_empty(),
        _ => false,
    });
    if has_text || input.trim().is_empty() {
        return segs;
    }
    let stripped = strip_tags(input);
    if stripped.trim().is_empty() {
        return segs;
    }
    vec![Segment::Text { text: stripped.trim().to_string(), style: TextStyle::default() }]
}

/// Crude fallback: drop everything between `<` and the next `>`, decode entities.
pub fn strip_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' if !in_tag => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
}

fn emit_text(
    out: &mut Builder,
    footnote: &mut Option<(u32, String, u32)>,
    skip: &Option<String>,
    raw: &str,
    italic: u32,
    bold: u32,
) {
    if skip.is_some() {
        return;
    }
    let text = decode_entities(raw);
    if let Some((_, label, _)) = footnote {
        label.push_str(&text);
    } else {
        out.text(text, TextStyle { italic: italic > 0, bold: bold > 0 });
    }
}

#[derive(Default)]
struct Builder {
    segs: Vec<Segment>,
}

impl Builder {
    fn text(&mut self, text: String, style: TextStyle) {
        if text.is_empty() {
            return;
        }
        if let Some(Segment::Text { text: prev, style: prev_style }) = self.segs.last_mut()
            && *prev_style == style
        {
            prev.push_str(&text);
            return;
        }
        self.segs.push(Segment::Text { text, style });
    }

    fn push(&mut self, seg: Segment) {
        // Collapse repeated / leading line breaks.
        if seg == Segment::LineBreak && matches!(self.segs.last(), None | Some(Segment::LineBreak)) {
            return;
        }
        self.segs.push(seg);
    }

    fn finish(mut self) -> Vec<Segment> {
        while self.segs.last() == Some(&Segment::LineBreak) {
            self.segs.pop();
        }
        self.segs
    }
}

/// Parse a tag at the start of `s` (which begins with '<'). Returns the tag and
/// its byte length, or `None` if this isn't a well-formed tag.
fn parse_tag(s: &str) -> Option<(Tag, usize)> {
    let bytes = s.as_bytes();
    let mut i = 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
        i += 1;
    }
    if i == name_start || !bytes[name_start].is_ascii_alphabetic() {
        return None;
    }
    let name = s[name_start..i].to_ascii_lowercase();

    // Find the closing '>' outside quotes.
    let mut quote: Option<u8> = None;
    let mut end = None;
    for (j, &b) in bytes.iter().enumerate().skip(i) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => {
                end = Some(j);
                break;
            }
            None if b == b'<' => return None,
            None => {}
        }
    }
    let end = end?;
    let mut inner = s[i..end].trim_end();
    let self_closing = inner.ends_with('/');
    if self_closing {
        inner = &inner[..inner.len() - 1];
    }
    Some((Tag { name, closing, self_closing, attrs: parse_attrs(inner) }, end + 1))
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let body = &after[1..];
                let close = body.find(q).unwrap_or(body.len());
                value = decode_entities(&body[..close]);
                rest = body.get(close + 1..).unwrap_or("");
            } else {
                let close = after.find(char::is_whitespace).unwrap_or(after.len());
                value = decode_entities(&after[..close]);
                rest = &after[close..];
            }
        } else if name.is_empty() {
            // Stray character such as '/': skip it.
            let mut chars = rest.chars();
            chars.next();
            rest = chars.as_str();
        }
        if !name.is_empty() {
            attrs.push((name, value));
        }
        rest = rest.trim_start();
    }
    attrs
}

/// Decode common named and numeric character references. Unknown ones stay literal.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..]
            .find(';')
            .filter(|&semi| semi <= 10)
            .and_then(|semi| decode_entity(&rest[1..1 + semi]).map(|c| (c, semi + 2)));
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ndash" => '–',
        "mdash" => '—',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "hellip" => '…',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of `input` (footnote markers and styling dropped).
    fn plain_text(input: &str) -> String {
        tokenize(input)
            .into_iter()
            .map(|s| match s {
                Segment::Text { text, .. } => text,
                Segment::LineBreak => "\n".into(),
                Segment::FootNote { .. } => String::new(),
            })
            .collect()
    }

    fn t(text: &str) -> Segment {
        Segment::Text { text: text.into(), style: TextStyle::default() }
    }
    fn it(text: &str) -> Segment {
        Segment::Text { text: text.into(), style: TextStyle { italic: true, bold: false } }
    }
    fn fnote(id: u32, label: &str) -> Segment {
        Segment::FootNote { id, label: label.into() }
    }

    #[test]
    fn plain_text_passthrough() {
        assert_eq!(tokenize("In the name of Allah"), vec![t("In the name of Allah")]);
        assert_eq!(tokenize(""), vec![]);
    }

    #[test]
    fn api_example() {
        assert_eq!(
            tokenize("He is <i>the</i> One<sup foot_note=77232>1</sup>"),
            vec![t("He is "), it("the"), t(" One"), fnote(77232, "1")]
        );
    }

    #[test]
    fn footnote_attribute_quoting() {
        for html in [
            r#"x<sup foot_note="5">2</sup>"#,
            "x<sup foot_note='5'>2</sup>",
            "x<sup foot_note=5>2</sup>",
            "x<SUP class=f FOOT_NOTE = \"5\" >2</SUP>",
            "x<sup data-x='a>b' foot_note=5>2</sup>",
        ] {
            assert_eq!(tokenize(html), vec![t("x"), fnote(5, "2")], "{html}");
        }
        // sup without foot_note: inner text kept as normal text.
        assert_eq!(tokenize("a<sup>2</sup>"), vec![t("a2")]);
        // bad id: text kept.
        assert_eq!(tokenize("a<sup foot_note=abc>2</sup>"), vec![t("a2")]);
    }

    #[test]
    fn nested_and_unknown_tags() {
        assert_eq!(
            tokenize("<span class=\"x\"><a href='http://e.com'>link</a> <i>it <b>both</b></i></span>"),
            vec![
                t("link "),
                it("it "),
                Segment::Text { text: "both".into(), style: TextStyle { italic: true, bold: true } },
            ]
        );
        assert_eq!(
            tokenize("a<sup foot_note=1>x<sup>y</sup>z</sup>b"),
            vec![t("a"), fnote(1, "xyz"), t("b")]
        );
        // Stray closing tag doesn't underflow.
        assert_eq!(tokenize("</i>a</b>"), vec![t("a")]);
    }

    #[test]
    fn dangerous_content_never_survives_as_markup() {
        let segs = tokenize(r#"<img src=x onerror="alert(1)">ok<script>alert(2)</script><style>p{}</style>!"#);
        assert_eq!(segs, vec![t("ok!")]);
        assert_eq!(tokenize("<!-- hidden -->shown"), vec![t("shown")]);
        // Escaped markup stays as literal text (rendered escaped by Dioxus).
        assert_eq!(tokenize("&lt;b&gt;x&lt;/b&gt;"), vec![t("<b>x</b>")]);
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("&amp; &quot;q&quot; &#39;s&#39; &#x41;&#66; &apos; &nbsp;&unknown; & a&b; &#0;"),
            "& \"q\" 's' AB ' \u{a0}&unknown; & a&b; &#0;"
        );
        assert_eq!(tokenize("Tom &amp; Jerry"), vec![t("Tom & Jerry")]);
        // Unquoted value decodes to `"9"`, which isn't a valid id: plain text.
        assert_eq!(tokenize("<sup foot_note=&quot;9&quot;>1</sup>"), vec![t("1")]);
        assert_eq!(tokenize(r#"<sup foot_note="&#57;">1</sup>"#), vec![fnote(9, "1")]);
    }

    #[test]
    fn malformed_input() {
        assert_eq!(tokenize("a < b > c"), vec![t("a < b > c")]);
        assert_eq!(tokenize("1<2"), vec![t("1<2")]);
        assert_eq!(tokenize("x <i unterminated"), vec![t("x <i unterminated")]);
        assert_eq!(tokenize("x<sup foot_note=3>1"), vec![t("x"), fnote(3, "1")]);
        assert_eq!(tokenize("<!-- never closed"), vec![]);
        assert_eq!(tokenize("<>"), vec![t("<>")]);
        assert_eq!(tokenize("مرحبا <i>عربي</i>"), vec![t("مرحبا "), it("عربي")]);
    }

    #[test]
    fn display_fallback_never_loses_text() {
        // Normal input: identical to tokenize.
        let html = "He is <i>the</i> One<sup foot_note=1>1</sup>";
        assert_eq!(display_segments(html), tokenize(html));
        // Text only inside content the tokenizer drops → raw text with tags stripped.
        assert_eq!(display_segments("<script>Allah is One</script>"), vec![t("Allah is One")]);
        assert_eq!(display_segments("<style>Light &amp; guidance</style>"), vec![t("Light & guidance")]);
        // Text only inside a footnote marker.
        assert_eq!(display_segments("<sup foot_note=4>Mercy</sup>"), vec![t("Mercy")]);
        // Nothing but tags/whitespace: no fallback text invented.
        assert_eq!(display_segments("<br><p> </p>"), tokenize("<br><p> </p>"));
        assert_eq!(plain_text("<br><p> </p>").trim(), "");
        assert_eq!(display_segments(""), vec![]);
        assert_eq!(strip_tags("a<b x='1'>b</b>&lt;c"), "ab<c");
    }

    #[test]
    fn line_breaks() {
        assert_eq!(
            tokenize("<p>one</p><p>two</p><br/><br>"),
            vec![t("one"), Segment::LineBreak, t("two")]
        );
        assert_eq!(plain_text("a<br>b <i>c</i>"), "a\nb c");
    }

    #[test]
    fn bracketed_words_are_text_not_tags() {
        let text = |input: &str| -> String {
            tokenize(input)
                .into_iter()
                .filter_map(|s| match s {
                    Segment::Text { text, .. } => Some(text),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(text("O you <ye> who believe"), "O you <ye> who believe");
        assert_eq!(text("<<Allah>> is"), "<<Allah>> is");
        assert_eq!(text("He is <i>the</i> One"), "He is the One");
    }
}
