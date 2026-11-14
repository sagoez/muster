/// An edit to an optional field: leave it, set it, or clear it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FieldEdit<T> {
    /// Leave the field as it is.
    #[default]
    Keep,
    /// Set the field to this value.
    Set(T),
    /// Clear the field.
    Clear,
}

impl<T> FieldEdit<T> {
    /// The field's value after this edit is applied to `current`.
    pub fn apply(self, current: Option<T>) -> Option<T> {
        match self {
            Self::Keep => current,
            Self::Set(value) => Some(value),
            Self::Clear => None,
        }
    }

    /// Whether this edit leaves the field as it is.
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_keep_set_and_clear() {
        assert_eq!(FieldEdit::Keep.apply(Some(1)), Some(1));
        assert_eq!(FieldEdit::Set(2).apply(Some(1)), Some(2));
        assert_eq!(FieldEdit::<i32>::Clear.apply(Some(1)), None);
    }
}
