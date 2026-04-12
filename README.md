# somelyric

这是一个给 `niri / Wayland` 使用的桌面歌词实验项目。

目前已经完成两条基础验证：

- 可以从 `Spotify MPRIS` 读取当前歌曲信息
- 可以从网易云搜索并下载当前歌曲歌词
- 可以把歌词缓存到本地
- 可以启动一个不占平铺空间的 `layer-shell` 浮动覆盖窗口
- 可以根据播放进度做主歌词动态高亮（已播放/未播放分层）

## 当前目录说明

- `scripts/fetch-current-song-lyrics.js`
  用当前 Spotify 歌曲信息去网易云抓歌词并保存到 `lyrics-cache/`
- `ui/lyrics-overlay.slint`
  桌面歌词窗口的 Slint 界面
- `src/main.rs`
  `Wayland layer-shell` 窗口入口
- `assets/icon-light.svg`
  你选的浅色图标版本
- `assets/icon-dark.svg`
  你选的深色图标版本
- `开发计划.md`
  当前开发计划

## 当前能做什么

### 1. 抓取当前歌曲歌词

```bash
npm run fetch
```

### 2. 启动桌面歌词窗口骨架

```bash
cargo run
```

启动后具备：

- 双行歌词显示（当前行 + 下一行）
- 主歌词按时间推进的动态进度高亮
- 锁定态鼠标穿透、编辑态可拖拽/缩放

## 当前调试方式

运行程序后，直接修改 [调试面板.json](/home/Arch/Downloads/vscode/somelyric/调试面板.json)：

- `locked`
- `panel_width`
- `panel_height`
- `panel_x`
- `panel_y`
- `title`
- `artist`
- `status`
- `hint`
- `note`
- `lines`

保存后面板会自动刷新。

## 当前托盘能力

- 托盘图标改为你选的两版 SVG 图标
- 可以通过托盘菜单锁定歌词
- 锁定后可以通过托盘再次解锁
- 也可以通过面板上的“锁定”按钮进入锁定状态

## 当前窗口特性

- 横向双行歌词条显示
- 主歌词“已播放/未播放”渐变分层，支持进度推进
- 浮动覆盖在普通程序之上
- 不占用平铺布局空间
- 目标运行环境是 `Wayland + niri`

## 当前阶段限制

当前版本仍有这些限制：

- 进度高亮是“按行时间”推进，暂未实现逐字 karaoke
- 行2默认作为下一句预告，暂未做翻译行对齐
- 拉词流程仍使用外部脚本调用，后续可改为异步任务
