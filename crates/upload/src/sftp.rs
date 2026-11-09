use std::fmt;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use ssh2::{ErrorCode, HostKeyType, Session, Sftp};

use crate::error::UploadError;
use crate::http::HttpTimeout;
use crate::queue::{DeletionLink, SecretStore, UploadFile, Uploaded, Uploader};

pub const PASSWORD_FIELD: &str = "password";
pub const PASSPHRASE_FIELD: &str = "passphrase";
pub const DEFAULT_PORT: u16 = 22;
pub const MAX_COLLISION_ATTEMPTS: u32 = 1000;

const SFTP_NO_SUCH_FILE: i32 = 2;
const SFTP_PERMISSION_DENIED: i32 = 3;
const SFTP_FAILURE: i32 = 4;
const SFTP_NO_CONNECTION: i32 = 6;
const SFTP_CONNECTION_LOST: i32 = 7;
const SFTP_OP_UNSUPPORTED: i32 = 8;
const SFTP_NO_SUCH_PATH: i32 = 10;
const SFTP_FILE_ALREADY_EXISTS: i32 = 11;
const SFTP_WRITE_PROTECT: i32 = 12;
const SFTP_INVALID_FILENAME: i32 = 13;
const SFTP_NO_SPACE: i32 = 14;
const SFTP_QUOTA_EXCEEDED: i32 = 15;
const SFTP_DIR_NOT_EMPTY: i32 = 18;
const SFTP_NOT_A_DIRECTORY: i32 = 19;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpAuthMethod {
    Password,
    PrivateKey,
}

impl SftpAuthMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKey => "private_key",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "password" | "" => Some(Self::Password),
            "private_key" | "privatekey" | "key" => Some(Self::PrivateKey),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpConfig {
    pub dest_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SftpAuthMethod,
    pub private_key_path: Option<PathBuf>,
    pub remote_folder: String,
    pub public_url_pattern: String,
}

impl SftpConfig {
    pub fn new(
        dest_id: impl Into<String>,
        host: impl Into<String>,
        username: impl Into<String>,
    ) -> Self {
        Self {
            dest_id: dest_id.into(),
            host: host.into(),
            port: DEFAULT_PORT,
            username: username.into(),
            auth: SftpAuthMethod::Password,
            private_key_path: None,
            remote_folder: String::new(),
            public_url_pattern: String::new(),
        }
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn auth(mut self, auth: SftpAuthMethod) -> Self {
        self.auth = auth;
        self
    }

    pub fn private_key_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.private_key_path = Some(path.into());
        self
    }

    pub fn remote_folder(mut self, folder: impl Into<String>) -> Self {
        self.remote_folder = folder.into();
        self
    }

