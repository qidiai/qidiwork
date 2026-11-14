//! 办公工作台数据面(方案 v2 P1):任务工作区扫描、产物卡片、fsnotify
//! 实时监听、系统打开(扩展名白名单 + 路径约束)。
//!
//! 数据契约与 office-artifact 技能(card.py)完全对齐:
//! `~/.qidi/office-workspaces/<task>/manifest.json` → `{task, artifacts:[...]}`。
//! GUI 对 manifest **只读**(方案 v2 D3),写操作全走 agent/card.py。

pub mod watch;

/// 标书导出 P0a(T1):模板与底座(数据模型 / docx 包 / 模板生成器)。
pub mod bid;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 系统打开的扩展名白名单(k3 M1 审计定稿:只收路径参数 + 白名单,
/// 不暴露通用 exec)。均为办公交付格式,调用系统默认程序(WPS/浏览器)。
/// manifest 大小上限(4 MiB)。
pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

pub const OPEN_ALLOWED_EXTENSIONS: &[&str] = &[
    "docx", "doc", "xlsx", "xls", "pptx", "ppt", "pdf", "png", "jpg", "jpeg", "html", "md", "txt",
];

/// 预览读取(office_read_file)专用白名单:在系统打开白名单上剔除 html。
/// html 字节进入 webview 即潜在 XSS(k3 放宽审计 W3):html 交给
/// 系统浏览器在 file:// 域打开(office_open),不进 webview。
pub const PREVIEW_READ_EXTENSIONS: &[&str] = &[
    "docx", "doc", "xlsx", "xls", "pptx", "ppt", "pdf", "png", "jpg", "jpeg", "md", "txt",
];

pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_DOCUMENT_BASE64_BYTES: usize = MAX_DOCUMENT_BYTES.div_ceil(3) * 4;
const MAX_DOCUMENT_SESSION_BYTES: usize = 64;
const DOCUMENT_EXTENSIONS: &[&str] = &[
    "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pdf", "txt", "md", "csv",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportedDocument {
    pub name: String,
    pub path: String,
    pub size: u64,
}

fn validate_document_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name.len() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
    {
        return Err("资料名必须是合法的单文件名".into());
    }
    let device = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_uppercase();
    let numbered_device = device
        .strip_prefix("COM")
        .or_else(|| device.strip_prefix("LPT"));
    if ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"].contains(&device.as_str())
        || numbered_device.is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    {
        return Err("资料名不能是保留设备名".into());
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !DOCUMENT_EXTENSIONS.contains(&ext.as_str()) {
        return Err("不支持的资料格式".into());
    }
    Ok(())
}

fn document_session_component(session_id: &str) -> Result<String, String> {
    if session_id.is_empty() || session_id.len() > MAX_DOCUMENT_SESSION_BYTES {
        return Err("会话 ID 长度必须为 1–64 字节".into());
    }
    // 小写十六进制在 Windows 大小写不敏感目录上也不会混淆会话。
    let mut encoded = String::with_capacity(session_id.len() * 2);
    for byte in session_id.bytes() {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(encoded)
}

fn reject_document_link(meta: &std::fs::Metadata) -> Result<(), String> {
    let linked = meta.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || meta.file_attributes() & 0x400 != 0 // 包括 junction/reparse point。
    };
    if linked {
        return Err("资料路径不能包含符号链接或重解析点".into());
    }
    Ok(())
}

fn document_cwd(cwd: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    if !cwd.is_absolute() || !is_local_absolute(cwd) {
        return Err("会话工作目录必须是本地绝对路径".into());
    }
    let mut part = PathBuf::new();
    for component in cwd.components() {
        match component {
            Component::ParentDir => return Err("会话工作目录不能包含 ..".into()),
            Component::Prefix(_) => {
                part.push(component.as_os_str());
                continue;
            }
            _ => part.push(component.as_os_str()),
        }
        let meta =
            std::fs::symlink_metadata(&part).map_err(|e| format!("检查工作目录失败: {e}"))?;
        reject_document_link(&meta)?;
        if !meta.is_dir() {
            return Err("会话工作目录路径不是目录".into());
        }
    }
    let canonical = std::fs::canonicalize(cwd).map_err(|e| format!("解析工作目录失败: {e}"))?;
    if !canonical.is_absolute() || !is_local_absolute(&canonical) {
        return Err("会话工作目录必须解析为本地绝对路径".into());
    }
    Ok(canonical)
}

fn checked_document_entry(
    path: &Path,
    root: &Path,
) -> Result<(PathBuf, std::fs::Metadata), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("检查资料路径失败: {e}"))?;
    reject_document_link(&meta)?;
    // Windows 长短路径都保留扩展前缀，避免同一目录因表示不同而误判越界。
    let canonical = std::fs::canonicalize(path).map_err(|e| format!("解析资料路径失败: {e}"))?;
    if !is_local_absolute(&canonical) || !canonical.starts_with(root) {
        return Err("资料路径超出会话目录".into());
    }
    Ok((canonical, meta))
}

