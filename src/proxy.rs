//! Shared proxy / HTTP utilities.
//!
//! `ProxyMode`, `ProxyConnector`, and the low-level `get` / `post` helpers
//! are defined here once and re-exported to every scraper module.
//! This eliminates the four near-identical copies that existed before.

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::proxy::matcher::Matcher;
use hyper_util::rt::TokioExecutor;

// ----------------------------------------------------------------
// ProxyMode
// ----------------------------------------------------------------

/// How to route outbound HTTP requests.
#[derive(Debug, Clone)]
pub enum ProxyMode {
    /// Detect proxy automatically (env vars → OS settings → direct).
    Auto,
    /// Always connect directly; ignore any system proxy.
    Direct,
    /// Use the supplied proxy URI (e.g. `"http://192.168.1.1:8080"`).
    Manual(String),
}

impl ProxyMode {
    /// Build a `ProxyMode` from an `Option<&str>`:
    /// - `None`       → `Auto`
    /// - `Some("")`   → `Direct`
    /// - `Some(uri)`  → `Manual(uri)`
    pub fn from_option(opt: Option<&str>) -> Self {
        match opt {
            None => Self::Auto,
            Some(s) if s.is_empty() => Self::Direct,
            Some(s) => Self::Manual(s.to_string()),
        }
    }
}

// ----------------------------------------------------------------
// ProxyConnector
// ----------------------------------------------------------------

/// A hyper `Connector` that always dials a fixed upstream proxy host:port,
/// regardless of the target URI.  The real target URI is carried in the
/// `Host` header (HTTP/1.1 CONNECT-less plain-HTTP proxy protocol).
#[derive(Clone)]
pub struct ProxyConnector {
    inner:      HttpConnector,
    proxy_host: String,
    proxy_port: u16,
}

impl ProxyConnector {
    pub fn new(proxy_host: impl Into<String>, proxy_port: u16) -> Self {
        let mut inner = HttpConnector::new();
        inner.enforce_http(false);
        Self { inner, proxy_host: proxy_host.into(), proxy_port }
    }
}

