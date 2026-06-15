//! HTML conversion utilities

use crate::types::{PageLink, PageMetadata};

/// Check if content-type indicates markdown (e.g. `text/markdown`).
pub fn is_markdown_content_type(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .and_then(|ct| ct.split(';').next())
        .map(|media_type| media_type.trim().eq_ignore_ascii_case("text/markdown"))
        .unwrap_or(false)
}

/// Check if content-type indicates plain text (e.g. `text/plain`).
pub fn is_plain_text_content_type(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .and_then(|ct| ct.split(';').next())
        .map(|media_type| media_type.trim().eq_ignore_ascii_case("text/plain"))
        .unwrap_or(false)
}

/// Check if content is HTML based on content type and body
///
/// Returns `true` if the content type contains `text/html` or `application/xhtml`,
/// or if the body starts with `<!DOCTYPE` or `<html`.
pub fn is_html(content_type: &Option<String>, body: &str) -> bool {
    // Check Content-Type
    if let Some(ct) = content_type {
        let ct_lower = ct.to_lowercase();
        if ct_lower.contains("text/html") || ct_lower.contains("application/xhtml") {
            return true;
        }
    }

    // Check body start
    let trimmed = body.trim_start();
    trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html")
}

/// Convert HTML to markdown
///
/// Converts common HTML elements (headings, lists, emphasis, code blocks, links,
/// blockquotes) to their Markdown equivalents. Strips script, style, noscript,
/// iframe, and svg elements. Decodes HTML entities.
///
/// # Examples
///
/// ```
/// use fetchkit::html_to_markdown;
///
/// let html = "<h1>Title</h1><p><strong>Bold</strong> text</p>";
/// let md = html_to_markdown(html);
/// assert!(md.contains("# Title"));
/// assert!(md.contains("**Bold**"));
/// ```
pub fn html_to_markdown(html: &str) -> String {
    let mut output = String::new();
    let mut in_skip_element = 0;
    let mut skip_elements: Vec<String> = Vec::new();
    let mut in_pre = false;
    let mut in_blockquote = false;

    // Link tracking: when we see <a href="...">, save href and record the output
    // position. On </a>, wrap the text collected since then in [text](href).
    let mut link_href: Option<String> = None;
    let mut link_start: usize = 0;

    // List tracking: stack of list types (true=ordered, false=unordered) with item counter
    let mut list_stack: Vec<(bool, usize)> = Vec::new();

    // Table tracking
    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut in_cell = false;
    let mut cell_buf = String::new();
    let mut is_header_row = false;

    let mut chars = html.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' {
            // Parse tag
            let mut tag = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            let tag_lower = tag.to_lowercase();
            let is_closing = tag_lower.starts_with('/');
            let tag_name = if is_closing {
                tag_lower[1..].split_whitespace().next().unwrap_or("")
            } else {
                tag_lower.split_whitespace().next().unwrap_or("")
            };

            // THREAT[TM-CONV-001]: Strip script/style/iframe/svg to prevent injection
            let skip_tags = ["script", "style", "noscript", "iframe", "svg"];
            if skip_tags.contains(&tag_name) {
                if is_closing {
                    if let Some(pos) = skip_elements.iter().rposition(|t| t == tag_name) {
                        skip_elements.remove(pos);
                        in_skip_element = skip_elements.len();
                    }
                } else if !tag.ends_with('/') {
                    skip_elements.push(tag_name.to_string());
                    in_skip_element = skip_elements.len();
                }
                continue;
            }

            if in_skip_element > 0 {
                continue;
            }

            // Handle markdown conversion
            match tag_name {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = tag_name[1..].parse::<usize>().unwrap_or(1);
                    if !is_closing {
                        output.push('\n');
                        for _ in 0..level {
                            output.push('#');
                        }
                        output.push(' ');
                    } else {
                        output.push_str("\n\n");
                    }
                }
                "p" | "div" | "section" | "article" | "main" | "header" | "footer"
                    if is_closing =>
                {
                    output.push_str("\n\n");
                }
                "br" => {
                    output.push('\n');
                }
                "hr" => {
                    output.push_str("\n---\n");
                }
                "ul" => {
                    if is_closing {
                        list_stack.pop();
                        if list_stack.is_empty() {
                            output.push('\n');
                        }
                    } else {
                        list_stack.push((false, 0));
                    }
                }
                "ol" => {
                    if is_closing {
                        list_stack.pop();
                        if list_stack.is_empty() {
                            output.push('\n');
                        }
                    } else {
                        list_stack.push((true, 0));
                    }
                }
                "li" if !is_closing => {
                    output.push('\n');
                    let depth = list_stack.len().saturating_sub(1);
                    for _ in 0..depth {
                        output.push_str("  ");
                    }
                    if let Some((is_ordered, counter)) = list_stack.last_mut() {
                        if *is_ordered {
                            *counter += 1;
                            output.push_str(&format!("{}. ", *counter));
                        } else {
                            output.push_str("- ");
                        }
                    } else {
                        output.push_str("- ");
                    }
                }
                "strong" | "b" => {
                    output.push_str("**");
                }
                "em" | "i" => {
                    output.push('*');
                }
                "pre" => {
                    if !is_closing {
                        output.push_str("\n```\n");
                        in_pre = true;
                    } else {
                        output.push_str("\n```\n");
                        in_pre = false;
                    }
                }
                "code" if !in_pre => {
                    output.push('`');
                }
                "blockquote" => {
                    if !is_closing {
                        in_blockquote = true;
                        output.push_str("\n> ");
                    } else {
                        in_blockquote = false;
                        output.push('\n');
                    }
                }
                "a" => {
                    if !is_closing {
                        if let Some(href) = extract_attribute(&tag, "href") {
                            if !href.is_empty() {
                                link_href = Some(href);
                                link_start = output.len();
                            }
                        }
                    } else if let Some(href) = link_href.take() {
                        let text = output[link_start..].trim().to_string();
                        output.truncate(link_start);
                        if text.is_empty() {
                            output.push_str(&format!("<{}>", href));
                        } else {
                            output.push_str(&format!("[{}]({})", text, href));
                        }
                    }
                }
                "img" if !is_closing => {
                    let alt = extract_attribute(&tag, "alt").unwrap_or_default();
                    if let Some(src) = extract_attribute(&tag, "src") {
                        output.push_str(&format!("![{}]({})", alt, src));
                    }
                }
                // Table handling
                "table" => {
                    if !is_closing {
                        in_table = true;
                        table_rows.clear();
                    } else {
                        in_table = false;
                        render_table(&table_rows, &mut output);
                        table_rows.clear();
                    }
                }
                "tr" => {
                    if !is_closing {
                        current_row.clear();
                        is_header_row = false;
                    } else if in_table {
                        table_rows.push(current_row.clone());
                        if is_header_row && table_rows.len() == 1 {
                            let sep: Vec<String> =
                                current_row.iter().map(|_| "---".to_string()).collect();
                            table_rows.push(sep);
                        }
                        current_row.clear();
                    }
                }
                "th" => {
                    if !is_closing {
                        in_cell = true;
                        cell_buf.clear();
                        is_header_row = true;
                    } else {
                        in_cell = false;
                        current_row.push(cell_buf.trim().to_string());
                        cell_buf.clear();
                    }
                }
                "td" => {
                    if !is_closing {
                        in_cell = true;
                        cell_buf.clear();
                    } else {
                        in_cell = false;
                        current_row.push(cell_buf.trim().to_string());
                        cell_buf.clear();
                    }
                }
                // Definition lists
                "dl" if is_closing => {
                    output.push_str("\n\n");
                }
                "dt" => {
                    if !is_closing {
                        output.push_str("\n**");
                    } else {
                        output.push_str("**\n");
                    }
                }
                "dd" => {
                    if !is_closing {
                        output.push_str(": ");
                    } else {
                        output.push('\n');
                    }
                }
                _ => {}
            }
        } else if in_skip_element == 0 {
            // Text content
            let decoded = decode_entity(c, &mut chars);

            if in_cell {
                cell_buf.push(decoded);
            } else if in_table {
                // Ignore text outside cells but inside table
            } else if in_blockquote && decoded == '\n' {
                output.push_str("\n> ");
            } else {
                output.push(decoded);
            }
        }
    }

    clean_whitespace(&output)
}

