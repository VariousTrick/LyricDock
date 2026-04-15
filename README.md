# LyricDock

这是一个给 `niri / Wayland` 使用的桌面歌词项目。

目前已经完成两条基础验证：

- 可以从 `Spotify MPRIS` 读取当前歌曲信息
- 可以从网易云搜索并下载当前歌曲歌词
- 可以把歌词缓存到本地
- 可以启动一个不占平铺空间的 `layer-shell` 浮动覆盖窗口
- 可以根据播放进度做主歌词动态高亮（已播放/未播放分层）
- 可以在锁定态实现真正的鼠标穿透

## 当前目录说明

- `scripts/fetch-current-song-lyrics.js`
  用当前 Spotify 歌曲信息去网易云抓歌词并保存到 `lyrics-cache/`
- `ui/lyrics-overlay.slint`
  桌面歌词窗口的 Slint 界面
- `src/main.rs`
  `Wayland layer-shell` 窗口入口
- `assets/app-icon.png`
  程序图标 PNG
- `assets/tray-icon.png`
  托盘图标 PNG
- `开发计划.md`
  当前开发计划
- `调试面板.json`
  当前本地窗口状态文件

## 资源来源

- 程序图标来源于 iconfont.cn 用户页面 [https://www.iconfont.cn/user/detail?spm=a313x.user_detail.i1.6.23913a81uJvglA&userViewType=activity&uid=942&nid=oUuw5uwrsXDY](https://www.iconfont.cn/user/detail?spm=a313x.user_detail.i1.6.23913a81uJvglA&userViewType=activity&uid=942&nid=oUuw5uwrsXDY)，作者/用户名为“不苳”

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
- 两行歌词居中显示，主副行均支持进度高亮
- 长歌词会按进度水平滚动，不再用省略号截断
- 锁定态鼠标穿透、编辑态可拖拽/缩放
- 编辑态顶部提供锁定、字体 +/-、配色切换图标按钮

## 当前本地状态文件

[调试面板.json](/home/Arch/Downloads/vscode/LyricDock/调试面板.json) 现在已经不再是早期用于塞静态文本的“调试面板输入文件”，而是程序实际使用的本地窗口状态文件。

它目前负责保存：

- `locked`
- `panel_width`
- `panel_height`
- `panel_x`
- `panel_y`
- `font_scale`
- `palette_color`

程序启动时会读取它，拖动、缩放、锁定/解锁、调节字体后也会写回这里。

所以现在它仍然需要，只是名字已经有点过时了。以后如果你想让仓库更干净，可以再把它改名成 `window-state.json` 或 `local-state.json`。

## 当前缓存说明

- 当前歌词默认缓存到仓库内的 `lyrics-cache/`
- 目前还可以手动指定缓存目录，但还没有做成界面设置项
- 还没有实现缓存空间上限、自动清理旧缓存
- 当前缓存文件名不是严格的“歌手-歌曲”标准格式；如果后续要给播放器或外部工具更稳定地识别，建议统一改成 `歌手 - 歌曲名`
- 目前缓存内容已经足够程序内部读取和播放使用，后续再补目录配置和清理策略

## 当前托盘能力

- 托盘图标改为你选的两版 PNG 图标
- 可以通过托盘菜单锁定歌词
- 锁定后可以通过托盘再次解锁
- 也可以通过面板上的“锁定”按钮进入锁定状态

## 当前窗口特性

- 横向双行歌词条显示
- 主歌词“已播放/未播放”渐变分层，支持进度推进
- 解锁态采用简洁黑色半透明面板
- 浮动覆盖在普通程序之上
- 不占用平铺布局空间
- 目标运行环境是 `Wayland + niri`

## 当前阶段限制

当前版本仍有这些限制：

- 进度高亮是“按行时间”推进，暂未实现逐字 karaoke
- 行2默认作为下一句预告，暂未做翻译行对齐
- 拉词流程仍使用外部脚本调用，后续可改为异步任务

## 当前更新

- 锁定态鼠标穿透已经通过真实 `wl_surface` 的 input region 切换实现
- 当前仓库附带 `vendor/layer-shika-adapters`，用于保留鼠标穿透所需的底层补丁
- 运行性能不会因为 `vendor` 产生额外负担，影响主要在仓库结构和后续依赖升级上
