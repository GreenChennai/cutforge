// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! `doctor --bundle`(册六 T6.4):诊断包 = 单文件 zip(doctor.json + environment.txt
//! + 会话/摘要日志 + rev 快照),给「端口起不来/导出失败」一类问题的一键取证面。
//!
//! zip 写入为**零依赖手写 store 形**(method 0 不压缩,UTF-8 文件名,CRC-32 校验)——
//! 新增 Cargo 依赖违反依赖纪律,而诊断包的体量(文本为主,KB 级)使压缩收益趋零。
//! 落盘走 `cutforge_io::atomic::atomic_write` 唯一落盘点(whole-bytes 一次成包)。

use std::fmt::Write as _;
use std::path::Path;

/// 一个待打包条目(全内存;诊断包体量 KB 级,不做流式)。
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

/// CRC-32(IEEE 802.3,zip 规范多项式 0xEDB88320;逐位无表实现——包小,速度无关紧要)。
pub fn crc32(data: &[u8]) -> u32 {
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

/// UNIX 秒 → DOS 时间(zip 必填字段;UTC 口径,诊断包不承诺本地时区)。
fn dos_datetime(secs: u64) -> (u16, u16) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // 民用日算法(Howard Hatcher/Gregorian):1980-01-01 为 DOS 纪元
    let z = days + 719_468; // 1970-01-01 → 719468 天到民用纪元
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    let year = y.clamp(1980, 2107) as u16;
    let month = mth.clamp(1, 12) as u16;
    (
        ((h as u16) << 11) | ((m as u16) << 5) | s as u16,
        (((year - 1980) << 9) | (month << 5) | d as u16),
    )
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// 打包为 zip(store 形;返回完整字节)。文件名走 UTF-8 旗标(0x0800),不做双名册。
pub fn zip_store(entries: &[Entry], unix_secs: u64) -> Vec<u8> {
    let (dostime, dosdate) = dos_datetime(unix_secs);
    let mut out = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for e in entries {
        let crc = crc32(&e.data);
        let offset = out.len() as u32;
        let name = e.name.as_bytes();
        // 本地文件头:PK\x03\x04
        put32(&mut out, 0x0403_4b50);
        put16(&mut out, 20); // version needed
        put16(&mut out, 0x0800); // UTF-8 文件名
        put16(&mut out, 0); // method = store
        put16(&mut out, dostime);
        put16(&mut out, dosdate);
        put32(&mut out, crc);
        put32(&mut out, e.data.len() as u32);
        put32(&mut out, e.data.len() as u32);
        put16(&mut out, name.len() as u16);
        put16(&mut out, 0); // extra len
        out.extend_from_slice(name);
        out.extend_from_slice(&e.data);
        // 中央目录项:PK\x01\x02
        put32(&mut central, 0x0201_4b50);
        put16(&mut central, 20); // version made by
        put16(&mut central, 20); // version needed
        put16(&mut central, 0x0800);
        put16(&mut central, 0);
        put16(&mut central, dostime);
        put16(&mut central, dosdate);
        put32(&mut central, crc);
        put32(&mut central, e.data.len() as u32);
        put32(&mut central, e.data.len() as u32);
        put16(&mut central, name.len() as u16);
        put16(&mut central, 0); // extra
        put16(&mut central, 0); // comment
        put16(&mut central, 0); // disk number
        put16(&mut central, 0); // internal attrs
        put32(&mut central, 0); // external attrs
        put32(&mut central, offset);
        central.extend_from_slice(name);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    // EOCD:PK\x05\x06
    put32(&mut out, 0x0605_4b50);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, entries.len() as u16);
    put16(&mut out, entries.len() as u16);
    put32(&mut out, cd_size);
    put32(&mut out, cd_offset);
    put16(&mut out, 0);
    out
}

/// 打包并原子落盘(唯一落盘点纪律;返回字节数)。
pub fn write_bundle(out_path: &Path, entries: Vec<Entry>) -> Result<usize, String> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let zip = zip_store(&entries, secs);
    let n = zip.len();
    cutforge_io::atomic::atomic_write(out_path, &zip)
        .map_err(|e| format!("诊断包写入失败({}): {e}", out_path.display()))?;
    Ok(n)
}

