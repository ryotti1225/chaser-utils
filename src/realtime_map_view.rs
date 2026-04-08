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

use encoding_rs::SHIFT_JIS;
use regex::Regex;
use std::sync::OnceLock;

use crate::proxy::{send_follow_redirects, url_encode, BoxError, ProxyMode};

// ----------------------------------------------------------------
// Public data types
// ----------------------------------------------------------------

/// One map cell (numeric part of the image filename).
/// e.g. "012.gif" → 12, "000.gif" → 0
pub type TileId = u32;

const SERVER_IMAGE_BASE: &str = "http://www7019ug.sakura.ne.jp/CHaserOnline003/img/";

/// Returns the full image URL for a given TileId.
pub fn tile_image_url(tile: TileId) -> String {
    format!("{}{:03}.gif", SERVER_IMAGE_BASE, tile)
}

/// Player information (from the right-side table).
#[derive(Debug, Clone)]
pub struct PlayerInfo {
    pub username: String,
    pub attr_a:   i32,
    pub attr_i:   i32,
    pub attr_p:   i32,
    pub attr_pd:  i32,
    pub attr_t:   i32,
    /// Command list (one entry per command line, e.g. "gr 12,0,12").
    pub commands: Vec<String>,
}

/// Full result of a map view fetch.
#[derive(Debug, Clone)]
pub struct MapViewResult {
    pub room_name:   String,
    pub turn:        u32,
    pub next_player: String,
    /// Map cells as a 2D array \[row\]\[col\].
    pub map:         Vec<Vec<TileId>>,
    pub players:     Vec<PlayerInfo>,
}

/// Fetch options.
#[derive(Debug, Clone, Default)]
pub struct MapViewOptions {
    /// Optional proxy URI.  `None` = auto-detect, `Some("")` = direct connection.
    pub proxy_uri: Option<String>,
}

// ----------------------------------------------------------------
// Lazy-compiled regexes
// ----------------------------------------------------------------

fn re_img_src() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"/img/(\d{1,3})\.gif").expect("re_img_src"))
}

fn re_turn() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"turn=(\d+)\s+Next=([^<\s]+)").expect("re_turn"))
}

fn re_room_name() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)<h1[^>]*>[^\[]*\[([^\]]+)\]").expect("re_room_name"))
}

// ----------------------------------------------------------------
// HTML parsing helpers
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

pub(crate) fn parse_room_name(html: &str) -> String {
    re_room_name()
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default()
}

pub(crate) fn parse_map(html: &str) -> (Vec<Vec<TileId>>, u32, String) {
    let (turn, next_player) = re_turn()
        .captures(html)
        .map(|c| (
            c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0),
            c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default(),
        ))
        .unwrap_or((0, String::new()));

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

    let tr_positions: Vec<usize> = table_lower
        .match_indices("<tr")
        .map(|(i, _)| i)
        .collect();

    let mut map: Vec<Vec<TileId>> = Vec::new();
    for (idx, &tr_start) in tr_positions.iter().enumerate() {
        let tr_end   = tr_positions.get(idx + 1).copied().unwrap_or(table_html.len());
        let tr_slice = &table_html[tr_start..tr_end];

        let row: Vec<TileId> = re_img_src()
            .captures_iter(tr_slice)
            .filter_map(|c| c.get(1).and_then(|m| {
                let id: u32 = m.as_str().parse().ok()?;
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

pub(crate) fn parse_players(dom: &tl::VDom) -> Vec<PlayerInfo> {
    let parser = dom.parser();

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

    let mut player_data: Vec<(String, i32, i32, i32, i32, i32)> = Vec::new();
    if let Some(tr0) = tr_htmls.first() {
        if let Ok(d) = tl::parse(tr0, tl::ParserOptions::default()) {
            let p = d.parser();
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

    let mut players = Vec::new();
    for (i, (username, attr_a, attr_i, attr_p, attr_pd, attr_t)) in player_data.into_iter().enumerate() {
        let commands = all_commands.get(i).cloned().unwrap_or_default();
        players.push(PlayerInfo { username, attr_a, attr_i, attr_p, attr_pd, attr_t, commands });
    }
    players
}

// ----------------------------------------------------------------
// URLs
// ----------------------------------------------------------------

const SERVER_CHECK_URL: &str =
    "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/UserCheck";
const MAP_VIEW_URL: &str =
    "http://www7019ug.sakura.ne.jp/CHaserOnline003/Server/MapView.jsp";

// ----------------------------------------------------------------
// Core logic
// ----------------------------------------------------------------

async fn fetch_inner(
    user:       &str,
    pass:       &str,
    proxy_mode: ProxyMode,
) -> Result<MapViewResult, BoxError> {
    // FIX: user and pass are percent-encoded to prevent query parameter injection.
    let check_url = format!(
        "{}?user={}&pass={}&select=mapview",
        SERVER_CHECK_URL,
        url_encode(user),
        url_encode(pass),
    );
    let (_, jsession) = send_follow_redirects(&check_url, &[], &proxy_mode).await?;
    let jsessionid = jsession.ok_or("Server JSESSIONID not found")?;
    let cookie = format!("JSESSIONID={}", jsessionid);

    let (body, _) =
        send_follow_redirects(MAP_VIEW_URL, &[("Cookie", cookie)], &proxy_mode).await?;
    let (html, _, _) = SHIFT_JIS.decode(&body);

    let room_name                = parse_room_name(&html);
    let (map, turn, next_player) = parse_map(&html);
    let dom                      = tl::parse(&html, tl::ParserOptions::default())?;
    let players                  = parse_players(&dom);

    Ok(MapViewResult { room_name, turn, next_player, map, players })
}

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Fetches the real-time game map view.
pub async fn fetch_map_view(
    user: &str,
    pass: &str,
    opts: MapViewOptions,
) -> Result<MapViewResult, BoxError> {
    let proxy_mode = ProxyMode::from_option(opts.proxy_uri.as_deref());
    fetch_inner(user, pass, proxy_mode).await
}