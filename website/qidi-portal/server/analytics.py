"""启迪官网的无 Cookie、按北京时间逐日去重的估算访问统计。仅使用标准库。"""
from __future__ import annotations

import argparse
from collections import OrderedDict
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import mimetypes
import os
from pathlib import Path
import re
import secrets
import socket
import sqlite3
import stat
import threading
import time
from urllib.parse import parse_qs, unquote, urlsplit

PROJECT_ROOT = Path(__file__).resolve().parents[1]
BEIJING = timezone(timedelta(hours=8), "Asia/Shanghai")
EVENTS = ("pageview", "download_click", "source_click", "product_click", "contact_click")
CLICKS = EVENTS[1:]
DIRECT = "直接/站内"
MAX_BODY = 2048
BOT = re.compile(r"bot|crawl|spider|slurp|headless|preview|facebookexternalhit|curl|wget|python-requests|httpclient|lighthouse|pingdom|uptime", re.I)
PAGE_PATH = re.compile(r"/(?:download/|(?:tech|blog)/(?:[a-zA-Z0-9][a-zA-Z0-9_-]{0,95}/)?)?\Z")
HOST_LABEL = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\Z")


def utc_now():
    return datetime.now(timezone.utc)


def hostname(value):
    """只接受 HTTP(S) URL；查询、路径与片段永不进入数据库。"""
    if not isinstance(value, str) or len(value) > 1024:
        raise ValueError("来源格式不正确")
    if any(ord(c) < 33 or ord(c) == 127 for c in value) or "\\" in value:
        raise ValueError("来源格式不正确")
    try:
        parts = urlsplit(value)
        host = parts.hostname
        if parts.scheme not in ("http", "https") or not host or parts.username is not None or parts.password is not None:
            raise ValueError
        if parts.port is not None and not 1 <= parts.port <= 65535:
            raise ValueError
        if "%" in host:
            raise ValueError
        try:
            host = str(ipaddress.ip_address(host))
        except ValueError:
            host = host.rstrip(".").encode("idna").decode("ascii").lower()
            if len(host) > 253 or not all(HOST_LABEL.fullmatch(label) for label in host.split(".")):
                raise ValueError
        return host
    except (ValueError, UnicodeError):
        raise ValueError("来源格式不正确") from None


def validate_origin(value):
    hostname(value)
    parts = urlsplit(value)
    if parts.path or parts.query or parts.fragment or "?" in value or "#" in value:
        raise ValueError("Origin 必须为不含路径、查询和片段的 HTTP(S) 源")
    return value


@dataclass(frozen=True)
class Config:
    password: str
    secret: str
    db_path: Path
    dist: Path = PROJECT_ROOT / "dist"
    port: int = 4173
    allowed_origins: tuple[str, ...] = ("http://127.0.0.1:4173",)
    api_only: bool = False
    trust_proxy: bool = False

    def __post_init__(self):
        if not isinstance(self.password, str) or not isinstance(self.secret, str):
            raise ValueError("必须独立配置管理密码和统计密钥")
        if min(len(self.password), len(self.secret)) < 24:
            raise ValueError("QIDI_ADMIN_PASSWORD 和 QIDI_ANALYTICS_SECRET 必须各至少 24 字符")
        if max(len(self.password), len(self.secret)) > 1024 or any(ord(c) < 32 or ord(c) == 127 for c in self.password + self.secret):
            raise ValueError("密码及密钥须为不含控制字符的 24–1024 字符")
        if any(not 33 <= ord(c) <= 126 for c in self.password):
            raise ValueError("管理密码仅允许英文字母、数字和半角符号，不得包含空白")
        if hmac.compare_digest(self.password.encode(), self.secret.encode()):
            raise ValueError("管理密码与统计密钥必须不同，请独立配置")
        dist = Path(self.dist).expanduser().resolve()
        db_path = Path(self.db_path).expanduser().resolve()
        if any(db_path.is_relative_to(root) for root in (dist, (PROJECT_ROOT / "dist").resolve())):
            raise ValueError("数据库禁止位于 dist 静态目录下")
        if not 0 <= self.port <= 65535:
            raise ValueError("端口必须在 0–65535 之间")
        if not self.allowed_origins or len(self.allowed_origins) > 32:
            raise ValueError("至少配置一个、最多配置 32 个允许的 Origin")
        for origin in self.allowed_origins:
            validate_origin(origin)
        object.__setattr__(self, "dist", dist)
        object.__setattr__(self, "db_path", db_path)

    @classmethod
    def from_env(cls, *, dist=PROJECT_ROOT / "dist", port=4173, api_only=False, trust_proxy=False):
        override = os.environ.get("QIDI_ANALYTICS_DB")
        if override:
            db_path = Path(override)
        else:
            state = os.environ.get("LOCALAPPDATA") if os.name == "nt" else os.environ.get("XDG_STATE_HOME")
            state = Path(state) if state else Path.home() / ".local" / "state"
            db_path = (state / "qidi-portal" / "analytics.sqlite3").resolve()
            if db_path.is_relative_to(PROJECT_ROOT):
                raise ValueError("默认状态目录不得位于网站项目内，请设置 QIDI_ANALYTICS_DB")
        origins = tuple(item.strip() for item in os.environ.get("QIDI_ALLOWED_ORIGINS", "http://127.0.0.1:4173").split(",") if item.strip())
        return cls(password=os.environ.get("QIDI_ADMIN_PASSWORD", ""), secret=os.environ.get("QIDI_ANALYTICS_SECRET", ""),
                   db_path=db_path, dist=Path(dist), port=port, allowed_origins=origins,
                   api_only=api_only, trust_proxy=trust_proxy)


