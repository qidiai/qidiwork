#!/usr/bin/env python3
"""把安装包提交 SignPath 做 Authenticode 代码签名,拉回签名后的产物。

这与 Ed25519 更新校验(tools/release/sign-update.py)是两层互不相干的签名:
  - 本脚本 = Authenticode 代码签名:给 Windows SmartScreen / 浏览器看的
    "发布者身份",私钥在 SignPath 的 HSM 上,我们只提交产物、拉回签名结果;
  - sign-update.py = 应用内自动更新的完整性校验,与本章无关。
两者都要做,且顺序固定:先本脚本(Authenticode),再 sign-update.py
(Authenticode 会改变文件字节,sha256/sig 必须算最终分发文件)。

API 契约(docs.signpath.io/build-system-integration):
  POST /Api/v1/{org}/SigningRequests/SubmitWithArtifact   multipart/form-data
    → 201 + Location: .../SigningRequests/{id}
  GET  /Api/v1/{org}/SigningRequests/{id}/Status          轮询到 isFinalStatus
    → {status, workflowStatus, isFinalStatus}
  GET  /Api/v1/{org}/SigningRequests/{id}/SignedArtifact  签名产物二进制

配置(环境变量;API 令牌绝不入库,见 docs/ops/code-signing.md §3):
  SIGNPATH_API_TOKEN            必填 CI 用户令牌
  SIGNPATH_ORGANIZATION_ID      必填 组织 ID(GUID)
  SIGNPATH_PROJECT_SLUG         必填
  SIGNPATH_SIGNING_POLICY_SLUG  必填
  SIGNPATH_ARTIFACT_CONFIG_SLUG 可选,缺省用项目默认配置
  SIGNPATH_TIMEOUT_MINUTES      可选,默认 30(审批等待可能很慢)
  SIGNPATH_POLL_SECONDS         可选,默认 10

用法:
    python tools/release/signpath-sign.py --file prod/releases/qidiwork-setup-0.1.13.exe
    python tools/release/signpath-sign.py --file <exe> --param productVersion=0.1.13 --verify

输出(stdout 单行 JSON,可留档):
    {"signingRequestId": "...", "statusUrl": "...", "file": "...", "sha256": "...", "size_bytes": N}
"""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

BASE_URL = "https://app.signpath.io/Api/v1"
# 仅测试用:指向本地 mock。生产不设该变量即走官方 API。
BASE_URL_OVERRIDE_ENV = "SIGNPATH_BASE_URL"
CHUNK = 1024 * 1024
USER_AGENT = "qidiwork-signpath-sign/1.0"


class SignPathError(RuntimeError):
    """面向运维的错误:消息里带状态/URL/重试次数,不裸抛堆栈。"""


def _request(method: str, url: str, token: str, *, data: bytes | None = None,
             headers: dict[str, str] | None = None, timeout: int = 300,
             dest: Path | None = None, attempts: int = 3):
    """带重试的 HTTP 调用。dest 非空时把响应体流式写 dest.part 并返回该临时路径。

    重试只覆盖网络错误 / 429 / 5xx;其余 4xx 直接抛(重试同样的提交没有意义)。
    """
    last = ""
    for i in range(attempts):
        req = urllib.request.Request(url, data=data, method=method)
        req.add_header("Authorization", f"Bearer {token}")
        req.add_header("User-Agent", USER_AGENT)
        for k, v in (headers or {}).items():
            req.add_header(k, v)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                if dest is not None:
                    tmp = dest.with_name(dest.name + ".part")
                    try:
                        with open(tmp, "wb") as f:
                            shutil.copyfileobj(resp, f, CHUNK)
                    except BaseException:
                        tmp.unlink(missing_ok=True)
                        raise
                    return resp.status, resp.headers, tmp
                return resp.status, resp.headers, resp.read()
        except urllib.error.HTTPError as e:
            body = e.read().decode("utf-8", "replace")[:600]
            last = f"HTTP {e.code}: {body}"
            if e.code != 429 and e.code < 500:
                raise SignPathError(f"{method} {url} 失败({e.code}): {body}") from None
        except (OSError, http.client.HTTPException) as e:
            # URLError/ConnectionError/TimeoutError 都是 OSError 子类;
            # RemoteDisconnected 等 HTTPException 不在其中,单独兜住,统一按可重试网络错误处理。
            last = f"网络错误: {e}"
        if i + 1 < attempts:
            time.sleep(min(5 * (2 ** i), 30))
    raise SignPathError(f"{method} {url} 重试 {attempts} 次仍失败: {last}")


