use snapline_core::secret::{Secret, SecretKey};
use thiserror::Error;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_NOT_FOUND, FILETIME};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS,
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("the credential value is too large for Credential Manager")]
    ValueTooLarge,
    #[error("Credential Manager could not read the credential")]
    Read(#[source] windows::core::Error),
    #[error("Credential Manager could not write the credential")]
    Write(#[source] windows::core::Error),
    #[error("Credential Manager could not delete the credential")]
    Delete(#[source] windows::core::Error),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CredentialStore;

impl CredentialStore {
    pub fn new() -> Self {
        Self
    }

    pub fn get(&self, key: &SecretKey) -> Result<Option<Secret>, CredentialError> {
        read_target(&key.target()).map_err(CredentialError::Read)
    }

    pub fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), CredentialError> {
        write_target(&key.target(), value)
    }

    pub fn delete(&self, key: &SecretKey) -> Result<(), CredentialError> {
        delete_target(&key.target())
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read_target(target: &str) -> Result<Option<Secret>, windows::core::Error> {
    let target = wide(target);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: the target buffer is null-terminated and the output pointer is valid for the call.
    let status = unsafe {
        CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            None,
            &mut credential,
        )
    };
    if let Err(error) = status {
        if error.code() == ERROR_NOT_FOUND.to_hresult() {
            return Ok(None);
        }
        return Err(error);
    }
    if credential.is_null() {
        return Ok(None);
    }
    let credential = CredentialBuffer(credential);
    Ok(Some(credential.secret()))
}

fn write_target(target: &str, value: &Secret) -> Result<(), CredentialError> {
    let Ok(blob_size) = u32::try_from(value.len()) else {
        return Err(CredentialError::ValueTooLarge);
    };
    if blob_size > CRED_MAX_CREDENTIAL_BLOB_SIZE {
        return Err(CredentialError::ValueTooLarge);
    }
    let target = wide(target);
    let user_name = wide(SecretKey::USER_NAME);
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_ptr().cast_mut()),
        Comment: PWSTR::null(),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: blob_size,
        CredentialBlob: if value.is_empty() {
            std::ptr::null_mut()
        } else {
            value.expose().as_ptr().cast_mut()
        },
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user_name.as_ptr().cast_mut()),
    };
    // SAFETY: every pointer in the credential describes a live buffer that outlives the call.
    unsafe { CredWriteW(&credential, 0) }.map_err(CredentialError::Write)
}

fn delete_target(target: &str) -> Result<(), CredentialError> {
    let target = wide(target);
    // SAFETY: the target buffer is null-terminated and lives for the duration of the call.
    let status = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) };
    match status {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(error) => Err(CredentialError::Delete(error)),
    }
}

struct CredentialBuffer(*mut CREDENTIALW);

impl CredentialBuffer {
    fn secret(&self) -> Secret {
        // SAFETY: the buffer came from CredReadW and is freed by this value's Drop after the copy.
        unsafe {
            let credential = &*self.0;
            let length = credential.CredentialBlobSize as usize;
            if credential.CredentialBlob.is_null() || length == 0 {
                Secret::new(Vec::new())
            } else {
                Secret::from_bytes(std::slice::from_raw_parts(
                    credential.CredentialBlob,
                    length,
                ))
            }
        }
    }
}

impl Drop for CredentialBuffer {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }
        // SAFETY: the pointer came from CredReadW and is freed exactly once here after zeroing.
        unsafe {
            let credential = &*self.0;
            let length = credential.CredentialBlobSize as usize;
            if !credential.CredentialBlob.is_null() && length > 0 {
                std::ptr::write_bytes(credential.CredentialBlob, 0, length);
            }
            CredFree(self.0.cast());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    const TEST_PREFIX: &str = "SnapLineTest:";

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct TestTarget {
        name: String,
    }

    impl TestTarget {
        fn new(label: &str) -> Self {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!("{TEST_PREFIX}{}:{label}:{id}", std::process::id());
            let _ = delete_target(&name);
            Self { name }
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    impl Drop for TestTarget {
        fn drop(&mut self) {
            let _ = delete_target(&self.name);
        }
    }

    fn expect_ok<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("credential operation failed: {error:?}"),
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn a_written_secret_reads_back_and_deletes() {
        let target = TestTarget::new("round-trip");
        let value = Secret::from("s3cret-value");
        expect_ok(write_target(target.name(), &value));
        let stored = expect_ok(read_target(target.name()));
        let stored = match stored {
            Some(secret) => secret,
            None => panic!("the written credential was not found"),
        };
        assert_eq!(stored, value);
        expect_ok(delete_target(target.name()));
        let gone = expect_ok(read_target(target.name()));
        assert!(gone.is_none());
    }

    #[test]
    #[ignore = "desktop"]
    fn a_missing_secret_reads_as_none() {
        let target = TestTarget::new("missing");
        let stored = expect_ok(read_target(target.name()));
        assert!(stored.is_none());
    }

    #[test]
    #[ignore = "desktop"]
    fn deleting_a_missing_secret_is_not_an_error() {
        let target = TestTarget::new("delete-missing");
        expect_ok(delete_target(target.name()));
        let stored = expect_ok(read_target(target.name()));
        assert!(stored.is_none());
    }
}
