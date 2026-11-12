#!/usr/bin/env python3
"""AI 资讯聚合：定时抓取公开 RSS 源头条，AI 关键词过滤后生成 /var/www/qidi-portal/data/news.json。
部署位置：服务器 /opt/qidi-news/fetch_news.py，cron 每 6 小时一次。
只用 Python 标准库，无第三方依赖。"""
import json
import re
import html
import sys
import urllib.request
import xml.etree.ElementTree as ET
from datetime import datetime, timezone, timedelta
from email.utils import parsedate_to_datetime
from pathlib import Path

SOURCES = [
    # 机器之心/36氪的 /rss、/feed 已返回反爬 HTML 页，故用实测可用的源
    ("量子位", "https://www.qbitai.com/feed"),
    ("爱范儿", "https://www.ifanr.com/feed"),
    ("Hacker News", "https://hnrss.org/frontpage?points=150"),
]
KEYWORDS = re.compile(
    r"ai|人工智能|大模型|llm|gpt|claude|gemini|deepseek|qwen|openai|anthropic|agent|智能体|"
    r"机器学习|深度学习|神经网络|transformer|多模态|推理模型|微调|rag|上下文|扩散模型|"
    r"copilot|coding|代码生成|sora|midjourney|算力|芯片|gpu",
    re.I,
)
MAX_ITEMS = 24
SUMMARY_LEN = 90
UA = "Mozilla/5.0 (compatible; QidiNewsBot/1.0; www.qidiai.ltd)"
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/var/www/qidi-portal/data/news.json")
CST = timezone(timedelta(hours=8))


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=20) as r:
        return r.read()


def strip_html(s: str) -> str:
    s = re.sub(r"<[^>]+>", "", s or "")
    return re.sub(r"\s+", " ", html.unescape(s)).strip()


def parse_feed(name: str, raw: bytes):
    # 外部输入加固：拒绝实体定义（billion laughs / 外部实体的载体）；RSS 标准 DOCTYPE 声明本身无害，放行
    if b"<!ENTITY" in raw[:8192].upper():
        raise ValueError("feed 含实体定义，已拒绝解析")
    items = []
    root = ET.fromstring(raw)
    for node in root.iter("item"):
        title = strip_html(node.findtext("title", ""))
        link = (node.findtext("link") or "").strip()
        if not title or not link or not link.startswith("http"):
            continue
        desc = strip_html(node.findtext("description", ""))
        if "Comments URL" in desc:  # hnrss 样板文，不适合做摘要
            desc = ""
        desc = desc[:SUMMARY_LEN]
        dt = None
        try:
            dt = parsedate_to_datetime(node.findtext("pubDate", ""))
        except (TypeError, ValueError):
            pass
        date = dt.astimezone(CST).strftime("%m-%d %H:%M") if dt else ""
        items.append({"title": title[:120], "link": link, "source": name, "date": date, "summary": desc, "_dt": dt})
    return items


def main():
    all_items, seen = [], set()
    for name, url in SOURCES:
        try:
            for it in parse_feed(name, fetch(url)):
                key = it["link"].split("#")[0].rstrip("/")
                if key in seen:
                    continue
                if KEYWORDS.search(it["title"] + " " + it["summary"]):
                    seen.add(key)
                    all_items.append(it)
        except Exception as e:
            print(f"[warn] {name}: {e}", file=sys.stderr)

    # 跨源按发布时间倒序，避免单源高产挤掉其他源；无时间的排最后
    fallback = datetime(1970, 1, 1, tzinfo=CST)
    all_items.sort(key=lambda it: it["_dt"].astimezone(CST) if it["_dt"] else fallback, reverse=True)
    for it in all_items:
        del it["_dt"]

    payload = {
        "updated": datetime.now(CST).strftime("%Y-%m-%d %H:%M"),
        "items": all_items[:MAX_ITEMS],
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    tmp = OUT.with_suffix(".tmp")
    tmp.write_text(json.dumps(payload, ensure_ascii=False, indent=1), encoding="utf-8")
    tmp.replace(OUT)
    print(f"news.json 已更新：{len(payload['items'])} 条")


if __name__ == "__main__":
    main()
