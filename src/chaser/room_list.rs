//! CHaser Online MeetingPlace scraper library
//!
//! # Quick start
//!
//! ```no_run
//! use chaser_util::chaser::room_list::{scrape, scrape_with_proxy, ScrapeOptions, RoomFilter, UserFilter};
//!
//! #[tokio::main]
//! async fn main() {
//!     // Auto proxy detection (Windows registry / macOS SCF / env vars)
//!     let result = scrape("hot", "hot", ScrapeOptions::default()).await.unwrap();
//!
//!     // Manual proxy
//!     let result = scrape_with_proxy(
//!         "hot", "hot",
//!         "http://proxy.example.com:8080",
//!         ScrapeOptions::default(),
//!     ).await.unwrap();
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

/// One room entry from the public room table
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub room:            u32,
    pub max_connections: u32,
    pub map_display:     String,  // "可" / "否"
    pub public_date:     String,
    pub patrol:          String,  // "有" / "×"
    pub remarks:         String,
}

/// One logged-in user row
#[derive(Debug, Clone)]
pub struct LoggedInUser {
    pub order:    u32,
    pub username: String,
    pub room:     u32,
    pub state:    u32,
}

/// Full scrape result
#[derive(Debug, Clone)]
pub struct ScrapeResult {
    /// None => "ログイン中のユーザーはいません"
    pub logged_in_users: Option<Vec<LoggedInUser>>,
    pub rooms:           Vec<RoomInfo>,
}

// ----------------------------------------------------------------
// Constants
// ----------------------------------------------------------------

/// マップ表示
#[allow(non_snake_case)]
pub mod MapDisplay {
    pub fn Enabled()  -> String { String::from("\u{53ef}") }
    pub fn Disabled() -> String { String::from("\u{5426}") }
}

/// 巡回
#[allow(non_snake_case)]
pub mod Patrol {
    pub fn Yes() -> String { String::from("\u{6709}") }
    pub fn No()  -> String { String::from("\u{00d7}") }
}

/// 備考
#[allow(non_snake_case)]
pub mod Remarks {
    pub fn Ra()  -> String { String::from("\u{30e9}") }
    pub fn Sai() -> String { String::from("\u{57fc}") }
    pub fn Zen() -> String { String::from("\u{5168}") }
}

// ----------------------------------------------------------------
// Filter types
// ----------------------------------------------------------------

/// Filter for room list.  All fields are `None` by default (= no filter).
#[derive(Debug, Clone, Default)]
pub struct RoomFilter {
    // ---- room number ----
    /// Exact room number match
    pub room:             Option<u32>,
    /// Room number >= this value
    pub room_min:         Option<u32>,
    /// Room number <= this value
    pub room_max:         Option<u32>,

    // ---- max_connections ----
    /// max_connections >= this value
    pub min_max_conn:     Option<u32>,
    /// max_connections <= this value
    pub max_max_conn:     Option<u32>,

    // ---- map_display ("可" / "否") ----
    /// Exact match  e.g. Some("可".to_string())
    pub map_display:      Option<String>,

    // ---- public_date ----
    /// Exact match  e.g. Some("4/1".to_string())
    pub public_date:      Option<String>,
    /// Contains substring  e.g. Some("7月".to_string())
    pub public_date_contains: Option<String>,

    // ---- patrol ("有" / "×") ----
    /// Exact match
    pub patrol:           Option<String>,

    // ---- remarks ----
    /// Exact match
    pub remarks:          Option<String>,
    /// Contains substring
    pub remarks_contains: Option<String>,
}

