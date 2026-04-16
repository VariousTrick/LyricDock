mod lyrics;
mod mpris;
mod settings;

use crate::lyrics::parser::{line_progress, parse_lrc, parse_yrc, LyricLine};
use crate::mpris::{MprisClient, TrackInfo};
use crate::settings::{
    ensure_preview_file, read_preview_data, resolve_window_state_path, write_preview_data,
    AppSettings, PreviewData,
};
use ksni::blocking::TrayMethods;
use layer_shika_adapters::SurfaceState;
use layer_shika::calloop::TimeoutAction;
use layer_shika::prelude::*;
use layer_shika::slint::{Brush, Color, ModelRc, SharedString, VecModel};
use layer_shika::slint_interpreter::{Struct, Value};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_compositor, wl_region, wl_registry, wl_surface};
use wayland_client::{delegate_noop, Connection as WlConnection, Dispatch, QueueHandle};

const SURFACE_NAME: &str = "LyricsOverlay";
const WINDOW_STATE_FILE: &str = "窗口状态.json";
const LEGACY_PREVIEW_FILE: &str = "调试面板.json";
const SETTINGS_FILE: &str = "配置.toml";
const LEGACY_LYRICS_DIR: &str = "lyrics-cache";

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
    line_1_segments: Vec<LyricRenderSegment>,
    line_2_segments: Vec<LyricRenderSegment>,
    progress_1: f32,
    progress_2: f32,
    line_1_active: bool,
    line_2_active: bool,
    scroll_1: f32,
    scroll_2: f32,
    offset_1: f32,
    offset_2: f32,
    font_scale: i32,
    show_secondary_line: bool,
    use_gradient: bool,
    lyric_effect: String,
    font_family: String,
    highlight_color: String,
    base_color: String,
    preview_color: String,
    stroke_color: String,
    stroke_width: f32,
    shadow_color: String,
    panel_background_color: String,
    panel_border_color: String,
    resize_handle_color: String,
    lyrics_opacity: f32,
    preview_opacity: f32,
}

#[derive(Debug, Clone)]
struct LyricRenderPair {
    line_1: String,
    line_2: String,
    line_1_segments: Vec<LyricRenderSegment>,
    line_2_segments: Vec<LyricRenderSegment>,
    progress_1: f32,
    progress_2: f32,
    line_1_active: bool,
    line_2_active: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct LyricRenderSegment {
    text: String,
    progress: f32,
}

#[derive(Debug, Default)]
struct DragState {
    move_origin: Option<(i32, i32)>,
    resize_origin: Option<(u32, u32)>,
}

#[derive(Debug, Default, Clone)]
struct InteractionState {
    dragging: bool,
    resizing: bool,
    dirty: bool,
}

#[derive(Debug, Default)]
struct RuntimeState {
    active_lyrics: Option<ActiveLyrics>,
    last_fetch_attempt: Option<(String, Instant)>,
    last_track: Option<TrackInfo>,
    last_track_poll: Option<Instant>,
    last_render: Option<RenderState>,
    last_locked: Option<bool>,
    passthrough: Option<WaylandPassthrough>,
}


#[derive(Debug)]
struct LyricDockTray {
    locked: bool,
    preview_path: String,
    preview_state: Arc<Mutex<PreviewData>>,
    config_path: String,
    lyrics_root: String,
    cache_dir: String,
}

impl ksni::Tray for LyricDockTray {
    fn id(&self) -> String {
        "lyricdock".into()
    }

