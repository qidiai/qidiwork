"""运行：python -B -m unittest discover -s server -p test_analytics.py -v"""
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from dataclasses import replace
from datetime import datetime, timedelta, timezone
import http.client
import importlib.util
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

# 避免环境中同名 analytics 包抢占本地模块。
SPEC = importlib.util.spec_from_file_location("qidi_analytics", Path(__file__).with_name("analytics.py"))
analytics = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = analytics
SPEC.loader.exec_module(analytics)

PASSWORD = "test-admin-password-independent-2026"
SECRET = "test-analytics-secret-independent-2026"
ORIGIN = "http://127.0.0.1:4173"
UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) TestBrowser/1.0"


class MutableClock:
    def __init__(self):
        self.value = datetime(2026, 10, 24, 4, tzinfo=timezone.utc)

    def __call__(self):
        return self.value


class ServerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="qidi-analytics-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        (self.dist / "index.html").write_text("启迪首页", encoding="utf-8")
        (self.dist / "empty").mkdir()
        (self.dist / ".hidden").write_text("hidden secret", encoding="utf-8")
        (self.dist / ".private").mkdir()
        (self.dist / ".private" / "index.html").write_text("hidden directory", encoding="utf-8")
        (self.root / "outside.txt").write_text("outside secret", encoding="utf-8")
        self.clock = MutableClock()
        self.config = analytics.Config(password=PASSWORD, secret=SECRET, db_path=self.root / "state" / "events.sqlite3", dist=self.dist, port=0)
        self.server = None
        self.start()
        self.addCleanup(self.stop)

    def start(self):
        self.server = analytics.AnalyticsServer(self.config, self.clock)
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
        self.thread.start()
        self.port = self.server.server_port

    def stop(self):
        if self.server is not None:
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(timeout=5)
            self.server = None

    def request(self, method, path, body=None, headers=None):
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=5)
        try:
            conn.request(method, path, body=body, headers=headers or {})
            response = conn.getresponse()
            raw = response.read()
            result = json.loads(raw) if raw and "application/json" in response.getheader("Content-Type", "") else raw
            return response.status, dict(response.getheaders()), result
        finally:
            conn.close()

    def event(self, data=None, **headers):
        payload = {"event": "pageview", "path": "/", "referrer": ""} if data is None else data
        defaults = {"Origin": ORIGIN, "Content-Type": "application/json", "User-Agent": UA}
        defaults.update(headers)
        return self.request("POST", "/api/analytics/event", json.dumps(payload, ensure_ascii=False).encode(), defaults)

    def report(self, days=7, password=PASSWORD, **headers):
        defaults = {"Authorization": "Bearer " + password}
        defaults.update(headers)
        return self.request("GET", f"/api/analytics/report?days={days}", headers=defaults)

    def test_only_listens_on_loopback(self):
        self.assertEqual(self.server.server_address[0], "127.0.0.1")

    def test_authentication_and_no_cache(self):
        self.event()
        for password in ("", "unknown", PASSWORD + "x"):
            status, headers, payload = self.report(password=password)
            self.assertEqual(status, 401)
            self.assertNotIn("today", payload)
            self.assertEqual(headers["Cache-Control"], "no-store")
            self.assertNotIn("WWW-Authenticate", headers)
        status, _, _ = self.request("GET", "/api/analytics/report?days=7")
        self.assertEqual(status, 401)
        status, _, _ = self.report(Authorization="Basic " + PASSWORD)
        self.assertEqual(status, 401)
        status, headers, payload = self.report()
        self.assertEqual(status, 200)
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertEqual(payload["today"]["pv"], 1)
        self.assertNotIn("Set-Cookie", headers)

    def test_reject_foreign_and_missing_origin(self):
        for origin in ("https://evil.example", "null", ORIGIN + "/", ORIGIN + ".evil.example"):
            status, headers, _ = self.event(Origin=origin)
            self.assertEqual(status, 403)
            self.assertNotIn("Access-Control-Allow-Origin", headers)
        status, _, _ = self.request("POST", "/api/analytics/event", b"{}", {"Content-Type": "application/json"})
        self.assertEqual(status, 403)
        self.assertEqual(self.report(Origin="https://evil.example")[0], 403)
        self.assertEqual(self.report()[2]["totals"]["pv"], 0)

    def test_preflight(self):
        headers = {"Origin": ORIGIN, "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"}
        status, response_headers, _ = self.request("OPTIONS", "/api/analytics/event", headers=headers)
        self.assertEqual(status, 204)
        self.assertEqual(response_headers["Access-Control-Allow-Origin"], ORIGIN)
        self.assertNotIn("Access-Control-Allow-Credentials", response_headers)
        headers["Access-Control-Request-Headers"] = "X-Real-IP"
        self.assertEqual(self.request("OPTIONS", "/api/analytics/event", headers=headers)[0], 403)
        headers["Origin"] = "https://evil.example"
        self.assertEqual(self.request("OPTIONS", "/api/analytics/event", headers=headers)[0], 403)

    def test_content_type_size_and_json(self):
        self.assertEqual(self.event(**{"Content-Type": "text/plain"})[0], 415)
        self.assertEqual(self.event(**{"Content-Type": "application/json; charset=gbk"})[0], 415)
        self.assertEqual(self.event(**{"Content-Type": "application/json; charset=UTF-8"})[0], 202)
        headers = {"Origin": ORIGIN, "Content-Type": "application/json"}
        self.assertEqual(self.request("POST", "/api/analytics/event", b"x" * 2049, headers)[0], 413)
        for raw in (b"{", b"\xff", b"null", b"[]", b"true", b'{"event":"pageview","event":"pageview","path":"/"}', b'{"event":NaN,"path":"/"}', b"[" * 1000 + b"]" * 1000):
            self.assertEqual(self.request("POST", "/api/analytics/event", raw, headers)[0], 400)
        headers["Transfer-Encoding"] = "chunked"
        self.assertEqual(self.request("POST", "/api/analytics/event", b"{}", headers)[0], 400)

    def test_invalid_fields_and_paths(self):
        cases = [
            {}, {"event": "pageview"}, {"event": "bogus", "path": "/"},
            {"event": [], "path": "/"}, {"event": "pageview", "path": 1},
            {"event": "pageview", "path": "/", "referrer": []},
            {"event": "pageview", "path": "/", "secret": "not-allowed"},
            {"event": "pageview", "path": "/", "referrer": "x" * 1025},
        ]
        cases.extend({"event": "pageview", "path": path} for path in (
            "/admin/", "/api/analytics/report", "/tech/../", "/?q=secret", "/download/?token=secret", "/tech/a/b/", "/tech/", "/tech/%2e%2e/", "/tech/a' OR 1=1--/", "/tech/<script>/", "/tech/" + "x" * 129 + "/"
        ) if path != "/tech/")
        for data in cases:
            with self.subTest(data=data):
                self.assertEqual(self.event(data)[0], 400)
        for path in ("/", "/tech/", "/tech/rust-provider-abstraction/", "/download/"):
            self.assertEqual(self.event({"event": "pageview", "path": path})[0], 202)

    def test_blog_and_tech_paths_share_event_rules(self):
        for section in ("tech", "blog"):
            for slug in ("", "free-quota-cheatsheet/", "qwen-image-6gb/", "Demo_1/", "x" * 96 + "/"):
                path = f"/{section}/{slug}"
                with self.subTest(path=path):
                    status, _, payload = self.event({"event": "pageview", "path": path})
                    self.assertEqual(status, 202)
                    self.assertTrue(payload["counted"])
            for suffix in ("..", "%2e%2e", "a/b", "<script>", "-slug", "x" * 97):
                path = f"/{section}/{suffix}/"
                with self.subTest(path=path):
                    self.assertEqual(self.event({"event": "pageview", "path": path})[0], 400)
            for path in (f"/{section}", f"/{section}/slug", f"/{section}/?q=secret", f"/{section}/slug/#anchor", f"/{section}//"):
                with self.subTest(path=path):
                    self.assertEqual(self.event({"event": "pageview", "path": path})[0], 400)
        for event in analytics.CLICKS:
            status, _, payload = self.event({"event": event, "path": "/blog/qwen-image-6gb/"})
            self.assertEqual(status, 202)
            self.assertTrue(payload["counted"])
        report = self.report()[2]
        self.assertEqual(report["totals"]["pv"], 10)
        self.assertEqual(report["totals"]["clicks"], len(analytics.CLICKS))
        self.assertIn("/blog/", {row["path"] for row in report["pages"]})

    def test_referrer_hostname_only_and_sql_xss(self):
        hostile_query = "https://example.com/path?secret=TOP_SECRET&sql=';DROP%20TABLE%20counts;--#<script>alert(1)</script>"
        self.assertEqual(self.event({"event": "pageview", "path": "/", "referrer": hostile_query})[0], 202)
        self.assertEqual(self.event({"event": "pageview", "path": "/", "referrer": ORIGIN + "/private?token=private"})[0], 202)
        for ref in ("javascript:alert(1)", "https://evil.example/<script> alert(1)</script>", "https://<script>.example", "https://a';DROP.example", "https://user:password@example.com", "https://good.example\\@evil.example", "https://example.com:99999", "https://example.com\n", "//example.com", "https://bad_host.example", "https://example.com%00"):
            self.assertEqual(self.event({"event": "pageview", "path": "/", "referrer": ref})[0], 400, ref)
        report = self.report()[2]
        self.assertEqual({row["hostname"] for row in report["referrers"]}, {"example.com", analytics.DIRECT})
        self.assertEqual(report["totals"]["pv"], 2)
        with closing(sqlite3.connect(self.config.db_path)) as db:
            text = "\n".join(db.iterdump())
        for raw in ("TOP_SECRET", "DROP TABLE", "127.0.0.1", UA, PASSWORD, SECRET, "<script>", "?token"):
            self.assertNotIn(raw, text)

    def test_bots_and_privacy_preferences_ignored(self):
        for headers in ({"DNT": "1"}, {"Sec-GPC": "1"}, {"User-Agent": "Googlebot/2.1"}, {"User-Agent": "HeadlessChrome"}, {"User-Agent": "curl/8.0"}):
            status, _, result = self.event(**headers)
            self.assertEqual(status, 202)
            self.assertFalse(result["counted"])
        self.assertEqual(self.report()[2]["totals"]["pv"], 0)
        self.assertTrue(self.event(DNT="0")[2]["counted"])

    def test_daily_dedup_and_click_definition(self):
        for _ in range(3):
            self.event()
        for event in analytics.CLICKS:
            self.event({"event": event, "path": "/download/", "referrer": ""})
        report = self.report()[2]
        self.assertEqual(report["today"]["pv"], 3)
        self.assertEqual(report["today"]["uv"], 1)
        self.assertEqual(report["today"]["clicks"], 4)
        self.assertEqual(report["totals"]["clicks_by_event"], dict.fromkeys(analytics.CLICKS, 1))
        self.assertIn("不代表完成下载", report["measurement"]["download_click"])
        self.assertNotIn("uv", report["totals"])
        self.event(**{"User-Agent": "Mozilla/5.0 iPhone Mobile"})
        self.assertEqual(self.report()[2]["today"]["uv"], 2)

    def test_click_without_pageview_has_no_uv(self):
        self.event({"event": "download_click", "path": "/download/"})
        report = self.report()[2]
        self.assertEqual(report["totals"]["clicks"], 1)
        self.assertEqual(report["totals"]["pv"], 0)
        self.assertEqual(report["totals"]["visitor_days"], 0)
        self.assertEqual(report["referrers"], [])

    def test_beijing_midnight_and_period_visitor_days(self):
        self.clock.value = datetime(2026, 10, 24, 15, 59, 59, tzinfo=timezone.utc)
        self.event()
        self.event()
        self.clock.value += timedelta(seconds=1)
        self.event()
        self.event({"event": "source_click", "path": "/tech/"})
        report = self.report()[2]
        self.assertEqual(report["end_date"], "2026-10-25")
        self.assertEqual(report["timezone"], "Asia/Shanghai")
        self.assertEqual(report["today"]["pv"], 1)
        self.assertEqual(report["today"]["uv"], 1)
        self.assertEqual(report["totals"]["pv"], 3)
        self.assertEqual(report["totals"]["visitor_days"], 2)
        self.assertEqual(report["daily"][-2]["pv"], 2)
        self.assertEqual(len(report["daily"]), 7)
        self.assertEqual(self.report(days=1)[2]["totals"]["visitor_days"], 1)
        with closing(sqlite3.connect(self.config.db_path)) as db:
            ids = db.execute("SELECT visitor FROM visitors ORDER BY day").fetchall()
        self.assertEqual(len(ids), 2)
        self.assertNotEqual(ids[0], ids[1])

    def test_report_days_validation_and_zero_fill(self):
        for days in (1, 7, 30, 90):
            status, _, report = self.report(days)
            self.assertEqual(status, 200)
            self.assertEqual(len(report["daily"]), days)
            self.assertEqual(report["totals"]["pv"], 0)
            self.assertEqual(report["pages"], [])
        for query in ("days=0", "days=365", "days=abc", "days=7&days=30", "days=7&extra=1", "days=", "days=7%27%20OR%201=1--"):
            self.assertEqual(self.request("GET", "/api/analytics/report?" + query, headers={"Authorization": "Bearer " + PASSWORD})[0], 400)
        self.assertEqual(self.request("GET", "/api/analytics/report", headers={"Authorization": "Bearer " + PASSWORD})[2]["days"], 7)

    def test_retention_keeps_exactly_90_days(self):
        self.event()
        self.clock.value += timedelta(days=89)
        self.event()
        self.assertEqual(self.report(90)[2]["totals"]["pv"], 2)
        self.clock.value += timedelta(days=1)
        self.server.store.prune()
        self.assertEqual(self.report(90)[2]["totals"]["pv"], 1)
        with closing(sqlite3.connect(self.config.db_path)) as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM visitors").fetchone()[0], 1)
            self.assertEqual(db.execute("SELECT COUNT(DISTINCT day) FROM counts").fetchone()[0], 1)

    def test_data_persists_after_restart(self):
        self.event()
        self.stop()
        self.start()
        self.assertEqual(self.report()[2]["totals"]["pv"], 1)
        self.event()
        report = self.report()[2]
        self.assertEqual(report["totals"]["pv"], 2)
        self.assertEqual(report["today"]["uv"], 1)

    def test_proxy_disabled_by_default_and_validated_when_enabled(self):
        self.event(**{"X-Real-IP": "198.51.100.1"})
        self.event(**{"X-Real-IP": "not an IP"})
        self.assertEqual(self.report()[2]["today"]["uv"], 1)
        self.stop()
        self.config = replace(self.config, trust_proxy=True)
        self.start()
        self.assertEqual(self.event(**{"X-Real-IP": "198.51.100.1"})[0], 202)
        self.assertEqual(self.event(**{"X-Real-IP": "198.51.100.2"})[0], 202)
        for value in ("bad", "1.2.3.4, 5.6.7.8", "fe80::1%eth0"):
            self.assertEqual(self.event(**{"X-Real-IP": value})[0], 400)
        self.assertEqual(self.report()[2]["today"]["uv"], 3)

    def test_event_report_and_global_limits(self):
        self.server.event_limiter = analytics.RateLimiter(capacity=2)
        self.assertEqual(self.event()[0], 202)
        self.assertEqual(self.event()[0], 202)
        status, headers, _ = self.event()
        self.assertEqual(status, 429)
        self.assertIn("Retry-After", headers)
        self.server.report_limiter = analytics.RateLimiter(capacity=1)
        self.assertEqual(self.report(password="bad")[0], 401)
        self.assertEqual(self.report()[0], 429)
        self.server.global_limiter = analytics.RateLimiter(capacity=1)
        self.server.event_limiter = analytics.RateLimiter()
        self.assertEqual(self.event()[0], 202)
        self.assertEqual(self.event()[0], 429)

    def test_rejected_unknown_length_body_is_drained(self):
        handler = object.__new__(analytics.Handler)
        handler.command = 'POST'
        handler.headers = Mock()
        handler.headers.get_all.return_value = []
        handler.connection = Mock()
        handler.rfile = Mock()
        handler._discard_rejected_body()
        handler.rfile.read.assert_called_once_with(analytics.MAX_BODY + 1)
        for _ in range(20):
            headers = {'Origin': ORIGIN, 'Content-Type': 'application/json', 'Transfer-Encoding': 'chunked'}
            status, _, payload = self.request('POST', '/api/analytics/event', b'{}', headers)
            self.assertEqual(status, 400)
            self.assertIn('error', payload)

    def test_repeated_rejections_deliver_json_without_connection_reset(self):
        # HTTP 客户端分开发送头和请求体，Windows 上过早 close 可吞掉错误响应。
        self.server.event_limiter = analytics.RateLimiter(capacity=1)
        self.assertEqual(self.event()[0], 202)
        for _ in range(20):
            status, _, payload = self.event()
            self.assertEqual(status, 429)
            self.assertIn("error", payload)
            self.assertEqual(self.event(Origin="https://evil.example")[0], 403)

    def test_concurrent_requests_do_not_lose_counts(self):
        with ThreadPoolExecutor(max_workers=6) as pool:
            results = list(pool.map(lambda _: self.event()[0], range(24)))
        self.assertEqual(results, [202] * 24)
        report = self.report()[2]
        self.assertEqual(report["totals"]["pv"], 24)
        self.assertEqual(report["today"]["uv"], 1)

    def test_device_and_page_breakdown(self):
        for path, ua in (("/", UA), ("/tech/", "Mozilla iPhone Mobile"), ("/download/", "Mozilla iPad"), ("/tech/demo/", "UnknownBrowser")):
            self.event({"event": "pageview", "path": path, "referrer": "https://example.com"}, **{"User-Agent": ua})
        report = self.report()[2]
        self.assertEqual({row["device"] for row in report["devices"]}, {"desktop", "mobile", "tablet", "other"})
        self.assertEqual(sum(row["pv"] for row in report["pages"]), 4)
        self.assertEqual(report["referrers"], [{"hostname": "example.com", "pv": 4}])

    def test_static_files_hidden_files_and_directory_traversal(self):
        self.assertEqual(self.request("GET", "/")[0], 200)
        self.assertEqual(self.request("GET", "/")[2].decode(), "启迪首页")
        self.assertEqual(self.request("HEAD", "/")[2], b"")
        for path in ("/../outside.txt", "/%2e%2e/outside.txt", "/%252e%252e/outside.txt", "/%5c..%5coutside.txt", "/.hidden", "/%2ehidden", "/.private/", "/empty/", "/C:/Windows/win.ini", "/index.html:$DATA", "/%00", "/missing"):
            status, _, result = self.request("GET", path)
            self.assertEqual(status, 404, path)
            self.assertNotIn("outside secret", str(result))
            self.assertNotIn("hidden secret", str(result))

    def test_symlink_escape_and_hidden_target(self):
        try:
            (self.dist / "escape.txt").symlink_to(self.root / "outside.txt")
            (self.dist / "alias.txt").symlink_to(self.dist / ".hidden")
        except OSError:
            self.skipTest("当前系统无创建符号链接权限")
        self.assertEqual(self.request("GET", "/escape.txt")[0], 404)
        self.assertEqual(self.request("GET", "/alias.txt")[0], 404)

    def test_api_only_does_not_serve_shell(self):
        self.stop()
        self.config = replace(self.config, api_only=True)
        self.start()
        self.assertEqual(self.request("GET", "/")[0], 404)
        self.assertEqual(self.request("GET", "/admin/")[0], 404)
        self.assertEqual(self.event()[0], 202)
        self.assertEqual(self.report()[0], 200)

    def test_unknown_api_and_event_query(self):
        self.assertEqual(self.request("GET", "/api/analytics/missing")[0], 404)
        self.assertEqual(self.request("POST", "/api/analytics/event?extra=1", b"{}", {"Origin": ORIGIN})[0], 404)

    def test_database_failure_does_not_leak_details(self):
        with patch.object(self.server.store, "report", side_effect=sqlite3.OperationalError("SECRET /private/db.sqlite")):
            status, _, payload = self.report()
        self.assertEqual(status, 500)
        self.assertNotIn("SECRET", json.dumps(payload))

    def test_slow_body_times_out(self):
        conn = socket.create_connection(("127.0.0.1", self.port), timeout=5)
        try:
            conn.sendall((f"POST /api/analytics/event HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: {ORIGIN}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{").encode())
            self.assertIn(b"408", conn.recv(4096))
        finally:
            conn.close()


