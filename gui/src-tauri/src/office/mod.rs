//! 办公工作台数据面(方案 v2 P1):任务工作区扫描、产物卡片、fsnotify
//! 实时监听、系统打开(扩展名白名单 + 路径约束)。
//!
//! 数据契约与 office-artifact 技能(card.py)完全对齐:
//! `~/.qidi/office-workspaces/<task>/manifest.json` → `{task, artifacts:[...]}`。
//! GUI 对 manifest **只读**(方案 v2 D3),写操作全走 agent/card.py。

pub mod watch;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 系统打开的扩展名白名单(k3 M1 审计定稿:只收路径参数 + 白名单,
/// 不暴露通用 exec)。均为办公交付格式,调用系统默认程序(WPS/浏览器)。
/// manifest 大小上限(4 MiB)。
pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

pub const OPEN_ALLOWED_EXTENSIONS: &[&str] = &[
    "docx", "doc", "xlsx", "xls", "pptx", "ppt", "pdf", "png", "jpg", "jpeg", "html", "md", "txt",
];

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

/// 系统打开的两道约束(k3 M1 审计 §IPC):扩展名白名单 + canonicalize
/// 后必须落在 workspaces 根内。通过返回规范化路径。
pub fn open_allowed(path: &str, root: &Path) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    if !candidate.exists() {
        return Err(format!("文件不存在: {path}"));
    }
    let ext = candidate
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !OPEN_ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!("扩展名 .{ext} 不在白名单内,拒绝打开"));
    }
    let canonical = dunce::canonicalize(candidate).map_err(|e| format!("路径解析失败: {e}"))?;
    let canonical_root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if !canonical.starts_with(&canonical_root) {
        return Err(format!(
            "路径越界:{} 不在 {} 内",
            canonical.display(),
            canonical_root.display()
        ));
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
    fn open_allowed_enforces_extension_and_root() {
        let root = fresh_root("open");
        let inside = root.join("report.docx");
        fs::write(&inside, "x").unwrap();
        let inside_txt = root.join("notes.txt");
        fs::write(&inside_txt, "x").unwrap();
        let outside_dir = std::env::temp_dir().join(format!("qidi-outside-{}", std::process::id()));
        fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("evil.docx");
        fs::write(&outside, "x").unwrap();
        let wrong_ext = root.join("archive.zip");
        fs::write(&wrong_ext, "x").unwrap();

        assert!(open_allowed(inside.to_str().unwrap(), &root).is_ok());
        assert!(open_allowed(inside_txt.to_str().unwrap(), &root).is_ok());
        assert!(
            open_allowed(wrong_ext.to_str().unwrap(), &root).is_err(),
            "zip 不在白名单"
        );
        assert!(
            open_allowed(outside.to_str().unwrap(), &root).is_err(),
            "根外路径拒绝"
        );
        assert!(open_allowed(root.join("nope.docx").to_str().unwrap(), &root).is_err());
    }
}
