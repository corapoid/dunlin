use super::*;
use crate::store::MemoryStore;
use dunlin_mcp_client::oauth::persistence::CredentialIntent;

struct NoJournal;
impl CredentialJournal for NoJournal {
    fn active(&self, _: &AuthContext, _: &str) -> Result<CredentialRecord, PersistenceError> {
        panic!("unexpected journal access")
    }
    fn begin_install(
        &self,
        _: &AuthContext,
        _: &str,
    ) -> Result<CredentialIntent, PersistenceError> {
        panic!("unexpected journal access")
    }
    fn begin_refresh(
        &self,
        _: &AuthContext,
        _: &str,
        _: &CredentialRecord,
    ) -> Result<CredentialIntent, PersistenceError> {
        panic!("unexpected journal access")
    }
    fn prepare_write(
        &self,
        _: &AuthContext,
        _: &str,
        _: &CredentialIntent,
        _: &str,
    ) -> Result<(), PersistenceError> {
        panic!("unexpected journal access")
    }
    fn claim_write(
        &self,
        _: &AuthContext,
        _: &str,
        _: &CredentialIntent,
        _: &str,
    ) -> Result<(), PersistenceError> {
        panic!("unexpected journal access")
    }
    fn acknowledge_write(
        &self,
        _: &AuthContext,
        _: &str,
        _: &CredentialIntent,
        _: &str,
    ) -> Result<(), PersistenceError> {
        panic!("unexpected journal access")
    }
    fn commit(
        &self,
        _: &AuthContext,
        _: &str,
        _: &CredentialIntent,
        _: &str,
    ) -> Result<(), PersistenceError> {
        panic!("unexpected journal access")
    }
    fn disable(&self, _: &AuthContext) -> Result<u64, PersistenceError> {
        panic!("unexpected journal access")
    }
}
const REFERENCE: &str = "mcp-oauth:01234567-89ab-7cde-8123-456789abcdef";
#[test]
fn oauth_adapter_isolates_records_from_general_secret_aliases_and_policies() {
    let secrets = Arc::new(MemoryStore::new());
    secrets.set("secret-http", "private-http-key").unwrap();
    secrets.set_policy("secret-http", "http-policy").unwrap();
    let credentials = OAuthCredentials::new(Arc::new(NoJournal), secrets.clone());
    for reference in [
        "secret-http",
        "mcp-oauth:../secret-http",
        "mcp-oauth:01234567-89ab-7cde-8123-456789abcdef\n",
        "mcp-oauth:01234567-89ab-7cde-8123-456789abcdeF",
    ] {
        assert!(credentials.read(reference).is_err());
        assert!(credentials.write(reference, "overwrite").is_err());
    }
    credentials
        .write(REFERENCE, "private-oauth-envelope")
        .unwrap();
    assert_eq!(
        &*credentials.read(REFERENCE).unwrap(),
        "private-oauth-envelope"
    );
    assert!(
        secrets.get_policy(REFERENCE).is_err(),
        "OAuth never registers a vault tool policy"
    );
    assert_eq!(&*secrets.get("secret-http").unwrap(), "private-http-key");
    assert_eq!(&*secrets.get_policy("secret-http").unwrap(), "http-policy");
}
#[test]
fn oauth_adapter_rejects_oversized_secret_records_before_write_and_after_read() {
    let secrets = Arc::new(MemoryStore::new());
    let credentials = OAuthCredentials::new(Arc::new(NoJournal), secrets.clone());
    let oversized = Zeroizing::new("x".repeat(65537));
    assert!(matches!(
        credentials.write(REFERENCE, &oversized),
        Err(PersistenceError::Limit)
    ));
    assert!(secrets.get(REFERENCE).is_err());
    secrets.set(REFERENCE, &oversized).unwrap();
    assert!(matches!(
        credentials.read(REFERENCE),
        Err(PersistenceError::Limit)
    ));
}
struct Unavailable;
impl SecretStore for Unavailable {
    fn get(&self, _: &str) -> Result<Zeroizing<String>, String> {
        Err("fixture-private-backend-error".into())
    }
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("fixture-private-backend-error".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        panic!("no cleanup without writer fencing")
    }
    fn get_policy(&self, _: &str) -> Result<Zeroizing<String>, String> {
        panic!("OAuth must not access vault policies")
    }
    fn set_policy(&self, _: &str, _: &str) -> Result<(), String> {
        panic!("OAuth must not access vault policies")
    }
    fn delete_policy(&self, _: &str) -> Result<(), String> {
        panic!("OAuth must not access vault policies")
    }
}
#[test]
fn oauth_adapter_storage_errors_do_not_expose_backend_messages_or_fall_back_to_other_storage() {
    let credentials = OAuthCredentials::new(Arc::new(NoJournal), Arc::new(Unavailable));
    for error in [
        credentials.read(REFERENCE).err().unwrap(),
        credentials
            .write(REFERENCE, "fixture-private-value")
            .err()
            .unwrap(),
    ] {
        assert!(matches!(error, PersistenceError::Storage));
        assert!(!error.to_string().contains("fixture-private"));
        assert!(!format!("{error:?}").contains("fixture-private"));
    }
}
