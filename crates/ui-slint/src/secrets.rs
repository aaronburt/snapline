use std::collections::HashMap;
use std::sync::Mutex;

use snapline_core::ports::{SecretError, SecretStore};
use snapline_core::secret::{Secret, SecretKey};

#[derive(Default)]
pub struct MemorySecrets {
    items: Mutex<HashMap<SecretKey, Secret>>,
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        let items = self.items.lock().map_err(|_| SecretError::Read)?;
        Ok(items.get(key).cloned())
    }

    fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError> {
        let mut items = self.items.lock().map_err(|_| SecretError::Write)?;
        items.insert(key.clone(), value.clone());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let mut items = self.items.lock().map_err(|_| SecretError::Delete)?;
        items.remove(key);
        Ok(())
    }
}
