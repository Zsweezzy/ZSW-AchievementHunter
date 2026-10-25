use std::path::PathBuf;

pub fn path(appid: u32, api_name: &str) -> PathBuf {
    let safe: String = api_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    crate::paths::pages_dir()
        .join(appid.to_string())
        .join(format!("{safe}.html"))
}

pub fn load(appid: u32, api_name: &str) -> String {
    std::fs::read_to_string(path(appid, api_name)).unwrap_or_default()
}

pub fn exists(appid: u32, api_name: &str) -> bool {
    path(appid, api_name).exists()
}

pub fn save(appid: u32, api_name: &str, html: &str) -> std::io::Result<PathBuf> {
    let path = path(appid, api_name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, html)?;
    Ok(path)
}

pub fn template(achievement: &str) -> String {
    format!(
        "<h1>{achievement}</h1>\n<p>Write your guide here.</p>\n<ul>\n<li>Step one</li>\n\
         <li>Step two</li>\n</ul>\n<p><img src=\"screenshot.png\" alt=\"screenshot\"></p>\n"
    )
}

#[derive(Clone, Debug, PartialEq)]
pub enum Span {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link(String, String),
}

impl Span {
    fn text(&self) -> &str {
        match self {
            Span::Text(text)
            | Span::Bold(text)
            | Span::Italic(text)
            | Span::Code(text)
            | Span::Link(text, _) => text,
        }
    }

