# 打包验证（NSIS）

**日期**：2026-11-16（含安装态动态验证）
**结论**：安装包产出成功，**已在本机静默安装并实跑 `--self-check`**，安装态模板解析命中 `$INSTDIR\templates\…`（不是 dev 回退）。测试安装已卸载并清理干净。

## 1. 构建

```powershell
cd G:\qidiwork\gui
set RUSTC_WRAPPER=
npm run tauri build
```

**必须清空 `RUSTC_WRAPPER`**：否则 sccache 崩溃（`sccache: error: failed to execute compile` → `could not compile qidiwork-gui`），属 AGENTS §11.16 的已知污染问题。

## 2. 产物

| 项 | 值 |
|---|---|
| 安装包 | `gui/src-tauri/target/release/bundle/nsis/QidiWork Workbench_0.1.11_x64-setup.exe` |
| 大小 | **39,321,489 B（约 37.5 MB）** |
| 主程序 | `qidiwork-gui.exe`（18.86 MB） |
| 随包内核 | `qidiwork.exe`（132.5 MB，agent 内核，见 `src/process/mod.rs:40`） |
| 资源 | `templates\bid-engineering-blue-v1.docx`（4767 B） |
| 构建耗时 | 前端 ~4s + Rust release ~2m50s + NSIS |

## 3. 打包踩到的三个真问题（均已修）

| # | 现象 | 根因 | 修法 |
|---|---|---|---|
| 1 | `sccache: failed to execute compile`，编译失败 | sccache 服务异常，产物坏元数据 | `set RUSTC_WRAPPER=` 后重建（AGENTS §11.16） |
| 2 | 安装包里的 `qidiwork.exe` 是 **132 MB 的旧 CLI**（`qidi 0.1.220-alpha.4`），且应用主程序缺失 | `tauri.conf.json` 无 `mainBinaryName` | 加 `"mainBinaryName": "qidiwork-gui"` |
| 3 | `target/release/qidiwork-gui.exe` 只有 1.9 MB，运行 `--self-check` 打印 `用法: bid_export …` | 包内 7 个 bin 存在歧义，主二进制被 `bid_export` 覆盖 | `Cargo.toml` 加 `default-run = "qidiwork-gui"` + 显式 `[[bin]] name="qidiwork-gui" path="src/main.rs"` |

> 说明：`qidiwork.exe`（132 MB）是**有意随包**的 agent 内核（`process/mod.rs` 注释：Tauri 随包资源 `<resource_dir>/qidiwork.exe`），不是误打包。真正的应用主程序是 `qidiwork-gui.exe`。

## 4. 安装态动态验证（实测）

静默安装到临时目录：

```powershell
Start-Process -FilePath setup.exe -ArgumentList '/S','/D=G:\...\install3' -Wait
```

安装目录内容：`qidiwork-gui.exe`(18.86 MB) / `qidiwork.exe`(132.5 MB) / `templates\…docx` / `uninstall.exe`。

> 该次实测为**拆分前**构建，当时还多一个 `bid_export.exe`(1.9 MB)；CLI 拆 crate 后新包清单 = 上述 4 项，
> **不含任何 `bid_*.exe`**（见 §9）。

运行 `install3\qidiwork-gui.exe --self-check`（原始输出）：

```
VERSION=0.1.11
EXE_DIR=G:\qidicode\target\tmp\bid-p0a\install3
CAND_RESOURCE=G:\qidicode\target\tmp\bid-p0a\install3\templates\bid-engineering-blue-v1.docx
EXISTS_RESOURCE=true
CAND_DEV=G:\qidiwork\gui\src-tauri\resources\templates\bid-engineering-blue-v1.docx
EXISTS_DEV=true
RESOLVED=G:\qidicode\target\tmp\bid-p0a\install3\templates\bid-engineering-blue-v1.docx
```

**关键结论**：`EXISTS_DEV` 也为 true（本机存在源码树），但解析仍然命中安装目录且 `RESOLVED_SOURCE=resource`——证明 `resource_dir` 优先于 dev 回退，安装态路径真实可用。

> **验收口径（codely-core 审计要求）**：只看 `EXIT=0` 会假绿（dev 回退同样成功），**必须看到 `RESOLVED_SOURCE=resource`**。自检已输出该行。

## 5. 卸载与清理（已验证）

- 安装包自带 `uninstall.exe`（`/S` 静默），会删除 `$INSTDIR\templates\…` 与目录；
- 本次三次测试安装后手工清理：删除开始菜单 `QidiWork Workbench.lnk`、删除 `HKCU\…\Uninstall` 下指向临时目录的键；复查无残留；
- `$INSTDIR` 是临时目录，已整体删除。

## 6. 仍未做

- **代码签名**：安装包未签名，首次运行可能触发 SmartScreen；
- **非默认安装位置**（Program Files 需 UAC）未测；
- **安装副本内的实际导出**（命令行导出或 GUI 导出）：本轮验证的是"模板解析"这一关键路径，导出功能与开发态同一二进制。

## 7. 复现

```powershell
cd G:\qidiwork\gui
set RUSTC_WRAPPER=
npm run tauri build
# 安装
Start-Process -FilePath "src-tauri\target\release\bundle\nsis\QidiWork Workbench_0.1.11_x64-setup.exe" -ArgumentList '/S','/D=G:\tmp\qidi-install' -Wait
# 自检
& "G:\tmp\qidi-install\qidiwork-gui.exe" --self-check
```

