//! C FFI layer
//!
//! Exposes the scraper API as a C-compatible interface.
//! C++ users include `chaser-util.h` and link against `chaser_util.dll` / `libchaser_util.so`.
//!
//! # Memory model
//! - All returned pointers are heap-allocated by Rust.
//! - The caller MUST free them with `scraper_free_result()`.
//! - Passing NULL for optional filter pointers means "no filter".
//!
//! # Safety
//! - `scraper_scrape` / `scraper_scrape_with_proxy` now return a result
//!   with `error_code = 2` if `user` or `pass` are NULL, rather than
//!   causing undefined behaviour.
//! - All `#[no_mangle] extern "C"` functions are wrapped in
//!   `catch_unwind` so that Rust panics cannot unwind across the FFI
//!   boundary (which would be UB).

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uint};
use std::panic;
use std::ptr;
use std::sync::OnceLock;

use crate::room_list::{
    scrape as rs_scrape, scrape_with_proxy as rs_scrape_with_proxy,
    RoomFilter, ScrapeOptions, ScrapeResult, UserFilter,
};

// ----------------------------------------------------------------
// Global Tokio runtime (reused across all FFI calls)
// ----------------------------------------------------------------

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Runtime::new()
            .unwrap_or_else(|e| panic!("failed to create Tokio runtime: {}", e))
    })
}

// ----------------------------------------------------------------
// C-compatible structs
// ----------------------------------------------------------------

#[repr(C)]
pub struct CRoomInfo {
    pub room:            c_uint,
    pub max_connections: c_uint,
    pub map_display:     *mut c_char,
    pub public_date:     *mut c_char,
    pub patrol:          *mut c_char,
    pub remarks:         *mut c_char,
}

#[repr(C)]
pub struct CLoggedInUser {
    pub order:    c_uint,
    pub username: *mut c_char,
    pub room:     c_uint,
    pub state:    c_uint,
}

/// C-compatible scrape result.
///
/// `rooms_cap` / `users_cap` store the actual Vec capacity so that
/// `scraper_free_result` can reconstruct the Vec correctly.
///
/// FIX: these fields were previously `pub`, which allowed C/C++ code to
/// accidentally modify them.  They are now private to Rust; the C header
/// still documents them as "internal – do not modify".
#[repr(C)]
pub struct CScrapeResult {
    pub rooms:      *mut CRoomInfo,
    pub rooms_len:  usize,
    rooms_cap:      usize,          // internal — do not touch from C/C++
    pub users:      *mut CLoggedInUser,
    pub users_len:  usize,
    users_cap:      usize,          // internal — do not touch from C/C++
    /// 0 = success, non-zero = error (call `scraper_last_error()` for message)
    pub error_code: c_uint,
}

#[repr(C)]
pub struct CRoomFilter {
    pub room_enabled:         c_uint,
    pub room:                 c_uint,
    pub room_min_enabled:     c_uint,
    pub room_min:             c_uint,
    pub room_max_enabled:     c_uint,
    pub room_max:             c_uint,
    pub min_max_conn_enabled: c_uint,
    pub min_max_conn:         c_uint,
    pub max_max_conn_enabled: c_uint,
    pub max_max_conn:         c_uint,
    pub map_display:          *const c_char,
    pub public_date:          *const c_char,
    pub public_date_contains: *const c_char,
    pub patrol:               *const c_char,
    pub remarks:              *const c_char,
    pub remarks_contains:     *const c_char,
}

#[repr(C)]
pub struct CUserFilter {
    pub order_enabled:     c_uint,
    pub order:             c_uint,
    pub order_min_enabled: c_uint,
    pub order_min:         c_uint,
    pub order_max_enabled: c_uint,
    pub order_max:         c_uint,
    pub username:          *const c_char,
    pub username_contains: *const c_char,
    pub room_enabled:      c_uint,
    pub room:              c_uint,
    pub room_min_enabled:  c_uint,
    pub room_min:          c_uint,
    pub room_max_enabled:  c_uint,
    pub room_max:          c_uint,
    pub state_enabled:     c_uint,
    pub state:             c_uint,
}

// ----------------------------------------------------------------
// Thread-local error message storage
// ----------------------------------------------------------------

thread_local! {
    static LAST_ERROR: std::cell::RefCell<CString> =
        std::cell::RefCell::new(CString::new("").unwrap_or_default());
}

