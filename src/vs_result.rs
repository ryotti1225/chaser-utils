//! CHaser Online battle result scraper.
//!
//! Fetches and parses the result list from:
//!   Server/ -> vsResultView.jsp (form) -> listvsresult (GET with query params)
//!
//! # Quick start
//!
//! ```no_run
//! use chaser_util::vs_result::{fetch_vs_result, VsResultQuery};
//!
//! #[tokio::main]
//! async fn main() {
//!     let query = VsResultQuery::default();
//!     let results = fetch_vs_result("cool30", "cool", query, None).await.unwrap();
//!     for r in &results {
//!         println!("room={} {} vs {} -> {} : {}", r.room, r.players[0].username, r.players[1].username, r.players[0].total_point, r.players[1].total_point);
//!     }
//! }
//! ```

use bytes::Bytes;
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

/// One player's result within a battle.
#[derive(Debug, Clone, Default)]
pub struct PlayerResult {
    /// Connection order (1 = first connected)
    pub order:          u32,
    pub username:       String,
    /// Number of turns the player acted
    pub get_turn:       i32,
    /// Remaining turns at game end
    pub rem_turn:       i32,
    pub total_point:    i32,
    pub action_point:   Option<i32>,
    pub item_point:     Option<i32>,
    pub put_point:      Option<i32>,
    pub put_damage:     Option<i32>,
}

/// One battle result (one room, one session).
#[derive(Debug, Clone)]
pub struct BattleResult {
    pub room:       u32,
    pub start_time: String,
    pub end_time:   String,
    /// All players in this battle, in connection order.
    pub players:    Vec<PlayerResult>,
}

/// Query parameters for listvsresult.
/// Defaults match a broad search (today, all rooms, all turns, no score filter).
#[derive(Debug, Clone)]
pub struct VsResultQuery {
    pub min_start_date:  String,
    pub min_end_date:    String,
    pub min_start_time:  String,
    pub min_end_time:    String,
    pub min_room:        u32,
    pub min_turn:        u32,
    pub max_start_date:  String,
    pub max_end_date:    String,
    pub max_start_time:  String,
    pub max_end_time:    String,
    pub max_room:        u32,
    pub max_turn:        u32,
    pub min_action_point: i32,
    pub min_item_point:   i32,
    pub min_put_point:    i32,
    pub min_put_damage:   i32,
    pub min_total_point:  i32,
    pub min_rem_turn:     i32,
    pub min_get_turn:     i32,
    pub max_action_point: i32,
    pub max_item_point:   i32,
    pub max_put_point:    i32,
    pub max_put_damage:   i32,
    pub max_total_point:  i32,
    pub max_rem_turn:     i32,
    pub max_get_turn:     i32,
}

impl Default for VsResultQuery {
    fn default() -> Self {
        // Default: today, all rooms (1-10000), turns 1-8, no score filter
        let today = {
            // Use a fixed default; caller should override with actual date
            "2026-04-07".to_string()
        };
        Self {
            min_start_date:  today.clone(),
            min_end_date:    today.clone(),
            min_start_time:  "00:00:00".to_string(),
            min_end_time:    "00:00:00".to_string(),
            min_room:        1,
            min_turn:        1,
            max_start_date:  today.clone(),
            max_end_date:    today.clone(),
            max_start_time:  "23:59:59".to_string(),
            max_end_time:    "23:59:59".to_string(),
            max_room:        10000,
            max_turn:        8,
            min_action_point: -1_000_000,
            min_item_point:   -1_000_000,
            min_put_point:    0,
            min_put_damage:   -20_000_000,
            min_total_point:  -30_000_000,
            min_rem_turn:     -10_000,
            min_get_turn:     0,
            max_action_point: 1_000_000,
            max_item_point:   1_000_000,
            max_put_point:    20_000_000,
            max_put_damage:   0,
            max_total_point:  20_000_000,
            max_rem_turn:     10_000,
            max_get_turn:     10_000,
        }
    }
}