fn document_directory(
    cwd: &Path,
    session_id: &str,
    create: bool,
) -> Result<Option<PathBuf>, String> {
    let session = document_session_component(session_id)?;
    let root = document_cwd(cwd)?;
    let mut dir = root.clone();
    for component in ["参考资料", session.as_str()] {
        dir.push(component);
        match std::fs::symlink_metadata(&dir) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                match std::fs::create_dir(&dir) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(format!("创建资料目录失败: {e}")),
                }
            }
            Err(e) => return Err(format!("检查资料目录失败: {e}")),
        }
        let (canonical, meta) = checked_document_entry(&dir, &root)?;
        if !meta.is_dir() {
            return Err("资料目录被普通文件占用".into());
        }
        dir = canonical;
    }
    Ok(Some(dir))
}

fn decode_document(data: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    if data.len() > MAX_DOCUMENT_BASE64_BYTES {
        return Err("资料 base64 超过 32 MiB 编码上限".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "资料不是有效的原始 base64".to_string())?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err("单份资料不能超过 32 MiB".into());
    }
    Ok(bytes)
}

/// cwd 只能来自桥登记;调用方须在阻塞线程执行。
pub fn import_document(
    cwd: &Path,
    session_id: &str,
    name: &str,
    data: &str,
) -> Result<ImportedDocument, String> {
    use std::io::Write as _;
    validate_document_name(name)?;
    if name.len() > 240 {
        return Err("导入资料名不能超过 240 字节(须为重名编号留空)".into());
    }
    document_session_component(session_id)?;
    let bytes = decode_document(data)?;
    let dir = document_directory(cwd, session_id, true)?.ok_or("资料目录不存在")?;
    let original = Path::new(name);
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("非法资料名")?;
    let ext = original
        .extension()
        .and_then(|s| s.to_str())
        .ok_or("非法资料扩展名")?;
    for index in 1..=10_000 {
        let name = if index == 1 {
            name.to_string()
        } else {
            format!("{stem} ({index}).{ext}")
        };
        let path = dir.join(&name);
        // 每次尝试都复查父目录;create_new 原子拒绝覆盖和最终文件链接。
        document_cwd(&dir)?;
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                reject_document_link(&meta)?;
                if !meta.is_file() {
                    return Err("同名资料路径不是普通文件".into());
                }
                checked_document_entry(&path, &dir)?;
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("检查同名资料失败: {e}")),
        }
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                checked_document_entry(&path, &dir)?;
                continue;
            }
            Err(e) => return Err(format!("创建资料文件失败: {e}")),
        };
        let result = (|| {
            file.write_all(&bytes)
                .map_err(|e| format!("写入资料失败: {e}"))?;
            file.sync_all().map_err(|e| format!("保存资料失败: {e}"))?;
            document_cwd(&dir)?;
            let (canonical, meta) = checked_document_entry(&path, &dir)?;
            if !meta.is_file() || meta.len() != bytes.len() as u64 {
                return Err("资料文件在写入期间发生变化".into());
            }
            Ok(ImportedDocument {
                name,
                path: dunce::simplified(&canonical)
                    .to_str()
                    .ok_or("资料路径不是 UTF-8")?
                    .to_string(),
                size: meta.len(),
            })
        })();
        drop(file);
        if result.is_err() && document_cwd(&dir).is_ok() {
            // 仅清理本次 create_new 成功的文件,不沿被替换的目录链接删除。
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(path = %path.display(), error = %e, "清理未完成资料失败");
            }
        }
        return result;
    }
    Err("同名资料数量过多".into())
}

