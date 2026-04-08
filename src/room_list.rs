//! CHaser Online MeetingPlace scraper library
//!
//! # Quick start
//!
//! ```no_run
//! use chaser_util::room_list::{scrape, scrape_with_proxy, ScrapeOptions};
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

use encoding_rs::SHIFT_JIS;

use crate::proxy::{send_follow_redirects, url_encode, BoxError, ProxyMode};

// ----------------------------------------------------------------
// Public data types
// ----------------------------------------------------------------

/// One room entry from the public room table.
#[derive(Debug, Clone)]
pub struct RoomInfo {
    pub room:            u32,
    pub max_connections: u32,
    pub map_display:     String,
    pub public_date:     String,
    pub patrol:          String,
    pub remarks:         String,
}

/// One logged-in user row.
#[derive(Debug, Clone)]
pub struct LoggedInUser {
    pub order:    u32,
    pub username: String,
    pub room:     u32,
    pub state:    u32,
}

/// Full scrape result.
#[derive(Debug, Clone)]
pub struct ScrapeResult {
    /// `None` means no users are currently logged in.
    pub logged_in_users: Option<Vec<LoggedInUser>>,
    pub rooms:           Vec<RoomInfo>,
}

// ----------------------------------------------------------------
// Constants
// ----------------------------------------------------------------

#[allow(non_snake_case)]
pub mod MapDisplay {
    pub const ENABLED:  &str = "\u{53ef}";  // 可
    pub const DISABLED: &str = "\u{5426}";  // 否
}

#[allow(non_snake_case)]
pub mod Patrol {
    pub const YES: &str = "\u{6709}";  // 有
    pub const NO:  &str = "\u{00d7}";  // ×
}

#[allow(non_snake_case)]
pub mod Remarks {
    pub const RA:  &str = "\u{30e9}";  // ラ
    pub const SAI: &str = "\u{57fc}";  // 埼
    pub const ZEN: &str = "\u{5168}";  // 全
}

// ----------------------------------------------------------------
// Filter types
// ----------------------------------------------------------------

/// Filter for room list.  All fields default to `None` (= no filter).
#[derive(Debug, Clone, Default)]
pub struct RoomFilter {
    pub room:                 Option<u32>,
    pub room_min:             Option<u32>,
    pub room_max:             Option<u32>,
    pub min_max_conn:         Option<u32>,
    pub max_max_conn:         Option<u32>,
    pub map_display:          Option<String>,
    pub public_date:          Option<String>,
    pub public_date_contains: Option<String>,
    pub patrol:               Option<String>,
    pub remarks:              Option<String>,
    pub remarks_contains:     Option<String>,
}

impl RoomFilter {
    pub fn matches(&self, r: &RoomInfo) -> bool {
        if let Some(n)     = self.room                 { if r.room != n                          { return false; } }
        if let Some(n)     = self.room_min             { if r.room < n                           { return false; } }
        if let Some(n)     = self.room_max             { if r.room > n                           { return false; } }
        if let Some(n)     = self.min_max_conn         { if r.max_connections < n                { return false; } }
        if let Some(n)     = self.max_max_conn         { if r.max_connections > n                { return false; } }
        if let Some(ref s) = self.map_display          { if r.map_display != *s                  { return false; } }
        if let Some(ref s) = self.public_date          { if r.public_date != *s                  { return false; } }
        if let Some(ref s) = self.public_date_contains { if !r.public_date.contains(s.as_str()) { return false; } }
        if let Some(ref s) = self.patrol               { if r.patrol != *s                       { return false; } }
        if let Some(ref s) = self.remarks              { if r.remarks != *s                      { return false; } }
        if let Some(ref s) = self.remarks_contains     { if !r.remarks.contains(s.as_str())     { return false; } }
        true
    }
}

/// Filter for logged-in user list.  All fields default to `None` (= no filter).
#[derive(Debug, Clone, Default)]
pub struct UserFilter {
    pub order:             Option<u32>,
    pub order_min:         Option<u32>,
    pub order_max:         Option<u32>,
    pub username:          Option<String>,
    pub username_contains: Option<String>,
    pub room:              Option<u32>,
    pub room_min:          Option<u32>,
    pub room_max:          Option<u32>,
    pub state:             Option<u32>,
}

