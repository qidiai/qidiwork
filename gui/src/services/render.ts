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

export function renderMarkdown(src: string): string {
  ensureHook();
  const html = marked.parse(src, { async: false }) as string;
  // dompurify 3.x 类型声明 TrustedHTML;未启用 requireTrustedTypesPolicy
  // 时运行时返回普通 string。
  return DOMPurify.sanitize(html, PURIFY_CONFIG) as unknown as string;
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