class RateLimiter:
    """有界令牌桶；满载时拒绝新键，不淘汰活跃键来绕过限流。键不含原始 IP。"""
    def __init__(self, capacity=120, period=60, max_buckets=4096, clock=time.monotonic):
        self.capacity = capacity
        self.period = period
        self.max_buckets = max_buckets
        self.clock = clock
        self.buckets = OrderedDict()
        self.lock = threading.Lock()

    def allow(self, key):
        now = self.clock()
        with self.lock:
            ttl = max(600, self.period * 2)
            while self.buckets and next(iter(self.buckets.values()))[1] <= now - ttl:
                self.buckets.popitem(last=False)
            old = self.buckets.pop(key, None)
            if old is None:
                if len(self.buckets) >= self.max_buckets:
                    return False
                tokens = self.capacity
            else:
                tokens = min(self.capacity, old[0] + max(0, now - old[1]) * self.capacity / self.period)
            allowed = tokens >= 1
            self.buckets[key] = (tokens - 1 if allowed else tokens, now)
            return allowed


class Store:
    def __init__(self, config, clock=utc_now):
        self.config = config
        self.clock = clock
        self.internal_hosts = {hostname(origin) for origin in config.allowed_origins}
        config.db_path.parent.mkdir(parents=True, exist_ok=True)
        self.lock = threading.Lock()
        with self.connect() as db:
            db.execute("PRAGMA journal_mode=WAL")
            db.executescript("""
                CREATE TABLE IF NOT EXISTS counts (
                    day TEXT NOT NULL, event TEXT NOT NULL, path TEXT NOT NULL,
                    referrer TEXT NOT NULL, device TEXT NOT NULL, count INTEGER NOT NULL,
                    PRIMARY KEY(day, event, path, referrer, device)
                );
                CREATE TABLE IF NOT EXISTS visitors (
                    day TEXT NOT NULL, visitor TEXT NOT NULL,
                    PRIMARY KEY(day, visitor)
                );
            """)
        self.prune()

    def connect(self):
        # 上下文管理器负责提交，显式 closing 负责关闭连接（见 _connection）。
        return _connection(self.config.db_path)

    def now(self):
        result = self.clock()
        if result.tzinfo is None:
            raise ValueError("统计时钟必须包含时区")
        return result.astimezone(BEIJING)

    @staticmethod
    def _prune(db, today):
        cutoff = (today - timedelta(days=89)).isoformat()
        db.execute("DELETE FROM counts WHERE day < ?", (cutoff,))
        db.execute("DELETE FROM visitors WHERE day < ?", (cutoff,))

    def prune(self):
        with self.lock, self.connect() as db:
            self._prune(db, self.now().date())

    def visitor_id(self, day, ip, ua):
        day_key = hmac.new(self.config.secret.encode(), ("visitor-day\0" + day).encode(), hashlib.sha256).digest()
        return hmac.new(day_key, (ip + "\0" + ua).encode(), hashlib.sha256).hexdigest()

    def record(self, event, path, referrer, ip, ua):
        today = self.now().date()
        day = today.isoformat()
        ref = hostname(referrer) if referrer else DIRECT
        if ref in self.internal_hosts:
            ref = DIRECT
        lower_ua = ua.lower()
        if "ipad" in lower_ua or "tablet" in lower_ua or ("android" in lower_ua and "mobile" not in lower_ua):
            device = "tablet"
        elif any(item in lower_ua for item in ("mobi", "iphone", "android")):
            device = "mobile"
        elif any(item in lower_ua for item in ("windows", "macintosh", "linux", "x11")):
            device = "desktop"
        else:
            device = "other"
        with self.lock, self.connect() as db:
            self._prune(db, today)
            db.execute("""INSERT INTO counts(day,event,path,referrer,device,count) VALUES(?,?,?,?,?,1)
                ON CONFLICT(day,event,path,referrer,device) DO UPDATE SET count=count+1""",
                       (day, event, path, ref, device))
            if event == "pageview":
                db.execute("INSERT OR IGNORE INTO visitors(day,visitor) VALUES(?,?)", (day, self.visitor_id(day, ip, ua)))

    def report(self, days):
        now = self.now()
        end = now.date()
        start = end - timedelta(days=days - 1)
        bounds = (start.isoformat(), end.isoformat())
        with self.lock, self.connect() as db:
            self._prune(db, end)
            counts = db.execute("SELECT day,event,SUM(count) FROM counts WHERE day BETWEEN ? AND ? GROUP BY day,event", bounds).fetchall()
            visitors = dict(db.execute("SELECT day,COUNT(*) FROM visitors WHERE day BETWEEN ? AND ? GROUP BY day", bounds))
            pages = [{"path": row[0], "pv": row[1], "clicks": row[2]} for row in db.execute(
                "SELECT path,SUM(CASE WHEN event=? THEN count ELSE 0 END),SUM(CASE WHEN event<>? THEN count ELSE 0 END) FROM counts WHERE day BETWEEN ? AND ? GROUP BY path ORDER BY 2 DESC,3 DESC,path LIMIT 20", ("pageview", "pageview", *bounds))]
            referrers = [{"hostname": row[0], "pv": row[1]} for row in db.execute(
                "SELECT referrer,SUM(count) FROM counts WHERE day BETWEEN ? AND ? AND event=? GROUP BY referrer ORDER BY 2 DESC,referrer LIMIT 20", (*bounds, "pageview"))]
            devices = [{"device": row[0], "pv": row[1]} for row in db.execute(
                "SELECT device,SUM(count) FROM counts WHERE day BETWEEN ? AND ? AND event=? GROUP BY device ORDER BY 2 DESC,device", (*bounds, "pageview"))]
        by_day = {}
        for offset in range(days):
            day = (start + timedelta(days=offset)).isoformat()
            by_day[day] = {"date": day, "pv": 0, "uv": visitors.get(day, 0), "clicks": 0, "clicks_by_event": dict.fromkeys(CLICKS, 0)}
        for day, event, count in counts:
            if event == "pageview":
                by_day[day]["pv"] += count
            elif event in CLICKS:
                by_day[day]["clicks"] += count
                by_day[day]["clicks_by_event"][event] += count
        daily = list(by_day.values())
        totals = {"pv": sum(row["pv"] for row in daily), "visitor_days": sum(row["uv"] for row in daily),
                  "clicks": sum(row["clicks"] for row in daily),
                  "clicks_by_event": {event: sum(row["clicks_by_event"][event] for row in daily) for event in CLICKS}}
        return {"timezone": "Asia/Shanghai", "days": days, "start_date": bounds[0], "end_date": bounds[1],
                "generated_at": now.isoformat(), "today": daily[-1], "totals": totals, "daily": daily,
                "pages": pages, "referrers": referrers, "devices": devices,
                "measurement": {"uv": "仅对浏览事件按北京时间日期与 IP、UA 的当日 HMAC 去重，为估算访客而非精准人数。",
                                "visitor_days": "每日估算访客的合计称为访客日，不代表期间独立人数。",
                                "download_click": "前往下载中心或点击安装包的次数，不代表完成下载。",
                                "retention_days": 90}}


