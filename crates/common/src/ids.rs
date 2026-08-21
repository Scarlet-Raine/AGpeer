//! Opaque, application-owned identifiers.
//!
//! Backend-specific identifiers are never exposed as the canonical identity of
//! a transfer or search result. Every object that an agent (or the UI) can act
//! on receives a random UUID that is meaningless outside the application.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! opaque_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a new random identifier.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// The underlying raw UUID.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

opaque_id!(TransferId, "Opaque identifier for a normalized transfer.");
opaque_id!(SearchId, "Opaque identifier for a search.");
opaque_id!(ResultId, "Opaque identifier for a single search result.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_as_string() {
        let id = TransferId::new();
        let s = id.to_string();
        let parsed: TransferId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn ids_are_distinct() {
        assert_ne!(TransferId::new(), TransferId::new());
        assert_ne!(SearchId::new().to_string(), ResultId::new().to_string());
    }
}
