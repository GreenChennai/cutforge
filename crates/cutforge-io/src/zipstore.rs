// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 零依赖 zip store 形读写(册七 T7.6 自 cutforge-cli bundle.rs 提取为单一实现):
//! 写 = method 0 不压缩 + UTF-8 文件名 + CRC-32(诊断包/.cfpkg 共用,KB..MB 级
//! 文本与小文件为主,压缩收益趋零);读 = 中央目录解析 + store 形取数据(只认
//! 本仓写出的形态;deflate 条目显式拒绝,不假装能解)。新增 Cargo 依赖违反依赖
//! 纪律(ADR-0009 同源),故手写。落盘一律由调用方走 `atomic::atomic_write`。

/// 一个待打包条目(全内存;诊断包与 .cfpkg 体量不做流式)。
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

/// CRC-32(IEEE 802.3,zip 规范多项式 0xEDB8_8320;逐位无表实现——包小,速度无关紧要)。
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

/// UNIX 秒 → DOS 时间(zip 必填字段;UTC 口径,不承诺本地时区)。
pub(crate) fn dos_datetime(secs: u64) -> (u16, u16) {
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

// ---------------- 读取(册七 T7.6:.cfpkg 解包;只认本仓写出的 store 形) ----------------

/// 解包错误(store 形之外/结构损坏/CRC 不符/越界读取)。
#[derive(Debug)]
pub struct ZipError(pub String);

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 一个已解析的 zip 条目(名 + 原始字节)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub data: Vec<u8>,
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// 解析 zip(store 形)为条目清单:尾部扫 EOCD → 中央目录逐项 → 按本地头定位取数。
/// 防 zip-slip 由**调用方**负责(本函数只如实给出条目名);CRC 逐条目复验。
pub fn zip_read_all(buf: &[u8]) -> Result<Vec<ZipEntry>, ZipError> {
    // EOCD:从尾部向前扫签名(注释长度字段我们写出恒 0,但容错外部工具加的注释)
    let eocd_sig = 0x0605_4b50u32;
    let scan_start = buf.len().saturating_sub(22 + 65_536);
    let mut eocd_off = None;
    let mut i = buf.len();
    while i > scan_start {
        i -= 1;
        if u32_at(buf, i) == Some(eocd_sig) {
            eocd_off = Some(i);
            break;
        }
    }
    let Some(eo) = eocd_off else {
        return Err(ZipError("EOCD 签名未找到(不是 zip 或已截断)".into()));
    };
    let count = u16_at(buf, eo + 10).ok_or_else(|| ZipError("EOCD 截断".into()))? as usize;
    let cd_size = u32_at(buf, eo + 12).ok_or_else(|| ZipError("EOCD 截断".into()))? as usize;
    let cd_off = u32_at(buf, eo + 16).ok_or_else(|| ZipError("EOCD 截断".into()))? as usize;
    if cd_off + cd_size > buf.len() {
        return Err(ZipError("中央目录越界(结构损坏)".into()));
    }
    let mut out = Vec::with_capacity(count);
    let mut p = cd_off;
    for _ in 0..count {
        // 中央目录项:PK\x01\x02
        if u32_at(buf, p) != Some(0x0201_4b50) {
            return Err(ZipError(format!("中央目录项签名不符 @{p}")));
        }
        let crc = u32_at(buf, p + 16).ok_or_else(|| ZipError("中央目录截断".into()))?;
        let size = u32_at(buf, p + 24).ok_or_else(|| ZipError("中央目录截断".into()))? as usize;
        let name_len = u16_at(buf, p + 28).ok_or_else(|| ZipError("中央目录截断".into()))? as usize;
        let extra_len =
            u16_at(buf, p + 30).ok_or_else(|| ZipError("中央目录截断".into()))? as usize;
        let comment_len =
            u16_at(buf, p + 32).ok_or_else(|| ZipError("中央目录截断".into()))? as usize;
        let lho = u32_at(buf, p + 42).ok_or_else(|| ZipError("中央目录截断".into()))? as usize;
        let name = String::from_utf8_lossy(
            buf.get(p + 46..p + 46 + name_len)
                .ok_or_else(|| ZipError("中央目录条目名越界".into()))?,
        )
        .into_owned();
        // 本地文件头:PK\x03\x04;名字/extra 长度以本地头为准(与中央目录应一致)
        if u32_at(buf, lho) != Some(0x0403_4b50) {
            return Err(ZipError(format!("本地头签名不符 @{lho}({name})")));
        }
        let method = u16_at(buf, lho + 8).ok_or_else(|| ZipError("本地头截断".into()))?;
        if method != 0 {
            return Err(ZipError(format!(
                "条目 {name} 压缩方法 {method} 非 store:本仓只写 store 形,deflate 拒绝(不假装能解)"
            )));
        }
        let ln_len = u16_at(buf, lho + 26).ok_or_else(|| ZipError("本地头截断".into()))? as usize;
        let ln_extra = u16_at(buf, lho + 28).ok_or_else(|| ZipError("本地头截断".into()))? as usize;
        let data_off = lho + 30 + ln_len + ln_extra;
        let data = buf
            .get(data_off..data_off + size)
            .ok_or_else(|| ZipError(format!("条目 {name} 数据越界(结构损坏)")))?
            .to_vec();
        if crc32(&data) != crc {
            return Err(ZipError(format!("条目 {name} CRC-32 不符(数据损坏)")));
        }
        out.push(ZipEntry { name, data });
        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
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
            Entry {
                name: "doctor.json".into(),
                data: b"{\"ok\":true}".to_vec(),
            },
            Entry {
                name: "logs/session.json".into(),
                data: "会话中文内容".as_bytes().to_vec(),
            },
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
        assert_eq!(
            &zip[cd_off as usize..cd_off as usize + 4],
            &[0x50, 0x4B, 0x01, 0x02]
        );
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

    /// 读取器:写出 → 读回逐条目相等(中文名/嵌套路径);截断/CRC 损坏如实报错。
    #[test]
    fn zip_read_all_roundtrip_and_tamper() {
        let entries = vec![
            Entry {
                name: "manifest.json".into(),
                data: b"{\"format\":\"cfpkg\"}".to_vec(),
            },
            Entry {
                name: "project/project.json".into(),
                data: "中文工程".as_bytes().to_vec(),
            },
            Entry {
                name: "media/素材/a.mp4".into(),
                data: vec![0u8; 1024],
            },
        ];
        let zip = zip_store(&entries, 1_790_726_400);
        let back = zip_read_all(&zip).unwrap();
        assert_eq!(back.len(), 3);
        for (e, b) in entries.iter().zip(&back) {
            assert_eq!(b.name, e.name);
            assert_eq!(b.data, e.data);
        }
        // 非 zip 输入 → EOCD 报错
        assert!(zip_read_all(b"not a zip").is_err());
        // 数据位翻转 → CRC 复验红(翻转位置取首个数据字节,避开结构字段)
        let mut tampered = zip.clone();
        let first_data = 30 + entries[0].name.len();
        tampered[first_data] ^= 0xFF;
        assert!(zip_read_all(&tampered).is_err(), "CRC 损坏必须拒绝");
    }
}
