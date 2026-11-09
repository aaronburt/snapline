use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

pub const FILE_NAME: &str = "known_hosts";
pub const DEFAULT_PORT: u16 = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyStatus {
    Match,
    Mismatch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHostEntry {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub key: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum KnownHostsError {
    #[error("The SnapLine data folder could not be found.")]
    NoDataFolder,
    #[error("The saved host keys could not be read.")]
    Read(#[source] io::Error),
    #[error("The saved host keys could not be written.")]
    Write(#[source] io::Error),
    #[error("The host key is not valid.")]
    Invalid,
    #[error("The saved host key does not match the server's key.")]
    Mismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn discover() -> Result<Self, KnownHostsError> {
        default_path()
            .map(Self::new)
            .ok_or(KnownHostsError::NoDataFolder)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> Result<Vec<KnownHostEntry>, KnownHostsError> {
        load(&self.path)
    }

    pub fn status(
        &self,
        host: &str,
        port: u16,
        key: &[u8],
    ) -> Result<HostKeyStatus, KnownHostsError> {
        Ok(check(&self.entries()?, host, port, key))
    }

    pub fn remember(
        &self,
        host: &str,
        port: u16,
        algorithm: &str,
        key: &[u8],
    ) -> Result<(), KnownHostsError> {
        if host.trim().is_empty() || key.is_empty() {
            return Err(KnownHostsError::Invalid);
        }
        if algorithm.is_empty() || algorithm.bytes().any(|byte| !byte.is_ascii_graphic()) {
            return Err(KnownHostsError::Invalid);
        }
        let entries = self.entries()?;
        match check(&entries, host, port, key) {
            HostKeyStatus::Match => return Ok(()),
            HostKeyStatus::Mismatch => return Err(KnownHostsError::Mismatch),
            HostKeyStatus::Unknown => {}
        }
        let mut text = match fs::read(&self.path) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|_| KnownHostsError::Invalid)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(KnownHostsError::Read(error)),
        };
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!(
            "{} {} {}\n",
            host_key(host, port),
            algorithm,
            base64_encode(key)
        ));
        write_atomic(&self.path, text.as_bytes()).map_err(KnownHostsError::Write)
    }
}

pub fn default_path() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|folder| folder.join("SnapLine").join(FILE_NAME))
}

pub fn host_key(host: &str, port: u16) -> String {
    if port == DEFAULT_PORT {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

pub fn load(path: &Path) -> Result<Vec<KnownHostEntry>, KnownHostsError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(parse(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(KnownHostsError::Read(error)),
    }
}

pub fn parse(text: &str) -> Vec<KnownHostEntry> {
    text.lines().flat_map(parse_line).collect()
}

pub fn check(entries: &[KnownHostEntry], host: &str, port: u16, key: &[u8]) -> HostKeyStatus {
    let mut found = false;
    for entry in entries {
        if entry.port == port && host_matches(&entry.host, host) {
            found = true;
            if entry.key == key {
                return HostKeyStatus::Match;
            }
        }
    }
    if found {
        HostKeyStatus::Mismatch
    } else {
        HostKeyStatus::Unknown
    }
}

pub fn fingerprint(key: &[u8]) -> String {
    format!("SHA256:{}", base64_encode(&sha256(key)))
}

fn parse_line(line: &str) -> Vec<KnownHostEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
        return Vec::new();
    }
    let mut parts = line.split_whitespace();
    let Some(hosts) = parts.next() else {
        return Vec::new();
    };
    let Some(algorithm) = parts.next() else {
        return Vec::new();
    };
    let Some(encoded) = parts.next() else {
        return Vec::new();
    };
    if algorithm.is_empty() {
        return Vec::new();
    }
    let Some(key) = base64_decode(encoded) else {
        return Vec::new();
    };
    if key.is_empty() {
        return Vec::new();
    }
    hosts
        .split(',')
        .filter_map(parse_host_token)
        .map(|(host, port)| KnownHostEntry {
            host,
            port,
            algorithm: algorithm.to_owned(),
            key: key.clone(),
        })
        .collect()
}

