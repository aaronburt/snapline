use std::fmt;

use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretKeyError {
    #[error("a secret key part must not be empty")]
    EmptyPart,
    #[error("a secret key part must not contain ':' or NUL")]
    InvalidPart,
}

#[derive(Clone)]
pub struct Secret {
    bytes: Zeroizing<Vec<u8>>,
}

impl Secret {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: Zeroizing::new(bytes.into()),
        }
    }

    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Self {
        Self::new(bytes.as_ref().to_vec())
    }

    pub fn expose(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.bytes.zeroize();
        #[cfg(test)]
        DROPPED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        let left = self.expose();
        let right = other.expose();
        if left.len() != right.len() {
            return false;
        }
        let mut difference = 0_u8;
        for (a, b) in left.iter().zip(right) {
            difference |= a ^ b;
        }
        difference == 0
    }
}

impl Eq for Secret {}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::from_bytes(value)
    }
}

impl From<Vec<u8>> for Secret {
    fn from(value: Vec<u8>) -> Self {
        Self::new(value)
    }
}

impl From<&[u8]> for Secret {
    fn from(value: &[u8]) -> Self {
        Self::from_bytes(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretKey {
    dest_id: String,
    field: String,
}

impl SecretKey {
    pub const TARGET_PREFIX: &'static str = "SnapLine:";
    pub const USER_NAME: &'static str = "SnapLine";

    pub fn new(
        dest_id: impl Into<String>,
        field: impl Into<String>,
    ) -> Result<Self, SecretKeyError> {
        let dest_id = dest_id.into();
        let field = field.into();
        validate_part(&dest_id)?;
        validate_part(&field)?;
        Ok(Self { dest_id, field })
    }

    pub fn dest_id(&self) -> &str {
        &self.dest_id
    }

    pub fn field(&self) -> &str {
        &self.field
    }

    pub fn target(&self) -> String {
        format!("{}{}:{}", Self::TARGET_PREFIX, self.dest_id, self.field)
    }
}

fn validate_part(part: &str) -> Result<(), SecretKeyError> {
    if part.is_empty() {
        return Err(SecretKeyError::EmptyPart);
    }
    if part.contains(':') || part.contains('\0') {
        return Err(SecretKeyError::InvalidPart);
    }
    Ok(())
}

#[cfg(test)]
static DROPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    #[test]
    fn debug_never_shows_the_value() {
        let secret = Secret::from("hunter2");
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(secret.expose(), b"hunter2");
    }

    #[test]
    fn dropping_a_secret_zeroises_its_backing_storage() {
        DROPPED.store(false, Ordering::SeqCst);
        let secret = Secret::from_bytes(b"top secret");
        assert_eq!(secret.len(), 10);
        assert!(!secret.is_empty());
        drop(secret);
        assert!(DROPPED.load(Ordering::SeqCst));
    }

    #[test]
    fn secrets_compare_by_value_in_constant_time() {
        assert_eq!(Secret::from("abc"), Secret::from("abc"));
        assert_ne!(Secret::from("abc"), Secret::from("abd"));
        assert_ne!(Secret::from("abc"), Secret::from("ab"));
    }

    #[test]
    fn cloning_keeps_the_same_bytes() {
        let secret = Secret::from("abc");
        assert_eq!(secret.clone(), secret);
    }

    #[test]
    fn a_key_builds_the_documented_target() {
        let Ok(key) = SecretKey::new("imgur", "token") else {
            panic!("a valid key was rejected");
        };
        assert_eq!(key.target(), "SnapLine:imgur:token");
        assert_eq!(key.dest_id(), "imgur");
        assert_eq!(key.field(), "token");
        assert_eq!(SecretKey::USER_NAME, "SnapLine");
    }

    #[test]
    fn empty_key_parts_are_rejected() {
        assert!(matches!(
            SecretKey::new("", "token"),
            Err(SecretKeyError::EmptyPart)
        ));
        assert!(matches!(
            SecretKey::new("imgur", ""),
            Err(SecretKeyError::EmptyPart)
        ));
    }

    #[test]
    fn key_parts_with_separators_or_nul_are_rejected() {
        assert!(matches!(
            SecretKey::new("img:ur", "token"),
            Err(SecretKeyError::InvalidPart)
        ));
        assert!(matches!(
            SecretKey::new("imgur", "to:ken"),
            Err(SecretKeyError::InvalidPart)
        ));
        assert!(matches!(
            SecretKey::new("img\0ur", "token"),
            Err(SecretKeyError::InvalidPart)
        ));
        assert!(matches!(
            SecretKey::new("imgur", "to\0ken"),
            Err(SecretKeyError::InvalidPart)
        ));
    }
}
