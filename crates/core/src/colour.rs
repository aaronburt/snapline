use std::fmt;

use thiserror::Error;

pub use crate::overlay::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColourFormat {
    Hex,
    Rgb,
    Hsl,
}

impl ColourFormat {
    pub const ALL: [Self; 3] = [Self::Hex, Self::Rgb, Self::Hsl];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hex => "Hex",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::Rgb => "rgb",
            Self::Hsl => "hsl",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "hex" => Some(Self::Hex),
            "rgb" => Some(Self::Rgb),
            "hsl" => Some(Self::Hsl),
            _ => None,
        }
    }

    pub fn display(self, colour: Rgb) -> String {
        match self {
            Self::Hex => Hex::from(colour).to_string(),
            Self::Rgb => colour.to_string(),
            Self::Hsl => Hsl::from(colour).to_string(),
        }
    }
}

impl Default for ColourFormat {
    fn default() -> Self {
        Self::Hex
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ColourError {
    #[error("The colour is empty.")]
    Empty,
    #[error("The hex colour is not valid.")]
    Hex,
    #[error("The RGB colour is not valid.")]
    Rgb,
    #[error("The HSL colour is not valid.")]
    Hsl,
    #[error("A colour component is out of range.")]
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hex {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Hex {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn parse(input: &str) -> Result<Self, ColourError> {
        let value = input.trim().strip_prefix('#').unwrap_or(input.trim());
        if value.is_empty() {
            return Err(ColourError::Empty);
        }
        if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ColourError::Hex);
        }
        let digits: Vec<u8> = value.bytes().map(hex_digit).collect();
        match digits.len() {
            3 | 4 => Ok(Self::new(digits[0] * 17, digits[1] * 17, digits[2] * 17)),
            6 | 8 => Ok(Self::new(
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
            )),
            _ => Err(ColourError::Hex),
        }
    }

    pub const fn to_rgb(self) -> Rgb {
        Rgb::new(self.r, self.g, self.b)
    }
}

impl From<Rgb> for Hex {
    fn from(colour: Rgb) -> Self {
        Self::new(colour.r, colour.g, colour.b)
    }
}

impl From<Hex> for Rgb {
    fn from(hex: Hex) -> Self {
        hex.to_rgb()
    }
}

impl fmt::Display for Hex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hsl {
    pub h: u16,
    pub s: u8,
    pub l: u8,
}

impl Hsl {
    pub const fn new(h: u16, s: u8, l: u8) -> Self {
        Self {
            h: h % 360,
            s: if s > 100 { 100 } else { s },
            l: if l > 100 { 100 } else { l },
        }
    }

    pub fn parse(input: &str) -> Result<Self, ColourError> {
        let value = input.trim();
        let lower = value.to_ascii_lowercase();
        if !lower.starts_with("hsl(") || !lower.ends_with(')') || value.len() < 5 {
            return Err(ColourError::Hsl);
        }
        let inner = &value[4..value.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() != 3 {
            return Err(ColourError::Hsl);
        }
        let h = number(parts[0], None).ok_or(ColourError::Hsl)?;
        let s = number(parts[1], Some('%')).ok_or(ColourError::Hsl)?;
        let l = number(parts[2], Some('%')).ok_or(ColourError::Hsl)?;
        if !h.is_finite() || !s.is_finite() || !l.is_finite() {
            return Err(ColourError::Hsl);
        }
        if !(0.0..=100.0).contains(&s) || !(0.0..=100.0).contains(&l) {
            return Err(ColourError::Range);
        }
        Ok(Self::from_degrees(h, s, l))
    }

    fn from_degrees(h: f64, s: f64, l: f64) -> Self {
        let h = h.round() as i64;
        let h = (h % 360 + 360) % 360;
        Self {
            h: u16::try_from(h).unwrap_or(0),
            s: round_unit(s),
            l: round_unit(l),
        }
    }

    pub fn to_rgb(self) -> Rgb {
        let h = f64::from(self.h % 360);
        let s = f64::from(self.s) / 100.0;
        let l = f64::from(self.l) / 100.0;
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let sector = h / 60.0;
        let x = c * (1.0 - ((sector % 2.0) - 1.0).abs());
        let (r, g, b) = match sector {
            value if value < 1.0 => (c, x, 0.0),
            value if value < 2.0 => (x, c, 0.0),
            value if value < 3.0 => (0.0, c, x),
            value if value < 4.0 => (0.0, x, c),
            value if value < 5.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        Rgb::new(channel(r + m), channel(g + m), channel(b + m))
    }
}

impl From<Rgb> for Hsl {
    fn from(colour: Rgb) -> Self {
        let r = f64::from(colour.r) / 255.0;
        let g = f64::from(colour.g) / 255.0;
        let b = f64::from(colour.b) / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let l = (max + min) / 2.0;
        if delta <= f64::EPSILON {
            return Self::from_degrees(0.0, 0.0, l * 100.0);
        }
        let s = delta / (1.0 - (2.0 * l - 1.0).abs());
        let degrees = if max == r {
            60.0 * (((g - b) / delta) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        Self::from_degrees(degrees, s * 100.0, l * 100.0)
    }
}

impl From<Hsl> for Rgb {
    fn from(hsl: Hsl) -> Self {
        hsl.to_rgb()
    }
}

impl fmt::Display for Hsl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "hsl({}, {}%, {}%)", self.h, self.s, self.l)
    }
}

impl Rgb {
    pub const fn from_rgba(pixel: [u8; 4]) -> Self {
        Self::new(pixel[0], pixel[1], pixel[2])
    }

    pub const fn from_bgra(pixel: [u8; 4]) -> Self {
        Self::new(pixel[2], pixel[1], pixel[0])
    }

    pub const fn to_hex(self) -> Hex {
        Hex::new(self.r, self.g, self.b)
    }

    pub fn to_hsl(self) -> Hsl {
        Hsl::from(self)
    }

    pub fn parse(input: &str) -> Result<Self, ColourError> {
        let value = input.trim();
        if value.is_empty() {
            return Err(ColourError::Empty);
        }
        let lower = value.to_ascii_lowercase();
        if lower.starts_with("rgb(") {
            parse_rgb(value)
        } else if lower.starts_with("hsl(") {
            Hsl::parse(value).map(Self::from)
        } else {
            Hex::parse(value).map(Self::from)
        }
    }

    pub fn display(self, format: ColourFormat) -> String {
        format.display(self)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "rgb({}, {}, {})", self.r, self.g, self.b)
    }
}

pub fn parse(input: &str) -> Result<Rgb, ColourError> {
    Rgb::parse(input)
}

fn parse_rgb(value: &str) -> Result<Rgb, ColourError> {
    if !value.to_ascii_lowercase().ends_with(')') || value.len() < 5 {
        return Err(ColourError::Rgb);
    }
    let inner = &value[4..value.len() - 1];
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 {
        return Err(ColourError::Rgb);
    }
    let r = parts[0]
        .trim()
        .parse::<u8>()
        .map_err(|_| ColourError::Rgb)?;
    let g = parts[1]
        .trim()
        .parse::<u8>()
        .map_err(|_| ColourError::Rgb)?;
    let b = parts[2]
        .trim()
        .parse::<u8>()
        .map_err(|_| ColourError::Rgb)?;
    Ok(Rgb::new(r, g, b))
}

fn number(part: &str, suffix: Option<char>) -> Option<f64> {
    let trimmed = part.trim();
    let trimmed = match suffix {
        Some(suffix) => trimmed.strip_suffix(suffix).unwrap_or(trimmed).trim(),
        None => trimmed.strip_suffix("deg").unwrap_or(trimmed).trim(),
    };
    trimmed.parse::<f64>().ok()
}

fn hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

fn round_unit(value: f64) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

fn channel(value: f64) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_display_is_upper_case_and_alpha_is_ignored() {
        let colour = Rgb::new(0xe5, 0x48, 0x4d);
        assert_eq!(colour.to_string(), "rgb(229, 72, 77)");
        assert_eq!(Hex::from(colour).to_string(), "#E5484D");
        assert_eq!(
            Rgb::from_rgba([0xE5, 0x48, 0x4D, 0]).display(ColourFormat::Hex),
            "#E5484D"
        );
        assert_eq!(
            Rgb::from_rgba([0xE5, 0x48, 0x4D, 255]).display(ColourFormat::Hex),
            "#E5484D"
        );
        assert_eq!(
            Rgb::from_bgra([0x4D, 0x48, 0xE5, 0]).display(ColourFormat::Hex),
            "#E5484D"
        );
    }

    #[test]
    fn known_hex_colours_convert_to_rgb_and_hsl() {
        let red = Rgb::parse("#FF0000").expect("red");
        assert_eq!(red, Rgb::new(255, 0, 0));
        assert_eq!(red.to_hsl(), Hsl::new(0, 100, 50));
        assert_eq!(red.to_hsl().to_string(), "hsl(0, 100%, 50%)");

        let green = Rgb::parse("#00FF00").expect("green");
        assert_eq!(green.to_hsl(), Hsl::new(120, 100, 50));

        let blue = Rgb::parse("#0000FF").expect("blue");
        assert_eq!(blue.to_hsl(), Hsl::new(240, 100, 50));

        let white = Rgb::parse("#FFFFFF").expect("white");
        assert_eq!(white.to_hsl(), Hsl::new(0, 0, 100));

        let black = Rgb::parse("#000000").expect("black");
        assert_eq!(black.to_hsl(), Hsl::new(0, 0, 0));

        let grey = Rgb::parse("#808080").expect("grey");
        assert_eq!(grey.to_hsl(), Hsl::new(0, 0, 50));

        let yellow = Rgb::parse("#FFD60A").expect("yellow");
        assert_eq!(yellow.to_hsl(), Hsl::new(50, 100, 52));
    }

    #[test]
    fn rgb_to_hsl_rounds_each_component() {
        let colour = Rgb::parse("#123456").expect("colour");
        assert_eq!(colour.to_hsl(), Hsl::new(210, 65, 20));
        assert_eq!(colour.to_hsl().to_string(), "hsl(210, 65%, 20%)");

        let brand = Rgb::parse("#E5484D").expect("brand");
        assert_eq!(brand.to_hsl(), Hsl::new(358, 75, 59));
        assert_eq!(brand.to_hsl().to_string(), "hsl(358, 75%, 59%)");
    }

    #[test]
    fn hsl_converts_back_to_rgb() {
        assert_eq!(Rgb::from(Hsl::new(0, 100, 50)), Rgb::new(255, 0, 0));
        assert_eq!(Rgb::from(Hsl::new(120, 100, 50)), Rgb::new(0, 255, 0));
        assert_eq!(Rgb::from(Hsl::new(240, 100, 50)), Rgb::new(0, 0, 255));
        assert_eq!(Rgb::from(Hsl::new(0, 0, 50)), Rgb::new(128, 128, 128));
        assert_eq!(Rgb::from(Hsl::new(0, 0, 0)), Rgb::new(0, 0, 0));
        assert_eq!(Rgb::from(Hsl::new(0, 0, 100)), Rgb::new(255, 255, 255));
        assert_eq!(Rgb::from(Hsl::new(210, 65, 20)), Rgb::new(18, 51, 84));
    }

    #[test]
    fn every_display_format_is_exact() {
        let colour = Rgb::new(18, 52, 86);
        assert_eq!(colour.display(ColourFormat::Hex), "#123456");
        assert_eq!(colour.display(ColourFormat::Rgb), "rgb(18, 52, 86)");
        assert_eq!(colour.display(ColourFormat::Hsl), "hsl(210, 65%, 20%)");
    }

    #[test]
    fn hsl_parsing_accepts_degrees_and_optional_percent_signs() {
        assert_eq!(Hsl::parse("hsl(210, 65%, 20%)"), Ok(Hsl::new(210, 65, 20)));
        assert_eq!(Hsl::parse("HSL(210deg, 65, 20)"), Ok(Hsl::new(210, 65, 20)));
        assert_eq!(
            Hsl::parse("hsl(210.4, 65.4%, 20.4%)"),
            Ok(Hsl::new(210, 65, 20))
        );
        assert_eq!(Hsl::parse("hsl(360, 0%, 100%)"), Ok(Hsl::new(0, 0, 100)));
    }

    #[test]
    fn hex_parsing_accepts_short_and_long_forms_and_ignores_alpha() {
        assert_eq!(Hex::parse("#abc"), Ok(Hex::new(0xAA, 0xBB, 0xCC)));
        assert_eq!(Hex::parse("aabbcc"), Ok(Hex::new(0xAA, 0xBB, 0xCC)));
        assert_eq!(Hex::parse("#abcd"), Ok(Hex::new(0xAA, 0xBB, 0xCC)));
        assert_eq!(Hex::parse("#11223344"), Ok(Hex::new(0x11, 0x22, 0x33)));
        assert_eq!(Rgb::parse("#E5484D"), Ok(Rgb::new(229, 72, 77)));
    }

    #[test]
    fn rgb_parsing_accepts_function_syntax() {
        assert_eq!(Rgb::parse("rgb(229, 72, 77)"), Ok(Rgb::new(229, 72, 77)));
        assert_eq!(Rgb::parse("RGB( 0 , 0 , 0 )"), Ok(Rgb::new(0, 0, 0)));
        assert_eq!(Rgb::parse("hsl(358, 75%, 59%)"), Ok(Rgb::new(229, 72, 77)));
    }

    #[test]
    fn invalid_input_is_rejected() {
        assert_eq!(Rgb::parse(""), Err(ColourError::Empty));
        assert_eq!(Rgb::parse("   "), Err(ColourError::Empty));
        assert_eq!(Hex::parse("#GGGGGG"), Err(ColourError::Hex));
        assert_eq!(Hex::parse("#12345"), Err(ColourError::Hex));
        assert_eq!(Hex::parse("#"), Err(ColourError::Empty));
        assert_eq!(Rgb::parse("rgb(256, 0, 0)"), Err(ColourError::Rgb));
        assert_eq!(Rgb::parse("rgb(1, 2)"), Err(ColourError::Rgb));
        assert_eq!(Rgb::parse("rgb(1, 2, 3"), Err(ColourError::Rgb));
        assert_eq!(Hsl::parse("hsl(0, 101%, 50%)"), Err(ColourError::Range));
        assert_eq!(Hsl::parse("hsl(0, 50%, -1%)"), Err(ColourError::Range));
        assert_eq!(Hsl::parse("hsl(0, 50%)"), Err(ColourError::Hsl));
        assert_eq!(Hsl::parse("nonsense"), Err(ColourError::Hsl));
        assert_eq!(Rgb::parse("nonsense"), Err(ColourError::Hex));
    }

    #[test]
    fn colour_format_labels_and_ids_round_trip() {
        for format in ColourFormat::ALL {
            assert_eq!(ColourFormat::parse(format.id()), Some(format));
            assert_eq!(ColourFormat::parse(format.label()), Some(format));
        }
        assert_eq!(ColourFormat::parse(" HEX "), Some(ColourFormat::Hex));
        assert_eq!(ColourFormat::parse("hsv"), None);
        assert_eq!(ColourFormat::default(), ColourFormat::Hex);
    }
}
