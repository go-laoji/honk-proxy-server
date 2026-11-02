# Honk Proxy Logo 设计评审

- 关联任务：[#28](https://github.com/go-laoji/honk-proxy-server/issues/28)
- 分支：`feature/brand-logo`，使用 `git flow feature start brand-logo develop` 从 `develop` 创建。
- 状态：设计提案阶段，等待用户确认后才能替换产品资源。

## 设计方向

以 **H + 双向通路** 为核心：用两条简洁、互相咬合的路径构成 H，表达代理连接、转发与有序路由。沿用现有控制台的蓝色基调，主色 `#2563EB`，辅助色 `#38BDF8`，深色底 `#0F172A`。保持足够粗的轮廓与负空间，兼顾侧栏、浏览器标签、桌面应用及单色托盘用途。

先交付一张概念评审图，展示同一个标志的独立图形、Honk Proxy 字标、深浅背景及桌面应用图标效果。它是设计预览，不是已经完成的矢量母版或通过小尺寸验收的发行图标。确认后再制作几何一致的 SVG 母版、单色变体和各平台尺寸资源。

## 执行步骤

1. 检查实际 worktree、分支基点及原始未跟踪文件，创建 Issue 和功能分支。
2. 使用内置 imagegen 生成概念评审图，检查文字、图形一致性与深浅色用途，新增保存到本设计目录。
3. 提交提案材料并等待用户明确确认；此阶段不修改 `web/console`、`src-tauri` 或现有 Desktop bundle。
4. 用户确认后，制作确定性的 SVG 母版与单色托盘版本，统一下列产品入口。
5. 验证所有页面、前端构建资源及 Tauri 图标打包，再将真实结果回复 Issue 并关闭。

## 确认后的替换范围

| 位置 | 当前实现 | 确认后的工作 |
| --- | --- | --- |
| 共享导航与品牌 | `web/console/src/App.tsx` 中的 Lucide `Waypoints` 图形 | 统一使用确认后的标志与字标 |
| 浏览器标签 | `web/console` 的 18 个 HTML 入口当前没有 favicon | 所有入口引用统一 favicon，验证相对路径和构建资源 |
| Web / Desktop 共用产物 | `vite.config.ts` 构建并生成 `web-manifest.json`，Daemon 与 Tauri 共用 `dist` | 确保新资源进入构建与内容哈希 |
| Tauri 打包图标 | `src-tauri/tauri.conf.json` 引用 PNG、ICNS、ICO | 从同一母版生成所需尺寸，检查包内图标 |
| 桌面托盘 | `src-tauri/src/desktop.rs` 复用 `default_window_icon()` | 评估独立单色模板，验证实际图标引用 |

不更改应用名称、标识符或系统服务配置。原工作区 `/Users/jack/data/go-laoji/honk-proxy-server` 的原始未跟踪素材保持原样。

## 验证边界

提案阶段只检查设计文件和 Git 范围，不需要启动服务、编译 Desktop 或执行 Rust 测试。后续如需运行开发应用，必须设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`；不运行任何宿主网络配置写入验收，不覆盖已安装 Desktop bundle 或偏好。

## 概念图生成规格

使用内置 `image_gen` 工具；新图生成，无输入参考图，无 CLI/API 密钥路径。提示词如下：

```text
Use case: logo-brand
Asset type: a single polished brand concept review board for the existing Honk Proxy desktop application and web proxy control console.
Primary request: design one original, memorable logo, combining the capital letter H with two interlocking bidirectional routing paths. Use thick geometric strokes, restrained rounded corners, clear negative space, a compact balanced silhouette, and flat vector-like construction. The same exact logo must appear consistently in every application on the board. Make the H legible first; the routing meaning should be subtle. Keep the core mark simple enough to be reconstructed as clean SVG and readable at favicon size.
Scene/backdrop: sophisticated editorial brand presentation, landscape canvas, generous whitespace, warm off-white main background, a dark navy panel on the right, precise aligned grid. No photographic scene.
Composition/framing: large primary blue mark and the wordmark Honk Proxy on the upper left; below, a concise row of the same mark at small sizes and in monochrome; right, the same mark in white/light blue on dark navy, plus one premium macOS-style rounded square blue app icon with a white version of the same mark. The app tile can have subtle depth, but the logo itself must remain flat. Small sizes must be literal reductions of the large symbol, not alternate shapes.
Color palette: primary #2563EB, optional secondary #38BDF8, dark #0F172A, white. Match the existing blue/slate software console. Ensure the mark also works in one solid color; no gradient-dependent details.
Style/medium: refined professional software identity, clean optical balance, crisp edges, strong silhouette; no clip art, no stock icons, no shield, globe, padlock, bird mascot, node spaghetti, lightning bolt, or 3D ribbon.
Text (verbatim): Honk Proxy. Optional small labels only: LOGO CONCEPT, LIGHT, DARK, APP ICON, MONO. Do not add slogans, Chinese text, mock feature claims, or any other brand name.
Constraints: original design, one coherent proposal, consistent geometry across all repeated marks, high readability, no watermarks. This is a review board, not a production UI change.
```
