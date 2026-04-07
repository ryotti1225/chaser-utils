//! CHaser Online game map view scraper.
//!
//! # Quick start
//!
//! ```no_run
//! use chaser_util::realtime_map_view::{fetch_map_view, MapViewOptions};
//!
//! #[tokio::main]
//! async fn main() {
//!     let result = fetch_map_view("hot", "hot", MapViewOptions::default()).await.unwrap();
//!     println!("room={} turn={} next={}", result.room_name, result.turn, result.next_player);
//! }
//! ```

use bytes::Bytes;
use regex::Regex;
use std::sync::OnceLock;
use encoding_rs::SHIFT_JIS;
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::proxy::matcher::Matcher;
use hyper_util::rt::TokioExecutor;

// ----------------------------------------------------------------
// Public data types
// ----------------------------------------------------------------

/// One map cell (numeric part of the image filename).
/// e.g. "012.gif" -> 12, "000.gif" -> 0
pub type TileId = u32;

/// Server base URL used to build tile image URLs.
const SERVER_IMAGE_BASE: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/img/";

/// Returns the full image URL for a given TileId.
/// e.g. 12 -> "http://www7019ug.sakura.ne.jp/CHaserOnline003/img/012.gif"
pub fn tile_image_url(tile: TileId) -> String {
    format!("{}{:03}.gif", SERVER_IMAGE_BASE, tile)
}

/// Player information (from the right-side table).
#[derive(Debug, Clone)]
pub struct PlayerInfo {
    pub username: String,
    pub attr_a:   i32,   // A: attack
    pub attr_i:   i32,   // I: intelligence
    pub attr_p:   i32,   // P: power
    pub attr_pd:  i32,   // PD: power defense
    pub attr_t:   i32,   // T: total
    /// Command list (one entry per command line, e.g. "gr 12,0,12").
    pub commands: Vec<String>,
}

/// Full result of a map view fetch.
#[derive(Debug, Clone)]
pub struct MapViewResult {
    /// Room name extracted from the H1 tag "[...]", e.g. "Renshuu_x5".
    pub room_name:   String,
    /// Current turn number.
    pub turn:        u32,
    /// Username of the player who acts next.
    pub next_player: String,
    /// Map cells as a 2D array [row][col].
    pub map:         Vec<Vec<TileId>>,
    /// List of player information.
    pub players:     Vec<PlayerInfo>,
}

/// Fetch options.
#[derive(Debug, Clone, Default)]
pub struct MapViewOptions {
    /// Optional proxy URI. None = auto-detect, Some("") = direct connection.
    pub proxy_uri: Option<String>,
}

// ----------------------------------------------------------------
// Proxy mode (internal)
// ----------------------------------------------------------------

enum ProxyMode {
    Auto,
    Direct,
    Manual(String),
}

#[derive(Clone)]
struct ProxyConnector {
    inner:      HttpConnector,
    proxy_host: String,
    proxy_port: u16,
}

impl tower_service::Service<http::Uri> for ProxyConnector {
    type Response = <HttpConnector as tower_service::Service<http::Uri>>::Response;
    type Error    = <HttpConnector as tower_service::Service<http::Uri>>::Error;
    type Future   = <HttpConnector as tower_service::Service<http::Uri>>::Future;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>)
        -> std::task::Poll<Result<(), Self::Error>>
    {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, _uri: http::Uri) -> Self::Future {
        let proxy_uri: http::Uri =
            format!("http://{}:{}", self.proxy_host, self.proxy_port)
            .parse()
            .unwrap_or_else(|_| http::Uri::from_static("http://127.0.0.1:8080"));
        self.inner.call(proxy_uri)
    }
}

// ----------------------------------------------------------------
// HTTP helpers (internal)
// ----------------------------------------------------------------