impl VsResultQuery {
    /// Encode into a query string for listvsresult.
    fn to_query_string(&self) -> String {
        format!(
            "minStartDate={}&minEndDate={}&minStartTime={}&minEndTime={}\
             &minRoomNumber={}&minTurnNumber={}\
             &maxStartDate={}&maxEndDate={}&maxStartTime={}&maxEndTime={}\
             &maxRoomNumber={}&maxTurnNumber={}\
             &minActionPoint={}&minItemPoint={}&minPutPoint={}&minPutDamage={}\
             &minTotalPoint={}&minRemTurn={}&minGetTurn={}\
             &maxActionPoint={}&maxItemPoint={}&maxPutPoint={}&maxPutDamage={}\
             &maxTotalPoint={}&maxRemTurn={}&maxGetTurn={}",
            self.min_start_date, self.min_end_date,
            encode(&self.min_start_time), encode(&self.min_end_time),
            self.min_room, self.min_turn,
            self.max_start_date, self.max_end_date,
            encode(&self.max_start_time), encode(&self.max_end_time),
            self.max_room, self.max_turn,
            self.min_action_point, self.min_item_point,
            self.min_put_point, self.min_put_damage,
            self.min_total_point, self.min_rem_turn, self.min_get_turn,
            self.max_action_point, self.max_item_point,
            self.max_put_point, self.max_put_damage,
            self.max_total_point, self.max_rem_turn, self.max_get_turn,
        )
    }
}

fn encode(s: &str) -> String {
    s.replace(':', "%3A").replace(' ', "+")
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
// HTTP helpers
// ----------------------------------------------------------------

async fn get_url(
    url:        &str,
    cookie:     &str,
    proxy_mode: &ProxyMode,
) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
    let target_uri: http::Uri = url.parse()?;

    macro_rules! do_req {
        ($client:expr) => {{
            let mut b = Request::builder().method("GET").uri(url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");
            if !cookie.is_empty() { b = b.header("Cookie", cookie); }
            let resp = $client.request(b.body(Empty::<Bytes>::new())?).await?;
            resp.into_body().collect().await?.to_bytes()
        }};
    }

    let body = match proxy_mode {
        ProxyMode::Auto => {
            let matcher = Matcher::from_system();
            if let Some(intercept) = matcher.intercept(&target_uri) {
                let ph = intercept.uri().host().unwrap_or("127.0.0.1").to_string();
                let pp = intercept.uri().port_u16().unwrap_or(8080);
                let mut conn = HttpConnector::new(); conn.enforce_http(false);
                let client = Client::builder(TokioExecutor::new())
                    .build::<_, Empty<Bytes>>(ProxyConnector { inner: conn, proxy_host: ph, proxy_port: pp });
                do_req!(client)
            } else {
                let mut conn = HttpConnector::new(); conn.enforce_http(false);
                let client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(conn);
                do_req!(client)
            }
        }
        ProxyMode::Direct => {
            let mut conn = HttpConnector::new(); conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(conn);
            do_req!(client)
        }
        ProxyMode::Manual(proxy_uri_str) => {
            let proxy_uri: http::Uri = proxy_uri_str.parse()?;
            let ph = proxy_uri.host().unwrap_or("127.0.0.1").to_string();
            let pp = proxy_uri.port_u16().unwrap_or(8080);
            let mut conn = HttpConnector::new(); conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new())
                .build::<_, Empty<Bytes>>(ProxyConnector { inner: conn, proxy_host: ph, proxy_port: pp });
            do_req!(client)
        }
    };
    Ok(body)
}

fn extract_jsessionid(headers: &hyper::HeaderMap) -> Option<String> {
    for val in headers.get_all("set-cookie").iter() {
        for part in val.to_str().unwrap_or("").split(';') {
            if let Some(id) = part.trim().strip_prefix("JSESSIONID=") {
                return Some(id.to_string());
            }
        }
    }
    None
}

