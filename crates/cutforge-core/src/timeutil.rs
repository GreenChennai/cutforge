// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! UTC RFC3339 时间格式化(纯 stdlib,不引 chrono;OpLog ts 字段用)。
//! 排序一律按 rev,ts 仅审计用(计划书 3.7)。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前时刻的 RFC3339(UTC,毫秒精度,`Z` 结尾)。
pub fn now_rfc3339() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format_unix_ms(d.as_millis() as u64)
}

/// Unix 毫秒 → RFC3339 UTC(Howard Hinnant civil-from-days 算法,不依赖平台时区)。
pub fn format_unix_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}


/// 当前 UTC 紧凑日期 `YYYYMMDD`(oplog 按天切分的文件名;全仓唯一日期算法出口)。
pub fn now_date_compact() -> String {
    now_rfc3339()[..10].replace('-', "")
}

/// 当前 UTC 紧凑日期时间 `YYYYMMDD-HHMMSS`(备份目录名;同上唯一出口)。
pub fn now_datetime_compact() -> String {
    let s = now_rfc3339();
    format!("{}-{}", s[..10].replace('-', ""), s[11..19].replace(':', ""))
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_epoch() {
        assert_eq!(format_unix_ms(0), "1970-01-01T00:00:00.000Z");
        // 2026-09-17T00:00:00Z = 1789603200
        assert_eq!(format_unix_ms(1_789_603_200_000), "2026-09-17T00:00:00.000Z");
        assert_eq!(format_unix_ms(1_789_603_200_123), "2026-09-17T00:00:00.123Z");
    }

    #[test]
    fn leap_day() {
        // 2024-02-29T12:00:00Z = 1709208000
        assert_eq!(format_unix_ms(1_709_208_000_000), "2024-02-29T12:00:00.000Z");
    }

    #[test]
    fn compact_variants() {
        assert_eq!(
            { let s = format_unix_ms(1_789_603_200_123); s[..10].replace('-', "") + "-" + &s[11..19].replace(':', "") },
            "20260917-000000"
        );
        let d = now_date_compact();
        assert_eq!(d.len(), 8);
        let dt = now_datetime_compact();
        assert_eq!(dt.len(), 15, "YYYYMMDD-HHMMSS");
    }

    #[test]
    fn now_is_parseable_shape() {
        let s = now_rfc3339();
        assert!(s.ends_with('Z') && s.len() == 24, "实际: {s}");
    }
}
