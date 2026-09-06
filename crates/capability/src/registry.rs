//! Canonical-id/alias bookkeeping and duplicate rejection shared by
//! capability registries and activation paths.

use std::collections::{HashMap, HashSet};

use crate::error::CapabilityError;

/// Canonical-id and alias index for a capability registry.
///
/// Owns the identity bookkeeping every registry needs: which canonical ids
/// exist, which legacy aliases resolve to them, and collision rejection when
/// a new registration would shadow an existing id or alias. Registries keep
/// their own `id -> implementation` storage and delegate identity questions
/// here so the Framework and the product resolve capability identity the
/// same way.
#[derive(Debug, Clone, Default)]
pub struct CapabilityIdIndex {
    canonical: HashSet<String>,
    /// Alias ID -> canonical ID.
    aliases: HashMap<String, String>,
}

impl CapabilityIdIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a canonical id and its aliases, rejecting collisions.
    ///
    /// Fails with [`CapabilityError::Duplicate`] when the canonical id (or
    /// any alias) is already present as a canonical id or alias.
    pub fn insert(
        &mut self,
        canonical: impl Into<String>,
        aliases: &[&str],
    ) -> Result<(), CapabilityError> {
        let canonical = canonical.into();
        if self.contains(&canonical) {
            return Err(CapabilityError::Duplicate { id: canonical });
        }
        for alias in aliases {
            if self.contains(alias) {
                return Err(CapabilityError::Duplicate {
                    id: (*alias).to_string(),
                });
            }
        }
        for alias in aliases {
            self.aliases.insert((*alias).to_string(), canonical.clone());
        }
        self.canonical.insert(canonical);
        Ok(())
    }

    /// Insert a canonical id and its aliases, replacing an existing entry
    /// with the same canonical id (legacy registry override semantics).
    pub fn insert_or_replace(&mut self, canonical: impl Into<String>, aliases: &[&str]) {
        let canonical = canonical.into();
        self.remove(&canonical);
        for alias in aliases {
            self.aliases.insert((*alias).to_string(), canonical.clone());
        }
        self.canonical.insert(canonical);
    }

    /// Resolve an id or alias to its canonical id, if registered.
    pub fn canonical_of<'a>(&'a self, id: &'a str) -> Option<&'a str> {
        if self.canonical.contains(id) {
            Some(id)
        } else {
            self.aliases
                .get(id)
                .filter(|canonical| self.canonical.contains(*canonical))
                .map(String::as_str)
        }
    }

    /// Whether the id is present as a canonical id or alias.
    pub fn contains(&self, id: &str) -> bool {
        self.canonical.contains(id) || self.aliases.contains_key(id)
    }

    /// Remove an id (or alias) and everything resolving to its canonical id.
    ///
    /// Returns the removed canonical id, if any.
    pub fn remove(&mut self, id: &str) -> Option<String> {
        let canonical = self.canonical_of(id)?.to_string();
        self.canonical.remove(&canonical);
        self.aliases.retain(|_, target| *target != canonical);
        Some(canonical)
    }

    /// Iterate the canonical ids in the index.
    pub fn canonical_ids(&self) -> impl Iterator<Item = &str> {
        self.canonical.iter().map(String::as_str)
    }
}

/// Duplicate-activation guard used when composing an agent's capability set.
///
/// Callers canonicalize each incoming reference (via their registry) and then
/// [`activate`](ActivationSet::activate) it; the second activation of the
/// same canonical id fails with [`CapabilityError::Duplicate`]. Duplicate ids
/// are never merged and later registrations never overwrite earlier ones.
#[derive(Debug, Clone, Default)]
pub struct ActivationSet {
    seen: HashSet<String>,
}

impl ActivationSet {
    /// Create an empty activation set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one canonical capability id, rejecting duplicates.
    pub fn activate(&mut self, canonical_id: impl Into<String>) -> Result<(), CapabilityError> {
        let canonical_id = canonical_id.into();
        if !self.seen.insert(canonical_id.clone()) {
            return Err(CapabilityError::Duplicate { id: canonical_id });
        }
        Ok(())
    }

    /// Whether a canonical id was already activated.
    pub fn contains(&self, canonical_id: &str) -> bool {
        self.seen.contains(canonical_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_rejects_duplicate_canonical_and_alias() {
        let mut index = CapabilityIdIndex::new();
        index.insert("bashkit_shell", &["virtual_bash"]).unwrap();

        let err = index.insert("bashkit_shell", &[]).unwrap_err();
        assert!(err.is_duplicate());
        let err = index.insert("other", &["virtual_bash"]).unwrap_err();
        assert!(err.is_duplicate());
        // Registering a canonical id that shadows an alias is also rejected.
        let err = index.insert("virtual_bash", &[]).unwrap_err();
        assert!(err.is_duplicate());
        let err = index
            .insert("other", &["fresh_alias", "bashkit_shell"])
            .unwrap_err();
        assert_eq!(err.id(), "bashkit_shell");
        assert!(err.is_duplicate());
        assert!(!index.contains("other"));
        assert!(!index.contains("fresh_alias"));
        assert_eq!(index.canonical_of("virtual_bash"), Some("bashkit_shell"));
    }

    #[test]
    fn index_resolves_aliases_to_canonical() {
        let mut index = CapabilityIdIndex::new();
        index.insert("bashkit_shell", &["virtual_bash"]).unwrap();
        assert_eq!(index.canonical_of("bashkit_shell"), Some("bashkit_shell"));
        assert_eq!(index.canonical_of("virtual_bash"), Some("bashkit_shell"));
        assert_eq!(index.canonical_of("unknown"), None);
    }

    #[test]
    fn replace_and_remove() {
        let mut index = CapabilityIdIndex::new();
        index.insert("cap", &["old_cap"]).unwrap();
        index.insert("other", &["other_alias"]).unwrap();
        index.insert_or_replace("cap", &["older_cap"]);
        assert_eq!(index.canonical_of("older_cap"), Some("cap"));
        assert_eq!(index.canonical_of("old_cap"), None);

        assert_eq!(index.remove("older_cap"), Some("cap".to_string()));
        assert!(!index.contains("cap"));
        assert!(!index.contains("older_cap"));
        assert_eq!(index.canonical_of("other_alias"), Some("other"));
        assert_eq!(index.canonical_ids().collect::<Vec<_>>(), ["other"]);
        assert_eq!(index.remove("missing"), None);
    }

    #[test]
    fn activation_set_rejects_second_activation() {
        let mut set = ActivationSet::new();
        set.activate("current_time").unwrap();
        let err = set.activate("current_time").unwrap_err();
        assert_eq!(err.id(), "current_time");
        assert!(err.is_duplicate());
        assert!(set.contains("current_time"));
        assert!(!set.contains("web_fetch"));
        set.activate("web_fetch").unwrap();
        assert!(set.contains("web_fetch"));
    }
}