/// Render collected table rows as a markdown table.
fn render_table(rows: &[Vec<String>], output: &mut String) {
    if rows.is_empty() {
        return;
    }

    output.push('\n');
    for row in rows {
        output.push_str("| ");
        output.push_str(&row.join(" | "));
        output.push_str(" |\n");
    }
}

/// Convert HTML to plain text
///
/// Strips all HTML tags and returns plain text content. Handles newlines
/// for block elements (p, div, headings). Decodes HTML entities.
///
/// # Examples
///
/// ```
/// use fetchkit::html_to_text;
///
/// let html = "<h1>Title</h1><p>Paragraph with &amp; entity</p>";
/// let text = html_to_text(html);
/// assert!(text.contains("Title"));
/// assert!(text.contains("Paragraph with & entity"));
/// ```
pub fn html_to_text(html: &str) -> String {
    let mut output = String::new();
    let mut in_skip_element = 0;
    let mut skip_elements: Vec<String> = Vec::new();

    let mut chars = html.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' {
            // Parse tag
            let mut tag = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            let tag_lower = tag.to_lowercase();
            let is_closing = tag_lower.starts_with('/');
            let tag_name = if is_closing {
                tag_lower[1..].split_whitespace().next().unwrap_or("")
            } else {
                tag_lower.split_whitespace().next().unwrap_or("")
            };

            // THREAT[TM-CONV-001]: Strip script/style/iframe/svg to prevent injection
            let skip_tags = ["script", "style", "noscript", "iframe", "svg"];
            if skip_tags.contains(&tag_name) {
                if is_closing {
                    if let Some(pos) = skip_elements.iter().rposition(|t| t == tag_name) {
                        skip_elements.remove(pos);
                        in_skip_element = skip_elements.len();
                    }
                } else if !tag.ends_with('/') {
                    skip_elements.push(tag_name.to_string());
                    in_skip_element = skip_elements.len();
                }
                continue;
            }

            if in_skip_element > 0 {
                continue;
            }

            // Handle newline-inducing elements
            let newline_tags = [
                "p", "div", "br", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr",
            ];
            if newline_tags.contains(&tag_name) && (is_closing || tag_name == "br") {
                output.push('\n');
            } else if newline_tags.contains(&tag_name) && !is_closing {
                // Opening tags like h1-h6, p, etc. also add newline
                if matches!(tag_name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p") {
                    output.push('\n');
                }
            }
        } else if in_skip_element == 0 {
            // Text content
            let decoded = decode_entity(c, &mut chars);
            output.push(decoded);
        }
    }

    clean_whitespace(&output)
}

