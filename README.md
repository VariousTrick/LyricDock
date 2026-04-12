# somelyric

这是一个给 `niri / Wayland` 使用的桌面歌词实验项目。

目前已经完成两条基础验证：

- 可以从 `Spotify MPRIS` 读取当前歌曲信息
- 可以从网易云搜索并下载当前歌曲歌词
- 可以把歌词缓存到本地
- 可以启动一个不占平铺空间的 `layer-shell` 浮动覆盖窗口

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
- 浮动覆盖在普通程序之上
- 不占用平铺布局空间
- 目标运行环境是 `Wayland + niri`

## 当前阶段限制

现在的窗口还只是第一版骨架，界面内容是静态示意文本。

下一步要做的是：

- 让解锁态支持拖动和记忆位置
- 研究并接入锁定态鼠标穿透
- 接入真实歌词缓存和播放进度
