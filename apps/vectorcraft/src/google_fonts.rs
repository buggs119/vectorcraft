//! The downloader `text.googleFonts` and `text.installGoogleFont` use
//! (`vectorcraft_engine::cmd::googlefonts::set_fetcher`): an HTTPS GET with a time limit, the body
//! capped.

use std::io::Read;
use std::time::Duration;

/// Install the downloader. Before the first frame.
pub fn install() {
    vectorcraft_engine::cmd::googlefonts::set_fetcher(fetch);
}

fn fetch(url: &str, max: u64) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).build().into();
    // GitHub's API asks every client for a User-Agent.
    let mut response = agent
        .get(url)
        .header("User-Agent", concat!("VectorCraft/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("couldn't download {url}: {e}"))?;
    let mut body = vec![];
    response.body_mut().as_reader().take(max.saturating_add(1)).read_to_end(&mut body).map_err(|e| format!("couldn't download {url}: {e}"))?;
    if body.len() as u64 > max {
        return Err(format!("{url} is larger than {} MB", max >> 20));
    }
    Ok(body)
}
