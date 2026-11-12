/* 构建脚本：src/ 模板 + data JSON + Markdown 文章 → dist/ 纯静态站
   用法：npm run build（产物在 dist/，整体 scp 上服务器即上线） */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { Marked } from "marked";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.join(ROOT, "src");
const DIST = path.join(ROOT, "dist");

const md = new Marked({ gfm: true, breaks: false });
const read = p => fs.readFileSync(p, "utf8");
const write = (p, content) => {
  if (p.endsWith('.html')) {
    content = content.replace(/((?:src|href)=")((?:\/assets\/[^"?]+\.(?:css|js|svg))|analytics\.js|qidi-logo\.svg)"/g, (_, prefix, url) => {
      const source = path.join(SRC, url.startsWith('/assets/') ? url.slice(1) : `assets/${url}`);
      const version = createHash('sha256').update(fs.readFileSync(source)).digest('hex').slice(0, 12);
      return `${prefix}${url}?v=${version}"`;
    });
  }
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
};
const esc = s => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/* ---------- 数据 ---------- */
const products = JSON.parse(read(path.join(SRC, "data/products.json")));
const cases = JSON.parse(read(path.join(SRC, "data/cases.json")));

const statusMap = { on: "on", dev: "dev", int: "int", os: "os" };
const productCards = products.map(p => `
      <article class="card">
        <div class="top">
          <div class="icon">${p.icon}</div>
          <div>
            <h3>${esc(p.name)}</h3>
            <div class="en">${esc(p.en)}</div>
          </div>
          <span class="status ${statusMap[p.status] || "int"}"><span class="d"></span>${esc(p.statusText)}</span>
        </div>
        <p class="desc">${esc(p.desc)}</p>
        <div class="tags">${p.tags.map(t => `<span class="tag${t === "Apache-2.0" ? " os" : ""}">${esc(t)}</span>`).join("")}</div>
        ${p.link
          ? `<a class="go" href="${esc(p.link)}" target="_blank" rel="noopener">${esc(p.linkText || "访问")} →</a>`
          : `<span class="go na">未公开 · 内部/内测</span>`}
      </article>`).join("");

const caseCards = cases.map(c => `
      <article class="case">
        <div class="ind">${esc(c.ind)}</div>
        <h3>${esc(c.title)}</h3>
        <p>${esc(c.desc)}</p>
        <div class="foot">${c.chips.map(t => `<span class="tag">${esc(t)}</span>`).join("")}</div>
      </article>`).join("");