/// 只枚举当前会话资料,不读写成果 manifest。
pub fn list_documents(cwd: &Path, session_id: &str) -> Result<Vec<ImportedDocument>, String> {
    let Some(dir) = document_directory(cwd, session_id, false)? else {
        return Ok(Vec::new());
    };
    let mut documents = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("枚举资料失败: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取资料目录项失败: {e}"))?;
        document_cwd(&dir)?;
        let (canonical, meta) = checked_document_entry(&entry.path(), &dir)?;
        if !meta.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_document_name(&name).is_err() {
            continue;
        }
        documents.push(ImportedDocument {
            name,
            path: dunce::simplified(&canonical)
                .to_str()
                .ok_or("资料路径不是 UTF-8")?
                .to_string(),
            size: meta.len(),
        });
    }
    documents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(documents)
}

/// 产物卡片(与 card.py / TUI ArtifactBlock 的 manifest schema 对齐)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactCard {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime: f64,
    pub registered_at: f64,
    pub skill: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceInfo {
    /// 工作区名(office-workspaces 下的子目录名,即 --task 值)。
    pub name: String,
    pub artifact_count: usize,
}

/// office-workspaces 根目录。
pub fn workspaces_root(home: &Path) -> PathBuf {
    home.join(".qidi").join("office-workspaces")
}

/// 扫描全部任务工作区(子目录,无论是否已有 manifest)。
pub fn list_workspaces(root: &Path) -> Vec<WorkspaceInfo> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let artifact_count = read_manifest(root, &name).map(|a| a.len()).unwrap_or(0);
        list.push(WorkspaceInfo {
            name,
            artifact_count,
        });
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// 解析某个任务的 manifest.json。文件不存在 → Ok(空);损坏 → Err。
/// task 名是路径拼接边界:拒绝分隔符与 `..`。
pub fn read_manifest(root: &Path, task: &str) -> Result<Vec<ArtifactCard>, String> {
    if task.contains(['/', '\\']) || task == ".." {
        return Err(format!("非法 task 名: {task}"));
    }
    let path = root.join(task).join("manifest.json");
    // 大小上限:异常大的 manifest 视为异常,拒绝解析(防 DoS)
    if let Ok(meta) = std::fs::metadata(&path)
        && meta.len() > MAX_MANIFEST_BYTES
    {
        return Err(format!(
            "manifest 超过 {}MB 上限,拒绝解析",
            MAX_MANIFEST_BYTES / (1024 * 1024)
        ));
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    #[derive(serde::Deserialize)]
    struct ManifestFile {
        #[serde(default)]
        artifacts: Vec<ArtifactCard>,
    }
    let manifest: ManifestFile =
        serde_json::from_str(&raw).map_err(|e| format!("解析 {} 失败: {e}", path.display()))?;
    Ok(manifest.artifacts)
}

/// Windows:仅本地盘符绝对路径(Disk/VerbatimDisk 前缀)。
/// UNC(\\host\share)会经 ShellExecute/SMB 触发 NTLM 认证造成哈希外泄,
/// 设备路径(\\.\)不可预测(k3 放宽审计 W2)。
#[cfg(windows)]
fn is_local_absolute(p: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(
        p.components().next(),
        Some(Component::Prefix(pre))
            if matches!(pre.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    )
}

/// 非 Windows:常规绝对路径。
#[cfg(not(windows))]
fn is_local_absolute(p: &Path) -> bool {
    matches!(p.components().next(), Some(std::path::Component::RootDir))
}

/// 预览读取/系统打开的放行门(k3 M1 审计 §IPC 定稿 + 2026-09-12 放宽):
/// ① **manifest 调解**——调用方只传 (task, name),服务端重读根内的
/// manifest.json 解析出路径,webview 永远不能传裸路径;② 扩展名白名单,
/// **以 canonicalize 之后的扩展名为准**(k3 放宽审计 W1:先查扩展名可被
/// 符号链接绕过——`x.docx` → 指向 `payload.exe`,系统打开即执行);
/// ③ canonicalize 后必须解析为本地盘符绝对路径(拒 UNC/设备/相对路径,W2)。
/// 原"落在 workspaces 根内"约束对读取/打开**已移除**:真实技能把交付物
/// 写在用户项目目录(如 E:\合肥方案\…)并登记原路径,根约束会拒绝全部
/// 真实产物(冒烟实测)。**删除**(office_delete_workspace)仍保留根约束。
pub fn open_allowed(path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    if !candidate.is_absolute() {
        return Err(format!("manifest 登记了相对路径,拒绝: {path}"));
    }
    if !candidate.exists() {
        return Err(format!("文件不存在: {path}"));
    }
    // 先解析符号链接/规范化,再对**最终路径**做白名单与盘符校验
    let canonical = dunce::canonicalize(candidate).map_err(|e| format!("路径解析失败: {e}"))?;
    if !is_local_absolute(&canonical) {
        return Err(format!("仅允许本地盘符路径,拒绝: {}", canonical.display()));
    }
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !OPEN_ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!("扩展名 .{ext} 不在白名单内,拒绝打开"));
    }
    Ok(canonical)
}