/// 环境信息(人类可读;诊断包第 2 件):系统/架构/依赖版本/CUTFORGE_* env 面。
pub fn environment_text(root: &Path) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "CutForge 诊断包 · 环境信息");
    let _ = writeln!(s, "generatedAt: {}", cutforge_core::timeutil::now_rfc3339());
    let _ = writeln!(s, "root: {}", root.display());
    if let Ok(exe) = std::env::current_exe() {
        let _ = writeln!(s, "exe: {}", exe.display());
    }
    let _ = writeln!(s, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(s, "family: {}", std::env::consts::FAMILY);
    for (label, bin, key) in [
        ("ffmpeg", "ffmpeg", "CUTFORGE_FFMPEG"),
        ("ffprobe", "ffprobe", "CUTFORGE_FFPROBE"),
    ] {
        let via_env = std::env::var_os(key).is_some_and(|v| !v.is_empty());
        let version = std::process::Command::new(bin).arg("-version").output().ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string())
            .unwrap_or_else(|| "不可用".into());
        let _ = writeln!(s, "{label}: {version}(env {key}: {via_env})");
    }
    let _ = writeln!(s, "── 相关环境变量(有则记值,无则记缺) ──");
    for key in ["CUTFORGE_FFMPEG", "CUTFORGE_FFPROBE", "CUTFORGE_WEB", "CUTFORGE_PROJECTS",
                "CUTFORGE_MEDIA", "CUTFORGE_RENDER", "CUTFORGE_SNAPSHOT_INTERVAL_MS"] {
        match std::env::var_os(key) {
            Some(v) => {
                let _ = writeln!(s, "{key} = {}", v.to_string_lossy());
            }
            None => {
                let _ = writeln!(s, "{key} = (未设)");
            }
        }
    }
    // 外部管线变量只记**在位性**不记值(诊断包可能外发,别把机器路径带出去)
    for key in ["CUTFLOW_REPO", "CUTFLOW_CONFIG"] {
        let _ = writeln!(s, "{key} = {}", if std::env::var_os(key).is_some() { "(在位,值略)" } else { "(未设)" });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CRC-32 已知向量:IEEE 检验串 "123456789" → 0xCBF43926。
    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// DOS 时间:纪元内三个抽样点(含 1980 下限钳制)。
    #[test]
    fn dos_datetime_samples() {
        // 2026-09-30T00:00:00Z = 1790726400
        let (t, d) = dos_datetime(1_790_726_400);
        assert_eq!((d >> 9) + 1980, 2026);
        assert_eq!((d >> 5) & 0xF, 9);
        assert_eq!(d & 0x1F, 30);
        assert_eq!(t >> 11, 0);
        // 1970(早于 DOS 纪元)→ 年份钳 1980,不回绕
        let (_, d0) = dos_datetime(0);
        assert_eq!((d0 >> 9) + 1980, 1980);
    }

    /// zip 结构:签名/条目数/EOCD 可逐字段回读;内容 CRC 自洽(自证,不依赖外部 unzip)。
    #[test]
    fn zip_store_roundtrip_structure() {
        let entries = vec![
            Entry { name: "doctor.json".into(), data: b"{\"ok\":true}".to_vec() },
            Entry { name: "logs/session.json".into(), data: "会话中文内容".as_bytes().to_vec() },
        ];
        let zip = zip_store(&entries, 1_790_745_600);
        assert_eq!(&zip[0..4], &[0x50, 0x4B, 0x03, 0x04], "本地头签名");
        // EOCD 在尾部 22 字节(无注释)
        let eocd = &zip[zip.len() - 22..];
        assert_eq!(&eocd[0..4], &[0x50, 0x4B, 0x05, 0x06], "EOCD 签名");
        let n = u16::from_le_bytes([eocd[10], eocd[11]]);
        assert_eq!(n, 2, "条目数");
        let cd_size = u32::from_le_bytes(eocd[12..16].try_into().unwrap());
        let cd_off = u32::from_le_bytes(eocd[16..20].try_into().unwrap());
        assert_eq!(cd_size + cd_off, zip.len() as u32 - 22, "中央目录紧贴 EOCD");
        // 中央目录签名在位
        assert_eq!(&zip[cd_off as usize..cd_off as usize + 4], &[0x50, 0x4B, 0x01, 0x02]);
        // 逐条目:本地头里的 CRC 与重算一致;store 形两长度相等
        let mut pos = 0usize;
        for e in &entries {
            assert_eq!(&zip[pos..pos + 4], &[0x50, 0x4B, 0x03, 0x04]);
            let method = u16::from_le_bytes(zip[pos + 8..pos + 10].try_into().unwrap());
            assert_eq!(method, 0, "store 形");
            let crc = u32::from_le_bytes(zip[pos + 14..pos + 18].try_into().unwrap());
            let sz = u32::from_le_bytes(zip[pos + 18..pos + 22].try_into().unwrap()) as usize;
            let nlen = u16::from_le_bytes(zip[pos + 26..pos + 28].try_into().unwrap()) as usize;
            let name = &zip[pos + 30..pos + 30 + nlen];
            assert_eq!(name, e.name.as_bytes());
            let data = &zip[pos + 30 + nlen..pos + 30 + nlen + sz];
            assert_eq!(crc, crc32(data));
            assert_eq!(data, e.data.as_slice());
            pos = pos + 30 + nlen + sz;
        }
    }
}