fn set_last_error(msg: &str) {
    let c = CString::new(msg.replace('\0', "?")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
}

/// Returns a pointer to the last error message string (UTF-8).
/// The pointer is valid until the next FFI call on this thread.
#[no_mangle]
pub extern "C" fn scraper_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

// ----------------------------------------------------------------
// Error result constructors
// ----------------------------------------------------------------

/// Build an error `CScrapeResult` with the given error code and message.
fn error_result(code: c_uint, msg: &str) -> *mut CScrapeResult {
    set_last_error(msg);
    Box::into_raw(Box::new(CScrapeResult {
        rooms: ptr::null_mut(), rooms_len: 0, rooms_cap: 0,
        users: ptr::null_mut(), users_len: 0, users_cap: 0,
        error_code: code,
    }))
}

// ----------------------------------------------------------------
// Conversion helpers
// ----------------------------------------------------------------

fn cstr_opt(p: *const c_char) -> Option<String> {
    if p.is_null() { return None; }
    unsafe { CStr::from_ptr(p).to_str().ok().map(|s| s.to_string()) }
}

fn to_cstring(s: &str) -> *mut c_char {
    CString::new(s.replace('\0', "?"))
        .map(|c| c.into_raw())
        .unwrap_or(ptr::null_mut())
}

fn enabled(flag: c_uint, val: u32) -> Option<u32> {
    if flag != 0 { Some(val) } else { None }
}

fn c_room_filter(f: &CRoomFilter) -> RoomFilter {
    RoomFilter {
        room:                 enabled(f.room_enabled,         f.room),
        room_min:             enabled(f.room_min_enabled,     f.room_min),
        room_max:             enabled(f.room_max_enabled,     f.room_max),
        min_max_conn:         enabled(f.min_max_conn_enabled, f.min_max_conn),
        max_max_conn:         enabled(f.max_max_conn_enabled, f.max_max_conn),
        map_display:          cstr_opt(f.map_display),
        public_date:          cstr_opt(f.public_date),
        public_date_contains: cstr_opt(f.public_date_contains),
        patrol:               cstr_opt(f.patrol),
        remarks:              cstr_opt(f.remarks),
        remarks_contains:     cstr_opt(f.remarks_contains),
    }
}

fn c_user_filter(f: &CUserFilter) -> UserFilter {
    UserFilter {
        order:             enabled(f.order_enabled,     f.order),
        order_min:         enabled(f.order_min_enabled, f.order_min),
        order_max:         enabled(f.order_max_enabled, f.order_max),
        username:          cstr_opt(f.username),
        username_contains: cstr_opt(f.username_contains),
        room:              enabled(f.room_enabled,      f.room),
        room_min:          enabled(f.room_min_enabled,  f.room_min),
        room_max:          enabled(f.room_max_enabled,  f.room_max),
        state:             enabled(f.state_enabled,     f.state),
    }
}

fn build_opts(
    room_filter: *const CRoomFilter,
    user_filter: *const CUserFilter,
) -> ScrapeOptions {
    let mut opts = ScrapeOptions::default();
    if !room_filter.is_null() {
        opts = opts.with_room_filter(c_room_filter(unsafe { &*room_filter }));
    }
    if !user_filter.is_null() {
        opts = opts.with_user_filter(c_user_filter(unsafe { &*user_filter }));
    }
    opts
}

// ----------------------------------------------------------------
// Convert ScrapeResult → *mut CScrapeResult
// ----------------------------------------------------------------

fn result_to_c(
    res: Result<ScrapeResult, Box<dyn std::error::Error + Send + Sync>>,
) -> *mut CScrapeResult {
    match res {
        Err(e) => error_result(1, &e.to_string()),
        Ok(sr) => {
            // rooms
            let mut c_rooms: Vec<CRoomInfo> = sr.rooms.iter().map(|r| CRoomInfo {
                room:            r.room,
                max_connections: r.max_connections,
                map_display:     to_cstring(&r.map_display),
                public_date:     to_cstring(&r.public_date),
                patrol:          to_cstring(&r.patrol),
                remarks:         to_cstring(&r.remarks),
            }).collect();
            let rooms_len = c_rooms.len();
            let rooms_cap = c_rooms.capacity();
            let rooms_ptr = if rooms_len > 0 {
                let p = c_rooms.as_mut_ptr();
                std::mem::forget(c_rooms);
                p
            } else {
                ptr::null_mut()
            };

            // users
            let (users_ptr, users_len, users_cap) = match sr.logged_in_users {
                None => (ptr::null_mut(), 0, 0),
                Some(users) => {
                    let mut c_users: Vec<CLoggedInUser> = users.iter().map(|u| CLoggedInUser {
                        order:    u.order,
                        username: to_cstring(&u.username),
                        room:     u.room,
                        state:    u.state,
                    }).collect();
                    let len = c_users.len();
                    let cap = c_users.capacity();
                    let p   = c_users.as_mut_ptr();
                    std::mem::forget(c_users);
                    (p, len, cap)
                }
            };

            Box::into_raw(Box::new(CScrapeResult {
                rooms: rooms_ptr, rooms_len, rooms_cap,
                users: users_ptr, users_len, users_cap,
                error_code: 0,
            }))
        }
    }
}

// ----------------------------------------------------------------
// NULL-safe CStr helper
// ----------------------------------------------------------------

/// Convert a (possibly NULL) `*const c_char` to a `&str`.
///
/// FIX: previously the code called `CStr::from_ptr()` on `user`/`pass`
/// without checking for NULL, which is undefined behaviour.  This helper
/// returns `Err` when the pointer is NULL so the FFI function can return a
/// proper error result to the caller instead of crashing.
unsafe fn cstr_required<'a>(
    p:    *const c_char,
    name: &'static str,
) -> Result<&'a str, String> {
    if p.is_null() {
        return Err(format!("argument `{}` must not be NULL", name));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|e| format!("argument `{}` is not valid UTF-8: {}", name, e))
}