async fn send_once(
    url:        &str,
    extra:      &[(&'static str, String)],
    proxy_mode: &ProxyMode,
) -> Result<(u16, hyper::HeaderMap, Bytes), Box<dyn std::error::Error + Send + Sync>> {
    let target_uri: http::Uri = url.parse()?;

    macro_rules! build_req {
        ($b:expr) => {{
            let mut b = $b;
            for (k, v) in extra { b = b.header(*k, v.as_str()); }
            b.body(Empty::<Bytes>::new())?
        }};
    }

    let resp = match proxy_mode {
        ProxyMode::Auto => {
            let matcher = Matcher::from_system();
            if let Some(intercept) = matcher.intercept(&target_uri) {
                let ph = intercept.uri().host().unwrap_or("127.0.0.1").to_string();
                let pp = intercept.uri().port_u16().unwrap_or(8080);
                let mut conn = HttpConnector::new();
                conn.enforce_http(false);
                let client = Client::builder(TokioExecutor::new())
                    .build::<_, Empty<Bytes>>(ProxyConnector {
                        inner: conn, proxy_host: ph, proxy_port: pp,
                    });
                let mut b = Request::builder().method("GET").uri(url)
                    .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");
                if let Some(auth) = intercept.basic_auth() {
                    b = b.header("Proxy-Authorization", auth);
                }
                client.request(build_req!(b)).await?
            } else {
                let mut conn = HttpConnector::new();
                conn.enforce_http(false);
                let client = Client::builder(TokioExecutor::new())
                    .build::<_, Empty<Bytes>>(conn);
                let b = Request::builder().method("GET").uri(url)
                    .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");
                client.request(build_req!(b)).await?
            }
        }
        ProxyMode::Direct => {
            let mut conn = HttpConnector::new();
            conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new())
                .build::<_, Empty<Bytes>>(conn);
            let b = Request::builder().method("GET").uri(url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");
            client.request(build_req!(b)).await?
        }
        ProxyMode::Manual(proxy_uri_str) => {
            let proxy_uri: http::Uri = proxy_uri_str.parse()?;
            let ph = proxy_uri.host().unwrap_or("127.0.0.1").to_string();
            let pp = proxy_uri.port_u16().unwrap_or(8080);
            let mut conn = HttpConnector::new();
            conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new())
                .build::<_, Empty<Bytes>>(ProxyConnector {
                    inner: conn, proxy_host: ph, proxy_port: pp,
                });
            let b = Request::builder().method("GET").uri(url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");
            client.request(build_req!(b)).await?
        }
    };

    let status  = resp.status().as_u16();
    let headers = resp.headers().clone();
    let body    = resp.into_body().collect().await?.to_bytes();
    Ok((status, headers, body))
}

async fn send_follow_redirects(
    start_url:  &str,
    extra:      &[(&'static str, String)],
    proxy_mode: &ProxyMode,
) -> Result<(Bytes, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    let mut url      = start_url.to_string();
    let mut jsession = None::<String>;

    for _ in 0..10 {
        let (status, headers, body) = send_once(&url, extra, proxy_mode).await?;

        for val in headers.get_all("set-cookie").iter() {
            for part in val.to_str().unwrap_or("").split(';') {
                if let Some(id) = part.trim().strip_prefix("JSESSIONID=") {
                    jsession = Some(id.to_string());
                }
            }
        }

        if (300..400).contains(&status) {
            if let Some(loc) = headers.get("location") {
                url = loc.to_str()?.to_string();
                continue;
            }
        }

        return Ok((body, jsession));
    }
    Err("too many redirects".into())
}

// ----------------------------------------------------------------
// HTML parsing
// ----------------------------------------------------------------

fn inner_text<'a>(node: &tl::Node<'a>, parser: &'a tl::Parser<'a>) -> String {
    match node {
        tl::Node::Raw(b)   => b.as_utf8_str().into_owned(),
        tl::Node::Tag(tag) => tag
            .children().top().iter()
            .filter_map(|h| h.get(parser))
            .map(|n| inner_text(n, parser))
            .collect(),
        _ => String::new(),
    }
}

/// Extracts "[RoomName]" from the H1 element using a lazy regex.
/// e.g. "[map view title] [Renshuu_x3]" -> "Renshuu_x3"
fn parse_room_name(html: &str) -> String {
    re_room_name()
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default()
}

/// Parses "turn=N Next=PlayerName" text into (turn, next_player).
fn parse_turn_info(text: &str) -> (u32, String) {
    let mut turn        = 0u32;
    let mut next_player = String::new();

    for token in text.split_whitespace() {
        if let Some(v) = token.strip_prefix("turn=") {
            turn = v.parse().unwrap_or(0);
        } else if let Some(v) = token.strip_prefix("Next=") {
            next_player = v.to_string();
        }
    }
    (turn, next_player)
}

// ----------------------------------------------------------------
// Lazy-compiled regexes (compiled once at first use via OnceLock)
// ----------------------------------------------------------------

/// Matches /img/NNN.gif (1-3 digit tile IDs only) directly in a tr slice.
fn re_img_src() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"/img/(\d{1,3})\.gif").unwrap())
}

/// Matches "turn=N Next=PlayerName".
fn re_turn() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"turn=(\d+)\s+Next=([^<\s]+)").unwrap())
}

/// Matches "[RoomName]" inside an H1 element.
fn re_room_name() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)<h1[^>]*>[^\[]*\[([^\]]+)\]").unwrap())
}

