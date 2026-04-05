//! C FFI layer
//!
//! Exposes the scraper API as a C-compatible interface.
//! C++ users include `scraper.h` and link against `scraper.dll` / `libscraper.so`.
//!
//! Memory model:
//!   - All returned pointers are heap-allocated by Rust.
//!   - The caller MUST free them with the corresponding `scraper_free_*` function.
//!   - Passing NULL for optional filter pointers means "no filter".

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uint};
use std::ptr;

use crate::chaser::room_list::{
    scrape as rs_scrape,
    scrape_with_proxy as rs_scrape_with_proxy,
    RoomFilter, ScrapeOptions, ScrapeResult, UserFilter,
};

// ----------------------------------------------------------------
// C-compatible structs
// ----------------------------------------------------------------

/// C-compatible room info
#[repr(C)]
pub struct CRoomInfo {
    pub room:            c_uint,
    pub max_connections: c_uint,
    pub map_display:     *mut c_char,  // "可" / "否"
    pub public_date:     *mut c_char,
    pub patrol:          *mut c_char,  // "有" / "×"
    pub remarks:         *mut c_char,
}

/// C-compatible logged-in user
#[repr(C)]
pub struct CLoggedInUser {
    pub order:    c_uint,
    pub username: *mut c_char,
    pub room:     c_uint,
    pub state:    c_uint,
}

/// C-compatible scrape result
#[repr(C)]
pub struct CScrapeResult {
    // rooms
    pub rooms:       *mut CRoomInfo,
    pub rooms_len:   usize,
    // logged-in users (NULL + len=0 means "no users")
    pub users:       *mut CLoggedInUser,
    pub users_len:   usize,
    /// 0 = success, non-zero = error (call scraper_last_error() for message)
    pub error_code:  c_uint,
}

/// C-compatible room filter
#[repr(C)]
pub struct CRoomFilter {
    // 0 = disabled, 1 = enabled
    pub room_enabled:              c_uint,
    pub room:                      c_uint,
    pub room_min_enabled:          c_uint,
    pub room_min:                  c_uint,
    pub room_max_enabled:          c_uint,
    pub room_max:                  c_uint,
    pub min_max_conn_enabled:      c_uint,
    pub min_max_conn:              c_uint,
    pub max_max_conn_enabled:      c_uint,
    pub max_max_conn:              c_uint,
    pub map_display:               *const c_char,  // NULL = disabled
    pub public_date:               *const c_char,
    pub public_date_contains:      *const c_char,
    pub patrol:                    *const c_char,
    pub remarks:                   *const c_char,
    pub remarks_contains:          *const c_char,
}

/// C-compatible user filter
#[repr(C)]
pub struct CUserFilter {
    pub order_enabled:             c_uint,
    pub order:                     c_uint,
    pub order_min_enabled:         c_uint,
    pub order_min:                 c_uint,
    pub order_max_enabled:         c_uint,
    pub order_max:                 c_uint,
    pub username:                  *const c_char,  // NULL = disabled
    pub username_contains:         *const c_char,
    pub room_enabled:              c_uint,
    pub room:                      c_uint,
    pub room_min_enabled:          c_uint,
    pub room_min:                  c_uint,
    pub room_max_enabled:          c_uint,
    pub room_max:                  c_uint,
    pub state_enabled:             c_uint,
    pub state:                     c_uint,
}

// ----------------------------------------------------------------
// Thread-local error message storage
// ----------------------------------------------------------------

thread_local! {
    static LAST_ERROR: std::cell::RefCell<CString> =
        std::cell::RefCell::new(CString::new("").unwrap());
}