/// Extract attribute value from tag
fn extract_attribute(tag: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=", attr);
    let start = tag.char_indices().find_map(|(idx, _)| {
        tag.get(idx..idx + pattern.len())
            .filter(|candidate| candidate.eq_ignore_ascii_case(&pattern))
            .map(|_| idx)
    });

    if let Some(start) = start {
        let rest = &tag[start + pattern.len()..];
        let rest = rest.trim_start();

        if let Some(rest) = rest.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                return Some(rest[..end].to_string());
            }
        } else if let Some(rest) = rest.strip_prefix('\'') {
            if let Some(end) = rest.find('\'') {
                return Some(rest[..end].to_string());
            }
        } else {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Decode HTML entity starting from ampersand
// THREAT[TM-CONV-004]: Limited named-entity set; rejects long/unknown sequences
fn decode_entity(c: char, chars: &mut std::iter::Peekable<std::str::Chars>) -> char {
    if c != '&' {
        return c;
    }

    let mut entity = String::new();
    while let Some(&next) = chars.peek() {
        if next == ';' {
            chars.next();
            break;
        }
        if next.is_whitespace() || entity.len() > 10 {
            // Not a valid entity
            return '&';
        }
        entity.push(chars.next().unwrap());
    }

    match entity.as_str() {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" | "#39" => '\'',
        "nbsp" => ' ',
        "mdash" => '—',
        "ndash" => '–',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "bull" => '•',
        "hellip" => '…',
        "laquo" => '«',
        "raquo" => '»',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201C}',
        "rdquo" => '\u{201D}',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "deg" => '°',
        "micro" => 'µ',
        "para" => '¶',
        "sect" => '§',
        "middot" => '·',
        "times" => '×',
        "divide" => '÷',
        "plusmn" => '±',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        _ => {
            // Check for numeric entities
            if let Some(num_str) = entity.strip_prefix('#') {
                if let Some(stripped) = num_str.strip_prefix('x') {
                    // Hex entity
                    if let Ok(code) = u32::from_str_radix(stripped, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            return ch;
                        }
                    }
                } else if let Ok(code) = num_str.parse::<u32>() {
                    if let Some(ch) = char::from_u32(code) {
                        return ch;
                    }
                }
            }
            // Unknown entity - return original
            '&'
        }
    }
}

/// Clean whitespace: collapse runs, trim, keep max 2 newlines.
/// Preserves indentation (spaces after newlines) for list nesting.
pub fn clean_whitespace(s: &str) -> String {
    let mut result = String::new();
    let mut last_was_space = false;
    let mut newline_count = 0;
    let mut at_line_start = true;

    for c in s.chars() {
        if c == '\n' {
            // Remove trailing space before newline
            if last_was_space && result.ends_with(' ') {
                result.pop();
            }
            newline_count += 1;
            last_was_space = true;
            at_line_start = true;
            if newline_count <= 2 {
                result.push(c);
            }
        } else if c == ' ' || c == '\t' {
            if at_line_start {
                // Preserve indentation at line start
                result.push(c);
            } else {
                newline_count = 0;
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            }
        } else if c.is_whitespace() {
            newline_count = 0;
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            newline_count = 0;
            last_was_space = false;
            at_line_start = false;
            result.push(c);
        }
    }

    result.trim().to_string()
}

/// Filter excessive newlines: keep at most 2 consecutive newlines
pub fn filter_excessive_newlines(s: &str) -> String {
    let mut result = String::new();
    let mut newline_count = 0;

    for c in s.chars() {
        if c == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(c);
            }
        } else {
            newline_count = 0;
            result.push(c);
        }
    }

    result
}

/// Extract structured metadata from HTML in a single pass.
///
/// Extracts title, description, language, canonical URL, author,
/// published/modified dates, links, and heading outline from HTML.
///
/// # Examples
///
/// ```
/// use fetchkit::{extract_metadata, extract_headings};
///
/// let html = r#"<html lang="en"><head><title>Hello</title></head><body><h1>World</h1></body></html>"#;
/// let mut meta = extract_metadata(html);
/// meta.headings = extract_headings(html);
/// assert_eq!(meta.title.as_deref(), Some("Hello"));
/// assert_eq!(meta.language.as_deref(), Some("en"));
/// assert_eq!(meta.headings, vec!["# World"]);
/// ```
pub fn extract_metadata(html: &str) -> PageMetadata {
    let mut meta = PageMetadata::default();
    let mut chars = html.chars().peekable();
    let mut in_title = false;
    let mut title_buf = String::new();
    let mut in_skip_element = 0;
    let mut skip_elements: Vec<String> = Vec::new();
    // Track current <a> href for link extraction
    let mut current_link_href: Option<String> = None;
    let mut current_link_text = String::new();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            let tag_lower = tag.to_lowercase();
            let is_closing = tag_lower.starts_with('/');
            let tag_name = if is_closing {
                tag_lower[1..].split_whitespace().next().unwrap_or("")
            } else {
                tag_lower.split_whitespace().next().unwrap_or("")
            };

            // Skip dangerous elements
            let skip_tags = ["script", "style", "noscript", "iframe", "svg"];
            if skip_tags.contains(&tag_name) {
                if is_closing {
                    if let Some(pos) = skip_elements.iter().rposition(|t| t == tag_name) {
                        skip_elements.remove(pos);
                        in_skip_element = skip_elements.len();
                    }
                } else if !tag.ends_with('/') {
                    skip_elements.push(tag_name.to_string());
                    in_skip_element = skip_elements.len();
                }
                continue;
            }

            if in_skip_element > 0 {
                continue;
            }

            match tag_name {
                "html" if !is_closing => {
                    if let Some(lang) = extract_attribute(&tag, "lang") {
                        if meta.language.is_none() && !lang.is_empty() {
                            meta.language = Some(lang);
                        }
                    }
                }
                "title" => {
                    if !is_closing {
                        in_title = true;
                        title_buf.clear();
                    } else {
                        in_title = false;
                        let title = title_buf.trim().to_string();
                        if meta.title.is_none() && !title.is_empty() {
                            meta.title = Some(title);
                        }
                    }
                }
                "meta" if !is_closing => {
                    extract_meta_tag(&tag, &mut meta);
                }
                "link" if !is_closing => {
                    if let Some(rel) = extract_attribute(&tag, "rel") {
                        if rel == "canonical" {
                            if let Some(href) = extract_attribute(&tag, "href") {
                                if meta.canonical_url.is_none() && !href.is_empty() {
                                    meta.canonical_url = Some(href);
                                }
                            }
                        }
                    }
                }
                "time" if !is_closing => {
                    if let Some(datetime) = extract_attribute(&tag, "datetime") {
                        if meta.published_date.is_none() && !datetime.is_empty() {
                            meta.published_date = Some(datetime);
                        }
                    }
                }
                "a" => {
                    if !is_closing {
                        if let Some(href) = extract_attribute(&tag, "href") {
                            if !href.is_empty() {
                                current_link_href = Some(href);
                                current_link_text.clear();
                            }
                        }
                    } else if let Some(href) = current_link_href.take() {
                        let text = current_link_text.trim().to_string();
                        // Cap links at 500 to prevent DoS on link-heavy pages
                        if meta.links.len() < 500 {
                            meta.links.push(PageLink { text, href });
                        }
                        current_link_text.clear();
                    }
                }
                _ => {}
            }
        } else if in_skip_element == 0 {
            let decoded = decode_entity(c, &mut chars);
            if in_title {
                title_buf.push(decoded);
            }
            if current_link_href.is_some() {
                current_link_text.push(decoded);
            }
        }
    }

    meta
}

