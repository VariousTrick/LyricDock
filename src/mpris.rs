use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zbus::blocking::{Connection as DBusConnection, Proxy as DBusProxy};
use zvariant::OwnedValue;

/// 当前播放曲目的统一描述。
/// Rust 主程序和拉词脚本都围绕这份结构工作，避免出现两个事实来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub title: String,
    pub album: String,
    pub artists: Vec<String>,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub playback_status: String,
}

/// 负责从 Spotify 的 MPRIS 服务读取当前播放信息。
pub struct MprisClient {
    connection: DBusConnection,
}

impl MprisClient {
    pub fn new() -> Option<Self> {
        DBusConnection::session().ok().map(|connection| Self { connection })
    }

    pub fn read_track(&self) -> Option<TrackInfo> {
        let proxy = DBusProxy::new(
            &self.connection,
            "org.mpris.MediaPlayer2.spotify",
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
        )
        .ok()?;

        let metadata: HashMap<String, OwnedValue> = proxy.get_property("Metadata").ok()?;
        let title = metadata
            .get("xesam:title")
            .and_then(|value| String::try_from(value.clone()).ok())?;
        let album = metadata
            .get("xesam:album")
            .and_then(|value| String::try_from(value.clone()).ok())
            .unwrap_or_default();
        let artists = metadata
            .get("xesam:artist")
            .and_then(|value| Vec::<String>::try_from(value.clone()).ok())
            .filter(|items| !items.is_empty())?;
        let duration_us = metadata
            .get("mpris:length")
            .and_then(|value| u64::try_from(value.clone()).ok())
            .unwrap_or(0);
        let position_us: i64 = proxy.get_property("Position").ok().unwrap_or(0);
        let playback_status: String = proxy
            .get_property("PlaybackStatus")
            .ok()
            .unwrap_or_else(|| "Stopped".into());

        Some(TrackInfo {
            title,
            album,
            artists,
            duration_ms: duration_us / 1_000,
            position_ms: position_us.max(0) as u64 / 1_000,
            playback_status,
        })
    }
}
