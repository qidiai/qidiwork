#!/usr/bin/env python3
"""生成更新包签名用的 Ed25519 密钥对。

与客户端契约（gui/src-tauri/src/updater.rs）对齐：
  - 公钥：32 字节 raw，base64 编码，编译期经 QIDIWORK_UPDATER_PUBKEY 注入；
  - 签名：对 sha256 的 32 字节**原始摘要**做 Ed25519，输出 64 字节 base64。

用法：
    python tools/release/gen-update-key.py [--force]

无参运行时在当前目录生成 `update-signing-key.pem`（PKCS8 Ed25519 私钥），
并打印两行可直接使用的环境变量：
    PRIVATE_KEY=<abs path>
    PUBLIC_KEY_B64=<base64 of 32-byte raw public key>

已存在同名文件时拒绝覆盖，除非显式 --force。
"""

from __future__ import annotations

import argparse
import base64
import os
import sys
from pathlib import Path

try:
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
except ImportError:  # pragma: no cover - 环境缺依赖时的友好提示
    sys.exit("缺少依赖：请先 `pip install cryptography`")

KEY_FILENAME = "update-signing-key.pem"


def public_key_b64(private_key: Ed25519PrivateKey) -> str:
    """导出 32 字节 raw 公钥并 base64 编码（与客户端 B64.decode → [u8;32] 完全对齐）。"""
    raw = private_key.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    # 32 字节 raw 不应带 padding 以外的变形；standard b64 与客户端 base64 crate 默认字母表一致。
    assert len(raw) == 32, f"公钥长度异常：{len(raw)}"
    return base64.b64encode(raw).decode("ascii")


def write_private_pem(path: Path, private_key: Ed25519PrivateKey) -> None:
    pem = private_key.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.PKCS8,
        encryption_algorithm=serialization.NoEncryption(),
    )
    path.write_bytes(pem)
    if os.name != "nt":  # Windows 上 chmod 语义有限，靠目录 ACL 保护
        try:
            os.chmod(path, 0o600)
        except OSError:
            pass


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="生成 Ed25519 更新包签名密钥对（update-signing-key.pem）",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help=f"覆盖已存在的 {KEY_FILENAME}（默认拒绝，避免误换钥匙导致老客户端无法验签）",
    )
    parser.add_argument(
        "--out-dir",
        default=".",
        help="私钥输出目录（默认当前目录）",
    )
    args = parser.parse_args(argv)

    out_dir = Path(args.out_dir).expanduser().resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    key_path = out_dir / KEY_FILENAME

    if key_path.exists() and not args.force:
        print(
            f"拒绝覆盖：{key_path} 已存在。\n"
            "换钥匙会让旧客户端无法验证新包（见 tools/release/README.md「失钥处置」）。\n"
            "确实要重新生成请加 --force。",
            file=sys.stderr,
        )
        return 2

    private_key = Ed25519PrivateKey.generate()
    write_private_pem(key_path, private_key)

    print(f"PRIVATE_KEY={key_path}")
    print(f"PUBLIC_KEY_B64={public_key_b64(private_key)}")
    print("", file=sys.stderr)
    print(
        "提醒：私钥绝不入库，请移动到 G:\\qidiwork-secrets\\ 并做好备份；"
        "公钥用 --force 重新生成后必须重新注入并重发客户端版本。",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
