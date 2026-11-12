//! 官方技能包一键安装(R8):下载 zip → 解压到 `~/.qidi/skills/`。
//!
//! 为什么自带 zip 解压:本 crate 是独立 workspace,「禁止新增依赖」约束下
//! 不能引入 `zip`;`flate2` 只作为 tauri 图标链(`png`)的传递依赖存在,
//! 不可直接使用。因此这里实现**只覆盖官方技能包所需能力**的最小实现:
//!
//! - ZIP 容器:只读中央目录(EOCD + central directory + local header),
//!   支持 stored(0)与 deflate(8);zip64 / 加密 / 其他压缩方式显式拒绝。
//! - DEFLATE(RFC 1951):stored / fixed huffman / dynamic huffman 三种块。
//! - 安全边界:zip slip(`..`、绝对路径、盘符)拒绝、单条目解压上限、
//!   CRC32 校验。
//!
//! 覆盖用户数据策略:顶层名(目录或文件)已存在的条目**整组跳过**,绝不
//! 覆盖用户已有技能。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// 官方技能包下载地址(与 `prod/releases/install-skills.bat` 同一地址)。
pub const SKILLS_PACKAGE_URL: &str =
    "https://qidiwork.qidiai.ltd/releases/qidiwork-skills-v0.1.5.zip";

/// 单条目解压上限(zip bomb 防护;官方包最大条目为 ~242KB 的 XSD,余量充足)。
const MAX_ENTRY_UNCOMPRESSED: u64 = 32 * 1024 * 1024;

/// 安装结果(前端展示用)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallReport {
    /// 新增的技能目录数(解压前后顶层目录差集)。
    pub installed: usize,
    /// 技能根目录绝对路径。
    pub path: String,
}

// ---------------------------------------------------------------- DEFLATE

/// RFC 1951 位流读取器(位序:字节内 LSB first)。
struct BitReader<'a> {
    data: &'a [u8],
    bitpos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bitpos: 0 }
    }

    fn read_bit(&mut self) -> Result<u32, String> {
        let byte = self.bitpos >> 3;
        let Some(b) = self.data.get(byte) else {
            return Err("deflate 数据提前结束".to_string());
        };
        let bit = (b >> (self.bitpos & 7)) & 1;
        self.bitpos += 1;
        Ok(bit as u32)
    }

    fn read_bits(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0u32;
        for i in 0..n {
            v |= self.read_bit()? << i;
        }
        Ok(v)
    }

    /// 跳到下一个字节边界(存储块头之后)。
    fn align_byte(&mut self) {
        self.bitpos = (self.bitpos + 7) & !7;
    }

    /// 读取 n 个整字节(仅在字节对齐时调用,存储块数据用)。
    fn read_aligned(&mut self, n: usize) -> Result<&'a [u8], String> {
        debug_assert_eq!(self.bitpos % 8, 0);
        let start = self.bitpos / 8;
        let end = start + n;
        let Some(slice) = self.data.get(start..end) else {
            return Err("deflate 数据提前结束".to_string());
        };
        self.bitpos = end * 8;
        Ok(slice)
    }
}

/// huffman 码表(以码长计数 + 按码长排序的符号表表示,同 zlib puff 做法)。
const MAX_BITS: usize = 15;

