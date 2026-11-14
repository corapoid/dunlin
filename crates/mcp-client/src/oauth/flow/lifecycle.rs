//! Explicit refresh and revocation. Positive refresh never reuses a token after
//! a potentially rotating request with an uncertain outcome.
use super::{AuthContext, AuthorizationFlow, CredentialStatus, Error, TokenSet};
use zeroize::Zeroize;

impl AuthorizationFlow {
    pub(super) fn bound(&self, context: &AuthContext, token: &TokenSet) -> Result<(), Error> {
        context.validate()?;
        if token.context != *context
            || token.issuer != self.issuer
            || token.endpoint != self.endpoint
            || token.resource != self.resource
            || token.client_id != self.registration.client_id
        {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    /// Rotates in place. Failure after admission leaves the credential blocked;
    /// reauthorization or explicit negative cleanup is required, never a retry.
    pub fn refresh(&self, context: &AuthContext, token: &mut TokenSet) -> Result<(), Error> {
        crate::authorize(self.token_guard.as_ref())?;
        self.bound(context, token)?;
        if token.status != CredentialStatus::Active {
            return Err(Error::Unauthorized);
        }
        let refresh = token.refresh_token.as_ref().ok_or(Error::Unsupported)?;
        token.status = CredentialStatus::RefreshUncertain;
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "refresh_token")
            .append_pair("refresh_token", &refresh.0)
            .append_pair("resource", &self.resource);
        if !token.scopes.is_empty() {
            form.append_pair("scope", &token.scopes.join(" "));
        }
        let validated = self.tokens(form, &token.scopes)?;
        token.access_token = validated.response.access_token;
        if let Some(refresh) = validated.response.refresh_token {
            token.refresh_token = Some(refresh);
        }
        token.scopes = validated.scopes;
        token.expires = validated.expires;
        token.status = CredentialStatus::Active;
        Ok(())
    }

    /// Disable locally before contacting the AS. Unconfirmed remote revocation
    /// retains secrets only for an explicit, idempotent cleanup retry.
    pub fn revoke(&self, context: &AuthContext, token: &mut TokenSet) -> Result<(), Error> {
        self.revoke_checked(context, token, || Ok(()))
    }

    pub(super) fn revoke_checked(
        &self,
        context: &AuthContext,
        token: &mut TokenSet,
        mut check: impl FnMut() -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.bound(context, token)?;
        if token.status == CredentialStatus::Revoked {
            return Ok(());
        }
        token.status = CredentialStatus::RevocationUnconfirmed;
        let endpoint = self
            .revocation_endpoint
            .as_ref()
            .ok_or(Error::Unsupported)?;
        if let Some(refresh) = &token.refresh_token {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("token", &refresh.0)
                .append_pair("token_type_hint", "refresh_token");
            check()?;
            self.post_form(endpoint, form)?;
            check()?;
        }
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("token", &token.access_token.0)
            .append_pair("token_type_hint", "access_token");
        check()?;
        self.post_form(endpoint, form)?;
        check()?;
        token.access_token.0.zeroize();
        token.refresh_token = None;
        token.status = CredentialStatus::Revoked;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
