import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const sourcePath = fileURLToPath(new URL('../src-tauri/src/office/mod.rs', import.meta.url));
const rawSource = fs.readFileSync(sourcePath, 'utf8');
const source = rawSource.replace(/\r\n/g, '\n');
function section(start, end) {
  const a = source.indexOf(start);
  const b = source.indexOf(end, a);
  if (a < 0 || b <= a) throw new Error(`未找到测试抽取边界：${start}`);
  return source.slice(a, b);
}
const code = 'use std::path::{Path, PathBuf};\nuse serde::Serialize;\n'
  + section('pub const MAX_DOCUMENT_BYTES', '/// 产物卡片')
  + section('#[cfg(windows)]\nfn is_local_absolute', '/// 预览读取/系统打开的放行门')
  + section('#[cfg(test)]\nmod tests', '    fn write_manifest') + '}\n';
if ((code.match(/#\[test\]/g) ?? []).length !== 8) throw new Error('聚焦测试数量发生变化，请核对测试抽取范围');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'qidi-office-import-'));
fs.writeFileSync(path.join(dir, 'lib.rs'), code);
fs.writeFileSync(path.join(dir, 'Cargo.toml'), `[package]
name = "qidi-office-import-check"
version = "0.0.0"
edition = "2024"
[lib]
path = "lib.rs"
[dependencies]
serde = { version = "1.0", features = ["derive"] }
base64 = "0.22"
dunce = "1"
tracing = "0.1"
`);
console.log(`仅统一换行符后抽取实际文件导入函数与8项测试；不涵盖 Tauri 命令与桌面接口。\n原始源码 SHA256：${createHash('sha256').update(rawSource).digest('hex')}\n临时测试目录：${dir}`);
const result = spawnSync('cargo', ['test', '--manifest-path', path.join(dir, 'Cargo.toml'), '--offline', '--', '--nocapture'], {
  cwd: fileURLToPath(new URL('../../', import.meta.url)),
  env: { ...process.env, CARGO_TARGET_DIR: path.join(os.tmpdir(), 'qidi-office-import-check', 'target') },
  stdio: 'inherit',
});
if (result.error) console.error(result.error.message);
process.exitCode = result.status ?? 1;
