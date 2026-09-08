# QidiWork GUI(办公工作台)

QIDI Code 的原生 GUI 办公界面(Tauri v2 + Vue 3)。方案与模块计划见
`docs/QidiWork-GUI办公界面方案-v2.md`;本 crate 是**独立 cargo workspace**
(主工程 `Cargo.toml` 已 `exclude = ["gui"]`)。

## 为什么独立 workspace

Tauri 依赖树庞大,并入主工程会显著拖慢 `cargo check` 并加剧编译内存压力
(docs/操作手册.md 5.1:内存不足会导致编译崩溃)。cf-acp-lib 等主工程
crate 自 M2 起以 path 依赖引入,跨 workspace 解析不受影响。

## 前置要求

- Node ≥ 20.19(vite 7 要求;本机 24.x)
- Rust ≥ 1.85(edition 2024)
- WebView2 Runtime(Win10/11 一般自带;本机已装 152.x)
- npm 建议走 npmmirror:cargo 走主工程 `.cargo/config.toml` 的 rsproxy

## 常用命令

```bash
cd gui
npm install                 # 首次(本仓库已提交 package-lock.json)
npm run build               # vue-tsc 类型检查 + vite 产物构建
cd src-tauri
cargo build                 # 首次约 9 分钟,增量也可能 10+ 分钟(改 tauri.conf 会触发较多重编)
cargo clippy --all-targets  # 提交前必跑(lints: unwrap/expect/panic/dbg = warn, unsafe = deny)
```

开发调试:`gui/` 下 `npm run dev`(起 vite,5173)后运行
`src-tauri/target/debug/qidiwork-gui.exe`(debug 构建加载 devUrl);
或直接 `npx tauri dev`。注意 vite watch 已排除 `src-tauri/`,
不要在无排除配置时并行 cargo build 与 vite dev(会 EBUSY)。

## 结构与模块约定

```
gui/src/               Vue3 前端(三区布局:工作区 / 对话+多Tab / 产物面板)
gui/src-tauri/src/     Rust 后端
  main.rs              薄装配层(模块边界约定见文件尾注释)
  commands.rs          (M2)#[tauri::command] IPC 命令
  acp/                 (M2/M3)ACP 协议层:cf-acp-lib client、session 管理
  process/             (M2)agent 子进程:spawn、Job Object、崩溃检测
gui/src-tauri/capabilities/  能力集:逐条授权,禁止批量放开(方案 D4)
```

## 安全基线(方案 v2 D4,M1 已落地部分)

- 生产 CSP:`script-src 'self'` + `object-src 'none'` + `base-uri 'self'`;
  `unsafe-inline` 仅存在于 devCsp(Vite 开发注入需要),禁止复制进生产。
- TODO(P1):文档预览 iframe 的 frame-src/sandbox 策略,随预览形态
  (srcdoc/blob/本地协议)定型时再开,现在保持关闭。
- IPC 白名单、DOMPurify、Job Object:M2/M4 落地。

## M2 已落地的决策与登记

已拍板(2026-09-08):日志 = tracing + tracing-subscriber(按日滚动至
app_log_dir);多开治理 = tauri-plugin-single-instance。

登记待办(k3 M2 审计 P1-4,建议内测版前落地):Job assign 竞态——spawn
与 AssignProcessToJobObject 之间存在孙进程逃逸窗口,标准修法是
CREATE_SUSPENDED 启动 → assign → ResumeThread(tokio 不直接支持,
需 std::process::Command + creation_flags 自行 resume)。

传输层可靠性契约(k3 M2 审计 P0-2 定稿):入站行走有界 mpsc 单消费者
可靠队列(`take_line_receiver`,协议帧不丢);写入走有界队列,满则
`Backpressure`(agent 假死信号);退出通知 broadcast(一次性事件)。