// ----------------------------------------------------------------
// Public C API
// ----------------------------------------------------------------

/// Scrape with automatic proxy detection.
///
/// Returns a heap-allocated `CScrapeResult` that **must** be freed with
/// `scraper_free_result()`.
///
/// FIX 1: `user` and `pass` are validated for NULL before use.
/// FIX 2: the function body is wrapped in `catch_unwind` so that Rust panics
///         cannot unwind across the FFI boundary (which would be UB).
#[no_mangle]
pub extern "C" fn scraper_scrape(
    user:        *const c_char,
    pass:        *const c_char,
    room_filter: *const CRoomFilter,
    user_filter: *const CUserFilter,
) -> *mut CScrapeResult {
    // FIX: catch_unwind prevents panic-unwind UB across the FFI boundary.
    let outcome = panic::catch_unwind(|| {
        // FIX: NULL-check user and pass before dereferencing.
        let user = unsafe { cstr_required(user, "user") };
        let pass = unsafe { cstr_required(pass, "pass") };
        let (user, pass) = match (user, pass) {
            (Ok(u), Ok(p)) => (u, p),
            (Err(e), _) | (_, Err(e)) => return error_result(2, &e),
        };
        let opts = build_opts(room_filter, user_filter);
        let res  = runtime().block_on(rs_scrape(user, pass, opts));
        result_to_c(res)
    });

    match outcome {
        Ok(ptr) => ptr,
        Err(_)  => error_result(3, "internal panic in scraper_scrape"),
    }
}

/// Scrape with a manually specified proxy.
///
/// Pass `proxy_uri = ""` for direct connection.
///
/// FIX 1: `user`, `pass`, and `proxy_uri` are validated for NULL before use.
/// FIX 2: wrapped in `catch_unwind` (see `scraper_scrape`).
#[no_mangle]
pub extern "C" fn scraper_scrape_with_proxy(
    user:        *const c_char,
    pass:        *const c_char,
    proxy_uri:   *const c_char,
    room_filter: *const CRoomFilter,
    user_filter: *const CUserFilter,
) -> *mut CScrapeResult {
    let outcome = panic::catch_unwind(|| {
        let user      = unsafe { cstr_required(user,      "user")      };
        let pass      = unsafe { cstr_required(pass,      "pass")      };
        let proxy_uri = unsafe { cstr_required(proxy_uri, "proxy_uri") };
        let (user, pass, proxy_uri) = match (user, pass, proxy_uri) {
            (Ok(u), Ok(p), Ok(x)) => (u, p, x),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return error_result(2, &e),
        };
        let opts = build_opts(room_filter, user_filter);
        let res  = runtime().block_on(rs_scrape_with_proxy(user, pass, proxy_uri, opts));
        result_to_c(res)
    });

    match outcome {
        Ok(ptr) => ptr,
        Err(_)  => error_result(3, "internal panic in scraper_scrape_with_proxy"),
    }
}

/// Free a `CScrapeResult` returned by `scraper_scrape*()`  .
/// Passing NULL is a no-op.
///
/// FIX: `catch_unwind` ensures that even a bug in the free path cannot
/// propagate a panic across the FFI boundary.
#[no_mangle]
pub extern "C" fn scraper_free_result(result: *mut CScrapeResult) {
    if result.is_null() {
        return;
    }
    let _ = panic::catch_unwind(|| {
        unsafe {
            let r = Box::from_raw(result);

            if !r.rooms.is_null() {
                let rooms = std::slice::from_raw_parts_mut(r.rooms, r.rooms_len);
                for room in rooms.iter() {
                    if !room.map_display.is_null() { drop(CString::from_raw(room.map_display)); }
                    if !room.public_date.is_null() { drop(CString::from_raw(room.public_date)); }
                    if !room.patrol.is_null()      { drop(CString::from_raw(room.patrol));      }
                    if !room.remarks.is_null()     { drop(CString::from_raw(room.remarks));     }
                }
                drop(Vec::from_raw_parts(r.rooms, r.rooms_len, r.rooms_cap));
            }

            if !r.users.is_null() {
                let users = std::slice::from_raw_parts_mut(r.users, r.users_len);
                for user in users.iter() {
                    if !user.username.is_null() { drop(CString::from_raw(user.username)); }
                }
                drop(Vec::from_raw_parts(r.users, r.users_len, r.users_cap));
            }
        }
    });
}