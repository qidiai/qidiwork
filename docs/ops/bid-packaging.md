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
| 大小 | **39,324,084 B（约 37.5 MB）** |
| 主程序 | `qidiwork-gui.exe`（18.86 MB） |
| 随包内核 | `qidiwork.exe`（132.5 MB，agent 内核，见 `src/process/mod.rs:40`） |
| 资源 | `templates\bid-engineering-blue-v1.docx`（4767 B） |
| 构建耗时 | 前端 ~4s + Rust release ~1m35s + NSIS |

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

**关键结论**：`EXISTS_DEV` 也为 true（本机存在源码树），但解析仍然命中安装目录——证明 `resource_dir` 优先于 dev 回退，安装态路径真实可用。

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
