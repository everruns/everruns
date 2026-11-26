use crate::{Error, Result};
use serde::{Deserialize, Serialize};

pub const MAX_OUTPUT_BYTES: usize = 512 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    pub id: String,
    pub label: String,
}
impl Button {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub title: String,
    pub body: String,
    pub actions: Vec<Button>,
}
impl Card {
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        actions: Vec<Button>,
    ) -> Result<Self> {
        let card = Self {
            title: title.into(),
            body: body.into(),
            actions,
        };
        card.validate()?;
        Ok(card)
    }
    pub fn validate(&self) -> Result<()> {
        if self.title.is_empty()
            || self.title.len() > 1024
            || self.body.len() > MAX_OUTPUT_BYTES
            || self.actions.len() > 25
        {
            return Err(Error::InvalidInput("card bounds"));
        }
        let mut ids = std::collections::HashSet::new();
        for button in &self.actions {
            if button.id.is_empty()
                || button.id.len() > 200
                || button.label.is_empty()
                || button.label.len() > 200
                || !ids.insert(&button.id)
            {
                return Err(Error::InvalidInput("card action"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Content {
    Text(String),
    Markdown(String),
    Card(Card),
}
impl Content {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Text(t) | Self::Markdown(t) if t.is_empty() || t.len() > MAX_OUTPUT_BYTES => {
                Err(Error::InvalidInput("message length"))
            }
            Self::Card(c) => c.validate(),
            _ => Ok(()),
        }
    }
    pub fn text(&self) -> &str {
        match self {
            Self::Text(t) | Self::Markdown(t) => t,
            Self::Card(c) => &c.body,
        }
    }
}
impl From<String> for Content {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<&str> for Content {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}
impl From<Card> for Content {
    fn from(value: Card) -> Self {
        Self::Card(value)
    }
}
/// Split on Unicode scalar boundaries; never produce an oversized chunk.
pub fn split_text(text: &str, max_chars: usize) -> Result<Vec<String>> {
    split_text_bounded(text, max_chars, MAX_OUTPUT_BYTES)
}

/// An explicitly budgeted host operation; ordinary output keeps its smaller bound.
pub fn split_text_bounded(text: &str, max_chars: usize, max_bytes: usize) -> Result<Vec<String>> {
    if max_bytes > 4 * 1024 * 1024
        || text.len() > max_bytes
        || max_chars == 0
        || (max_bytes > MAX_OUTPUT_BYTES && max_chars < 512)
    {
        return Err(Error::InvalidInput("split bounds"));
    }
    let mut result = Vec::new();
    let mut part = String::new();
    let mut count = 0;
    for ch in text.chars() {
        if count == max_chars {
            result.push(std::mem::take(&mut part));
            count = 0;
        }
        part.push(ch);
        count += 1;
    }
    if !part.is_empty() {
        result.push(part);
    }
    Ok(result)
}
#[macro_export]
macro_rules! card { (title: $title:expr, body: $body:expr, actions: [$($action:expr),* $(,)?] $(,)?) => { $crate::Card::new($title, $body, vec![$($action),*]) }; }
/// Expands to the same builder as ordinary registration. No implicit state or runtime.
#[macro_export]
macro_rules! channels {
    (adapters: [$($adapter:expr),+ $(,)?] $(, $field:ident: $value:expr)* $(,)?) => {{
        let builder = $crate::Channels::builder()$(.adapter($adapter))+;
        $crate::channels!(@fields builder; $($field: $value),*)
    }};
    (@fields $builder:expr;) => { $builder.build() };
    (@fields $builder:expr; data: $value:expr $(, $field:ident: $rest:expr)*) => { $crate::channels!(@fields $builder.data($value); $($field: $rest),*) };
    (@fields $builder:expr; state: $value:expr $(, $field:ident: $rest:expr)*) => { $crate::channels!(@fields $builder.state($value); $($field: $rest),*) };
    (@fields $builder:expr; $field:ident: $value:expr $(, $next:ident: $rest:expr)*) => { $crate::channels!(@fields $builder.$field($value); $($next: $rest),*) };
}

/// Split long Markdown at line/Unicode boundaries, closing and reopening fenced code.
/// Inline formatting remains provider-owned. Fence headers are limited to 128 bytes.
pub fn split_markdown(text: &str, max_chars: usize) -> Result<Vec<String>> {
    split_markdown_bounded(text, max_chars, MAX_OUTPUT_BYTES)
}

/// Bounded host-owned large replies, without raising coordinator/output defaults.
pub fn split_markdown_bounded(
    text: &str,
    max_chars: usize,
    max_bytes: usize,
) -> Result<Vec<String>> {
    if max_bytes > 4 * 1024 * 1024 || text.len() > max_bytes || max_chars < 512 {
        return Err(Error::InvalidInput("markdown split bounds"));
    }
    if text.chars().count() <= max_chars {
        return Ok(vec![text.to_owned()]);
    }
    let mut fences = Vec::new();
    let mut active: Option<(String, String)> = None;
    let mut position = 0;
    for line in text.split_inclusive('\n') {
        let header = line.trim_end_matches(['\r', '\n']);
        let trimmed = header.trim_start_matches(' ');
        if header.len() - trimmed.len() <= 3 {
            let first = trimmed.chars().next().unwrap_or(' ');
            let count = trimmed.chars().take_while(|c| *c == first).count();
            let closes = active.as_ref().is_some_and(|(marker, _)| {
                marker.starts_with(first)
                    && count >= marker.len()
                    && trimmed[count..].trim().is_empty()
            });
            if matches!(first, '`' | '~') && count >= 3 && (active.is_none() || closes) {
                if header.len() > 128 || count > 16 {
                    return Err(Error::InvalidInput("markdown fence bounds"));
                }
                let marker = first.to_string().repeat(count);
                active = if closes {
                    None
                } else {
                    Some((marker, header.to_owned()))
                };
                fences.push((position, position + line.len(), active.clone()));
            }
        }
        position += line.len();
    }
    let mut output = Vec::new();
    let mut offset = 0;
    let mut event = 0;
    active = None;
    while offset < text.len() {
        let prefix = active
            .as_ref()
            .map_or(String::new(), |(_, header)| format!("{header}\n"));
        // Reserve the maximum supported closing marker plus a preceding newline.
        let available = max_chars - prefix.chars().count() - 17;
        let mut end = text[offset..]
            .char_indices()
            .nth(available)
            .map_or(text.len(), |(i, _)| offset + i);
        if end < text.len() {
            if let Some(newline) = text[offset..end].rfind('\n') {
                end = offset + newline + 1;
            }
            if let Some((start, finish, _)) =
                fences.get(fences.partition_point(|(_, finish, _)| *finish <= end))
                && *start < end
                && end < *finish
            {
                end = *start;
            }
        }
        if end <= offset {
            return Err(Error::InvalidInput("markdown framing"));
        }
        while event < fences.len() && fences[event].1 <= end {
            active = fences[event].2.clone();
            event += 1;
        }
        let mut part = prefix;
        part.push_str(&text[offset..end]);
        if let Some((marker, _)) = &active {
            if !part.ends_with('\n') {
                part.push('\n');
            }
            part.push_str(marker);
        }
        debug_assert!(part.chars().count() <= max_chars);
        output.push(part);
        offset = end;
    }
    Ok(output)
}