fn parse_host_token(token: &str) -> Option<(String, u16)> {
    let token = token.trim();
    if token.is_empty() || token.starts_with('|') {
        return None;
    }
    if let Some(rest) = token.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        let port = match tail.strip_prefix(':') {
            Some(value) => value.parse::<u16>().ok()?,
            None if tail.is_empty() => DEFAULT_PORT,
            None => return None,
        };
        if port == 0 {
            return None;
        }
        return Some((host.to_owned(), port));
    }
    if let Some((host, value)) = token.rsplit_once(':') {
        if !host.is_empty() && !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
        {
            if let Ok(port) = value.parse::<u16>() {
                if port != 0 {
                    return Some((host.to_owned(), port));
                }
            }
        }
    }
    Some((token.to_owned(), DEFAULT_PORT))
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    if pattern == host {
        return true;
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return false;
    }
    wildcard(pattern.as_bytes(), host.as_bytes())
}

fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' => {
            wildcard(&pattern[1..], text) || (!text.is_empty() && wildcard(pattern, &text[1..]))
        }
        b'?' => !text.is_empty() && wildcard(&pattern[1..], &text[1..]),
        byte => !text.is_empty() && byte == text[0] && wildcard(&pattern[1..], &text[1..]),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(ALPHABET[usize::from(first >> 2)]));
        encoded.push(char::from(
            ALPHABET[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        if chunk.len() > 1 {
            encoded.push(char::from(
                ALPHABET[usize::from(((second & 0x0f) << 2) | (third >> 6))],
            ));
        }
        if chunk.len() > 2 {
            encoded.push(char::from(ALPHABET[usize::from(third & 0x3f)]));
        }
    }
    encoded
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut values = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        values.push(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        });
    }
    if values.len() % 4 == 1 {
        return None;
    }
    let mut decoded = Vec::with_capacity(values.len() * 3 / 4);
    for chunk in values.chunks(4) {
        match chunk.len() {
            4 => {
                decoded.push((chunk[0] << 2) | (chunk[1] >> 4));
                decoded.push((chunk[1] << 4) | (chunk[2] >> 2));
                decoded.push((chunk[2] << 6) | chunk[3]);
            }
            3 => {
                decoded.push((chunk[0] << 2) | (chunk[1] >> 4));
                decoded.push((chunk[1] << 4) | (chunk[2] >> 2));
            }
            2 => decoded.push((chunk[0] << 2) | (chunk[1] >> 4)),
            _ => {}
        }
    }
    Some(decoded)
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_length = (data.len() as u64).wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    for chunk in message.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[index * 4],
                chunk[index * 4 + 1],
                chunk[index * 4 + 2],
                chunk[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let first = words[index - 15];
            let second = words[index - 2];
            let sigma0 = first.rotate_right(7) ^ first.rotate_right(18) ^ (first >> 3);
            let sigma1 = second.rotate_right(17) ^ second.rotate_right(19) ^ (second >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    let mut digest = [0_u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn key() -> Vec<u8> {
        b"host-key-bytes".to_vec()
    }

    fn encoded_key() -> String {
        base64_encode(&key())
    }

    #[test]
    fn host_key_uses_brackets_only_for_non_default_ports() {
        assert_eq!(host_key("example.com", 22), "example.com");
        assert_eq!(host_key("example.com", 2222), "[example.com]:2222");
        assert_eq!(host_key("10.0.0.5", 22), "10.0.0.5");
        assert_eq!(host_key("10.0.0.5", 2200), "[10.0.0.5]:2200");
    }

    #[test]
    fn parses_openssh_lines_and_skips_comments_and_markers() {
        let text = format!(
            "# comment\n\n@cert-authority *.example.com ssh-ed25519 {}\n[example.com]:2222 ssh-ed25519 {}\nexample.com,www.example.com ssh-rsa {}\nhashed |1|salt|hash ssh-ed25519 {}\n",
            encoded_key(),
            encoded_key(),
            encoded_key(),
            encoded_key()
        );
        let entries = parse(&text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].host, "example.com");
        assert_eq!(entries[0].port, 2222);
        assert_eq!(entries[1].host, "example.com");
        assert_eq!(entries[1].port, 22);
        assert_eq!(entries[2].host, "www.example.com");
        assert_eq!(entries[2].algorithm, "ssh-rsa");
        assert_eq!(entries[2].key, key());
    }

    #[test]
    fn check_matches_mismatches_and_unknown_hosts() {
        let entries = parse(&format!(
            "[example.com]:2222 ssh-ed25519 {}\n",
            encoded_key()
        ));
        assert_eq!(
            check(&entries, "example.com", 2222, &key()),
            HostKeyStatus::Match
        );
        assert_eq!(
            check(&entries, "example.com", 2222, b"other-key"),
            HostKeyStatus::Mismatch
        );
        assert_eq!(
            check(&entries, "example.com", 22, &key()),
            HostKeyStatus::Unknown
        );
        assert_eq!(
            check(&entries, "other.example.com", 2222, &key()),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn wildcard_patterns_match_hosts() {
        let entries = parse(&format!("*.example.com ssh-ed25519 {}\n", encoded_key()));
        assert_eq!(
            check(&entries, "cdn.example.com", 22, &key()),
            HostKeyStatus::Match
        );
        assert_eq!(
            check(&entries, "example.com", 22, &key()),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn remember_adds_an_unknown_key_after_confirmation() {
        let directory = TempDir::new().expect("temp directory");
        let path = directory.path().join(FILE_NAME);
        let store = Store::new(&path);
        assert_eq!(
            store.status("example.com", 22, &key()).expect("status"),
            HostKeyStatus::Unknown
        );
        store
            .remember("example.com", 22, "ssh-ed25519", &key())
            .expect("remember");
        let text = fs::read_to_string(&path).expect("host keys");
        assert!(text.contains(&format!("example.com ssh-ed25519 {}", encoded_key())));
        assert_eq!(
            store.status("example.com", 22, &key()).expect("status"),
            HostKeyStatus::Match
        );
        assert!(!directory.path().join("known_hosts.tmp").exists());
    }

    #[test]
    fn remember_never_replaces_a_different_key() {
        let directory = TempDir::new().expect("temp directory");
        let path = directory.path().join(FILE_NAME);
        let store = Store::new(&path);
        store
            .remember("example.com", 22, "ssh-ed25519", &key())
            .expect("remember");
        let before = fs::read_to_string(&path).expect("host keys");
        let error = store
            .remember("example.com", 22, "ssh-ed25519", b"different-key")
            .expect_err("mismatch");
        assert!(matches!(error, KnownHostsError::Mismatch));
        assert_eq!(fs::read_to_string(&path).expect("host keys"), before);
    }

    #[test]
    fn remember_keeps_existing_comments_and_lines() {
        let directory = TempDir::new().expect("temp directory");
        let path = directory.path().join(FILE_NAME);
        fs::write(&path, "# keep me\nother.example.com ssh-rsa b3RoZXI=").expect("host keys");
        let store = Store::new(&path);
        store
            .remember("example.com", 2222, "ssh-ed25519", &key())
            .expect("remember");
        let text = fs::read_to_string(&path).expect("host keys");
        assert!(text.starts_with("# keep me\nother.example.com ssh-rsa b3RoZXI=\n"));
        assert!(text.contains(&format!("[example.com]:2222 ssh-ed25519 {}", encoded_key())));
    }

    #[test]
    fn fingerprints_are_sha256_in_openssh_form() {
        assert_eq!(
            fingerprint(b"hello"),
            "SHA256:LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ"
        );
        assert_eq!(
            fingerprint(b""),
            "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU"
        );
    }

    #[test]
    fn base64_round_trips() {
        for value in [
            b"a".as_slice(),
            b"ab".as_slice(),
            b"abc".as_slice(),
            b"host-key-bytes".as_slice(),
        ] {
            let encoded = base64_encode(value);
            assert_eq!(base64_decode(&encoded).expect("decode").as_slice(), value);
        }
        assert_eq!(
            base64_decode("b3RoZXI=").expect("padding").as_slice(),
            b"other".as_slice()
        );
    }

    #[test]
    fn invalid_entries_are_skipped() {
        let entries = parse("example.com ssh-ed25519 !!!\n[not-a-host ssh-rsa b3RoZXI=\n");
        assert!(entries.is_empty());
    }
}
