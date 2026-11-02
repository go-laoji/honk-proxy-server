# Honk Proxy 正式品牌资源

用户于 2026-11-03 确认蓝色 H + 双向通路 Logo 及 LIGHT 配色。概念参考保留在 `docs/design/brand-visual-direction-v2.png`，正式文件均由本目录的矢量母版导出。

| 文件 | 用途 |
| --- | --- |
| `honk-mark.svg` | 唯一几何母版，蓝色主体与亮蓝上行箭头，透明背景 |
| `honk-mark-dark.svg` | 深色界面的白色主体与亮蓝箭头 |
| `honk-mark-mono.svg` / `honk-mark-white.svg` | 单色、反白标志 |
| `app-icon.svg` / `app-icon-1024.png` | 圆角蓝色应用图标，白色标志 |
| `png/` | 16、24、32、48、64、128、256、512、1024 像素透明版本 |
| `assets-manifest.json` | 母版及 68 个导出文件的 SHA-256 清单 |

产品文件：`web/console/public/brand/` 包含自适应 SVG favicon、ICO、16/32/48 PNG、180 像素 touch icon 与 192/512 图标；`src-tauri/icons/` 包含 Tauri 需要的 PNG/ICO/ICNS 与 Windows Store 尺寸，以及 22/44 像素透明单色托盘模板。iOS/Android 当前不属于产品运行平台，没有将临时导出的移动端资源纳入仓库。

在当前 checkout 重新导出：

```sh
pnpm install --frozen-lockfile
# 需要系统 librsvg 提供 rsvg-convert
pnpm brand:generate
```

脚本只从母版提取几何，通过 librsvg 渲染 PNG，并使用锁定版本的 Tauri CLI 制作原生图标容器。ICNS 记录经过稳定排序，消除上游 HashMap 遍历造成的字节差异，并保留旧 RGB/alpha 记录对的相邻关系。不要手工编辑导出文件；在隔离 worktree 修改母版后重新生成。原始 AI 概念图只用于设计参考，不参与产品构建，也不从其截图裁切图标。

小尺寸保留 H 的粗轮廓与互相分离的箭头；不同尺寸直接从 SVG 渲染。macOS 托盘使用单色透明 template，并由系统适配菜单栏外观；应用窗口、Dock 及其他平台使用彩色应用图标。
