use crate::cli::parse_tweet_id;
use crate::context::AppContext;
use crate::errors::XmasterError;
use crate::intel::store::{FullSnapshot, IntelStore};
use crate::output::{self, CsvRenderable, OutputFormat, Tableable};
use chrono::{SecondsFormat, Utc};
use reqwest_oauth1::OAuthClientProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// API response types — multi-tweet GET /2/tweets?ids=...
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct ApiBatchEnvelope {
    #[serde(default)]
    data: Vec<TweetMetricsData>,
}

#[derive(Debug, Deserialize, Clone)]
struct TweetMetricsData {
    id: String,
    /// RFC3339 timestamp from X API. Requires `tweet.fields=created_at`.
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    public_metrics: Option<PublicMetrics>,
    #[serde(default)]
    non_public_metrics: Option<NonPublicMetrics>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct PublicMetrics {
    #[serde(default)]
    like_count: u64,
    #[serde(default)]
    retweet_count: u64,
    #[serde(default)]
    reply_count: u64,
    #[serde(default)]
    impression_count: u64,
    #[serde(default)]
    quote_count: u64,
    #[serde(default)]
    bookmark_count: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct NonPublicMetrics {
    #[serde(default)]
    url_link_clicks: u64,
    #[serde(default)]
    user_profile_clicks: u64,
}

// ---------------------------------------------------------------------------
// Agent-facing output types
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct MetricsDelta {
    /// Seconds elapsed since the previous snapshot we stored for this tweet.
    since_seconds: i64,
    /// Human-readable elapsed time since the previous snapshot ("2m", "1h 15m").
    since_human: String,
    impressions: i64,
    likes: i64,
    retweets: i64,
    replies: i64,
    quotes: i64,
    bookmarks: i64,
    profile_clicks: i64,
}

#[derive(Serialize, Clone)]
struct Velocity {
    /// Average impressions per minute since the post was created.
    /// `None` when we don't know `created_at` or age is zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    imps_per_min_since_post: Option<f64>,
    /// Average impressions per minute since the previous snapshot (instantaneous rate).
    /// `None` when there is no previous snapshot or the gap is zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    imps_per_min_since_last: Option<f64>,
}

#[derive(Serialize, Clone)]
struct MetricsRow {
    #[serde(rename = "id")]
    tweet_id: String,
    /// Post creation time (RFC3339 UTC). Requires X API `tweet.fields=created_at`.
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    /// Seconds since the post was created, computed server-side.
    #[serde(skip_serializing_if = "Option::is_none")]
    age_seconds: Option<i64>,
    /// Human-readable age ("9 min", "1h 23m", "3d 5h") — pre-formatted so the
    /// agent doesn't need a clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    age_human: Option<String>,
    impressions: u64,
    likes: u64,
    retweets: u64,
    replies: u64,
    quotes: u64,
    bookmarks: u64,
    profile_clicks: u64,
    url_clicks: u64,
    /// Change since the previous snapshot we stored in metric_snapshots.
    /// `None` on the first-ever call for this tweet.
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<MetricsDelta>,
    /// Impressions-per-minute velocity (two windows).
    #[serde(skip_serializing_if = "Option::is_none")]
    velocity: Option<Velocity>,
}

impl Tableable for MetricsRow {
    fn to_table(&self) -> comfy_table::Table {
        let mut table = comfy_table::Table::new();
        table.set_header(vec!["Metric", "Value"]);
        table.add_row(vec!["Tweet ID", &self.tweet_id]);
        if let Some(ref age) = self.age_human {
            table.add_row(vec!["Posted", &format!("{age} ago")]);
        }
        let imps_cell = match self.delta.as_ref() {
            Some(d) if d.impressions != 0 => format!(
                "{} ({:+} in {})",
                self.impressions, d.impressions, d.since_human
            ),
            _ => self.impressions.to_string(),
        };
        table.add_row(vec!["Impressions", &imps_cell]);
        table.add_row(vec!["Likes", &self.likes.to_string()]);
        table.add_row(vec!["Retweets", &self.retweets.to_string()]);
        table.add_row(vec!["Replies", &self.replies.to_string()]);
        table.add_row(vec!["Quotes", &self.quotes.to_string()]);
        table.add_row(vec!["Bookmarks", &self.bookmarks.to_string()]);
        table.add_row(vec!["Profile Clicks", &self.profile_clicks.to_string()]);
        table.add_row(vec!["URL Clicks", &self.url_clicks.to_string()]);
        if let Some(ref v) = self.velocity {
            if let Some(post_v) = v.imps_per_min_since_post {
                table.add_row(vec![
                    "Velocity (since post)",
                    &format!("{post_v:.1} imps/min"),
                ]);
            }
            if let Some(last_v) = v.imps_per_min_since_last {
                table.add_row(vec![
                    "Velocity (since last)",
                    &format!("{last_v:.1} imps/min"),
                ]);
            }
        }
        table
    }
}

impl CsvRenderable for MetricsRow {
    fn csv_headers() -> Vec<&'static str> {
        vec![
            "tweet_id",
            "age_seconds",
            "impressions",
            "likes",
            "retweets",
            "replies",
            "quotes",
            "bookmarks",
            "profile_clicks",
            "url_clicks",
            "delta_impressions",
            "delta_since_seconds",
        ]
    }
    fn csv_rows(&self) -> Vec<Vec<String>> {
        vec![vec![
            self.tweet_id.clone(),
            self.age_seconds.map(|s| s.to_string()).unwrap_or_default(),
            self.impressions.to_string(),
            self.likes.to_string(),
            self.retweets.to_string(),
            self.replies.to_string(),
            self.quotes.to_string(),
            self.bookmarks.to_string(),
            self.profile_clicks.to_string(),
            self.url_clicks.to_string(),
            self.delta
                .as_ref()
                .map(|d| d.impressions.to_string())
                .unwrap_or_default(),
            self.delta
                .as_ref()
                .map(|d| d.since_seconds.to_string())
                .unwrap_or_default(),
        ]]
    }
}

#[derive(Serialize)]
struct MetricsBatch {
    /// Server-side "now" in RFC3339 UTC. Agents should trust this, not their
    /// own clock, because the CLI runs in a real shell with an accurate clock.
    now: String,
    rows: Vec<MetricsRow>,
}

impl Tableable for MetricsBatch {
    fn to_table(&self) -> comfy_table::Table {
        let mut table = comfy_table::Table::new();
        table.set_header(vec![
            "Tweet ID",
            "Age",
            "Impressions (Δ)",
            "Likes",
            "Replies",
            "Profile Clicks",
            "Imps/min",
        ]);
        for r in &self.rows {
            let age = r.age_human.clone().unwrap_or_else(|| "—".into());
            let imps_cell = match r.delta.as_ref() {
                Some(d) if d.impressions != 0 => {
                    format!("{} ({:+})", r.impressions, d.impressions)
                }
                _ => r.impressions.to_string(),
            };
            let velocity_cell = r
                .velocity
                .as_ref()
                .and_then(|v| v.imps_per_min_since_last.or(v.imps_per_min_since_post))
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "—".into());
            table.add_row(vec![
                r.tweet_id.clone(),
                age,
                imps_cell,
                r.likes.to_string(),
                r.replies.to_string(),
                r.profile_clicks.to_string(),
                velocity_cell,
            ]);
        }
        table
    }
}

