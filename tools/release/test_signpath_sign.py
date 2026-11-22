#!/usr/bin/env python3
"""tools/release/signpath-sign.py 的自测(纯 stdlib,本地 mock SignPath,不联网)。

覆盖:主链路(提交→轮询→拉回→就地替换)、--out 输出、审批被拒、提交 403、
缺少环境变量、--param 格式错误。

用法:
    python tools/release/test_signpath_sign.py
全部通过退出码 0;任一失败打印 FAIL 并退出码 1。
"""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "signpath-sign.py"

spec = importlib.util.spec_from_file_location("signpath_sign", SCRIPT)
signpath_sign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signpath_sign)  # type: ignore[attr-defined]

ORIGINAL = b"UNSIGNED-PAYLOAD-v1"
SIGNED = b"SIGNED-PAYLOAD-v1"
BASE = "http://127.0.0.1:8931/Api/v1"

# mock 行为开关:由 projectSlug 驱动
STATE = {"status_calls": 0, "deny": False, "forbidden": False}


class MockHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _json(self, code: int, payload: dict | bytes, headers: dict | None = None):
        data = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers["Content-Length"])
        body = self.rfile.read(length)
        if self.headers.get("Authorization") != "Bearer test-token":
            self._json(401, {"message": "bad token"})
            return
        if STATE["forbidden"]:
            self._json(403, {"message": "token lacks permission"})
            return
        # 最小契约校验:字段名、参数前缀、产物字节都在
        for needle in (b'name="projectSlug"', b'name="signingPolicySlug"',
                       b'name="parameters.productVersion"', b"0.1.13",
                       b'filename="', ORIGINAL):
            assert needle in body, f"multipart 缺少 {needle!r}"
        STATE["status_calls"] = 0
        self._json(201, b"{}", {"Location": f"{BASE}/test-org/SigningRequests/req-1"})

    def do_GET(self):
        if self.headers.get("Authorization") != "Bearer test-token":
            self._json(401, {"message": "bad token"})
            return
        if self.path.endswith("/Status"):
            STATE["status_calls"] += 1
            if STATE["status_calls"] == 1:
                payload = {"status": "InProgress", "isFinalStatus": False}
            else:
                final = "Denied" if STATE["deny"] else "Completed"
                payload = {"status": final, "isFinalStatus": True}
            self._json(200, payload)
        elif self.path.endswith("/SignedArtifact"):
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(SIGNED)))
            self.end_headers()
            self.wfile.write(SIGNED)
        else:
            self._json(404, {"message": "not found"})


def run_script(argv: list[str], env: dict[str, str], tmp: Path) -> tuple[int, str, str]:
    """在打完补丁的环境里跑 main(),返回 (退出码, stdout, stderr)。

    两种退出形态都收:main() 返回 int,以及 sys.exit(1)/sys.exit("消息")。
    """
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(env)
    try:
        import contextlib
        import io
        out, err = io.StringIO(), io.StringIO()
        code = 0
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                result = signpath_sign.main(argv)
            code = result if isinstance(result, int) else 0
        except SystemExit as e:
            if isinstance(e.code, int):
                code = e.code
            else:
                # sys.exit("消息"):消息在 e.code 里,补写进 stderr 便于断言
                code = 1
                err.write(f"{e.code}\n")
        return code, out.getvalue(), err.getvalue()
    finally:
        os.environ.clear()
        os.environ.update(saved)


def base_env(**over: str) -> dict[str, str]:
    env = {
        "SIGNPATH_API_TOKEN": "test-token",
        "SIGNPATH_ORGANIZATION_ID": "test-org",
        "SIGNPATH_PROJECT_SLUG": "test-project",
        "SIGNPATH_SIGNING_POLICY_SLUG": "test-signing",
        "SIGNPATH_BASE_URL": BASE,
        "SIGNPATH_POLL_SECONDS": "0",
    }
    env.update(over)
    return env


CASES: list[tuple[str, object]] = []