    pub fn public_url_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.public_url_pattern = pattern.into();
        self
    }

    pub fn public_url(&self, file_name: &str) -> Result<String, UploadError> {
        if self.host.trim().is_empty() {
            return Err(UploadError::Permanent("Enter the SFTP host.".to_owned()));
        }
        if self.port == 0 {
            return Err(UploadError::Permanent(
                "The SFTP port must be between 1 and 65535.".to_owned(),
            ));
        }
        if self.username.trim().is_empty() {
            return Err(UploadError::Permanent(
                "Enter the SFTP username.".to_owned(),
            ));
        }
        if !self.public_url_pattern.contains("<key>") {
            return Err(UploadError::Permanent(
                "The public URL pattern must contain <key>.".to_owned(),
            ));
        }
        Ok(self
            .public_url_pattern
            .replace("<key>", &crate::s3::sigv4::encode_component(file_name)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    pub algorithm: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyState {
    Match,
    Mismatch,
    Unknown,
}

pub trait HostKeyStore: Send + Sync {
    fn state(&self, host: &str, port: u16, key: &[u8]) -> Result<HostKeyState, UploadError>;

    fn remember(
        &self,
        host: &str,
        port: u16,
        algorithm: &str,
        key: &[u8],
    ) -> Result<(), UploadError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

pub trait HostKeyConfirmer: Send + Sync {
    fn confirm(&self, prompt: &HostKeyPrompt) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub struct RejectHostKeys;

impl HostKeyConfirmer for RejectHostKeys {
    fn confirm(&self, _prompt: &HostKeyPrompt) -> bool {
        false
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum SftpCredentials {
    Password(String),
    PrivateKey {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

impl fmt::Debug for SftpCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => formatter
                .debug_struct("SftpCredentials::Password")
                .field("password", &"***")
                .finish(),
            Self::PrivateKey { path, passphrase } => formatter
                .debug_struct("SftpCredentials::PrivateKey")
                .field("path", path)
                .field("passphrase", &passphrase.as_ref().map(|_| "***"))
                .finish(),
        }
    }
}

pub trait SftpConnection: Send {
    fn host_key(&mut self) -> Result<HostKey, UploadError>;

    fn authenticate(
        &mut self,
        username: &str,
        credentials: &SftpCredentials,
    ) -> Result<(), UploadError>;

    fn write_file(
        &mut self,
        folder: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<String, UploadError>;

    fn remove_file(&mut self, path: &str) -> Result<(), UploadError>;

    fn supports_delete(&self) -> bool {
        true
    }
}

pub trait SftpConnector: Send + Sync {
    fn connect(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<Box<dyn SftpConnection>, UploadError>;
}

pub struct Ssh2Connector;

impl SftpConnector for Ssh2Connector {
    fn connect(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<Box<dyn SftpConnection>, UploadError> {
        let address = (host, port)
            .to_socket_addrs()
            .map_err(|_| UploadError::Retryable("The server was not found.".to_owned()))?
            .next()
            .ok_or_else(|| UploadError::Retryable("The server was not found.".to_owned()))?;
        let stream = TcpStream::connect_timeout(&address, timeout)
            .map_err(|_| UploadError::Retryable("The server could not be reached.".to_owned()))?;
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(timeout));
        let mut session = Session::new().map_err(|_| {
            UploadError::Retryable("The secure connection could not start.".to_owned())
        })?;
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        session.set_timeout(timeout_ms);
        session.set_tcp_stream(stream);
        session.handshake().map_err(|_| {
            UploadError::Retryable("The secure connection could not be established.".to_owned())
        })?;
        Ok(Box::new(Ssh2Connection {
            session,
            sftp: None,
        }))
    }
}

struct Ssh2Connection {
    session: Session,
    sftp: Option<Sftp>,
}

impl Ssh2Connection {
    fn sftp(&mut self) -> Result<&Sftp, UploadError> {
        if self.sftp.is_none() {
            self.sftp = Some(
                self.session
                    .sftp()
                    .map_err(|error| map_session_error(&error))?,
            );
        }
        self.sftp
            .as_ref()
            .ok_or_else(|| UploadError::Permanent("The SFTP session could not start.".to_owned()))
    }
}

impl SftpConnection for Ssh2Connection {
    fn host_key(&mut self) -> Result<HostKey, UploadError> {
        let (bytes, kind) = self.session.host_key().ok_or_else(|| {
            UploadError::Permanent("The server did not present a host key.".to_owned())
        })?;
        let bytes = bytes.to_vec();
        let algorithm = key_algorithm(kind, &bytes);
        Ok(HostKey { algorithm, bytes })
    }

    fn authenticate(
        &mut self,
        username: &str,
        credentials: &SftpCredentials,
    ) -> Result<(), UploadError> {
        match credentials {
            SftpCredentials::Password(password) => self
                .session
                .userauth_password(username, password)
                .map_err(|error| map_session_error(&error))?,
            SftpCredentials::PrivateKey { path, passphrase } => self
                .session
                .userauth_pubkey_file(username, None, path, passphrase.as_deref())
                .map_err(|error| map_session_error(&error))?,
        }
        if self.session.authenticated() {
            Ok(())
        } else {
            Err(UploadError::Auth)
        }
    }

    fn write_file(
        &mut self,
        folder: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<String, UploadError> {
        let path = {
            let sftp = self.sftp()?;
            ensure_dirs(sftp, folder)?;
            available_path(sftp, folder, file_name)?
        };
        let sftp = self.sftp()?;
        let mut file = sftp
            .create(Path::new(&path))
            .map_err(|error| map_sftp_error(&error))?;
        if let Err(error) = file.write_all(bytes) {
            let _ = file.close();
            return Err(map_io_error(&error));
        }
        file.close().map_err(|error| map_sftp_error(&error))?;
        Ok(path)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), UploadError> {
        self.sftp()?
            .unlink(Path::new(path))
            .map_err(|error| map_sftp_error(&error))
    }
}

pub struct SftpUploader {
    config: SftpConfig,
    connector: Arc<dyn SftpConnector>,
    known_hosts: Arc<dyn HostKeyStore>,
    confirmer: Arc<dyn HostKeyConfirmer>,
    timeout: Duration,
}

impl SftpUploader {
    pub fn new(
        config: SftpConfig,
        connector: Arc<dyn SftpConnector>,
        known_hosts: Arc<dyn HostKeyStore>,
        confirmer: Arc<dyn HostKeyConfirmer>,
        timeout: Duration,
    ) -> Self {
        Self {
            config,
            connector,
            known_hosts,
            confirmer,
            timeout,
        }
    }

    pub fn from_settings(
        config: SftpConfig,
        timeout: HttpTimeout,
        known_hosts: Arc<dyn HostKeyStore>,
        confirmer: Arc<dyn HostKeyConfirmer>,
    ) -> Self {
        Self::new(
            config,
            Arc::new(Ssh2Connector),
            known_hosts,
            confirmer,
            Duration::from_secs(timeout.seconds()),
        )
    }

    pub fn config(&self) -> &SftpConfig {
        &self.config
    }

    fn credentials(&self, secrets: &dyn SecretStore) -> Result<SftpCredentials, UploadError> {
        match self.config.auth {
            SftpAuthMethod::Password => {
                let password = secrets
                    .secret(&self.config.dest_id, PASSWORD_FIELD)?
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        UploadError::Permanent("No SFTP password is saved.".to_owned())
                    })?;
                Ok(SftpCredentials::Password(password))
            }
            SftpAuthMethod::PrivateKey => {
                let path = self
                    .config
                    .private_key_path
                    .clone()
                    .filter(|path| !path.as_os_str().is_empty())
                    .ok_or_else(|| {
                        UploadError::Permanent("Choose the SFTP private key file.".to_owned())
                    })?;
                if !path.is_file() {
                    return Err(UploadError::Permanent(
                        "The SFTP private key file could not be read.".to_owned(),
                    ));
                }
                let passphrase = secrets.secret(&self.config.dest_id, PASSPHRASE_FIELD)?;
                Ok(SftpCredentials::PrivateKey { path, passphrase })
            }
        }
    }

    fn connect(&self, secrets: &dyn SecretStore) -> Result<Box<dyn SftpConnection>, UploadError> {
        let credentials = self.credentials(secrets)?;
        let mut connection =
            self.connector
                .connect(&self.config.host, self.config.port, self.timeout)?;
        let host_key = connection.host_key()?;
        let fingerprint = fingerprint_sha256(&host_key.bytes);
        let state = self
            .known_hosts
            .state(&self.config.host, self.config.port, &host_key.bytes)?;
        let mut confirmed = false;
        match state {
            HostKeyState::Match => {}
            HostKeyState::Mismatch => {
                return Err(UploadError::Permanent(
                    "The server's host key has changed, so the connection was stopped.".to_owned(),
                ));
            }
            HostKeyState::Unknown => {
                let prompt = HostKeyPrompt {
                    host: self.config.host.clone(),
                    port: self.config.port,
                    algorithm: host_key.algorithm.clone(),
                    fingerprint,
                };
                if !self.confirmer.confirm(&prompt) {
                    return Err(UploadError::Permanent(
                        "The server's host key was not accepted.".to_owned(),
                    ));
                }
                confirmed = true;
            }
        }
        connection.authenticate(&self.config.username, &credentials)?;
        if confirmed {
            self.known_hosts.remember(
                &self.config.host,
                self.config.port,
                &host_key.algorithm,
                &host_key.bytes,
            )?;
        }
        Ok(connection)
    }

    fn deletion_path(&self, link: &DeletionLink) -> Result<String, UploadError> {
        let path = if let Some(token) = link
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            token.to_owned()
        } else {
            let url = link.url.trim();
            match url.strip_prefix("sftp://") {
                Some(rest) => match rest.find('/') {
                    Some(index) => rest[index..].to_owned(),
                    None => String::new(),
                },
                None if !url.contains("://") => url.to_owned(),
                None => String::new(),
            }
        };
        if path.is_empty() || !inside_folder(&self.config.remote_folder, &path) {
            return Err(UploadError::Permanent(
                "The remote file is not inside the destination's folder.".to_owned(),
            ));
        }
        Ok(path)
    }
}

impl fmt::Debug for SftpUploader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SftpUploader")
            .field("dest_id", &self.config.dest_id)
            .field("host", &self.config.host)
            .field("port", &self.config.port)
            .field("auth", &self.config.auth)
            .finish_non_exhaustive()
    }
}

impl Uploader for SftpUploader {
    fn upload(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
    ) -> Result<Uploaded, UploadError> {
        let file_name = safe_file_name(&file.file_name);
        self.config.public_url(&file_name)?;
        let mut connection = self.connect(secrets)?;
        let remote_path =
            connection.write_file(&self.config.remote_folder, &file_name, &file.bytes)?;
        let uploaded_name = remote_path
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or(&file_name);
        let url = self.config.public_url(uploaded_name)?;
        let deletion_url = format!(
            "sftp://{}:{}/{}",
            self.config.host,
            self.config.port,
            remote_path.trim_start_matches('/')
        );
        Ok(Uploaded::new(url).with_deletion_link(
            DeletionLink::new(self.config.dest_id.clone(), deletion_url).with_token(remote_path),
        ))
    }

    fn delete(&self, link: &DeletionLink, secrets: &dyn SecretStore) -> Result<(), UploadError> {
        let path = self.deletion_path(link)?;
        let mut connection = self.connect(secrets)?;
        if !connection.supports_delete() {
            return Err(UploadError::Permanent(
                "The server does not support deleting files over SFTP.".to_owned(),
            ));
        }
        connection.remove_file(&path)
    }
}

pub fn fingerprint_sha256(key: &[u8]) -> String {
    format!("SHA256:{}", base64_encode(&Sha256::digest(key)))
}

fn safe_file_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    if name.is_empty() || name == "." || name == ".." {
        return "upload".to_owned();
    }
    name.chars()
        .map(|character| {
            if character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect()
}

fn join_remote(folder: &str, file_name: &str) -> String {
    let folder = folder.trim();
    if folder.is_empty() {
        file_name.to_owned()
    } else if folder.ends_with('/') {
        format!("{folder}{file_name}")
    } else {
        format!("{folder}/{file_name}")
    }
}

fn inside_folder(folder: &str, path: &str) -> bool {
    let folder = folder.trim().trim_end_matches('/');
    if folder.is_empty() {
        return !path.is_empty()
            && !path.contains('/')
            && !path.contains('\\')
            && path != "."
            && path != "..";
    }
    let prefix = format!("{folder}/");
    path.starts_with(&prefix)
        && path.len() > prefix.len()
        && !path[prefix.len()..].contains("..")
        && !path[prefix.len()..].contains('\\')
}

fn ensure_dirs(sftp: &Sftp, folder: &str) -> Result<(), UploadError> {
    let folder = folder.trim().trim_end_matches('/');
    if folder.is_empty() || folder == "/" {
        return Ok(());
    }
    let absolute = folder.starts_with('/');
    let mut current = String::new();
    for segment in folder.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(UploadError::Permanent(
                "The remote folder is not valid.".to_owned(),
            ));
        }
        if absolute {
            current.push('/');
        }
        current.push_str(segment);
        if sftp.stat(Path::new(&current)).is_err() {
            sftp.mkdir(Path::new(&current), 0o755)
                .map_err(|error| map_sftp_error(&error))?;
        }
    }
    Ok(())
}

fn available_path(sftp: &Sftp, folder: &str, file_name: &str) -> Result<String, UploadError> {
    let (stem, extension) = split_extension(file_name);
    for index in 0..MAX_COLLISION_ATTEMPTS {
        let candidate = if index == 0 {
            file_name.to_owned()
        } else {
            format!("{stem}-{index}{extension}")
        };
        let path = join_remote(folder, &candidate);
        match sftp.stat(Path::new(&path)) {
            Ok(_) => continue,
            Err(error) if is_missing(&error) => return Ok(path),
            Err(error) => return Err(map_sftp_error(&error)),
        }
    }
    Err(UploadError::Permanent(
        "A free file name could not be found on the server.".to_owned(),
    ))
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    }
}

fn is_missing(error: &ssh2::Error) -> bool {
    matches!(
        error.code(),
        ErrorCode::SFTP(SFTP_NO_SUCH_FILE | SFTP_NO_SUCH_PATH)
    )
}

fn key_algorithm(kind: HostKeyType, key: &[u8]) -> String {
    if let Some(value) = wire_key_algorithm(key) {
        return value;
    }
    match kind {
        HostKeyType::Rsa => "ssh-rsa",
        HostKeyType::Dss => "ssh-dss",
        HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
        HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
        HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
        HostKeyType::Ed25519 => "ssh-ed25519",
        HostKeyType::Unknown => "ssh-unknown",
    }
    .to_owned()
}

fn wire_key_algorithm(key: &[u8]) -> Option<String> {
    if key.len() < 4 {
        return None;
    }
    let length = u32::from_be_bytes([key[0], key[1], key[2], key[3]]) as usize;
    if length == 0 || length > key.len() - 4 {
        return None;
    }
    let value = std::str::from_utf8(&key[4..4 + length]).ok()?;
    if value.is_empty() || value.bytes().any(|byte| !byte.is_ascii_graphic()) {
        return None;
    }
    Some(value.to_owned())
}

fn map_session_error(error: &ssh2::Error) -> UploadError {
    let message = error.message().to_ascii_lowercase();
    if message.contains("authentication failed")
        || message.contains("password expired")
        || message.contains("publickey")
        || message.contains("keyfile auth failed")
        || message.contains("no more auth methods")
    {
        return UploadError::Auth;
    }
    if message.contains("timed out")
        || message.contains("timeout")
        || message.contains("socket")
        || message.contains("connection")
        || message.contains("kex")
        || message.contains("key exchange")
        || message.contains("banner")
        || message.contains("would block")
    {
        return UploadError::Retryable("The connection to the server failed.".to_owned());
    }
    UploadError::Permanent("The SFTP session failed.".to_owned())
}

fn map_sftp_error(error: &ssh2::Error) -> UploadError {
    match error.code() {
        ErrorCode::SFTP(SFTP_NO_SUCH_FILE | SFTP_NO_SUCH_PATH) => {
            UploadError::Permanent("The remote file or folder was not found.".to_owned())
        }
        ErrorCode::SFTP(SFTP_PERMISSION_DENIED | SFTP_WRITE_PROTECT) => UploadError::Auth,
        ErrorCode::SFTP(SFTP_NO_CONNECTION | SFTP_CONNECTION_LOST) => {
            UploadError::Retryable("The connection to the server was lost.".to_owned())
        }
        ErrorCode::SFTP(SFTP_OP_UNSUPPORTED) => {
            UploadError::Permanent("The server does not support that SFTP operation.".to_owned())
        }
        ErrorCode::SFTP(SFTP_FILE_ALREADY_EXISTS) => {
            UploadError::Permanent("The remote file already exists.".to_owned())
        }
        ErrorCode::SFTP(SFTP_INVALID_FILENAME) => {
            UploadError::Permanent("The remote file name is not valid.".to_owned())
        }
        ErrorCode::SFTP(SFTP_NO_SPACE | SFTP_QUOTA_EXCEEDED) => {
            UploadError::Permanent("The server has no room for the file.".to_owned())
        }
        ErrorCode::SFTP(SFTP_DIR_NOT_EMPTY | SFTP_NOT_A_DIRECTORY | SFTP_FAILURE) => {
            UploadError::Permanent("The remote folder could not be used.".to_owned())
        }
        _ => {
            let mapped = map_session_error(error);
            match mapped {
                UploadError::Auth => UploadError::Auth,
                UploadError::Retryable(_) => {
                    UploadError::Retryable("The connection to the server was lost.".to_owned())
                }
                other => other,
            }
        }
    }
}

fn map_io_error(error: &std::io::Error) -> UploadError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            UploadError::Retryable("The upload timed out.".to_owned())
        }
        std::io::ErrorKind::PermissionDenied => UploadError::Auth,
        std::io::ErrorKind::NotFound => {
            UploadError::Permanent("The remote folder was not found.".to_owned())
        }
        _ => UploadError::Permanent("The file could not be uploaded.".to_owned()),
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use tempfile::TempDir;

    use super::*;
    use crate::queue::SecretError;

    #[derive(Default)]
    struct StubSecrets {
        values: HashMap<(String, String), String>,
        requested: Mutex<Vec<(String, String)>>,
    }

    impl StubSecrets {
        fn with(dest_id: &str, field: &str, value: &str) -> Self {
            let mut values = HashMap::new();
            values.insert((dest_id.to_owned(), field.to_owned()), value.to_owned());
            Self {
                values,
                requested: Mutex::new(Vec::new()),
            }
        }

        fn requested(&self) -> Vec<(String, String)> {
            self.requested.lock().expect("requests").clone()
        }
    }

    impl SecretStore for StubSecrets {
        fn get(&self, dest_id: &str, field: &str) -> Result<Option<Vec<u8>>, SecretError> {
            self.requested
                .lock()
                .expect("requests")
                .push((dest_id.to_owned(), field.to_owned()));
            Ok(self
                .values
                .get(&(dest_id.to_owned(), field.to_owned()))
                .map(|value| value.as_bytes().to_vec()))
        }
    }

    struct FakeHostKeys {
        state: HostKeyState,
        remembered: Mutex<Vec<(String, u16, String, Vec<u8>)>>,
        error: Option<UploadError>,
    }

    impl FakeHostKeys {
        fn new(state: HostKeyState) -> Arc<Self> {
            Arc::new(Self {
                state,
                remembered: Mutex::new(Vec::new()),
                error: None,
            })
        }

        fn remembered(&self) -> Vec<(String, u16, String, Vec<u8>)> {
            self.remembered.lock().expect("remembered").clone()
        }
    }

    impl HostKeyStore for FakeHostKeys {
        fn state(&self, _host: &str, _port: u16, _key: &[u8]) -> Result<HostKeyState, UploadError> {
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(self.state),
            }
        }

        fn remember(
            &self,
            host: &str,
            port: u16,
            algorithm: &str,
            key: &[u8],
        ) -> Result<(), UploadError> {
            self.remembered.lock().expect("remembered").push((
                host.to_owned(),
                port,
                algorithm.to_owned(),
                key.to_vec(),
            ));
            Ok(())
        }
    }

    struct FakeConfirmer {
        accept: bool,
        prompts: Mutex<Vec<HostKeyPrompt>>,
    }

    impl FakeConfirmer {
        fn new(accept: bool) -> Arc<Self> {
            Arc::new(Self {
                accept,
                prompts: Mutex::new(Vec::new()),
            })
        }

        fn prompts(&self) -> Vec<HostKeyPrompt> {
            self.prompts.lock().expect("prompts").clone()
        }
    }

    impl HostKeyConfirmer for FakeConfirmer {
        fn confirm(&self, prompt: &HostKeyPrompt) -> bool {
            self.prompts.lock().expect("prompts").push(prompt.clone());
            self.accept
        }
    }

    struct FakeState {
        connect_error: Option<UploadError>,
        host_key: HostKey,
        auth_error: Option<UploadError>,
        write_error: Option<UploadError>,
        remote_path: Option<String>,
        delete_error: Option<UploadError>,
        supports_delete: bool,
        connects: Vec<(String, u16, Duration)>,
        auths: Vec<(String, SftpCredentials)>,
        writes: Vec<(String, String, Vec<u8>)>,
        deletes: Vec<String>,
    }

    impl FakeState {
        fn new() -> Self {
            Self {
                connect_error: None,
                host_key: HostKey {
                    algorithm: "ssh-ed25519".to_owned(),
                    bytes: b"server-host-key".to_vec(),
                },
                auth_error: None,
                write_error: None,
                remote_path: None,
                delete_error: None,
                supports_delete: true,
                connects: Vec::new(),
                auths: Vec::new(),
                writes: Vec::new(),
                deletes: Vec::new(),
            }
        }
    }

    struct FakeConnector {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeConnector {
        fn new() -> (Arc<Self>, Arc<Mutex<FakeState>>) {
            let state = Arc::new(Mutex::new(FakeState::new()));
            (
                Arc::new(Self {
                    state: state.clone(),
                }),
                state,
            )
        }
    }

    impl SftpConnector for FakeConnector {
        fn connect(
            &self,
            host: &str,
            port: u16,
            timeout: Duration,
        ) -> Result<Box<dyn SftpConnection>, UploadError> {
            let mut state = self.state.lock().expect("state");
            state.connects.push((host.to_owned(), port, timeout));
            if let Some(error) = &state.connect_error {
                return Err(error.clone());
            }
            drop(state);
            Ok(Box::new(FakeConnection {
                state: self.state.clone(),
            }))
        }
    }

    struct FakeConnection {
        state: Arc<Mutex<FakeState>>,
    }

    impl SftpConnection for FakeConnection {
        fn host_key(&mut self) -> Result<HostKey, UploadError> {
            Ok(self.state.lock().expect("state").host_key.clone())
        }

        fn authenticate(
            &mut self,
            username: &str,
            credentials: &SftpCredentials,
        ) -> Result<(), UploadError> {
            let mut state = self.state.lock().expect("state");
            state.auths.push((username.to_owned(), credentials.clone()));
            match &state.auth_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn write_file(
            &mut self,
            folder: &str,
            file_name: &str,
            bytes: &[u8],
        ) -> Result<String, UploadError> {
            let mut state = self.state.lock().expect("state");
            state
                .writes
                .push((folder.to_owned(), file_name.to_owned(), bytes.to_vec()));
            if let Some(error) = &state.write_error {
                return Err(error.clone());
            }
            Ok(state
                .remote_path
                .clone()
                .unwrap_or_else(|| join_remote(folder, file_name)))
        }

        fn remove_file(&mut self, path: &str) -> Result<(), UploadError> {
            let mut state = self.state.lock().expect("state");
            state.deletes.push(path.to_owned());
            match &state.delete_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn supports_delete(&self) -> bool {
            self.state.lock().expect("state").supports_delete
        }
    }

    fn config(folder: &str) -> SftpConfig {
        SftpConfig::new("sftp", "example.com", "aaron")
            .port(2222)
            .remote_folder(folder)
            .public_url_pattern("https://example.test/shots/<key>")
    }

    fn file() -> UploadFile {
        UploadFile::new(
            PathBuf::from("C:\\captures\\shot.png"),
            "shot.png",
            "image/png",
            vec![1_u8, 2, 3],
        )
    }

    fn uploader(
        config: SftpConfig,
        connector: Arc<FakeConnector>,
        known: Arc<FakeHostKeys>,
        confirmer: Arc<FakeConfirmer>,
    ) -> SftpUploader {
        SftpUploader::new(config, connector, known, confirmer, Duration::from_secs(30))
    }

    #[test]
    fn fingerprints_are_sha256_in_openssh_form() {
        assert_eq!(
            fingerprint_sha256(b"hello"),
            "SHA256:LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ"
        );
        assert_eq!(
            fingerprint_sha256(b""),
            "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU"
        );
    }

    #[test]
    fn upload_connects_authenticates_and_returns_the_public_url() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Unknown);
        let confirmer = FakeConfirmer::new(true);
        let secrets = StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse");
        let uploaded = uploader(
            config("/srv/shots"),
            connector,
            known.clone(),
            confirmer.clone(),
        )
        .upload(&file(), &secrets)
        .expect("upload");

        let state = state.lock().expect("state");
        assert_eq!(state.connects.len(), 1);
        assert_eq!(state.connects[0].0, "example.com");
        assert_eq!(state.connects[0].1, 2222);
        assert_eq!(state.connects[0].2, Duration::from_secs(30));
        assert_eq!(state.auths.len(), 1);
        assert_eq!(state.auths[0].0, "aaron");
        assert_eq!(
            state.auths[0].1,
            SftpCredentials::Password("correct horse".to_owned())
        );
        assert_eq!(state.writes.len(), 1);
        assert_eq!(state.writes[0].0, "/srv/shots");
        assert_eq!(state.writes[0].1, "shot.png");
        assert_eq!(state.writes[0].2, vec![1_u8, 2, 3]);
        assert_eq!(state.host_key.bytes, known.remembered()[0].3);
        drop(state);

        assert_eq!(uploaded.link, "https://example.test/shots/shot.png");
        let deletion = uploaded.deletion_link.expect("deletion link");
        assert_eq!(deletion.dest, "sftp");
        assert_eq!(deletion.token.as_deref(), Some("/srv/shots/shot.png"));
        assert_eq!(deletion.url, "sftp://example.com:2222/srv/shots/shot.png");
        let prompts = confirmer.prompts();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].host, "example.com");
        assert_eq!(prompts[0].port, 2222);
        assert_eq!(prompts[0].algorithm, "ssh-ed25519");
        assert!(prompts[0].fingerprint.starts_with("SHA256:"));
        assert_eq!(known.remembered().len(), 1);
        assert_eq!(known.remembered()[0].0, "example.com");
        assert_eq!(known.remembered()[0].1, 2222);
        assert_eq!(known.remembered()[0].2, "ssh-ed25519");
        assert_eq!(
            secrets.requested(),
            vec![("sftp".to_owned(), PASSWORD_FIELD.to_owned())]
        );
    }

    #[test]
    fn a_saved_matching_host_key_skips_the_prompt_and_remember() {
        let (connector, _state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let secrets = StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse");
        uploader(
            config("/srv/shots"),
            connector,
            known.clone(),
            confirmer.clone(),
        )
        .upload(&file(), &secrets)
        .expect("upload");
        assert!(confirmer.prompts().is_empty());
        assert!(known.remembered().is_empty());
    }

    #[test]
    fn a_changed_host_key_stops_before_authentication() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Mismatch);
        let confirmer = FakeConfirmer::new(true);
        let secrets = StubSecrets::with("sftp", PASSWORD_FIELD, "very secret");
        let error = uploader(config("/srv/shots"), connector, known, confirmer)
            .upload(&file(), &secrets)
            .expect_err("mismatch");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("host key"));
        assert!(!error.to_string().contains("very secret"));
        assert!(!format!("{error:?}").contains("very secret"));
        assert!(state.lock().expect("state").auths.is_empty());
    }

    #[test]
    fn declining_a_new_host_key_stops_before_authentication() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Unknown);
        let confirmer = FakeConfirmer::new(false);
        let secrets = StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse");
        let error = uploader(config("/srv/shots"), connector, known.clone(), confirmer)
            .upload(&file(), &secrets)
            .expect_err("declined");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("not accepted"));
        assert!(state.lock().expect("state").auths.is_empty());
        assert!(known.remembered().is_empty());
    }

    #[test]
    fn a_failed_authentication_does_not_remember_a_new_host_key() {
        let (connector, state) = FakeConnector::new();
        state.lock().expect("state").auth_error = Some(UploadError::Auth);
        let known = FakeHostKeys::new(HostKeyState::Unknown);
        let confirmer = FakeConfirmer::new(true);
        let secrets = StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse");
        let error = uploader(config("/srv/shots"), connector, known.clone(), confirmer)
            .upload(&file(), &secrets)
            .expect_err("auth");
        assert_eq!(error, UploadError::Auth);
        assert!(known.remembered().is_empty());
    }

    #[test]
    fn a_missing_password_is_permanent_and_never_connects() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let error = uploader(config("/srv/shots"), connector, known, confirmer)
            .upload(&file(), &StubSecrets::default())
            .expect_err("missing password");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("password"));
        assert!(state.lock().expect("state").connects.is_empty());
    }

    #[test]
    fn private_key_auth_reads_the_passphrase_and_uses_the_key_path() {
        let directory = TempDir::new().expect("temp directory");
        let key_path = directory.path().join("id_ed25519");
        std::fs::write(&key_path, "not a real key").expect("key file");
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let secrets = StubSecrets::with("sftp", PASSPHRASE_FIELD, "key secret");
        let mut config = config("/srv/shots").auth(SftpAuthMethod::PrivateKey);
        config.private_key_path = Some(key_path.clone());
        uploader(config, connector, known, confirmer)
            .upload(&file(), &secrets)
            .expect("upload");
        let state = state.lock().expect("state");
        assert_eq!(
            state.auths[0].1,
            SftpCredentials::PrivateKey {
                path: key_path,
                passphrase: Some("key secret".to_owned()),
            }
        );
        assert_eq!(
            secrets.requested(),
            vec![("sftp".to_owned(), PASSPHRASE_FIELD.to_owned())]
        );
    }

    #[test]
    fn a_missing_private_key_is_permanent_and_never_connects() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let config = config("/srv/shots")
            .auth(SftpAuthMethod::PrivateKey)
            .private_key_path("C:\\keys\\missing");
        let error = uploader(config, connector, known, confirmer)
            .upload(&file(), &StubSecrets::default())
            .expect_err("missing key");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("key file"));
        assert!(state.lock().expect("state").connects.is_empty());
    }

    #[test]
    fn connection_failures_are_retryable() {
        let (connector, state) = FakeConnector::new();
        state.lock().expect("state").connect_error = Some(UploadError::Retryable(
            "The server could not be reached.".to_owned(),
        ));
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let error = uploader(config("/srv/shots"), connector, known, confirmer)
            .upload(
                &file(),
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect_err("connection");
        assert!(error.is_retryable());
    }

    #[test]
    fn an_invalid_public_url_pattern_is_permanent_and_never_connects() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let config = config("/srv/shots").public_url_pattern("https://example.test/no-token");
        let error = uploader(config, connector, known, confirmer)
            .upload(
                &file(),
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect_err("pattern");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("<key>"));
        assert!(state.lock().expect("state").connects.is_empty());
    }

    #[test]
    fn a_collision_remote_name_is_used_for_the_public_url() {
        let (connector, state) = FakeConnector::new();
        state.lock().expect("state").remote_path = Some("/srv/shots/shot-1.png".to_owned());
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let uploaded = uploader(config("/srv/shots"), connector, known, confirmer)
            .upload(
                &file(),
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect("upload");
        assert_eq!(uploaded.link, "https://example.test/shots/shot-1.png");
        assert_eq!(
            uploaded.deletion_link.expect("deletion").token.as_deref(),
            Some("/srv/shots/shot-1.png")
        );
    }

    #[test]
    fn file_names_cannot_escape_the_remote_folder() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let file = UploadFile::new(
            PathBuf::from("C:\\captures\\shot.png"),
            "..\\..\\escape.png",
            "image/png",
            vec![1_u8],
        );
        uploader(config("/srv/shots"), connector, known, confirmer)
            .upload(
                &file,
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect("upload");
        let state = state.lock().expect("state");
        assert_eq!(state.writes[0].1, "escape.png");
    }

    #[test]
    fn delete_removes_a_file_inside_the_remote_folder() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let link = DeletionLink::new("sftp", "sftp://example.com:2222/srv/shots/shot.png")
            .with_token("/srv/shots/shot.png");
        uploader(config("/srv/shots"), connector, known, confirmer)
            .delete(
                &link,
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect("delete");
        assert_eq!(
            state.lock().expect("state").deletes,
            vec!["/srv/shots/shot.png".to_owned()]
        );
    }

    #[test]
    fn delete_reads_the_path_from_a_deletion_url() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let link = DeletionLink::new("sftp", "sftp://example.com:2222/srv/shots/shot.png");
        uploader(config("/srv/shots"), connector, known, confirmer)
            .delete(
                &link,
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect("delete");
        assert_eq!(
            state.lock().expect("state").deletes,
            vec!["/srv/shots/shot.png".to_owned()]
        );
    }

    #[test]
    fn delete_refuses_paths_outside_the_remote_folder() {
        let (connector, state) = FakeConnector::new();
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let link = DeletionLink::new("sftp", "/etc/passwd").with_token("/etc/passwd");
        let error = uploader(config("/srv/shots"), connector, known, confirmer)
            .delete(
                &link,
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect_err("outside folder");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(state.lock().expect("state").connects.is_empty());
    }

    #[test]
    fn delete_reports_an_unsupported_server() {
        let (connector, state) = FakeConnector::new();
        state.lock().expect("state").supports_delete = false;
        let known = FakeHostKeys::new(HostKeyState::Match);
        let confirmer = FakeConfirmer::new(false);
        let link =
            DeletionLink::new("sftp", "/srv/shots/shot.png").with_token("/srv/shots/shot.png");
        let error = uploader(config("/srv/shots"), connector, known, confirmer)
            .delete(
                &link,
                &StubSecrets::with("sftp", PASSWORD_FIELD, "correct horse"),
            )
            .expect_err("unsupported");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("does not support deleting"));
    }

    #[test]
    fn a_relative_remote_folder_allows_only_a_single_file_name() {
        assert!(inside_folder("", "shot.png"));
        assert!(!inside_folder("", "folder/shot.png"));
        assert!(inside_folder("shots", "shots/shot.png"));
        assert!(!inside_folder("shots", "shots/../secret"));
        assert!(!inside_folder("/srv/shots", "/srv/other/shot.png"));
    }

    #[test]
    fn credentials_debug_never_prints_a_secret() {
        let password = SftpCredentials::Password("top secret".to_owned());
        let key = SftpCredentials::PrivateKey {
            path: PathBuf::from("C:\\keys\\id_ed25519"),
            passphrase: Some("key secret".to_owned()),
        };
        assert!(!format!("{password:?}").contains("top secret"));
        assert!(!format!("{key:?}").contains("key secret"));
        assert!(format!("{password:?}").contains("***"));
    }
}