struct Huffman {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    fn build(lengths: &[u8]) -> Result<Self, String> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            if l as usize > MAX_BITS {
                return Err("deflate huffman 码长越界".to_string());
            }
            count[l as usize] += 1;
        }
        // 全零码表在 deflate 里合法(该树本次不被使用),解码时才会失败
        if count[0] as usize == lengths.len() {
            return Ok(Self {
                count,
                symbol: vec![0u16; lengths.len()],
            });
        }
        // 过订阅检测(left < 0 = 码表非法)
        let mut left: i32 = 1;
        for c in count.iter().skip(1) {
            left <<= 1;
            left -= *c as i32;
            if left < 0 {
                return Err("deflate huffman 码表过订阅".to_string());
            }
        }
        let mut offs = [0usize; MAX_BITS + 2];
        for len in 1..MAX_BITS {
            offs[len + 1] = offs[len] + count[len] as usize;
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize]] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Self { count, symbol })
    }

    fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, String> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: usize = 0;
        for len in 1..=MAX_BITS {
            code |= br.read_bit()? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                let idx = index + (code - first) as usize;
                return self
                    .symbol
                    .get(idx)
                    .copied()
                    .ok_or_else(|| "deflate huffman 符号越界".to_string());
            }
            index += count as usize;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("deflate huffman 解码失败(码表不完整)".to_string())
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// 动态 huffman 里码长码的传输顺序(RFC 1951 §3.2.7)。
const CLEN_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// 解压裸 deflate 流(无 zlib/gzip 头)。`expected` 为中央目录声明的原始长度。
pub fn inflate_raw(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut br = BitReader::new(data);
    let mut out: Vec<u8> = Vec::with_capacity(expected.min(MAX_ENTRY_UNCOMPRESSED as usize));
    loop {
        let bfinal = br.read_bit()?;
        let btype = br.read_bits(2)?;
        match btype {
            0 => {
                br.align_byte();
                let len = br.read_bits(16)? as usize;
                let nlen = br.read_bits(16)? as usize;
                if len as u32 != !(nlen as u32) & 0xFFFF {
                    return Err("deflate 存储块长度校验失败".to_string());
                }
                out.extend_from_slice(br.read_aligned(len)?);
            }
            1 => {
                let (lit, dist) = fixed_tables()?;
                inflate_block(&mut br, &mut out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_tables(&mut br)?;
                inflate_block(&mut br, &mut out, &lit, &dist)?;
            }
            _ => return Err("deflate 块类型 3 非法".to_string()),
        }
        if bfinal == 1 {
            break;
        }
        if out.len() > MAX_ENTRY_UNCOMPRESSED as usize {
            return Err("deflate 输出超过单条目上限".to_string());
        }
    }
    if out.len() != expected {
        return Err(format!(
            "deflate 输出长度不符:期望 {expected} 字节,实际 {} 字节",
            out.len()
        ));
    }
    Ok(out)
}

fn fixed_tables() -> Result<(Huffman, Huffman), String> {
    let mut lit = [0u8; 288];
    for (i, l) in lit.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    // 固定距离码:32 个 5 位码,符号 30/31 不出现,按 puff 取前 30 个
    let dist = [5u8; 30];
    Ok((Huffman::build(&lit)?, Huffman::build(&dist)?))
}

fn dynamic_tables(br: &mut BitReader<'_>) -> Result<(Huffman, Huffman), String> {
    let hlit = br.read_bits(5)? as usize + 257;
    let hdist = br.read_bits(5)? as usize + 1;
    let hclen = br.read_bits(4)? as usize + 4;
    let mut clen = [0u8; 19];
    for &slot in CLEN_ORDER.iter().take(hclen) {
        clen[slot] = br.read_bits(3)? as u8;
    }
    let clen_tree = Huffman::build(&clen)?;

    let total = hlit + hdist;
    let mut lengths = vec![0u8; total];
    let mut i = 0usize;
    while i < total {
        let sym = clen_tree.decode(br)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err("deflate 重复码无前置码长".to_string());
                }
                let prev = lengths[i - 1];
                let n = 3 + br.read_bits(2)? as usize;
                for _ in 0..n {
                    if i >= total {
                        return Err("deflate 码长重复越界".to_string());
                    }
                    lengths[i] = prev;
                    i += 1;
                }
            }
            17 | 18 => {
                let n = if sym == 17 {
                    3 + br.read_bits(3)? as usize
                } else {
                    11 + br.read_bits(7)? as usize
                };
                for _ in 0..n {
                    if i >= total {
                        return Err("deflate 码长重复越界".to_string());
                    }
                    lengths[i] = 0;
                    i += 1;
                }
            }
            _ => return Err("deflate 码长码越界".to_string()),
        }
    }
    Ok((
        Huffman::build(&lengths[..hlit])?,
        Huffman::build(&lengths[hlit..])?,
    ))
}