impl CsvRenderable for MetricsBatch {
    fn csv_headers() -> Vec<&'static str> {
        MetricsRow::csv_headers()
    }
    fn csv_rows(&self) -> Vec<Vec<String>> {
        self.rows.iter().flat_map(|r| r.csv_rows()).collect()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn oauth_secrets(ctx: &AppContext) -> reqwest_oauth1::Secrets<'_> {
    let k = &ctx.config.keys;
    reqwest_oauth1::Secrets::new(&k.api_key, &k.api_secret)
        .token(&k.access_token, &k.access_token_secret)
}

/// Format an elapsed duration in seconds as a compact human-readable string.
/// Examples: `30s`, `5 min`, `2h`, `2h 15m`, `3d`, `3d 4h`.
/// Negative values return `"future"` (clock skew).
fn format_age_human(seconds: i64) -> String {
    if seconds < 0 {
        return "future".into();
    }
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let hours = minutes / 60;
    let remaining_min = minutes % 60;
    if hours < 24 {
        if remaining_min == 0 {
            return format!("{hours}h");
        }
        return format!("{hours}h {remaining_min}m");
    }
    let days = hours / 24;
    let remaining_h = hours % 24;
    if remaining_h == 0 {
        format!("{days}d")
    } else {
        format!("{days}d {remaining_h}h")
    }
}

