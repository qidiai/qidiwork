// 技能面板数据源:Rust 只读扫描 ~/.qidi/skills/*/SKILL.md frontmatter
// (commands.rs skills_list)。GUI 只展示与触发,执行在 agent 内核侧。
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface SkillInfo {
  name: string;
  display_name: string;
  description: string;
}

const skills = ref<SkillInfo[]>([]);
let loaded = false;

/** 办公用户常用技能过滤(方案 v2 P1:office-* / bid-* 可点触发) */
export function isOfficeSkill(name: string): boolean {
  return name.startsWith("office-") || name.startsWith("bid-");
}

/** 应用初始化时调用一次;失败时清 loaded 标记允许后续重试,由调用方提示。 */
export async function initSkills(): Promise<void> {
  if (loaded) return;
  loaded = true;
  try {
    skills.value = await invoke<SkillInfo[]>("skills_list");
  } catch (e) {
    loaded = false;
    throw e; // 调用方决定如何提示
  }
}

export function useSkills() {
  return { skills };
}