async fn get_with_jsession(
    url:        &str,
    proxy_mode: &ProxyMode,
) -> Result<(Bytes, String), Box<dyn std::error::Error + Send + Sync>> {
    let target_uri: http::Uri = url.parse()?;

    macro_rules! do_req {
        ($client:expr) => {{
            let b = Request::builder().method("GET").uri(url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36");
            let resp = $client.request(b.body(Empty::<Bytes>::new())?).await?;
            let headers = resp.headers().clone();
            let body = resp.into_body().collect().await?.to_bytes();
            (body, headers)
        }};
    }

    let (body, headers) = match proxy_mode {
        ProxyMode::Auto => {
            let matcher = Matcher::from_system();
            if let Some(intercept) = matcher.intercept(&target_uri) {
                let ph = intercept.uri().host().unwrap_or("127.0.0.1").to_string();
                let pp = intercept.uri().port_u16().unwrap_or(8080);
                let mut conn = HttpConnector::new(); conn.enforce_http(false);
                let client = Client::builder(TokioExecutor::new())
                    .build::<_, Empty<Bytes>>(ProxyConnector { inner: conn, proxy_host: ph, proxy_port: pp });
                do_req!(client)
            } else {
                let mut conn = HttpConnector::new(); conn.enforce_http(false);
                let client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(conn);
                do_req!(client)
            }
        }
        ProxyMode::Direct => {
            let mut conn = HttpConnector::new(); conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(conn);
            do_req!(client)
        }
        ProxyMode::Manual(proxy_uri_str) => {
            let proxy_uri: http::Uri = proxy_uri_str.parse()?;
            let ph = proxy_uri.host().unwrap_or("127.0.0.1").to_string();
            let pp = proxy_uri.port_u16().unwrap_or(8080);
            let mut conn = HttpConnector::new(); conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new())
                .build::<_, Empty<Bytes>>(ProxyConnector { inner: conn, proxy_host: ph, proxy_port: pp });
            do_req!(client)
        }
    };

    let jsessionid = extract_jsessionid(&headers).unwrap_or_default();
    Ok((body, jsessionid))
}

// ----------------------------------------------------------------
// HTML parsing
// ----------------------------------------------------------------

fn cell_text(s: &str) -> String {
    // Trim whitespace and replace full-width space (ideographic space U+3000 / &nbsp; equiv)
    // used as empty cell marker in this server's HTML
    let t = s.trim().replace('\u{3000}', "").replace('\u{00a0}', "").trim().to_string();
    t
}

fn parse_opt_i32(s: &str) -> Option<i32> {
    let t = cell_text(s);
    if t.is_empty() { None } else { t.parse().ok() }
}

fn parse_i32(s: &str) -> i32 {
    cell_text(s).parse().unwrap_or(0)
}

fn parse_u32(s: &str) -> u32 {
    cell_text(s).parse().unwrap_or(0)
}

