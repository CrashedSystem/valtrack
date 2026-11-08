//! Local Riot Client token exchange.
//!
//! GET https://127.0.0.1:{port}/entitlements/v1/token
//! Authorization: Basic base64("riot:" + lockfile_password)
//!
//! The certificate is self-signed, so verification is skipped for this
//! localhost-only client. The endpoint only exists while the Riot Client is
//! actually serving a game session (i.e. VALORANT is running).

use crate::paths::Lockfile;
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(rename = "token")]
    pub entitlement: String,
    pub subject: String,
    #[serde(default)]
    pub issuer: String,
}

pub fn local_http() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(8))
        .build()
        .context("failed to build local HTTP client")
}

pub fn fetch_tokens(lockfile: &Lockfile) -> Result<TokenResponse> {
    let http = local_http()?;
    let creds = format!("riot:{}", lockfile.password);
    let header = base64::engine::general_purpose::STANDARD.encode(creds);
    let url = format!(
        "https://127.0.0.1:{}/entitlements/v1/token",
        lockfile.port
    );

    let resp = http
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Basic {}", header))
        .send()
        .with_context(|| {
            format!(
                "cannot reach the Riot Client on 127.0.0.1:{} - is VALORANT running?",
                lockfile.port
            )
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        bail!(
            "local token endpoint returned {} ({}). \
             The REST surface is usually only exposed while VALORANT is running.",
            status,
            body.trim()
        );
    }

    resp.json::<TokenResponse>()
        .context("unexpected token response shape")
}
