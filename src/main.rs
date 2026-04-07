use chaser_util::vs_result::{fetch_vs_result, VsResultQuery};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let query = VsResultQuery::default();
    match fetch_vs_result("cool33", "cool", query, None).await {
        Err(e) => println!("fetch failed: {}", e),
        Ok(battles) => {
            println!("{} battle(s) found", battles.len());
            for b in &battles {
                println!("room={} {} -> {}", b.room, b.start_time, b.end_time);
                for p in &b.players {
                    println!("  [{}] {} | get={} rem={} total={} action={:?} item={:?} put={:?} damage={:?}",
                        p.order, p.username, p.get_turn, p.rem_turn, p.total_point,
                        p.action_point, p.item_point, p.put_point, p.put_damage);
                }
            }
        }
    }
    Ok(())
}