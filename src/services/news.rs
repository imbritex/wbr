use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Utc};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const FX_TWITTER_PROFILE_API: &str = "https://api.fxtwitter.com/2/profile";
const NEWS_SOURCE_CONCURRENCY: usize = 2;
const NEWS_SOURCE_MAX_ATTEMPTS: u32 = 3;
const NEWS_SOURCE_RETRY_DELAY_MS: u64 = 600;

#[derive(Debug, Clone, Copy)]
pub struct NewsSource {
    pub name: &'static str,
    pub username: &'static str,
    pub include_reposts: bool,
}

pub const DEFAULT_NEWS_SOURCES: &[NewsSource] = &[
    NewsSource {
        name: "Funkin News",
        username: "News_Funkin",
        include_reposts: true,
    },
    NewsSource {
        name: "Discussing FNF",
        username: "Discussingfnf",
        include_reposts: false,
    },
    NewsSource {
        name: "FNF Developers",
        username: "FNF_Developers",
        include_reposts: true,
    },
    NewsSource {
        name: "Funkin Interviews",
        username: "Funkininterview",
        include_reposts: false,
    },
    NewsSource {
        name: "GameBanana FNF",
        username: "GameBananaFNF",
        include_reposts: false,
    },
    NewsSource {
        name: "WeekBox",
        username: "WeekBoxTeam",
        include_reposts: true,
    },
    NewsSource {
        name: "DirectMania",
        username: "DirectMania",
        include_reposts: true,
    },
    NewsSource {
        name: "FunkCon",
        username: "FunkCon24",
        include_reposts: true,
    },
];

