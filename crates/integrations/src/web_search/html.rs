//! Just enough HTML reading for search result pages.
//!
//! The workspace has no HTML parser and free search reads three small, stable
//! layouts, so this is a tag scanner rather than a DOM: callers cut the page
//! into one block per result first, then read each field inside its own
//! block. A snippet can therefore never be paired with another result's title,
//! which was the main bug of the regex scraper this replaces.

/// One element found by [`find`]: its attribute text and inner HTML.
pub(super) struct Element<'a> {
    attrs: &'a str,
    pub(super) inner: &'a str,
}

impl Element<'_> {
    pub(super) fn attr(&self, name: &str) -> Option<String> {
        attr(self.attrs, name)
    }

    pub(super) fn text(&self) -> String {
        text(self.inner)
    }
}

/// Split `html` into blocks that each start at an element whose class list
/// contains `class` (any `<tag>` when `class` is empty). Text before the first
/// block is dropped.
pub(super) fn blocks<'a>(html: &'a str, tag: &str, class: &str) -> Vec<&'a str> {
    let starts: Vec<usize> = tag_starts(html, tag)
        .filter(|&(start, end)| class.is_empty() || has_class(&html[start..end], class))
        .map(|(start, _)| start)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(html.len());
            &html[start..end]
        })
        .collect()
}

/// First `<tag>` in `html` whose class list contains `class` (any `<tag>` when
/// `class` is empty), with its inner HTML up to the next `</tag>`.
pub(super) fn find<'a>(html: &'a str, tag: &str, class: &str) -> Option<Element<'a>> {
    let lower = html.to_ascii_lowercase();
    tag_starts(html, tag)
        .find(|&(start, end)| class.is_empty() || has_class(&html[start..end], class))
        .map(|(start, end)| {
            let close = format!("</{tag}");
            let inner_end = lower[end..]
                .find(&close)
                .map_or(html.len(), |offset| end + offset);
            Element {
                attrs: &html[start + 1 + tag.len()..end.saturating_sub(1)],
                inner: &html[end..inner_end],
            }
        })
}

/// Every `<tag ...>` opening tag as `(start, end)` byte offsets, where `end`
/// is just past the closing `>`.
fn tag_starts<'a>(html: &'a str, tag: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{tag}");
    let mut from = 0;
    std::iter::from_fn(move || {
        loop {
            let start = from + lower[from..].find(&needle)?;
            let after = start + needle.len();
            from = after;
            // `<a` must not match `<abbr`.
            match lower.as_bytes().get(after) {
                Some(b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') => {}
                _ => continue,
            }
            let end = after + html[after..].find('>')? + 1;
            from = end;
            return Some((start, end));
        }
    })
}

fn has_class(open_tag: &str, class: &str) -> bool {
    attr(open_tag, "class")
        .is_some_and(|classes| classes.split_whitespace().any(|token| token == class))
}

/// Value of attribute `name` in an opening tag or attribute string.
fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let bytes = attrs.as_bytes();
    let mut from = 0;
    while let Some(offset) = lower[from..].find(name) {
        let start = from + offset;
        from = start + name.len();
        let preceded = start == 0 || bytes[start - 1].is_ascii_whitespace();
        let rest = attrs[from..].trim_start();
        if !preceded || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let (quote, body) = match value.chars().next()? {
            q @ ('"' | '\'') => (Some(q), &value[1..]),
            _ => (None, value),
        };
        let end = match quote {
            Some(q) => body.find(q)?,
            None => body
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(body.len()),
        };
        return Some(decode_entities(&body[..end]));
    }
    None
}

/// Visible text of an HTML fragment: tags removed, entities decoded,
/// whitespace collapsed.
pub(super) fn text(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = decode_entities(&out);
    let mut collapsed = String::with_capacity(decoded.len());
    for word in decoded.split_whitespace() {
        if !collapsed.is_empty() && !matches!(word, "," | "." | ";" | ":" | ")") {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    collapsed
}

pub(super) fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "rdquo" => Some('”'),
            "ldquo" => Some('“'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_elements_by_class_token_inside_blocks() {
        let html = r#"<div class="result x">
            <a class="result__a" href="/one?a=1&amp;b=2">One <b>bold</b></a>
            <a class='result__snippet'>first &amp; snippet</a></div>
            <div class="result">
            <a class="result__a" href="/two">Two</a></div>"#;
        let blocks = blocks(html, "div", "result");
        assert_eq!(blocks.len(), 2);
        let title = find(blocks[0], "a", "result__a").unwrap();
        assert_eq!(title.attr("href").unwrap(), "/one?a=1&b=2");
        assert_eq!(title.text(), "One bold");
        assert_eq!(
            find(blocks[0], "a", "result__snippet").unwrap().text(),
            "first & snippet"
        );
        // The second block has no snippet; it must not borrow the first one.
        assert!(find(blocks[1], "a", "result__snippet").is_none());
    }

    #[test]
    fn class_match_is_by_token_and_tag_by_name() {
        let html = r#"<abbr class="result">x</abbr><a class="result--ad">y</a>"#;
        assert!(find(html, "a", "result").is_none());
        assert!(find(html, "a", "result--ad").is_some());
    }

    #[test]
    fn decodes_numeric_and_named_entities() {
        assert_eq!(
            decode_entities("&#39;x&#x27; &lt;y&gt; &unknown; a&b"),
            "'x' <y> &unknown; a&b"
        );
    }
}