/// Second pass specifically for heading extraction (cheap — headings are sparse).
/// Called after the main metadata extraction to keep the main function clean.
pub fn extract_headings(html: &str) -> Vec<String> {
    let mut headings = Vec::new();
    let mut chars = html.chars().peekable();
    let mut in_heading: Option<u8> = None; // heading level 1-6
    let mut heading_buf = String::new();
    let mut in_skip_element = 0;
    let mut skip_elements: Vec<String> = Vec::new();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            let tag_lower = tag.to_lowercase();
            let is_closing = tag_lower.starts_with('/');
            let tag_name = if is_closing {
                tag_lower[1..].split_whitespace().next().unwrap_or("")
            } else {
                tag_lower.split_whitespace().next().unwrap_or("")
            };

            let skip_tags = ["script", "style", "noscript", "iframe", "svg"];
            if skip_tags.contains(&tag_name) {
                if is_closing {
                    if let Some(pos) = skip_elements.iter().rposition(|t| t == tag_name) {
                        skip_elements.remove(pos);
                        in_skip_element = skip_elements.len();
                    }
                } else if !tag.ends_with('/') {
                    skip_elements.push(tag_name.to_string());
                    in_skip_element = skip_elements.len();
                }
                continue;
            }

            if in_skip_element > 0 {
                continue;
            }

            if let Some(level) = heading_level(tag_name) {
                if is_closing {
                    if in_heading == Some(level) {
                        let text = heading_buf.trim().to_string();
                        if !text.is_empty() && headings.len() < 200 {
                            let prefix = "#".repeat(level as usize);
                            headings.push(format!("{} {}", prefix, text));
                        }
                        in_heading = None;
                        heading_buf.clear();
                    }
                } else {
                    in_heading = Some(level);
                    heading_buf.clear();
                }
            }
        } else if in_skip_element == 0 {
            let decoded = decode_entity(c, &mut chars);
            if in_heading.is_some() {
                heading_buf.push(decoded);
            }
        }
    }

    headings
}