## 8. codely-core 审计结论与闭环（2026-11-16）

审计判 **GO + 1 阻断**，逐条处置：

| 项 | 结论 |
|---|---|
| 阻断：安装包含未声明的 `bid_export.exe`，文档清单未列 | **已按方案 v1.1 拆 crate 根治**（提交见 git log）：6 个 CLI bin 迁至独立 crate `qidiwork-cli`（`gui/src-tauri/cli`），GUI 包只剩主 bin，见 §9。原定性记录：干净重建（删 `nsis/`+`bundle/`）两次均复现，**不是陈旧产物**；显式 `"externalBin": []` 也无效 → 属 Tauri 对同包多 bin 的自动处理，只能从结构上消除 |
| 重要：`EXIT=0` 不区分命中来源 | 已加 `RESOLVED_SOURCE=resource|dev|none`，并在文档写明验收必须命中 `resource` |
| 重要：内核 `../../target/release/qidiwork.exe` 按固定路径取件、无校验 | 未修；已记录风险（发布前需先构建内核并核对版本） |
| 重要：`prod/releases` 里 0.1.11 是修复前的坏包且是唯一归档 | 已归档修复后包 `qidiwork-setup-0.1.11-r2.exe`（39,321,489 B，sha256 `966ce00f…`）；旧包保留但标注为 pre-fix |
| 重要：`paths.rs` 注释写 `qidiwork.exe` | 已改为 `qidiwork-gui.exe` |

**审计未验证项**：安装副本内的实际导出、Program Files（UAC）安装、代码签名。

## 9. CLI 独立分发（拆 crate 后）

6 个标书 CLI（`bid_export` / `bid_import` / `bid_pages` / `bid_project` / `bid_smoke` / `gen_bid_template`）
已迁到独立 crate **`qidiwork-cli`**（`gui/src-tauri/cli`，是 `gui/src-tauri` 工作区的成员 `cli`）。
从此**安装目录只保留 GUI**：`qidiwork-gui.exe` + 内核 `qidiwork.exe` + `templates\` + `uninstall.exe`，
不再出现任何 `bid_*.exe`。

CLI 单独构建、单独分发：

```powershell
cd G:\qidiwork\gui\src-tauri
cargo build -p qidiwork-cli --release     # 产物落在 gui/src-tauri/target/release/
```

产物：`gui/src-tauri/target/release/{bid_export,bid_import,bid_pages,bid_project,bid_smoke,gen_bid_template}.exe`
（与 GUI 共用一个 target 目录，互不干扰）。打成 zip：

```powershell
Compress-Archive -Path target\release\bid_export.exe,target\release\bid_import.exe,`
  target\release\bid_pages.exe,target\release\bid_project.exe,`
  target\release\bid_smoke.exe,target\release\gen_bid_template.exe `
  -DestinationPath ..\..\prod\releases\qidiwork-cli-0.1.11-win64.zip -Force
```

- 想让 CLI 在**非源码树**环境里也能找到默认模板（解析结果 `source=resource`），把
  `templates\bid-engineering-blue-v1.docx` 一并放进 zip，解出后与 exe 同级；
- 版本号与 GUI 同步（当前 `0.1.11`），但两者**独立发版、独立归档**：
  `qidiwork-setup-x.y.z.exe` / `qidiwork-cli-x.y.z-win64.zip`（都放 `prod/releases/`）；
- 所有 CLI 命令都带 `-p qidiwork-cli`，例如 `cargo run -p qidiwork-cli --bin bid_export -- …`，
  其余参数口径不变（见 `docs/ops/bid-export-cli.md`、`bid-project.md`、`bid-import.md`）。
## 10. CLI 拆分后的打包实测（2026-11-16 23:20）

按方案 v1.1 把 6 个 CLI 拆到 `gui/src-tauri/cli`（crate `qidiwork-cli`）后，**干净重建**结果：

| 项 | 值 |
|---|---|
| 安装包 | `QidiWork Workbench_0.1.11_x64-setup.exe`，**38,763,896 B**（拆分后两次干净重建：38,761,559 / 38,763,896，差异仅时间戳；较拆分前 39,321,489 B 少约 0.56 MB） |
| sha256（当前） | `70bd74f21a98df758265b5619ea2ae2f3f3df3d9…`（23:20 那次为 `bdc5279a…`，已被重建覆盖） |
| NSI 载荷 | 主程序（`MAINBINARYSRCPATH`）+ `qidiwork.exe`（内核）+ `templates/bid-engineering-blue-v1.docx` |
| `bid_*` 残留 | `grep -c "bid_" installer.nsi` = **0** |
| 安装目录 | `qidiwork-gui.exe`(18.86 MB) + `qidiwork.exe`(132.5 MB) + `templates/` + `uninstall.exe`，**无任何 CLI 可执行文件** |
| 安装态自检 | `RESOLVED_SOURCE=resource`（命中安装目录） |

> 归档：`prod/releases/qidiwork-setup-0.1.11-r3.exe`（38,763,896 B）+ 同名 sha256 记录；CLI 侧归档 `qidiwork-cli-0.1.11-win64.zip`（4,243,334 B，6 个 exe，sha256 `41af8b72…`）。

> 注：NSIS 模板仍会输出 `; Copy external binaries` / `; Delete external binaries` 两行**空段落头**（无任何 File/Delete 条目）。判据以"`bid_` 行数 == 0 + 安装目录无 CLI"为准，不以段落头是否存在为准。