impl RoomFilter {
    pub fn matches(&self, r: &RoomInfo) -> bool {
        if let Some(n)     = self.room              { if r.room != n                              { return false; } }
        if let Some(n)     = self.room_min           { if r.room < n                               { return false; } }
        if let Some(n)     = self.room_max           { if r.room > n                               { return false; } }
        if let Some(n)     = self.min_max_conn       { if r.max_connections < n                    { return false; } }
        if let Some(n)     = self.max_max_conn       { if r.max_connections > n                    { return false; } }
        if let Some(ref s) = self.map_display        { if r.map_display != *s                     { return false; } }
        if let Some(ref s) = self.public_date        { if r.public_date != *s                     { return false; } }
        if let Some(ref s) = self.public_date_contains { if !r.public_date.contains(s.as_str())  { return false; } }
        if let Some(ref s) = self.patrol             { if r.patrol != *s                          { return false; } }
        if let Some(ref s) = self.remarks            { if r.remarks != *s                         { return false; } }
        if let Some(ref s) = self.remarks_contains   { if !r.remarks.contains(s.as_str())        { return false; } }
        true
    }
}

/// Filter for logged-in user list.  All fields are `None` by default (= no filter).
#[derive(Debug, Clone, Default)]
pub struct UserFilter {
    // ---- order ----
    /// Exact order match
    pub order:            Option<u32>,
    /// order >= this value
    pub order_min:        Option<u32>,
    /// order <= this value
    pub order_max:        Option<u32>,

    // ---- username ----
    /// Exact match
    pub username:         Option<String>,
    /// Contains substring
    pub username_contains: Option<String>,

    // ---- room ----
    /// Exact room number match
    pub room:             Option<u32>,
    /// room >= this value
    pub room_min:         Option<u32>,
    /// room <= this value
    pub room_max:         Option<u32>,

    // ---- state ----
    /// Exact state match
    pub state:            Option<u32>,
}

impl UserFilter {
    pub fn matches(&self, u: &LoggedInUser) -> bool {
        if let Some(n)     = self.order              { if u.order != n                             { return false; } }
        if let Some(n)     = self.order_min          { if u.order < n                              { return false; } }
        if let Some(n)     = self.order_max          { if u.order > n                              { return false; } }
        if let Some(ref s) = self.username           { if u.username != *s                        { return false; } }
        if let Some(ref s) = self.username_contains  { if !u.username.contains(s.as_str())       { return false; } }
        if let Some(n)     = self.room               { if u.room != n                             { return false; } }
        if let Some(n)     = self.room_min           { if u.room < n                              { return false; } }
        if let Some(n)     = self.room_max           { if u.room > n                              { return false; } }
        if let Some(n)     = self.state              { if u.state != n                            { return false; } }
        true
    }
}

/// Scraping options: filters applied after fetching
#[derive(Debug, Clone, Default)]
pub struct ScrapeOptions {
    pub room_filter: Option<RoomFilter>,
    pub user_filter: Option<UserFilter>,
}

impl ScrapeOptions {
    pub fn with_room_filter(mut self, f: RoomFilter) -> Self {
        self.room_filter = Some(f);
        self
    }
    pub fn with_user_filter(mut self, f: UserFilter) -> Self {
        self.user_filter = Some(f);
        self
    }
}

// ----------------------------------------------------------------
// Proxy mode (internal)
// ----------------------------------------------------------------

enum ProxyMode {
    Auto,
    Direct,
    Manual(String),  // proxy URI string e.g. "http://host:8080"
}

// ----------------------------------------------------------------
// ProxyConnector: always connects to a fixed proxy address
// ----------------------------------------------------------------

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
            .parse().unwrap();
        self.inner.call(proxy_uri)
    }
}

// ----------------------------------------------------------------
// HTTP layer
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
                    .header("User-Agent", "Mozilla/5.0 (compatible; RustScraper/1.0)");
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
                    .header("User-Agent", "Mozilla/5.0 (compatible; RustScraper/1.0)");
                client.request(build_req!(b)).await?
            }
        }

        ProxyMode::Direct => {
            let mut conn = HttpConnector::new();
            conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new())
                .build::<_, Empty<Bytes>>(conn);
            let b = Request::builder().method("GET").uri(url)
                .header("User-Agent", "Mozilla/5.0 (compatible; RustScraper/1.0)");
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
                .header("User-Agent", "Mozilla/5.0 (compatible; RustScraper/1.0)");
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
// tl helpers
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