impl UserFilter {
    pub fn matches(&self, u: &LoggedInUser) -> bool {
        if let Some(n)     = self.order             { if u.order != n                           { return false; } }
        if let Some(n)     = self.order_min         { if u.order < n                            { return false; } }
        if let Some(n)     = self.order_max         { if u.order > n                            { return false; } }
        if let Some(ref s) = self.username          { if u.username != *s                      { return false; } }
        if let Some(ref s) = self.username_contains { if !u.username.contains(s.as_str())     { return false; } }
        if let Some(n)     = self.room              { if u.room != n                            { return false; } }
        if let Some(n)     = self.room_min          { if u.room < n                             { return false; } }
        if let Some(n)     = self.room_max          { if u.room > n                             { return false; } }
        if let Some(n)     = self.state             { if u.state != n                           { return false; } }
        true
    }
}

/// Scraping options: filters applied after fetching.
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

fn trim_full(s: String) -> String {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{3000}' || c == '\u{00a0}')
     .to_string()
}

fn parse_tr_cells(tr_html: &str) -> Vec<String> {
    let dom = match tl::parse(tr_html, tl::ParserOptions::default()) {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let p = dom.parser();
    dom.query_selector("td")
        .into_iter()
        .flatten()
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
        .next()?
        .get(parser)?;

    let td_html = match node {
        tl::Node::Tag(tag) => children_html(tag, parser),
        _ => return None,
    };

    // "ログイン中のユーザーはいません"
    const NO_USERS: &str =
        "\u{30ed}\u{30b0}\u{30a4}\u{30f3}\u{4e2d}\u{306e}\
         \u{30e6}\u{30fc}\u{30b6}\u{30fc}\u{306f}\
         \u{3044}\u{307e}\u{305b}\u{3093}";

    if td_html.contains(NO_USERS) {
        return None;
    }

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
        if cells.len() < 4 {
            continue;
        }
        if is_header {
            is_header = false;
            continue;
        }

        let order = match cells[0].parse::<u32>() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let room  = match cells[2].parse::<u32>() {
            Ok(n) => n,
            Err(_) => continue,
        };
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
        Ok(d)  => d,
        Err(_) => return vec![],
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
        if cells.len() < 6 {
            continue;
        }
        if is_header {
            is_header = false;
            continue;
        }

        let room = match cells[0].parse::<u32>() {
            Ok(n) => n,
            Err(_) => continue,
        };
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
// URLs
// ----------------------------------------------------------------

const BASE_URL:  &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/MeetingPlace";
const CHECK_URL: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/MeetingPlace/UserCheck";

// ----------------------------------------------------------------
// Core scrape logic
// ----------------------------------------------------------------

async fn scrape_inner(
    user:       &str,
    pass:       &str,
    opts:       ScrapeOptions,
    proxy_mode: ProxyMode,
) -> Result<ScrapeResult, BoxError> {
    // Step 1: Fetch the top page to obtain JSESSIONID
    let (_, jsession) = send_follow_redirects(BASE_URL, &[], &proxy_mode).await?;
    let jsessionid = jsession.ok_or("JSESSIONID not found")?;

    // Step 2: Authenticate
    // FIX: user and pass are percent-encoded to prevent query parameter injection.
    let check_url = format!(
        "{}?user={}&pass={}",
        CHECK_URL,
        url_encode(user),
        url_encode(pass),
    );
    let cookie    = format!("JSESSIONID={}", jsessionid);
    let (body, _) = send_follow_redirects(
        &check_url,
        &[("Cookie", cookie)],
        &proxy_mode,
    ).await?;

    let (html, _, _) = SHIFT_JIS.decode(&body);
    let dom = tl::parse(&html, tl::ParserOptions::default())?;

    let logged_in_users = parse_logged_in_users_html(&dom).and_then(|users| {
        let filtered: Vec<LoggedInUser> = match &opts.user_filter {
            Some(f) => users.into_iter().filter(|u| f.matches(u)).collect(),
            None    => users,
        };
        if filtered.is_empty() { None } else { Some(filtered) }
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
pub async fn scrape(
    user: &str,
    pass: &str,
    opts: ScrapeOptions,
) -> Result<ScrapeResult, BoxError> {
    scrape_inner(user, pass, opts, ProxyMode::Auto).await
}

/// Scrape with a manually specified proxy.
/// Pass `""` for direct connection.
pub async fn scrape_with_proxy(
    user:      &str,
    pass:      &str,
    proxy_uri: &str,
    opts:      ScrapeOptions,
) -> Result<ScrapeResult, BoxError> {
    scrape_inner(user, pass, opts, ProxyMode::from_option(Some(proxy_uri))).await
}