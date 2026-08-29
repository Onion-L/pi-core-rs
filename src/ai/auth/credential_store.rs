//! Port of `pi-core/ai/src/auth/credential-store.ts`: the in-memory default
//! credential store with per-provider serialized writes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Mutex as AsyncMutex;

use super::types::{
    AuthFuture, AuthOperationOptions, AuthStorageError, Credential, CredentialInfo, CredentialStore,
};

struct Inner {
    credentials: Mutex<HashMap<String, Credential>>,
    locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl Inner {
    fn lock_for(&self, provider_id: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(locks.entry(provider_id.to_string()).or_default())
    }

    fn read_sync(&self, provider_id: &str) -> Option<Credential> {
        self.credentials
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(provider_id)
            .cloned()
    }

    fn write_sync(&self, provider_id: &str, credential: Option<Credential>) {
        let mut map = self
            .credentials
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match credential {
            Some(credential) => {
                map.insert(provider_id.to_string(), credential);
            }
            None => {
                map.remove(provider_id);
            }
        }
    }
}

/// Port of `InMemoryCredentialStore`. Writes are serialized per provider via
/// a per-provider async mutex, standing in for the TypeScript promise chain.
#[derive(Clone, Default)]
pub struct InMemoryCredentialStore {
    inner: Arc<Inner>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            credentials: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
        }
    }
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn read(
        &self,
        provider_id: &str,
        _options: Option<&AuthOperationOptions>,
    ) -> AuthFuture<Result<Option<Credential>, AuthStorageError>> {
        let inner = Arc::clone(&self.inner);
        let provider_id = provider_id.to_string();
        Box::pin(async move { Ok(inner.read_sync(&provider_id)) })
    }

    fn list(
        &self,
        _options: Option<&AuthOperationOptions>,
    ) -> AuthFuture<Result<Vec<CredentialInfo>, AuthStorageError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            let map = inner
                .credentials
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(map
                .iter()
                .map(|(provider_id, credential)| CredentialInfo {
                    provider_id: provider_id.clone(),
                    credential_type: credential.type_name().to_string(),
                })
                .collect())
        })
    }

    fn modify(
        &self,
        provider_id: &str,
        modify: super::types::ModifyFn,
        _options: Option<&AuthOperationOptions>,
    ) -> AuthFuture<Result<Option<Credential>, super::types::BoxedAuthError>> {
        let inner = Arc::clone(&self.inner);
        let lock = inner.lock_for(provider_id);
        let provider_id = provider_id.to_string();
        Box::pin(async move {
            let _guard = lock.lock().await;
            let current = inner.read_sync(&provider_id);
            let next = modify(current).await?;
            inner.write_sync(&provider_id, next.clone());
            Ok(next)
        })
    }

    fn delete(
        &self,
        provider_id: &str,
        _options: Option<&AuthOperationOptions>,
    ) -> AuthFuture<Result<(), AuthStorageError>> {
        let inner = Arc::clone(&self.inner);
        let lock = inner.lock_for(provider_id);
        let provider_id = provider_id.to_string();
        Box::pin(async move {
            let _guard = lock.lock().await;
            inner.write_sync(&provider_id, None);
            Ok(())
        })
    }
}
