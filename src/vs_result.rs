//! CHaser Online battle result scraper.
//!
//! # Quick start
//!
//! ```no_run
//! use chaser_util::vs_result::{fetch_vs_result, VsResultQuery};
//!
//! #[tokio::main]
//! async fn main() {
//!     let query = VsResultQuery::today();   // FIX: was a hardcoded date
//!     let results = fetch_vs_result("cool", "cool", query, None).await.unwrap();
//!     for r in &results {
//!         println!("room={} {} vs {} -> {} : {}",
//!             r.room,
//!             r.players[0].username, r.players[1].username,
//!             r.players[0].total_point, r.players[1].total_point,
//!         );
//!     }
//! }
//! ```

use encoding_rs::SHIFT_JIS;

use crate::proxy::{send_follow_redirects, send_once, url_encode, BoxError, ProxyMode};

// ----------------------------------------------------------------
// Public data types
// ----------------------------------------------------------------

/// One player's result within a battle.
#[derive(Debug, Clone, Default)]
pub struct PlayerResult {
    pub order:        u32,
    pub username:     String,
    pub get_turn:     i32,
    pub rem_turn:     i32,
    pub total_point:  i32,
    pub action_point: Option<i32>,
    pub item_point:   Option<i32>,
    pub put_point:    Option<i32>,
    pub put_damage:   Option<i32>,
}

/// One battle result (one room, one session).
#[derive(Debug, Clone)]
pub struct BattleResult {
    pub room:       u32,
    pub start_time: String,
    pub end_time:   String,
    pub players:    Vec<PlayerResult>,
}

/// Query parameters for listvsresult.
#[derive(Debug, Clone)]
pub struct VsResultQuery {
    pub min_start_date:   String,
    pub min_end_date:     String,
    pub min_start_time:   String,
    pub min_end_time:     String,
    pub min_room:         u32,
    pub min_turn:         u32,
    pub max_start_date:   String,
    pub max_end_date:     String,
    pub max_start_time:   String,
    pub max_end_time:     String,
    pub max_room:         u32,
    pub max_turn:         u32,
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

impl VsResultQuery {
    /// Create a query covering today's results (date computed at runtime).
    ///
    /// FIX: Previously `Default::default()` used a hardcoded date string
    /// ("2026-04-07") that needed manual updating every day.  This method
    /// calls `chrono::Local::now()` so it is always correct.
    pub fn today() -> Self {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        Self::for_date(&today)
    }

