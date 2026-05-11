use axum::http::{HeaderMap, StatusCode};

use crate::session;

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for cookie in cookie_header.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

pub fn extract_user_id(headers: &HeaderMap, jwt_secret: &[u8]) -> Option<String> {
    let token = cookie_value(headers, "session")?;
    session::verify_token(jwt_secret, &token)
}

pub fn require_user(headers: &HeaderMap, jwt_secret: &[u8]) -> Result<String, (StatusCode, String)> {
    extract_user_id(headers, jwt_secret)
        .ok_or((StatusCode::UNAUTHORIZED, "login required".into()))
}
