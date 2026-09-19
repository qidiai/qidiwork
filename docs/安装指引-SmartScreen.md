# QIDI 办公工作台 · Windows 安装指引

> 适用版本：v0.1.0 及以上 · 下载：<https://qidiwork.qidiai.ltd/>

## 安装步骤

1. **下载安装包**：在下载中心点击「下载 v0.1.0」，得到 `qidiwork-setup-0.1.0.exe`（约 34 MB）。
2. **（可选）校验文件**：在 PowerShell 中执行
   ```powershell
   Get-FileHash .\qidiwork-setup-0.1.0.exe -Algorithm SHA256
   ```
   应与 `https://qidiwork.qidiai.ltd/releases/latest.json` 中的 `sha256` 一致：
   `982ee34fe635e8f42d7c74ebd54e3303823624b04c2ee5ad1e43f7d4a15d0b67`
3. **运行安装程序**：双击安装包。
4. **首次启动**：安装勾选"运行"时程序会自动打开；也可从桌面/开始菜单启动「QIDI 办公工作台」。
5. **配置模型**：首次使用请在设置面板填入模型 API Key（支持多家模型服务商），保存后即可对话。

## 遇到 SmartScreen 蓝色提示怎么办

因为我们当前未购买代码签名证书，Windows 11/10 首次运行安装包时可能弹出蓝色窗口
「Windows 已保护你的电脑（Windows protected your PC）」。这是**正常现象**，不代表文件有毒：

1. 点击提示框中的「**更多信息**」（"More info"）；
2. 再点击出现的「**仍要运行**」（"Run anyway"）按钮。

> 若个别杀软误报，可将安装目录加入信任白名单；也欢迎先用上面第 2 步的哈希与官网比对。

## UAC 管理员授权

安装过程会弹出一次「用户账户控制」确认框，点「是」即可。程序默认安装在
`%LOCALAPPDATA%\QidiWork Workbench\`，卸载通过控制面板或开始菜单的「卸载」项。

## 安装后目录结构

```
QidiWork Workbench\
├── qidiwork-gui.exe    # 桌面工作台主程序
├── qidiwork.exe        # AI 内核（随包内置，自动被主程序拉起，勿单独删除）
└── uninstall.exe
```

## 更新

当前版本更新方式为：从下载中心重新下载最新安装包覆盖安装（配置与会话数据不受影响）。
应用内自动更新（`检查更新`）将在后续版本开放。

## 常见问题

| 现象 | 处理 |
|---|---|
| 双击安装包无反应 | 任务管理器结束残留的 `QidiWork*setup*` 进程后重试；确认已点 SmartScreen 的「仍要运行」 |
| 打开后状态栏显示"未连接" | 说明内核未拉起：确认 `qidiwork.exe` 与主程序在同一目录；杀毒软件隔离区找回 |
| 对话无回复 | 检查设置面板中 API Key 是否有效、网络可达所选模型服务商 |
