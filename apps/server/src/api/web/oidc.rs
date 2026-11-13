//! Browser-bound authorization code callback. Never return provider tokens.
use super::{ApiError, AppState, SESSION_TTL, cookie, private_response, request_origin};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Redirect, Response},
};
use serde::Deserialize;
const FLOW_COOKIE: &str = "dunlin_oidc_flow";

fn flow_cookie(value: &str, secure: bool, ttl: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{FLOW_COOKIE}={value}; Path=/v1/web/oidc; HttpOnly; SameSite=Lax; Max-Age={ttl}{}",
        if secure { "; Secure" } else { "" }
    ))
    .expect("generated cookie")
}
fn browser(headers: &HeaderMap) -> Result<String, ApiError> {
    let mut result = None;
    for raw in headers.get_all(header::COOKIE) {
        for part in raw.to_str().map_err(|_| ApiError::forbidden())?.split(';') {
            if let Some((name, value)) = part.trim().split_once('=')
                && name == FLOW_COOKIE
            {
                if result.is_some()
                    || value.len() != 47
                    || !value.starts_with("kst_")
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err(ApiError::forbidden());
                }
                result = Some(value.to_string());
            }
        }
    }
    result.ok_or_else(ApiError::forbidden)
}
fn configured(state: &AppState, headers: &HeaderMap) -> Result<crate::oidc::Config, ApiError> {
    request_origin(headers, false)?;
    let config = state.config.oidc.as_ref().ok_or_else(ApiError::not_found)?;
    config
        .validate()
        .map_err(|_| ApiError::internal("Invalid OIDC configuration"))?;
    let redirect: axum::http::Uri = config
        .redirect_uri
        .parse()
        .map_err(|_| ApiError::internal("Invalid OIDC callback"))?;
    if headers.get(header::HOST).and_then(|h| h.to_str().ok())
        != redirect.authority().map(|a| a.as_str())
    {
        return Err(ApiError::forbidden());
    }
    Ok(config.clone())
}

pub(in crate::api) async fn configuration(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    request_origin(&headers, false)?;
    Ok(private_response(Json(
        serde_json::json!({"enabled":configured(&state,&headers).is_ok()}),
    )))
}

pub(in crate::api) async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let secure = request_origin(&headers, true)?;
    let config = configured(&state, &headers)?;
    let permit = state
        .oidc
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "OIDC login busy"))?;
    let service = state.oidc.clone();
    let (authorization, browser) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.start(&config)
    })
    .await
    .map_err(|_| ApiError::internal("OIDC unavailable"))?
    .map_err(|_| ApiError::bad_request("OIDC login unavailable"))?;
    let mut response =
        private_response(Json(serde_json::json!({"authorization_url":authorization})));
    response
        .headers_mut()
        .insert(header::SET_COOKIE, flow_cookie(&browser, secure, 300));
    Ok(response)
}

#[derive(Deserialize)]
pub(in crate::api) struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

pub(in crate::api) async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Result<Response, ApiError> {
    let config = configured(&state, &headers)?;
    let browser = browser(&headers)?;
    let secure = config.redirect_uri.starts_with("https:");
    let permit = state
        .oidc
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "OIDC login busy"))?;
    let service = state.oidc.clone();
    let original = config.clone();
    let verified = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.finish(
            &config,
            &query.state,
            &browser,
            if query.error.is_some() {
                ""
            } else {
                query.code.as_deref().unwrap_or("")
            },
        )
    })
    .await
    .map_err(|_| ApiError::internal("OIDC unavailable"))?;
    let mut response = match verified {
        Ok((subject, lifetime)) => {
            let user_id = original
                .bindings
                .get(&subject)
                .ok_or_else(ApiError::forbidden)?;
            let user = state
                .store
                .lock()
                .map_err(|_| super::unavailable())?
                .find_user(user_id)?
                .ok_or_else(ApiError::forbidden)?;
            super::super::check_host_account_policy(&state, &user)?;
            let token = {
                let store = state.store.lock().map_err(|_| super::unavailable())?;
                store.create_oidc_browser_session(
                    &user.id,
                    &subject,
                    &original.digest(),
                    lifetime.min(SESSION_TTL.as_secs()),
                )?
            };
            let mut response = private_response(Redirect::to("/"));
            response.headers_mut().append(
                header::SET_COOKIE,
                cookie(&token, secure, lifetime.min(SESSION_TTL.as_secs())),
            );
            response
        }
        Err(_) => private_response((
            StatusCode::UNAUTHORIZED,
            "OIDC login rejected. Start a new login from Dunlin.",
        )),
    };
    response
        .headers_mut()
        .append(header::SET_COOKIE, flow_cookie("", secure, 0));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests;
