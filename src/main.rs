use chaser_util::chaser::room_list::{
    scrape, ScrapeOptions,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let result = scrape("hot", "hot", ScrapeOptions::default()).await?;

    match &result.logged_in_users {
        None => println!("ログイン中のユーザーはいません"),
        Some(users) => {
            println!("ログイン中 ({} 件):", users.len());
            for u in users {
                println!(
                    "  order={} user={} room={} state={}",
                    u.order, u.username, u.room, u.state
                );
            }
        }
    }

    Ok(())
}