use crate::lyrics::parser::{parse_lrc, parse_yrc, LyricLine};
use std::fs;
use std::path::Path;

/// 读取歌词文件。
/// 如果启用了 karaoke，并且存在同名 `.yrc`，则优先读取逐字歌词；
/// 否则回退到普通 `.lrc` 逐行歌词。
pub fn read_lyric_file(path: &Path, enable_karaoke: bool) -> Option<Vec<LyricLine>> {
    if enable_karaoke {
        let mut yrc_path = path.to_path_buf();
        yrc_path.set_extension("yrc");
        if yrc_path.exists() {
            if let Ok(content) = fs::read_to_string(&yrc_path) {
                let lines = parse_yrc(&content);
                if !lines.is_empty() {
                    return Some(lines);
                }
            }
        }
    }

    let content = fs::read_to_string(path).ok()?;
    let lines = parse_lrc(&content);
    (!lines.is_empty()).then_some(lines)
}
