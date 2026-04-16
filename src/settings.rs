use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 主配置文件中可选的原始字段。
/// 这里保持 Option，方便缺省时回退到程序默认值。
#[derive(Debug, Clone, Deserialize)]
pub struct AppSettingsFile {
    pub lyrics_dir: Option<String>,
    pub cache_limit_mb: Option<u64>,
    pub show_secondary_line: Option<bool>,
    pub use_gradient: Option<bool>,
    pub lyric_effect: Option<String>,
    pub font_family: Option<String>,
    pub highlight_color: Option<String>,
    pub base_color: Option<String>,
    pub preview_color: Option<String>,
    pub stroke_color: Option<String>,
    pub stroke_width: Option<f32>,
    pub shadow_color: Option<String>,
    pub panel_background_color: Option<String>,
    pub panel_border_color: Option<String>,
    pub resize_handle_color: Option<String>,
    pub lyrics_opacity: Option<f32>,
    pub preview_opacity: Option<f32>,
}

/// 程序运行时使用的完整配置。
/// 所有字段在这里都已经补齐默认值，可以直接使用。
#[derive(Debug, Clone)]
pub struct AppSettings {
    pub lyrics_root: PathBuf,
    pub imported_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub cache_limit_bytes: u64,
    pub show_secondary_line: bool,
    pub use_gradient: bool,
    pub lyric_effect: String,
    pub font_family: String,
    pub highlight_color: String,
    pub base_color: String,
    pub preview_color: String,
    pub stroke_color: String,
    pub stroke_width: f32,
    pub shadow_color: String,
    pub panel_background_color: String,
    pub panel_border_color: String,
    pub resize_handle_color: String,
    pub lyrics_opacity: f32,
    pub preview_opacity: f32,
    pub config_path: PathBuf,
}

impl AppSettings {
    pub fn from_file(config_path: PathBuf) -> Self {
        ensure_settings_file(&config_path);
        let parsed = fs::read_to_string(&config_path)
            .ok()
            .and_then(|content| toml::from_str::<AppSettingsFile>(&content).ok());
        let base_dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let lyrics_dir = parsed
            .as_ref()
            .and_then(|settings| settings.lyrics_dir.as_deref())
            .unwrap_or("./歌词数据");
        let lyrics_root = resolve_config_path(&base_dir, lyrics_dir);
        let imported_dir = lyrics_root.join("导入歌词");
        let cache_dir = lyrics_root.join("缓存歌词");
        let cache_limit_mb = parsed
            .as_ref()
            .and_then(|settings| settings.cache_limit_mb)
            .unwrap_or(256);
        let show_secondary_line = parsed
            .as_ref()
            .and_then(|settings| settings.show_secondary_line)
            .unwrap_or(true);
        let use_gradient = parsed
            .as_ref()
            .and_then(|settings| settings.use_gradient)
            .unwrap_or(false);
        let lyric_effect = parsed
            .as_ref()
            .and_then(|settings| settings.lyric_effect.clone())
            .unwrap_or_else(|| "flat".to_string());
        let lyric_effect = if lyric_effect.eq_ignore_ascii_case("floating") {
            "floating".to_string()
        } else {
            "flat".to_string()
        };
        let font_family = parsed
            .as_ref()
            .and_then(|settings| settings.font_family.clone())
            .unwrap_or_else(|| {
                "Noto Sans CJK SC, Source Han Sans SC, Noto Sans, sans-serif".to_string()
            });
        let highlight_color = parsed
            .as_ref()
            .and_then(|settings| settings.highlight_color.clone())
            .unwrap_or_else(|| "#00e676".to_string());
        let base_color = parsed
            .as_ref()
            .and_then(|settings| settings.base_color.clone())
            .unwrap_or_else(|| "#f5f7fb".to_string());
        let preview_color = parsed
            .as_ref()
            .and_then(|settings| settings.preview_color.clone())
            .unwrap_or_else(|| "#f5f7fb".to_string());
        let stroke_color = parsed
            .as_ref()
            .and_then(|settings| settings.stroke_color.clone())
            .unwrap_or_else(|| "#081019e0".to_string());
        let stroke_width = parsed
            .as_ref()
            .and_then(|settings| settings.stroke_width)
            .unwrap_or(3.2)
            .clamp(1.2, 6.0);
        let shadow_color = parsed
            .as_ref()
            .and_then(|settings| settings.shadow_color.clone())
            .unwrap_or_else(|| "#000000c4".to_string());
        let panel_background_color = parsed
            .as_ref()
            .and_then(|settings| settings.panel_background_color.clone())
            .unwrap_or_else(|| "#00000095".to_string());
        let panel_border_color = parsed
            .as_ref()
            .and_then(|settings| settings.panel_border_color.clone())
            .unwrap_or_else(|| "#ffffff28".to_string());
        let resize_handle_color = parsed
            .as_ref()
            .and_then(|settings| settings.resize_handle_color.clone())
            .unwrap_or_else(|| "#ffffffa8".to_string());
        let lyrics_opacity = parsed
            .as_ref()
            .and_then(|settings| settings.lyrics_opacity)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let preview_opacity = parsed
            .as_ref()
            .and_then(|settings| settings.preview_opacity)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        let _ = fs::create_dir_all(&imported_dir);
        let _ = fs::create_dir_all(&cache_dir);

        Self {
            lyrics_root,
            imported_dir,
            cache_dir,
            cache_limit_bytes: cache_limit_mb.saturating_mul(1024 * 1024),
            show_secondary_line,
            use_gradient,
            lyric_effect,
            font_family,
            highlight_color,
            base_color,
            preview_color,
            stroke_color,
            stroke_width,
            shadow_color,
            panel_background_color,
            panel_border_color,
            resize_handle_color,
            lyrics_opacity,
            preview_opacity,
            config_path,
        }
    }
}

