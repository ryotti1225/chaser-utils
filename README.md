# chaser-util 使い方ガイド

## 目次

1. [インストール](#インストール)
2. [モジュール概要](#モジュール概要)
3. [room_list — ルーム一覧・ログインユーザー取得](#room_list--ルーム一覧ログインユーザー取得)
4. [realtime_map_view — リアルタイムマップ取得（単発）](#realtime_map_view--リアルタイムマップ取得単発)
5. [poll_realtime_map_view — リアルタイムマップ取得（ポーリング）](#poll_realtime_map_view--リアルタイムマップ取得ポーリング)
6. [vs_result — 対戦結果取得](#vs_result--対戦結果取得)
7. [プロキシ設定](#プロキシ設定)
8. [フィルタリング詳細](#フィルタリング詳細)
9. [C / C++ FFI](#c--c-ffi)
10. [エラーハンドリング](#エラーハンドリング)

---

## インストール

`Cargo.toml` に追記します。

```toml
[dependencies]
chaser-util = "0.1"
tokio = { version = "1", features = ["full"] }
```

---

## モジュール概要

| モジュール | 用途 |
|---|---|
| `room_list` | ミーティングプレイスのルーム一覧・ログインユーザー取得 |
| `realtime_map_view` | ゲームサーバーのマップ・プレイヤー情報を1回取得 |
| `poll_realtime_map_view` | マップ情報を一定間隔でポーリングし続ける |
| `vs_result` | 対戦結果一覧の取得 |

---

## room_list — ルーム一覧・ログインユーザー取得

### 基本的な使い方

```rust
use chaser_util::room_list::{scrape, ScrapeOptions};

#[tokio::main]
async fn main() {
    let result = scrape("ユーザー名", "パスワード", ScrapeOptions::default())
        .await
        .unwrap();

    // ルーム一覧
    for room in &result.rooms {
        println!(
            "ルーム{} 最大{}人 マップ表示:{} 公開日:{} 巡回:{} 備考:{}",
            room.room,
            room.max_connections,
            room.map_display,
            room.public_date,
            room.patrol,
            room.remarks,
        );
    }

    // ログイン中ユーザー（誰もいない場合は None）
    if let Some(users) = &result.logged_in_users {
        for user in users {
            println!(
                "{}番 {} ルーム{} 状態:{}",
                user.order, user.username, user.room, user.state
            );
        }
    } else {
        println!("ログイン中のユーザーはいません");
    }
}
```

### 手動プロキシを使う場合

```rust
use chaser_util::room_list::{scrape_with_proxy, ScrapeOptions};

let result = scrape_with_proxy(
    "ユーザー名",
    "パスワード",
    "http://192.168.1.1:8080",  // "" を渡すと直接接続
    ScrapeOptions::default(),
).await.unwrap();
```

### フィルタリング

```rust
use chaser_util::room_list::{scrape, RoomFilter, UserFilter, ScrapeOptions, MapDisplay, Patrol};

let room_filter = RoomFilter {
    room_min: Some(1),
    room_max: Some(10),              // ルーム番号 1〜10 のみ
    min_max_conn: Some(4),           // 最大接続数が4以上
    map_display: Some(MapDisplay::ENABLED.to_string()),  // マップ表示あり
    patrol: Some(Patrol::YES.to_string()),               // 巡回あり
    ..Default::default()
};

let user_filter = UserFilter {
    username_contains: Some("cool".to_string()),  // 名前に "cool" を含む
    ..Default::default()
};

let opts = ScrapeOptions::default()
    .with_room_filter(room_filter)
    .with_user_filter(user_filter);

let result = scrape("ユーザー名", "パスワード", opts).await.unwrap();
```

### RoomInfo フィールド一覧

| フィールド | 型 | 内容 |
|---|---|---|
| `room` | `u32` | ルーム番号 |
| `max_connections` | `u32` | 最大接続数 |
| `map_display` | `String` | マップ表示（`MapDisplay::ENABLED` / `DISABLED`） |
| `public_date` | `String` | 公開日 |
| `patrol` | `String` | 巡回（`Patrol::YES` / `NO`） |
| `remarks` | `String` | 備考（`Remarks::RA` / `SAI` / `ZEN` など） |

### LoggedInUser フィールド一覧

| フィールド | 型 | 内容 |
|---|---|---|
| `order` | `u32` | 接続順番号 |
| `username` | `String` | ユーザー名 |
| `room` | `u32` | 接続中のルーム番号 |
| `state` | `u32` | 状態 |

---

## realtime_map_view — リアルタイムマップ取得（単発）

ゲームサーバーのマップビューページから現在のマップ・ターン・プレイヤー情報を1回だけ取得します。

### 基本的な使い方

```rust
use chaser_util::realtime_map_view::{fetch_map_view, MapViewOptions};

#[tokio::main]
async fn main() {
    let result = fetch_map_view(
        "ユーザー名",
        "パスワード",
        MapViewOptions::default(),
    ).await.unwrap();

    println!("ルーム名: {}", result.room_name);
    println!("ターン: {}", result.turn);
    println!("次のプレイヤー: {}", result.next_player);
    println!("マップサイズ: {}行 x {}列",
        result.map.len(),
        result.map.first().map(|r| r.len()).unwrap_or(0)
    );

    // マップ表示
    for row in &result.map {
        for &tile in row {
            print!("{:03} ", tile);
        }
        println!();
    }

    // プレイヤー情報
    for player in &result.players {
        println!(
            "{}: A={} I={} P={} PD={} T={}",
            player.username,
            player.attr_a, player.attr_i,
            player.attr_p, player.attr_pd, player.attr_t
        );
        for cmd in &player.commands {
            println!("  コマンド: {}", cmd);
        }
    }
}
```

### タイル画像URLの取得

```rust
use chaser_util::realtime_map_view::tile_image_url;

// tile_id=12 → "http://.../img/012.gif"
let url = tile_image_url(12);
```

### MapViewResult フィールド一覧

| フィールド | 型 | 内容 |
|---|---|---|
| `room_name` | `String` | ルーム名（H1タグの `[...]` 内） |
| `turn` | `u32` | 現在のターン番号 |
| `next_player` | `String` | 次に行動するプレイヤー名 |
| `map` | `Vec<Vec<TileId>>` | マップの2次元配列（`[行][列]`） |
| `players` | `Vec<PlayerInfo>` | プレイヤー情報一覧 |

### PlayerInfo フィールド一覧

| フィールド | 型 | 内容 |
|---|---|---|
| `username` | `String` | ユーザー名 |
| `attr_a` | `i32` | 攻撃力 (A) |
| `attr_i` | `i32` | 知性 (I) |
| `attr_p` | `i32` | パワー (P) |
| `attr_pd` | `i32` | パワー防御 (PD) |
| `attr_t` | `i32` | トータル (T) |
| `commands` | `Vec<String>` | コマンド一覧（例: `"gr 12,0,12"`） |

---

## poll_realtime_map_view — リアルタイムマップ取得（ポーリング）

一度認証してJSESSIONIDを再利用しながら、指定間隔でマップを取得し続けます。セッション切れ時は自動で再認証します。

### 基本的な使い方

```rust
use std::time::Duration;
use chaser_util::poll_realtime_map_view::{poll_map_view, PollOptions};

#[tokio::main]
async fn main() {
    let mut rx = poll_map_view(
        "ユーザー名",
        "パスワード",
        Duration::from_secs(2),      // サーバーの更新間隔に合わせて2秒推奨
        PollOptions::default(),
    );

    while let Some(mv) = rx.recv().await {
        println!(
            "ターン={:4} 次={:10} マップ={}x{} プレイヤー={}",
            mv.turn,
            mv.next_player,
            mv.map.len(),
            mv.map.first().map(|r| r.len()).unwrap_or(0),
            mv.players.len(),
        );
    }
    // rx がドロップされると自動的にバックグラウンドタスクが終了する
}
```

### 一定ターン数だけ取得する例

```rust
let mut rx = poll_map_view("user", "pass", Duration::from_secs(2), PollOptions::default());
let mut count = 0;

while let Some(mv) = rx.recv().await {
    println!("ターン {}", mv.turn);
    count += 1;
    if count >= 20 {
        break;  // rx がドロップされ、バックグラウンドタスクも停止する
    }
}
```

### プロキシを指定する場合

```rust
use chaser_util::poll_realtime_map_view::PollOptions;

let opts = PollOptions {
    proxy_uri: Some("http://192.168.1.1:8080".to_string()),
};
// 直接接続の場合: proxy_uri: Some("".to_string())
// 自動検出の場合: proxy_uri: None  （デフォルト）
```

> **注意**: エラー（認証失敗・ネットワークエラー）は `eprintln!` で標準エラー出力に記録されます。成功したフレームのみチャンネルに送信されます。

---

## vs_result — 対戦結果取得

### 今日の対戦結果を全件取得

```rust
use chaser_util::vs_result::{fetch_vs_result, VsResultQuery};

#[tokio::main]
async fn main() {
    let results = fetch_vs_result(
        "ユーザー名",
        "パスワード",
        VsResultQuery::today(),   // 今日の日付を自動設定
        None,                     // プロキシ: None=自動検出
    ).await.unwrap();

    for battle in &results {
        println!(
            "ルーム{} {} 〜 {}",
            battle.room, battle.start_time, battle.end_time
        );
        for player in &battle.players {
            println!(
                "  {}番 {} | ターン取得:{} 残:{} 合計:{}pt",
                player.order, player.username,
                player.get_turn, player.rem_turn, player.total_point
            );
            // 詳細ポイント（存在しない場合もある）
            if let Some(ap) = player.action_point {
                println!("    行動:{} アイテム:{} 設置:{} ダメージ:{}",
                    ap,
                    player.item_point.unwrap_or(0),
                    player.put_point.unwrap_or(0),
                    player.put_damage.unwrap_or(0),
                );
            }
        }
    }
}
```

### 特定の日付を指定する

```rust
let results = fetch_vs_result(
    "user", "pass",
    VsResultQuery::for_date("2026-04-07"),
    None,
).await.unwrap();
```

### 検索条件を細かく指定する

```rust
let query = VsResultQuery {
    min_room: 1,
    max_room: 5,                          // ルーム1〜5のみ
    min_total_point: 0,                   // 合計ポイントが0以上
    min_start_date: "2026-04-01".to_string(),
    max_start_date: "2026-04-07".to_string(),  // 日付範囲指定
    // 他のフィールドはデフォルト値を使う
    ..VsResultQuery::today()
};
```

### BattleResult フィールド一覧

| フィールド | 型 | 内容 |
|---|---|---|
| `room` | `u32` | ルーム番号 |
| `start_time` | `String` | 対戦開始時刻 |
| `end_time` | `String` | 対戦終了時刻 |
| `players` | `Vec<PlayerResult>` | 参加プレイヤー一覧（接続順） |

### PlayerResult フィールド一覧

| フィールド | 型 | 内容 |
|---|---|---|
| `order` | `u32` | 接続順（1=先手） |
| `username` | `String` | ユーザー名 |
| `get_turn` | `i32` | 取得ターン数 |
| `rem_turn` | `i32` | 残りターン数 |
| `total_point` | `i32` | 合計ポイント |
| `action_point` | `Option<i32>` | 行動ポイント（ない場合あり） |
| `item_point` | `Option<i32>` | アイテムポイント（ない場合あり） |
| `put_point` | `Option<i32>` | 設置ポイント（ない場合あり） |
| `put_damage` | `Option<i32>` | 設置ダメージ（ない場合あり） |

---

## プロキシ設定

全モジュール共通で以下の3モードをサポートします。

| 設定値 | 動作 |
|---|---|
| `None` | 自動検出（環境変数 → Windowsレジストリ → macOS SCF → 直接接続） |
| `Some("")` | 強制直接接続 |
| `Some("http://host:port")` | 指定プロキシを使用 |

環境変数 `HTTP_PROXY` / `HTTPS_PROXY` も自動検出対象です。

---

## フィルタリング詳細

### RoomFilter フィールド一覧

| フィールド | 型 | 条件 |
|---|---|---|
| `room` | `Option<u32>` | ルーム番号が一致 |
| `room_min` | `Option<u32>` | ルーム番号がこれ以上 |
| `room_max` | `Option<u32>` | ルーム番号がこれ以下 |
| `min_max_conn` | `Option<u32>` | 最大接続数がこれ以上 |
| `max_max_conn` | `Option<u32>` | 最大接続数がこれ以下 |
| `map_display` | `Option<String>` | マップ表示が一致 |
| `public_date` | `Option<String>` | 公開日が完全一致 |
| `public_date_contains` | `Option<String>` | 公開日に文字列を含む |
| `patrol` | `Option<String>` | 巡回が一致 |
| `remarks` | `Option<String>` | 備考が完全一致 |
| `remarks_contains` | `Option<String>` | 備考に文字列を含む |

### UserFilter フィールド一覧

| フィールド | 型 | 条件 |
|---|---|---|
| `order` | `Option<u32>` | 接続番号が一致 |
| `order_min` | `Option<u32>` | 接続番号がこれ以上 |
| `order_max` | `Option<u32>` | 接続番号がこれ以下 |
| `username` | `Option<String>` | ユーザー名が完全一致 |
| `username_contains` | `Option<String>` | ユーザー名に文字列を含む |
| `room` | `Option<u32>` | ルーム番号が一致 |
| `room_min` | `Option<u32>` | ルーム番号がこれ以上 |
| `room_max` | `Option<u32>` | ルーム番号がこれ以下 |
| `state` | `Option<u32>` | 状態が一致 |

### 定数

```rust
use chaser_util::room_list::{MapDisplay, Patrol, Remarks};

MapDisplay::ENABLED   // "可"
MapDisplay::DISABLED  // "否"

Patrol::YES           // "有"
Patrol::NO            // "×"

Remarks::RA           // "ラ"
Remarks::SAI          // "埼"
Remarks::ZEN          // "全"
```

---

## C / C++ FFI

ライブラリを共有ライブラリとしてビルドし、`chaser-util.h` を使ってC/C++から呼び出せます。

### ビルド

```sh
cargo build --release
# Windows: target/release/chaser_util.dll  +  chaser_util.dll.lib
# Linux:   target/release/libchaser_util.so
```

### C++ での使い方

```cpp
#include "chaser-util.h"
#include <iostream>

int main() {
    // フィルターなしで取得
    auto result = chaser_util::scrape("ユーザー名", "パスワード");

    for (auto& room : result.rooms) {
        std::cout << "ルーム" << room.room
                  << " 最大" << room.max_connections << "人\n";
    }

    if (result.logged_in_users) {
        for (auto& user : *result.logged_in_users) {
            std::cout << user.username << " ルーム" << user.room << "\n";
        }
    }
}
```

### フィルターを使う場合（C++）

```cpp
chaser_util::RoomFilter rf;
rf.room_range(1, 10)                         // ルーム1〜10
  .min_max_conn(4)                            // 最大接続数4以上
  .map_display(chaser_util::MapDisplay::ENABLED);  // マップ表示あり

chaser_util::UserFilter uf;
uf.username_contains(u8"cool");

auto result = chaser_util::scrape("user", "pass", &rf, &uf);
```

### 手動プロキシ（C++）

```cpp
auto result = chaser_util::scrape_with_proxy(
    "user", "pass",
    "http://192.168.1.1:8080",  // "" で直接接続
    nullptr, nullptr
);
```

### エラー処理（C++）

C++ ラッパーはエラー時に `std::runtime_error` を投げます。

```cpp
try {
    auto result = chaser_util::scrape("user", "pass");
} catch (const std::runtime_error& e) {
    std::cerr << "エラー: " << e.what() << "\n";
}
```

### エラーコード一覧（C API）

| `error_code` | 意味 |
|---|---|
| `0` | 成功 |
| `1` | スクレイプ / ネットワークエラー（`scraper_last_error()` で詳細取得） |
| `2` | 不正な引数（`user` / `pass` / `proxy_uri` が NULL） |
| `3` | 内部パニック（発生した場合はバグ報告をお願いします） |

---

## エラーハンドリング

全ての非同期関数は `Result<T, Box<dyn std::error::Error + Send + Sync>>` を返します。

```rust
match scrape("user", "pass", ScrapeOptions::default()).await {
    Ok(result) => {
        println!("ルーム数: {}", result.rooms.len());
    }
    Err(e) => {
        eprintln!("取得失敗: {}", e);
        // ネットワークエラー、認証失敗、HTML解析エラーなどが含まれる
    }
}
```

`poll_map_view` はチャンネルを返す同期関数で、エラーはバックグラウンドタスク内で `eprintln!` に出力されます。チャンネルには成功したフレームのみ届きます。