fn set_last_error(msg: &str) {
    let c = CString::new(msg.replace('\0', "?")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
}

/// Returns a pointer to the last error message string.
/// The pointer is valid until the next FFI call on this thread.
#[no_mangle]
pub extern "C" fn scraper_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
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
        order:             enabled(f.order_enabled,    f.order),
        order_min:         enabled(f.order_min_enabled,f.order_min),
        order_max:         enabled(f.order_max_enabled,f.order_max),
        username:          cstr_opt(f.username),
        username_contains: cstr_opt(f.username_contains),
        room:              enabled(f.room_enabled,     f.room),
        room_min:          enabled(f.room_min_enabled, f.room_min),
        room_max:          enabled(f.room_max_enabled, f.room_max),
        state:             enabled(f.state_enabled,    f.state),
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

fn result_to_c(res: Result<ScrapeResult, Box<dyn std::error::Error + Send + Sync>>) -> *mut CScrapeResult {
    match res {
        Err(e) => {
            set_last_error(&e.to_string());
            let r = Box::new(CScrapeResult {
                rooms: ptr::null_mut(), rooms_len: 0,
                users: ptr::null_mut(), users_len: 0,
                error_code: 1,
            });
            Box::into_raw(r)
        }
        Ok(sr) => {
            // convert rooms
            let mut c_rooms: Vec<CRoomInfo> = sr.rooms.iter().map(|r| CRoomInfo {
                room:            r.room,
                max_connections: r.max_connections,
                map_display:     to_cstring(&r.map_display),
                public_date:     to_cstring(&r.public_date),
                patrol:          to_cstring(&r.patrol),
                remarks:         to_cstring(&r.remarks),
            }).collect();
            c_rooms.shrink_to_fit();
            let rooms_len = c_rooms.len();
            let rooms_ptr = if rooms_len > 0 {
                let p = c_rooms.as_mut_ptr();
                std::mem::forget(c_rooms);
                p
            } else { ptr::null_mut() };

            // convert users
            let (users_ptr, users_len) = match sr.logged_in_users {
                None => (ptr::null_mut(), 0),
                Some(users) => {
                    let mut c_users: Vec<CLoggedInUser> = users.iter().map(|u| CLoggedInUser {
                        order:    u.order,
                        username: to_cstring(&u.username),
                        room:     u.room,
                        state:    u.state,
                    }).collect();
                    c_users.shrink_to_fit();
                    let len = c_users.len();
                    let p   = c_users.as_mut_ptr();
                    std::mem::forget(c_users);
                    (p, len)
                }
            };

            let r = Box::new(CScrapeResult {
                rooms: rooms_ptr, rooms_len,
                users: users_ptr, users_len,
                error_code: 0,
            });
            Box::into_raw(r)
        }
    }
}

// ----------------------------------------------------------------
// Public C API
// ----------------------------------------------------------------

/// Scrape with automatic proxy detection.
///
/// @param user         Login username (UTF-8, null-terminated)
/// @param pass         Login password (UTF-8, null-terminated)
/// @param room_filter  Pointer to CRoomFilter, or NULL for no filter
/// @param user_filter  Pointer to CUserFilter, or NULL for no filter
/// @return             Heap-allocated CScrapeResult. Free with scraper_free_result().
#[no_mangle]
pub extern "C" fn scraper_scrape(
    user:        *const c_char,
    pass:        *const c_char,
    room_filter: *const CRoomFilter,
    user_filter: *const CUserFilter,
) -> *mut CScrapeResult {
    let user = unsafe { CStr::from_ptr(user) }.to_str().unwrap_or("");
    let pass = unsafe { CStr::from_ptr(pass) }.to_str().unwrap_or("");
    let opts = build_opts(room_filter, user_filter);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt.block_on(rs_scrape(user, pass, opts));
    result_to_c(res)
}

/// Scrape with a manually specified proxy.
///
/// @param user        Login username (UTF-8, null-terminated)
/// @param pass        Login password (UTF-8, null-terminated)
/// @param proxy_uri   Proxy URI e.g. "http://192.168.1.1:8080", or "" for direct
/// @param room_filter Pointer to CRoomFilter, or NULL for no filter
/// @param user_filter Pointer to CUserFilter, or NULL for no filter
/// @return            Heap-allocated CScrapeResult. Free with scraper_free_result().
#[no_mangle]
pub extern "C" fn scraper_scrape_with_proxy(
    user:        *const c_char,
    pass:        *const c_char,
    proxy_uri:   *const c_char,
    room_filter: *const CRoomFilter,
    user_filter: *const CUserFilter,
) -> *mut CScrapeResult {
    let user      = unsafe { CStr::from_ptr(user)      }.to_str().unwrap_or("");
    let pass      = unsafe { CStr::from_ptr(pass)      }.to_str().unwrap_or("");
    let proxy_uri = unsafe { CStr::from_ptr(proxy_uri) }.to_str().unwrap_or("");
    let opts = build_opts(room_filter, user_filter);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt.block_on(rs_scrape_with_proxy(user, pass, proxy_uri, opts));
    result_to_c(res)
}

/// Free a CScrapeResult returned by scraper_scrape* functions.
#[no_mangle]
pub extern "C" fn scraper_free_result(result: *mut CScrapeResult) {
    if result.is_null() { return; }
    unsafe {
        let r = Box::from_raw(result);

        // free room strings
        if !r.rooms.is_null() {
            let rooms = std::slice::from_raw_parts_mut(r.rooms, r.rooms_len);
            for room in rooms.iter() {
                if !room.map_display.is_null() { drop(CString::from_raw(room.map_display)); }
                if !room.public_date.is_null() { drop(CString::from_raw(room.public_date)); }
                if !room.patrol.is_null()      { drop(CString::from_raw(room.patrol));      }
                if !room.remarks.is_null()     { drop(CString::from_raw(room.remarks));     }
            }
            drop(Vec::from_raw_parts(r.rooms, r.rooms_len, r.rooms_len));
        }

        // free user strings
        if !r.users.is_null() {
            let users = std::slice::from_raw_parts_mut(r.users, r.users_len);
            for user in users.iter() {
                if !user.username.is_null() { drop(CString::from_raw(user.username)); }
            }
            drop(Vec::from_raw_parts(r.users, r.users_len, r.users_len));
        }
    }
}