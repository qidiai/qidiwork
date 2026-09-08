//! 产物卡片 Block —— 滚动区内嵌显示 office-artifact 登记的交付物。
//!
//! 数据源: `~/.qidi/office-workspaces/<task>/manifest.json`
//! （由 skills/office-artifact/scripts/card.py 写入）。
//!
//! 交互路线（P1 实现，本文件不含）: 卡片 Enter/点击 → `BlockViewerPane`
//! 预览面板（**不**复用 agent 会话耦合的 `PeekPanelState`）；`o` → 系统
//! 关联程序打开。本 Block 只负责滚动区内的只读呈现。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde::Deserialize;

use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{AccentStyle, BlockContext, BlockLine, BlockOutput, DisplayMode};
use crate::theme::Theme;

/// manifest.json 单条产物记录（与 card.py 写入 schema 对齐）。
#[derive(Clone, Debug, Deserialize)]
pub struct ArtifactCard {
    /// 产物绝对路径。
    pub path: String,
    /// 文件名（展示用）。
    pub name: String,
    /// 字节数。
    #[serde(default)]
    pub size: u64,
    /// 文件 mtime（epoch 秒，浮点）。
    #[serde(default)]
    pub mtime: f64,
    /// 登记时间（epoch 秒，浮点）。
    #[serde(default)]
    pub registered_at: f64,
    /// 产出 skill 名（如 bid-write / canvas）。
    #[serde(default)]
    pub skill: String,
    /// 备注（如 "高亮"）。
    #[serde(default)]
    pub note: String,
}

impl ArtifactCard {
    /// 人类可读大小（`36777` → `"36.0KB"`）。
    pub fn size_str(&self) -> String {
        if self.size < 1024 {
            format!("{}B", self.size)
        } else if self.size < 1024 * 1024 {
            format!("{:.1}KB", self.size as f64 / 1024.0)
        } else {
            format!("{:.1}MB", self.size as f64 / (1024.0 * 1024.0))
        }
    }

    /// 登记时间 `"MM-DD HH:MM"`；未登记（0）时返回空串。
    pub fn registered_str(&self) -> String {
        if self.registered_at <= 0.0 {
            return String::new();
        }
        chrono::DateTime::from_timestamp(self.registered_at as i64, 0)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// 渲染为单行卡片（expanded 模式下每个产物一行）。
    fn to_line(&self, theme: &Theme, highlighted: bool) -> Line<'static> {
        let marker = if highlighted { "› " } else { "  " };
        let name_style = if highlighted {
            Style::default()
                .fg(theme.accent_skill)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.accent_skill)
        };
        let mut spans = vec![
            Span::styled(marker, Style::default().fg(theme.accent_skill)),
            Span::styled(self.name.clone(), name_style),
            Span::raw("  "),
            Span::styled(self.size_str(), Style::default().fg(theme.gray_dim)),
        ];
        if !self.skill.is_empty() {
            spans.push(Span::raw("  ["));
            spans.push(Span::styled(
                self.skill.clone(),
                Style::default().fg(theme.accent_plan),
            ));
            spans.push(Span::raw("]"));
        }
        if !self.note.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                self.note.clone(),
                Style::default().fg(theme.gray),
            ));
        }
        let ts = self.registered_str();
        if !ts.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(ts, Style::default().fg(theme.gray_dim)));
        }
        Line::from(spans)
    }
}

/// manifest.json 顶层结构。
#[derive(Debug, Deserialize)]
struct Manifest {
    /// 任务名（保留字段，当前仅校验用）。
    #[serde(default)]
    #[allow(dead_code)]
    task: String,
    #[serde(default)]
    artifacts: Vec<ArtifactCard>,
}

/// 产物卡片列表 Block：折叠时一行摘要，展开时逐行列出产物。
#[derive(Clone, Debug, Default)]
pub struct ArtifactBlock {
    /// 已登记产物（按登记顺序）。
    pub cards: Vec<ArtifactCard>,
    /// 块内聚焦下标（P1 交互用；当前仅作高亮展示）。
    pub focused_idx: usize,
}