/* ---------- Markdown 文章（frontmatter: title/date/summary/tags） ---------- */
function parseFrontmatter(raw) {
  const m = raw.match(/^---\n([\s\S]*?)\n---\n?/);
  if (!m) throw new Error("文章缺少 frontmatter（--- 包裹的 title/date/summary/tags）");
  const meta = {};
  for (const line of m[1].split("\n")) {
    const i = line.indexOf(":");
    if (i < 0) continue;
    const k = line.slice(0, i).trim();
    const v = line.slice(i + 1).trim();
    if (v.startsWith("[") && v.endsWith("]")) {
      meta[k] = v.slice(1, -1).split(",").map(s => s.trim().replace(/^["']|["']$/g, "")).filter(Boolean);
    } else {
      meta[k] = v.replace(/^["']|["']$/g, "");
    }
  }
  return { meta, body: raw.slice(m[0].length) };
}

const postsDir = path.join(SRC, "content/tech");
const posts = fs.existsSync(postsDir)
  ? fs.readdirSync(postsDir).filter(f => f.endsWith(".md")).map(f => {
      const { meta, body } = parseFrontmatter(read(path.join(postsDir, f)));
      for (const k of ["title", "date", "summary"]) if (!meta[k]) throw new Error(`${f} 缺少 frontmatter 字段: ${k}`);
      return {
        slug: f.replace(/\.md$/, ""),
        title: meta.title, date: meta.date, summary: meta.summary,
        tags: meta.tags || [],
        html: md.parse(body),
      };
    }).sort((a, b) => b.date.localeCompare(a.date))
  : [];

/* ---------- 已发布 HTML 来源：保留原文，不继承旧导航及推送脚本 ---------- */
const published = [
  { slug: "free-token-resources", section: "tech", group: "免费资源分享", tags: ["免费额度", "模型鉴别"] },
  { slug: "free-quota-cheatsheet", section: "blog", group: "免费资源分享", tags: ["额度", "实测", "备忘"] },
  { slug: "qwen-image-6gb", section: "blog", group: "技术文章", tags: ["本地生图", "低显存", "实测"] },
].map(p => {
  const raw = read(path.join(SRC, `content/published/${p.slug}.html`));
  const required = (pattern, label) => {
    const match = raw.match(pattern);
    if (!match) throw new Error(`${p.slug} 缺少 ${label}`);
    return match[1];
  };
  const jsonLd = required(/(<script type="application\/ld\+json">[\s\S]*?<\/script>)/, "Article JSON-LD");
  const article = JSON.parse(jsonLd.match(/<script[^>]*>([\s\S]*?)<\/script>/)[1]);
  const canonical = required(/<link rel="canonical" href="([^"]+)"/, "canonical");
  if (article["@type"] !== "Article" || canonical !== `https://www.qidiai.ltd/${p.section}/${p.slug}/`
      || !article.datePublished || !article.dateModified) throw new Error(`${p.slug} 发布元数据不正确`);
  return {
    ...p, canonical, jsonLd,
    title: required(/<title>([\s\S]*?)<\/title>/, "title").replace(/\s*·\s*启迪技术分享(?: QIDI AI)?$/, ""),
    summary: required(/<meta name="description" content="([^"]*)"/, "description"),
    date: article.datePublished, lastmod: article.dateModified,
    html: required(/(<article\b[^>]*>[\s\S]*?<\/article>)/, "正文"),
    styles: (raw.match(/<style\b[^>]*>[\s\S]*?<\/style>/g) || []).join("\n"),
    ogImage: required(/<meta property="og:image" content="([^"]+)"/, "og:image"),
  };
});
const techPosts = [...posts, ...published.filter(p => p.section === "tech")]
  .sort((a, b) => b.date.localeCompare(a.date));
const blogPosts = [...posts.map(p => ({ ...p, section: "tech", group: "技术文章" })), ...published]
  .sort((a, b) => b.date.localeCompare(a.date));
function postCard(p) {
  return `
      <article class="techcard">
        <div class="date">${esc(p.date)}</div>
        <h3>${esc(p.title)}</h3>
        <p>${esc(p.summary)}</p>
        <div class="tags">${p.tags.map(t => `<span class="tag">${esc(t)}</span>`).join("")}</div>
        <a class="go" href="/${p.section || "tech"}/${p.slug}/">阅读全文 →</a>
      </article>`;
}

/* ---------- 公共页头/页脚（与首页模板保持同源样式） ---------- */
const homeTpl = read(path.join(SRC, "pages/index.html"));
const headMatch = homeTpl.match(/<head>[\s\S]*?<\/head>/)[0];
const footerMatch = homeTpl.match(/<footer>[\s\S]*?<\/footer>/)[0];

function page({ title, desc, canonical, body, navCur, ogType = "website", ogImage, extraHead = "" }) {
  let head = headMatch
    .replace(/<title>[\s\S]*?<\/title>/, () => `<title>${esc(title)} · 启迪技术分享 QIDI AI</title>`)
    .replace(/<meta name="description" content="[^"]*"/, () => `<meta name="description" content="${esc(desc)}"`)
    .replace(/<link rel="canonical" href="[^"]*"/, () => `<link rel="canonical" href="${esc(canonical)}"`)
    .replace(/<meta property="og:title" content="[^"]*"/, () => `<meta property="og:title" content="${esc(title)} · 启迪技术分享"`)
    .replace(/<meta property="og:description" content="[^"]*"/, () => `<meta property="og:description" content="${esc(desc)}"`)
    .replace(/<meta property="og:url" content="[^"]*"/, () => `<meta property="og:url" content="${esc(canonical)}"`)
    .replace(/<meta name="twitter:title" content="[^"]*"/, () => `<meta name="twitter:title" content="${esc(title)} · 启迪技术分享"`)
    .replace(/<meta name="twitter:description" content="[^"]*"/, () => `<meta name="twitter:description" content="${esc(desc)}"`)
    .replace(/<script type="application\/ld\+json">[\s\S]*?<\/script>\s*/, () => "")
    .replace(/<meta property="og:type" content="[^"]*"/, () => `<meta property="og:type" content="${esc(ogType)}"`);
  if (ogImage) {
    head = head.replace(/<meta (?:property="og:image"|name="twitter:image")[^>]*>\s*/g, "")
      .replace(/<meta name="twitter:card" content="[^"]*"/, '<meta name="twitter:card" content="summary_large_image"');
    extraHead += `\n<meta property="og:image" content="${esc(ogImage)}">\n<meta name="twitter:image" content="${esc(ogImage)}">`;
  }
  head = head.replace("</head>", () => `${extraHead}\n</head>`);
  const nav = homeTpl.match(/<header>[\s\S]*?<\/header>/)[0]
    .replace(/href="\/(tech|blog)\/"/g, (link, section) => `${link}${navCur === section ? ' class="cur" aria-current="page"' : ""}`);
  return `<!DOCTYPE html>
<html lang="zh-CN">
${head}
<body>
<div class="bg-fx"></div>
${nav}
<main id="top">
${body}
</main>
${footerMatch}
<script src="/assets/site.js" defer></script>
<script src="/assets/analytics.js" defer></script>
</body>
</html>
`;
}

/* ---------- 首页 ---------- */
const techCardsHome = techPosts.slice(0, 3).map(postCard).join("") || `
      <article class="techcard"><p>方案笔记整理中，敬请期待。</p></article>`;

const digestRaw = fs.existsSync(path.join(SRC, "content/digest.md")) ? read(path.join(SRC, "content/digest.md")) : "";
const digest = digestRaw ? parseFrontmatter(digestRaw) : { meta: { date: "—" }, body: "<p>本周要点整理中。</p>" };
const digestHtml = digest.body.trim() ? md.parse(digest.body) : "<p>本周要点整理中。</p>";

const homeFinal = homeTpl
  .replace("@PRODUCT_CARDS", () => productCards)
  .replace("@CASE_CARDS", () => caseCards)
  .replace("@TECH_CARDS", () => techCardsHome)
  .replace("@DIGEST_DATE", () => esc(digest.meta.date))
  .replace("@DIGEST_BODY", () => digestHtml.trim());
write(path.join(DIST, "index.html"), homeFinal);

/* ---------- /tech/ 列表页 ---------- */
const techIndexBody = `
  <section class="wrap" style="padding-top:64px">
    <div class="sec-head">
      <div class="kicker">技术实践与方案分享</div>
      <h2>技术方案</h2>
      <p>把真实项目里的架构选型与踩坑经验写成可复用的方案笔记，全部由启迪团队原创。</p>
    </div>
    <div class="grid tech">
${techPosts.map(postCard).join("") || `
      <article class="techcard"><p>方案笔记整理中，敬请期待。</p></article>`}
    </div>
  </section>`;
write(path.join(DIST, "tech/index.html"), page({
  title: "技术方案", desc: "启迪技术分享 · QIDI AI 原创技术方案与工程实践笔记。",
  canonical: "https://www.qidiai.ltd/tech/", navCur: "tech", body: techIndexBody,
}));

/* ---------- /blog/ 统一列表：保留文章原 URL ---------- */
write(path.join(DIST, "blog/index.html"), page({
  title: "技术博客", desc: "启迪技术分享 QIDI AI 技术博客：原创工程实践、模型实测与免费 Token 资源分享，持续更新。",
  canonical: "https://www.qidiai.ltd/blog/", navCur: "blog",
  body: `
  <section class="wrap" style="padding-top:64px">
    <div class="sec-head">
      <div class="kicker">BLOG</div>
      <h1>技术博客</h1>
      <p>启迪团队原创的工程实践、模型实测与免费资源整理，持续更新。</p>
    </div>
    ${["技术文章", "免费资源分享"].map(group => `
    <h2 style="margin:36px 0 14px;font-size:22px">${group}</h2>
    <div class="grid tech">${blogPosts.filter(p => p.group === group).map(postCard).join("")}</div>`).join("")}
  </section>`,
}));

/* ---------- 文章页 ---------- */
for (const p of posts) {
  const body = `
  <article class="post">
    <div class="crumb"><a href="/">首页</a> / <a href="/tech/">技术方案</a></div>
    <h1>${esc(p.title)}</h1>
    <div class="meta"><span>发布于 ${esc(p.date)}</span><span>${p.tags.map(esc).join(" · ")}</span></div>
    <div class="content">${p.html}</div>
    <a class="back" href="/tech/">← 返回技术方案列表</a>
  </article>`;
  write(path.join(DIST, `tech/${p.slug}/index.html`), page({
    title: p.title, desc: p.summary,
    canonical: `https://www.qidiai.ltd/tech/${p.slug}/`, navCur: "tech", body,
  }));
}

for (const p of published) {
  write(path.join(DIST, `${p.section}/${p.slug}/index.html`), page({
    title: p.title, desc: p.summary, canonical: p.canonical, navCur: p.section,
    body: p.html, ogType: "article", ogImage: p.ogImage, extraHead: `${p.styles}\n${p.jsonLd}`,
  }));
}

/* ---------- 静态资源 / 资讯种子数据 / sitemap ---------- */
fs.cpSync(path.join(SRC, "assets"), path.join(DIST, "assets"), { recursive: true });
fs.copyFileSync(path.join(SRC, "assets/og-cover.png"), path.join(DIST, "og-cover.png"));
fs.copyFileSync(path.join(SRC, "robots.txt"), path.join(DIST, "robots.txt"));
write(path.join(DIST, "admin/index.html"), read(path.join(SRC, "pages/admin.html")));
write(path.join(DIST, "download/index.html"), read(path.join(ROOT, "../qidiwork-download/index.html")));
write(path.join(DIST, "download/analytics.js"), read(path.join(SRC, "assets/analytics.js")));
fs.copyFileSync(path.join(SRC, "assets/qidi-logo.svg"), path.join(DIST, "download/qidi-logo.svg"));
fs.mkdirSync(path.join(DIST, "data"), { recursive: true });
if (!fs.existsSync(path.join(DIST, "data/news.json"))) {
  write(path.join(DIST, "data/news.json"), JSON.stringify({ updated: "", items: [] }, null, 2));
}

// 线上 2026-10-24 快照的 lastmod 不等于发布日期；仅首页使用本次改版日期。
const originalLastmod = { "rust-provider-abstraction": "2026-09-21", "multitenant-credential-layers": "2026-09-21" };
const urls = [
  { url: "https://www.qidiai.ltd/", lastmod: "2026-10-24" },
  { url: "https://www.qidiai.ltd/tech/", lastmod: "2026-10-20" },
  ...posts.map(p => ({ url: `https://www.qidiai.ltd/tech/${p.slug}/`, lastmod: originalLastmod[p.slug] || p.date })),
  { url: "https://www.qidiai.ltd/blog/", lastmod: "2026-10-21" },
  ...published.map(p => ({ url: p.canonical, lastmod: p.lastmod })),
];
write(path.join(DIST, "sitemap.xml"),
`<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls.map(u => `  <url><loc>${esc(u.url)}</loc><lastmod>${esc(u.lastmod)}</lastmod></url>`).join("\n")}
</urlset>
`);

console.log(`构建完成：${posts.length + published.length} 篇文章 → dist/`);
