/**
 * chaser_util.h  -  CHaser Online MeetingPlace scraper  C / C++ API
 *
 * -----------------------------------------------------------------
 * ビルド手順
 * -----------------------------------------------------------------
 *
 *   cargo build --release
 *
 *   生成物:
 *     Windows : target\release\chaser_util.dll
 *               target\release\chaser_util.dll.lib   (インポートライブラリ)
 *     Linux   : target/release/libchaser_util.so
 *     Android : target/<abi>/release/libchaser_util.so
 *
 * -----------------------------------------------------------------
 * リンク手順
 * -----------------------------------------------------------------
 *
 *   Windows (MSVC):
 *     cl /std:c++17 your_app.cpp /I<include_dir> chaser_util.dll.lib
 *     実行時に chaser_util.dll を exe と同じフォルダに置く
 *
 *   Linux / Android (GCC / Clang):
 *     g++ -std=c++17 your_app.cpp -I<include_dir> \
 *         -L<lib_dir> -lchaser_util -Wl,-rpath,<lib_dir> -o your_app
 *
 * -----------------------------------------------------------------
 * メモリ管理ルール
 * -----------------------------------------------------------------
 *
 *   scraper_scrape*() が返すポインタは Rust ヒープ上に確保されます。
 *   必ず scraper_free_result() で解放してください。
 *   個別フィールドを free() しないでください。
 *
 * -----------------------------------------------------------------
 * 文字列エンコーディング
 * -----------------------------------------------------------------
 *
 *   全文字列は UTF-8 / null 終端です。
 */

#pragma once
#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>

/* ================================================================
 * C 構造体
 * ================================================================ */

typedef struct {
    unsigned int  room;
    unsigned int  max_connections;
    char*         map_display;     /* "可" or "否" */
    char*         public_date;
    char*         patrol;          /* "有" or "×" */
    char*         remarks;
} CRoomInfo;

typedef struct {
    unsigned int  order;
    char*         username;
    unsigned int  room;
    unsigned int  state;
} CLoggedInUser;

typedef struct {
    CRoomInfo*      rooms;
    size_t          rooms_len;
    CLoggedInUser*  users;       /* NULL = ログイン中ユーザーなし */
    size_t          users_len;   /* 0    = ログイン中ユーザーなし */
    unsigned int    error_code;  /* 0 = 成功 */
} CScrapeResult;

/* ================================================================
 * フィルター構造体
 *
 *   数値フィールド: *_enabled = 0 で無効、1 で有効
 *   文字列フィールド: NULL で無効
 * ================================================================ */

typedef struct {
    unsigned int  room_enabled;
    unsigned int  room;
    unsigned int  room_min_enabled;
    unsigned int  room_min;
    unsigned int  room_max_enabled;
    unsigned int  room_max;
    unsigned int  min_max_conn_enabled;
    unsigned int  min_max_conn;
    unsigned int  max_max_conn_enabled;
    unsigned int  max_max_conn;
    const char*   map_display;            /* NULL = フィルターなし */
    const char*   public_date;
    const char*   public_date_contains;
    const char*   patrol;
    const char*   remarks;
    const char*   remarks_contains;
} CRoomFilter;

typedef struct {
    unsigned int  order_enabled;
    unsigned int  order;
    unsigned int  order_min_enabled;
    unsigned int  order_min;
    unsigned int  order_max_enabled;
    unsigned int  order_max;
    const char*   username;               /* NULL = フィルターなし */
    const char*   username_contains;
    unsigned int  room_enabled;
    unsigned int  room;
    unsigned int  room_min_enabled;
    unsigned int  room_min;
    unsigned int  room_max_enabled;
    unsigned int  room_max;
    unsigned int  state_enabled;
    unsigned int  state;
} CUserFilter;

/* ================================================================
 * C API 関数
 * ================================================================ */

/**
 * プロキシ自動検出でスクレイプします。
 *
 * 検出順序:
 *   1. HTTP_PROXY / HTTPS_PROXY 環境変数
 *   2. Windows レジストリ (Windows のみ)
 *   3. macOS System Configuration (macOS のみ)
 *   4. 直接接続
 *
 * @param user        ログインユーザー名 (UTF-8)
 * @param pass        ログインパスワード (UTF-8)
 * @param room_filter ルームフィルター (NULL = フィルターなし)
 * @param user_filter ユーザーフィルター (NULL = フィルターなし)
 * @return scraper_free_result() で解放必須
 */