    fn push(&mut self, more: &str) {
        match self {
            Span::Text(text)
            | Span::Bold(text)
            | Span::Italic(text)
            | Span::Code(text)
            | Span::Link(text, _) => text.push_str(more),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(Vec<Span>),
    Paragraph(Vec<Span>),
    Item(Vec<Span>),
    Quote(Vec<Span>),
    Code(String),
    Rule,
    Image(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Paragraph,
    Heading,
    Item,
    Quote,
}

#[derive(Clone, PartialEq)]
enum Inline {
    Bold,
    Italic,
    Code,
    Link(String),
}

pub fn parse(html: &str) -> Vec<Block> {
    let html = strip_element(&strip_element(html, "script"), "style");
    let mut blocks = Vec::new();
    let mut spans = Vec::new();
    let mut kind = Kind::Paragraph;
    let mut inline: Vec<Inline> = Vec::new();
    let mut space = false;
    let mut tokens = tokens(&html).into_iter().peekable();

    while let Some(token) = tokens.next() {
        match token {
            Tok::Text(text) => {
                let text = crate::providers::decode_entities(&text);
                let lead = text.starts_with(char::is_whitespace);
                let trail = text.ends_with(char::is_whitespace);
                let core = collapse(&text);
                if core.is_empty() {
                    space |= lead || trail;
                    continue;
                }
                if (space || lead) && !spans.is_empty() {
                    push(&mut spans, &inline, " ");
                }
                push(&mut spans, &inline, &core);
                space = trail;
            }
            Tok::Open(name, attrs) => match name.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush(&mut blocks, &mut spans, kind);
                    kind = Kind::Heading;
                }
                "p" | "div" | "section" | "article" | "header" | "footer" | "tr" | "td" | "th"
                | "ul" | "ol" | "table" => {
                    flush(&mut blocks, &mut spans, kind);
                }
                "blockquote" => {
                    flush(&mut blocks, &mut spans, kind);
                    kind = Kind::Quote;
                }
                "li" => {
                    flush(&mut blocks, &mut spans, kind);
                    kind = Kind::Item;
                }
                "br" => {
                    flush(&mut blocks, &mut spans, kind);
                    space = false;
                }
                "hr" => {
                    flush(&mut blocks, &mut spans, kind);
                    blocks.push(Block::Rule);
                }
                "img" => {
                    flush(&mut blocks, &mut spans, kind);
                    if let Some(src) = attr(&attrs, "src").filter(|src| !src.trim().is_empty()) {
                        blocks.push(Block::Image(src));
                    }
                }
                "pre" => {
                    flush(&mut blocks, &mut spans, kind);
                    let mut raw = String::new();
                    for token in tokens.by_ref() {
                        match token {
                            Tok::Close(name) if name == "pre" => break,
                            Tok::Text(text) => raw.push_str(&text),
                            Tok::Open(name, _) => raw.push_str(&format!("<{name}>")),
                            Tok::Close(name) => raw.push_str(&format!("</{name}>")),
                        }
                    }
                    let raw = crate::providers::decode_entities(raw.trim_matches('\n'));
                    if !raw.trim().is_empty() {
                        blocks.push(Block::Code(dedent(&raw)));
                    }
                }
                "b" | "strong" => inline.push(Inline::Bold),
                "i" | "em" => inline.push(Inline::Italic),
                "code" => inline.push(Inline::Code),
                "a" => inline.push(Inline::Link(attr(&attrs, "href").unwrap_or_default())),
                _ => {}
            },
            Tok::Close(name) => match name.as_str() {
                "p" | "div" | "section" | "article" | "header" | "footer" | "tr" | "td" | "th"
                | "ul" | "ol" | "table" | "blockquote" | "li" | "h1" | "h2" | "h3" | "h4"
                | "h5" | "h6" => {
                    flush(&mut blocks, &mut spans, kind);
                    kind = Kind::Paragraph;
                }
                "b" | "strong" => pop_matching(&mut inline, Inline::Bold),
                "i" | "em" => pop_matching(&mut inline, Inline::Italic),
                "code" => pop_matching(&mut inline, Inline::Code),
                "a" => {
                    if matches!(inline.last(), Some(Inline::Link(_))) {
                        inline.pop();
                    }
                }
                _ => {}
            },
        }
    }
    flush(&mut blocks, &mut spans, kind);
    blocks
}

fn pop_matching(inline: &mut Vec<Inline>, want: Inline) {
    if inline.last() == Some(&want) {
        inline.pop();
    } else if let Some(at) = inline.iter().rposition(|item| *item == want) {
        inline.remove(at);
    }
}

fn push(spans: &mut Vec<Span>, inline: &[Inline], text: &str) {
    let span = match inline.last() {
        Some(Inline::Bold) => Span::Bold(text.to_owned()),
        Some(Inline::Italic) => Span::Italic(text.to_owned()),
        Some(Inline::Code) => Span::Code(text.to_owned()),
        Some(Inline::Link(href)) => Span::Link(text.to_owned(), href.clone()),
        None => Span::Text(text.to_owned()),
    };
    match spans.last_mut() {
        Some(last) if std::mem::discriminant(last) == std::mem::discriminant(&span) => {
            last.push(text)
        }
        _ => spans.push(span),
    }
}

fn flush(blocks: &mut Vec<Block>, spans: &mut Vec<Span>, kind: Kind) {
    if let Some(last) = spans.last_mut()
        && let Span::Text(text)
        | Span::Bold(text)
        | Span::Italic(text)
        | Span::Code(text)
        | Span::Link(text, _) = last
    {
        text.truncate(text.trim_end().len());
    }
    if spans.iter().any(|span| !span.text().trim().is_empty()) {
        let taken = std::mem::take(spans);
        blocks.push(match kind {
            Kind::Heading => Block::Heading(taken),
            Kind::Paragraph => Block::Paragraph(taken),
            Kind::Item => Block::Item(taken),
            Kind::Quote => Block::Quote(taken),
        });
    } else {
        spans.clear();
    }
}

fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() || ch == '\u{a0}' {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(ch);
    }
    out
}