fn inflate_block(
    br: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), String> {
    loop {
        if out.len() > MAX_ENTRY_UNCOMPRESSED as usize {
            return Err("deflate 输出超过单条目上限".to_string());
        }
        let sym = lit.decode(br)? as usize;
        if sym < 256 {
            out.push(sym as u8);
            continue;
        }
        if sym == 256 {
            return Ok(());
        }
        let li = sym - 257;
        let Some(&base) = LENGTH_BASE.get(li) else {
            return Err("deflate 长度码越界".to_string());
        };
        let length = base as usize + br.read_bits(LENGTH_EXTRA[li] as u32)? as usize;
        let dsym = dist.decode(br)? as usize;
        let Some(&dbase) = DIST_BASE.get(dsym) else {
            return Err("deflate 距离码越界".to_string());
        };
        let distance = dbase as usize + br.read_bits(DIST_EXTRA[dsym] as u32)? as usize;
        if distance == 0 || distance > out.len() {
            return Err("deflate 回溯距离越界".to_string());
        }
        let start = out.len() - distance;
        for k in 0..length {
            let b = out[start + k];
            out.push(b);
        }
    }
}

// -------------------------------------------------------------------- ZIP

fn le_u16(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn le_u32(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// CRC-32(IEEE 802.3),zip 条目完整性校验用。
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

struct ZipEntry {
    name: String,
    method: u16,
    crc: u32,
    compressed_size: usize,
    uncompressed_size: usize,
    local_offset: usize,
}

const EOCD_SIG: u32 = 0x0605_4B50;
const CENTRAL_SIG: u32 = 0x0201_4B50;
const LOCAL_SIG: u32 = 0x0403_4B50;

/// 从尾部找中央目录结尾记录(EOCD),容忍最多 64KB 注释。
fn find_eocd(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 22 {
        return None;
    }
    let lowest = bytes.len().saturating_sub(22 + 0xFFFF);
    let mut i = bytes.len() - 22;
    loop {
        if le_u32(bytes, i) == Some(EOCD_SIG) {
            return Some(i);
        }
        if i == lowest {
            return None;
        }
        i -= 1;
    }
}

fn parse_entries(bytes: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let eocd = find_eocd(bytes).ok_or("技能包损坏:未找到 zip 中央目录结尾")?;
    let total = le_u16(bytes, eocd + 10).ok_or("技能包损坏:中央目录结尾不完整")? as usize;
    let cd_off = le_u32(bytes, eocd + 16).ok_or("技能包损坏:中央目录偏移缺失")? as usize;
    if total == 0xFFFF || cd_off == 0xFFFF_FFFF {
        return Err("技能包使用了 zip64 扩展,暂不支持".to_string());
    }
    if cd_off >= bytes.len() {
        return Err("技能包损坏:中央目录偏移越界".to_string());
    }
    let mut out = Vec::with_capacity(total);
    let mut p = cd_off;
    for _ in 0..total {
        if le_u32(bytes, p) != Some(CENTRAL_SIG) {
            return Err("技能包损坏:中央目录条目签名不匹配".to_string());
        }
        let flag = le_u16(bytes, p + 8).unwrap_or(0);
        let method = le_u16(bytes, p + 10).unwrap_or(0);
        let crc = le_u32(bytes, p + 16).unwrap_or(0);
        let compressed = le_u32(bytes, p + 20).unwrap_or(0);
        let uncompressed = le_u32(bytes, p + 24).unwrap_or(0);
        let nlen = le_u16(bytes, p + 28).unwrap_or(0) as usize;
        let elen = le_u16(bytes, p + 30).unwrap_or(0) as usize;
        let clen = le_u16(bytes, p + 32).unwrap_or(0) as usize;
        let local_offset = le_u32(bytes, p + 42).unwrap_or(0);
        let name_bytes = bytes
            .get(p + 46..p + 46 + nlen)
            .ok_or("技能包损坏:条目名越界")?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        if flag & 0x1 != 0 {
            return Err(format!("技能包条目已加密,不支持:{name}"));
        }
        if compressed == 0xFFFF_FFFF || uncompressed == 0xFFFF_FFFF || local_offset == 0xFFFF_FFFF
        {
            return Err(format!("技能包条目使用 zip64 扩展,暂不支持:{name}"));
        }
        out.push(ZipEntry {
            name,
            method,
            crc,
            compressed_size: compressed as usize,
            uncompressed_size: uncompressed as usize,
            local_offset: local_offset as usize,
        });
        p += 46 + nlen + elen + clen;
    }
    Ok(out)
}

/// 读取单个条目的原始字节(stored 直取,deflate 解压),并校验长度与 CRC。
fn read_entry_data(bytes: &[u8], e: &ZipEntry) -> Result<Vec<u8>, String> {
    if le_u32(bytes, e.local_offset) != Some(LOCAL_SIG) {
        return Err(format!("技能包损坏:本地头签名不匹配:{}", e.name));
    }
    if e.uncompressed_size as u64 > MAX_ENTRY_UNCOMPRESSED {
        return Err(format!("技能包条目过大,已拒绝:{}", e.name));
    }
    let nlen = le_u16(bytes, e.local_offset + 26).unwrap_or(0) as usize;
    let elen = le_u16(bytes, e.local_offset + 28).unwrap_or(0) as usize;
    let start = e.local_offset + 30 + nlen + elen;
    let raw = bytes
        .get(start..start + e.compressed_size)
        .ok_or_else(|| format!("技能包损坏:条目数据越界:{}", e.name))?;
    let data = match e.method {
        0 => raw.to_vec(),
        8 => inflate_raw(raw, e.uncompressed_size)
            .map_err(|err| format!("技能包解压失败({}):{err}", e.name))?,
        m => return Err(format!("技能包压缩方式不支持({m}):{}", e.name)),
    };
    if crc32(&data) != e.crc {
        return Err(format!("技能包条目校验失败(CRC 不符):{}", e.name));
    }
    Ok(data)
}

/// 条目名 → 安全相对路径组件。拒绝绝对路径、盘符与 `..`(zip slip)。
fn sanitize_name(raw: &str) -> Result<Vec<String>, String> {
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(format!("技能包条目为绝对路径,已拒绝:{raw}"));
    }
    let mut parts = Vec::new();
    for seg in normalized.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(format!("技能包条目含上级目录引用,已拒绝:{raw}"));
        }
        if seg.as_bytes().get(1) == Some(&b':') {
            return Err(format!("技能包条目含盘符,已拒绝:{raw}"));
        }
        parts.push(seg.to_string());
    }
    Ok(parts)
}