/// Parses the listvsresult HTML table into a list of BattleResult.
///
/// The table rows are grouped by battle: the first player row in a group
/// contains room number, start time, end time, and score details.
/// Subsequent rows in the same group have empty room/time cells (full-width space).
fn parse_results(html: &str) -> Vec<BattleResult> {
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let parser = dom.parser();

    // Find the border=1 table
    let table = match dom
        .query_selector(r#"table[border="1"]"#)
        .and_then(|mut q| q.next())
        .and_then(|h| h.get(parser))
    {
        Some(tl::Node::Tag(t)) => t,
        _ => return vec![],
    };

    // Collect all tr inner HTML
    let tr_htmls: Vec<String> = table
        .children().top().iter()
        .filter_map(|h| h.get(parser))
        .filter_map(|n| match n {
            tl::Node::Tag(t) if t.name().as_bytes().eq_ignore_ascii_case(b"tbody") => {
                Some(t.children().top().iter()
                    .filter_map(|h| h.get(parser))
                    .filter_map(|n| match n {
                        tl::Node::Tag(tr) if tr.name().as_bytes().eq_ignore_ascii_case(b"tr") =>
                            Some(tr.outer_html(parser).to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>())
            }
            tl::Node::Tag(t) if t.name().as_bytes().eq_ignore_ascii_case(b"tr") =>
                Some(vec![t.outer_html(parser).to_string()]),
            _ => None,
        })
        .flatten()
        .collect();

    let mut results: Vec<BattleResult> = Vec::new();
    let mut skip_header = true;

    for tr_html in &tr_htmls {
        // Parse cells from this row
        let tr_dom = match tl::parse(tr_html, tl::ParserOptions::default()) {
            Ok(d) => d, Err(_) => continue,
        };
        let tp = tr_dom.parser();

        let cells: Vec<String> = tr_dom
            .query_selector("td").into_iter().flatten()
            .filter_map(|h| h.get(tp))
            .map(|n| {
                // Collect all text within the td
                fn text<'a>(node: &tl::Node<'a>, p: &'a tl::Parser<'a>) -> String {
                    match node {
                        tl::Node::Raw(b) => b.as_utf8_str().into_owned(),
                        tl::Node::Tag(t) => t.children().top().iter()
                            .filter_map(|h| h.get(p))
                            .map(|n| text(n, p))
                            .collect(),
                        _ => String::new(),
                    }
                }
                text(n, tp)
            })
            .collect();

        if cells.len() < 12 { continue; }

        // Skip header row
        if skip_header {
            skip_header = false;
            continue;
        }

        // A new battle starts when col 1 (start_time) is non-empty.
        // col 0 (room) is only filled on the first row of each battle group;
        // subsequent battles in the same group have an empty room cell but a new start_time.
        let start_time_text = cell_text(&cells[1]);
        let is_new_battle = !start_time_text.is_empty();

        if is_new_battle {
            let player = PlayerResult {
                order:        parse_u32(&cells[3]),
                username:     cell_text(&cells[4]),
                get_turn:     parse_i32(&cells[5]),
                rem_turn:     parse_i32(&cells[6]),
                total_point:  parse_i32(&cells[7]),
                action_point: parse_opt_i32(&cells[8]),
                item_point:   parse_opt_i32(&cells[9]),
                put_point:    parse_opt_i32(&cells[10]),
                put_damage:   parse_opt_i32(&cells[11]),
            };
            // Room number is only present on the first battle in a group;
            // reuse the last known room number for subsequent battles in the same group.
            let room = {
                let r = parse_u32(&cells[0]);
                if r > 0 { r } else { results.last().map(|b| b.room).unwrap_or(0) }
            };
            results.push(BattleResult {
                room,
                start_time: start_time_text,
                end_time:   cell_text(&cells[2]),
                players:    vec![player],
            });
        } else {
            // Continuation row: add player to the last battle
            if let Some(last) = results.last_mut() {
                last.players.push(PlayerResult {
                    order:        parse_u32(&cells[3]),
                    username:     cell_text(&cells[4]),
                    get_turn:     parse_i32(&cells[5]),
                    rem_turn:     parse_i32(&cells[6]),
                    total_point:  parse_i32(&cells[7]),
                    action_point: parse_opt_i32(&cells[8]),
                    item_point:   parse_opt_i32(&cells[9]),
                    put_point:    parse_opt_i32(&cells[10]),
                    put_damage:   parse_opt_i32(&cells[11]),
                });
            }
        }
    }

    results
}

// ----------------------------------------------------------------
// URLs
// ----------------------------------------------------------------

const SERVER_BASE_URL:   &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/";
const SERVER_CHECK_URL:  &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/UserCheck";
const LIST_RESULT_URL:   &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/listvsresult";

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Fetches and parses the battle result list.
///
/// # Arguments
/// * `user`       - Login username
/// * `pass`       - Login password
/// * `query`      - Search parameters (use `VsResultQuery::default()` for today's results)
/// * `proxy_uri`  - Optional proxy URI; None = auto-detect, Some("") = direct connection
pub async fn fetch_vs_result(
    user:      &str,
    pass:      &str,
    query:     VsResultQuery,
    proxy_uri: Option<&str>,
) -> Result<Vec<BattleResult>, Box<dyn std::error::Error + Send + Sync>> {
    let proxy_mode = match proxy_uri {
        None              => ProxyMode::Auto,
        Some(s) if s.is_empty() => ProxyMode::Direct,
        Some(s)           => ProxyMode::Manual(s.to_string()),
    };

    // Step 1: Get JSESSIONID from Server base
    let (_, jsessionid) = get_with_jsession(SERVER_BASE_URL, &proxy_mode).await?;
    if jsessionid.is_empty() {
        return Err("JSESSIONID not found".into());
    }
    let cookie = format!("JSESSIONID={}", jsessionid);

    // Step 2: Authenticate via UserCheck with vsresultview selection
    let check_url = format!("{}?user={}&pass={}&select=vsresultview", SERVER_CHECK_URL, user, pass);
    let _ = get_url(&check_url, &cookie, &proxy_mode).await?;

    // Step 3: Fetch listvsresult with query parameters
    let list_url = format!("{}?{}", LIST_RESULT_URL, query.to_query_string());
    let body = get_url(&list_url, &cookie, &proxy_mode).await?;

    // Step 4: Decode Shift-JIS and parse
    let (html, _, _) = SHIFT_JIS.decode(&body);
    Ok(parse_results(&html))
}