/// Parses the map table into a 2D TileId array.
/// - turn/next extracted via re_turn()
/// - Table located by string search; rows split by <tr position
/// - Tile IDs extracted per row via re_img_src() (no dot-all regex, fast)
fn parse_map(html: &str) -> (Vec<Vec<TileId>>, u32, String) {
    // Extract turn number and next player
    let (turn, next_player) = re_turn()
        .captures(html)
        .map(|c| (
            c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0),
            c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default(),
        ))
        .unwrap_or((0, String::new()));

    // Locate the map table by attribute; lowercase conversion done only once
    let lower = html.to_ascii_lowercase();
    let table_start = match lower.find(r#"cellpadding="0""#)
        .and_then(|i| html[..i].rfind('<'))
    {
        Some(i) => i,
        None    => return (vec![], turn, next_player),
    };
    let table_end = match lower[table_start..].find("</table") {
        Some(i) => table_start + i,
        None    => return (vec![], turn, next_player),
    };
    let table_html  = &html[table_start..table_end];
    let table_lower = &lower[table_start..table_end];

    // Collect <tr positions to delimit row ranges
    let tr_positions: Vec<usize> = table_lower
        .match_indices("<tr")
        .map(|(i, _)| i)
        .collect();

    let mut map: Vec<Vec<TileId>> = Vec::new();
    for (idx, &tr_start) in tr_positions.iter().enumerate() {
        let tr_end = tr_positions.get(idx + 1).copied().unwrap_or(table_html.len());
        let tr_slice = &table_html[tr_start..tr_end];

        let row: Vec<TileId> = re_img_src()
            .captures_iter(tr_slice)
            .filter_map(|c| c.get(1).and_then(|m| {
                let id: u32 = m.as_str().parse().ok()?;
                // Exclude player tile IDs (1000-9000 range)
                if id >= 1000 { return None; }
                Some(id)
            }))
            .collect();

        if !row.is_empty() {
            map.push(row);
        }
    }

    (map, turn, next_player)
}