/// 预览读取(office_read_file)专用:在系统打开门之上剔除 html。
pub fn open_allowed_for_read(path: &str) -> Result<PathBuf, String> {
    let canonical = open_allowed(path)?;
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !PREVIEW_READ_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!("扩展名 .{ext} 不支持网页预览,请用系统程序打开"));
    }
    Ok(canonical)
}

/// 用系统默认程序打开文件(fire-and-forget)。
/// 经 tauri-plugin-opener(ShellExecuteW 语义),**不经过 cmd.exe**——
/// `cmd /c start` 对文件名做二次解析,& % ^ 等元字符即命令注入
/// (k3 P1a 审计 P0)。
pub fn open_in_system(path: &Path) {
    let Some(path_str) = path.to_str() else {
        tracing::warn!("打开失败:路径非 UTF-8: {path:?}");
        return;
    };
    if let Err(e) = tauri_plugin_opener::open_path(path_str, None::<&str>) {
        tracing::warn!("系统打开失败: {path_str}: {e}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::fs;

    fn fresh_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qidi-office-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn documents_chinese_names_do_not_overwrite_and_reload_from_disk() {
        let root = fresh_root("documents-chinese");
        let first = import_document(&root, "session", "报告.docx", "5Lit5paH").unwrap();
        let second = import_document(&root, "session", "报告.docx", "bmV3").unwrap();
        assert_eq!(first.name, "报告.docx");
        assert_eq!(first.size, 6);
        assert_eq!(second.name, "报告 (2).docx");
        assert_ne!(first.path, second.path);
        assert_eq!(fs::read(&first.path).unwrap(), "中文".as_bytes());
        assert_eq!(fs::read(&second.path).unwrap(), b"new");
        assert!(Path::new(&first.path).is_absolute());
        assert!(Path::new(&first.path).starts_with(dunce::canonicalize(&root).unwrap()));
        let dir = Path::new(&first.path).parent().unwrap();
        fs::write(dir.join("manifest.json"), "not a manifest").unwrap();
        fs::write(dir.join("ignore.exe"), "x").unwrap();
        fs::create_dir(dir.join("folder.txt")).unwrap();
        fs::write(&second.path, b"updated").unwrap();
        let listed = list_documents(&root, "session").unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&first));
        assert_eq!(
            listed.iter().find(|d| d.name == second.name).unwrap().size,
            7
        );
        assert_eq!(
            fs::read(dir.join("manifest.json")).unwrap(),
            b"not a manifest"
        );
        assert!(!root.join("manifest.json").exists());
    }

    #[test]
    fn documents_reject_invalid_names_and_extensions() {
        let root = fresh_root("documents-names");
        for name in [
            "",
            " ",
            ".",
            "..",
            "../a.txt",
            "a/b.txt",
            "a\\b.txt",
            "C:\\a.txt",
            "a:stream.txt",
            "a\0.txt",
            "a\n.txt",
            "a\u{7f}.txt",
            "a?.txt",
            "a*.txt",
            "a<.txt",
            "a>.txt",
            "a|.txt",
            "a\".txt",
            "a.txt.",
            "a.txt ",
            "CON.txt",
            "con.docx",
            "PRN.pdf",
            "aux.csv",
            "NUL.xlsx",
            "CON .txt",
            "COM1.txt",
            "com9.doc",
            "LPT1.ppt",
            "lpt9.pptx",
            "COM¹.txt",
            "LPT².txt",
            "CONIN$.txt",
            "CONOUT$.txt",
            "CLOCK$.txt",
            "a.exe",
            "a.html",
            "a.png",
            "a.zip",
            "a",
            "a.docx.exe",
        ] {
            assert!(
                import_document(&root, "s", name, "eA==").is_err(),
                "{name:?}"
            );
        }
        assert!(import_document(&root, "s", &format!("{}.txt", "a".repeat(237)), "eA==").is_err());
        assert!(!root.join("参考资料").exists());
        for ext in DOCUMENT_EXTENSIONS {
            assert!(validate_document_name(&format!("资料.{}", ext.to_uppercase())).is_ok());
        }
        let name = format!("{}.txt", "a".repeat(236));
        import_document(&root, "s", &name, "eA==").unwrap();
        import_document(&root, "s", &name, "eA==").unwrap();
        assert_eq!(list_documents(&root, "s").unwrap().len(), 2);
    }

    #[test]
    fn documents_validate_base64_before_creating_files() {
        let root = fresh_root("documents-base64");
        for data in [
            "???",
            "data:text/plain;base64,eA==",
            "eA=",
            "eA==\n",
            "____",
        ] {
            assert!(import_document(&root, "s", "a.txt", data).is_err());
        }
        assert!(!root.join("参考资料").exists());
        assert!(decode_document("").unwrap().is_empty());
    }

    #[test]
    fn documents_enforce_encoded_and_decoded_size_limits() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        {
            let oversized = "A".repeat(MAX_DOCUMENT_BASE64_BYTES + 1);
            assert!(
                decode_document(&oversized)
                    .unwrap_err()
                    .contains("编码上限")
            );
        }
        {
            let encoded = STANDARD.encode(vec![0; MAX_DOCUMENT_BYTES]);
            assert_eq!(decode_document(&encoded).unwrap().len(), MAX_DOCUMENT_BYTES);
        }
        {
            // 32 MiB + 1 与边界值的编码长度相同,必须有解码后校验。
            let encoded = STANDARD.encode(vec![0; MAX_DOCUMENT_BYTES + 1]);
            assert_eq!(encoded.len(), MAX_DOCUMENT_BASE64_BYTES);
            assert!(decode_document(&encoded).unwrap_err().contains("单份资料"));
        }
    }

    #[test]
    fn documents_are_isolated_by_session_and_cwd() {
        let root = fresh_root("documents-isolation");
        let other = fresh_root("documents-other-cwd");
        assert!(list_documents(&root, "missing").unwrap().is_empty());
        assert!(!root.join("参考资料").exists());
        let a = import_document(&root, "A/../会话", "资料.csv", "YQ==").unwrap();
        let b = import_document(&root, "a/../会话", "资料.csv", "Yg==").unwrap();
        assert_ne!(a.path.to_lowercase(), b.path.to_lowercase());
        assert_eq!(list_documents(&root, "A/../会话").unwrap(), vec![a]);
        assert_eq!(list_documents(&root, "a/../会话").unwrap(), vec![b]);
        assert!(list_documents(&other, "A/../会话").unwrap().is_empty());
        for id in ["".to_string(), "x".repeat(MAX_DOCUMENT_SESSION_BYTES + 1)] {
            assert!(import_document(&root, &id, "a.txt", "eA==").is_err());
            assert!(list_documents(&root, &id).is_err());
        }
    }

    #[test]
    fn documents_reject_invalid_cwd_and_directory_file_collisions() {
        let root = fresh_root("documents-cwd");
        for cwd in [
            Path::new("."),
            Path::new("relative"),
            Path::new("C:relative"),
        ] {
            assert!(import_document(cwd, "s", "a.txt", "eA==").is_err());
            assert!(list_documents(cwd, "s").is_err());
        }
        #[cfg(windows)]
        for cwd in [r"\\server\share", r"\\?\UNC\server\share", r"\\.\C:\"] {
            assert!(list_documents(Path::new(cwd), "s").is_err());
        }
        fs::write(root.join("参考资料"), "keep").unwrap();
        assert!(import_document(&root, "s", "a.txt", "eA==").is_err());
        assert!(list_documents(&root, "s").is_err());
        assert_eq!(fs::read(root.join("参考资料")).unwrap(), b"keep");
    }

    #[cfg(any(unix, windows))]
    fn document_test_symlink(target: &Path, link: &Path, directory: bool) -> bool {
        #[cfg(unix)]
        let result = {
            let _ = directory;
            std::os::unix::fs::symlink(target, link)
        };
        #[cfg(windows)]
        let result = if directory {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        #[cfg(windows)]
        if result
            .as_ref()
            .is_err_and(|e| e.raw_os_error() == Some(1314))
        {
            eprintln!("跳过 symlink 测试: Windows 未授予创建符号链接权限");
            return false;
        }
        result.unwrap();
        true
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn documents_reject_symlink_directories_at_every_level() {
        let outside = fresh_root("documents-link-outside");
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        fs::create_dir(outside.join("nested")).unwrap();
        for level in ["cwd", "ancestor", "references", "session"] {
            let root = fresh_root(&format!("documents-link-{level}"));
            let (cwd, link) = match level {
                "cwd" => (root.join("alias"), root.join("alias")),
                "ancestor" => (root.join("alias/nested"), root.join("alias")),
                "references" => (root.clone(), root.join("参考资料")),
                _ => {
                    fs::create_dir(root.join("参考资料")).unwrap();
                    (
                        root.clone(),
                        root.join("参考资料")
                            .join(document_session_component("s").unwrap()),
                    )
                }
            };
            if !document_test_symlink(&outside, &link, true) {
                return;
            }
            assert!(
                import_document(&cwd, "s", "a.txt", "eA==").is_err(),
                "{level}"
            );
            assert!(list_documents(&cwd, "s").is_err(), "{level}");
            assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep");
            assert!(!outside.join("a.txt").exists());
            assert!(!outside.join("参考资料").exists());
        }
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn documents_reject_symlink_files_including_dangling_links() {
        let outside = fresh_root("documents-file-link-outside");
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        for target in ["keep.txt", "missing.txt"] {
            let root = fresh_root(&format!("documents-file-link-{target}"));
            let dir = document_directory(&root, "s", true).unwrap().unwrap();
            if !document_test_symlink(&outside.join(target), &dir.join("a.txt"), false) {
                return;
            }
            assert!(import_document(&root, "s", "a.txt", "eA==").is_err());
            assert!(list_documents(&root, "s").is_err());
            assert!(!dir.join("a (2).txt").exists());
        }
        assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep");
        assert!(!outside.join("missing.txt").exists());
    }

    fn write_manifest(root: &Path, task: &str, artifacts: serde_json::Value) {
        let dir = root.join(task);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_string(&artifacts).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn list_workspaces_sorted_with_counts() {
        let root = fresh_root("list");
        write_manifest(
            &root,
            "bbb",
            serde_json::json!({"task":"bbb","artifacts":[
                {"path":"x.docx","name":"x.docx","size":1,"mtime":1.0,"registered_at":2.0,"skill":"s","note":""},
                {"path":"y.xlsx","name":"y.xlsx","size":1,"mtime":1.0,"registered_at":2.0,"skill":"s","note":""}
            ]}),
        );
        write_manifest(
            &root,
            "aaa",
            serde_json::json!({"task":"aaa","artifacts":[]}),
        );
        fs::create_dir_all(root.join("no-manifest")).unwrap();
        fs::write(root.join("a-file.txt"), "x").unwrap(); // 文件不算工作区

        let list = list_workspaces(&root);
        let names: Vec<&str> = list.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, vec!["aaa", "bbb", "no-manifest"]);
        assert_eq!(
            list.iter()
                .find(|w| w.name == "bbb")
                .unwrap()
                .artifact_count,
            2
        );
    }

    #[test]
    fn read_manifest_rejects_path_traversal() {
        let root = fresh_root("traverse");
        assert!(read_manifest(&root, "../other").is_err());
        assert!(read_manifest(&root, "a\\b").is_err());
        assert!(read_manifest(&root, "..").is_err());
    }

    #[test]
    fn read_manifest_missing_is_empty() {
        let root = fresh_root("missing");
        assert!(read_manifest(&root, "nope").unwrap().is_empty());
    }

    /// schema 回归(k3 P1a 审计 §3):fixture 直接取 card.py 真实产出的
    /// manifest(mtime/registered_at 为 float 秒,skill/note 可空)。
    #[test]
    fn real_card_py_manifest_parses() {
        let root = fresh_root("schema");
        let real = r#"{
 "task": "default",
 "artifacts": [
  {
   "path": "C:/Users/ASUS/.qidi/office-workspaces/default/office-e2e-report.txt",
   "name": "office-e2e-report.txt",
   "size": 43,
   "mtime": 1788624239.6703873,
   "registered_at": 1788624244.6703873,
   "skill": "office-tools",
   "note": ""
  },
  {
   "path": "C:/Users/ASUS/.qidi/office-workspaces/default/函.docx",
   "name": "函.docx",
   "size": 38000,
   "mtime": 1788624244.5,
   "registered_at": 1788624245.1,
   "skill": "bid-write",
   "note": "高亮"
  }
 ]
}"#;
        fs::create_dir_all(root.join("default")).unwrap();
        fs::write(root.join("default").join("manifest.json"), real).unwrap();
        // 显式允许:回归测试锚点——真实 schema 若解析失败,测试必须 panic
        #[allow(clippy::expect_used)]
        let cards = read_manifest(&root, "default").expect("真实 schema 必须可解析");
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].skill, "office-tools");
        assert!((cards[0].mtime - 1788624239.6703873).abs() < 1e-9);
        assert_eq!(cards[1].name, "函.docx", "中文文件名");
        assert_eq!(cards[1].note, "高亮");
    }

    #[test]
    fn open_allowed_enforces_extension_and_allows_registered_out_of_root() {
        let root = fresh_root("open");
        let inside = root.join("report.docx");
        fs::write(&inside, "x").unwrap();
        let inside_txt = root.join("notes.txt");
        fs::write(&inside_txt, "x").unwrap();
        // 真实技能把交付物写在项目目录并登记原路径(2026-09-12 放宽):
        // 根外的白名单格式文件必须放行(manifest 调解是信任边界)
        let outside_dir = std::env::temp_dir().join(format!("qidi-outside-{}", std::process::id()));
        fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("deliverable.docx");
        fs::write(&outside, "x").unwrap();
        let wrong_ext = root.join("archive.zip");
        fs::write(&wrong_ext, "x").unwrap();

        assert!(open_allowed(inside.to_str().unwrap()).is_ok());
        assert!(open_allowed(inside_txt.to_str().unwrap()).is_ok());
        assert!(
            open_allowed(wrong_ext.to_str().unwrap()).is_err(),
            "zip 不在白名单"
        );
        assert!(
            open_allowed(outside.to_str().unwrap()).is_ok(),
            "根外白名单格式放行(真实交付物路径)"
        );
        assert!(open_allowed(root.join("nope.docx").to_str().unwrap()).is_err());
    }

    #[test]
    fn open_allowed_rejects_relative_and_read_rejects_html() {
        let root = fresh_root("open2");
        // 相对路径拒绝(k3 放宽审计 W2:按 GUI 进程 CWD 解析不可预测)
        assert!(open_allowed("some/relative/report.md").is_err());

        // html:系统打开放行,预览读取拒绝(W3:html 字节进 webview=潜在 XSS)
        let page = root.join("page.html");
        fs::write(&page, "<p>x</p>").unwrap();
        assert!(open_allowed(page.to_str().unwrap()).is_ok());
        assert!(open_allowed_for_read(page.to_str().unwrap()).is_err());

        // md:两条链都放行
        let notes = root.join("notes.md");
        fs::write(&notes, "# t").unwrap();
        assert!(open_allowed_for_read(notes.to_str().unwrap()).is_ok());
    }
}