impl ArtifactBlock {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 `~/.qidi/office-workspaces/<task>/manifest.json` 加载。
    ///
    /// manifest 不存在视为"无产物"而非错误；存在但解析失败才返回 Err。
    /// 主目录解析与 watch 侧共用 [`office_home_dir`]（$HOME 优先，与
    /// card.py 的 expanduser 一致），保证 PTY 沙箱 HOME 与真实用户态一致。
    pub fn load_from_manifest(task: &str) -> Result<Self, String> {
        let Some(home) = crate::app::office_watch::office_home_dir() else {
            return Err("cannot locate home dir".to_string());
        };
        let path = home
            .join(".qidi")
            .join("office-workspaces")
            .join(task)
            .join("manifest.json");
        if !path.exists() {
            return Ok(Self::new());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .map_err(|e| format!("parse {}: {e}", path.display()))?;
        Ok(Self {
            cards: manifest.artifacts,
            focused_idx: 0,
        })
    }
}

impl BlockContent for ArtifactBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        let is_collapsed = ctx.mode == DisplayMode::Collapsed;

        if self.cards.is_empty() {
            let line = Line::from(Span::styled(
                "暂无产物 · 办公 skill 产出文件后自动登记至此",
                Style::default().fg(theme.gray_dim),
            ));
            return BlockOutput {
                lines: vec![BlockLine::styled(line).with_selection_range(Some(0))],
            };
        }

        if is_collapsed {
            // 折叠: 单行摘要 "产物 × N · 最新: b.docx"
            let latest = self.cards.last().map(|c| c.name.as_str()).unwrap_or("");
            let line = Line::from(vec![
                Span::styled(
                    format!("产物 × {}", self.cards.len()),
                    Style::default()
                        .fg(theme.accent_skill)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" · 最新: ", Style::default().fg(theme.gray_dim)),
                Span::styled(latest.to_string(), Style::default().fg(theme.accent_skill)),
            ]);
            return BlockOutput {
                lines: vec![BlockLine::styled(line).with_selection_range(Some(0))],
            };
        }

        // 展开: 逐行卡片 + 底部操作提示
        let mut lines: Vec<BlockLine> = self
            .cards
            .iter()
            .enumerate()
            .map(|(i, card)| {
                let highlighted = ctx.is_selected && i == self.focused_idx;
                BlockLine::styled(card.to_line(&theme, highlighted)).with_selection_range(Some(0))
            })
            .collect();
        lines.push(BlockLine::separator(Line::from(Span::styled(
            "Enter 预览 · o 系统打开 · [/] 切换",
            Style::default().fg(theme.gray_dim),
        ))));
        BlockOutput { lines }
    }

    fn accent(&self, _ctx: &BlockContext) -> Option<AccentStyle> {
        Some(AccentStyle::static_color(Theme::current().accent_skill))
    }

    fn has_bullet(&self, _ctx: &BlockContext) -> bool {
        true
    }

    fn is_foldable(&self) -> bool {
        true
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Expanded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_schema() {
        let json = r#"{
            "task": "C",
            "artifacts": [{
                "path": "C:\\Users\\ASUS\\.qidi\\_final_audit\\b.docx",
                "name": "b.docx",
                "size": 36777,
                "mtime": 1788163298.6010756,
                "registered_at": 1788163299.5021179,
                "skill": "canvas",
                "note": "高亮"
            }]
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.artifacts.len(), 1);
        let c = &m.artifacts[0];
        assert_eq!(c.name, "b.docx");
        assert_eq!(c.size, 36777);
        assert_eq!(c.skill, "canvas");
        assert_eq!(c.note, "高亮");
        assert_eq!(c.size_str(), "35.9KB");
        assert!(!c.registered_str().is_empty());
    }

    #[test]
    fn parse_manifest_missing_optional_fields() {
        let json = r#"{"artifacts": [{"path": "/tmp/a.xlsx", "name": "a.xlsx"}]}"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        let c = &m.artifacts[0];
        assert_eq!(c.size, 0);
        assert_eq!(c.skill, "");
        assert_eq!(c.registered_str(), "");
    }

    #[test]
    fn load_missing_manifest_is_empty_not_error() {
        let blk = ArtifactBlock::load_from_manifest("__nonexistent_task__").unwrap();
        assert!(blk.cards.is_empty());
    }
}
