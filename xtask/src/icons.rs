use std::f32::consts::PI;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

const VIEWBOX: f32 = 24.0;
const STROKE: f32 = 1.6;
const SUPERSAMPLE: usize = 4;

const MENU_ICONS: [&str; 22] = [
    "region", "window", "screen", "record", "gif", "cursor", "timer", "redo", "colour", "ocr",
    "ruler", "qr", "edit", "pin", "open", "gear", "pause", "play", "power", "check", "server",
    "cloud",
];

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

impl Point {
    const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

enum Token {
    Cmd(char),
    Num(f32),
}

fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(&current) = chars.peek() {
        if current.is_whitespace() || current == ',' {
            chars.next();
            continue;
        }
        if current.is_ascii_alphabetic() {
            tokens.push(Token::Cmd(current));
            chars.next();
            continue;
        }
        let mut number = String::new();
        if current == '-' || current == '+' {
            number.push(current);
            chars.next();
        }
        let mut dot = false;
        while let Some(&next) = chars.peek() {
            if next.is_ascii_digit() {
                number.push(next);
                chars.next();
            } else if next == '.' && !dot {
                dot = true;
                number.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if number.is_empty() || number == "-" || number == "+" {
            anyhow::bail!("unsupported path data near {source}");
        }
        let value: f32 = number
            .parse()
            .with_context(|| format!("unsupported number {number} in {source}"))?;
        tokens.push(Token::Num(value));
    }
    Ok(tokens)
}

fn number(tokens: &[Token], index: &mut usize) -> Result<f32> {
    match tokens.get(*index) {
        Some(Token::Num(value)) => {
            *index += 1;
            Ok(*value)
        }
        _ => anyhow::bail!("path data is missing a number"),
    }
}

fn cubic(from: Point, one: Point, two: Point, to: Point) -> Vec<Point> {
    let mut points = Vec::with_capacity(24);
    for step in 1..=24 {
        let t = step as f32 / 24.0;
        let inverse = 1.0 - t;
        let a = inverse * inverse * inverse;
        let b = 3.0 * inverse * inverse * t;
        let c = 3.0 * inverse * t * t;
        let d = t * t * t;
        points.push(Point::new(
            a * from.x + b * one.x + c * two.x + d * to.x,
            a * from.y + b * one.y + c * two.y + d * to.y,
        ));
    }
    points
}

fn quadratic(from: Point, control: Point, to: Point) -> Vec<Point> {
    let mut points = Vec::with_capacity(16);
    for step in 1..=16 {
        let t = step as f32 / 16.0;
        let inverse = 1.0 - t;
        let a = inverse * inverse;
        let b = 2.0 * inverse * t;
        let c = t * t;
        points.push(Point::new(
            a * from.x + b * control.x + c * to.x,
            a * from.y + b * control.y + c * to.y,
        ));
    }
    points
}

fn arc(
    from: Point,
    rx: f32,
    ry: f32,
    rotation: f32,
    large: bool,
    sweep: bool,
    to: Point,
) -> Vec<Point> {
    if (from.x - to.x).abs() < f32::EPSILON && (from.y - to.y).abs() < f32::EPSILON {
        return Vec::new();
    }
    let mut rx = rx.abs();
    let mut ry = ry.abs();
    let phi = rotation * PI / 180.0;
    let cosine = phi.cos();
    let sine = phi.sin();
    let half_x = (from.x - to.x) / 2.0;
    let half_y = (from.y - to.y) / 2.0;
    let x1 = cosine * half_x + sine * half_y;
    let y1 = -sine * half_x + cosine * half_y;
    let scale = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if scale > 1.0 {
        let factor = scale.sqrt();
        rx *= factor;
        ry *= factor;
    }
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let numerator = rx2 * ry2 - rx2 * y1 * y1 - ry2 * x1 * x1;
    let denominator = rx2 * y1 * y1 + ry2 * x1 * x1;
    let factor =
        if large == sweep { -1.0 } else { 1.0 } * (numerator / denominator).max(0.0).sqrt();
    let centre_x = factor * rx * y1 / ry;
    let centre_y = -factor * ry * x1 / rx;
    let cx = cosine * centre_x - sine * centre_y + (from.x + to.x) / 2.0;
    let cy = sine * centre_x + cosine * centre_y + (from.y + to.y) / 2.0;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let determinant = ux * vy - uy * vx;
        let dot = ux * vx + uy * vy;
        determinant.atan2(dot)
    };
    let ux = (x1 - centre_x) / rx;
    let uy = (y1 - centre_y) / ry;
    let vx = (-x1 - centre_x) / rx;
    let vy = (-y1 - centre_y) / ry;
    let start = angle(1.0, 0.0, ux, uy);
    let mut sweep_angle = angle(ux, uy, vx, vy);
    if !sweep && sweep_angle > 0.0 {
        sweep_angle -= 2.0 * PI;
    } else if sweep && sweep_angle < 0.0 {
        sweep_angle += 2.0 * PI;
    }
    let steps = ((sweep_angle.abs() / (PI / 16.0)).ceil() as usize).max(4);
    let mut points = Vec::with_capacity(steps);
    for step in 1..=steps {
        let theta = start + sweep_angle * step as f32 / steps as f32;
        points.push(Point::new(
            cx + cosine * rx * theta.cos() - sine * ry * theta.sin(),
            cy + sine * rx * theta.cos() + cosine * ry * theta.sin(),
        ));
    }
    points
}

fn flatten(source: &str) -> Result<Vec<Vec<Point>>> {
    let tokens = tokenize(source)?;
    let mut paths = Vec::new();
    let mut points: Vec<Point> = Vec::new();
    let mut x = 0.0;
    let mut y = 0.0;
    let mut start = Point::new(0.0, 0.0);
    let mut command: Option<char> = None;
    let mut last_cubic: Option<Point> = None;
    let mut last_quadratic: Option<Point> = None;
    let mut index = 0;
    while index < tokens.len() {
        if let Token::Cmd(letter) = tokens[index] {
            index += 1;
            if letter == 'Z' || letter == 'z' {
                if !points.is_empty() {
                    if points.last() != Some(&start) {
                        points.push(start);
                    }
                    paths.push(std::mem::take(&mut points));
                }
                x = start.x;
                y = start.y;
                command = None;
                last_cubic = None;
                last_quadratic = None;
                continue;
            }
            command = Some(letter);
            continue;
        }
        let letter = command.context("path data starts without a command")?;
        let relative = letter.is_ascii_lowercase();
        let upper = letter.to_ascii_uppercase();
        match upper {
            'M' => {
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                if !points.is_empty() {
                    paths.push(std::mem::take(&mut points));
                }
                x = nx;
                y = ny;
                start = Point::new(x, y);
                points.push(start);
                command = Some(if relative { 'l' } else { 'L' });
                last_cubic = None;
                last_quadratic = None;
            }
            'L' => {
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                x = nx;
                y = ny;
                points.push(Point::new(x, y));
                last_cubic = None;
                last_quadratic = None;
            }
            'H' => {
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                x = nx;
                points.push(Point::new(x, y));
                last_cubic = None;
                last_quadratic = None;
            }
            'V' => {
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                y = ny;
                points.push(Point::new(x, y));
                last_cubic = None;
                last_quadratic = None;
            }
            'C' => {
                let one = Point::new(
                    number(&tokens, &mut index)? + if relative { x } else { 0.0 },
                    number(&tokens, &mut index)? + if relative { y } else { 0.0 },
                );
                let two = Point::new(
                    number(&tokens, &mut index)? + if relative { x } else { 0.0 },
                    number(&tokens, &mut index)? + if relative { y } else { 0.0 },
                );
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                let from = Point::new(x, y);
                let to = Point::new(nx, ny);
                points.extend(cubic(from, one, two, to));
                x = nx;
                y = ny;
                last_cubic = Some(two);
                last_quadratic = None;
            }
            'S' => {
                let one = match last_cubic {
                    Some(control) => Point::new(2.0 * x - control.x, 2.0 * y - control.y),
                    None => Point::new(x, y),
                };
                let two = Point::new(
                    number(&tokens, &mut index)? + if relative { x } else { 0.0 },
                    number(&tokens, &mut index)? + if relative { y } else { 0.0 },
                );
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                let from = Point::new(x, y);
                let to = Point::new(nx, ny);
                points.extend(cubic(from, one, two, to));
                x = nx;
                y = ny;
                last_cubic = Some(two);
                last_quadratic = None;
            }
            'Q' => {
                let control = Point::new(
                    number(&tokens, &mut index)? + if relative { x } else { 0.0 },
                    number(&tokens, &mut index)? + if relative { y } else { 0.0 },
                );
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                let from = Point::new(x, y);
                let to = Point::new(nx, ny);
                points.extend(quadratic(from, control, to));
                x = nx;
                y = ny;
                last_quadratic = Some(control);
                last_cubic = None;
            }
            'T' => {
                let control = match last_quadratic {
                    Some(previous) => Point::new(2.0 * x - previous.x, 2.0 * y - previous.y),
                    None => Point::new(x, y),
                };
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                let from = Point::new(x, y);
                let to = Point::new(nx, ny);
                points.extend(quadratic(from, control, to));
                x = nx;
                y = ny;
                last_quadratic = Some(control);
                last_cubic = None;
            }
            'A' => {
                let rx = number(&tokens, &mut index)?;
                let ry = number(&tokens, &mut index)?;
                let rotation = number(&tokens, &mut index)?;
                let large = number(&tokens, &mut index)? != 0.0;
                let sweep = number(&tokens, &mut index)? != 0.0;
                let nx = number(&tokens, &mut index)? + if relative { x } else { 0.0 };
                let ny = number(&tokens, &mut index)? + if relative { y } else { 0.0 };
                let from = Point::new(x, y);
                let to = Point::new(nx, ny);
                points.extend(arc(from, rx, ry, rotation, large, sweep, to));
                x = nx;
                y = ny;
                last_cubic = None;
                last_quadratic = None;
            }
            other => anyhow::bail!("unsupported path command {other}"),
        }
    }
    if !points.is_empty() {
        paths.push(points);
    }
    Ok(paths)
}

fn distance(point: Point, from: Point, to: Point) -> f32 {
    let vx = to.x - from.x;
    let vy = to.y - from.y;
    let length = vx * vx + vy * vy;
    let t = if length <= f32::EPSILON {
        0.0
    } else {
        (((point.x - from.x) * vx + (point.y - from.y) * vy) / length).clamp(0.0, 1.0)
    };
    let dx = point.x - (from.x + t * vx);
    let dy = point.y - (from.y + t * vy);
    (dx * dx + dy * dy).sqrt()
}

fn coverage(
    paths: &[Vec<Point>],
    pixel_x: usize,
    pixel_y: usize,
    scale: f32,
    half_width: f32,
) -> f32 {
    let sample = Point::new(
        (pixel_x as f32 + 0.5) / scale,
        (pixel_y as f32 + 0.5) / scale,
    );
    let mut nearest = f32::MAX;
    for path in paths {
        for pair in path.windows(2) {
            nearest = nearest.min(distance(sample, pair[0], pair[1]));
        }
    }
    (half_width + 0.5 / scale - nearest) * scale
}

fn render(source: &str, size: usize, colour: [u8; 3]) -> Result<Vec<u8>> {
    let paths = flatten(source)?;
    let width = size * SUPERSAMPLE;
    let scale = width as f32 / VIEWBOX;
    let half_width = STROKE / 2.0;
    let mut pixels = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let mut total = 0.0;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let alpha = coverage(
                        &paths,
                        x * SUPERSAMPLE + sx,
                        y * SUPERSAMPLE + sy,
                        scale,
                        half_width,
                    );
                    total += alpha.clamp(0.0, 1.0);
                }
            }
            let alpha = total / (SUPERSAMPLE * SUPERSAMPLE) as f32;
            let offset = (y * size + x) * 4;
            pixels[offset] = colour[0];
            pixels[offset + 1] = colour[1];
            pixels[offset + 2] = colour[2];
            pixels[offset + 3] = (alpha * 255.0).round() as u8;
        }
    }
    Ok(pixels)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn png(size: usize, pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(size * (size * 4 + 1));
    for row in 0..size {
        raw.push(0);
        raw.extend_from_slice(&pixels[row * size * 4..(row + 1) * size * 4]);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut offset = 0;
    while offset < raw.len() {
        let end = (offset + 65535).min(raw.len());
        let block = &raw[offset..end];
        let final_block = end == raw.len();
        zlib.push(u8::from(final_block));
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
        offset = end;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut header = Vec::new();
    header.extend_from_slice(&(size as u32).to_be_bytes());
    header.extend_from_slice(&(size as u32).to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header, &mut out);
    chunk(b"IDAT", &zlib, &mut out);
    chunk(b"IEND", &[], &mut out);
    out
}

fn read_icons(root: &Path) -> Result<Vec<(String, String)>> {
    let path = root.join("crates/ui-slint/ui/icons.slint");
    let source =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut icons = Vec::new();
    for line in source.lines() {
        let Some(rest) = line.trim().strip_prefix("out property <string> ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches(';').trim();
        let Some(path_data) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        icons.push((camel(name.trim()), path_data.to_owned()));
    }
    if icons.is_empty() {
        anyhow::bail!("crates/ui-slint/ui/icons.slint has no icons");
    }
    Ok(icons)
}

fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for character in name.chars() {
        if character == '-' {
            upper = true;
        } else if upper {
            out.extend(character.to_uppercase());
            upper = false;
        } else {
            out.push(character);
        }
    }
    out
}

fn find_icon(icons: &[(String, String)], name: &str) -> Result<String> {
    icons
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, path)| path.clone())
        .with_context(|| format!("crates/ui-slint/ui/icons.slint has no {name} icon"))
}

