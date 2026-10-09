// Splitting one agent reply into pieces Slack will render.
//
// Slack's `markdown` block takes 12k characters and a message takes 50 of them,
// so a long reply has to be cut somewhere. Cutting inside a fenced code block
// would leave both halves rendering as prose, so the fence is closed and
// reopened around the seam — which is what makes this more than a chunker, and
// what the TM-DOS-040 bounds here are about.
//
// Split out of `delivery/mod.rs`, which is on the source-file size debt list.

/// Characters Slack accepts in one `markdown` block.
pub(super) const SLACK_MARKDOWN_BLOCK_LIMIT: usize = 12_000;

/// Blocks Slack accepts in one `chat.postMessage` call.
pub(super) const SLACK_MAX_BLOCKS_PER_MESSAGE: usize = 50;

/// Maximum assistant output considered for one Slack delivery.
///
/// This bounds synchronous payload construction before the first Slack API
/// request. The cap still allows a full message's worth of Markdown blocks.
pub(super) const SLACK_MAX_OUTBOUND_REPLY_CHARS: usize =
    SLACK_MARKDOWN_BLOCK_LIMIT * SLACK_MAX_BLOCKS_PER_MESSAGE;

/// Truncate on a char boundary, so multi-byte text cannot panic the slice.
pub(super) fn truncate_chars(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

/// Split Markdown into pieces that each fit one Slack `markdown` block.
///
/// Splits on line boundaries, and never leaves a fenced code block open: when a
/// boundary lands inside a fence the fence is closed at the end of the piece and
/// reopened — with its original info string — at the start of the next, so a
/// split code block still renders as code on both sides.
pub(super) fn split_markdown_for_blocks(text: &str, limit: usize) -> Vec<String> {
    // THREAT[TM-DOS-040]: Fence continuations must be bounded and every hard
    // split iteration must either consume input or reset to a consumable state.
    debug_assert!(limit > 8, "limit must leave room for fence markers");
    if text.chars().count() <= limit {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    // The fence currently open, as (marker, info string) — e.g. ("```", "rust").
    let mut open_fence: Option<(String, String)> = None;
    // Reopened at the top of the next chunk when a split interrupts a fence.
    let mut reopen: Option<String> = None;

    // Reserve room for the closing fence we may have to append.
    let effective = limit.saturating_sub(4);

    let flush = |current: &mut String,
                 current_len: &mut usize,
                 open_fence: &Option<(String, String)>,
                 chunks: &mut Vec<String>,
                 reopen: &mut Option<String>| {
        if current.is_empty() {
            return;
        }
        let mut chunk = std::mem::take(current);
        if let Some((marker, info)) = open_fence {
            // Close the fence here and reopen it in the next chunk.
            if !chunk.ends_with('\n') {
                chunk.push('\n');
            }
            chunk.push_str(marker);
            *reopen = Some(format!("{}{}", marker, info));
        } else {
            *reopen = None;
        }
        chunks.push(chunk);
        *current_len = 0;
    };

    for line in text.split_inclusive('\n') {
        let line_len = line.chars().count();

        // A line that does not fit the current chunk has to go through the
        // hard-split path. This includes a short line following a long fence
        // reopen prefix, not only lines that exceed an empty chunk.
        if line_len > effective.saturating_sub(current_len) {
            flush(
                &mut current,
                &mut current_len,
                &open_fence,
                &mut chunks,
                &mut reopen,
            );
            if let Some(ref head) = reopen.take() {
                current.push_str(head);
                current.push('\n');
                current_len = head.chars().count() + 1;
            }
            let mut rest = line;
            while rest.chars().count() > effective.saturating_sub(current_len) {
                let room = effective.saturating_sub(current_len);
                // Fence reopen prefixes are bounded below, so this is
                // unreachable. Keep the guard at the consumption point: no
                // malformed Markdown may turn this into a zero-progress loop.
                if room == 0 {
                    current.clear();
                    current_len = 0;
                    open_fence = None;
                    reopen = None;
                    continue;
                }
                let head = truncate_chars(rest, room);
                current.push_str(head);
                current_len += head.chars().count();
                rest = &rest[head.len()..];
                flush(
                    &mut current,
                    &mut current_len,
                    &open_fence,
                    &mut chunks,
                    &mut reopen,
                );
                if let Some(ref h) = reopen.take() {
                    current.push_str(h);
                    current.push('\n');
                    current_len = h.chars().count() + 1;
                }
            }
            current.push_str(rest);
            current_len += rest.chars().count();
            continue;
        }

        if current_len + line_len > effective {
            flush(
                &mut current,
                &mut current_len,
                &open_fence,
                &mut chunks,
                &mut reopen,
            );
            if let Some(head) = reopen.take() {
                current.push_str(&head);
                current.push('\n');
                current_len = head.chars().count() + 1;
            }
        }

        // Track fence state after placement, so the marker line itself lands in
        // the chunk that opens or closes it.
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("```")
            .or_else(|| trimmed.strip_prefix("~~~"))
        {
            let marker = &trimmed[..3];
            match open_fence {
                // A closing fence carries no info string.
                Some((ref open_marker, _)) if open_marker == marker => open_fence = None,
                Some(_) => {}
                None => {
                    // A continuation must leave room for its newline and at
                    // least one input character. Fence info is presentation
                    // metadata, so bounding only the repeated copy preserves
                    // the original opening line while guaranteeing progress.
                    let max_info_len = effective.saturating_sub(marker.chars().count() + 2);
                    let info = truncate_chars(rest.trim_end(), max_info_len).to_string();
                    open_fence = Some((marker.to_string(), info));
                }
            }
        }

        current.push_str(line);
        current_len += line_len;
    }

    flush(
        &mut current,
        &mut current_len,
        &open_fence,
        &mut chunks,
        &mut reopen,
    );

    chunks
}