/// Parse an RFC3339 timestamp from the X API `created_at` field into a Unix
/// timestamp (seconds). Returns None on parse failure.
fn parse_created_at(created_at: Option<&str>) -> Option<i64> {
    created_at
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc).timestamp())
}

// ---------------------------------------------------------------------------
// HTTP: single batched fetch against GET /2/tweets?ids=ID1,ID2,...
// Replaces the previous per-tweet GET /2/tweets/{id} loop.
// X API allows up to 100 IDs per call.
// ---------------------------------------------------------------------------

/// Fetch metrics for up to 100 tweet IDs in a single HTTP call.
///
/// Tries full fields first (includes `non_public_metrics` — only visible for
/// your own tweets). If that 403s (batch contains only tweets you don't own),
/// falls back to `public_metrics` only.
///
/// Only 403 triggers the fallback. 401/429/5xx propagate.
async fn fetch_tweet_metrics_batch(
    ctx: &AppContext,
    tweet_ids: &[String],
) -> Result<Vec<TweetMetricsData>, XmasterError> {
    if tweet_ids.is_empty() {
        return Ok(Vec::new());
    }

    let ids_param = tweet_ids.join(",");
    let url_full = format!(
        "https://api.x.com/2/tweets?ids={ids_param}&tweet.fields=public_metrics,non_public_metrics,organic_metrics,created_at"
    );
    let resp = ctx
        .client
        .clone()
        .oauth1(oauth_secrets(ctx))
        .get(&url_full)
        .send()
        .await?;

    let first_status = resp.status();
    let first_body = resp.text().await.unwrap_or_default();

    if first_status.is_success() {
        if let Ok(envelope) = serde_json::from_str::<ApiBatchEnvelope>(&first_body) {
            return Ok(envelope.data);
        }
    }

    if first_status == 401 {
        return Err(XmasterError::AuthMissing {
            provider: "x",
            message: format!(
                "HTTP 401: {}",
                crate::utils::safe_truncate(&first_body, 200)
            ),
        });
    }
    if first_status == 429 {
        return Err(XmasterError::RateLimited {
            provider: "x",
            reset_at: 0,
        });
    }
    if first_status.as_u16() >= 500 {
        return Err(XmasterError::ServerError {
            status: first_status.as_u16(),
        });
    }

    // 403 or other client error — try public-only fields.
    let url_public = format!(
        "https://api.x.com/2/tweets?ids={ids_param}&tweet.fields=public_metrics,created_at,author_id,text"
    );
    let resp = ctx
        .client
        .clone()
        .oauth1(oauth_secrets(ctx))
        .get(&url_public)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(XmasterError::NotFound(format!(
            "Tweets {} (HTTP {status}: {})",
            ids_param,
            crate::utils::safe_truncate(&text, 100)
        )));
    }

    let envelope: ApiBatchEnvelope = resp.json().await?;
    Ok(envelope.data)
}

// ---------------------------------------------------------------------------
// Delta + velocity computation against the local metric_snapshots history.
// ---------------------------------------------------------------------------

fn build_delta(
    prev: &FullSnapshot,
    current: &PublicMetrics,
    current_np: &NonPublicMetrics,
    now_ts: i64,
) -> MetricsDelta {
    let since_seconds = (now_ts - prev.snapshot_at).max(0);
    MetricsDelta {
        since_seconds,
        since_human: format_age_human(since_seconds),
        impressions: current.impression_count as i64 - prev.impressions,
        likes: current.like_count as i64 - prev.likes,
        retweets: current.retweet_count as i64 - prev.retweets,
        replies: current.reply_count as i64 - prev.replies,
        quotes: current.quote_count as i64 - prev.quotes,
        bookmarks: current.bookmark_count as i64 - prev.bookmarks,
        profile_clicks: current_np.user_profile_clicks as i64 - prev.profile_clicks,
    }
}

