//! Allocation-safe containers and protocol-wide structural limits.

use std::{fmt, ops::Deref};

use serde::{
    de::{Error as _, IgnoredAny, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

/// Maximum serialized size of one command request.
pub const MAX_COMMAND_BYTES: usize = 64 * 1024;
/// Maximum number of domain waypoints in one unit route.
pub const MAX_ROUTE_STEPS: usize = 4_096;
/// Maximum number of party members carried by one group movement request.
pub const MAX_PARTY_MEMBERS: usize = 6;
/// Maximum number of lattice cells accepted in one decision answer.
pub const MAX_DECISION_CELLS: usize = 256;
/// Maximum number of units represented by an encounter manifest or session projection.
pub const MAX_SESSION_UNITS: usize = 64;
/// Maximum number of persistent effects disclosed on one unit.
pub const MAX_UNIT_EFFECTS: usize = 64;
/// Maximum bytes in an authored stable identity such as a scenario or archetype name.
pub const MAX_IDENTITY_BYTES: usize = 128;
/// Maximum bytes in a build revision or semantic version identity.
pub const MAX_BUILD_IDENTITY_BYTES: usize = 64;
/// Maximum bytes in an advertised DNS name or IP literal.
pub const MAX_ADVERTISED_HOST_BYTES: usize = 253;
/// Maximum accepted encoded direct connection code size.
pub const MAX_CONNECTION_CODE_BYTES: usize = 512;
/// Maximum compressed/serialized live snapshot allocation.
pub const MAX_LIVE_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
/// Largest absolute cube coordinate accepted from an untrusted command.
pub const MAX_ABS_COMMAND_COORDINATE: u32 = 4_096;
/// Largest absolute voxel level accepted from an untrusted command.
pub const MAX_ABS_COMMAND_LEVEL: u32 = 1_024;
/// Largest absolute axial lattice coordinate accepted in a decision answer.
pub const MAX_ABS_LATTICE_COORDINATE: u32 = 64;

/// A validation failure for a bounded wire container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundError {
    /// A stable text field was empty.
    EmptyText,
    /// A text field exceeded its byte bound.
    TextTooLong {
        /// Maximum permitted UTF-8 byte length.
        maximum: usize,
        /// Actual UTF-8 byte length.
        actual: usize,
    },
    /// A text field contained a control character.
    ControlCharacter,
    /// A collection exceeded its element bound.
    TooManyItems {
        /// Maximum permitted item count.
        maximum: usize,
        /// Actual item count.
        actual: usize,
    },
}

impl fmt::Display for BoundError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyText => formatter.write_str("text must not be empty"),
            Self::TextTooLong { maximum, actual } => {
                write!(formatter, "text is {actual} bytes; maximum is {maximum}")
            }
            Self::ControlCharacter => formatter.write_str("text contains a control character"),
            Self::TooManyItems { maximum, actual } => {
                write!(
                    formatter,
                    "collection has {actual} items; maximum is {maximum}"
                )
            }
        }
    }
}

impl std::error::Error for BoundError {}

/// Owned UTF-8 text whose allocation and contents are bounded by a wire contract.
///
/// Deserialization requests a borrowed string from the format, validates it, and only
/// then allocates the owned value. This avoids trusting a serialized length as an
/// allocation request.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoundedText<const MAX: usize>(String);

impl<const MAX: usize> BoundedText<MAX> {
    /// Validates and owns text.
    pub fn new(value: impl Into<String>) -> Result<Self, BoundError> {
        let value = value.into();
        validate_text::<MAX>(&value)?;
        Ok(Self(value))
    }

    /// Borrows the validated text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns its text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl<const MAX: usize> fmt::Debug for BoundedText<MAX> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<const MAX: usize> fmt::Display for BoundedText<MAX> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<const MAX: usize> AsRef<str> for BoundedText<MAX> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const MAX: usize> Serialize for BoundedText<MAX> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, const MAX: usize> Deserialize<'de> for BoundedText<MAX> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TextVisitor<const LIMIT: usize>;

