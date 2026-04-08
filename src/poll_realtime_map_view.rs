//! Polling wrapper for the CHaser Online real-time map view.
//!
//! Authenticates once and reuses the JSESSIONID across all subsequent fetches.
//! Re-authenticates automatically only when the session expires.
//!
//! # Quick start
//!
//! ```no_run
//! use std::time::Duration;
//! use chaser_util::poll_realtime_map_view::{poll_map_view, PollOptions};
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut rx = poll_map_view("hot", "hot", Duration::from_secs(2), PollOptions::default());
//!     while let Some(mv) = rx.recv().await {
//!         println!("turn={} next={}", mv.turn, mv.next_player);
//!     }
//! }
//! ```

use std::time::Duration;
use tokio::sync::mpsc;
use encoding_rs::SHIFT_JIS;

use crate::proxy::{send_once, url_encode, BoxError, ProxyMode};
use crate::realtime_map_view::{
    parse_map, parse_players, parse_room_name, MapViewResult,
};

// ----------------------------------------------------------------
// Public types
// ----------------------------------------------------------------

/// Options for `poll_map_view`.
#[derive(Debug, Clone, Default)]
pub struct PollOptions {
    /// Optional proxy URI.  `None` = auto-detect, `Some("")` = direct connection.
    pub proxy_uri: Option<String>,
}

// ----------------------------------------------------------------
// URLs
// ----------------------------------------------------------------

const SERVER_CHECK_URL: &str =
    "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/UserCheck";
const MAP_VIEW_URL: &str =
    "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/MapView.jsp";

// ----------------------------------------------------------------
// Session management
// ----------------------------------------------------------------

/// Authenticates and returns a JSESSIONID.
/// FIX: user/pass are percent-encoded to prevent query parameter injection.
async fn authenticate(
    user:       &str,
    pass:       &str,
    proxy_mode: &ProxyMode,
) -> Result<String, BoxError> {
    let url = format!(
        "{}?user={}&pass={}&select=mapview",
        SERVER_CHECK_URL,
        url_encode(user),
        url_encode(pass),
    );
    let (_, headers, _) = send_once(&url, &[], proxy_mode).await?;
    for val in headers.get_all("set-cookie").iter() {
        for part in val.to_str().unwrap_or("").split(';') {
            if let Some(id) = part.trim().strip_prefix("JSESSIONID=") {
                return Ok(id.to_string());
            }
        }
    }
    Err("JSESSIONID not found in authentication response".into())
}

/// Fetches MapView.jsp using an existing JSESSIONID.
/// Returns `None` if the session has expired.
async fn fetch_map(
    jsessionid: &str,
    proxy_mode: &ProxyMode,
) -> Result<Option<MapViewResult>, BoxError> {
    let cookie = format!("JSESSIONID={}", jsessionid);
    let (status, _, body) =
        send_once(MAP_VIEW_URL, &[("Cookie", cookie)], proxy_mode).await?;

    // Session expired: server redirects (3xx)
    if status >= 300 {
        return Ok(None);
    }

    let (html, _, _) = SHIFT_JIS.decode(&body);

    // Detect login form response (session expired without redirect)
    if html.contains("UserCheck") && html.contains(r#"type="text" name="user""#) {
        return Ok(None);
    }

    let room_name                = parse_room_name(&html);
    let (map, turn, next_player) = parse_map(&html);
    let dom                      = tl::parse(&html, tl::ParserOptions::default())?;
    let players                  = parse_players(&dom);

    Ok(Some(MapViewResult { room_name, turn, next_player, map, players }))
}

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Starts polling the map view at the given interval.
///
/// Errors during polling are logged to `eprintln!` rather than silently
/// discarded, making it possible to diagnose network and parse failures
/// without attaching a dedicated logging framework.
///
/// The background task stops when the returned `Receiver` is dropped.
pub fn poll_map_view(
    user:     impl Into<String>,
    pass:     impl Into<String>,
    interval: Duration,
    opts:     PollOptions,
) -> mpsc::Receiver<MapViewResult> {
    let user = user.into();
    let pass = pass.into();

    let (tx, rx) = mpsc::channel(32);

    tokio::spawn(async move {
        let proxy_mode = ProxyMode::from_option(opts.proxy_uri.as_deref());

        // Authenticate once at startup; retry on failure with logging.
        let mut jsessionid = loop {
            match authenticate(&user, &pass, &proxy_mode).await {
                Ok(id)   => break id,
                // FIX: errors are no longer silently swallowed.
                Err(e)   => {
                    eprintln!("[poll_map_view] authentication failed: {e}; retrying in {interval:?}");
                    tokio::time::sleep(interval).await;
                }
            }
        };

        loop {
            match fetch_map(&jsessionid, &proxy_mode).await {
                Ok(Some(result)) => {
                    if tx.send(result).await.is_err() {
                        // Receiver dropped; stop polling cleanly.
                        break;
                    }
                }
                Ok(None) => {
                    // Session expired; re-authenticate.
                    eprintln!("[poll_map_view] session expired, re-authenticating…");
                    loop {
                        match authenticate(&user, &pass, &proxy_mode).await {
                            Ok(id) => {
                                jsessionid = id;
                                break;
                            }
                            Err(e) => {
                                eprintln!("[poll_map_view] re-authentication failed: {e}; retrying in {interval:?}");
                                tokio::time::sleep(interval).await;
                            }
                        }
                    }
                }
                // FIX: network / parse errors are logged instead of silently skipped.
                Err(e) => {
                    eprintln!("[poll_map_view] fetch error: {e}; skipping tick");
                }
            }

            tokio::time::sleep(interval).await;
        }
    });

    rx
}