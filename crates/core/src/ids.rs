use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CaptureId(i64);

impl CaptureId {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> i64 {
        self.0
    }

    pub const fn positive(self) -> bool {
        self.0 > 0
    }
}

impl std::fmt::Display for CaptureId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::str::FromStr for CaptureId {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DestId(String);

impl DestId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DestId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for DestId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DestId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkflowId(String);

impl WorkflowId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for WorkflowId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for WorkflowId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowId(u64);

impl WindowId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_id_is_transparent() {
        let id = CaptureId::new(42);
        assert_eq!(id.get(), 42);
        assert!(id.positive());
        assert!(!CaptureId::new(0).positive());
        assert_eq!(id.to_string(), "42");
        assert_eq!("42".parse::<CaptureId>(), Ok(id));
    }

    #[test]
    fn dest_and_workflow_ids_display_their_value() {
        assert_eq!(DestId::new("imgur").to_string(), "imgur");
        assert_eq!(WorkflowId::new("default").as_str(), "default");
    }

    #[test]
    fn window_id_round_trips() {
        assert_eq!(WindowId::new(99).get(), 99);
    }
}