class ConfigAndLimiterTests(unittest.TestCase):
    def test_bounded_memory_and_refill(self):
        now = [0.0]
        limiter = analytics.RateLimiter(capacity=2, period=60, max_buckets=2, clock=lambda: now[0])
        self.assertTrue(limiter.allow(b"a"))
        self.assertTrue(limiter.allow(b"a"))
        self.assertFalse(limiter.allow(b"a"))
        self.assertTrue(limiter.allow(b"b"))
        for i in range(100):
            self.assertFalse(limiter.allow(str(i).encode()))
        self.assertEqual(len(limiter.buckets), 2)
        now[0] = 30
        self.assertTrue(limiter.allow(b"a"))
        now[0] = 700
        self.assertTrue(limiter.allow(b"new"))
        self.assertEqual(len(limiter.buckets), 1)

    def test_required_independent_credentials_and_db_boundary(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            config = dict(password=PASSWORD, secret=SECRET, db_path=root / "analytics.sqlite3", dist=root / "dist")
            for password, secret in (("", ""), ("short", SECRET), (PASSWORD, "short"), (PASSWORD, PASSWORD), ("密" * 24, SECRET), (PASSWORD + " ", SECRET)):
                with self.assertRaises(ValueError):
                    analytics.Config(**dict(config, password=password, secret=secret))
            for path in (root / "dist" / "analytics.sqlite3", root / "dist" / "sub" / ".." / "analytics.sqlite3", analytics.PROJECT_ROOT / "dist" / "analytics.sqlite3"):
                with self.assertRaises(ValueError):
                    analytics.Config(**dict(config, db_path=path))
            for origins in (("*",), ("null",), (ORIGIN + "/",), ("https://user:pass@example.com",), ()):
                with self.assertRaises(ValueError):
                    analytics.Config(**dict(config, allowed_origins=origins))

    def test_environment_override_and_default_outside_project(self):
        with tempfile.TemporaryDirectory() as home, patch.dict(os.environ, {
            "QIDI_ADMIN_PASSWORD": PASSWORD, "QIDI_ANALYTICS_SECRET": SECRET,
            "HOME": home, "USERPROFILE": home
        }, clear=True):
            config = analytics.Config.from_env()
            self.assertFalse(config.db_path.is_relative_to(analytics.PROJECT_ROOT))
            self.assertEqual(config.allowed_origins, (ORIGIN,))
            self.assertFalse(config.trust_proxy)
        with tempfile.TemporaryDirectory() as temp, patch.dict(os.environ, {
            "QIDI_ADMIN_PASSWORD": PASSWORD, "QIDI_ANALYTICS_SECRET": SECRET,
            "QIDI_ANALYTICS_DB": str(Path(temp) / "db.sqlite3"),
            "QIDI_ALLOWED_ORIGINS": "https://example.com, https://www.example.com"
        }, clear=True):
            config = analytics.Config.from_env()
            self.assertEqual(config.db_path, (Path(temp) / "db.sqlite3").resolve())
            self.assertEqual(len(config.allowed_origins), 2)

    def test_cli_refuses_missing_configuration(self):
        env = {k: v for k, v in os.environ.items() if k not in ("QIDI_ADMIN_PASSWORD", "QIDI_ANALYTICS_SECRET")}
        result = subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("analytics.py")), "--api-only"], env=env, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"QIDI_ADMIN_PASSWORD", result.stderr)
        help_result = subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("analytics.py")), "--help"], capture_output=True, timeout=10)
        self.assertEqual(help_result.returncode, 0)
        self.assertIn(b"--trust-proxy", help_result.stdout)


if __name__ == "__main__":
    unittest.main()
