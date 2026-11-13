//! Dedicated Lax flow cookie permits AS navigation while the main browser
//! credential stays Strict. Only its captured hash remains in the pending flow.
use super::*;
use crate::{
    auth,
    mcp_oauth::{BrowserProof, config::Provider},
};
use axum::{
    extract::OriginalUri,
    http::{HeaderValue, header},
    response::Redirect,
};
use zeroize::Zeroizing;
const FLOW_COOKIE: &str = "dunlin_mcp_oauth_flow";
fn flow_cookie(value: &str, secure: bool, ttl: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{FLOW_COOKIE}={value}; Path=/v1/web/mcp/oauth; HttpOnly; SameSite=Lax; Max-Age={ttl}{}",
        if secure { "; Secure" } else { "" }
    ))
    .expect("generated flow cookie")
}
fn browser(headers: &HeaderMap) -> Result<Zeroizing<String>, ApiError> {
    let mut found = None;
    for raw in headers.get_all(header::COOKIE) {
        for part in raw.to_str().map_err(|_| ApiError::forbidden())?.split(';') {
            if let Some((name, value)) = part.trim().split_once('=')
                && name == FLOW_COOKIE
            {
                if found.is_some()
                    || value.len() != 47
                    || !value.starts_with("kst_")
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                {
                    return Err(ApiError::forbidden());
                }
                found = Some(Zeroizing::new(value.to_owned()));
            }
        }
    }
    found.ok_or_else(ApiError::forbidden)
}
fn configured(state: &AppState, headers: &HeaderMap, id: &str) -> Result<Provider, ApiError> {
    request_origin(headers, false)?;
    crate::mcp_oauth::config::validate_registry(&state.config.mcp_oauth)
        .map_err(|_| ApiError::internal("Invalid MCP provider configuration"))?;
    let provider = state
        .config
        .mcp_oauth
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(ApiError::not_found)?;
    let redirect: axum::http::Uri = provider
        .redirect_uri
        .parse()
        .map_err(|_| ApiError::internal("Invalid MCP callback"))?;
    if headers.get(header::HOST).and_then(|h| h.to_str().ok())
        != redirect.authority().map(|a| a.as_str())
    {
        return Err(ApiError::forbidden());
    }
    Ok(provider.clone())
}
pub(in crate::api) async fn providers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> Result<Response, ApiError> {
    let result = owner(&state, &headers, false, |store, user| {
        if !store
            .team_membership(&workspace, user)?
            .is_some_and(|m| m.active)
        {
            return Err(ApiError::forbidden());
        }
        crate::mcp_oauth::config::validate_registry(&state.config.mcp_oauth)
            .map_err(|_| ApiError::internal("Invalid MCP provider configuration"))?;
        Ok(state
            .config
            .mcp_oauth
            .iter()
            .map(|p| json!({"id":p.id,"name":p.name}))
            .collect::<Vec<_>>())
    })?;
    Ok(private_response(Json(json!({"providers":result}))))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct Start {
    connection_id: Option<String>,
    expected_revision: Option<u64>,
    display_name: Option<String>,
}
pub(in crate::api) async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workspace, provider)): Path<(String, String)>,
    Json(request): Json<Start>,
) -> Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let provider = configured(&state, &headers, &provider)?;
    let secure = provider.redirect_uri.starts_with("https:");
    let display_name = request.display_name.map(|name| name.trim().to_owned());
    if display_name.as_ref().is_some_and(|name| {
        name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control)
    }) {
        return Err(ApiError::bad_request(
            "Connection name must have 1 to120 characters without controls",
        ));
    }
    let token = Zeroizing::new(cookie_token(&headers)?);
    let (context, proof) = {
        let store = state
            .store
            .lock()
            .map_err(|_| super::super::unavailable())?;
        let auth = authorized_in_store(&state, &store, &token)?;
        expected_user(&headers, &auth.user.id)?;
        let (connection_id, revision) = match (request.connection_id, request.expected_revision) {
            (None, None) => (format!("mcp-{}", uuid::Uuid::now_v7()), 1),
            (Some(id), Some(revision)) => {
                store
                    .mcp_oauth_connection(&auth.user.id, &workspace, &id)
                    .map_err(error)?
                    .ok_or_else(ApiError::not_found)?;
                (id, revision)
            }
            _ => {
                return Err(ApiError::bad_request(
                    "Connection ID and revision must be supplied together",
                ));
            }
        };
        let context = AuthContext {
            user_id: auth.user.id.clone(),
            workspace_id: workspace,
            connection_id,
            connection_revision: revision,
        };
        store.mcp_oauth_can_authorize(&context).map_err(error)?;
        let proof = BrowserProof {
            user: auth.user.id,
            device: auth.device.id,
            session_hash: auth::hash_token(&token),
        };
        (context, proof)
    };
    let permit = state
        .mcp_oauth
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "MCP authorization busy"))?;
    let service = state.mcp_oauth.clone();
    let started = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.start(&state, &provider, context, proof, display_name)
    })
    .await
    .map_err(|_| ApiError::internal("MCP authorization unavailable"))?
    .map_err(error)?;
    let mut response = private_response(Json(
        json!({"authorization_url":started.authorization_url,"connection_id":started.context.connection_id,"revision":started.context.connection_revision}),
    ));
    response.headers_mut().insert(
        header::SET_COOKIE,
        flow_cookie(&started.browser, secure, 300),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(response)
}
pub(in crate::api) async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let provider = configured(&state, &headers, &id)?;
    let browser = browser(&headers)?;
    let query = uri
        .query()
        .filter(|q| q.len() <= 16384)
        .ok_or_else(ApiError::forbidden)?;
    let callback = Zeroizing::new(format!("{}?{query}", provider.redirect_uri));
    let permit = state
        .mcp_oauth
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "MCP authorization busy"))?;
    // A wrong state/cookie/provider must not consume another browser's record
    // or clear its cookie. Once taken, the callback is never replayed.
    let pending = state
        .mcp_oauth
        .take(&provider, &browser, &callback)
        .map_err(error)?;
    let secure = provider.redirect_uri.starts_with("https:");
    let service = state.mcp_oauth.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.finish(state, pending, &callback)
    })
    .await;
    let mut response = if matches!(outcome, Ok(Ok(()))) {
        private_response(Redirect::to("/"))
    } else {
        private_response((
            StatusCode::UNAUTHORIZED,
            "MCP authorization failed. Start a new authorization from Dunlin.",
        ))
    };
    response
        .headers_mut()
        .insert(header::SET_COOKIE, flow_cookie("", secure, 0));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(response)
}
