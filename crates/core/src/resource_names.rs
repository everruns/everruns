//! Shared helpers for naming external resources created from user-visible titles.

use rand::Rng;

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
    fn unique_resource_name_appends_lowercase_alphanumeric_suffix() {
        let name = unique_resource_name("sandbox-name");
        let suffix = name
            .canonical_name
            .strip_prefix("sandbox-name-")
            .expect("suffix should be appended");

        assert_eq!(name.requested_name, "sandbox-name");
        assert_eq!(suffix.len(), 6);
        assert!(
            suffix
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        );
    }

    #[test]
    fn unique_resource_name_trims_input_and_trailing_dash() {
        let name = unique_resource_name("  sandbox-name-  ");

        assert_eq!(name.requested_name, "sandbox-name-");
        assert!(name.canonical_name.starts_with("sandbox-name-"));
        assert!(!name.canonical_name.starts_with("sandbox-name--"));
    }
}