fn heading_level(tag_name: &str) -> Option<u8> {
    match tag_name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// Extract metadata from a `<meta>` tag.
fn extract_meta_tag(tag: &str, meta: &mut PageMetadata) {
    // <meta name="..." content="...">
    if let Some(content) = extract_attribute(tag, "content") {
        if content.is_empty() {
            return;
        }
        // Check name attribute
        if let Some(name) = extract_attribute(tag, "name") {
            match name.to_lowercase().as_str() {
                "description" if meta.description.is_none() => {
                    meta.description = Some(content.clone());
                }
                "author" if meta.author.is_none() => {
                    meta.author = Some(content.clone());
                }
                _ => {}
            }
        }
        // Check property attribute (Open Graph)
        if let Some(property) = extract_attribute(tag, "property") {
            match property.to_lowercase().as_str() {
                "og:title" => {
                    // og:title overrides <title>
                    meta.title = Some(content.clone());
                }
                "og:description" => {
                    // og:description overrides <meta description>
                    meta.description = Some(content.clone());
                }
                "article:published_time" if meta.published_date.is_none() => {
                    meta.published_date = Some(content.clone());
                }
                "article:modified_time" if meta.modified_date.is_none() => {
                    meta.modified_date = Some(content);
                }
                _ => {}
            }
        }
    }
}

/// Strip boilerplate elements from HTML, keeping only main content.
///
/// Removes `<nav>`, `<footer>`, `<aside>`, and elements with
/// `role="navigation"`, `role="banner"`, `role="contentinfo"`.
/// If `<main>` or `<article>` is present, extracts only their content.
///
/// # Examples
///
/// ```
/// use fetchkit::strip_boilerplate;
///
/// let html = r#"<nav>Menu</nav><main><p>Content</p></main><footer>Footer</footer>"#;
/// let result = strip_boilerplate(html);
/// assert!(result.contains("Content"));
/// assert!(!result.contains("Menu"));
/// assert!(!result.contains("Footer"));
/// ```
pub fn strip_boilerplate(html: &str) -> String {
    // Strategy: if <main> or <article> exists, extract just that content.
    // Otherwise, strip known boilerplate elements.

    // Check if there's a <main> or <article> to focus on
    if let Some(focused) = extract_main_content(html) {
        return focused;
    }

    // Fallback: strip boilerplate elements
    strip_boilerplate_elements(html)
}

/// Extract content from `<main>` or `<article>` tag if present.
fn extract_main_content(html: &str) -> Option<String> {
    // Try <main> first, then <article>
    for target_tag in &["main", "article"] {
        if let Some(content) = extract_tag_content(html, target_tag) {
            return Some(content);
        }
    }

    // Try role="main"
    extract_role_content(html, "main")
}

/// Extract the inner content of the first occurrence of a given tag.
fn extract_tag_content(html: &str, target: &str) -> Option<String> {
    let mut chars = html.chars().peekable();
    let mut depth = 0i32;
    let mut capturing = false;
    let mut output = String::new();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            let tag_lower = tag.to_lowercase();
            let is_closing = tag_lower.starts_with('/');
            let tag_name = if is_closing {
                tag_lower[1..].split_whitespace().next().unwrap_or("")
            } else {
                tag_lower.split_whitespace().next().unwrap_or("")
            };

            if tag_name == target {
                if is_closing {
                    depth -= 1;
                    if depth == 0 && capturing {
                        return Some(output);
                    }
                } else if !tag.ends_with('/') {
                    depth += 1;
                    if depth == 1 && !capturing {
                        capturing = true;
                        continue;
                    }
                }
            }

            if capturing {
                output.push('<');
                output.push_str(&tag);
                output.push('>');
            }
        } else if capturing {
            output.push(c);
        }
    }

    None
}

/// Extract content of the first element with a given role attribute.
fn extract_role_content(html: &str, role: &str) -> Option<String> {
    let mut chars = html.chars().peekable();
    let mut capture_tag: Option<String> = None;
    let mut depth = 0i32;
    let mut output = String::new();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            let tag_lower = tag.to_lowercase();
            let is_closing = tag_lower.starts_with('/');
            let tag_name = if is_closing {
                tag_lower[1..].split_whitespace().next().unwrap_or("")
            } else {
                tag_lower.split_whitespace().next().unwrap_or("")
            };

            if let Some(ref target) = capture_tag {
                if tag_name == target.as_str() {
                    if is_closing {
                        depth -= 1;
                        if depth == 0 {
                            return Some(output);
                        }
                    } else if !tag.ends_with('/') {
                        depth += 1;
                    }
                }

                if depth > 0 {
                    output.push('<');
                    output.push_str(&tag);
                    output.push('>');
                }
            } else if !is_closing {
                // Check for role attribute
                if let Some(attr_role) = extract_attribute(&tag, "role") {
                    if attr_role.eq_ignore_ascii_case(role) && !tag.ends_with('/') {
                        capture_tag = Some(tag_name.to_string());
                        depth = 1;
                        continue;
                    }
                }
            }
        } else if capture_tag.is_some() && depth > 0 {
            output.push(c);
        }
    }

    None
}

/// Boilerplate tags to strip when no <main>/<article> found.
const BOILERPLATE_TAGS: &[&str] = &["nav", "footer", "aside", "header"];

/// Roles that indicate boilerplate.
const BOILERPLATE_ROLES: &[&str] = &["navigation", "banner", "contentinfo", "complementary"];

