import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const gui = fileURLToPath(new URL('../', import.meta.url));
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'qidi-acp-lifecycle-'));
const quoted = relative => JSON.stringify(path.join(gui, relative).replaceAll('\\', '/'));
fs.writeFileSync(path.join(dir, 'lib.rs'), `#[path = ${quoted('src-tauri/src/transport.rs')}] pub mod transport;
#[path = ${quoted('src-tauri/src/acp/mod.rs')}] pub mod acp;
#[cfg(test)] #[path = ${quoted('tests/orchestra-t2-bridge.rs')}] mod lifecycle_tests;
`);
fs.writeFileSync(path.join(dir, 'Cargo.toml'), `[package]
name = "qidi-acp-lifecycle-check"
version = "0.0.0"
edition = "2024"
[lib]
path = "lib.rs"
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1", features = ["sync", "macros", "rt-multi-thread", "time"] }
tracing = "0.1"
`);
for (const relative of ['src-tauri/src/acp/bridge.rs', 'tests/orchestra-t2-bridge.rs']) {
  console.log(`${relative} SHA256 ${createHash('sha256').update(fs.readFileSync(path.join(gui, relative))).digest('hex')}`);
}
console.log('编译真实ACP桥与传输层，不复制或替换业务逻辑；不覆盖Tauri命令装配或真实内核进程。');
const result = spawnSync('cargo', ['test', '--manifest-path', path.join(dir, 'Cargo.toml'), '--offline', '--', '--nocapture'], {
  cwd: path.dirname(gui),
  env: { ...process.env, CARGO_TARGET_DIR: path.join(os.tmpdir(), 'qidi-acp-lifecycle-target') },
  stdio: 'inherit',
});
if (result.error) console.error(result.error.message);
process.exitCode = result.status ?? 1;
