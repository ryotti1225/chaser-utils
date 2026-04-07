use chaser_util::realtime_map_view::{fetch_map_view, tile_image_url, MapViewOptions};
use chaser_util::room_list::{scrape, RoomFilter, ScrapeOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // ================================================================
    // Room list: 2-player rooms with map display enabled, no bot patrol
    // ================================================================
    let room_filter = RoomFilter {
        min_max_conn: Some(2),
        max_max_conn: Some(2),
        map_display:  Some(chaser_util::room_list::MapDisplay::ENABLED.to_string()),
        patrol:       Some(chaser_util::room_list::Patrol::NO.to_string()),
        ..Default::default()
    };
    let result = scrape("hot", "hot", ScrapeOptions::default().with_room_filter(room_filter)).await?;

    println!("=== Rooms (2-player, map enabled, no patrol) ({} found) ===", result.rooms.len());
    for r in &result.rooms {
        println!(
            "  room={:3}  max_conn={:2}  map={}  date={}  patrol={}  remarks={}",
            r.room, r.max_connections, r.map_display,
            r.public_date, r.patrol, r.remarks
        );
    }

    println!();
    match &result.logged_in_users {
        None => println!("No users currently logged in."),
        Some(users) => {
            println!("=== Logged-in users ({}) ===", users.len());
            for u in users {
                println!(
                    "  order={} user={} room={} state={}",
                    u.order, u.username, u.room, u.state
                );
            }
        }
    }

    // ================================================================
    // Map view (set user/pass to an active game participant)
    // ================================================================
    println!();
    println!("=== Map view ===");
    match fetch_map_view("cool33", "cool", MapViewOptions::default()).await {
        Err(e) => println!("  fetch failed: {}", e),
        Ok(mv) => {
            // Basic info
            println!("  room_name  : {}", mv.room_name);
            println!("  turn       : {}", mv.turn);
            println!("  next_player: {}", mv.next_player);
            println!("  map size   : {} rows x {} cols",
                mv.map.len(),
                mv.map.first().map(|r| r.len()).unwrap_or(0)
            );

            // Unique tile image URLs
            let mut tile_ids: Vec<u32> = mv.map.iter()
                .flat_map(|row| row.iter().copied())
                .collect();
            tile_ids.sort_unstable();
            tile_ids.dedup();
            println!();
            println!("  --- Tile image URLs ({} types) ---", tile_ids.len());
            for tid in &tile_ids {
                println!("    {:03} : {}", tid, tile_image_url(*tid));
            }

            // Map grid
            println!();
            println!("  --- Map ---");
            for (i, row) in mv.map.iter().enumerate() {
                let line: Vec<String> = row.iter().map(|t| format!("{:03}", t)).collect();
                println!("  row{:02}: {}", i, line.join(" "));
            }

            // Player info + command list
            println!();
            println!("  --- Players ({}) ---", mv.players.len());
            for p in &mv.players {
                println!("  +-- username : {}", p.username);
                println!("  |   A={:8}  I={:8}  P={:4}  PD={:4}  T={:8}",
                    p.attr_a, p.attr_i, p.attr_p, p.attr_pd, p.attr_t);
                if p.commands.is_empty() {
                    println!("  +-- commands : (none)");
                } else {
                    println!("  |   commands ({}):", p.commands.len());
                    for (i, cmd) in p.commands.iter().enumerate() {
                        let prefix = if i == p.commands.len() - 1 { "  +" } else { "  |" };
                        println!("  {}   [{}] {}", prefix, i, cmd);
                    }
                }
            }
        }
    }

    Ok(())
}