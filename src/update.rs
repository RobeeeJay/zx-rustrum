//! Whether GitHub has a newer release than the one running, asked once at
//! launch the same way PurpleCAD asks: the releases API's `latest`, which
//! leaves drafts and pre-releases out. Any failure — offline, rate-limited, a
//! reply of the wrong shape — is "nothing new": a check made at launch must
//! never get in the way.

use std::sync::mpsc;
use std::time::Duration;

use crate::mcp::json::{self, Json};

pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/RobeeeJay/zx-rustrum/releases/latest";

/// The version this build is.
pub const RUNNING: &str = env!("CARGO_PKG_VERSION");

/// A newer release: its version, and the page to download it from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub url: String,
}

/// "v1.0.10" as [1, 0, 10]; None for anything that is not dotted numbers.
pub fn parse_version(v: &str) -> Option<Vec<u32>> {
    let v = v.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    if v.is_empty() {
        return None;
    }
    v.split('.').map(|part| part.parse().ok()).collect()
}

/// Compared part by part as numbers: as text, 1.0.10 sorts before 1.0.9.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some(a), Some(b)) = (parse_version(candidate), parse_version(current)) else {
        return false;
    };
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// What a reply from the API offers, if it is a published release newer than
/// `current`.
pub fn read_release(body: &str, current: &str) -> Option<Update> {
    let release = json::parse(body).ok()?;
    let flag = |key| matches!(release.get(key), Some(Json::Bool(true)));
    if flag("draft") || flag("prerelease") {
        return None;
    }
    let tag = release.get("tag_name")?.as_str()?;
    if !is_newer(tag, current) {
        return None;
    }
    let version = parse_version(tag)?
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".");
    let url = release.get("html_url")?.as_str()?.to_string();
    Some(Update { version, url })
}

/// Ask GitHub, waiting at most ten seconds.
pub fn check(current: &str) -> Option<Update> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let body = agent
        .get(LATEST_RELEASE_URL)
        .set("Accept", "application/vnd.github+json")
        // GitHub refuses API requests that carry no User-Agent.
        .set("User-Agent", &format!("ZX-Rustrum/{current}"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    read_release(&body, current)
}

/// Ask on a thread of its own, and wake the window when the answer is in: a
/// paused machine's window is otherwise asleep.
pub fn check_in_background(ctx: egui::Context) -> mpsc::Receiver<Option<Update>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let found = check(RUNNING);
        let _ = send.send(found);
        ctx.request_repaint();
    });
    receive
}
