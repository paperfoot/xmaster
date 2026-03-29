use crate::cli::parse_tweet_id;
use crate::context::AppContext;
use crate::errors::XmasterError;
use crate::output::{self, CsvRenderable, OutputFormat, Tableable};
use crate::providers::xapi::XApi;
use reqwest::Method;
use reqwest_oauth1::OAuthClientProvider;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;

const BASE: &str = "https://api.x.com/2";

// ---------------------------------------------------------------------------
// OAuth helper (same pattern as xapi.rs)
// ---------------------------------------------------------------------------

fn oauth_secrets(ctx: &AppContext) -> reqwest_oauth1::Secrets<'_> {
    let k = &ctx.config.keys;
    reqwest_oauth1::Secrets::new(&k.api_key, &k.api_secret)
        .token(&k.access_token, &k.access_token_secret)
}

fn require_auth(ctx: &AppContext) -> Result<(), XmasterError> {
    if !ctx.config.has_x_auth() {
        return Err(XmasterError::AuthMissing {
            provider: "x",
            message: "X API credentials not configured".into(),
        });
    }
    Ok(())
}

async fn signed_request(
    ctx: &AppContext,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<(), XmasterError> {
    require_auth(ctx)?;

    let resp = match method {
        Method::PUT => {
            let mut b = ctx.client.clone().oauth1(oauth_secrets(ctx)).put(url);
            if let Some(ref json) = body {
                b = b
                    .header("Content-Type", "application/json")
                    .body(serde_json::to_string(json)?);
            }
            b.send().await?
        }
        Method::POST => {
            let mut b = ctx.client.clone().oauth1(oauth_secrets(ctx)).post(url);
            if let Some(ref json) = body {
                b = b
                    .header("Content-Type", "application/json")
                    .body(serde_json::to_string(json)?);
            }
            b.send().await?
        }
        Method::DELETE => {
            ctx.client.clone().oauth1(oauth_secrets(ctx)).delete(url).send().await?
        }
        _ => {
            return Err(XmasterError::Api {
                provider: "x",
                code: "unsupported_method",
                message: format!("Unsupported method: {method}"),
            });
        }
    };

    let status = resp.status();
    if status == 401 || status == 403 {
        let text = resp.text().await.unwrap_or_default();
        return Err(XmasterError::AuthMissing {
            provider: "x",
            message: format!("HTTP {status}: {text}"),
        });
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(XmasterError::Api {
            provider: "x",
            code: "api_error",
            message: format!("HTTP {status}: {}", crate::utils::safe_truncate(&text, 200)),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ModerationResult {
    action: String,
    target: String,
    success: bool,
}

impl Tableable for ModerationResult {
    fn to_table(&self) -> comfy_table::Table {
        let mut table = comfy_table::Table::new();
        table.set_header(vec!["Action", "Target", "Status"]);
        table.add_row(vec![
            self.action.as_str(),
            self.target.as_str(),
            if self.success { "OK" } else { "Failed" },
        ]);
        table
    }
}

impl CsvRenderable for ModerationResult {}

fn render_success(format: OutputFormat, action: &str, target: String) {
    let display = ModerationResult {
        action: action.to_string(),
        target,
        success: true,
    };
    output::render(format, &display, None);
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn hide_reply(
    ctx: Arc<AppContext>,
    format: OutputFormat,
    id: &str,
) -> Result<(), XmasterError> {
    let tweet_id = parse_tweet_id(id);
    signed_request(
        &ctx,
        Method::PUT,
        &format!("{BASE}/tweets/{tweet_id}/hidden"),
        Some(json!({ "hidden": true })),
    )
    .await?;
    render_success(format, "hide_reply", tweet_id);
    Ok(())
}

pub async fn unhide_reply(
    ctx: Arc<AppContext>,
    format: OutputFormat,
    id: &str,
) -> Result<(), XmasterError> {
    let tweet_id = parse_tweet_id(id);
    signed_request(
        &ctx,
        Method::PUT,
        &format!("{BASE}/tweets/{tweet_id}/hidden"),
        Some(json!({ "hidden": false })),
    )
    .await?;
    render_success(format, "unhide_reply", tweet_id);
    Ok(())
}

pub async fn block(
    ctx: Arc<AppContext>,
    format: OutputFormat,
    username: &str,
) -> Result<(), XmasterError> {
    let api = XApi::new(ctx.clone());
    let uid = api.get_authenticated_user_id().await?;
    let target = api.get_user_by_username(username).await?;

    signed_request(
        &ctx,
        Method::POST,
        &format!("{BASE}/users/{uid}/blocking"),
        Some(json!({ "target_user_id": target.id })),
    )
    .await?;

    render_success(format, "block", format!("@{username}"));
    Ok(())
}

pub async fn unblock(
    ctx: Arc<AppContext>,
    format: OutputFormat,
    username: &str,
) -> Result<(), XmasterError> {
    let api = XApi::new(ctx.clone());
    let uid = api.get_authenticated_user_id().await?;
    let target = api.get_user_by_username(username).await?;

    signed_request(
        &ctx,
        Method::DELETE,
        &format!("{BASE}/users/{uid}/blocking/{}", target.id),
        None,
    )
    .await?;

    render_success(format, "unblock", format!("@{username}"));
    Ok(())
}

pub async fn mute(
    ctx: Arc<AppContext>,
    format: OutputFormat,
    username: &str,
) -> Result<(), XmasterError> {
    let api = XApi::new(ctx.clone());
    let uid = api.get_authenticated_user_id().await?;
    let target = api.get_user_by_username(username).await?;

    signed_request(
        &ctx,
        Method::POST,
        &format!("{BASE}/users/{uid}/muting"),
        Some(json!({ "target_user_id": target.id })),
    )
    .await?;

    render_success(format, "mute", format!("@{username}"));
    Ok(())
}

pub async fn unmute(
    ctx: Arc<AppContext>,
    format: OutputFormat,
    username: &str,
) -> Result<(), XmasterError> {
    let api = XApi::new(ctx.clone());
    let uid = api.get_authenticated_user_id().await?;
    let target = api.get_user_by_username(username).await?;

    signed_request(
        &ctx,
        Method::DELETE,
        &format!("{BASE}/users/{uid}/muting/{}", target.id),
        None,
    )
    .await?;

    render_success(format, "unmute", format!("@{username}"));
    Ok(())
}