CScrapeResult* scraper_scrape(
    const char*        user,
    const char*        pass,
    const CRoomFilter* room_filter,
    const CUserFilter* user_filter
);

/**
 * プロキシを手動指定してスクレイプします。
 * Android など自動検出が使えない環境向け。
 *
 * @param user        ログインユーザー名 (UTF-8)
 * @param pass        ログインパスワード (UTF-8)
 * @param proxy_uri   "http://192.168.1.1:8080" 形式。"" で直接接続。
 * @param room_filter ルームフィルター (NULL = フィルターなし)
 * @param user_filter ユーザーフィルター (NULL = フィルターなし)
 * @return scraper_free_result() で解放必須
 */
CScrapeResult* scraper_scrape_with_proxy(
    const char*        user,
    const char*        pass,
    const char*        proxy_uri,
    const CRoomFilter* room_filter,
    const CUserFilter* user_filter
);

/**
 * CScrapeResult を解放します。必ず呼び出してください。
 */
void scraper_free_result(CScrapeResult* result);

/**
 * 最後のエラーメッセージを返します (UTF-8)。
 * 次の FFI 呼び出しまで有効です。
 */
const char* scraper_last_error(void);

#ifdef __cplusplus
} /* extern "C" */
#endif


/* ================================================================
 * C++ ラッパー (ヘッダーオンリー)
 * ================================================================ */
#ifdef __cplusplus
#include <string>
#include <vector>
#include <optional>
#include <stdexcept>

namespace chaser {
namespace RoomList {

/* ---- 定数 namespace ---- */

namespace MapDisplay {
    static constexpr const char* Enabled  = "\xe5\x8f\xaf";  ///< 可 (UTF-8)
    static constexpr const char* Disabled = "\xe5\x90\xa6";  ///< 否 (UTF-8)
}

namespace Patrol {
    static constexpr const char* Yes = "\xe6\x9c\x89";  ///< 有 (UTF-8)
    static constexpr const char* No  = "\xc3\x97";        ///< × (UTF-8)
}

namespace Remarks {
    static constexpr const char* Ra  = "\xe3\x83\xa9";  ///< ラ (UTF-8)
    static constexpr const char* Sai = "\xe5\x9f\xbc";  ///< 埼 (UTF-8)
    static constexpr const char* Zen = "\xe5\x85\xa8";  ///< 全 (UTF-8)
}

/* ---- データ型 ---- */

struct RoomInfo {
    unsigned int room;
    unsigned int max_connections;
    std::string  map_display;
    std::string  public_date;
    std::string  patrol;
    std::string  remarks;
};

struct LoggedInUser {
    unsigned int order;
    std::string  username;
    unsigned int room;
    unsigned int state;
};

struct ScrapeResult {
    std::optional<std::vector<LoggedInUser>> logged_in_users; // nullopt = ユーザーなし
    std::vector<RoomInfo>                    rooms;
};

/* ---- フィルタービルダー (メソッドチェーン) ---- */

struct RoomFilter {
    CRoomFilter c{};

    RoomFilter& room(unsigned int v)
        { c.room_enabled=1; c.room=v; return *this; }
    RoomFilter& room_range(unsigned int lo, unsigned int hi)
        { c.room_min_enabled=1; c.room_min=lo;
          c.room_max_enabled=1; c.room_max=hi; return *this; }
    RoomFilter& min_max_conn(unsigned int v)
        { c.min_max_conn_enabled=1; c.min_max_conn=v; return *this; }
    RoomFilter& max_max_conn(unsigned int v)
        { c.max_max_conn_enabled=1; c.max_max_conn=v; return *this; }
    RoomFilter& map_display(const char* v)
        { c.map_display=v; return *this; }
    RoomFilter& map_display(const char8_t* v)
        { return map_display(reinterpret_cast<const char*>(v)); }
    RoomFilter& public_date(const char* v)
        { c.public_date=v; return *this; }
    RoomFilter& public_date(const char8_t* v)
        { return public_date(reinterpret_cast<const char*>(v)); }
    RoomFilter& public_date_contains(const char* v)
        { c.public_date_contains=v; return *this; }
    RoomFilter& public_date_contains(const char8_t* v)
        { return public_date_contains(reinterpret_cast<const char*>(v)); }
    RoomFilter& patrol(const char* v)
        { c.patrol=v; return *this; }
    RoomFilter& patrol(const char8_t* v)
        { return patrol(reinterpret_cast<const char*>(v)); }
    RoomFilter& remarks(const char* v)
        { c.remarks=v; return *this; }
    RoomFilter& remarks(const char8_t* v)
        { return remarks(reinterpret_cast<const char*>(v)); }
    RoomFilter& remarks_contains(const char* v)
        { c.remarks_contains=v; return *this; }
    RoomFilter& remarks_contains(const char8_t* v)
        { return remarks_contains(reinterpret_cast<const char*>(v)); }
};

struct UserFilter {
    CUserFilter c{};

