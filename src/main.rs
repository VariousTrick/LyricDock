use ksni::blocking::TrayMethods;
use layer_shika::calloop::TimeoutAction;
use layer_shika::prelude::*;
use layer_shika::slint::SharedString;
use layer_shika::slint_interpreter::Value;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wayland_backend::client::{Backend as WlBackend, ObjectId as WlObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_compositor, wl_region, wl_registry, wl_surface};
use wayland_client::{delegate_noop, Connection as WlConnection, Dispatch, Proxy, QueueHandle};
use zbus::blocking::{Connection as DBusConnection, Proxy as DBusProxy};
use zvariant::OwnedValue;

const SURFACE_NAME: &str = "LyricsOverlay";
const PREVIEW_FILE: &str = "调试面板.json";
const LYRICS_DIR: &str = "lyrics-cache";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct PreviewData {
    locked: Option<bool>,
    panel_width: Option<u32>,
    panel_height: Option<u32>,
    panel_x: Option<i32>,
    panel_y: Option<i32>,
}

impl Default for PreviewData {
    fn default() -> Self {
        Self {
            locked: Some(false),
            panel_width: Some(640),
            panel_height: Some(88),
            panel_x: Some(36),
            panel_y: Some(24),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackInfo {
    title: String,
    album: String,
    artists: Vec<String>,
    duration_ms: u64,
    position_ms: u64,
    playback_status: String,
}

#[derive(Debug, Clone)]
struct LyricLine {
    time_ms: u64,
    text: String,
}

#[derive(Debug, Clone)]
struct ActiveLyrics {
    track_key: String,
    lines: Vec<LyricLine>,
}

#[derive(Debug, Clone, PartialEq)]
struct RenderState {
    locked: bool,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    line_1: String,
    line_2: String,
}

#[derive(Debug, Default)]
struct DragState {
    move_origin: Option<(i32, i32)>,
    resize_origin: Option<(u32, u32)>,
}

#[derive(Debug, Default)]
struct RuntimeState {
    active_lyrics: Option<ActiveLyrics>,
    last_fetch_attempt: Option<(String, Instant)>,
    last_render: Option<RenderState>,
    last_locked: Option<bool>,
    passthrough: Option<WaylandPassthrough>,
}

#[derive(Debug)]
struct MprisClient {
    connection: DBusConnection,
}

impl MprisClient {
    fn new() -> Option<Self> {
        DBusConnection::session().ok().map(|connection| Self { connection })
    }

    fn read_track(&self) -> Option<TrackInfo> {
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

#[derive(Debug)]
struct SomelyricTray {
    locked: bool,
    preview_path: String,
}

impl ksni::Tray for SomelyricTray {
    fn id(&self) -> String {
        "somelyric".into()
    }

    fn title(&self) -> String {
        if self.locked {
            "Somelyric 已锁定".into()
        } else {
            "Somelyric 可编辑".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon()]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Somelyric".into(),
            description: if self.locked {
                "桌面歌词当前处于锁定状态".into()
            } else {
                "桌面歌词当前处于可编辑状态".into()
            },
            icon_name: String::new(),
            icon_pixmap: vec![tray_icon()],
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        vec![
            CheckmarkItem {
                label: "锁定歌词".into(),
                checked: self.locked,
                activate: Box::new(|tray: &mut Self| {
                    tray.locked = !tray.locked;
                    update_preview_file(Path::new(&tray.preview_path), |preview| {
                        preview.locked = Some(tray.locked);
                    });
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "解锁并显示编辑框".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.locked = false;
                    update_preview_file(Path::new(&tray.preview_path), |preview| {
                        preview.locked = Some(false);
                    });
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "退出".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[derive(Debug)]
struct WaylandNoopState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandNoopState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &WlConnection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(WaylandNoopState: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandNoopState: ignore wl_region::WlRegion);
delegate_noop!(WaylandNoopState: ignore wl_surface::WlSurface);

#[derive(Debug)]
struct WaylandPassthrough {
    connection: WlConnection,
    event_queue: wayland_client::EventQueue<WaylandNoopState>,
    queue_handle: QueueHandle<WaylandNoopState>,
    compositor: wl_compositor::WlCompositor,
    surface: wl_surface::WlSurface,
}

impl WaylandPassthrough {
    fn from_component(component: &layer_shika::slint_interpreter::ComponentInstance) -> Option<Self> {
        let window = component.window();
        let window_handle = window.window_handle();
        let display_handle = window.window_handle();

        let (surface_ptr, display_ptr) = match (
            window_handle.window_handle().ok()?.as_raw(),
            display_handle.display_handle().ok()?.as_raw(),
        ) {
            (
                RawWindowHandle::Wayland(window),
                RawDisplayHandle::Wayland(display),
            ) => (window.surface.as_ptr(), display.display.as_ptr()),
            _ => return None,
        };

        let backend = unsafe { WlBackend::from_foreign_display(display_ptr.cast()) };
        let connection = WlConnection::from_backend(backend);
        let (globals, event_queue) = registry_queue_init::<WaylandNoopState>(&connection).ok()?;
        let queue_handle = event_queue.handle();
        let compositor = globals.bind(&queue_handle, 1..=6, ()).ok()?;
        let object_id =
            unsafe { WlObjectId::from_ptr(&wl_surface::WlSurface::interface(), surface_ptr.cast()) }
                .ok()?;
        let surface = wl_surface::WlSurface::from_id(&connection, object_id).ok()?;

        Some(Self {
            connection,
            event_queue,
            queue_handle,
            compositor,
            surface,
        })
    }

    fn set_interactive(&mut self, interactive: bool) {
        if interactive {
            self.surface.set_input_region(None);
        } else {
            let region = self.compositor.create_region(&self.queue_handle, ());
            region.add(0, 0, 0, 0);
            self.surface.set_input_region(Some(&region));
            region.destroy();
        }

        self.surface.commit();
        let _ = self.connection.flush();
        let _ = self.event_queue.dispatch_pending(&mut WaylandNoopState);
    }
}

fn main() -> Result<()> {
    let preview_path = Path::new(PREVIEW_FILE).to_path_buf();
    let initial_preview = read_preview_data(&preview_path);
    let shared_preview = Rc::new(RefCell::new(initial_preview.clone()));
    let drag_state = Rc::new(RefCell::new(DragState::default()));
    let runtime_state = Rc::new(RefCell::new(RuntimeState::default()));
    let mpris_client = Rc::new(MprisClient::new());
    let tray_handle = SomelyricTray {
        locked: initial_preview.locked.unwrap_or(false),
        preview_path: preview_path.to_string_lossy().into_owned(),
    }
    .spawn()
    .ok();

    let mut shell = Shell::from_file("ui/lyrics-overlay.slint")
        .surface(SURFACE_NAME)
        .width(initial_preview.panel_width.unwrap_or(640))
        .height(initial_preview.panel_height.unwrap_or(88))
        .layer(Layer::Overlay)
        .anchor(AnchorEdges::empty().with_top().with_left())
        .exclusive_zone(0)
        .namespace("somelyric-overlay")
        .keyboard_interactivity(KeyboardInteractivity::None)
        .output_policy(OutputPolicy::PrimaryOnly)
        .build()?;

    shell.with_surface(SURFACE_NAME, |component| {
        apply_preview_properties(component, &initial_preview);
        let _ = component.set_property("line_1", shared(Some("正在等待 Spotify...")));
        let _ = component.set_property("line_2", shared(Some("")));
    })?;

    {
        let preview_path_for_lock = preview_path.clone();
        let preview_state = Rc::clone(&shared_preview);
        shell.select(Surface::named(SURFACE_NAME))
            .on_callback("request-lock", move |_| {
                update_preview_file(&preview_path_for_lock, |preview| {
                    preview.locked = Some(true);
                });
                *preview_state.borrow_mut() = read_preview_data(&preview_path_for_lock);
                true
            });
    }

    {
        let preview_path_for_move_2 = preview_path.clone();
        let preview_state = Rc::clone(&shared_preview);
        let preview_state_2 = Rc::clone(&shared_preview);
        let drag_state = Rc::clone(&drag_state);
        let drag_state_2 = Rc::clone(&drag_state);
        shell.select(Surface::named(SURFACE_NAME))
            .on_callback("begin-move", move |_| {
                let preview = preview_state.borrow().clone();
                drag_state.borrow_mut().move_origin = Some((
                    preview.panel_x.unwrap_or(36),
                    preview.panel_y.unwrap_or(24),
                ));
                true
            })
            .on_callback_with_args("request-move", move |args, _| {
                let dx = value_to_i32(args.first());
                let dy = value_to_i32(args.get(1));
                if let Some((origin_x, origin_y)) = drag_state_2.borrow().move_origin {
                    update_preview_file(&preview_path_for_move_2, |preview| {
                        preview.panel_x = Some((origin_x + dx).max(0));
                        preview.panel_y = Some((origin_y + dy).max(0));
                    });
                    *preview_state_2.borrow_mut() = read_preview_data(&preview_path_for_move_2);
                }
                true
            });
    }

    {
        let preview_path_for_resize_2 = preview_path.clone();
        let preview_state = Rc::clone(&shared_preview);
        let preview_state_2 = Rc::clone(&shared_preview);
        let drag_state = Rc::clone(&drag_state);
        let drag_state_2 = Rc::clone(&drag_state);
        shell.select(Surface::named(SURFACE_NAME))
            .on_callback("begin-resize", move |_| {
                let preview = preview_state.borrow().clone();
                drag_state.borrow_mut().resize_origin = Some((
                    preview.panel_width.unwrap_or(640),
                    preview.panel_height.unwrap_or(88),
                ));
                true
            })
            .on_callback_with_args("request-resize", move |args, _| {
                let dw = value_to_i32(args.first());
                let dh = value_to_i32(args.get(1));
                if let Some((origin_w, origin_h)) = drag_state_2.borrow().resize_origin {
                    update_preview_file(&preview_path_for_resize_2, |preview| {
                        preview.panel_width = Some(((origin_w as i32 + dw).max(320)) as u32);
                        preview.panel_height = Some(((origin_h as i32 + dh).max(72)) as u32);
                    });
                    *preview_state_2.borrow_mut() = read_preview_data(&preview_path_for_resize_2);
                }
                true
            });
    }

    let event_loop = shell.event_loop_handle();
    let tray_state = Arc::new(Mutex::new(tray_handle));
    let tray_state_for_timer = Arc::clone(&tray_state);
    let preview_path_for_timer = preview_path.clone();
    let preview_state_for_timer = Rc::clone(&shared_preview);
    let runtime_state_for_timer = Rc::clone(&runtime_state);
    let mpris_for_timer = Rc::clone(&mpris_client);

    event_loop.add_timer(Duration::from_millis(200), move |_, app_state| {
        let preview = read_preview_data(&preview_path_for_timer);
        *preview_state_for_timer.borrow_mut() = preview.clone();

        let mut runtime = runtime_state_for_timer.borrow_mut();
        let playback = mpris_for_timer
            .as_ref()
            .as_ref()
            .and_then(|client| client.read_track());
        let text_pair = current_lines_for_track(
            playback.as_ref(),
            &mut runtime,
            Path::new(LYRICS_DIR),
            Path::new("scripts/fetch-current-song-lyrics.js"),
        );

        let render = RenderState {
            locked: preview.locked.unwrap_or(false),
            width: preview.panel_width.unwrap_or(640),
            height: preview.panel_height.unwrap_or(88),
            x: preview.panel_x.unwrap_or(36).max(0),
            y: preview.panel_y.unwrap_or(24).max(0),
            line_1: text_pair.0,
            line_2: text_pair.1,
        };

        let needs_passthrough_update = runtime.last_locked != Some(render.locked);

        if runtime.last_render.as_ref() != Some(&render) {
            for surface in app_state.surfaces_by_name_mut(SURFACE_NAME) {
                let component = surface.component_instance();

                let _ = component.set_property("locked", Value::Bool(render.locked));
                let _ = component.set_property("panel_width", Value::Number(render.width as f64));
                let _ = component.set_property("panel_height", Value::Number(render.height as f64));
                let _ = component.set_property("line_1", shared(Some(&render.line_1)));
                let _ = component.set_property("line_2", shared(Some(&render.line_2)));

                surface.layer_surface().set_size(render.width, render.height);
                surface
                    .layer_surface()
                    .set_margin(render.y, 0, 0, render.x);
                surface.commit_surface();

                if needs_passthrough_update {
                    if runtime.passthrough.is_none() {
                        runtime.passthrough = WaylandPassthrough::from_component(component);
                    }
                    if let Some(passthrough) = runtime.passthrough.as_mut() {
                        passthrough.set_interactive(!render.locked);
                    }
                }
            }

            if let Ok(guard) = tray_state_for_timer.lock() {
                if let Some(handle) = guard.as_ref() {
                    let _ = handle.update(|tray: &mut SomelyricTray| {
                        tray.locked = render.locked;
                    });
                }
            }

            runtime.last_render = Some(render.clone());
        } else if needs_passthrough_update {
            for surface in app_state.surfaces_by_name_mut(SURFACE_NAME) {
                let component = surface.component_instance();
                if runtime.passthrough.is_none() {
                    runtime.passthrough = WaylandPassthrough::from_component(component);
                }
            }
            if let Some(passthrough) = runtime.passthrough.as_mut() {
                passthrough.set_interactive(!render.locked);
            }
        }

        runtime.last_locked = Some(render.locked);
        TimeoutAction::ToDuration(Duration::from_millis(200))
    })?;

    shell.select(Surface::named(SURFACE_NAME)).configure(|_, surface| {
        surface.set_layer(Layer::Overlay);
        surface.set_anchor_edges(AnchorEdges::empty().with_top().with_left());
        surface.set_exclusive_zone(0);
        surface.set_size(
            initial_preview.panel_width.unwrap_or(640),
            initial_preview.panel_height.unwrap_or(88),
        );
        surface.set_margins(Margins::new(
            initial_preview.panel_y.unwrap_or(24),
            0,
            0,
            initial_preview.panel_x.unwrap_or(36),
        ));
        surface.commit();
    });

    shell.run()
}

fn read_preview_data(path: &Path) -> PreviewData {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<PreviewData>(&content).ok())
        .unwrap_or_default()
}

fn update_preview_file(path: &Path, updater: impl FnOnce(&mut PreviewData)) {
    let mut preview = read_preview_data(path);
    updater(&mut preview);
    if let Ok(content) = serde_json::to_string_pretty(&preview) {
        let _ = fs::write(path, content);
    }
}

fn apply_preview_properties(
    component: &layer_shika::slint_interpreter::ComponentInstance,
    preview: &PreviewData,
) {
    let _ = component.set_property(
        "locked",
        Value::Bool(preview.locked.unwrap_or(false)),
    );
    let _ = component.set_property(
        "panel_width",
        Value::Number(preview.panel_width.unwrap_or(640) as f64),
    );
    let _ = component.set_property(
        "panel_height",
        Value::Number(preview.panel_height.unwrap_or(88) as f64),
    );
}

fn current_lines_for_track(
    playback: Option<&TrackInfo>,
    runtime: &mut RuntimeState,
    lyrics_dir: &Path,
    fetch_script: &Path,
) -> (String, String) {
    let Some(track) = playback else {
        runtime.active_lyrics = None;
        return ("正在等待 Spotify...".into(), "".into());
    };

    let track_key = track_cache_key(track);
    let should_reload = runtime
        .active_lyrics
        .as_ref()
        .map(|lyrics| lyrics.track_key != track_key)
        .unwrap_or(true);

    if should_reload {
        runtime.active_lyrics = load_or_fetch_lyrics(track, lyrics_dir, fetch_script, runtime)
            .map(|lines| ActiveLyrics {
                track_key: track_key.clone(),
                lines,
            });
    }

    if let Some(active) = runtime.active_lyrics.as_ref() {
        return lyric_pair_for_position(&active.lines, track.position_ms);
    }

    let artist = track.artists.first().cloned().unwrap_or_default();
    ("未找到歌词".into(), format!("{} - {}", track.title, artist))
}

fn load_or_fetch_lyrics(
    track: &TrackInfo,
    lyrics_dir: &Path,
    fetch_script: &Path,
    runtime: &mut RuntimeState,
) -> Option<Vec<LyricLine>> {
    if let Some(path) = find_cached_lyric_path(track, lyrics_dir) {
        return read_lrc_file(&path);
    }

    let key = track_cache_key(track);
    let should_fetch = runtime
        .last_fetch_attempt
        .as_ref()
        .map(|(last_key, last_when)| last_key != &key || last_when.elapsed() > Duration::from_secs(8))
        .unwrap_or(true);

    if should_fetch {
        let _ = Command::new("node")
            .arg(fetch_script)
            .current_dir(Path::new("."))
            .status();
        runtime.last_fetch_attempt = Some((key.clone(), Instant::now()));
    }

    find_cached_lyric_path(track, lyrics_dir).and_then(|path| read_lrc_file(&path))
}

fn find_cached_lyric_path(track: &TrackInfo, lyrics_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(lyrics_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let content = fs::read_to_string(&path).ok()?;
        let cached: CachedLyricMeta = serde_json::from_str(&content).ok()?;
        let same_title = normalize_for_match(&cached.track.title) == normalize_for_match(&track.title);
        let same_artist = cached
            .track
            .artists
            .first()
            .map(|artist| normalize_for_match(artist))
            == track.artists.first().map(|artist| normalize_for_match(artist));

        if same_title && same_artist {
            let mut lrc_path = path.clone();
            lrc_path.set_extension("lrc");
            if lrc_path.exists() {
                return Some(lrc_path);
            }
        }
    }

    None
}

fn read_lrc_file(path: &Path) -> Option<Vec<LyricLine>> {
    let content = fs::read_to_string(path).ok()?;
    let lines = parse_lrc(&content);
    (!lines.is_empty()).then_some(lines)
}

fn parse_lrc(content: &str) -> Vec<LyricLine> {
    let mut parsed = Vec::new();

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut rest = trimmed;
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else {
                break;
            };
            let tag = &stripped[..end];
            if let Some(ms) = parse_timestamp(tag) {
                times.push(ms);
                rest = &stripped[end + 1..];
            } else {
                break;
            }
        }

        let text = rest.trim();
        if times.is_empty() || text.is_empty() || is_credit_line(text) {
            continue;
        }

        for time_ms in times {
            parsed.push(LyricLine {
                time_ms,
                text: text.to_string(),
            });
        }
    }

    parsed.sort_by_key(|line| line.time_ms);
    parsed
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let mut parts = tag.split(':');
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds_part = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let mut seconds_split = seconds_part.split('.');
    let seconds = seconds_split.next()?.parse::<u64>().ok()?;
    let millis_text = seconds_split.next().unwrap_or("0");
    let millis = match millis_text.len() {
        0 => 0,
        1 => millis_text.parse::<u64>().ok()? * 100,
        2 => millis_text.parse::<u64>().ok()? * 10,
        _ => millis_text.get(..3)?.parse::<u64>().ok()?,
    };

    Some(minutes * 60_000 + seconds * 1_000 + millis)
}

fn lyric_pair_for_position(lines: &[LyricLine], position_ms: u64) -> (String, String) {
    if lines.is_empty() {
        return ("未找到歌词".into(), "".into());
    }

    let current_index = lines
        .iter()
        .rposition(|line| line.time_ms <= position_ms)
        .unwrap_or(0);
    let current = lines.get(current_index).map(|line| line.text.clone()).unwrap_or_default();
    let next = lines
        .get(current_index + 1)
        .map(|line| line.text.clone())
        .unwrap_or_default();

    (current, next)
}

fn is_credit_line(text: &str) -> bool {
    ["作词", "作曲", "编曲", "制作人", "监制"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

fn normalize_for_match(text: &str) -> String {
    text
        .replace('連', "连")
        .replace('藉', "借")
        .replace('舊', "旧")
        .replace('沒', "没")
        .replace('（', "(")
        .replace('）', ")")
        .replace('　', " ")
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn track_cache_key(track: &TrackInfo) -> String {
    format!(
        "{}::{}",
        normalize_for_match(&track.title),
        track
            .artists
            .first()
            .map(|artist| normalize_for_match(artist))
            .unwrap_or_default()
    )
}

fn value_to_i32(value: Option<&Value>) -> i32 {
    match value {
        Some(Value::Number(number)) => number.round() as i32,
        _ => 0,
    }
}

fn shared(text: Option<&str>) -> Value {
    Value::from(SharedString::from(text.unwrap_or_default()))
}

fn tray_icon() -> ksni::Icon {
    let width = 22_i32;
    let height = 22_i32;
    let mut data = vec![0_u8; (width * height * 4) as usize];

    let bg = [0xFF, 0xFF, 0xFF, 0xFF];
    let fill = [0xFF, 0x2C, 0x2C, 0x2C];

    let shape = [
        "......................",
        "..##############......",
        "..##############......",
        "..##..........##......",
        "..##..........##......",
        "..##..........##..###.",
        "..##..........##.#####",
        "..##..#####...##.#####",
        "..##..#####...##..###.",
        "..##..##......##......",
        "..##..##......##......",
        "..##..##......##......",
        "..##..##......##......",
        "..##..##########......",
        "..##..##########......",
        "..##..................",
        "..##..................",
        "..################....",
        "..################....",
        "......................",
        "......................",
        "......................",
    ];

    for (y, row) in shape.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            let idx = ((y as i32 * width + x as i32) * 4) as usize;
            if ch == '#' {
                data[idx..idx + 4].copy_from_slice(&fill);
            } else if ch == '.' && x >= 2 && x <= 17 && y >= 1 && y <= 18 {
                data[idx..idx + 4].copy_from_slice(&bg);
            }
        }
    }

    ksni::Icon { width, height, data }
}

#[derive(Debug, Deserialize)]
struct CachedLyricMeta {
    track: CachedTrack,
}

#[derive(Debug, Deserialize)]
struct CachedTrack {
    title: String,
    artists: Vec<String>,
}
