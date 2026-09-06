//! Shared helpers for naming external resources created from user-visible titles.

use rand::RngExt;

const SUFFIX_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const SUFFIX_LEN: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueResourceName {
    pub requested_name: String,
    pub canonical_name: String,
}

pub fn unique_resource_name(requested_name: &str) -> UniqueResourceName {
    let requested_name = requested_name.trim().to_string();
    let base_name = requested_name.trim_end_matches('-');
    let mut rng = rand::rng();
    let suffix: String = (0..SUFFIX_LEN)
        .map(|_| {
            let index = rng.random_range(0..SUFFIX_ALPHABET.len());
            SUFFIX_ALPHABET[index] as char
        })
        .collect();
    let canonical_name = if base_name.is_empty() {
        suffix
    } else {
        format!("{base_name}-{suffix}")
    };

    UniqueResourceName {
        requested_name,
        canonical_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_resource_name_preserves_title_and_appends_safe_suffix() {
        for (input, requested, prefix) in [
            ("sandbox-name", "sandbox-name", "sandbox-name-"),
            ("  sandbox-name-  ", "sandbox-name-", "sandbox-name-"),
            ("sandbox-name---", "sandbox-name---", "sandbox-name-"),
            ("  ", "", ""),
            ("---", "---", ""),
        ] {
            let name = unique_resource_name(input);
            assert_eq!(name.requested_name, requested);
            let suffix = name
                .canonical_name
                .strip_prefix(prefix)
                .expect("canonical base");
            assert_eq!(suffix.len(), 6, "{input:?}");
            assert!(
                suffix
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()),
                "{input:?}: {suffix}"
            );
        }
    }
}