impl tower_service::Service<http::Uri> for ProxyConnector {
    type Response = <HttpConnector as tower_service::Service<http::Uri>>::Response;
    type Error    = <HttpConnector as tower_service::Service<http::Uri>>::Error;
    type Future   = <HttpConnector as tower_service::Service<http::Uri>>::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
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
// Standard User-Agent string
// ----------------------------------------------------------------

/// Use a realistic browser UA everywhere so the target server does not
/// reject requests.  (Previously `room_list.rs` used a different string,
/// causing inconsistent behaviour.)
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/124.0.0.0 Safari/537.36";

// ----------------------------------------------------------------
// Shared error type alias
// ----------------------------------------------------------------

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

// ----------------------------------------------------------------
// send_once  (single request, no redirect following)
// ----------------------------------------------------------------

/// Send a single GET request and return `(status, headers, body)`.
///
/// `extra` is a slice of additional `(header-name, value)` pairs appended
/// to every request.
pub async fn send_once(
    url:        &str,
    extra:      &[(&'static str, String)],
    proxy_mode: &ProxyMode,
) -> Result<(u16, hyper::HeaderMap, Bytes), BoxError> {
    let target_uri: http::Uri = url.parse()?;

    // Macro avoids repeating the header-appending loop for every proxy branch.
    macro_rules! build_req {
        ($builder:expr) => {{
            let mut b = $builder;
            for (k, v) in extra {
                b = b.header(*k, v.as_str());
            }
            b.body(Empty::<Bytes>::new())?
        }};
    }

    macro_rules! base_builder {
        ($url:expr) => {
            Request::builder()
                .method("GET")
                .uri($url)
                .header("User-Agent", USER_AGENT)
        };
    }

    let resp = match proxy_mode {
        ProxyMode::Auto => {
            let matcher = Matcher::from_system();
            if let Some(intercept) = matcher.intercept(&target_uri) {
                let ph = intercept.uri().host().unwrap_or("127.0.0.1").to_string();
                let pp = intercept.uri().port_u16().unwrap_or(8080);
                let client = Client::builder(TokioExecutor::new())
                    .build::<_, Empty<Bytes>>(ProxyConnector::new(ph, pp));
                let mut b = base_builder!(url);
                if let Some(auth) = intercept.basic_auth() {
                    b = b.header("Proxy-Authorization", auth);
                }
                client.request(build_req!(b)).await?
            } else {
                let mut conn = HttpConnector::new();
                conn.enforce_http(false);
                let client = Client::builder(TokioExecutor::new())
                    .build::<_, Empty<Bytes>>(conn);
                client.request(build_req!(base_builder!(url))).await?
            }
        }

        ProxyMode::Direct => {
            let mut conn = HttpConnector::new();
            conn.enforce_http(false);
            let client = Client::builder(TokioExecutor::new())
                .build::<_, Empty<Bytes>>(conn);
            client.request(build_req!(base_builder!(url))).await?
        }

        ProxyMode::Manual(proxy_uri_str) => {
            let proxy_uri: http::Uri = proxy_uri_str.parse()?;
            let ph = proxy_uri.host().unwrap_or("127.0.0.1").to_string();
            let pp = proxy_uri.port_u16().unwrap_or(8080);
            let client = Client::builder(TokioExecutor::new())
                .build::<_, Empty<Bytes>>(ProxyConnector::new(ph, pp));
            client.request(build_req!(base_builder!(url))).await?
        }
    };

    let status  = resp.status().as_u16();
    let headers = resp.headers().clone();
    let body    = resp.into_body().collect().await?.to_bytes();
    Ok((status, headers, body))
}

// ----------------------------------------------------------------
// send_follow_redirects
// ----------------------------------------------------------------

/// Follow up to `MAX_REDIRECTS` HTTP 3xx responses automatically.
///
/// Returns `(final_body, Option<jsessionid>)`.
/// The JSESSIONID is collected from `Set-Cookie` headers on any hop.
///
/// Relative `Location` values are resolved against the current URL so that
/// servers returning `/path?foo=bar` instead of an absolute URI are handled
/// correctly.
pub async fn send_follow_redirects(
    start_url:  &str,
    extra:      &[(&'static str, String)],
    proxy_mode: &ProxyMode,
) -> Result<(Bytes, Option<String>), BoxError> {
    const MAX_REDIRECTS: usize = 10;

    let mut url      = start_url.to_string();
    let mut jsession = None::<String>;

    for _ in 0..MAX_REDIRECTS {
        let (status, headers, body) = send_once(&url, extra, proxy_mode).await?;

        // Harvest JSESSIONID from every hop
        for val in headers.get_all("set-cookie").iter() {
            for part in val.to_str().unwrap_or("").split(';') {
                if let Some(id) = part.trim().strip_prefix("JSESSIONID=") {
                    jsession = Some(id.to_string());
                }
            }
        }

        if (300..400).contains(&status) {
            if let Some(loc) = headers.get("location") {
                let loc_str = loc.to_str()?;
                // Resolve relative Location against current URL
                url = resolve_url(&url, loc_str)?;
                continue;
            }
        }

        return Ok((body, jsession));
    }

    Err("too many redirects".into())
}

// ----------------------------------------------------------------
// URL helpers
// ----------------------------------------------------------------

/// Resolve `location` (possibly relative) against `base`.
///
/// Examples:
///   resolve("http://host/a/b", "/c/d")   → "http://host/c/d"
///   resolve("http://host/a/b", "c/d")    → "http://host/a/c/d"
///   resolve("http://host/a/b", "http://other/x") → "http://other/x"
pub fn resolve_url(base: &str, location: &str) -> Result<String, BoxError> {
    // Already absolute
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }

    // Parse base to extract scheme + authority
    let base_uri: http::Uri = base.parse()?;
    let scheme    = base_uri.scheme_str().unwrap_or("http");
    let authority = base_uri.authority().map(|a| a.as_str()).unwrap_or("");

    if location.starts_with('/') {
        // Absolute path
        Ok(format!("{}://{}{}", scheme, authority, location))
    } else {
        // Relative path: resolve against base path's directory
        let base_path = base_uri.path();
        let dir = match base_path.rfind('/') {
            Some(i) => &base_path[..=i],
            None    => "/",
        };
        Ok(format!("{}://{}{}{}", scheme, authority, dir, location))
    }
}

// ----------------------------------------------------------------
// URL encoding helper
// ----------------------------------------------------------------

/// Percent-encode a string for use as a query parameter value.
/// Uses `percent-encoding` crate with `NON_ALPHANUMERIC` set (safe for all values).
pub fn url_encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC)
        .to_string()
}

// ----------------------------------------------------------------
// JSESSIONID extraction
// ----------------------------------------------------------------

/// Extract JSESSIONID from a header map's `Set-Cookie` values.
pub fn extract_jsessionid(headers: &hyper::HeaderMap) -> Option<String> {
    for val in headers.get_all("set-cookie").iter() {
        for part in val.to_str().unwrap_or("").split(';') {
            if let Some(id) = part.trim().strip_prefix("JSESSIONID=") {
                return Some(id.to_string());
            }
        }
    }
    None
}