/// 本地窗口状态文件。
/// 这里只存“运行后会变化”的状态，例如位置、大小、字体缩放。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PreviewData {
    pub locked: Option<bool>,
    pub panel_width: Option<u32>,
    pub panel_height: Option<u32>,
    pub panel_x: Option<i32>,
    pub panel_y: Option<i32>,
    pub font_scale: Option<i32>,
}

impl Default for PreviewData {
    fn default() -> Self {
        Self {
            locked: Some(false),
            panel_width: Some(640),
            panel_height: Some(112),
            panel_x: Some(36),
            panel_y: Some(24),
            font_scale: Some(0),
        }
    }
}

pub fn read_preview_data(path: &Path) -> PreviewData {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<PreviewData>(&content).ok())
        .unwrap_or_default()
}

pub fn resolve_window_state_path(current_name: &str, legacy_name: &str) -> PathBuf {
    let current = Path::new(current_name).to_path_buf();
    if current.exists() {
        return current;
    }

    let legacy = Path::new(legacy_name).to_path_buf();
    if legacy.exists() {
        let _ = fs::rename(&legacy, &current);
        if current.exists() {
            return current;
        }
    }

    current
}

pub fn ensure_preview_file(path: &Path) {
    if path.exists() {
        return;
    }

    let preview = PreviewData::default();
    write_preview_data(path, &preview);
}

pub fn write_preview_data(path: &Path, preview: &PreviewData) {
    if let Ok(content) = serde_json::to_string_pretty(preview) {
        let _ = fs::write(path, content);
    }
}

pub fn ensure_settings_file(path: &Path) {
    if path.exists() {
        return;
    }

    let default_content = include_str!("../配置.toml");
    let _ = fs::write(path, default_content);
}

pub fn resolve_config_path(base_dir: &Path, configured: &str) -> PathBuf {
    let configured_path = PathBuf::from(configured);
    if configured_path.is_absolute() {
        configured_path
    } else {
        base_dir.join(configured_path)
    }
}
