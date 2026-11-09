use spotcore::{Paging, Spotify, web::SearchKind};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cdp_var = std::env::var("SPOTCORE_CDP_URL").ok();
    let arg1 = std::env::args().nth(1);

    let mut spotify = match cdp_var {
        Some(endpoint) => Spotify::connect_web(&endpoint).await?,
        None => Spotify::launch().await?,
    };

    if let Some(uri) = &arg1
        && (uri.starts_with("spotify:track:")
            || uri.starts_with("spotify:album:")
            || uri.starts_with("spotify:playlist:"))
    {
        println!("Loading & playing: {uri}");
        spotify.player.play(uri).await?;
        println!("Playback started! Streaming events (press Ctrl+C to stop)...");
        loop {
            tokio::select! {
                event = spotify.player.next_event() => println!("{event:?}"),
                result = tokio::signal::ctrl_c() => { result?; break; }
            }
        }
        return Ok(());
    }

    let query = arg1.unwrap_or_else(|| "Miles Davis".into());
    let result = spotify
        .api
        .search()
        .query(SearchKind::Tracks, &query, Paging::default())
        .await?;
    for item in result.items {
        println!("{} — {}", item.name, item.uri);
    }
    // A second argument explicitly requests playback; searching alone changes nothing.
    if let Some(uri) = std::env::args().nth(2) {
        spotify.player.play(&uri).await?;
        loop {
            tokio::select! {event=spotify.player.next_event()=>println!("{:?}",event?),result=tokio::signal::ctrl_c()=>{result?;break;}}
        }
    }
    Ok(())
}