fn top_level_dirs(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut set = BTreeSet::new();
    let entries = std::fs::read_dir(root).map_err(|e| format!("读取技能目录失败:{e}"))?;
    for entry in entries.flatten() {
        if entry.path().is_dir()
            && let Ok(name) = entry.file_name().into_string()
        {
            set.insert(name);
        }
    }
    Ok(set)
}

/// 解压技能包字节流到 `dest`(通常是 `~/.qidi/skills`)。
///
/// 顶层名已存在(目录或文件)的条目**整组跳过**,不覆盖用户已有数据。
/// 返回本次新增的顶层目录名(升序),供调用方计数。
pub fn extract_zip_to_dir(bytes: &[u8], dest: &Path) -> Result<Vec<String>, String> {
    let entries = parse_entries(bytes)?;
    std::fs::create_dir_all(dest).map_err(|e| format!("创建技能目录失败:{e}"))?;

    let dirs_before = top_level_dirs(dest)?;
    let names_before: BTreeSet<String> = std::fs::read_dir(dest)
        .map_err(|e| format!("读取技能目录失败:{e}"))?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    for entry in &entries {
        let parts = sanitize_name(&entry.name)?;
        let Some(top) = parts.first() else {
            continue;
        };
        if names_before.contains(top) {
            continue; // 已存在同名顶层条目:跳过,绝不覆盖用户数据
        }
        let target: PathBuf = parts.iter().fold(dest.to_path_buf(), |acc, p| acc.join(p));
        let is_dir = entry.name.ends_with('/') || entry.name.ends_with('\\');
        if is_dir {
            std::fs::create_dir_all(&target)
                .map_err(|e| format!("创建目录失败({}):{e}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败({}):{e}", parent.display()))?;
        }
        let data = read_entry_data(bytes, entry)?;
        std::fs::write(&target, &data)
            .map_err(|e| format!("写入技能文件失败({}):{e}", target.display()))?;
    }

    let dirs_after = top_level_dirs(dest)?;
    Ok(dirs_after.difference(&dirs_before).cloned().collect())
}

// ------------------------------------------------------------ 内置技能包

/// 解压随 GUI 二进制内置的技能包(`include_bytes!` 嵌入 zip)到 `dest`。
///
/// 首启自动释放:复用 `extract_zip_to_dir` 的幂等语义——顶层同名技能整组
/// 跳过,绝不覆盖用户已有技能;返回本次新增的顶层目录名。
pub fn extract_bundled_skills(dest: &Path) -> Result<Vec<String>, String> {
    let bytes = include_bytes!("../resources/qidiwork-skills-bundle.zip");
    extract_zip_to_dir(bytes, dest)
}

