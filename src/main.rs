use std::time::Duration;
use chaser_util::poll_realtime_map_view::{poll_map_view, PollOptions};

#[tokio::main]
async fn main() {
    let mut rx = poll_map_view(
        "cool30",
        "cool",
        Duration::from_secs(2),
        PollOptions::default(),
    );

    while let Some(mv) = rx.recv().await {
        println!(
            "turn={:4}  next={:10}  map={}x{}  players={}",
            mv.turn,
            mv.next_player,
            mv.map.len(),
            mv.map.first().map(|r| r.len()).unwrap_or(0),
            mv.players.len(),
        );
    }
}