def case(fn):
    CASES.append((fn.__name__, fn))
    return fn


@case
def happy_path_in_place(tmp: Path) -> None:
    f = tmp / "setup.exe"
    f.write_bytes(ORIGINAL)
    code, out, _ = run_script(
        ["--file", str(f), "--param", "productVersion=0.1.13"], base_env(), tmp)
    assert code == 0, f"退出码 {code}"
    assert f.read_bytes() == SIGNED, "就地替换失败"
    payload = json.loads(out)
    assert payload["signingRequestId"] == "req-1"
    assert payload["size_bytes"] == len(SIGNED)
    assert payload["file"] == "setup.exe"
    assert not list(tmp.glob("*.part")), "残留 .part 文件"


@case
def out_to_separate_path(tmp: Path) -> None:
    src = tmp / "unsigned.exe"
    dst = tmp / "signed" / "setup.exe"
    dst.parent.mkdir()
    src.write_bytes(ORIGINAL)
    code, _, _ = run_script(
        ["--file", str(src), "--out", str(dst), "--param", "productVersion=0.1.13"],
        base_env(), tmp)
    assert code == 0
    assert src.read_bytes() == ORIGINAL, "--out 模式下原文件被改动"
    assert dst.read_bytes() == SIGNED, "--out 产物内容不对"


@case
def denied_leaves_file_intact(tmp: Path) -> None:
    STATE["deny"] = True
    try:
        f = tmp / "denied.exe"
        f.write_bytes(ORIGINAL)
        code, _, err = run_script(
            ["--file", str(f), "--param", "productVersion=0.1.13"], base_env(), tmp)
        assert code == 1, f"被拒应退出 1,得到 {code}"
        assert "Denied" in err, f"错误信息未说明 Denied: {err}"
        assert f.read_bytes() == ORIGINAL, "被拒后原文件被改动"
    finally:
        STATE["deny"] = False


@case
def forbidden_no_retry(tmp: Path) -> None:
    STATE["forbidden"] = True
    try:
        f = tmp / "forbidden.exe"
        f.write_bytes(ORIGINAL)
        code, _, err = run_script(
            ["--file", str(f), "--param", "productVersion=0.1.13"], base_env(), tmp)
        assert code == 1
        assert "403" in err and "lacks permission" in err, f"403 信息不完整: {err}"
        assert f.read_bytes() == ORIGINAL
    finally:
        STATE["forbidden"] = False


@case
def missing_env_vars(tmp: Path) -> None:
    f = tmp / "x.exe"
    f.write_bytes(ORIGINAL)
    env = base_env()
    del env["SIGNPATH_PROJECT_SLUG"]
    code, _, err = run_script(["--file", str(f)], env, tmp)
    assert code == 1
    assert "SIGNPATH_PROJECT_SLUG" in err, f"未指明缺哪个变量: {err}"
    assert f.read_bytes() == ORIGINAL


@case
def bad_param_format(tmp: Path) -> None:
    f = tmp / "y.exe"
    f.write_bytes(ORIGINAL)
    code, _, err = run_script(["--file", str(f), "--param", "oops"], base_env(), tmp)
    assert code == 1
    assert "KEY=VALUE" in err, f"参数错误提示不对: {err}"


@case
def missing_file(tmp: Path) -> None:
    code, _, err = run_script(["--file", str(tmp / "nope.exe")], base_env(), tmp)
    assert code == 1
    assert "不存在" in err


def main() -> int:
    server = HTTPServer(("127.0.0.1", 8931), MockHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    time.sleep(0.2)
    import tempfile
    failures = 0
    try:
        for name, fn in CASES:
            with tempfile.TemporaryDirectory() as d:
                try:
                    fn(Path(d))
                    print(f"PASS {name}")
                except AssertionError as e:
                    failures += 1
                    print(f"FAIL {name}: {e}")
    finally:
        server.shutdown()
    print(f"{len(CASES) - failures}/{len(CASES)} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
