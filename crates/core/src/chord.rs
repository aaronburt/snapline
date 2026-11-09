use std::fmt;
use std::ops::BitOr;
use std::str::FromStr;

use thiserror::Error;

const CTRL_BIT: u8 = 0b0001;
const ALT_BIT: u8 = 0b0010;
const SHIFT_BIT: u8 = 0b0100;
const WIN_BIT: u8 = 0b1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const CTRL: Self = Self(CTRL_BIT);
    pub const ALT: Self = Self(ALT_BIT);
    pub const SHIFT: Self = Self(SHIFT_BIT);
    pub const WIN: Self = Self(WIN_BIT);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    fn parse_name(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().as_str() {
            "CTRL" => Some(Self::CTRL),
            "ALT" => Some(Self::ALT),
            "SHIFT" => Some(Self::SHIFT),
            "WIN" => Some(Self::WIN),
            _ => None,
        }
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.contains(Self::CTRL) {
            parts.push("Ctrl");
        }
        if self.contains(Self::ALT) {
            parts.push("Alt");
        }
        if self.contains(Self::SHIFT) {
            parts.push("Shift");
        }
        if self.contains(Self::WIN) {
            parts.push("Win");
        }
        formatter.write_str(&parts.join("+"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum KeyKind {
    PrtSc,
    Pause,
    Letter(u8),
    Digit(u8),
    Function(u8),
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(KeyKind);

impl Key {
    pub fn parse(name: &str) -> Result<Self, ChordError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ChordError::UnknownKey(String::new()));
        }
        let upper = trimmed.to_ascii_uppercase();
        let kind = match upper.as_str() {
            "PRTSC" => KeyKind::PrtSc,
            "PAUSE" => KeyKind::Pause,
            "INSERT" => KeyKind::Insert,
            "DELETE" => KeyKind::Delete,
            "HOME" => KeyKind::Home,
            "END" => KeyKind::End,
            "PAGEUP" => KeyKind::PageUp,
            "PAGEDOWN" => KeyKind::PageDown,
            "UP" => KeyKind::Up,
            "DOWN" => KeyKind::Down,
            "LEFT" => KeyKind::Left,
            "RIGHT" => KeyKind::Right,
            other => {
                let bytes = other.as_bytes();
                if bytes.len() == 1 {
                    match bytes[0] {
                        b'A'..=b'Z' => KeyKind::Letter(bytes[0]),
                        b'0'..=b'9' => KeyKind::Digit(bytes[0] - b'0'),
                        _ => return Err(ChordError::UnknownKey(trimmed.to_string())),
                    }
                } else if let Some(number) = other.strip_prefix('F') {
                    if number.is_empty() || number.starts_with('0') {
                        return Err(ChordError::UnknownKey(trimmed.to_string()));
                    }
                    let function = number
                        .parse::<u8>()
                        .map_err(|_| ChordError::UnknownKey(trimmed.to_string()))?;
                    if !(1..=24).contains(&function) {
                        return Err(ChordError::UnknownKey(trimmed.to_string()));
                    }
                    KeyKind::Function(function)
                } else {
                    return Err(ChordError::UnknownKey(trimmed.to_string()));
                }
            }
        };
        Ok(Self(kind))
    }

    pub const fn is_allowed_without_modifier(self) -> bool {
        matches!(
            self.0,
            KeyKind::PrtSc | KeyKind::Pause | KeyKind::Function(_)
        )
    }
}

impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            KeyKind::PrtSc => formatter.write_str("PrtSc"),
            KeyKind::Pause => formatter.write_str("Pause"),
            KeyKind::Letter(letter) => write!(formatter, "{}", letter as char),
            KeyKind::Digit(digit) => write!(formatter, "{digit}"),
            KeyKind::Function(function) => write!(formatter, "F{function}"),
            KeyKind::Insert => formatter.write_str("Insert"),
            KeyKind::Delete => formatter.write_str("Delete"),
            KeyKind::Home => formatter.write_str("Home"),
            KeyKind::End => formatter.write_str("End"),
            KeyKind::PageUp => formatter.write_str("PageUp"),
            KeyKind::PageDown => formatter.write_str("PageDown"),
            KeyKind::Up => formatter.write_str("Up"),
            KeyKind::Down => formatter.write_str("Down"),
            KeyKind::Left => formatter.write_str("Left"),
            KeyKind::Right => formatter.write_str("Right"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Chord {
    pub fn new(modifiers: Modifiers, key: Key) -> Result<Self, ChordError> {
        let chord = Self { modifiers, key };
        chord.validate()?;
        Ok(chord)
    }

    pub fn parse(input: &str) -> Result<Self, ChordError> {
        input.parse()
    }

    pub fn validate(&self) -> Result<(), ChordError> {
        if self.modifiers.is_empty() && !self.key.is_allowed_without_modifier() {
            return Err(ChordError::MissingModifier(self.key));
        }
        if self.is_windows_reserved() {
            return Err(ChordError::WindowsReserved);
        }
        Ok(())
    }

    pub fn is_windows_reserved(&self) -> bool {
        if !self.modifiers.contains(Modifiers::WIN) {
            return false;
        }
        let win_shift = Modifiers::WIN | Modifiers::SHIFT;
        let win_ctrl = Modifiers::WIN | Modifiers::CTRL;
        let win_alt = Modifiers::WIN | Modifiers::ALT;
        let plain_win_key = matches!(
            self.key.0,
            KeyKind::PrtSc
                | KeyKind::Pause
                | KeyKind::Letter(_)
                | KeyKind::Digit(_)
                | KeyKind::Home
                | KeyKind::Up
                | KeyKind::Down
                | KeyKind::Left
                | KeyKind::Right
        );
        if self.modifiers == Modifiers::WIN && plain_win_key {
            return true;
        }
        match self.key.0 {
            KeyKind::Letter(b'S') => self.modifiers == win_shift,
            KeyKind::Letter(b'D') => self.modifiers == win_ctrl,
            KeyKind::Letter(b'R') => self.modifiers == win_alt,
            KeyKind::Letter(b'B') => self.modifiers == win_ctrl | Modifiers::SHIFT,
            KeyKind::PrtSc => self.modifiers == win_alt,
            KeyKind::Left | KeyKind::Right => {
                self.modifiers == win_ctrl || self.modifiers == win_shift
            }
            KeyKind::Up | KeyKind::Down => self.modifiers == win_shift,
            _ => false,
        }
    }
}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut modifiers = Modifiers::NONE;
        let mut key = None;
        for part in input.split('+') {
            let part = part.trim();
            if part.is_empty() {
                return Err(ChordError::EmptyPart);
            }
            if let Some(modifier) = Modifiers::parse_name(part) {
                modifiers.insert(modifier);
                continue;
            }
            if let Ok(parsed) = Key::parse(part) {
                if key.replace(parsed).is_some() {
                    return Err(ChordError::MultipleKeys);
                }
                continue;
            }
            return Err(ChordError::UnknownPart(part.to_string()));
        }
        let key = key.ok_or(ChordError::MissingKey)?;
        let chord = Self { modifiers, key };
        chord.validate()?;
        Ok(chord)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.is_empty() {
            write!(formatter, "{}", self.key)
        } else {
            write!(formatter, "{}+{}", self.modifiers, self.key)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChordError {
    #[error("Unknown key or modifier: {0}")]
    UnknownPart(String),
    #[error("Unknown key: {0}")]
    UnknownKey(String),
    #[error("A chord contains an empty part")]
    EmptyPart,
    #[error("A chord needs a key")]
    MissingKey,
    #[error("A chord needs exactly one key")]
    MultipleKeys,
    #[error("A chord needs Ctrl, Alt, Shift or Win, unless the key is PrtSc, Pause or F1 to F24")]
    MissingModifier(Key),
    #[error("Windows uses this shortcut.")]
    WindowsReserved,
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: &[&str] = &[
        "PrtSc", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
        "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7",
        "8", "9", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13",
        "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24", "Pause",
        "Insert", "Delete", "Home", "End", "PageUp", "PageDown", "Up", "Down", "Left", "Right",
    ];

    const DEFAULTS: &[&str] = &[
        "PrtSc",
        "Alt+PrtSc",
        "Ctrl+PrtSc",
        "Ctrl+Alt+S",
        "Ctrl+Shift+R",
        "Shift+PrtSc",
        "Ctrl+Shift+G",
        "Ctrl+Shift+T",
        "Ctrl+Shift+C",
        "Ctrl+Shift+M",
        "Ctrl+Shift+V",
        "Ctrl+Shift+A",
        "Ctrl+Alt+R",
        "Ctrl+Alt+T",
        "Ctrl+Alt+PrtSc",
    ];

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("valid chord")
    }

    #[test]
    fn parses_every_key_name() {
        for name in KEYS {
            let text = format!("Ctrl+{name}");
            let parsed = chord(&text);
            assert_eq!(parsed.key.to_string(), *name);
            assert_eq!(parsed.to_string(), text);
            assert!(parsed.modifiers.contains(Modifiers::CTRL));
        }
    }

    #[test]
    fn parses_modifier_order_in_any_order() {
        let expected = chord("Ctrl+Alt+Shift+F5");
        for text in [
            "Ctrl+Alt+Shift+F5",
            "Ctrl+Shift+Alt+F5",
            "Alt+Ctrl+Shift+F5",
            "Alt+Shift+Ctrl+F5",
            "Shift+Ctrl+Alt+F5",
            "Shift+Alt+Ctrl+F5",
        ] {
            assert_eq!(chord(text), expected, "{text}");
            assert_eq!(chord(text).to_string(), "Ctrl+Alt+Shift+F5");
        }
        let with_win = chord("Ctrl+Alt+Shift+Win+F5");
        for text in [
            "Ctrl+Alt+Shift+Win+F5",
            "Win+Ctrl+Alt+Shift+F5",
            "Alt+Win+Shift+Ctrl+F5",
            "Shift+Win+Alt+Ctrl+F5",
        ] {
            assert_eq!(chord(text), with_win, "{text}");
            assert_eq!(chord(text).to_string(), "Ctrl+Alt+Shift+Win+F5");
        }
    }

    #[test]
    fn normalising_is_idempotent() {
        let mut inputs: Vec<String> = DEFAULTS.iter().map(|value| (*value).to_string()).collect();
        inputs.extend([
            "ctrl+shift+r".to_string(),
            "Shift+Ctrl+R".to_string(),
            "Ctrl+Ctrl+A".to_string(),
            "alt+win+f24".to_string(),
            "win+shift+f5".to_string(),
            "PAGEUP+ctrl".to_string(),
        ]);
        for text in inputs {
            if let Ok(once) = Chord::parse(&text) {
                let displayed = once.to_string();
                let twice = chord(&displayed);
                assert_eq!(once, twice, "{text}");
                assert_eq!(displayed, twice.to_string(), "{text}");
            }
        }
    }

    #[test]
    fn every_default_displays_exactly_as_written() {
        for text in DEFAULTS {
            assert_eq!(chord(text).to_string(), *text);
        }
    }

    #[test]
    fn modifiers_without_a_key_are_allowed_for_the_documented_keys() {
        for text in ["PrtSc", "Pause", "F1", "F24", "f12"] {
            assert!(Chord::parse(text).is_ok(), "{text}");
        }
        for text in [
            "A", "0", "Up", "Insert", "Delete", "Home", "End", "PageUp", "PageDown",
        ] {
            assert_eq!(
                Chord::parse(text),
                Err(ChordError::MissingModifier(chord_key(text)))
            );
        }
    }

    #[test]
    fn windows_reserved_chords_are_refused_with_the_documented_text() {
        for text in ["Win+L", "Win+D", "Win+PrtSc", "Win+Shift+S", "Win+Ctrl+D"] {
            let error = Chord::parse(text).expect_err("reserved chord");
            assert_eq!(error, ChordError::WindowsReserved, "{text}");
            assert_eq!(error.to_string(), "Windows uses this shortcut.", "{text}");
        }
        assert!(Chord::parse("Win+F5").is_ok());
        assert!(Chord::parse("Win+Shift+A").is_ok());
    }

    #[test]
    fn rejects_malformed_chords() {
        assert_eq!(Chord::parse(""), Err(ChordError::EmptyPart));
        assert_eq!(Chord::parse("Ctrl+"), Err(ChordError::EmptyPart));
        assert_eq!(Chord::parse("Ctrl"), Err(ChordError::MissingKey));
        assert_eq!(Chord::parse("Ctrl+A+B"), Err(ChordError::MultipleKeys));
        assert_eq!(
            Chord::parse("Ctrl+Nope"),
            Err(ChordError::UnknownPart("Nope".to_string()))
        );
        assert_eq!(
            Chord::parse("F25"),
            Err(ChordError::UnknownPart("F25".to_string()))
        );
        assert_eq!(
            Chord::parse("Ctrl+F0"),
            Err(ChordError::UnknownPart("F0".to_string()))
        );
    }

    #[test]
    fn display_order_is_ctrl_alt_shift_win() {
        assert_eq!(
            chord("Win+Shift+Alt+Ctrl+PrtSc").to_string(),
            "Ctrl+Alt+Shift+Win+PrtSc"
        );
    }

    fn chord_key(name: &str) -> Key {
        Key::parse(name).expect("valid key")
    }
}
