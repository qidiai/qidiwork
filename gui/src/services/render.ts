// Markdown 安全渲染:marked 解析 + DOMPurify 白名单收敛。
// agent 产出按不可信输入处理(方案 v2 §D4),审计定稿(k3 M4):
// - 显式 ALLOWED_TAGS/ATTR,不依赖默认放行面;
// - URI 只放行 https?/mailto(封 tel/ftp/javascript 等处理程序唤起);
// - 链接强制 target=_blank + rel=noopener,点击统一经 opener 插件走
//   系统浏览器,禁止 webview 内导航。
import { marked } from "marked";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";

const PURIFY_CONFIG: Parameters<typeof DOMPurify.sanitize>[1] = {
  ALLOWED_TAGS: [
    "p", "br", "strong", "em", "b", "i", "u", "s", "del", "code", "pre",
    "ul", "ol", "li", "a", "h1", "h2", "h3", "h4", "h5", "h6",
    "blockquote", "table", "thead", "tbody", "tr", "th", "td", "hr", "span",
  ],
  ALLOWED_ATTR: ["href"],
  ALLOWED_URI_REGEXP: /^(?:https?|mailto):/i,
};

let hookInstalled = false;

function ensureHook(): void {
  if (hookInstalled) return;
  hookInstalled = true;
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "A") {
      node.setAttribute("target", "_blank");
      node.setAttribute("rel", "noopener noreferrer");
    }
  });
}

// 渲染结果记忆化(性能审计 2026-09-19):模板内联调用在任意流式
// chunk 到达时会对全部历史消息重算,已完成消息内容不变 → 命中缓存。
// 上限防止长会话内存膨胀,满则整体清空(简单可靠,重算成本低)。
const MD_CACHE_MAX = 300;
const mdCache = new Map<string, string>();

export function renderMarkdown(src: string): string {
  const hit = mdCache.get(src);
  if (hit !== undefined) return hit;
  ensureHook();
  const html = marked.parse(src, { async: false }) as string;
  // dompurify 3.x 类型声明 TrustedHTML;未启用 requireTrustedTypesPolicy
  // 时运行时返回普通 string。
  const out = DOMPurify.sanitize(html, PURIFY_CONFIG) as unknown as string;
  if (mdCache.size >= MD_CACHE_MAX) mdCache.clear();
  mdCache.set(src, out);
  return out;
}

/** 消息区点击拦截:外链交系统处理(https→浏览器,mailto→邮件客户端),
 * 其余协议不动作;一律阻止 webview 导航。与 ALLOWED_URI_REGEXP 白名单
 * 保持同集(k3 审计 W4:mailto 此前被放行渲染却被点击处理吞掉)。 */
export function handleLinkClick(e: MouseEvent): void {
  const target = e.target as HTMLElement | null;
  const anchor = target?.closest?.("a");
  if (!anchor) return;
  e.preventDefault();
  e.stopPropagation();
  const href = anchor.getAttribute("href") ?? "";
  if (/^(?:https?|mailto):/i.test(href)) {
    void openUrl(href);
  }
}
