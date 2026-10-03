const MILLION_CONTEXT_SUFFIX: &[u8] = b"[1m]";

pub(super) fn matches_alias(model_id: &[u8], alias: &[u8]) -> bool {
    model_id.eq_ignore_ascii_case(alias)
        || matches_versioned_alias(model_id, alias)
        || match (
            without_million_context(model_id),
            without_million_context(alias),
        ) {
            (Some(model_id), Some(alias)) => matches_versioned_alias(model_id, alias),
            _ => false,
        }
}

fn without_million_context(value: &[u8]) -> Option<&[u8]> {
    let split = value.len().checked_sub(MILLION_CONTEXT_SUFFIX.len())?;
    value[split..]
        .eq_ignore_ascii_case(MILLION_CONTEXT_SUFFIX)
        .then_some(&value[..split])
}

fn matches_versioned_alias(model_id: &[u8], alias: &[u8]) -> bool {
    model_id.len() > alias.len()
        && model_id[alias.len()] == b'-'
        && model_id[..alias.len()].eq_ignore_ascii_case(alias)
        && is_version_suffix(&model_id[alias.len() + 1..])
}

fn is_version_suffix(suffix: &[u8]) -> bool {
    suffix.eq_ignore_ascii_case(b"latest")
        || (suffix.len() == 8 && suffix.iter().all(u8::is_ascii_digit))
        || (suffix.len() == 5
            && suffix[2] == b'-'
            && suffix[..2].iter().all(u8::is_ascii_digit)
            && suffix[3..].iter().all(u8::is_ascii_digit))
        || (suffix.len() == 10
            && suffix[4] == b'-'
            && suffix[7] == b'-'
            && suffix
                .iter()
                .enumerate()
                .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit()))
        || (suffix.len() == 13
            && suffix[..8].eq_ignore_ascii_case(b"preview-")
            && suffix[10] == b'-'
            && suffix[8..10].iter().all(u8::is_ascii_digit)
            && suffix[11..].iter().all(u8::is_ascii_digit))
}