    /// Create a query covering a specific date (format: "YYYY-MM-DD").
    pub fn for_date(date: &str) -> Self {
        Self {
            min_start_date:   date.to_string(),
            min_end_date:     date.to_string(),
            min_start_time:   "00:00:00".to_string(),
            min_end_time:     "00:00:00".to_string(),
            min_room:         1,
            min_turn:         1,
            max_start_date:   date.to_string(),
            max_end_date:     date.to_string(),
            max_start_time:   "23:59:59".to_string(),
            max_end_time:     "23:59:59".to_string(),
            max_room:         10000,
            max_turn:         8,
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

    /// Encode into a query string for listvsresult.
    /// Time values are percent-encoded via `url_encode` (replaces the old
    /// hand-rolled `encode()` that only escaped `:` and ` `).
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
            url_encode(&self.min_start_date), url_encode(&self.min_end_date),
            url_encode(&self.min_start_time), url_encode(&self.min_end_time),
            self.min_room, self.min_turn,
            url_encode(&self.max_start_date), url_encode(&self.max_end_date),
            url_encode(&self.max_start_time), url_encode(&self.max_end_time),
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

/// `Default` now delegates to `VsResultQuery::today()` instead of using a
/// hardcoded date, so existing code calling `VsResultQuery::default()` still
/// compiles and works correctly.
impl Default for VsResultQuery {
    fn default() -> Self {
        Self::today()
    }
}

// ----------------------------------------------------------------
// HTML parsing helpers
// ----------------------------------------------------------------

fn cell_text(s: &str) -> String {
    s.trim()
     .replace('\u{3000}', "")
     .replace('\u{00a0}', "")
     .trim()
     .to_string()
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

fn parse_results(html: &str) -> Vec<BattleResult> {
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(d)  => d,
        Err(_) => return vec![],
    };
    let parser = dom.parser();

    let table = match dom
        .query_selector(r#"table[border="1"]"#)
        .and_then(|mut q| q.next())
        .and_then(|h| h.get(parser))
    {
        Some(tl::Node::Tag(t)) => t,
        _ => return vec![],
    };

    let tr_htmls: Vec<String> = table
        .children().top().iter()
        .filter_map(|h| h.get(parser))
        .filter_map(|n| match n {
            tl::Node::Tag(t) if t.name().as_bytes().eq_ignore_ascii_case(b"tbody") => {
                Some(
                    t.children().top().iter()
                        .filter_map(|h| h.get(parser))
                        .filter_map(|n| match n {
                            tl::Node::Tag(tr)
                                if tr.name().as_bytes().eq_ignore_ascii_case(b"tr") =>
                                    Some(tr.outer_html(parser).to_string()),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )
            }
            tl::Node::Tag(t) if t.name().as_bytes().eq_ignore_ascii_case(b"tr") => {
                Some(vec![t.outer_html(parser).to_string()])
            }
            _ => None,
        })
        .flatten()
        .collect();

    let mut results: Vec<BattleResult> = Vec::new();
    let mut skip_header = true;

    for tr_html in &tr_htmls {
        let tr_dom = match tl::parse(tr_html, tl::ParserOptions::default()) {
            Ok(d)  => d,
            Err(_) => continue,
        };
        let tp = tr_dom.parser();

        let cells: Vec<String> = tr_dom
            .query_selector("td").into_iter().flatten()
            .filter_map(|h| h.get(tp))
            .map(|n| {
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

        if cells.len() < 12 {
            continue;
        }

        if skip_header {
            skip_header = false;
            continue;
        }

        let start_time_text = cell_text(&cells[1]);
        let is_new_battle   = !start_time_text.is_empty();

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
            // FIX: room number is inherited from the previous *battle* (not just
            // the previous row) when it is absent, so grouping within the same
            // table section works correctly.
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
        } else if let Some(last) = results.last_mut() {
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

    results
}

// ----------------------------------------------------------------
// URLs
// ----------------------------------------------------------------

const SERVER_BASE_URL:  &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/";
const SERVER_CHECK_URL: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/UserCheck";
const LIST_RESULT_URL:  &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/listvsresult";

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Fetches and parses the battle result list.
///
/// # Arguments
/// * `user`      - Login username
/// * `pass`      - Login password
/// * `query`     - Search parameters (`VsResultQuery::today()` for today's results)
/// * `proxy_uri` - Optional proxy URI; `None` = auto-detect, `Some("")` = direct
pub async fn fetch_vs_result(
    user:      &str,
    pass:      &str,
    query:     VsResultQuery,
    proxy_uri: Option<&str>,
) -> Result<Vec<BattleResult>, BoxError> {
    let proxy_mode = ProxyMode::from_option(proxy_uri);

    // Step 1: Obtain JSESSIONID from Server base page.
    let (_, jsession) =
        send_follow_redirects(SERVER_BASE_URL, &[], &proxy_mode).await?;
    let jsessionid = jsession.ok_or("JSESSIONID not found")?;
    let cookie = format!("JSESSIONID={}", jsessionid);

    // Step 2: Authenticate.
    // FIX: user and pass are percent-encoded to prevent query parameter injection.
    let check_url = format!(
        "{}?user={}&pass={}&select=vsresultview",
        SERVER_CHECK_URL,
        url_encode(user),
        url_encode(pass),
    );
    let _ = send_once(&check_url, &[("Cookie", cookie.clone())], &proxy_mode).await?;

    // Step 3: Fetch result list.
    let list_url = format!("{}?{}", LIST_RESULT_URL, query.to_query_string());
    let (_, _, body) = send_once(&list_url, &[("Cookie", cookie)], &proxy_mode).await?;

    // Step 4: Decode Shift-JIS and parse.
    let (html, _, _) = SHIFT_JIS.decode(&body);
    Ok(parse_results(&html))
}