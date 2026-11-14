use super::{
    config::Provider,
    journal::{BrowserJournal, BrowserProof},
};
use crate::{api::AppState, auth};
use dunlin_mcp_client::oauth::{AuthContext, AuthorizationFlow, persistence::PersistenceError};
use dunlin_vault::{
    oauth::OAuthCredentials,
    store::{SecretStore, ServiceKeychainStore},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const LIFETIME: Duration = Duration::from_secs(300);
struct Pending {
    context: AuthContext,
    proof: BrowserProof,
    provider: String,
    config_digest: String,
    state_hash: String,
    flow: AuthorizationFlow,
    created: Instant,
}
#[derive(Default)]
struct Records {
    pending: HashMap<String, Pending>,
    starting: HashMap<String, String>,
}
pub(crate) struct Service {
    records: Mutex<Records>,
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub slots: Arc<tokio::sync::Semaphore>,
    pub runtime_slots: Arc<tokio::sync::Semaphore>,
    pub cleanup_slots: Arc<tokio::sync::Semaphore>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            records: Mutex::new(Records::default()),
            secrets: Arc::new(ServiceKeychainStore::relay()),
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
            runtime_slots: Arc::new(tokio::sync::Semaphore::new(8)),
            cleanup_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}
struct Starting<'a> {
    records: &'a Mutex<Records>,
    session: String,
}
impl Drop for Starting<'_> {
    fn drop(&mut self) {
        if let Ok(mut records) = self.records.lock() {
            records.starting.remove(&self.session);
        }
    }
}
pub(crate) struct Started {
    pub authorization_url: String,
    pub context: AuthContext,
    pub browser: Zeroizing<String>,
}
pub(crate) struct Callback(Pending);
impl Service {
    #[cfg(test)]
    pub fn testing(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            secrets,
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub fn expire_pending(&self) {
        for pending in self.records.lock().unwrap().pending.values_mut() {
            pending.created = Instant::now() - LIFETIME - Duration::from_secs(1);
        }
    }
    fn reserve(
        &self,
        state: &AppState,
        proof: &BrowserProof,
    ) -> Result<Starting<'_>, PersistenceError> {
        let mut records = self.records.lock().map_err(|_| PersistenceError::Storage)?;
        records
            .pending
            .retain(|_, p| p.created.elapsed() < LIFETIME);
        let previous = records
            .pending
            .iter()
            .find(|(_, p)| p.proof.session_hash == proof.session_hash)
            .map(|(id, p)| (id.clone(), p.context.clone(), p.flow.credential_binding()));
        if let Some((id, context, binding)) = previous {
            let store = state.store.lock().map_err(|_| PersistenceError::Storage)?;
            match store.mcp_oauth_authorizing(&context, &binding) {
                Ok(()) => return Err(PersistenceError::Conflict),
                Err(PersistenceError::Conflict) => {
                    records.pending.remove(&id);
                }
                Err(error) => return Err(error),
            }
        }
        if records.pending.len() + records.starting.len() >= 32 {
            return Err(PersistenceError::Limit);
        }
        if records.starting.contains_key(&proof.session_hash) {
            return Err(PersistenceError::Conflict);
        }
        let owned = records
            .pending
            .values()
            .filter(|p| p.proof.user == proof.user)
            .count()
            + records
                .starting
                .values()
                .filter(|user| *user == &proof.user)
                .count();
        if owned >= 4 {
            return Err(PersistenceError::Limit);
        }
        records
            .starting
            .insert(proof.session_hash.clone(), proof.user.clone());
        Ok(Starting {
            records: &self.records,
            session: proof.session_hash.clone(),
        })
    }
    pub fn start(
        &self,
        state: &AppState,
        provider: &Provider,
        mut context: AuthContext,
        proof: BrowserProof,
        display_name: Option<String>,
    ) -> Result<Started, PersistenceError> {
        {
            let store = state.store.lock().map_err(|_| PersistenceError::Storage)?;
            proof.validate(state, &store, &context)?;
            store.mcp_oauth_can_authorize(&context)?;
        }
        let _starting = self.reserve(state, &proof)?;
        let flow = provider
            .flow_authorized(
                self.secrets.as_ref(),
                dunlin_mcp_client::DispatchGuard::new(Arc::new({
                    let state = state.clone();
                    let proof = proof.clone();
                    let context = context.clone();
                    move || {
                        let Ok(store) = state.store.lock() else {
                            return false;
                        };
                        proof.validate(&state, &store, &context).is_ok()
                            && store.mcp_oauth_can_authorize(&context).is_ok()
                    }
                })),
            )
            .map_err(|_| PersistenceError::Storage)?;
        {
            let store = state.store.lock().map_err(|_| PersistenceError::Storage)?;
            proof.validate(state, &store, &context)?;
            context.connection_revision = store.mcp_oauth_prepare(
                &context,
                &flow.credential_binding(),
                &provider.id,
                display_name.as_deref(),
                &|store, context| proof.validate(state, store, context),
            )?;
        }
        let request = flow.begin(context.clone(), provider.scopes.clone())?;
        let state_hash = state_hash(&request.url)?;
        let browser = Zeroizing::new(auth::random_token());
        let mut records = self.records.lock().map_err(|_| PersistenceError::Storage)?;
        let browser_hash = auth::hash_token(&browser);
        if records.pending.contains_key(&browser_hash) {
            return Err(PersistenceError::Limit);
        }
        records.pending.insert(
            browser_hash,
            Pending {
                context: context.clone(),
                proof,
                provider: provider.id.clone(),
                config_digest: provider.digest(),
                state_hash,
                flow,
                created: Instant::now(),
            },
        );
        Ok(Started {
            authorization_url: request.url,
            context,
            browser,
        })
    }
    pub fn take(
        &self,
        provider: &Provider,
        browser: &str,
        callback: &str,
    ) -> Result<Callback, PersistenceError> {
        let key = auth::hash_token(browser);
        let state_hash = state_hash(callback)?;
        let mut records = self.records.lock().map_err(|_| PersistenceError::Storage)?;
        records
            .pending
            .retain(|_, p| p.created.elapsed() < LIFETIME);
        let record = records
            .pending
            .get(&key)
            .ok_or(PersistenceError::Unauthorized)?;
        if record.provider != provider.id
            || record.config_digest != provider.digest()
            || record.state_hash != state_hash
        {
            return Err(PersistenceError::Unauthorized);
        }
        Ok(Callback(
            records
                .pending
                .remove(&key)
                .ok_or(PersistenceError::Unauthorized)?,
        ))
    }
    pub fn finish(
        &self,
        state: AppState,
        record: Callback,
        callback: &str,
    ) -> Result<(), PersistenceError> {
        let mut pending = record.0;
        let journal = Arc::new(BrowserJournal {
            membership_revision: None,
            state,
            proof: pending.proof,
        });
        journal.authorizing(&pending.context, &pending.flow.credential_binding())?;
        let _writer = journal.writer_lease(&pending.context)?;
        journal.authorizing(&pending.context, &pending.flow.credential_binding())?;
        let binding = pending.flow.credential_binding();
        pending.flow = pending
            .flow
            .with_token_guard(dunlin_mcp_client::DispatchGuard::new(Arc::new({
                let journal = journal.clone();
                let context = pending.context.clone();
                move || journal.authorizing(&context, &binding).is_ok()
            })));
        let token = pending.flow.finish(&pending.context, callback)?;
        OAuthCredentials::new(journal, self.secrets.clone()).install(
            &pending.flow,
            &pending.context,
            token,
        )?;
        Ok(())
    }
}
fn state_hash(uri: &str) -> Result<String, PersistenceError> {
    if uri.len() > 16384 {
        return Err(PersistenceError::Limit);
    }
    let query = uri
        .split_once('?')
        .map(|(_, q)| q)
        .ok_or(PersistenceError::Unauthorized)?;
    let mut result = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "state" {
            if value.is_empty() || value.len() > 128 || result.is_some() {
                return Err(PersistenceError::Unauthorized);
            }
            result = Some(auth::hash_token(&value));
        }
    }
    result.ok_or(PersistenceError::Unauthorized)
}