fn dedent(text: &str) -> String {
    let indent = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|line| line.get(indent..).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_element(html: &str, element: &str) -> String {
    let open = format!("<{element}");
    let close = format!("</{element}>");
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.to_ascii_lowercase().find(&open) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        match tail.to_ascii_lowercase().find(&close) {
            Some(end) => rest = &tail[end + close.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn tokens(html: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        if start > 0 {
            out.push(Tok::Text(rest[..start].to_owned()));
        }
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            out.push(Tok::Text(tail.to_owned()));
            return out;
        };
        let raw = &tail[1..end];
        rest = &tail[end + 1..];
        if raw.starts_with('!') || raw.starts_with('?') {
            continue;
        }
        if let Some(name) = raw.strip_prefix('/') {
            out.push(Tok::Close(name.trim().to_ascii_lowercase()));
        } else {
            let name_end = raw
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(raw.len());
            out.push(Tok::Open(
                raw[..name_end].to_ascii_lowercase(),
                raw[name_end..].to_owned(),
            ));
        }
    }
    if !rest.is_empty() {
        out.push(Tok::Text(rest.to_owned()));
    }
    out
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&format!("{name}=")) {
        let at = from + at;
        let boundary = at == 0 || attrs[..at].ends_with(char::is_whitespace);
        if boundary {
            let rest = attrs[at + name.len() + 1..].trim_start();
            let (quote, rest) = match rest.chars().next()? {
                quote @ ('"' | '\'') => (Some(quote), &rest[1..]),
                _ => (None, rest),
            };
            let end = match quote {
                Some(quote) => rest.find(quote)?,
                None => rest.find(char::is_whitespace).unwrap_or(rest.len()),
            };
            return Some(crate::providers::decode_entities(&rest[..end]));
        }
        from = at + name.len() + 1;
    }
    None
}

enum Tok {
    Text(String),
    Open(String, String),
    Close(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(block: &Block) -> String {
        let spans = match block {
            Block::Heading(spans)
            | Block::Paragraph(spans)
            | Block::Item(spans)
            | Block::Quote(spans) => spans,
            Block::Code(text) => return text.clone(),
            Block::Rule => return "<hr>".to_owned(),
            Block::Image(src) => return format!("<img {src}>"),
        };
        spans.iter().map(|span| span.text()).collect()
    }

    #[test]
    fn crucial_page_renders_the_documented_subset() {
        let blocks = parse(
            "<h1>Title</h1><p>one <b>two</b> three</p><ul><li>a</li><li>b</li></ul><hr>\
             <p><img src=\"shot.png\" alt=\"x\"></p><pre>  code\n  more</pre>",
        );
        let shapes: Vec<String> = blocks.iter().map(plain).collect();
        assert_eq!(
            shapes,
            vec![
                "Title",
                "one two three",
                "a",
                "b",
                "<hr>",
                "<img shot.png>",
                "code\nmore"
            ]
        );
        assert!(matches!(blocks[0], Block::Heading(_)));
        assert!(matches!(blocks[1], Block::Paragraph(_)));
        assert!(
            matches!(blocks[1], Block::Paragraph(ref spans) if spans.iter().any(|s| matches!(s, Span::Bold(t) if t.contains("two"))))
        );
    }

    #[test]
    fn crucial_page_ignores_scripts_styles_and_unknown_tags() {
        let blocks = parse(
            "<style>p{color:red}</style><script>alert('<h1>x</h1>')</script>\
             <div class=\"x\"><span>kept</span></div>",
        );
        assert_eq!(blocks.iter().map(plain).collect::<Vec<_>>(), vec!["kept"]);
    }

    #[test]
    fn crucial_page_decodes_entities_and_keeps_links_as_spans() {
        let blocks = parse("<p>Tom &amp; Jerry <a href=\"https://x/y?a=1&amp;b=2\">link</a></p>");
        let spans = match &blocks[0] {
            Block::Paragraph(spans) => spans,
            other => panic!("{other:?}"),
        };
        let text: String = spans.iter().map(|span| span.text()).collect();
        assert_eq!(text, "Tom & Jerry link");
        assert!(spans.iter().any(|span| matches!(
            span,
            Span::Link(link, href) if link.contains("link") && href == "https://x/y?a=1&b=2"
        )));
    }

    #[test]
    fn crucial_page_saves_under_the_data_dir() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };
        assert!(!exists(1, "A/B"));
        let saved = save(1, "A/B", "<p>hi</p>").unwrap();
        assert!(saved.ends_with("pages/1/A_B.html"), "{saved:?}");
        assert_eq!(load(1, "A/B"), "<p>hi</p>");
        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_page_keeps_line_breaks_inside_pre_and_headings_separate() {
        let blocks = parse("<pre>\n  a\n  b\n</pre><h2>x</h2><p>y</p>");
        assert_eq!(blocks.len(), 3);
        assert_eq!(plain(&blocks[0]), "a\nb");
    }
}