/// Parses the player information table.
/// Format: "[img] username A:N I:N P:N PD:N T:N"
fn parse_players(dom: &tl::VDom) -> Vec<PlayerInfo> {
    let parser = dom.parser();

    // The border=1 table contains player information
    let player_table = match dom
        .query_selector(r#"table[border="1"]"#)
        .and_then(|mut q| q.next())
        .and_then(|h| h.get(parser))
    {
        Some(tl::Node::Tag(t)) => t,
        _ => return vec![],
    };

    let table_html: String = player_table
        .children().top().iter()
        .filter_map(|h| h.get(parser))
        .map(|n| n.outer_html(parser).to_string())
        .collect();

    let Ok(dom2) = tl::parse(&table_html, tl::ParserOptions::default()) else {
        return vec![];
    };
    let p2 = dom2.parser();

    let mut players = Vec::new();

    // Table structure:
    //   tr[0]: each td[valign="top"] holds one player's info
    //   tr[1]: each td[valign="top"] holds one player's command list (<font size="2">)
    // Match by index.

    // Collect tr HTML strings
    let tr_htmls: Vec<String> = dom2
        .query_selector("tr").into_iter().flatten()
        .filter_map(|h| h.get(p2))
        .filter_map(|n| match n {
            tl::Node::Tag(t) => Some(
                t.children().top().iter()
                    .filter_map(|h| h.get(p2))
                    .map(|n| n.outer_html(p2).to_string())
                    .collect::<String>()
            ),
            _ => None,
        })
        .collect();

    // tr[0]: parse player info from each td
    let mut player_data: Vec<(String, i32, i32, i32, i32, i32)> = Vec::new();
    if let Some(tr0) = tr_htmls.first() {
        if let Ok(d) = tl::parse(tr0, tl::ParserOptions::default()) {
            let p = d.parser();
            // Collect td HTML strings first to avoid lifetime issues
            let td_htmls: Vec<String> = d.query_selector(r#"td[valign="top"]"#)
                .into_iter().flatten()
                .filter_map(|h| h.get(p))
                .filter_map(|n| match n {
                    tl::Node::Tag(t) => Some(
                        t.children().top().iter()
                            .filter_map(|h| h.get(p))
                            .map(|n| n.outer_html(p).to_string())
                            .collect::<String>()
                    ),
                    _ => None,
                })
                .collect();

            for td_html in &td_htmls {
                if let Ok(td_dom) = tl::parse(td_html, tl::ParserOptions::default()) {
                    let tp = td_dom.parser();
                    // Collect text from all nodes
                    let text: String = td_dom.nodes().iter()
                        .map(|n| inner_text(n, tp))
                        .collect::<Vec<_>>()
                        .join(" ");

                    let mut username  = String::new();
                    let mut attr_a    = 0i32;
                    let mut attr_i    = 0i32;
                    let mut attr_p    = 0i32;
                    let mut attr_pd   = 0i32;
                    let mut attr_t    = 0i32;
                    let mut found_any = false;

                    for token in text.split_whitespace() {
                        if let Some(v) = token.strip_prefix("A:") {
                            attr_a = v.parse().unwrap_or(0); found_any = true;
                        } else if let Some(v) = token.strip_prefix("I:") {
                            attr_i = v.parse().unwrap_or(0);
                        } else if let Some(v) = token.strip_prefix("PD:") {
                            attr_pd = v.parse().unwrap_or(0);
                        } else if let Some(v) = token.strip_prefix("P:") {
                            attr_p = v.trim_end_matches(']').parse().unwrap_or(0);
                        } else if let Some(v) = token.strip_prefix("T:") {
                            attr_t = v.trim_end_matches(']').parse().unwrap_or(0);
                        } else if username.is_empty() && !token.starts_with('[') && !token.is_empty() {
                            username = token.to_string();
                        }
                    }
                    if found_any {
                        player_data.push((username, attr_a, attr_i, attr_p, attr_pd, attr_t));
                    }
                }
            }
        }
    }

    // tr[1]: parse command list from each font element
    let mut all_commands: Vec<Vec<String>> = Vec::new();
    if let Some(tr1) = tr_htmls.get(1) {
        if let Ok(d) = tl::parse(tr1, tl::ParserOptions::default()) {
            let p = d.parser();
            let font_htmls: Vec<String> = d.query_selector("font")
                .into_iter().flatten()
                .filter_map(|h| h.get(p))
                .filter_map(|n| match n {
                    tl::Node::Tag(t) => Some(
                        t.children().top().iter()
                            .filter_map(|h| h.get(p))
                            .map(|n| n.outer_html(p).to_string())
                            .collect::<String>()
                    ),
                    _ => None,
                })
                .collect();

            for font_html in &font_htmls {
                if let Ok(fd) = tl::parse(font_html, tl::ParserOptions::default()) {
                    let fp = fd.parser();
                    let text = fd.nodes().iter()
                        .map(|n| inner_text(n, fp))
                        .collect::<String>();
                    let cmds: Vec<String> = text.lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect();
                    all_commands.push(cmds);
                }
            }
        }
    }

    // Pair player_data with all_commands by index to build PlayerInfo entries
    for (i, (username, attr_a, attr_i, attr_p, attr_pd, attr_t)) in player_data.into_iter().enumerate() {
        let commands = all_commands.get(i).cloned().unwrap_or_default();
        players.push(PlayerInfo { username, attr_a, attr_i, attr_p, attr_pd, attr_t, commands });
    }

    players
}

// ----------------------------------------------------------------
// URLs (Server path is separate from MeetingPlace)
// ----------------------------------------------------------------

/// Base URL for obtaining a JSESSIONID (Server side).
const SERVER_BASE_URL: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/";
/// Authentication endpoint.
const SERVER_CHECK_URL: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/UserCheck";
/// Map view page.
const MAP_VIEW_URL: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/MapView.jsp";

// ----------------------------------------------------------------
// Core logic
// ----------------------------------------------------------------

async fn fetch_inner(
    user:       &str,
    pass:       &str,
    proxy_mode: ProxyMode,
) -> Result<MapViewResult, Box<dyn std::error::Error + Send + Sync>> {
    // Step 1+2 combined: fetch JSESSIONID and authenticate in a single request
    let check_url = format!("{}?user={}&pass={}&select=mapview", SERVER_CHECK_URL, user, pass);
    let (_, jsession) = send_follow_redirects(
        &check_url,
        &[],
        &proxy_mode,
    ).await?;
    let jsessionid = jsession.ok_or("Server JSESSIONID not found")?;
    let cookie = format!("JSESSIONID={}", jsessionid);

    let (body, _) = send_follow_redirects(
        MAP_VIEW_URL, &[("Cookie", cookie)], &proxy_mode,
    ).await?;
    let (html, _, _) = SHIFT_JIS.decode(&body);

    let room_name                = parse_room_name(&html);
    let (map, turn, next_player) = parse_map(&html);

    let dom     = tl::parse(&html, tl::ParserOptions::default())?;
    let players = parse_players(&dom);

    Ok(MapViewResult { room_name, turn, next_player, map, players })
}

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Fetches the real-time game map view.
///
/// # Arguments
/// * `user` - Login username (managed by caller; changes per game)
/// * `pass` - Login password (managed by caller; changes per game)
/// * `opts` - Fetch options
pub async fn fetch_map_view(
    user: &str,
    pass: &str,
    opts: MapViewOptions,
) -> Result<MapViewResult, Box<dyn std::error::Error + Send + Sync>> {
    let proxy_mode = match opts.proxy_uri {
        None                        => ProxyMode::Auto,
        Some(ref s) if s.is_empty() => ProxyMode::Direct,
        Some(s)                     => ProxyMode::Manual(s),
    };
    fetch_inner(user, pass, proxy_mode).await
}