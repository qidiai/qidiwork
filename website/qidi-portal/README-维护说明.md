# QIDI AI 门户维护说明（重构版 · 2026-09-21）

www.qidiai.ltd 已从"单文件 index.html"重构为 **多页静态站**：写内容不用碰 HTML，发布仍是一条 scp。

## 目录结构

```
qidi-portal/
├── src/
│   ├── pages/index.html      首页模板（含 @PRODUCT_CARDS 等占位符）
│   ├── assets/site.css|js    全站样式 / 运行时脚本（资讯流加载、终端动效）
│   ├── data/products.json    产品矩阵卡片
│   ├── data/cases.json       工程案例卡片
│   └── content/
│       ├── digest.md         首页"本周要点"（人工精选，frontmatter 填 date）
│       └── tech/*.md         技术方案文章，一篇一个文件
├── server/fetch_news.py      RSS 聚合脚本（部署在服务器 cron，生成 data/news.json）
├── build.mjs                 构建：模板+数据+Markdown → dist/ 纯静态页
├── deploy.sh                 一键构建+发布（scp）
└── legacy/                   重构前的单文件版存档
```

## 日常操作

```bash
npm install        # 首次
node build.mjs     # 构建到 dist/
npm run preview    # 本地预览 http://localhost:4173
./deploy.sh        # 发布上线（改动内容后唯一需要记住的命令）
```

- **改产品/案例**：编辑 `src/data/*.json`，字段说明见原 index.html 注释。
- **发技术方案**：在 `src/content/tech/` 新建 `英文slug.md`，头部写
  `title / date / summary / tags`，正文标准 Markdown（支持代码块、表格、引用）。
- **更本周要点**：改 `src/content/digest.md` 的列表和 date。
- 首页"技术方案"区自动取最新 3 篇；`/tech/` 列表页与 sitemap 自动生成。

## AI 资讯板块（双层）

1. **本周要点**（左栏）：人工精选，随 deploy 静态发布，内容完全可控。
2. **资讯流**（右栏）：服务器 cron 每 6 小时跑 `fetch_news.py`，聚合公开 RSS 头条
   （量子位 / 爱范儿 / Hacker News），AI 关键词过滤后按发布时间倒序写
   `/var/www/qidi-portal/data/news.json`，前端只展示标题+一句摘要+跳转原文。
   - 加源：改 fetch_news.py 顶部 `SOURCES`；机器之心、36氪的 feed 已返回反爬 HTML，勿再加回。
   - 手动刷新：`ssh ubuntu@111.231.166.204 python3 /opt/qidi-news/fetch_news.py`
   - deploy.sh **不会**覆盖线上 news.json。

## 联系我们（邮箱 + 微信群二维码）

- 微信群码 **7 天过期**（微信侧硬限制）。换码流程：新图覆盖
  `src/assets/wechat-group.png` → `./deploy.sh`，只更新一张图。
- 页面上已注明"过期请走邮箱"，即使忘换码也不误导访客。
- **不要**在网页内嵌在线客服/评论等交互功能：公安备案按"交互服务=否"申报且审核中，
  网站内交互会直接推翻申报口径。


## 备案口径（勿破坏）

- 每页页脚必须保留"启迪技术分享 · 皖ICP备2026028669号"（模板已统一处理）。
- 公安备案判定为"静态展示、无交互"：资讯流只放标题/摘要/外链，不引入评论、注册、全文转载；如未来要加交互功能，须先重新评估备案口径。
- 皖公网安备号下发后，补挂到 `src/pages/index.html` 页脚（所有页面同模板生成，改一处即可）。

## 服务器侧一次性部署（需 root/sudo 之外均可，ubuntu 用户即可）

```bash
sudo mkdir -p /opt/qidi-news
scp server/fetch_news.py ubuntu@111.231.166.204:/tmp/
ssh ubuntu@111.231.166.204 'sudo mv /tmp/fetch_news.py /opt/qidi-news/ && (crontab -l 2>/dev/null; echo "15 */6 * * * /usr/bin/python3 /opt/qidi-news/fetch_news.py >> /var/log/qidi-news.log 2>&1") | crontab -'
```

## 遗留注意

- build 不清空 dist/：删除或改名文章后，线上旧页面需手动 `ssh ... rm -r /var/www/qidi-portal/tech/<slug>`。
- 服务器上 9-21 之前散落的 `index.html.bak*` 待部署确认后统一清理。