fn colours() -> [(&'static str, [u8; 3]); 2] {
    [("dark", [0xd0, 0xd0, 0xd0]), ("light", [0x3b, 0x3b, 0x3b])]
}

fn output_path(root: &Path, name: &str, size: usize, theme: &str) -> std::path::PathBuf {
    root.join(format!(
        "apps/snapline/icons/menu/{name}-{size}-{theme}.png"
    ))
}

pub fn generate(root: &Path) -> Result<()> {
    let icons = read_icons(root)?;
    let dir = root.join("apps/snapline/icons/menu");
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut written = 0;
    for name in MENU_ICONS {
        let source = find_icon(&icons, name)?;
        for size in [16, 32] {
            for (theme, colour) in colours() {
                let pixels = render(&source, size, colour)?;
                let path = output_path(root, name, size, theme);
                fs::write(&path, png(size, &pixels))
                    .with_context(|| format!("cannot write {}", path.display()))?;
                written += 1;
            }
        }
    }
    println!("icons: wrote {written} files to {}", dir.display());
    Ok(())
}

pub fn check(root: &Path) -> Result<()> {
    let icons = read_icons(root)?;
    let mut stale = Vec::new();
    for name in MENU_ICONS {
        let source = find_icon(&icons, name)?;
        for size in [16, 32] {
            for (theme, colour) in colours() {
                let pixels = render(&source, size, colour)?;
                let expected = png(size, &pixels);
                let path = output_path(root, name, size, theme);
                match fs::read(&path) {
                    Ok(current) if current == expected => {}
                    Ok(_) => stale.push(format!("{} is out of date", path.display())),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        stale.push(format!("{} is missing", path.display()));
                    }
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("cannot read {}", path.display()));
                    }
                }
            }
        }
    }
    if !stale.is_empty() {
        for problem in &stale {
            eprintln!("icons: {problem}");
        }
        anyhow::bail!("icons are out of date; run cargo xtask icons");
    }
    println!("icons: up to date");
    Ok(())
}