def _build_multipart(fields: list[tuple[str, str]], file_field: str,
                     file_path: Path) -> tuple[bytes, str]:
    boundary = f"----qidiwork-{uuid.uuid4().hex}"
    parts: list[bytes] = []
    for name, value in fields:
        parts.append(f"--{boundary}\r\n".encode())
        parts.append(f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode())
        parts.append(f"{value}\r\n".encode())
    parts.append(f"--{boundary}\r\n".encode())
    parts.append(
        f'Content-Disposition: form-data; name="{file_field}"; '
        f'filename="{file_path.name}"\r\n'.encode())
    parts.append(b"Content-Type: application/octet-stream\r\n\r\n")
    parts.append(file_path.read_bytes())
    parts.append(b"\r\n")
    parts.append(f"--{boundary}--\r\n".encode())
    return b"".join(parts), f"multipart/form-data; boundary={boundary}"


def submit(base_url: str, token: str, org: str, project: str, policy: str,
           artifact_config: str | None, description: str | None,
           params: list[tuple[str, str]], file_path: Path) -> str:
    fields = [("projectSlug", project), ("signingPolicySlug", policy)]
    if artifact_config:
        fields.append(("artifactConfigurationSlug", artifact_config))
    if description:
        fields.append(("description", description))
    fields.extend((f"parameters.{k}", v) for k, v in params)
    body, content_type = _build_multipart(fields, "artifact", file_path)
    url = f"{base_url}/{org}/SigningRequests/SubmitWithArtifact"
    status, headers, _ = _request("POST", url, token, data=body,
                                  headers={"Content-Type": content_type},
                                  timeout=600)
    if status != 201:
        raise SignPathError(f"提交失败: 期望 201,得到 {status}")
    location = headers.get("Location")
    if not location:
        raise SignPathError("提交成功(201)但响应缺少 Location 头,无法轮询/下载")
    return location


def wait_for_completion(base_url: str, token: str, location: str,
                        timeout_min: int, poll_sec: int) -> str:
    """轮询到 isFinalStatus,返回最终 status。审批流可能等很久,超时不撤请求。"""
    deadline = time.monotonic() + timeout_min * 60
    status_url = f"{location}/Status"
    last = ""
    while True:
        _, _, body = _request("GET", status_url, token)
        data = json.loads(body)
        st = data.get("status", "?")
        ws = data.get("workflowStatus", "")
        if st != last:
            print(f"[signpath] 状态: {st}({ws})", file=sys.stderr)
            last = st
        if data.get("isFinalStatus"):
            return st
        if time.monotonic() > deadline:
            raise SignPathError(
                f"轮询超时({timeout_min} 分钟),最后状态 {st}({ws});"
                f"请求仍在,可手动查看 {location} 或调大 SIGNPATH_TIMEOUT_MINUTES")
        time.sleep(poll_sec)


def download_signed(token: str, location: str, dest: Path) -> None:
    _, _, tmp = _request("GET", f"{location}/SignedArtifact", token,
                         timeout=900, dest=dest)
    os.replace(tmp, dest)


def find_signtool() -> str | None:
    kits = Path("C:/Program Files (x86)/Windows Kits/10/bin")
    if not kits.is_dir():
        return None
    candidates = sorted(kits.glob("*/x64/signtool.exe"))
    return str(candidates[-1]) if candidates else None


def verify_with_signtool(signtool: str, file: Path) -> None:
    r = subprocess.run([signtool, "verify", "/pa", "/q", str(file)],
                       capture_output=True, text=True, errors="replace")
    if r.returncode != 0:
        raise SignPathError(
            f"signtool 验签失败(退出码 {r.returncode}): "
            f"{(r.stdout + r.stderr).strip()[-600:]}")