        impl<const LIMIT: usize> Visitor<'_> for TextVisitor<LIMIT> {
            type Value = BoundedText<LIMIT>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "non-empty text of at most {LIMIT} UTF-8 bytes")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                BoundedText::new(value).map_err(E::custom)
            }

            fn visit_borrowed_str<E>(self, value: &'_ str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(value)
            }
        }

        deserializer.deserialize_str(TextVisitor::<MAX>)
    }
}

fn validate_text<const MAX: usize>(value: &str) -> Result<(), BoundError> {
    if value.is_empty() {
        return Err(BoundError::EmptyText);
    }
    if value.len() > MAX {
        return Err(BoundError::TextTooLong {
            maximum: MAX,
            actual: value.len(),
        });
    }
    if value.chars().any(char::is_control) {
        return Err(BoundError::ControlCharacter);
    }
    Ok(())
}

/// An owned sequence whose deserializer refuses an excessive declared length before
/// allocating it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoundedVec<T, const MAX: usize>(Vec<T>);

impl<T, const MAX: usize> Default for BoundedVec<T, MAX> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T, const MAX: usize> BoundedVec<T, MAX> {
    /// Validates and owns a vector.
    pub fn new(values: Vec<T>) -> Result<Self, BoundError> {
        if values.len() > MAX {
            return Err(BoundError::TooManyItems {
                maximum: MAX,
                actual: values.len(),
            });
        }
        Ok(Self(values))
    }

    /// Borrows all values in order.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    /// Consumes the wrapper and returns the values.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }

    /// Number of retained values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the sequence is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T, const MAX: usize> Deref for BoundedVec<T, MAX> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T, const MAX: usize> IntoIterator for BoundedVec<T, MAX> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T: Serialize, const MAX: usize> Serialize for BoundedVec<T, MAX> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for BoundedVec<T, MAX> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VecVisitor<T, const LIMIT: usize>(std::marker::PhantomData<T>);

        impl<'de, T: Deserialize<'de>, const LIMIT: usize> Visitor<'de> for VecVisitor<T, LIMIT> {
            type Value = BoundedVec<T, LIMIT>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a sequence of at most {LIMIT} items")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                if let Some(actual) = sequence.size_hint() {
                    if actual > LIMIT {
                        return Err(A::Error::invalid_length(actual, &self));
                    }
                }

                let capacity = sequence.size_hint().unwrap_or(0).min(LIMIT);
                let mut values = Vec::with_capacity(capacity);
                while values.len() < LIMIT {
                    let Some(value) = sequence.next_element()? else {
                        return Ok(BoundedVec(values));
                    };
                    values.push(value);
                }

                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    return Err(A::Error::invalid_length(LIMIT.saturating_add(1), &self));
                }
                Ok(BoundedVec(values))
            }
        }

        deserializer.deserialize_seq(VecVisitor::<T, MAX>(std::marker::PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_text_rejects_empty_long_and_control_text() {
        assert_eq!(BoundedText::<4>::new(""), Err(BoundError::EmptyText));
        assert!(matches!(
            BoundedText::<4>::new("hello"),
            Err(BoundError::TextTooLong { .. })
        ));
        assert_eq!(
            BoundedText::<16>::new("bad\nvalue"),
            Err(BoundError::ControlCharacter)
        );
        assert_eq!(
            BoundedText::<4>::new("okay").map(|text| text.into_string()),
            Ok("okay".to_owned())
        );
    }

    #[test]
    fn bounded_vec_rejects_declared_and_constructed_overflow() {
        assert!(matches!(
            BoundedVec::<u8, 2>::new(vec![1, 2, 3]),
            Err(BoundError::TooManyItems { .. })
        ));
        let decoded = serde_json::from_str::<BoundedVec<u8, 2>>("[1,2,3]");
        assert!(decoded.is_err());
    }
}
