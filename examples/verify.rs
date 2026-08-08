use anyhow::Result;
use climusic::config::Config;
use climusic::player::mpv::MpvPlayer;
use climusic::sources::soundcloud::SoundCloudSource;
use climusic::sources::youtube::YouTubeSource;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("Config path: {:?}", Config::config_path()?);
    let config = Config::load()?;
    println!("mpv_path from config: {}", config.player.mpv_path);
    let mut player = MpvPlayer::new(&config.player.mpv_path, config.player.audio_exclusive);
    println!("Starting mpv...");
    player.start().await?;
    println!("mpv started.");
    player.set_volume(30).await?;
    println!("Volume set.");

    // Test 1: local file playback (skip if no test audio is present)
    let local_path = "test_audio.mp3";
    if std::path::Path::new(local_path).exists() {
        println!("Playing local file: {}", local_path);
        player.load(local_path, false).await?;
        sleep(Duration::from_secs(3)).await;
    } else {
        println!("Skipping local file test ({} not found)", local_path);
    }

    // Test 2: YouTube playback (public domain / Creative Commons)
    let mut youtube = YouTubeSource::new(&config.player.yt_dlp_path, &config.player.cookies_from_browser);
    let video_url = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
    println!("Resolving YouTube audio URL: {video_url}");
    let audio_url = youtube.get_audio_url(video_url).await?;
    println!("YouTube audio URL resolved. Playing...");
    player.load(&audio_url, false).await?;
    sleep(Duration::from_secs(5)).await;

    // Test 3: SoundCloud playback
    let soundcloud_url = "https://soundcloud.com/tien-pham-418156952/she-neva-know-lofi-ver";
    let mut soundcloud = SoundCloudSource::new(&config.player.yt_dlp_path, &config.player.cookies_from_browser);
    println!("Resolving SoundCloud audio URL: {soundcloud_url}");
    let sc_audio_url = soundcloud.get_audio_url(soundcloud_url).await?;
    println!("SoundCloud audio URL resolved. Playing...");
    player.load(&sc_audio_url, false).await?;
    sleep(Duration::from_secs(5)).await;

    // Verify caching works: second call should be instant
    let start = std::time::Instant::now();
    let cached_url = youtube.get_audio_url(video_url).await?;
    println!("Cached URL resolved in {:?}: {}", start.elapsed(), cached_url);

    player.stop().await?;
    println!("Playback verification complete.");
    Ok(())
}
