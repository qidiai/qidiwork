#!/usr/bin/env python3
"""为安装包生成清单字段：sha256(hex) + Ed25519 sig(base64)。

与客户端契约（gui/src-tauri/src/updater.rs 的 verify_ed25519 / check_signature）逐字节对齐：
  - 签名对象 = sha256 的 **32 字节原始摘要**（不是 hex 字符串）；
  - 签名 = Ed25519，输出 **base64（64 字节）**；
  - 公钥 = base64（32 字节 raw），编译期经 QIDIWORK_UPDATER_PUBKEY 注入。

用法：
    python tools/release/sign-update.py --key <pem> --file prod/releases/qidiwork-setup-x.exe [--version 0.1.12]
    python tools/release/sign-update.py --key <pem> --file <exe> --verify
    # 自检（对给定签名做验签，篡改一个字符应失败）：
    python tools/release/sign-update.py --key <pem> --file <exe> --sig <b64> --verify

输出（stdout，可直接粘进 prod/releases/latest-*.json 的 gui 段）：
    {"version": ..., "file": ..., "sha256": "<hex>", "size_bytes": N, "sig": "<b64>"}
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import sys
from pathlib import Path

try:
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
except ImportError:  # pragma: no cover
    sys.exit("缺少依赖：请先 `pip install cryptography`")

CHUNK = 1024 * 1024
# 在去掉扩展名的文件名上匹配：qidiwork-setup-0.1.11-r4 / qidiwork-kernel-0.1.220-alpha.6
VERSION_RE = re.compile(r"(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)")


def sha256_of(path: Path) -> tuple[str, bytes, int]:
    """流式计算 sha256，返回 (hex, 32 字节原始摘要, 文件字节数)。"""
    h = hashlib.sha256()
    size = 0
    with path.open("rb") as fh:
        while True:
            chunk = fh.read(CHUNK)
            if not chunk:
                break
            h.update(chunk)
            size += len(chunk)
    digest = h.digest()
    assert len(digest) == 32
    return h.hexdigest(), digest, size


def load_private_key(path: Path) -> Ed25519PrivateKey:
    data = path.read_bytes()
    key = serialization.load_pem_private_key(data, password=None)
    if not isinstance(key, Ed25519PrivateKey):
        sys.exit(f"不是 Ed25519 私钥：{path}")
    return key


def public_key_b64(private_key: Ed25519PrivateKey) -> str:
    raw = private_key.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    return base64.b64encode(raw).decode("ascii")


REBUILD_SUFFIX_RE = re.compile(r"-r\d+$")


def derive_version(exe_name: str) -> str | None:
    """从安装包文件名推导版本。

    - 先去掉扩展名，避免把 `.exe` 吞进版本号；
    - 再去掉重打包后缀 `-rN`（`qidiwork-setup-0.1.11-r4.exe` 的清单 version 是 `0.1.11`，
      而 `-alpha.6` 这类语义化预发布标签要保留）。
    """
    stem = Path(exe_name).stem
    m = VERSION_RE.search(stem)
    if not m:
        return None
    return REBUILD_SUFFIX_RE.sub("", m.group(1).rstrip("."))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="为安装包生成 sha256 + Ed25519 签名（客户端 updater.rs 契约）",
    )
    parser.add_argument("--key", required=True, help="PKCS8 Ed25519 私钥 PEM 路径（绝不入库）")
    parser.add_argument("--file", required=True, help="要签名的安装包路径")
    parser.add_argument(
        "--version",
        default=None,
        help="清单里的 version 字段；缺省时从文件名推导（如 qidiwork-setup-0.1.11-r4.exe → 0.1.11）",
    )
    parser.add_argument(
        "--verify",
        action="store_true",
        help="用私钥导出的公钥自验签一次，通过则打印 SIGN OK",
    )
    parser.add_argument(
        "--sig",
        default=None,
        help="配合 --verify 使用：验签这个给定的 base64 签名（用于篡改自检），默认验本次生成的签名",
    )
    parser.add_argument(
        "--compact",
        action="store_true",
        help="输出单行 JSON（默认 2 空格缩进，便于直接粘进清单）",
    )
    args = parser.parse_args(argv)

    key_path = Path(args.key).expanduser()
    file_path = Path(args.file).expanduser()
    if not key_path.is_file():
        sys.exit(f"私钥不存在：{key_path}")
    if not file_path.is_file():
        sys.exit(f"安装包不存在：{file_path}")

    version = args.version
    if version is None:
        version = derive_version(file_path.name)
        if version is None:
            sys.exit("无法从文件名推导版本，请显式传 --version")
        print(f"提示：未传 --version，从文件名推导为 {version}", file=sys.stderr)

    private_key = load_private_key(key_path)
    sha_hex, digest, size = sha256_of(file_path)

    # 签名对象是 32 字节原始摘要（与 verify_strict(digest, &sig) 一致），不是 hex 字符串
    sig_b64 = base64.b64encode(private_key.sign(digest)).decode("ascii")
    assert len(base64.b64decode(sig_b64)) == 64

    if args.verify:
        to_check = args.sig if args.sig is not None else sig_b64
        try:
            sig_raw = base64.b64decode(to_check.strip(), validate=True)
        except Exception as exc:  # noqa: BLE001 - 用户输入的 base64 可能非法
            print(f"SIGN FAIL: 签名不是合法 base64（{exc}）", file=sys.stderr)
            return 1
        if len(sig_raw) != 64:
            print(f"SIGN FAIL: 签名长度 {len(sig_raw)} != 64", file=sys.stderr)
            return 1
        try:
            private_key.public_key().verify(sig_raw, digest)
        except InvalidSignature:
            print("SIGN FAIL: Ed25519 验签不通过（签名与摘要不匹配）", file=sys.stderr)
            return 1
        print("SIGN OK")

    fragment = {
        "version": version,
        "file": file_path.name,
        "sha256": sha_hex,
        "size_bytes": size,
        "sig": sig_b64,
    }
    print(json.dumps(fragment, ensure_ascii=False, indent=None if args.compact else 2))

    # 便于构建：把公钥也打到 stderr，避免污染可粘贴的 JSON
    print(f"PUBLIC_KEY_B64={public_key_b64(private_key)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