class _connection:
    def __init__(self, path):
        self.path = path

    def __enter__(self):
        self.db = sqlite3.connect(str(self.path), timeout=3)
        return self.db

    def __exit__(self, exc_type, exc, traceback):
        try:
            if exc_type is None:
                self.db.commit()
            else:
                self.db.rollback()
        finally:
            self.db.close()


class AnalyticsServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, config, clock=utc_now):
        self.config = config
        self.store = Store(config, clock)
        self.event_limiter = RateLimiter()
        self.report_limiter = RateLimiter(capacity=20)
        self.global_limiter = RateLimiter(capacity=1200, max_buckets=1)
        self.limiter_key = secrets.token_bytes(32)
        self.slots = threading.BoundedSemaphore(64)
        self.last_prune = time.monotonic()
        super().__init__(("127.0.0.1", config.port), Handler)

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(3)
        return connection, address

    def process_request(self, request, client_address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except Exception:
            self.slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.slots.release()

    def handle_error(self, request, client_address):
        # 禁止默认 traceback / 请求日志泄露来源、IP 或凭据。
        pass

    def service_actions(self):
        if time.monotonic() - self.last_prune >= 3600:
            try:
                self.store.prune()
                self.last_prune = time.monotonic()
            except sqlite3.Error:
                pass


class Handler(BaseHTTPRequestHandler):
    server_version = "QidiAnalytics"
    sys_version = ""
    protocol_version = "HTTP/1.0"

    def log_message(self, format, *args):
        pass

    def send_error(self, code, message=None, explain=None):
        self.respond(code, {"error": "请求无法处理"})

    def respond(self, status, payload=None, extra=None):
        body = b"" if payload is None else json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.common_headers()
        origin = self.headers.get("Origin") if hasattr(self, "headers") else None
        if origin in self.server.config.allowed_origins:
            self.send_header("Access-Control-Allow-Origin", origin)
        self.send_header("Vary", "Origin")
        if extra:
            for name, value in extra.items():
                self.send_header(name, value)
        self.end_headers()
        if self.command != "HEAD" and body:
            self.wfile.write(body)
        self._discard_rejected_body()

    def _discard_rejected_body(self):
        # 有界排空可防止 Windows 关闭未读连接时以 RST 丢弃错误响应。
        if self.command != "POST" or getattr(self, "body_consumed", False) or not hasattr(self, "headers"):
            return
        lengths = self.headers.get_all("Content-Length", [])
        limit = MAX_BODY + 1
        if len(lengths) == 1 and re.fullmatch(r"[0-9]{1,10}", lengths[0]):
            limit = min(int(lengths[0]), limit)
        self.body_consumed = True
        try:
            self.connection.settimeout(0.2)
            self.rfile.read(limit)
        except OSError:
            pass

    def common_headers(self):
        self.close_connection = True
        self.send_header("Connection", "close")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Pragma", "no-cache")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("X-Frame-Options", "DENY")

    def _origin_allowed(self, required=True):
        values = self.headers.get_all("Origin", [])
        return (not required and not values) or (len(values) == 1 and values[0] in self.server.config.allowed_origins)

    def _identity(self):
        ip = self.client_address[0]
        if self.server.config.trust_proxy and "X-Real-IP" in self.headers:
            if len(self.headers.get_all("X-Real-IP")) != 1:
                raise ValueError("代理 IP 格式不正确")
            ip = self.headers["X-Real-IP"]
        try:
            if "%" in ip:
                raise ValueError
            ip = str(ipaddress.ip_address(ip))
        except ValueError:
            raise ValueError("代理 IP 格式不正确") from None
        ua = self.headers.get("User-Agent", "")
        if len(ua) > 512 or any(ord(c) < 32 or ord(c) == 127 for c in ua):
            raise ValueError("请求标识过长或格式不正确")
        return ip, ua

    def _limited(self, ip, limiter):
        key = hmac.new(self.server.limiter_key, ip.encode(), hashlib.sha256).digest()
        if not self.server.global_limiter.allow(b"global") or not limiter.allow(key):
            self.respond(429, {"error": "请求过于频繁，请稍后重试"}, {"Retry-After": "60"})
            return True
        return False

    def _request_path(self):
        if len(self.path) > 2048 or not self.path.startswith("/") or self.path.startswith("//") or "#" in self.path:
            raise ValueError("请求路径格式不正确")
        if sum(len(k) + len(v) for k, v in self.headers.items()) > 16384:
            raise ValueError("请求头过长")
        parts = urlsplit(self.path)
        if parts.netloc or parts.scheme:
            raise ValueError("请求路径格式不正确")
        return parts

    def _run(self, method):
        try:
            parts = self._request_path()
            method(parts)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except (socket.timeout, TimeoutError):
            self.respond(408, {"error": "请求超时"})
        except (ValueError, UnicodeError):
            self.respond(400, {"error": "请求格式不正确"})
        except (sqlite3.Error, OSError):
            self.respond(500, {"error": "统计服务暂时不可用，请稍后重试"})
        except Exception:
            self.respond(500, {"error": "统计服务暂时不可用，请稍后重试"})

    def do_POST(self):
        self._run(self._event)

    def _event(self, parts):
        if parts.path != "/api/analytics/event" or parts.query:
            self.respond(404, {"error": "接口不存在"})
            return
        if not self._origin_allowed():
            self.respond(403, {"error": "请求来源不在允许名单中"})
            return
        ip, ua = self._identity()
        if self._limited(ip, self.server.event_limiter):
            return
        lengths = self.headers.get_all("Content-Length", [])
        if "Transfer-Encoding" in self.headers or len(lengths) != 1 or not re.fullmatch(r"[0-9]{1,10}", lengths[0]):
            self.respond(400, {"error": "必须提供唯一有效的请求长度，不支持分块请求"})
            return
        length = int(lengths[0])
        if length > MAX_BODY:
            self.respond(413, {"error": "请求内容不得超过 2KB"})
            return
        types = self.headers.get_all("Content-Type", [])
        if len(types) != 1 or not re.fullmatch(r'application/json(?:\s*;\s*charset\s*=\s*(?:utf-8|"utf-8"))?\s*', types[0], re.I):
            self.respond(415, {"error": "仅支持 UTF-8 application/json"})
            return
        raw = self.rfile.read(length)
        self.body_consumed = True
        if len(raw) != length:
            raise ValueError("请求内容不完整")
        try:
            data = json.loads(raw.decode("utf-8"), object_pairs_hook=_unique_object,
                              parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
        except (ValueError, RecursionError):
            raise ValueError("JSON 格式不正确") from None
        if not isinstance(data, dict) or not {"event", "path"} <= data.keys() or data.keys() - {"event", "path", "referrer"}:
            raise ValueError("字段不正确")
        event, path, referrer = data["event"], data["path"], data.get("referrer", "")
        if not isinstance(event, str) or event not in EVENTS:
            raise ValueError("事件不正确")
        if not isinstance(path, str) or len(path) > 128 or not PAGE_PATH.fullmatch(path):
            raise ValueError("页面路径不正确")
        if not isinstance(referrer, str) or len(referrer) > 1024:
            raise ValueError("来源格式不正确")
        if referrer:
            hostname(referrer)
        if self.headers.get("DNT") == "1" or self.headers.get("Sec-GPC") == "1" or BOT.search(ua):
            self.respond(202, {"ok": True, "counted": False})
            return
        self.server.store.record(event, path, referrer, ip, ua)
        self.respond(202, {"ok": True, "counted": True})

    def do_GET(self):
        self._run(self._get)

    def do_HEAD(self):
        self._run(self._get)

    def _get(self, parts):
        if parts.path == "/api/analytics/report":
            ip, _ = self._identity()
            if self._limited(ip, self.server.report_limiter):
                return
            values = self.headers.get_all("Authorization", [])
            supplied = values[0][7:] if len(values) == 1 and values[0].startswith("Bearer ") else ""
            if not hmac.compare_digest(supplied.encode(), self.server.config.password.encode()):
                self.respond(401, {"error": "管理密码不正确或会话已失效"})
                return
            if not self._origin_allowed(required=False):
                self.respond(403, {"error": "请求来源不在允许名单中"})
                return
            query = parse_qs(parts.query, keep_blank_values=True, strict_parsing=True, max_num_fields=2)
            if set(query) - {"days"} or len(query.get("days", [])) > 1:
                raise ValueError("报表参数不正确")
            days = query.get("days", ["7"])[0]
            if days not in ("1", "7", "30", "90"):
                raise ValueError("报表天数不正确")
            self.respond(200, self.server.store.report(int(days)))
        elif parts.path.startswith("/api/") or self.server.config.api_only:
            self.respond(404, {"error": "接口不存在"})
        else:
            self._static(parts.path)

    def do_OPTIONS(self):
        self._run(self._options)

    def _options(self, parts):
        methods = {"/api/analytics/event": "POST", "/api/analytics/report": "GET"}
        if parts.path not in methods or parts.query:
            self.respond(404, {"error": "接口不存在"})
            return
        if not self._origin_allowed():
            self.respond(403, {"error": "请求来源不在允许名单中"})
            return
        ip, _ = self._identity()
        if self._limited(ip, self.server.event_limiter):
            return
        requested = self.headers.get("Access-Control-Request-Method", "")
        headers = {item.strip().lower() for item in self.headers.get("Access-Control-Request-Headers", "").split(",") if item.strip()}
        if requested != methods[parts.path] or headers - {"content-type", "authorization"}:
            self.respond(403, {"error": "预检方法或请求头不允许"})
            return
        self.respond(204, extra={"Access-Control-Allow-Methods": methods[parts.path], "Access-Control-Allow-Headers": "Content-Type, Authorization"})

    def _static(self, raw_path):
        path = unquote(raw_path, errors="strict")
        components = path.lstrip("/").split("/")
        if "\\" in path or ":" in path or "%" in path or any(ord(c) < 32 or ord(c) == 127 for c in path):
            self.respond(404, {"error": "页面不存在"})
            return
        if any(part.startswith(".") or part.endswith((".", " ")) for part in components if part):
            self.respond(404, {"error": "页面不存在"})
            return
        root = self.server.config.dist
        candidate = root.joinpath(*components).resolve()
        if not candidate.is_relative_to(root):
            self.respond(404, {"error": "页面不存在"})
            return
        if candidate.is_dir():
            candidate = (candidate / "index.html").resolve()
        if not candidate.is_relative_to(root) or any(part.startswith(".") for part in candidate.relative_to(root).parts):
            self.respond(404, {"error": "页面不存在"})
            return
        # 同时拒绝 Windows 隐藏属性及指向隐藏目录的符号链接。
        for target in (candidate, *candidate.parents):
            if target == root:
                break
            if target.exists() and getattr(target.stat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_HIDDEN:
                self.respond(404, {"error": "页面不存在"})
                return
        if not candidate.is_file():
            self.respond(404, {"error": "页面不存在"})
            return
        with candidate.open("rb") as source:
            self.send_response(200)
            self.send_header("Content-Type", mimetypes.guess_type(str(candidate))[0] or "application/octet-stream")
            self.send_header("Content-Length", str(os.fstat(source.fileno()).st_size))
            self.common_headers()
            self.end_headers()
            if self.command != "HEAD":
                while chunk := source.read(65536):
                    self.wfile.write(chunk)


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("JSON 字段重复")
        result[key] = value
    return result


class ChineseParser(argparse.ArgumentParser):
    def error(self, message):
        self.print_usage()
        self.exit(2, "参数错误，请使用 --help 查看中文帮助。\n")


def main():
    parser = ChineseParser(description="启迪官网本地访问统计：只监听 127.0.0.1，数据保留 90 天。",
                           epilog="必须分别设置 QIDI_ADMIN_PASSWORD、QIDI_ANALYTICS_SECRET（不同且各至少 24 字符）。QIDI_ANALYTICS_DB 可覆盖用户状态目录中的数据库路径，但禁止放在 dist 内。QIDI_ALLOWED_ORIGINS 为逗号分隔的允许源，默认 http://127.0.0.1:4173；改端口后须相应配置。仅用于本地开发或受控反代后端。", add_help=False)
    parser._optionals.title = "可选参数"
    parser.add_argument("-h", "--help", action="help", help="显示此帮助并退出")
    parser.add_argument("--port", type=int, default=4173, help="本地监听端口，默认 4173")
    parser.add_argument("--dist", type=Path, default=PROJECT_ROOT / "dist", help="静态产物目录，默认网站项目下的 dist")
    parser.add_argument("--api-only", action="store_true", help="仅提供统计 API，不提供任何静态文件")
    parser.add_argument("--trust-proxy", action="store_true", help="信任并校验 X-Real-IP；仅在反向代理强制覆盖该请求头时启用，默认关闭")
    args = parser.parse_args()
    try:
        config = Config.from_env(**vars(args))
        server = AnalyticsServer(config)
    except ValueError as error:
        parser.exit(2, f"启动失败：{error}\n")
    except (OSError, sqlite3.Error):
        parser.exit(2, "启动失败：无法打开本地数据库或监听端口。\n")
    print(f"启迪统计服务已启动：http://127.0.0.1:{server.server_port}（{'仅 API' if config.api_only else '静态站与 API'}）")
    try:
        server.serve_forever(poll_interval=0.5)
    except KeyboardInterrupt:
        print("\n启迪统计服务已停止。")
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