// ------------------------------------------------------------ 下载 + 安装

/// 下载官方技能包字节流。
///
/// 总超时 60s(技能包 ~750KB;官方 updater 的 10s 不适用于下载)。
pub async fn download_package() -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("无法初始化网络客户端:{e}"))?;
    let resp = client
        .get(SKILLS_PACKAGE_URL)
        .send()
        .await
        .map_err(|e| format!("技能包下载失败:{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("技能包下载失败:服务器返回 {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("技能包下载中断:{e}"))?;
    Ok(bytes.to_vec())
}

/// 下载官方技能包 → 落 %TEMP%(与 `install-skills.bat` 同行为,顺带暴露
/// 磁盘写失败)→ 解压到 `dest` → 删除临时文件。返回新增顶层目录名。
pub async fn download_and_extract(dest: &Path) -> Result<Vec<String>, String> {
    let bytes = download_package().await?;
    let tmp = std::env::temp_dir().join(format!("qidiwork-skills-{}.zip", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("技能包写入临时文件失败(磁盘空间?):{e}"))?;
    let result = std::fs::read(&tmp)
        .map_err(|e| format!("技能包临时文件读取失败:{e}"))
        .and_then(|b| extract_zip_to_dir(&b, dest));
    let _ = std::fs::remove_file(&tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// .NET DeflateStream 生成的裸 deflate 流(UTF-8: "QidiWork 技能包解压测试 " × 200 + "END"),
    /// 覆盖动态 huffman 块 + 回溯复制。
    const DEFLATE_DYNAMIC: &[u8] = &[
        0xed, 0xc9, 0xb1, 0x0d, 0x40, 0x40, 0x00, 0x00, 0xc0, 0x55, 0x0c, 0x43, 0x2b, 0xf9, 0xca,
        0x02, 0x1a, 0x51, 0x48, 0x4c, 0x20, 0x11, 0x0d, 0xa2, 0x33, 0x81, 0x42, 0xa9, 0x52, 0x1a,
        0xe7, 0xe5, 0xc7, 0x30, 0x88, 0xbb, 0xf6, 0x42, 0x53, 0x37, 0x55, 0xd7, 0xb7, 0xd9, 0x3b,
        0x0f, 0x69, 0x7c, 0xe2, 0x3a, 0xa5, 0xf3, 0x88, 0xdb, 0xf2, 0xde, 0x4b, 0xba, 0xf6, 0x2c,
        0x68, 0xad, 0xb5, 0xd6, 0x5a, 0x6b, 0xad, 0xb5, 0xd6, 0x5a, 0x6b, 0xad, 0xb5, 0xd6, 0xfa,
        0x3f, 0x5d, 0x94, 0xf9, 0x07,
    ];

    /// 同一压缩器对 "hello world" 的输出(固定 huffman 块)。
    const DEFLATE_FIXED: &[u8] = &[
        0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x28, 0xcf, 0x2f, 0xca, 0x49, 0x01, 0x00,
    ];

    fn dynamic_plain() -> String {
        "QidiWork 技能包解压测试 ".repeat(200) + "END"
    }

    #[test]
    fn crc32_matches_known_answer() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn inflate_dynamic_huffman_block() {
        let plain = dynamic_plain();
        let got = inflate_raw(DEFLATE_DYNAMIC, plain.len()).unwrap();
        assert_eq!(String::from_utf8(got).unwrap(), plain);
    }

    #[test]
    fn inflate_fixed_huffman_block() {
        let got = inflate_raw(DEFLATE_FIXED, 11).unwrap();
        assert_eq!(got, b"hello world");
    }

    #[test]
    fn inflate_stored_block() {
        // BFINAL=1 BTYPE=00 → 0x01,然后对齐 + LEN/NLEN(LE)+ 原始字节
        let payload = b"stored-block-payload";
        let mut z = vec![0x01u8];
        z.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(payload.len() as u16)).to_le_bytes());
        z.extend_from_slice(payload);
        assert_eq!(inflate_raw(&z, payload.len()).unwrap(), payload);
    }

    #[test]
    fn inflate_rejects_truncated_and_length_mismatch() {
        assert!(inflate_raw(&DEFLATE_FIXED[..5], 11).is_err());
        assert!(inflate_raw(DEFLATE_FIXED, 12).is_err());
    }

    /// 构造最小 zip(stored 方法):返回 (zip 字节, 各条目内容)。
    fn build_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let name_bytes = name.as_bytes();
            let offset = body.len() as u32;
            let crc = crc32(data);
            body.extend_from_slice(&LOCAL_SIG.to_le_bytes());
            body.extend_from_slice(&20u16.to_le_bytes()); // version needed
            body.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8
            body.extend_from_slice(&0u16.to_le_bytes()); // method: stored
            body.extend_from_slice(&0u16.to_le_bytes()); // time
            body.extend_from_slice(&0u16.to_le_bytes()); // date
            body.extend_from_slice(&crc.to_le_bytes());
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes()); // extra len
            body.extend_from_slice(name_bytes);
            body.extend_from_slice(data);

            central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0x0800u16.to_le_bytes()); // flags
            central.extend_from_slice(&0u16.to_le_bytes()); // method
            central.extend_from_slice(&0u16.to_le_bytes()); // time
            central.extend_from_slice(&0u16.to_le_bytes()); // date
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk start
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name_bytes);
        }
        let cd_off = body.len() as u32;
        let cd_size = central.len() as u32;
        let mut zip = body;
        zip.extend_from_slice(&central);
        zip.extend_from_slice(&EOCD_SIG.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // disk
        zip.extend_from_slice(&0u16.to_le_bytes()); // cd disk
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&cd_size.to_le_bytes());
        zip.extend_from_slice(&cd_off.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // comment len
        zip
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "qidiwork-skillpkg-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn extract_zip_writes_files_and_reports_new_dirs() {
        // 目录条目 + 反斜杠分隔(与官方包一致)+ 顶层散文件
        let zip = build_stored_zip(&[
            ("office-tools\\scripts\\", b""),
            ("office-tools\\SKILL.md", b"---\nname: office-tools\n---\n"),
            ("office-tools\\scripts\\run.py", b"print(1)\n"),
            ("bid-write/SKILL.md", b"---\nname: bid-write\n---\n"),
            ("安装说明.txt", b"install notes"),
        ]);
        let dest = temp_dir("newdirs");
        let added = extract_zip_to_dir(&zip, &dest).unwrap();
        assert_eq!(added, vec!["bid-write".to_string(), "office-tools".to_string()]);
        assert_eq!(
            std::fs::read(dest.join("office-tools").join("SKILL.md")).unwrap(),
            b"---\nname: office-tools\n---\n"
        );
        assert_eq!(
            std::fs::read(dest.join("office-tools").join("scripts").join("run.py")).unwrap(),
            b"print(1)\n"
        );
        assert_eq!(
            std::fs::read(dest.join("安装说明.txt")).unwrap(),
            b"install notes"
        );
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn extract_zip_skips_existing_top_level_without_overwrite() {
        // 非 ASCII 内容走 as_bytes(byte string 字面量不允许非 ASCII)
        let user_skill = "用户自己的技能".as_bytes();
        let dest = temp_dir("skip");
        std::fs::create_dir_all(dest.join("office-tools")).unwrap();
        std::fs::write(dest.join("office-tools").join("SKILL.md"), user_skill).unwrap();
        let zip = build_stored_zip(&[
            ("office-tools\\SKILL.md", "官方版本".as_bytes()),
            ("office-tools\\scripts\\x.py", b"x"),
            ("bid-write\\SKILL.md", "官方 bid".as_bytes()),
        ]);
        let added = extract_zip_to_dir(&zip, &dest).unwrap();
        assert_eq!(added, vec!["bid-write".to_string()]);
        assert_eq!(
            std::fs::read(dest.join("office-tools").join("SKILL.md")).unwrap(),
            user_skill
        );
        assert!(!dest.join("office-tools").join("scripts").join("x.py").exists());
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn extract_zip_is_idempotent() {
        let dest = temp_dir("idempotent");
        let zip = build_stored_zip(&[("bid-write\\SKILL.md", b"a")]);
        assert_eq!(extract_zip_to_dir(&zip, &dest).unwrap().len(), 1);
        assert!(extract_zip_to_dir(&zip, &dest).unwrap().is_empty());
        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn extract_zip_rejects_zip_slip_and_absolute_paths() {
        for bad in ["../evil.txt", "office-tools/../../evil.txt", "C:\\evil.txt", "/evil.txt"] {
            let zip = build_stored_zip(&[(bad, b"x")]);
            let dest = temp_dir("slip");
            let err = extract_zip_to_dir(&zip, &dest).unwrap_err();
            assert!(err.contains("已拒绝"), "条目 {bad} 应被拒绝,实际:{err}");
            assert!(!dest.join("evil.txt").exists());
            std::fs::remove_dir_all(&dest).ok();
        }
    }

    #[test]
    fn extract_zip_rejects_unsupported_compression_and_corrupt_crc() {
        // 把压缩方式改成 12(不支持)
        let mut zip = build_stored_zip(&[("bid-write\\SKILL.md", b"a")]);
        let cd_off = find_eocd(&zip).unwrap();
        let cd = le_u32(&zip, cd_off + 16).unwrap() as usize;
        zip[cd + 10..cd + 12].copy_from_slice(&12u16.to_le_bytes());
        let dest = temp_dir("badmethod");
        assert!(extract_zip_to_dir(&zip, &dest).unwrap_err().contains("压缩方式不支持"));

        // 篡改 CRC
        let mut zip2 = build_stored_zip(&[("bid-write\\SKILL.md", b"a")]);
        let cd2 = le_u32(&zip2, find_eocd(&zip2).unwrap() + 16).unwrap() as usize;
        zip2[cd2 + 16..cd2 + 20].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        let dest2 = temp_dir("badcrc");
        assert!(extract_zip_to_dir(&zip2, &dest2).unwrap_err().contains("CRC"));
        std::fs::remove_dir_all(&dest).ok();
        std::fs::remove_dir_all(&dest2).ok();
    }

    #[test]
    fn extract_zip_rejects_non_zip_bytes() {
        let dest = temp_dir("notzip");
        assert!(extract_zip_to_dir(b"definitely not a zip", &dest).is_err());
        std::fs::remove_dir_all(&dest).ok();
    }

    /// 内置技能包回归:解压到 tempdir,12 个技能目录全部新增且各含 SKILL.md。
    #[test]
    fn bundled_skills_extract_twelve() {
        let dest = temp_dir("bundled");
        let added = extract_bundled_skills(&dest).unwrap();
        assert_eq!(added.len(), 12, "内置包应安装 12 个技能目录,实际:{:?}", added);
        for name in &added {
            assert!(
                dest.join(name).join("SKILL.md").is_file(),
                "技能 {name} 缺少 SKILL.md"
            );
        }
        // 幂等:二次解压不新增也不覆盖
        assert!(extract_bundled_skills(&dest).unwrap().is_empty());
        std::fs::remove_dir_all(&dest).ok();
    }

    /// 真实官方技能包回归:解压 30 个技能目录并逐条目核对长度(CRC 已由
    /// read_entry_data 校验)。包不存在时跳过(其他机器上不会失败)。
    #[test]
    fn extract_official_package_if_present() {
        let path = std::env::var("QIDIWORK_SKILLS_PKG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../prod/releases/qidiwork-skills-v0.1.5.zip")
            });
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("跳过:未找到官方技能包 {}", path.display());
            return;
        };
        let entries = parse_entries(&bytes).unwrap();
        let expect_total: usize = entries
            .iter()
            .filter(|e| !(e.name.ends_with('/') || e.name.ends_with('\\')))
            .map(|e| e.uncompressed_size)
            .sum();

        let dest = temp_dir("official");
        let added = extract_zip_to_dir(&bytes, &dest).unwrap();
        assert_eq!(added.len(), 30, "官方包应安装 30 个技能目录");

        // 逐条目:解压结果必须与中央目录声明的原始长度完全一致
        let mut got_total = 0usize;
        for e in &entries {
            if e.name.ends_with('/') || e.name.ends_with('\\') {
                continue;
            }
            let rel = sanitize_name(&e.name).unwrap();
            let f = rel.iter().fold(dest.clone(), |acc, p| acc.join(p));
            let len = std::fs::metadata(&f).unwrap().len() as usize;
            assert_eq!(len, e.uncompressed_size, "条目长度不符:{}", e.name);
            got_total += len;
        }
        assert_eq!(got_total, expect_total);
        eprintln!(
            "官方技能包:{} 个条目,{} 字节解压完成,新增 {} 个技能目录",
            entries.len(),
            got_total,
            added.len()
        );
        std::fs::remove_dir_all(&dest).ok();
    }
}