fn build_velocity(
    current: &PublicMetrics,
    age_seconds: Option<i64>,
    delta: Option<&MetricsDelta>,
) -> Option<Velocity> {
    let since_post = age_seconds.filter(|&a| a > 0).map(|a| {
        let minutes = a as f64 / 60.0;
        current.impression_count as f64 / minutes
    });
    let since_last = delta.filter(|d| d.since_seconds > 0).map(|d| {
        let minutes = d.since_seconds as f64 / 60.0;
        d.impressions as f64 / minutes
    });
    if since_post.is_none() && since_last.is_none() {
        return None;
    }
    Some(Velocity {
        imps_per_min_since_post: since_post,
        imps_per_min_since_last: since_last,
    })
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

pub async fn execute_batch(
    ctx: Arc<AppContext>,
    format: OutputFormat,
    ids: &[String],
) -> Result<(), XmasterError> {
    if ids.is_empty() {
        return Err(XmasterError::Config("No tweet IDs provided".into()));
    }

    if !ctx.config.has_x_auth() {
        return Err(XmasterError::AuthMissing {
            provider: "x",
            message: "X API credentials not configured".into(),
        });
    }

    // Normalize IDs (strip URLs) up front.
    let tweet_ids: Vec<String> = ids.iter().map(|id| parse_tweet_id(id)).collect();

    let now_ts = Utc::now().timestamp();
    let now_iso = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    // Open the intel store once and reuse for every tweet. A single failed
    // open means we silently skip delta/velocity but still return metrics —
    // the API data is the ground truth.
    let store = IntelStore::open().ok();

    let mut rows: Vec<MetricsRow> = Vec::with_capacity(tweet_ids.len());

    for chunk in tweet_ids.chunks(100) {
        let tweets = match fetch_tweet_metrics_batch(&ctx, chunk).await {
            Ok(tweets) => tweets,
            Err(e) => {
                // On batch failure, emit a warning for each ID in this chunk and skip.
                for id in chunk {
                    eprintln!("Warning: {id}: {e}");
                }
                continue;
            }
        };

        // Index by id so we preserve the caller's requested order in the output.
        let mut by_id: HashMap<String, TweetMetricsData> = tweets
            .into_iter()
            .map(|tweet| (tweet.id.clone(), tweet))
            .collect();

        for id in chunk {
            let Some(tweet) = by_id.remove(id) else {
                eprintln!("Warning: {id}: not returned by /2/tweets");
                continue;
            };

            let public = tweet.public_metrics.clone().unwrap_or_default();
            let non_public = tweet.non_public_metrics.clone().unwrap_or_default();

            // ── Age since creation ──
            let created_ts = parse_created_at(tweet.created_at.as_deref());
            let age_seconds = created_ts.map(|c| (now_ts - c).max(0));
            let age_human = age_seconds.map(format_age_human);

            // ── Delta vs previous snapshot + velocity ──
            let (delta, velocity) = if let Some(ref store) = store {
                let prev = store.latest_snapshot_full(&tweet.id).ok().flatten();
                let delta = prev
                    .as_ref()
                    .map(|p| build_delta(p, &public, &non_public, now_ts));
                let velocity = build_velocity(&public, age_seconds, delta.as_ref());

                // Save the current snapshot so the NEXT call has a baseline.
                // Minutes-since-post is best-effort; falls back to 0 when unknown.
                let minutes_since_post = age_seconds.map(|a| a / 60).unwrap_or(0);
                let _ = store.log_metric_snapshot(
                    &tweet.id,
                    public.like_count as i64,
                    public.retweet_count as i64,
                    public.reply_count as i64,
                    public.impression_count as i64,
                    public.bookmark_count as i64,
                    public.quote_count as i64,
                    non_public.user_profile_clicks as i64,
                    minutes_since_post,
                );

                (delta, velocity)
            } else {
                // No intel store available — still compute velocity-since-post
                // from age alone, but no delta.
                let velocity = build_velocity(&public, age_seconds, None);
                (None, velocity)
            };

            rows.push(MetricsRow {
                tweet_id: tweet.id,
                created_at: tweet.created_at,
                age_seconds,
                age_human,
                impressions: public.impression_count,
                likes: public.like_count,
                retweets: public.retweet_count,
                replies: public.reply_count,
                quotes: public.quote_count,
                bookmarks: public.bookmark_count,
                profile_clicks: non_public.user_profile_clicks,
                url_clicks: non_public.url_link_clicks,
                delta,
                velocity,
            });
        }
    }

    // For a single tweet, render the detailed single-row view. For multiple,
    // render the compact batch table. JSON output always uses the batch shape
    // so agents get a stable schema regardless of input size.
    if rows.len() == 1 && format == OutputFormat::Table {
        let meta = serde_json::json!({ "now": now_iso });
        output::render(format, &rows[0], Some(meta));
    } else {
        let batch = MetricsBatch { now: now_iso, rows };
        output::render_csv(format, &batch, None);
    }
    Ok(())
}

