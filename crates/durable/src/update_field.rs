// UpdateField<T> — a typed alternative to Option<Option<T>> for update semantics.
//
// Encodes three states: leave unchanged, clear the value, or set a new value.
// Replaces the confusing Option<Option<T>> pattern (None=unchanged,
// Some(None)=clear, Some(Some(v))=set) with explicit variants.

/// A field in an update operation that can be unchanged, cleared, or set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateField<T> {
    /// Leave the current value unchanged.
    #[default]
    Unchanged,
    /// Clear the value (set to NULL/None).
    Clear,
    /// Set to the given value.
    Set(T),
}

impl<T> UpdateField<T> {
    /// Whether this field should be applied (Clear or Set).
    pub fn is_changed(&self) -> bool {
        !matches!(self, Self::Unchanged)
    }

    /// Convert to `Option<T>` for SQL binding: Clear→None, Set(v)→Some(v).
    /// Returns `None` for both `Unchanged` and `Clear`.
    /// Use with `is_changed()` to distinguish between the two.
    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Unchanged => None,
            Self::Clear => None,
            Self::Set(v) => Some(v),
        }
    }

    /// Apply this update to a mutable `Option<T>` field.
    pub fn apply(self, target: &mut Option<T>) {
        match self {
            Self::Unchanged => {}
            Self::Clear => *target = None,
            Self::Set(v) => *target = Some(v),
        }
    }

    /// Convert `Option<T>` where the caller explicitly chose to set or clear:
    /// `Some(v)` → `Set(v)`, `None` → `Clear`.
    ///
    /// Use this when `None` means "clear the value", not "leave unchanged".
    /// For PATCH-style fields where `None` means "omitted / unchanged",
    /// use `map_or(UpdateField::Unchanged, UpdateField::Set)` instead.
    pub fn from_option(opt: Option<T>) -> Self {
        match opt {
            Some(v) => Self::Set(v),
            None => Self::Clear,
        }
    }

    /// Map the inner value if `Set`.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> UpdateField<U> {
        match self {
            Self::Unchanged => UpdateField::Unchanged,
            Self::Clear => UpdateField::Clear,
            Self::Set(v) => UpdateField::Set(f(v)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_unchanged() {
        let field: UpdateField<String> = UpdateField::default();
        assert!(!field.is_changed());
    }

    #[test]
    fn test_clear_is_changed() {
        let field: UpdateField<String> = UpdateField::Clear;
        assert!(field.is_changed());
        assert_eq!(field.into_value(), None);
    }

    #[test]
    fn test_set_is_changed() {
        let field = UpdateField::Set(42);
        assert!(field.is_changed());
        assert_eq!(field.into_value(), Some(42));
    }

    #[test]
    fn test_apply() {
        let mut target = Some(10);

        UpdateField::<i32>::Unchanged.apply(&mut target);
        assert_eq!(target, Some(10));

        UpdateField::Clear.apply(&mut target);
        assert_eq!(target, None);

        UpdateField::Set(20).apply(&mut target);
        assert_eq!(target, Some(20));
    }

    #[test]
    fn test_from_option() {
        assert_eq!(UpdateField::from_option(Some(42)), UpdateField::Set(42));
        assert_eq!(
            UpdateField::from_option(None::<i32>),
            UpdateField::<i32>::Clear
        );
    }

    #[test]
    fn test_map() {
        let field = UpdateField::Set(42);
        let mapped = field.map(|v| v.to_string());
        assert_eq!(mapped, UpdateField::Set("42".to_string()));

        let unchanged: UpdateField<i32> = UpdateField::Unchanged;
        let mapped = unchanged.map(|v| v.to_string());
        assert_eq!(mapped, UpdateField::<String>::Unchanged);
    }
}