def required_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        sys.exit(f"缺少环境变量 {name}(配置见 docs/ops/code-signing.md §3)")
    return value


def _run(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="提交安装包到 SignPath 做 Authenticode 代码签名并拉回签名产物")
    parser.add_argument("--file", required=True, type=Path, help="待签名的 exe")
    parser.add_argument("--out", type=Path, default=None,
                        help="签名产物输出路径,缺省就地替换 --file")
    parser.add_argument("--description", default=None,
                        help="签名请求描述,如版本号(缺省用文件名)")
    parser.add_argument("--param", action="append", default=[],
                        metavar="KEY=VALUE",
                        help="用户定义参数,如 --param productVersion=0.1.13,可重复")
    parser.add_argument("--verify", nargs="?", const="auto", default=None,
                        metavar="SIGNTOOL",
                        help="拉回后用 signtool 自验签;省略路径=自动探测(auto)")
    args = parser.parse_args(argv)

    src = args.file
    if not src.is_file():
        sys.exit(f"文件不存在: {src}")
    dest = args.out or src
    params: list[tuple[str, str]] = []
    for item in args.param:
        if "=" not in item:
            sys.exit(f"--param 需要 KEY=VALUE 形式,得到: {item}")
        k, v = item.split("=", 1)
        params.append((k.strip(), v.strip()))

    token = required_env("SIGNPATH_API_TOKEN")
    org = required_env("SIGNPATH_ORGANIZATION_ID")
    project = required_env("SIGNPATH_PROJECT_SLUG")
    policy = required_env("SIGNPATH_SIGNING_POLICY_SLUG")
    artifact_config = os.environ.get("SIGNPATH_ARTIFACT_CONFIG_SLUG", "").strip() or None
    timeout_min = int(os.environ.get("SIGNPATH_TIMEOUT_MINUTES", "30"))
    poll_sec = int(os.environ.get("SIGNPATH_POLL_SECONDS", "10"))
    description = args.description or f"{src.name} (qidiwork release)"
    base_url = os.environ.get(BASE_URL_OVERRIDE_ENV, "").strip().rstrip("/") or BASE_URL

    print(f"[signpath] 提交 {src} ({src.stat().st_size} 字节) "
          f"项目={project} 策略={policy}", file=sys.stderr)
    original_size = src.stat().st_size
    location = submit(base_url, token, org, project, policy, artifact_config,
                      description, params, src)
    print(f"[signpath] 签名请求已创建: {location}", file=sys.stderr)

    status = wait_for_completion(base_url, token, location, timeout_min, poll_sec)
    if status != "Completed":
        sys.exit(f"签名未完成: status={status}(查看 {location})")
    print("[signpath] 签名完成,拉回产物", file=sys.stderr)

    download_signed(token, location, dest)
    digest = hashlib.sha256()
    with open(dest, "rb") as f:
        for block in iter(lambda: f.read(CHUNK), b""):
            digest.update(block)
    if dest.stat().st_size == original_size:
        print("[signpath] 注意: 产物大小与提交时一致,请人工确认签名已生效",
              file=sys.stderr)

    if args.verify:
        signtool = args.verify if args.verify != "auto" else find_signtool()
        if not signtool:
            sys.exit("--verify 已指定但找不到 signtool(可显式传路径)")
        print(f"[signpath] signtool 自验签: {signtool}", file=sys.stderr)
        verify_with_signtool(signtool, dest)
        print("[signpath] signtool 验签通过", file=sys.stderr)

    print(json.dumps({
        "signingRequestId": location.rstrip("/").rsplit("/", 1)[-1],
        "statusUrl": location,
        "file": dest.name,
        "sha256": digest.hexdigest(),
        "size_bytes": dest.stat().st_size,
    }))
    return 0


def main(argv: list[str] | None = None) -> int:
    """CLI 入口:把面向运维的错误收敛成一行 stderr + 退出码 1,不裸抛堆栈。"""
    try:
        return _run(argv)
    except SignPathError as e:
        print(f"signpath 失败: {e}", file=sys.stderr)
        return 1
    except OSError as e:
        print(f"signpath 失败: 文件/网络 IO 错误: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