/// Strip known boilerplate elements from HTML.
fn strip_boilerplate_elements(html: &str) -> String {
    let mut output = String::new();
    let mut chars = html.chars().peekable();
    let mut skip_depth = 0i32;
    let mut skip_tag: Option<String> = None;

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                tag.push(chars.next().unwrap());
            }

            let tag_lower = tag.to_lowercase();
            let is_closing = tag_lower.starts_with('/');
            let tag_name = if is_closing {
                tag_lower[1..].split_whitespace().next().unwrap_or("")
            } else {
                tag_lower.split_whitespace().next().unwrap_or("")
            };

            // Track skip state
            if let Some(ref target) = skip_tag {
                if tag_name == target.as_str() {
                    if is_closing {
                        skip_depth -= 1;
                        if skip_depth == 0 {
                            skip_tag = None;
                            continue;
                        }
                    } else if !tag.ends_with('/') {
                        skip_depth += 1;
                    }
                }
                continue; // Skip everything inside boilerplate
            }

            // Check if this tag should be skipped
            if !is_closing && !tag.ends_with('/') {
                let is_boilerplate_tag = BOILERPLATE_TAGS.contains(&tag_name);
                let is_boilerplate_role = extract_attribute(&tag, "role")
                    .map(|r| {
                        BOILERPLATE_ROLES
                            .iter()
                            .any(|br| r.eq_ignore_ascii_case(br))
                    })
                    .unwrap_or(false);

                if is_boilerplate_tag || is_boilerplate_role {
                    skip_tag = Some(tag_name.to_string());
                    skip_depth = 1;
                    continue;
                }
            }

            output.push('<');
            output.push_str(&tag);
            output.push('>');
        } else if skip_tag.is_none() {
            output.push(c);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_html_by_content_type() {
        assert!(is_html(&Some("text/html".to_string()), ""));
        assert!(is_html(&Some("text/html; charset=utf-8".to_string()), ""));
        assert!(is_html(&Some("application/xhtml+xml".to_string()), ""));
        assert!(!is_html(&Some("text/plain".to_string()), ""));
        assert!(!is_html(&Some("application/json".to_string()), ""));
    }

    #[test]
    fn test_is_html_by_body() {
        assert!(is_html(&None, "<!DOCTYPE html><html>"));
        assert!(is_html(&None, "  <!DOCTYPE html>"));
        assert!(is_html(&None, "<html><body>"));
        assert!(!is_html(&None, "Hello world"));
        assert!(!is_html(&None, "{\"json\": true}"));
    }

    #[test]
    fn test_html_to_markdown_headers() {
        let html = "<h1>Title</h1><h2>Subtitle</h2>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("## Subtitle"));
    }

    #[test]
    fn test_html_to_markdown_paragraphs() {
        let html = "<p>First paragraph</p><p>Second paragraph</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("First paragraph"));
        assert!(md.contains("Second paragraph"));
    }

    #[test]
    fn test_html_to_markdown_lists() {
        let html = "<ul><li>Item 1</li><li>Item 2</li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("- Item 1"));
        assert!(md.contains("- Item 2"));
    }

    #[test]
    fn test_html_to_markdown_emphasis() {
        let html = "<p><strong>bold</strong> and <em>italic</em></p>";
        let md = html_to_markdown(html);
        assert!(md.contains("**bold**"));
        assert!(md.contains("*italic*"));
    }

    #[test]
    fn test_html_to_markdown_code() {
        let html = "<pre>code block</pre>";
        let md = html_to_markdown(html);
        assert!(md.contains("```"));
        assert!(md.contains("code block"));
    }

    #[test]
    fn test_html_to_markdown_skip_script() {
        let html = "<p>Before</p><script>alert('bad');</script><p>After</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("Before"));
        assert!(md.contains("After"));
        assert!(!md.contains("alert"));
    }

    #[test]
    fn test_html_to_text_simple() {
        let html = "<p>Hello</p><p>World</p>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[test]
    fn test_html_to_text_skip_script() {
        let html = "<p>Before</p><script>alert('bad');</script><p>After</p>";
        let text = html_to_text(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_entity_decoding() {
        let html = "<p>&amp; &lt; &gt; &quot; &apos; &nbsp; &mdash; &ndash; &copy; &reg;</p>";
        let text = html_to_text(html);
        assert!(text.contains('&'));
        assert!(text.contains('<'));
        assert!(text.contains('>'));
        assert!(text.contains('"'));
        assert!(text.contains('\''));
        assert!(text.contains('—'));
        assert!(text.contains('–'));
        assert!(text.contains('©'));
        assert!(text.contains('®'));
    }

    #[test]
    fn test_filter_excessive_newlines() {
        let input = "line1\n\n\n\n\nline2";
        let output = filter_excessive_newlines(input);
        assert_eq!(output, "line1\n\nline2");
    }

    #[test]
    fn test_clean_whitespace() {
        let input = "  hello   world  \n\n\n\n  test  ";
        let output = clean_whitespace(input);
        assert_eq!(output, "hello world\n\n  test");
    }

    #[test]
    fn test_clean_whitespace_preserves_indentation() {
        let input = "top\n  indented\n    deeper";
        let output = clean_whitespace(input);
        assert_eq!(output, "top\n  indented\n    deeper");
    }

    #[test]
    fn test_is_markdown_content_type() {
        assert!(is_markdown_content_type(&Some("text/markdown".to_string())));
        assert!(is_markdown_content_type(&Some(
            "text/markdown; charset=utf-8".to_string()
        )));
        assert!(is_markdown_content_type(&Some("Text/Markdown".to_string())));
        assert!(!is_markdown_content_type(&Some(
            "text/html; profile=\"text/markdown\"".to_string()
        )));
        assert!(!is_markdown_content_type(&Some("text/html".to_string())));
        assert!(!is_markdown_content_type(&Some("text/plain".to_string())));
        assert!(!is_markdown_content_type(&None));
    }

    #[test]
    fn test_is_plain_text_content_type() {
        assert!(is_plain_text_content_type(&Some("text/plain".to_string())));
        assert!(is_plain_text_content_type(&Some(
            "text/plain; charset=utf-8".to_string()
        )));
        assert!(is_plain_text_content_type(&Some("Text/Plain".to_string())));
        assert!(!is_plain_text_content_type(&Some(
            "text/html; profile=\"text/plain\"".to_string()
        )));
        assert!(!is_plain_text_content_type(&Some("text/html".to_string())));
        assert!(!is_plain_text_content_type(&Some(
            "text/markdown".to_string()
        )));
        assert!(!is_plain_text_content_type(&None));
    }

    #[test]
    fn test_extract_attribute() {
        assert_eq!(
            extract_attribute("a href=\"https://example.com\" class=\"link\"", "href"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            extract_attribute("img src='image.png'", "src"),
            Some("image.png".to_string())
        );
        assert_eq!(
            extract_attribute("div class=test", "class"),
            Some("test".to_string())
        );
        assert_eq!(
            extract_attribute("a title=\"İİ\" href=x", "href"),
            Some("x".to_string())
        );
    }

    #[test]
    fn test_extract_metadata_title() {
        let html = "<html><head><title>My Page</title></head><body></body></html>";
        let meta = extract_metadata(html);
        assert_eq!(meta.title.as_deref(), Some("My Page"));
    }

    #[test]
    fn test_extract_metadata_og_title_overrides() {
        let html = r#"<html><head>
            <title>Basic Title</title>
            <meta property="og:title" content="OG Title">
        </head></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.title.as_deref(), Some("OG Title"));
    }

    #[test]
    fn test_extract_metadata_description() {
        let html = r#"<html><head>
            <meta name="description" content="A page about things">
        </head></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.description.as_deref(), Some("A page about things"));
    }

    #[test]
    fn test_extract_metadata_og_description_overrides() {
        let html = r#"<html><head>
            <meta name="description" content="Basic desc">
            <meta property="og:description" content="OG desc">
        </head></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.description.as_deref(), Some("OG desc"));
    }

    #[test]
    fn test_extract_metadata_language() {
        let html = r#"<html lang="en-US"><head><title>Test</title></head></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.language.as_deref(), Some("en-US"));
    }

    #[test]
    fn test_extract_metadata_canonical_url() {
        let html = r#"<html><head>
            <link rel="canonical" href="https://example.com/page">
        </head></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(
            meta.canonical_url.as_deref(),
            Some("https://example.com/page")
        );
    }

    #[test]
    fn test_extract_metadata_author() {
        let html = r#"<html><head>
            <meta name="author" content="Jane Doe">
        </head></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.author.as_deref(), Some("Jane Doe"));
    }

    #[test]
    fn test_extract_metadata_dates() {
        let html = r#"<html><head>
            <meta property="article:published_time" content="2024-01-15T10:00:00Z">
            <meta property="article:modified_time" content="2024-02-20T12:00:00Z">
        </head></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.published_date.as_deref(), Some("2024-01-15T10:00:00Z"));
        assert_eq!(meta.modified_date.as_deref(), Some("2024-02-20T12:00:00Z"));
    }

    #[test]
    fn test_extract_metadata_time_element() {
        let html = r#"<html><body>
            <time datetime="2024-03-01">March 1, 2024</time>
        </body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.published_date.as_deref(), Some("2024-03-01"));
    }

    #[test]
    fn test_extract_metadata_links() {
        let html = r#"<html><body>
            <a href="https://example.com">Example</a>
            <a href="/about">About Us</a>
        </body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.links.len(), 2);
        assert_eq!(meta.links[0].href, "https://example.com");
        assert_eq!(meta.links[0].text, "Example");
        assert_eq!(meta.links[1].href, "/about");
        assert_eq!(meta.links[1].text, "About Us");
    }

    #[test]
    fn test_extract_headings() {
        let html = "<h1>Title</h1><h2>Section 1</h2><h3>Subsection</h3><h2>Section 2</h2>";
        let headings = extract_headings(html);
        assert_eq!(
            headings,
            vec!["# Title", "## Section 1", "### Subsection", "## Section 2"]
        );
    }

    #[test]
    fn test_extract_metadata_skips_script_content() {
        let html = r#"<html><head>
            <title>Real Title</title>
            <script>document.title = "Fake";</script>
        </head><body>
            <a href="/real">Real Link</a>
            <script><a href="/fake">Fake</a></script>
        </body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.title.as_deref(), Some("Real Title"));
        assert_eq!(meta.links.len(), 1);
        assert_eq!(meta.links[0].href, "/real");
    }

    #[test]
    fn test_extract_metadata_empty_html() {
        let meta = extract_metadata("");
        assert!(meta.is_empty());
    }

    #[test]
    fn test_extract_metadata_full_page() {
        let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <title>Article Title</title>
    <meta name="description" content="An interesting article">
    <meta name="author" content="John Smith">
    <meta property="og:title" content="OG Article Title">
    <meta property="article:published_time" content="2024-06-15">
    <link rel="canonical" href="https://example.com/article">
</head>
<body>
    <h1>Article Title</h1>
    <p>Some content with a <a href="https://link.example.com">link</a>.</p>
    <h2>Section One</h2>
    <p>More content.</p>
</body>
</html>"#;
        let mut meta = extract_metadata(html);
        meta.headings = extract_headings(html);

        assert_eq!(meta.title.as_deref(), Some("OG Article Title"));
        assert_eq!(meta.description.as_deref(), Some("An interesting article"));
        assert_eq!(meta.author.as_deref(), Some("John Smith"));
        assert_eq!(meta.language.as_deref(), Some("en"));
        assert_eq!(
            meta.canonical_url.as_deref(),
            Some("https://example.com/article")
        );
        assert_eq!(meta.published_date.as_deref(), Some("2024-06-15"));
        assert_eq!(meta.links.len(), 1);
        assert_eq!(meta.links[0].text, "link");
        assert_eq!(meta.headings, vec!["# Article Title", "## Section One"]);
        assert!(!meta.is_empty());
    }

    #[test]
    fn test_page_metadata_is_empty() {
        let meta = PageMetadata::default();
        assert!(meta.is_empty());

        let meta = PageMetadata {
            title: Some("test".to_string()),
            ..Default::default()
        };
        assert!(!meta.is_empty());
    }

    #[test]
    fn test_strip_boilerplate_extracts_main() {
        let html = r#"<nav><a href="/">Home</a></nav>
            <main><p>Important content</p></main>
            <footer>Copyright 2024</footer>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Important content"));
        assert!(!result.contains("Home"));
        assert!(!result.contains("Copyright"));
    }

    #[test]
    fn test_strip_boilerplate_extracts_article() {
        let html = r#"<nav>Menu</nav>
            <article><h1>Title</h1><p>Body text</p></article>
            <aside>Sidebar</aside>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Title"));
        assert!(result.contains("Body text"));
        assert!(!result.contains("Menu"));
        assert!(!result.contains("Sidebar"));
    }

    #[test]
    fn test_strip_boilerplate_main_takes_precedence_over_article() {
        let html = r#"<main><p>Main content</p></main>
            <article><p>Article content</p></article>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Main content"));
        // Article is outside main, so not included
        assert!(!result.contains("Article content"));
    }

    #[test]
    fn test_strip_boilerplate_fallback_strips_nav_footer_aside() {
        let html = r#"<div>
            <nav>Navigation links</nav>
            <p>Content paragraph</p>
            <footer>Footer info</footer>
            <aside>Sidebar widget</aside>
        </div>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Content paragraph"));
        assert!(!result.contains("Navigation links"));
        assert!(!result.contains("Footer info"));
        assert!(!result.contains("Sidebar widget"));
    }

    #[test]
    fn test_strip_boilerplate_role_navigation() {
        let html = r#"<div role="navigation">Nav menu</div>
            <p>Content</p>
            <div role="contentinfo">Footer stuff</div>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Content"));
        assert!(!result.contains("Nav menu"));
        assert!(!result.contains("Footer stuff"));
    }

    #[test]
    fn test_strip_boilerplate_role_main() {
        let html = r#"<nav>Nav</nav>
            <div role="main"><p>Main content here</p></div>
            <footer>Foot</footer>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Main content here"));
        assert!(!result.contains("Nav"));
        assert!(!result.contains("Foot"));
    }

    #[test]
    fn test_strip_boilerplate_nested_nav() {
        let html = r#"<nav><ul><li><a href="/">Home</a></li><li><a href="/about">About</a></li></ul></nav>
            <p>Page content</p>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Page content"));
        assert!(!result.contains("Home"));
        assert!(!result.contains("About"));
    }

    #[test]
    fn test_strip_boilerplate_no_semantic_html() {
        // No main/article/nav/footer — returns everything
        let html = "<div><p>Content 1</p></div><div><p>Content 2</p></div>";
        let result = strip_boilerplate(html);
        assert!(result.contains("Content 1"));
        assert!(result.contains("Content 2"));
    }

    #[test]
    fn test_strip_boilerplate_preserves_header_inside_main() {
        let html = r#"<header>Site header</header>
            <main><header><h1>Article header</h1></header><p>Body</p></main>"#;
        let result = strip_boilerplate(html);
        assert!(result.contains("Article header"));
        assert!(result.contains("Body"));
        assert!(!result.contains("Site header"));
    }

    #[test]
    fn test_html_to_markdown_links() {
        let html = r#"<p>Visit <a href="https://example.com">Example Site</a> today.</p>"#;
        let md = html_to_markdown(html);
        assert!(
            md.contains("[Example Site](https://example.com)"),
            "Got: {}",
            md
        );
    }

    #[test]
    fn test_html_to_markdown_link_no_text() {
        let html = r#"<a href="https://example.com"></a>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("<https://example.com>"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_images() {
        let html = r#"<img src="photo.jpg" alt="A photo">"#;
        let md = html_to_markdown(html);
        assert!(md.contains("![A photo](photo.jpg)"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_image_no_alt() {
        let html = r#"<img src="photo.jpg">"#;
        let md = html_to_markdown(html);
        assert!(md.contains("![](photo.jpg)"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_ordered_list() {
        let html = "<ol><li>First</li><li>Second</li><li>Third</li></ol>";
        let md = html_to_markdown(html);
        assert!(md.contains("1. First"), "Got: {}", md);
        assert!(md.contains("2. Second"), "Got: {}", md);
        assert!(md.contains("3. Third"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_nested_lists() {
        let html = "<ul><li>Top<ul><li>Nested</li></ul></li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("- Top"), "Got: {}", md);
        assert!(md.contains("  - Nested"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_table() {
        let html = r#"<table>
            <tr><th>Name</th><th>Age</th></tr>
            <tr><td>Alice</td><td>30</td></tr>
            <tr><td>Bob</td><td>25</td></tr>
        </table>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("| Name | Age |"), "Got: {}", md);
        assert!(md.contains("| --- | --- |"), "Got: {}", md);
        assert!(md.contains("| Alice | 30 |"), "Got: {}", md);
        assert!(md.contains("| Bob | 25 |"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_table_no_header() {
        let html = r#"<table>
            <tr><td>A</td><td>B</td></tr>
            <tr><td>C</td><td>D</td></tr>
        </table>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("| A | B |"), "Got: {}", md);
        assert!(md.contains("| C | D |"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_definition_list() {
        let html = "<dl><dt>Term</dt><dd>Definition</dd></dl>";
        let md = html_to_markdown(html);
        assert!(md.contains("**Term**"), "Got: {}", md);
        assert!(md.contains(": Definition"), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_expanded_entities() {
        let html = "<p>&trade; &bull; &hellip; &euro; &pound; &larr; &rarr;</p>";
        let md = html_to_markdown(html);
        assert!(md.contains('™'), "Got: {}", md);
        assert!(md.contains('•'), "Got: {}", md);
        assert!(md.contains('…'), "Got: {}", md);
        assert!(md.contains('€'), "Got: {}", md);
        assert!(md.contains('£'), "Got: {}", md);
        assert!(md.contains('←'), "Got: {}", md);
        assert!(md.contains('→'), "Got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_smart_quotes() {
        let html = "<p>&ldquo;Hello&rdquo; &lsquo;World&rsquo;</p>";
        let md = html_to_markdown(html);
        assert!(md.contains('\u{201C}'), "Got: {}", md);
        assert!(md.contains('\u{201D}'), "Got: {}", md);
        assert!(md.contains('\u{2018}'), "Got: {}", md);
        assert!(md.contains('\u{2019}'), "Got: {}", md);
    }
}