fn to_html<'a>(node: &tl::Node<'a>, parser: &'a tl::Parser<'a>) -> String {
    node.outer_html(parser).to_string()
}

fn children_html<'a>(tag: &tl::HTMLTag<'a>, parser: &'a tl::Parser<'a>) -> String {
    tag.children().top().iter()
        .filter_map(|h| h.get(parser))
        .map(|n| to_html(n, parser))
        .collect()
}

/// 半角・全角スペース、ノーブレークスペース等を除去
fn trim_full(s: String) -> String {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{3000}' || c == '\u{00a0}')
     .to_string()
}

fn parse_tr_cells(tr_html: &str) -> Vec<String> {
    let dom = match tl::parse(tr_html, tl::ParserOptions::default()) {
        Ok(d) => d, Err(_) => return vec![],
    };
    let p = dom.parser();
    dom.query_selector("td").into_iter().flatten()
        .filter_map(|h| h.get(p))
        .map(|n| inner_text(n, p).trim().to_string())
        .collect()
}

// ----------------------------------------------------------------
// HTML parsers
// ----------------------------------------------------------------

fn parse_logged_in_users_html(dom: &tl::VDom) -> Option<Vec<LoggedInUser>> {
    let parser = dom.parser();
    let node   = dom
        .query_selector(r#"td[valign="top"]"#)?
        .next()?.get(parser)?;

    let td_html = match node {
        tl::Node::Tag(tag) => children_html(tag, parser),
        _ => return None,
    };

    const NO_USERS: &str =
        "\u{30ed}\u{30b0}\u{30a4}\u{30f3}\u{4e2d}\u{306e}\
         \u{30e6}\u{30fc}\u{30b6}\u{30fc}\u{306f}\
         \u{3044}\u{307e}\u{305b}\u{3093}";

    if td_html.contains(NO_USERS) { return None; }

    let dom2 = tl::parse(&td_html, tl::ParserOptions::default()).ok()?;
    let p2   = dom2.parser();

    let mut users     = Vec::new();
    let mut is_header = true;

    for tr_handle in dom2.query_selector("tr").into_iter().flatten() {
        let tr_html = match tr_handle.get(p2) {
            Some(tl::Node::Tag(tag)) => children_html(tag, p2),
            _ => continue,
        };
        let cells = parse_tr_cells(&tr_html);
        if cells.len() < 4 { continue; }
        if is_header { is_header = false; continue; }

        let order = match cells[0].parse::<u32>() { Ok(n) => n, Err(_) => continue };
        let room  = match cells[2].parse::<u32>() { Ok(n) => n, Err(_) => continue };
        let state = cells[3].parse::<u32>().unwrap_or(0);

        users.push(LoggedInUser { order, username: cells[1].clone(), room, state });
    }

    if users.is_empty() { None } else { Some(users) }
}

fn parse_rooms_html(dom: &tl::VDom) -> Vec<RoomInfo> {
    let parser = dom.parser();

    let center_html = match dom
        .query_selector(r#"td[align="center"]"#)
        .and_then(|mut q| q.next())
        .and_then(|h| h.get(parser))
    {
        Some(tl::Node::Tag(tag)) => children_html(tag, parser),
        _ => return vec![],
    };

    let dom2 = match tl::parse(&center_html, tl::ParserOptions::default()) {
        Ok(d) => d, Err(_) => return vec![],
    };
    let p2 = dom2.parser();

    let mut rooms     = Vec::new();
    let mut is_header = true;

    for tr_handle in dom2.query_selector("tr").into_iter().flatten() {
        let tr_html = match tr_handle.get(p2) {
            Some(tl::Node::Tag(tag)) => children_html(tag, p2),
            _ => continue,
        };
        let cells = parse_tr_cells(&tr_html);
        if cells.len() < 6 { continue; }
        if is_header { is_header = false; continue; }

        let room     = match cells[0].parse::<u32>() { Ok(n) => n, Err(_) => continue };
        let max_conn = cells[1].parse::<u32>().unwrap_or(0);

        rooms.push(RoomInfo {
            room,
            max_connections: max_conn,
            map_display:     trim_full(cells[2].clone()),
            public_date:     trim_full(cells[3].clone()),
            patrol:          trim_full(cells[4].clone()),
            remarks:         trim_full(cells[5].clone()),
        });
    }
    rooms
}

// ----------------------------------------------------------------
// Core scrape logic (shared)
// ----------------------------------------------------------------

const BASE_URL:  &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/MeetingPlace";
const CHECK_URL: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/MeetingPlace/UserCheck";

async fn scrape_inner(
    user:       &str,
    pass:       &str,
    opts:       ScrapeOptions,
    proxy_mode: ProxyMode,
) -> Result<ScrapeResult, Box<dyn std::error::Error + Send + Sync>> {
    // Step 1: get JSESSIONID
    let (_, jsession) = send_follow_redirects(BASE_URL, &[], &proxy_mode).await?;
    let jsessionid = jsession.ok_or("JSESSIONID not found")?;

    // Step 2: fetch UserCheck with credentials + session cookie
    let check_url = format!("{}?user={}&pass={}", CHECK_URL, user, pass);
    let cookie    = format!("JSESSIONID={}", jsessionid);
    let (body, _) = send_follow_redirects(
        &check_url, &[("Cookie", cookie)], &proxy_mode,
    ).await?;

    // Step 3: decode Shift-JIS and parse HTML
    let (html, _, _) = SHIFT_JIS.decode(&body);
    let dom = tl::parse(&html, tl::ParserOptions::default())?;

    let logged_in_users = parse_logged_in_users_html(&dom).map(|users| {
        match &opts.user_filter {
            Some(f) => users.into_iter().filter(|u| f.matches(u)).collect(),
            None    => users,
        }
    });

    let rooms = {
        let all = parse_rooms_html(&dom);
        match &opts.room_filter {
            Some(f) => all.into_iter().filter(|r| f.matches(r)).collect(),
            None    => all,
        }
    };

    Ok(ScrapeResult { logged_in_users, rooms })
}

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Scrape with automatic proxy detection.
///
/// Proxy detection order:
///   1. `HTTP_PROXY` / `HTTPS_PROXY` environment variables
///   2. Windows registry (IE/system settings)  ← Windows only
///   3. macOS System Configuration framework    ← macOS only
///   4. Direct connection (no proxy found)
///
/// On Android, only environment variables are checked.
///
/// # Arguments
/// * `user` - login username
/// * `pass` - login password
/// * `opts` - optional filters for rooms and users
pub async fn scrape(
    user: &str,
    pass: &str,
    opts: ScrapeOptions,
) -> Result<ScrapeResult, Box<dyn std::error::Error + Send + Sync>> {
    scrape_inner(user, pass, opts, ProxyMode::Auto).await
}

/// Scrape with a manually specified proxy.
///
/// Useful for Android or any environment where automatic detection
/// is unavailable.  Pass an empty string to force direct connection.
///
/// # Arguments
/// * `user`      - login username
/// * `pass`      - login password
/// * `proxy_uri` - proxy URI, e.g. `"http://192.168.1.1:8080"`.
///                 Pass `""` for direct connection.
/// * `opts`      - optional filters for rooms and users
pub async fn scrape_with_proxy(
    user:      &str,
    pass:      &str,
    proxy_uri: &str,
    opts:      ScrapeOptions,
) -> Result<ScrapeResult, Box<dyn std::error::Error + Send + Sync>> {
    let mode = if proxy_uri.is_empty() {
        // empty string → direct connection (skip proxy entirely)
        ProxyMode::Direct
    } else {
        ProxyMode::Manual(proxy_uri.to_string())
    };
    scrape_inner(user, pass, opts, mode).await
}