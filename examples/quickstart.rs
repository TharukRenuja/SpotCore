use spotcore::{BrowserRole, Paging, PlaybackEvent, SearchKind, SpotCore};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (query, cookie) = arguments(std::env::args().skip(1))?;
    let playback = query.starts_with("spotify:");
    let mut builder = SpotCore::builder();
    if let Some(cookie) = cookie {
        builder = builder.session_cookie(cookie);
    } else if let Some(file) = std::env::var_os("SPOTCORE_COOKIE_FILE") {
        let cookie = std::fs::read_to_string(file)?;
        builder = builder.session_cookie(cookie.trim());
    }
    if !playback {
        builder = builder.browser_role(BrowserRole::Services);
    }
    #[cfg(feature = "native")]
    let builder = match std::env::var("SOLOIST_API_KEY") {
        Ok(key) if playback => builder.native(key),
        _ => builder,
    };
    let mut spotify = builder.launch().await?;
    if let Some(reason) = spotify.native_fallback_reason() {
        eprintln!("Using Web playback: {reason}");
    }
    if !playback {
        let tracks = spotify
            .search()
            .query(SearchKind::Tracks, &query, Paging::default())
            .await?;
        for track in tracks.items {
            println!("{} — {}", track.name, track.uri);
        }
        spotify.shutdown().await?;
        return Ok(());
    }
    spotify.play(query).await?;
    loop {
        tokio::select! {
            event = spotify.next_event() => match event? {
                PlaybackEvent::Snapshot(state) => println!("{:?}: {:?}", state.state, state.track.map(|track| track.name)),
                event => println!("{event:?}"),
            },
            result = tokio::signal::ctrl_c() => { result?; break; }
        }
    }
    spotify.pause().await?;
    spotify.shutdown().await?;
    Ok(())
}

fn arguments(
    args: impl IntoIterator<Item = String>,
) -> Result<(String, Option<String>), &'static str> {
    let mut args = args.into_iter();
    let mut query = None;
    let mut cookie = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sp-dc" => {
                if cookie.is_some() {
                    return Err("--sp-dc supplied more than once");
                }
                let value = args.next().ok_or("--sp-dc requires a cookie value")?;
                if value.trim().is_empty() || value.starts_with("--") {
                    return Err("--sp-dc requires a cookie value");
                }
                cookie = Some(value.trim().to_owned());
            }
            _ if arg.starts_with("--") => return Err("unknown option"),
            _ if query.is_none() => query = Some(arg),
            _ => return Err("supply one quoted search query or Spotify URI"),
        }
    }
    Ok((query.unwrap_or_else(|| "Miles Davis".into()), cookie))
}

#[cfg(test)]
mod tests {
    use super::arguments;

    #[test]
    fn accepts_cookie_before_or_after_query() {
        for args in [
            vec!["--sp-dc", "test-cookie", "spotify:track:test"],
            vec!["spotify:track:test", "--sp-dc", "test-cookie"],
        ] {
            let (query, cookie) = arguments(args.into_iter().map(String::from)).unwrap();
            assert_eq!(query, "spotify:track:test");
            assert_eq!(cookie.as_deref(), Some("test-cookie"));
        }
        assert_eq!(arguments(Vec::new()).unwrap(), ("Miles Davis".into(), None));
    }

    #[test]
    fn rejects_invalid_arguments_without_echoing_values() {
        for args in [
            vec!["--sp-dc"],
            vec!["--sp-dc", " "],
            vec!["--sp-dc", "--other"],
            vec!["--sp-dc", "private-value", "--sp-dc", "other"],
            vec!["--other"],
            vec!["query", "extra"],
        ] {
            let error = arguments(args.into_iter().map(String::from)).unwrap_err();
            assert!(!error.contains("private-value"));
        }
    }
}