    UserFilter& order(unsigned int v)
        { c.order_enabled=1; c.order=v; return *this; }
    UserFilter& order_range(unsigned int lo, unsigned int hi)
        { c.order_min_enabled=1; c.order_min=lo;
          c.order_max_enabled=1; c.order_max=hi; return *this; }
    UserFilter& username(const char* v)
        { c.username=v; return *this; }
    UserFilter& username(const char8_t* v)
        { return username(reinterpret_cast<const char*>(v)); }
    UserFilter& username_contains(const char* v)
        { c.username_contains=v; return *this; }
    UserFilter& username_contains(const char8_t* v)
        { return username_contains(reinterpret_cast<const char*>(v)); }
    UserFilter& room(unsigned int v)
        { c.room_enabled=1; c.room=v; return *this; }
    UserFilter& room_range(unsigned int lo, unsigned int hi)
        { c.room_min_enabled=1; c.room_min=lo;
          c.room_max_enabled=1; c.room_max=hi; return *this; }
    UserFilter& state(unsigned int v)
        { c.state_enabled=1; c.state=v; return *this; }
};

/* ---- 内部変換ヘルパー ---- */

inline ScrapeResult convert(CScrapeResult* raw) {
    if (!raw) throw std::runtime_error("null result");
    if (raw->error_code != 0) {
        std::string msg = scraper_last_error();
        scraper_free_result(raw);
        throw std::runtime_error(msg);
    }

    ScrapeResult out;

    for (size_t i = 0; i < raw->rooms_len; ++i) {
        auto& r = raw->rooms[i];
        out.rooms.push_back({
            r.room,
            r.max_connections,
            r.map_display  ? r.map_display  : "",
            r.public_date  ? r.public_date  : "",
            r.patrol       ? r.patrol       : "",
            r.remarks      ? r.remarks      : "",
        });
    }

    if (raw->users && raw->users_len > 0) {
        std::vector<LoggedInUser> users;
        for (size_t i = 0; i < raw->users_len; ++i) {
            auto& u = raw->users[i];
            users.push_back({
                u.order,
                u.username ? u.username : "",
                u.room,
                u.state,
            });
        }
        out.logged_in_users = std::move(users);
    }

    scraper_free_result(raw);
    return out;
}

/* ---- 公開 API ---- */

/**
 * プロキシ自動検出でスクレイプします。
 * 失敗時は std::runtime_error をスローします。
 */
inline ScrapeResult scrape(
    const std::string& user,
    const std::string& pass,
    const RoomFilter*  rf = nullptr,
    const UserFilter*  uf = nullptr)
{
    return convert(scraper_scrape(
        user.c_str(), pass.c_str(),
        rf ? &rf->c : nullptr,
        uf ? &uf->c : nullptr
    ));
}

/**
 * プロキシを手動指定してスクレイプします。
 * proxy_uri="" で直接接続。
 * 失敗時は std::runtime_error をスローします。
 */
inline ScrapeResult scrape_with_proxy(
    const std::string& user,
    const std::string& pass,
    const std::string& proxy_uri,
    const RoomFilter*  rf = nullptr,
    const UserFilter*  uf = nullptr)
{
    return convert(scraper_scrape_with_proxy(
        user.c_str(), pass.c_str(), proxy_uri.c_str(),
        rf ? &rf->c : nullptr,
        uf ? &uf->c : nullptr
    ));
}

} // namespace RoomList
} // namespace chaser

#endif /* __cplusplus */