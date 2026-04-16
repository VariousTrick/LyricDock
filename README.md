# LyricDock

这是一个给 `niri / Wayland` 使用的桌面歌词项目。

目前已经完成两条基础验证：

- 可以从 `Spotify MPRIS` 读取当前歌曲信息
- 可以从网易云搜索并下载当前歌曲歌词
- 可以把歌词缓存到本地
- 可以启动一个不占平铺空间的 `layer-shell` 浮动覆盖窗口
- 可以根据播放进度做主歌词动态高亮（已播放/未播放分层）
- 可以在有 `yrc` 逐字歌词时启用按片段推进的 karaoke 高亮
- 可以在锁定态实现真正的鼠标穿透

## 当前目录说明

- `scripts/fetch-current-song-lyrics.js`
  用当前 Spotify 歌曲信息去网易云抓歌词并保存到配置指定的缓存目录
  正常运行时由 Rust 主程序把曲目信息传给脚本，脚本不再自己重复读取一次 Spotify
- `ui/lyrics-overlay.slint`
  桌面歌词窗口的 Slint 界面，当前已支持按片段渲染的 karaoke 行
- `src/main.rs`
  `Wayland layer-shell` 窗口入口
- `src/lyrics/parser.rs`
  LRC / YRC 解析与歌词进度计算
- `src/mpris.rs`
  Spotify `MPRIS` 读取逻辑
- `src/settings.rs`
  配置文件与窗口状态文件读写
- `assets/app-icon.png`
  程序图标 PNG
- `assets/tray-icon.png`
  托盘图标 PNG
- `开发计划.md`
  当前开发计划
- `配置.toml`
  主配置文件，带中文注释，可手动填写歌词目录、缓存上限和第二行开关
- `窗口状态.json`
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
- 第二行可以通过配置文件开关，默认开启
- 有 `yrc` 时按歌词片段直接推进，不再只靠整句裁剪宽度估算
- 锁定态鼠标穿透、编辑态可拖拽/缩放
- 编辑态顶部提供锁定、字体 +/- 图标按钮

## 当前本地状态文件

[窗口状态.json](/home/Arch/Downloads/vscode/LyricDock/窗口状态.json) 现在就是程序实际使用的本地窗口状态文件。

它目前负责保存：

- `locked`
- `panel_width`
- `panel_height`
- `panel_x`
- `panel_y`
- `font_scale`

程序启动时会读取它，拖动、缩放、锁定/解锁、调节字体后也会写回这里。

如果文件不存在，程序启动时会自动创建一个默认状态文件；如果仓库里仍然留着老名字 `调试面板.json`，程序也会在启动时自动迁移到新名字。

## 当前缓存说明

- 当前通过 [配置.toml](/home/Arch/Downloads/vscode/LyricDock/配置.toml) 指定歌词根目录
- 可以通过 `show_secondary_line = true/false` 控制是否显示第二行歌词
- 可以通过 `use_gradient = true/false` 控制歌词高亮使用渐变还是纯色，默认纯色
- 可以通过 `lyric_effect = "flat"/"floating"` 切换平面全描边和轻微浮动两种效果
- 颜色、描边、面板底色、歌词透明度现在都可以直接在 `配置.toml` 中手动填写
- 程序会在歌词根目录下自动创建：
  - `导入歌词/`
  - `缓存歌词/`
- 你手动导入的歌词请放进 `导入歌词/`
- 程序自动抓取的歌词会放进 `缓存歌词/`
- 缓存歌词现在会优先使用 `歌手 - 歌曲名.lrc` 这类人能看懂的文件名
- 超出 `cache_limit_mb` 后，只会清理 `缓存歌词/` 里的旧文件，不会删除你手动导入的歌词
- 老的 `lyrics-cache/` 目录目前仍会作为兼容回退读取，但新歌词默认不再写进去

## 当前托盘能力

- 托盘图标改为你选的两版 PNG 图标
- 可以通过托盘菜单锁定歌词
- 锁定后可以通过托盘再次解锁
- 也可以通过面板上的“锁定”按钮进入锁定状态
- 可以通过托盘打开配置文件
- 可以通过托盘打开歌词目录
- 可以通过托盘手动清理缓存歌词

## 当前窗口特性

- 横向双行歌词条显示
- 双行模式改为交错接力：当前句在哪一行，另一行就提前显示下一句
- 主歌词支持“已播放/未播放”分层推进，可切换纯色或渐变
- 提供 `flat/floating` 两种歌词效果：
  - `flat`：平面全描边，优先可读性，适合亮背景
  - `floating`：轻微浮动与边缘高亮，偏视觉质感
- 有同名 `.yrc` 文件时会优先走逐字时间轴，没有则自动回退到普通 `.lrc`
- `karaoke` 当前改为按片段渲染：每个字/词片段单独计算自己的高亮进度
- 解锁态采用简洁黑色半透明面板
- 浮动覆盖在普通程序之上
- 不占用平铺布局空间
- 目标运行环境是 `Wayland + niri`

## 当前阶段限制

当前版本仍有这些限制：

- 并不是每首歌都有 `yrc`，没有逐字歌词的歌曲会自动退回普通逐行模式
- 行2默认作为下一句预告，暂未做翻译行对齐
- 双行模式当前仍未加入更明显的句间位移动画
- 拉词流程仍使用外部脚本调用，后续可改为异步任务

## 样式策略

当前样式策略改为“可读性优先，效果可切换”：

- 先保证亮色/复杂背景下的歌词可读性
- 通过配置文件切换 `flat` 与 `floating`，而不是绑定单一视觉风格
- 样式参数尽量配置化，减少反复改代码

推荐：

- 亮色背景优先使用 `lyric_effect = "flat"`
- 配合深色 `stroke_color` 和更大的 `stroke_width`
- `lyrics_opacity = 1.0` 保持不透明以获得最稳可读性

## 当前更新

- 锁定态鼠标穿透已经通过真实 `wl_surface` 的 input region 切换实现
- 当前仓库附带 `vendor/layer-shika-adapters`，用于保留鼠标穿透所需的底层补丁
- 运行性能不会因为 `vendor` 产生额外负担，影响主要在仓库结构和后续依赖升级上
- 当前歌曲信息已经统一以 Rust 里的 MPRIS 读取为准，拉词脚本只负责搜词和落缓存
- 已增加带中文注释的 [配置.toml](/home/Arch/Downloads/vscode/LyricDock/配置.toml)
- 已将歌词目录拆成 `导入歌词/` 与 `缓存歌词/`
- 已将歌词解析、MPRIS、配置/窗口状态拆出 `main.rs`
- 已修正部分 `.yrc` 歌曲的时长解析问题
- 已将 `karaoke` 显示从“整句宽度裁剪”切换为“按片段推进”
