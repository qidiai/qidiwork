# 打包验证（NSIS）

**日期**：2026-11-16
**结论**：安装包**已成功产出**，模板资源确认随包落位；"从安装副本运行"这一动态验证**尚未做**。

## 1. 构建

```powershell
cd G:\qidiwork\gui
npm run tauri build
```

**必须清空 `RUSTC_WRAPPER`**：首次构建时 sccache 崩溃（`sccache: error: failed to execute compile` → `could not compile qidiwork-gui`），属于 AGENTS §11.16 记录过的 sccache 污染问题。重跑方式：

```powershell
set RUSTC_WRAPPER=
npm run tauri build
```

## 2. 产物

| 项 | 值 |
|---|---|
| 安装包 | `gui/src-tauri/target/release/bundle/nsis/QidiWork Workbench_0.1.11_x64-setup.exe` |
| 大小 | 34,848,985 B（约 33.2 MB） |
| sha256（前 32 位） | `96adf2bf81298bfcf447d4bcf4121a79…` |
| 主程序 | `target/release/qidiwork.exe`（126.4 MB） |
| 暂存资源 | `target/release/templates/bid-engineering-blue-v1.docx`（4767 B） |
| 构建耗时 | 前端 4.2s + Rust release 2m53s + NSIS 打包 |

## 3. 模板是否随包

NSIS 脚本（`target/release/nsis/x64/installer.nsi`）明确包含：

```nsis
CreateDirectory "$INSTDIR\templates"
File /a "/oname=templates\bid-engineering-blue-v1.docx" "...\resources\templates\bid-engineering-blue-v1.docx"
```

卸载段也成对删除（`Delete "$INSTDIR\templates\…"` + `RMDir`）。

安装后的布局：

```
<install>\qidiwork.exe
<install>\templates\bid-engineering-blue-v1.docx
```

`paths::resolve_from(style_id, Some(resource_dir))` 会先查 `resource_dir()/templates/<id>.docx`，因此安装态命中该文件；开发态仍回退 `CARGO_MANIFEST_DIR/resources/templates/`。

## 4. 已验证 / 未验证

**已验证（静态 + 构建产物）**
- 安装包成功生成，`bundle.resources` 键生效；
- 资源被复制到 `target/release/templates/` 并被 NSIS `File` 指令引用；
- 卸载脚本会清理该文件与目录。

**未验证（需人工/后续）**
- **从安装副本启动**后 `bid_export_docx` / `bid_export_md` 能否解析模板（需要真机安装一次；会写开始菜单与卸载项，属系统变更，未自动执行）；
- 非管理员安装（`$LOCALAPPDATA`）与 Program Files 安装两种路径；
- 安装后被 Windows Defender/SmartScreen 拦截的情况（未签名）。

## 5. 给使用者的最短路径

1. 运行 `QidiWork Workbench_0.1.11_x64-setup.exe` 安装；
2. 打开应用 →「导出标书」→ 选 Markdown → 导出；
3. 若报"模板未找到"，检查 `<安装目录>\templates\bid-engineering-blue-v1.docx` 是否存在。

## 6. 代码签名（未做）

安装包**未签名**，首次运行可能触发 SmartScreen 提示。正式分发前建议购买代码签名证书，或在内部分发说明中注明"允许运行"。