    fn title(&self) -> String {
        if self.locked {
            "LyricDock 已锁定".into()
        } else {
            "LyricDock 可编辑".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon()]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "LyricDock".into(),
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
                    if let Ok(mut preview) = tray.preview_state.lock() {
                        preview.locked = Some(tray.locked);
                        write_preview_data(Path::new(&tray.preview_path), &preview);
                    }
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "打开配置文件".into(),
                icon_name: "document-open".into(),
                activate: Box::new(|tray: &mut Self| {
                    open_path(Path::new(&tray.config_path));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "打开歌词目录".into(),
                icon_name: "folder-music".into(),
                activate: Box::new(|tray: &mut Self| {
                    open_path(Path::new(&tray.lyrics_root));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "清理缓存歌词".into(),
                icon_name: "edit-clear".into(),
                activate: Box::new(|tray: &mut Self| {
                    clear_cache_dir(Path::new(&tray.cache_dir));
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
    fn from_surface_state(surface_state: &SurfaceState) -> Option<Self> {
        let connection: WlConnection = surface_state.surface().connection().as_ref().clone();
        let (globals, event_queue) = match registry_queue_init::<WaylandNoopState>(&connection) {
            Ok(value) => value,
            Err(_) => return None,
        };
        let queue_handle = event_queue.handle();
        let compositor = match globals.bind(&queue_handle, 1..=6, ()) {
            Ok(value) => value,
            Err(_) => return None,
        };
        let surface: wl_surface::WlSurface = surface_state.surface().inner().as_ref().clone();

        Some(Self {
            connection,
            event_queue,
            queue_handle,
            compositor,
            surface,
        })
    }

    fn set_interactive(&mut self, interactive: bool) {
        let region = self.compositor.create_region(&self.queue_handle, ());
        if interactive {
            region.add(0, 0, i32::MAX, i32::MAX);
        } else {
        }
        self.surface.set_input_region(Some(&region));
        region.destroy();

        self.surface.commit();
        let _ = self.connection.flush();
        let _ = self.event_queue.roundtrip(&mut WaylandNoopState);
    }
}

fn main() -> Result<()> {
    let settings = AppSettings::from_file(Path::new(SETTINGS_FILE).to_path_buf());
    let preview_path = resolve_window_state_path(WINDOW_STATE_FILE, LEGACY_PREVIEW_FILE);
    ensure_preview_file(&preview_path);
    let initial_preview = read_preview_data(&preview_path);
    let shared_preview = Rc::new(RefCell::new(initial_preview.clone()));
    let tray_preview_state = Arc::new(Mutex::new(initial_preview.clone()));
    let drag_state = Rc::new(RefCell::new(DragState::default()));
    let interaction_state = Rc::new(RefCell::new(InteractionState::default()));
    let runtime_state = Rc::new(RefCell::new(RuntimeState::default()));
    let mpris_client = Rc::new(MprisClient::new());
    let tray_handle = LyricDockTray {
        locked: initial_preview.locked.unwrap_or(false),
        preview_path: preview_path.to_string_lossy().into_owned(),
        preview_state: Arc::clone(&tray_preview_state),
        config_path: settings.config_path.to_string_lossy().into_owned(),
        lyrics_root: settings.lyrics_root.to_string_lossy().into_owned(),
        cache_dir: settings.cache_dir.to_string_lossy().into_owned(),
    }
    .spawn()
    .ok();

    let mut shell = Shell::from_file("ui/lyrics-overlay.slint")
        .surface(SURFACE_NAME)
        .width(initial_preview.panel_width.unwrap_or(640))
        .height(initial_preview.panel_height.unwrap_or(112))
        .layer(Layer::Overlay)
        .anchor(AnchorEdges::empty().with_top().with_left())
        .exclusive_zone(0)
        .namespace("lyricdock-overlay")
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
        let tray_preview_state = Arc::clone(&tray_preview_state);
        shell.select(Surface::named(SURFACE_NAME))
            .on_callback("request-lock", move |_| {
                let mut preview = preview_state.borrow_mut();
                let next_locked = !preview.locked.unwrap_or(false);
                preview.locked = Some(next_locked);
                if let Ok(mut tray_preview) = tray_preview_state.lock() {
                    *tray_preview = preview.clone();
                }
                write_preview_data(&preview_path_for_lock, &preview);
                true
            });
    }

    {
        let preview_path_for_font = preview_path.clone();
        let preview_state = Rc::clone(&shared_preview);
        let tray_preview_state = Arc::clone(&tray_preview_state);
        shell.select(Surface::named(SURFACE_NAME))
            .on_callback_with_args("request-font-step", move |args, _| {
                let step = value_to_i32(args.first());
                let mut preview = preview_state.borrow_mut();
                let current = preview.font_scale.unwrap_or(0);
                preview.font_scale = Some((current + step).clamp(-8, 18));
                if let Ok(mut tray_preview) = tray_preview_state.lock() {
                    *tray_preview = preview.clone();
                }
                write_preview_data(&preview_path_for_font, &preview);
                true
            });
    }

    {
        let preview_path_for_move_2 = preview_path.clone();
        let preview_state = Rc::clone(&shared_preview);
        let preview_state_2 = Rc::clone(&shared_preview);
        let preview_state_3 = Rc::clone(&shared_preview);
        let tray_preview_state = Arc::clone(&tray_preview_state);
        let drag_state = Rc::clone(&drag_state);
        let drag_state_2 = Rc::clone(&drag_state);
        let drag_state_3 = Rc::clone(&drag_state);
        let interaction_state_begin = Rc::clone(&interaction_state);
        let interaction_state_move = Rc::clone(&interaction_state);
        let interaction_state_end = Rc::clone(&interaction_state);
        shell.select(Surface::named(SURFACE_NAME))
            .on_callback("begin-move", move |_| {
                let preview = preview_state.borrow().clone();
                drag_state.borrow_mut().move_origin = Some((
                    preview.panel_x.unwrap_or(36),
                    preview.panel_y.unwrap_or(24),
                ));
                let mut interaction = interaction_state_begin.borrow_mut();
                interaction.dragging = true;
                interaction.dirty = true;
                true
            })
            .on_callback_with_args("request-move", move |args, _| {
                let dx = value_to_i32(args.first());
                let dy = value_to_i32(args.get(1));
                if let Some((origin_x, origin_y)) = drag_state_2.borrow().move_origin {
                    let mut preview = preview_state_2.borrow_mut();
                    preview.panel_x = Some((origin_x + dx).max(0));
                    preview.panel_y = Some((origin_y + dy).max(0));
                    if let Ok(mut tray_preview) = tray_preview_state.lock() {
                        *tray_preview = preview.clone();
                    }
                }
                interaction_state_move.borrow_mut().dirty = true;
                true
            })
            .on_callback("end-move", move |_| {
                let mut drag = drag_state_3.borrow_mut();
                drag.move_origin = None;
                let preview = preview_state_3.borrow().clone();
                write_preview_data(&preview_path_for_move_2, &preview);
                let mut interaction = interaction_state_end.borrow_mut();
                interaction.dragging = false;
                interaction.dirty = true;
                true
            });
    }

    {
        let preview_path_for_resize_2 = preview_path.clone();
        let preview_state = Rc::clone(&shared_preview);
        let preview_state_2 = Rc::clone(&shared_preview);
        let preview_state_3 = Rc::clone(&shared_preview);
        let tray_preview_state = Arc::clone(&tray_preview_state);
        let drag_state = Rc::clone(&drag_state);
        let drag_state_2 = Rc::clone(&drag_state);
        let drag_state_3 = Rc::clone(&drag_state);
        let interaction_state_begin = Rc::clone(&interaction_state);
        let interaction_state_resize = Rc::clone(&interaction_state);
        let interaction_state_end = Rc::clone(&interaction_state);
        shell.select(Surface::named(SURFACE_NAME))
            .on_callback("begin-resize", move |_| {
                let preview = preview_state.borrow().clone();
                drag_state.borrow_mut().resize_origin = Some((
                    preview.panel_width.unwrap_or(640),
                    preview.panel_height.unwrap_or(112),
                ));
                let mut interaction = interaction_state_begin.borrow_mut();
                interaction.resizing = true;
                interaction.dirty = true;
                true
            })
            .on_callback_with_args("request-resize", move |args, _| {
                let dw = value_to_i32(args.first());
                let dh = value_to_i32(args.get(1));
                if let Some((origin_w, origin_h)) = drag_state_2.borrow().resize_origin {
                    let mut preview = preview_state_2.borrow_mut();
                    preview.panel_width = Some(((origin_w as i32 + dw).max(320)) as u32);
                    preview.panel_height = Some(((origin_h as i32 + dh).max(96)) as u32);
                    if let Ok(mut tray_preview) = tray_preview_state.lock() {
                        *tray_preview = preview.clone();
                    }
                }
                interaction_state_resize.borrow_mut().dirty = true;
                true
            })
            .on_callback("end-resize", move |_| {
                let mut drag = drag_state_3.borrow_mut();
                drag.resize_origin = None;
                let preview = preview_state_3.borrow().clone();
                write_preview_data(&preview_path_for_resize_2, &preview);
                let mut interaction = interaction_state_end.borrow_mut();
                interaction.resizing = false;
                interaction.dirty = true;
                true
            });
    }

    let event_loop = shell.event_loop_handle();
    let tray_state = Arc::new(Mutex::new(tray_handle));
    let tray_state_for_timer = Arc::clone(&tray_state);
    let tray_preview_state_for_timer = Arc::clone(&tray_preview_state);
    let preview_state_for_timer = Rc::clone(&shared_preview);
    let interaction_state_for_timer = Rc::clone(&interaction_state);
    let runtime_state_for_timer = Rc::clone(&runtime_state);
    let mpris_for_timer = Rc::clone(&mpris_client);
    let settings_for_timer = settings.clone();

    event_loop.add_timer(Duration::from_millis(16), move |_, app_state| {
        if let Ok(tray_preview) = tray_preview_state_for_timer.lock() {
            if *tray_preview != *preview_state_for_timer.borrow() {
                *preview_state_for_timer.borrow_mut() = tray_preview.clone();
            }
        }

        let preview = preview_state_for_timer.borrow().clone();
        let interaction = interaction_state_for_timer.borrow().clone();

        let mut runtime = runtime_state_for_timer.borrow_mut();
        let mut is_playing = runtime
            .last_track
            .as_ref()
            .map(|track| track.playback_status.eq_ignore_ascii_case("playing"))
            .unwrap_or(false);
        let render = if interaction.dragging || interaction.resizing || interaction.dirty {
            let mut render = runtime.last_render.clone().unwrap_or(RenderState {
                locked: preview.locked.unwrap_or(false),
                width: preview.panel_width.unwrap_or(640),
                height: preview.panel_height.unwrap_or(112),
                x: preview.panel_x.unwrap_or(36).max(0),
                y: preview.panel_y.unwrap_or(24).max(0),
                line_1: String::new(),
                line_2: String::new(),
                line_1_segments: Vec::new(),
                line_2_segments: Vec::new(),
                progress_1: 1.0,
                progress_2: 0.0,
                line_1_active: true,
                line_2_active: false,
                scroll_1: 0.0,
                scroll_2: 0.0,
                offset_1: 0.0,
                offset_2: 0.0,
                font_scale: preview.font_scale.unwrap_or(0),
                show_secondary_line: settings_for_timer.show_secondary_line,
                use_gradient: settings_for_timer.use_gradient,
                lyric_effect: settings_for_timer.lyric_effect.clone(),
                font_family: settings_for_timer.font_family.clone(),
                highlight_color: settings_for_timer.highlight_color.clone(),
                base_color: settings_for_timer.base_color.clone(),
                preview_color: settings_for_timer.preview_color.clone(),
                stroke_color: settings_for_timer.stroke_color.clone(),
                stroke_width: settings_for_timer.stroke_width,
                shadow_color: settings_for_timer.shadow_color.clone(),
                panel_background_color: settings_for_timer.panel_background_color.clone(),
                panel_border_color: settings_for_timer.panel_border_color.clone(),
                resize_handle_color: settings_for_timer.resize_handle_color.clone(),
                lyrics_opacity: settings_for_timer.lyrics_opacity,
                preview_opacity: settings_for_timer.preview_opacity,
            });
            render.locked = preview.locked.unwrap_or(false);
            render.width = preview.panel_width.unwrap_or(640);
            render.height = preview.panel_height.unwrap_or(112);
            render.x = preview.panel_x.unwrap_or(36).max(0);
            render.y = preview.panel_y.unwrap_or(24).max(0);
            render.font_scale = preview.font_scale.unwrap_or(0);
            render.show_secondary_line = settings_for_timer.show_secondary_line;
            render.use_gradient = settings_for_timer.use_gradient;
            render.lyric_effect = settings_for_timer.lyric_effect.clone();
            render.font_family = settings_for_timer.font_family.clone();
            render.highlight_color = settings_for_timer.highlight_color.clone();
            render.base_color = settings_for_timer.base_color.clone();
            render.preview_color = settings_for_timer.preview_color.clone();
            render.stroke_color = settings_for_timer.stroke_color.clone();
            render.stroke_width = settings_for_timer.stroke_width;
            render.shadow_color = settings_for_timer.shadow_color.clone();
            render.panel_background_color = settings_for_timer.panel_background_color.clone();
            render.panel_border_color = settings_for_timer.panel_border_color.clone();
            render.resize_handle_color = settings_for_timer.resize_handle_color.clone();
            render.lyrics_opacity = settings_for_timer.lyrics_opacity;
            render.preview_opacity = settings_for_timer.preview_opacity;
            render
        } else {
            let should_poll_track = runtime
                .last_track_poll
                .map(|last_poll| last_poll.elapsed() >= Duration::from_millis(200))
                .unwrap_or(true);
            if should_poll_track {
                runtime.last_track = mpris_for_timer
                    .as_ref()
                    .as_ref()
                    .and_then(|client| client.read_track());
                runtime.last_track_poll = Some(Instant::now());
            }

            let playback = runtime.last_track.as_ref().map(|track| {
                let mut predicted = track.clone();
                if predicted.playback_status.eq_ignore_ascii_case("playing") {
                    if let Some(last_poll) = runtime.last_track_poll {
                        let elapsed_ms = last_poll.elapsed().as_millis() as u64;
                        predicted.position_ms = predicted
                            .position_ms
                            .saturating_add(elapsed_ms)
                            .min(predicted.duration_ms);
                    }
                }
                predicted
            });
            is_playing = playback
                .as_ref()
                .map(|track| track.playback_status.eq_ignore_ascii_case("playing"))
                .unwrap_or(false);

            let lyric_pair = current_lines_for_track(
                playback.as_ref(),
                &mut runtime,
                &settings_for_timer,
                Path::new("scripts/fetch-current-song-lyrics.js"),
            );
            RenderState {
                locked: preview.locked.unwrap_or(false),
                width: preview.panel_width.unwrap_or(640),
                height: preview.panel_height.unwrap_or(112),
                x: preview.panel_x.unwrap_or(36).max(0),
                y: preview.panel_y.unwrap_or(24).max(0),
                line_1: lyric_pair.line_1,
                line_2: lyric_pair.line_2,
                line_1_segments: lyric_pair.line_1_segments,
                line_2_segments: lyric_pair.line_2_segments,
                progress_1: lyric_pair.progress_1,
                progress_2: lyric_pair.progress_2,
                line_1_active: lyric_pair.line_1_active,
                line_2_active: lyric_pair.line_2_active,
                scroll_1: 0.0,
                scroll_2: 0.0,
                offset_1: 0.0,
                offset_2: 0.0,
                font_scale: preview.font_scale.unwrap_or(0),
                show_secondary_line: settings_for_timer.show_secondary_line,
                use_gradient: settings_for_timer.use_gradient,
                lyric_effect: settings_for_timer.lyric_effect.clone(),
                font_family: settings_for_timer.font_family.clone(),
                highlight_color: settings_for_timer.highlight_color.clone(),
                base_color: settings_for_timer.base_color.clone(),
                preview_color: settings_for_timer.preview_color.clone(),
                stroke_color: settings_for_timer.stroke_color.clone(),
                stroke_width: settings_for_timer.stroke_width,
                shadow_color: settings_for_timer.shadow_color.clone(),
                panel_background_color: settings_for_timer.panel_background_color.clone(),
                panel_border_color: settings_for_timer.panel_border_color.clone(),
                resize_handle_color: settings_for_timer.resize_handle_color.clone(),
                lyrics_opacity: settings_for_timer.lyrics_opacity,
                preview_opacity: settings_for_timer.preview_opacity,
            }
        };

        if interaction.dirty {
            interaction_state_for_timer.borrow_mut().dirty = false;
        }

        let needs_passthrough_update = runtime.last_locked != Some(render.locked);
        if runtime.last_render.as_ref() != Some(&render) {
            for surface in app_state.surfaces_by_name_mut(SURFACE_NAME) {
                let component = surface.component_instance();

                let _ = component.set_property("locked", Value::Bool(render.locked));
                let _ = component.set_property("panel_width", Value::Number(render.width as f64));
                let _ = component.set_property("panel_height", Value::Number(render.height as f64));
                let _ = component.set_property("line_1", shared(Some(&render.line_1)));
                let _ = component.set_property("line_2", shared(Some(&render.line_2)));
                let _ = component.set_property("line_1_segments", build_segment_model(&render.line_1_segments));
                let _ = component.set_property("line_2_segments", build_segment_model(&render.line_2_segments));
                let _ = component.set_property("progress_1", Value::Number(render.progress_1 as f64));
                let _ = component.set_property("progress_2", Value::Number(render.progress_2 as f64));
                let _ = component.set_property("line_1_active", Value::Bool(render.line_1_active));
                let _ = component.set_property("line_2_active", Value::Bool(render.line_2_active));
                let _ = component.set_property("scroll_1", Value::Number(render.scroll_1 as f64));
                let _ = component.set_property("scroll_2", Value::Number(render.scroll_2 as f64));
                let _ = component.set_property("offset_1", Value::Number(render.offset_1 as f64));
                let _ = component.set_property("offset_2", Value::Number(render.offset_2 as f64));
                let _ = component.set_property("font_scale", Value::Number(render.font_scale as f64));
                let _ = component.set_property("show_secondary_line", Value::Bool(render.show_secondary_line));
                let _ = component.set_property("use_gradient", Value::Bool(render.use_gradient));
                let _ = component.set_property("lyric_effect", Value::String(render.lyric_effect.clone().into()));
                let _ = component.set_property("lyric_font", Value::String(render.font_family.clone().into()));
                let _ = component.set_property(
                    "highlight_color",
                    Value::Brush(Brush::from(parse_hex_color(&render.highlight_color))),
                );
                let _ = component.set_property(
                    "base_color",
                    Value::Brush(Brush::from(parse_hex_color(&render.base_color))),
                );
                let _ = component.set_property(
                    "preview_color",
                    Value::Brush(Brush::from(parse_hex_color(&render.preview_color))),
                );
                let _ = component.set_property(
                    "stroke_color",
                    Value::Brush(Brush::from(parse_hex_color(&render.stroke_color))),
                );
                let _ = component.set_property(
                    "stroke_width",
                    Value::Number(render.stroke_width as f64),
                );
                let _ = component.set_property(
                    "shadow_color",
                    Value::Brush(Brush::from(parse_hex_color(&render.shadow_color))),
                );
                let _ = component.set_property(
                    "panel_background_brush",
                    Value::Brush(Brush::from(parse_hex_color(&render.panel_background_color))),
                );
                let _ = component.set_property(
                    "panel_border_brush",
                    Value::Brush(Brush::from(parse_hex_color(&render.panel_border_color))),
                );
                let _ = component.set_property(
                    "resize_handle_brush",
                    Value::Brush(Brush::from(parse_hex_color(&render.resize_handle_color))),
                );
                let _ = component.set_property(
                    "lyrics_opacity",
                    Value::Number(render.lyrics_opacity as f64),
                );
                let _ = component.set_property(
                    "preview_opacity",
                    Value::Number(render.preview_opacity as f64),
                );

                surface.layer_surface().set_size(render.width, render.height);
                surface
                    .layer_surface()
                    .set_margin(render.y, 0, 0, render.x);
                surface.commit_surface();

                if needs_passthrough_update {
                    if runtime.passthrough.is_none() {
                        runtime.passthrough = WaylandPassthrough::from_surface_state(surface);
                    }
                    if let Some(passthrough) = runtime.passthrough.as_mut() {
                        passthrough.set_interactive(!render.locked);
                    }
                }
            }

            if let Ok(guard) = tray_state_for_timer.lock() {
                if let Some(handle) = guard.as_ref() {
                    let _ = handle.update(|tray: &mut LyricDockTray| {
                        tray.locked = render.locked;
                    });
                }
            }

            runtime.last_render = Some(render.clone());
        } else if needs_passthrough_update {
            for surface in app_state.surfaces_by_name_mut(SURFACE_NAME) {
                if runtime.passthrough.is_none() {
                    runtime.passthrough = WaylandPassthrough::from_surface_state(surface);
                }
            }
            if let Some(passthrough) = runtime.passthrough.as_mut() {
                passthrough.set_interactive(!render.locked);
            }
        }

        runtime.last_locked = Some(render.locked);
        if interaction.dragging || interaction.resizing || interaction.dirty || is_playing {
            TimeoutAction::ToDuration(Duration::from_millis(16))
        } else {
            TimeoutAction::ToDuration(Duration::from_millis(200))
        }
    })?;

    shell.select(Surface::named(SURFACE_NAME)).configure(|_, surface| {
        surface.set_layer(Layer::Overlay);
        surface.set_anchor_edges(AnchorEdges::empty().with_top().with_left());
        surface.set_exclusive_zone(0);
        surface.set_size(
            initial_preview.panel_width.unwrap_or(640),
            initial_preview.panel_height.unwrap_or(112),
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
        Value::Number(preview.panel_height.unwrap_or(112) as f64),
    );
    let _ = component.set_property(
        "font_scale",
        Value::Number(preview.font_scale.unwrap_or(0) as f64),
    );
    let _ = component.set_property(
        "lyric_font",
        Value::String("Noto Sans CJK SC, Source Han Sans SC, Noto Sans, sans-serif".into()),
    );
    let _ = component.set_property("show_secondary_line", Value::Bool(true));
    let _ = component.set_property("use_gradient", Value::Bool(false));
    let _ = component.set_property("lyric_effect", Value::String("flat".into()));
    let _ = component.set_property("highlight_color", Value::Brush(Brush::from(parse_hex_color("#00e676"))));
    let _ = component.set_property("base_color", Value::Brush(Brush::from(parse_hex_color("#f5f7fb"))));
    let _ = component.set_property("preview_color", Value::Brush(Brush::from(parse_hex_color("#f5f7fb"))));
    let _ = component.set_property("stroke_color", Value::Brush(Brush::from(parse_hex_color("#081019e0"))));
    let _ = component.set_property("stroke_width", Value::Number(3.2));
    let _ = component.set_property("shadow_color", Value::Brush(Brush::from(parse_hex_color("#000000c4"))));
    let _ = component.set_property(
        "panel_background_brush",
        Value::Brush(Brush::from(parse_hex_color("#00000095"))),
    );
    let _ = component.set_property(
        "panel_border_brush",
        Value::Brush(Brush::from(parse_hex_color("#ffffff28"))),
    );
    let _ = component.set_property(
        "resize_handle_brush",
        Value::Brush(Brush::from(parse_hex_color("#ffffffa8"))),
    );
    let _ = component.set_property("lyrics_opacity", Value::Number(1.0));
    let _ = component.set_property("preview_opacity", Value::Number(1.0));
    let _ = component.set_property("line_1_segments", build_segment_model(&[]));
    let _ = component.set_property("line_2_segments", build_segment_model(&[]));
}

fn build_segment_model(segments: &[LyricRenderSegment]) -> Value {
    let rows: Vec<Value> = segments
        .iter()
        .map(|segment| {
            Value::Struct(Struct::from_iter([
                ("text".to_string(), Value::String(segment.text.clone().into())),
                ("progress".to_string(), Value::Number(segment.progress as f64)),
            ]))
        })
        .collect();
    Value::Model(ModelRc::new(VecModel::from(rows)))
}

fn current_lines_for_track(
    playback: Option<&TrackInfo>,
    runtime: &mut RuntimeState,
    settings: &AppSettings,
    fetch_script: &Path,
) -> LyricRenderPair {
    let Some(track) = playback else {
        runtime.active_lyrics = None;
        return LyricRenderPair {
            line_1: "正在等待 Spotify...".into(),
            line_2: "".into(),
            line_1_segments: vec![LyricRenderSegment {
                text: "正在等待 Spotify...".into(),
                progress: 1.0,
            }],
            line_2_segments: Vec::new(),
            progress_1: 1.0,
            progress_2: 0.0,
            line_1_active: true,
            line_2_active: false,
        };
    };

    let track_key = track_cache_key(track);
    let should_reload = runtime
        .active_lyrics
        .as_ref()
        .map(|lyrics| lyrics.track_key != track_key)
        .unwrap_or(true);

    if should_reload {
        runtime.active_lyrics = load_or_fetch_lyrics(track, settings, fetch_script, runtime)
            .map(|lines| ActiveLyrics {
                track_key: track_key.clone(),
                lines,
            });
    }

    if let Some(active) = runtime.active_lyrics.as_ref() {
        return lyric_pair_for_position(&active.lines, track.position_ms, settings.show_secondary_line);
    }

    let artist = track.artists.first().cloned().unwrap_or_default();
    LyricRenderPair {
        line_1: format!("{} - {}", artist, track.title),
        line_2: "".into(),
        line_1_segments: vec![LyricRenderSegment {
            text: format!("{} - {}", artist, track.title),
            progress: 1.0,
        }],
        line_2_segments: Vec::new(),
        progress_1: 1.0,
        progress_2: 0.0,
        line_1_active: true,
        line_2_active: false,
    }
}

fn load_or_fetch_lyrics(
    track: &TrackInfo,
    settings: &AppSettings,
    fetch_script: &Path,
    runtime: &mut RuntimeState,
) -> Option<Vec<LyricLine>> {
    if let Some(path) = find_imported_lyric_path(track, &settings.imported_dir) {
        return read_lyric_file(&path);
    }

    if let Some(path) = find_cached_lyric_path(track, &settings.cache_dir) {
        return read_lyric_file(&path);
    }

    if let Some(path) = find_cached_lyric_path(track, Path::new(LEGACY_LYRICS_DIR)) {
        return read_lyric_file(&path);
    }

    let key = track_cache_key(track);
    let should_fetch = runtime
        .last_fetch_attempt
        .as_ref()
        .map(|(last_key, last_when)| last_key != &key || last_when.elapsed() > Duration::from_secs(8))
        .unwrap_or(true);

    if should_fetch {
        let mut command = Command::new("node");
        command
            .arg(fetch_script)
            .env("LYRICDOCK_CACHE_DIR", &settings.cache_dir)
            .current_dir(Path::new("."));
        if let Ok(track_json) = serde_json::to_string(track) {
            command.env("LYRICDOCK_TRACK_JSON", track_json);
        }
        let _ = command.status();
        runtime.last_fetch_attempt = Some((key.clone(), Instant::now()));
        enforce_cache_limit(&settings.cache_dir, settings.cache_limit_bytes);
    }

    find_cached_lyric_path(track, &settings.cache_dir)
        .or_else(|| find_cached_lyric_path(track, Path::new(LEGACY_LYRICS_DIR)))
        .and_then(|path| read_lyric_file(&path))
}

fn find_imported_lyric_path(track: &TrackInfo, imported_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(imported_dir).ok()?;
    let artist = track
        .artists
        .first()
        .map(|item| normalize_for_match(item))
        .unwrap_or_default();
    let title = normalize_for_match(&track.title);

    for entry in entries.flatten() {
        let path = entry.path();
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
        if extension != "lrc" && extension != "yrc" {
            continue;
        }
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or_default();
        let normalized_stem = normalize_filename_for_match(stem);
        if normalized_stem.contains(&artist) && normalized_stem.contains(&title) {
            return Some(path);
        }
    }

    None
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
            let mut yrc_path = path.clone();
            yrc_path.set_extension("yrc");
            if yrc_path.exists() {
                return Some(yrc_path);
            }
        }
    }

    None
}

fn read_lyric_file(path: &Path) -> Option<Vec<LyricLine>> {
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

    let content = fs::read_to_string(path).ok()?;
    let lines = parse_lrc(&content);
    (!lines.is_empty()).then_some(lines)
}

fn parse_hex_color(input: &str) -> Color {
    let text = input.trim().trim_start_matches('#');
    let bytes = match text.len() {
        6 => u32::from_str_radix(text, 16).ok().map(|value| 0xff00_0000u32 | value),
        8 => u32::from_str_radix(text, 16).ok(),
        _ => None,
    }
    .unwrap_or(0xffff_ffff);

    Color::from_argb_u8(
        ((bytes >> 24) & 0xff) as u8,
        ((bytes >> 16) & 0xff) as u8,
        ((bytes >> 8) & 0xff) as u8,
        (bytes & 0xff) as u8,
    )
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());

    let (r1, g1, b1) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    let m = v - c;
    let r = ((r1 + m) * 255.0) as u8;
    let g = ((g1 + m) * 255.0) as u8;
    let b = ((b1 + m) * 255.0) as u8;
    (r, g, b)
}

fn hsv_to_hex_color(h: f32, s: f32, v: f32) -> String {
    let (r, g, b) = hsv_to_rgb(h, s, v);
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

fn lyric_pair_for_position(
    lines: &[LyricLine],
    position_ms: u64,
    show_secondary_line: bool,
) -> LyricRenderPair {
    if lines.is_empty() {
        return LyricRenderPair {
            line_1: "未找到歌词".into(),
            line_2: "".into(),
            line_1_segments: vec![LyricRenderSegment {
                text: "未找到歌词".into(),
                progress: 1.0,
            }],
            line_2_segments: Vec::new(),
            progress_1: 1.0,
            progress_2: 0.0,
            line_1_active: true,
            line_2_active: false,
        };
    }

    let current_index = find_current_line_index(lines, position_ms);
    if !show_secondary_line {
        let current = lines
            .get(current_index)
            .map(|line| line.text.clone())
            .unwrap_or_default();
        let progress_1 = lines
            .get(current_index)
            .map(|line| line_progress(line, position_ms))
            .unwrap_or(0.0);
        return LyricRenderPair {
            line_1: current,
            line_2: "".into(),
            line_1_segments: build_render_segments(lines.get(current_index), position_ms, true),
            line_2_segments: Vec::new(),
            progress_1,
            progress_2: 0.0,
            line_1_active: true,
            line_2_active: false,
        };
    }

    if current_index % 2 == 0 {
        let current = lines.get(current_index);
        let next = lines.get(current_index + 1);
        return LyricRenderPair {
            line_1: current.map(|line| line.text.clone()).unwrap_or_default(),
            line_2: next.map(|line| line.text.clone()).unwrap_or_default(),
            line_1_segments: build_render_segments(current, position_ms, true),
            line_2_segments: build_render_segments(next, position_ms, false),
            progress_1: current
                .map(|line| line_progress(line, position_ms))
                .unwrap_or(0.0),
            progress_2: 0.0,
            line_1_active: true,
            line_2_active: false,
        };
    }

    let upcoming = lines.get(current_index + 1);
    let current = lines.get(current_index);
    LyricRenderPair {
        line_1: upcoming.map(|line| line.text.clone()).unwrap_or_default(),
        line_2: current.map(|line| line.text.clone()).unwrap_or_default(),
        line_1_segments: build_render_segments(upcoming, position_ms, false),
        line_2_segments: build_render_segments(current, position_ms, true),
        progress_1: 0.0,
        progress_2: current
            .map(|line| line_progress(line, position_ms))
            .unwrap_or(0.0),
        line_1_active: false,
        line_2_active: current.is_some(),
    }
}

fn build_render_segments(
    line: Option<&LyricLine>,
    position_ms: u64,
    active: bool,
) -> Vec<LyricRenderSegment> {
    let Some(line) = line else {
        return Vec::new();
    };

    if line.segments.is_empty() {
        return vec![LyricRenderSegment {
            text: line.text.clone(),
            progress: if active {
                line_progress(line, position_ms)
            } else {
                0.0
            },
        }];
    }

    line.segments
        .iter()
        .map(|segment| {
            let progress = if !active {
                0.0
            } else if position_ms >= segment.end_time_ms {
                1.0
            } else if position_ms <= segment.start_time_ms {
                0.0
            } else {
                let duration = segment
                    .end_time_ms
                    .saturating_sub(segment.start_time_ms)
                    .max(1);
                (position_ms.saturating_sub(segment.start_time_ms)) as f32 / duration as f32
            };

            LyricRenderSegment {
                text: segment.text.clone(),
                progress: progress.clamp(0.0, 1.0),
            }
        })
        .collect()
}

fn find_current_line_index(lines: &[LyricLine], position_ms: u64) -> usize {
    if lines.is_empty() {
        return 0;
    }

    let mut lo = 0usize;
    let mut hi = lines.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if lines[mid].time_ms <= position_ms {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    lo.saturating_sub(1)
}

fn line_motion(text: &str, font_px: f32, available_px: f32, progress: f32) -> (f32, f32) {
    if text.is_empty() {
        return (0.0, 0.0);
    }

    let columns = UnicodeWidthStr::width(text) as f32;
    let text_px = columns * font_px * 0.52;
    if text_px <= available_px {
        return ((available_px - text_px) * 0.5, 0.0);
    }

    let overflow = text_px - available_px;
    (0.0, overflow * progress.clamp(0.0, 1.0))
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

fn normalize_filename_for_match(text: &str) -> String {
    normalize_for_match(&text.replace('-', " ").replace('_', " "))
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

fn open_path(path: &Path) {
    let _ = Command::new("xdg-open").arg(path).spawn();
}

fn clear_cache_dir(cache_dir: &Path) {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

fn enforce_cache_limit(cache_dir: &Path, limit_bytes: u64) {
    if limit_bytes == 0 {
        return;
    }

    let mut groups = cache_file_groups(cache_dir);
    let mut total_size: u64 = groups.iter().map(|group| group.total_size).sum();

    if total_size <= limit_bytes {
        return;
    }

    groups.sort_by_key(|group| group.modified_unix_secs);
    for group in groups {
        if total_size <= limit_bytes {
            break;
        }
        total_size = total_size.saturating_sub(group.total_size);
        for file in group.files {
            let _ = fs::remove_file(file);
        }
    }
}

#[derive(Debug)]
struct CacheFileGroup {
    modified_unix_secs: u64,
    total_size: u64,
    files: Vec<PathBuf>,
}

fn cache_file_groups(cache_dir: &Path) -> Vec<CacheFileGroup> {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return Vec::new();
    };

    let mut groups: HashMap<String, CacheFileGroup> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let metadata = match entry.metadata() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let modified_unix_secs = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let total_size = metadata.len();

        groups
            .entry(stem)
            .and_modify(|group| {
                group.total_size = group.total_size.saturating_add(total_size);
                group.modified_unix_secs = group.modified_unix_secs.min(modified_unix_secs);
                group.files.push(path.clone());
            })
            .or_insert_with(|| CacheFileGroup {
                modified_unix_secs,
                total_size,
                files: vec![path.clone()],
            });
    }

    groups.into_values().collect()
}

fn tray_icon() -> ksni::Icon {
    let bytes = include_bytes!("../assets/tray-icon.png");
    let image = image::load(Cursor::new(bytes), image::ImageFormat::Png)
        .expect("failed to decode tray icon png")
        .to_rgba8();
    let width = image.width() as i32;
    let height = image.height() as i32;
    ksni::Icon {
        width,
        height,
        data: image.into_raw(),
    }
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