#[derive(Debug, Clone, Default)]
pub struct NewsPost {
    pub id: String,
    pub author_name: String,
    pub author_handle: String,
    pub source_name: String,
    pub text: String,
    pub timestamp: i64,
    pub time_label: String,
    pub url: String,
    pub is_repost: bool,
    pub reposted_by: String,
    pub quote_author: String,
    pub quote_text: String,
    pub likes: u64,
    pub reposts: u64,
    pub replies: u64,
    pub views: u64,
    pub photo_url: String,
    pub has_video: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NewsFeedBatch {
    pub posts: Vec<NewsPost>,
    pub available_sources: Vec<String>,
    pub cursors: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxAuthorRaw {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    screen_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxPhotoRaw {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxFormatRaw {
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxVideoRaw {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    thumbnail_url: Option<String>,
    #[serde(default)]
    transcode_url: Option<String>,
    #[serde(default)]
    formats: Option<Vec<FxFormatRaw>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxMediaRaw {
    #[serde(default)]
    photos: Option<Vec<FxPhotoRaw>>,
    #[serde(default)]
    videos: Option<Vec<FxVideoRaw>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxStatusRaw {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    created_timestamp: Option<Value>,
    #[serde(default)]
    author: Option<FxAuthorRaw>,
    #[serde(default)]
    reposted_by: Option<FxAuthorRaw>,
    #[serde(rename = "repostedBy", default)]
    reposted_by_camel: Option<FxAuthorRaw>,
    #[serde(rename = "isRetweet", default)]
    is_retweet: Option<bool>,
    #[serde(default)]
    replying_to: Option<Value>,
    #[serde(default)]
    media: Option<FxMediaRaw>,
    #[serde(default)]
    likes: Option<u64>,
    #[serde(default)]
    reposts: Option<u64>,
    #[serde(default)]
    replies: Option<u64>,
    #[serde(default)]
    views: Option<u64>,
    #[serde(default)]
    quote: Option<Box<FxStatusRaw>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxCursorRaw {
    #[serde(default)]
    bottom: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FxProfileResponseRaw {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    results: Option<Vec<FxStatusRaw>>,
    #[serde(default)]
    cursor: Option<FxCursorRaw>,
}

pub struct NewsService {
    http: Client,
}

impl Default for NewsService {
    fn default() -> Self {
        Self::new()
    }
}

impl NewsService {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent("wb-r/3.0.0")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { http }
    }

    pub async fn fetch_feed(
        &self,
        selected_account: Option<&str>,
        cursors: &HashMap<String, String>,
    ) -> anyhow::Result<NewsFeedBatch> {
        let target_account = selected_account
            .map(|s| s.trim().trim_start_matches('@'))
            .filter(|s| !s.is_empty());

        let sources: Vec<NewsSource> = DEFAULT_NEWS_SOURCES
            .iter()
            .copied()
            .filter(|s| match target_account {
                Some(acc) => s.username.eq_ignore_ascii_case(acc),
                None => true,
            })
            .collect();

        if sources.is_empty() {
            return Ok(NewsFeedBatch::default());
        }

        let sem = Arc::new(Semaphore::new(NEWS_SOURCE_CONCURRENCY));
        let mut join_set = JoinSet::new();

        for source in sources {
            let client = self.http.clone();
            let sem = sem.clone();
            let cursor = find_cursor_for_source(cursors, source.username);

            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await;
                let res = fetch_source_with_retries(&client, source, cursor.as_deref()).await;
                (source, res)
            });
        }

        let mut posts_by_id: HashMap<String, NewsPost> = HashMap::new();
        let mut available_set: HashSet<&'static str> = HashSet::new();
        let mut next_cursors: HashMap<String, String> = HashMap::new();
        let mut had_success = false;
        let mut last_error: Option<anyhow::Error> = None;

        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok((source, Ok((bottom_cursor, posts)))) => {
                    had_success = true;
                    if !posts.is_empty() {
                        available_set.insert(source.username);
                    }
                    if let Some(cur) = bottom_cursor.filter(|c| !c.is_empty()) {
                        next_cursors.insert(source.username.to_string(), cur);
                    }
                    for post in posts {
                        posts_by_id.insert(post.id.clone(), post);
                    }
                }
                Ok((source, Err(err))) => {
                    tracing::warn!(
                        source = source.name,
                        %err,
                        "Failed to load news source after retries"
                    );
                    last_error = Some(err);
                }
                Err(err) => {
                    last_error = Some(err.into());
                }
            }
        }

        if !had_success && posts_by_id.is_empty() {
            if let Some(err) = last_error {
                return Err(err);
            }
        }

        let mut posts: Vec<NewsPost> = posts_by_id.into_values().collect();
        posts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        let available_sources = DEFAULT_NEWS_SOURCES
            .iter()
            .filter(|s| available_set.contains(s.username))
            .map(|s| s.username.to_string())
            .collect();

        Ok(NewsFeedBatch {
            posts,
            available_sources,
            cursors: next_cursors,
        })
    }
}

fn find_cursor_for_source(cursors: &HashMap<String, String>, username: &str) -> Option<String> {
    if let Some(val) = cursors.get(username) {
        if !val.trim().is_empty() {
            return Some(val.trim().to_string());
        }
    }
    cursors
        .iter()
        .find(|(k, v)| k.eq_ignore_ascii_case(username) && !v.trim().is_empty())
        .map(|(_, v)| v.trim().to_string())
}

async fn fetch_source_with_retries(
    client: &Client,
    source: NewsSource,
    cursor: Option<&str>,
) -> anyhow::Result<(Option<String>, Vec<NewsPost>)> {
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..NEWS_SOURCE_MAX_ATTEMPTS {
        match fetch_source_once(client, source, cursor, attempt).await {
            Ok(result) => return Ok(result),
            Err(err) => {
                last_err = Some(err);
                if attempt + 1 < NEWS_SOURCE_MAX_ATTEMPTS {
                    let delay = NEWS_SOURCE_RETRY_DELAY_MS * (1u64 << attempt);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Unknown error fetching {}", source.name)))
}

async fn fetch_source_once(
    client: &Client,
    source: NewsSource,
    cursor: Option<&str>,
    attempt: u32,
) -> anyhow::Result<(Option<String>, Vec<NewsPost>)> {
    let base = format!("{FX_TWITTER_PROFILE_API}/{}/statuses", source.username);
    let mut url = Url::parse(&base)?;
    {
        let mut qp = url.query_pairs_mut();
        if let Some(c) = cursor.filter(|c| !c.is_empty()) {
            qp.append_pair("cursor", c);
        }
        let cache_buster = format!("{}_{}", Utc::now().timestamp_millis(), attempt);
        qp.append_pair("_t", &cache_buster);
    }

    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("{} returned HTTP {}", source.name, resp.status());
    }

    let payload: FxProfileResponseRaw = resp.json().await?;
    if let Some(code) = payload.code {
        if code != 200 {
            anyhow::bail!("{} returned code {}", source.name, code);
        }
    }

    let now = Utc::now();
    let posts = payload
        .results
        .unwrap_or_default()
        .into_iter()
        .filter(|status| status.kind.as_deref() == Some("status"))
        .filter(|status| source.include_reposts || !is_retweet(status))
        .filter(|status| keep_reply_filter(status, source))
        .filter_map(|status| normalize_status(&status, source, now))
        .collect();

    let bottom_cursor = payload
        .cursor
        .and_then(|c| c.bottom)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok((bottom_cursor, posts))
}

fn is_retweet(status: &FxStatusRaw) -> bool {
    if status.is_retweet.unwrap_or(false)
        || status.reposted_by.is_some()
        || status.reposted_by_camel.is_some()
    {
        return true;
    }

    let probe = status
        .text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(status.title.as_deref())
        .unwrap_or_default()
        .trim();
    starts_with_rt_prefix(probe)
}

fn starts_with_rt_prefix(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 4 || !bytes[..2].eq_ignore_ascii_case(b"rt") {
        return false;
    }
    let after_rt = text[2..].trim_start();
    if after_rt.starts_with('@') {
        return true;
    }
    if after_rt.len() >= 3 && after_rt.as_bytes()[..2].eq_ignore_ascii_case(b"by") {
        let after_by = &after_rt[2..];
        let trimmed = after_by.trim_start();
        return after_by.len() > trimmed.len() && trimmed.starts_with('@');
    }
    false
}

fn keep_reply_filter(status: &FxStatusRaw, source: NewsSource) -> bool {
    let Some(replying_to) = status.replying_to.as_ref() else {
        return true;
    };
    if replying_to.is_null() {
        return true;
    }

    let reply_target = match replying_to {
        Value::String(s) => s.trim(),
        Value::Object(map) => map
            .get("screen_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or(""),
        _ => "",
    };

    if reply_target.is_empty() {
        return false;
    }

    let author_screen = status
        .author
        .as_ref()
        .and_then(|a| a.screen_name.as_deref())
        .unwrap_or("");

    reply_target.eq_ignore_ascii_case(source.username)
        || (!author_screen.is_empty() && reply_target.eq_ignore_ascii_case(author_screen))
}

fn normalize_status(
    status: &FxStatusRaw,
    source: NewsSource,
    now: DateTime<Utc>,
) -> Option<NewsPost> {
    let id = extract_id(status.id.as_ref());
    if id.is_empty() {
        return None;
    }

    let timestamp = extract_timestamp(status)?;
    let time_label = format_relative_time(timestamp, now);

    let text = status
        .text
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    let repost_flag = is_retweet(status);
    let reposter = status
        .reposted_by
        .as_ref()
        .or(status.reposted_by_camel.as_ref());

    let reposted_by = if repost_flag {
        reposter
            .and_then(|r| {
                r.name
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| r.screen_name.as_deref().filter(|s| !s.trim().is_empty()))
            })
            .unwrap_or(source.name)
            .trim()
            .to_string()
    } else {
        String::new()
    };

    // Match twitter-normalizer.ts author resolution, while also keeping original author when available.
    let display_author = if source.include_reposts {
        status.author.as_ref().or(reposter)
    } else {
        status.author.as_ref()
    };

    let author_handle = display_author
        .and_then(|a| a.screen_name.as_deref())
        .map(|s| s.trim().trim_start_matches('@'))
        .filter(|s| !s.is_empty())
        .unwrap_or(source.username)
        .to_string();

    let author_name = display_author
        .and_then(|a| {
            a.name
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| a.screen_name.as_deref().filter(|s| !s.trim().is_empty()))
        })
        .unwrap_or(source.name)
        .trim()
        .to_string();

    let url = to_safe_external_url(status.url.as_deref().unwrap_or_default(), Some("https://x.com/"));

    let (photo_url, has_video) = extract_media(status.media.as_ref());

    let (quote_author, quote_text) = match status.quote.as_deref() {
        Some(q) => {
            let q_author = q
                .author
                .as_ref()
                .and_then(|a| {
                    let name = a.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
                    let handle = a
                        .screen_name
                        .as_deref()
                        .map(|s| s.trim().trim_start_matches('@'))
                        .filter(|s| !s.is_empty());
                    match (name, handle) {
                        (Some(n), Some(h)) => Some(format!("{n} (@{h})")),
                        (Some(n), None) => Some(n.to_string()),
                        (None, Some(h)) => Some(format!("@{h}")),
                        (None, None) => None,
                    }
                })
                .unwrap_or_else(|| "Quoted post".to_string());
            let q_text = q
                .text
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            (q_author, q_text)
        }
        None => (String::new(), String::new()),
    };

    let has_quote = !quote_text.is_empty()
        || status
            .quote
            .as_deref()
            .is_some_and(|q| !extract_id(q.id.as_ref()).is_empty());

    if text.is_empty() && !has_quote && photo_url.is_empty() && !has_video {
        return None;
    }

    Some(NewsPost {
        id,
        author_name,
        author_handle,
        source_name: source.name.to_string(),
        text,
        timestamp,
        time_label,
        url,
        is_repost: repost_flag,
        reposted_by,
        quote_author,
        quote_text,
        likes: status.likes.unwrap_or(0),
        reposts: status.reposts.unwrap_or(0),
        replies: status.replies.unwrap_or(0),
        views: status.views.unwrap_or(0),
        photo_url,
        has_video,
    })
}

fn extract_id(val: Option<&Value>) -> String {
    match val {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn extract_timestamp(status: &FxStatusRaw) -> Option<i64> {
    if let Some(val) = status.created_timestamp.as_ref() {
        let parsed = match val {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => s
                .trim()
                .parse::<i64>()
                .ok()
                .or_else(|| s.trim().parse::<f64>().ok().map(|f| f as i64)),
            _ => None,
        };
        if let Some(ts) = parsed.filter(|t| *t > 0) {
            return Some(if ts > 10_000_000_000 { ts / 1000 } else { ts });
        }
    }

    if let Some(created_at) = status.created_at.as_deref() {
        let trimmed = created_at.trim();
        if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
            return Some(dt.timestamp());
        }
        if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
            return Some(dt.timestamp());
        }
    }

    None
}

fn extract_media(media: Option<&FxMediaRaw>) -> (String, bool) {
    let Some(media) = media else {
        return (String::new(), false);
    };

    let mut video_thumbs: Vec<String> = Vec::new();
    let mut has_video = false;

    if let Some(videos) = media.videos.as_ref() {
        for v in videos {
            let thumb = to_safe_external_url(v.thumbnail_url.as_deref().unwrap_or_default(), None);
            let direct = to_safe_external_url(v.url.as_deref().unwrap_or_default(), None);
            let transcode = to_safe_external_url(v.transcode_url.as_deref().unwrap_or_default(), None);
            let has_mp4_format = v.formats.as_ref().is_some_and(|fmts| {
                fmts.iter().any(|f| {
                    f.container.as_deref() == Some("mp4")
                        && !to_safe_external_url(f.url.as_deref().unwrap_or_default(), None)
                            .is_empty()
                })
            });

            if !direct.is_empty() || !transcode.is_empty() || has_mp4_format || !thumb.is_empty() {
                has_video = true;
            }
            if !thumb.is_empty() {
                video_thumbs.push(thumb);
            }
        }
    }

    let video_thumb_identities: HashSet<String> = video_thumbs
        .iter()
        .map(|u| media_identity(u))
        .filter(|id| !id.is_empty())
        .collect();

    let first_photo = media
        .photos
        .as_ref()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let safe = to_safe_external_url(p.url.as_deref().unwrap_or_default(), None);
            if safe.is_empty() {
                None
            } else {
                Some(safe)
            }
        })
        .find(|url| !video_thumb_identities.contains(&media_identity(url)));

    let photo_url = first_photo
        .or_else(|| video_thumbs.into_iter().next())
        .unwrap_or_default();

    (photo_url, has_video)
}

fn media_identity(value: &str) -> String {
    if let Ok(parsed) = Url::parse(value) {
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = parsed.path().to_ascii_lowercase();
        if !host.is_empty() || !path.is_empty() {
            return format!("{host}{path}");
        }
    }
    value.trim().to_ascii_lowercase()
}

fn to_safe_external_url(raw: &str, base_url: Option<&str>) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let parsed = if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        Url::parse(trimmed).ok()
    } else if let Some(base) = base_url {
        Url::parse(base)
            .ok()
            .and_then(|b| b.join(trimmed.trim_start_matches('/')).ok())
    } else {
        None
    };

    match parsed {
        Some(url) if url.scheme() == "https" => url.to_string(),
        _ => String::new(),
    }
}

fn format_relative_time(timestamp: i64, now: DateTime<Utc>) -> String {
    let Some(dt) = DateTime::from_timestamp(timestamp, 0) else {
        return String::new();
    };

    let diff_secs = (now.timestamp() - timestamp).max(0);
    if diff_secs < 60 {
        return "Just now".to_string();
    }
    if diff_secs < 3600 {
        return format!("{}m ago", diff_secs / 60);
    }
    if diff_secs < 86400 {
        return format!("{}h ago", diff_secs / 3600);
    }
    if diff_secs < 604_800 {
        return format!("{}d ago", diff_secs / 86400);
    }

    if dt.year() == now.year() {
        format!("{} {}", dt.format("%b"), dt.day())
    } else {
        format!("{} {}, {}", dt.format("%b"), dt.day(), dt.year())
    }
}
