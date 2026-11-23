//! 时间戳识别与日期格式化。
//!
//! 移植自 JetBrains 插件 ramustha/unix-timestamp 的 Helper.kt。
//! 不依赖外部 crate，避免 wasm32-wasip2 下的兼容性问题。

/// 文档中找到的一个时间戳（字节偏移）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// 日期时间各字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parts {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
    pub weekday: u32, // 0=Sunday
}

const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_LONG: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];
const DAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DAYS_LONG: [&str; 7] = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
];

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 在原插件支持的范围内查找时间戳。
///
/// 对应正则 `\b(\d{10,13}([lL])?|\d{16,19})(\.\d{1,9})?\b`，
/// 再按「10/13 位、或开启的微秒/纳秒、或带 l/L 后缀、或带小数」过滤。
pub fn find_timestamps(text: &str, support_micros: bool, support_nanos: bool) -> Vec<Found> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // 左单词边界：前一个字符不能是字母/数字/下划线
        if i > 0 && is_word(bytes[i - 1]) {
            while i < bytes.len() && is_word(bytes[i]) {
                i += 1;
            }
            continue;
        }

        let start = i;
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        let digits = j - start;

        // 可选小数部分，最多 9 位
        let mut frac_digits = 0usize;
        if j + 1 < bytes.len() && bytes[j] == b'.' && bytes[j + 1].is_ascii_digit() {
            j += 1; // 指向小数第一位
            let frac_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() && j - frac_start < 9 {
                j += 1;
            }
            frac_digits = j - frac_start;
        }

        // 可选的 Java long 后缀，仅整数形式允许
        let mut has_suffix = false;
        if frac_digits == 0
            && (10..=13).contains(&digits)
            && j < bytes.len()
            && (bytes[j] == b'l' || bytes[j] == b'L')
        {
            has_suffix = true;
            j += 1;
        }

        // 右单词边界
        if j < bytes.len() && is_word(bytes[j]) {
            i = j;
            continue;
        }

        // 整数部分必须是合法宽度；小数只是修饰，不能让 IP 地址这类
        // 任意位数 + 小数点的数字混入（否则 192.168 会被当成时间戳）。
        let keep = digits == 10
            || digits == 13
            || (digits == 16 && support_micros)
            || (digits == 19 && support_nanos)
            || has_suffix;

        if keep {
            out.push(Found {
                start,
                end: j,
                text: text[start..j].to_string(),
            });
        }
        i = j;
    }

    out
}

/// 把匹配文本换算为「纳秒级 Unix 时间戳」。
///
/// 对齐原插件 createInstantFormat：16/19 位先截断到毫秒，避免伪精度。
pub fn instant_nanos(raw: &str) -> Option<i128> {
    let s = raw
        .strip_suffix('l')
        .or_else(|| raw.strip_suffix('L'))
        .unwrap_or(raw);
    if s.is_empty() {
        return None;
    }

    if let Some(dot) = s.find('.') {
        let sec: i128 = s[..dot].parse().ok()?;
        let mut frac = s[dot + 1..].to_string();
        while frac.len() < 9 {
            frac.push('0');
        }
        let nano: i128 = frac.parse().ok()?;
        Some(sec * 1_000_000_000 + nano)
    } else {
        let n: i128 = s.parse().ok()?;
        match s.len() {
            19 => Some(n / 1_000_000 * 1_000_000_000),
            16 => Some(n / 1_000 * 1_000_000_000),
            13 => Some(n * 1_000_000),
            _ => Some(n * 1_000_000_000),
        }
    }
}

/// days since 1970-01-01 → (year, month, day)，Howard Hinnant 的 civil_from_days。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

/// 纳秒时间戳 + 固定时区偏移（秒）→ 各日期字段。
pub fn parts(nanos: i128, offset_secs: i32) -> Parts {
    let secs = nanos.div_euclid(1_000_000_000) + offset_secs as i128;
    let sub = nanos.rem_euclid(1_000_000_000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days as i64);

    Parts {
        year,
        month,
        day,
        hour: (sod / 3600) as u32,
        minute: (sod % 3600 / 60) as u32,
        second: (sod % 60) as u32,
        millis: (sub / 1_000_000) as u32,
        // 1970-01-01 是周四，0=Sunday
        weekday: ((days + 4).rem_euclid(7)) as u32,
    }
}

/// 合理性校验：时间戳换算出的年份必须落在合理区间，否则不显示提示。
///
/// Unix 时间戳从 1970 年起算，但 1970~1999 的秒级时间戳只有 9~10 位，
/// 本插件只识别 10/13/16/19 位，其中 10 位最小是 2001-09-09（1000000000）。
/// 因此把下限定在 2000 年，可滤掉 IP 地址片段（192.168 → 1970）这类误报；
/// 上限定在 2100 年，滤掉溢出的离谱数值。
pub fn is_plausible(nanos: i128) -> bool {
    let year = parts(nanos, 0).year;
    (2000..=2100).contains(&year)
}

/// 解析时区字符串：UTC/GMT/Z、固定偏移 +08:00、local（读 TZ 环境变量）。
/// 失败返回 None，调用方决定回退值。
pub fn parse_tz(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if matches!(s.to_ascii_uppercase().as_str(), "UTC" | "GMT" | "Z") {
        return Some(0);
    }
    if s.eq_ignore_ascii_case("local") {
        return std::env::var("TZ").ok().and_then(|v| parse_tz(&v));
    }
    let b = s.as_bytes();
    if (b[0] == b'+' || b[0] == b'-') && b.len() >= 6 && b[3] == b':' {
        let sign = if b[0] == b'-' { -1 } else { 1 };
        let h: i32 = s[1..3].parse().ok()?;
        let m: i32 = s[4..6].parse().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        return Some(sign * (h * 3600 + m * 60));
    }
    None
}

/// 按 SimpleDateFormat 风格格式化日期。
/// 支持 yyyy/yy、MMM/MM/M、dd/d、HH/H、hh/h、mm/m、ss/s、SSS、EEE/E、a，
/// 单引号内为字面量。
pub fn format(nanos: i128, offset_secs: i32, pattern: &str) -> String {
    let p = parts(nanos, offset_secs);
    let b = pattern.as_bytes();
    let mut out = String::new();
    let mut i = 0;

    while i < b.len() {
        // 引号字面量，'' 表示一个单引号
        if b[i] == b'\'' {
            if i + 1 < b.len() && b[i + 1] == b'\'' {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                out.push(b[i] as char);
                i += 1;
            }
            if i < b.len() {
                i += 1;
            }
            continue;
        }

        let c = b[i] as char;
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }

        let start = i;
        while i < b.len() && b[i] == b[start] {
            i += 1;
        }
        let n = i - start;
        match c {
            'y' => {
                if n >= 4 {
                    out.push_str(&format!("{:04}", p.year));
                } else {
                    out.push_str(&format!("{:02}", p.year % 100));
                }
            }
            'M' => match n {
                1 => out.push_str(&p.month.to_string()),
                2 => out.push_str(&format!("{:02}", p.month)),
                3 => out.push_str(MONTHS_SHORT[(p.month - 1) as usize]),
                _ => out.push_str(MONTHS_LONG[(p.month - 1) as usize]),
            },
            'd' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", p.day));
                } else {
                    out.push_str(&p.day.to_string());
                }
            }
            'H' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", p.hour));
                } else {
                    out.push_str(&p.hour.to_string());
                }
            }
            'h' => {
                let h12 = match p.hour % 12 {
                    0 => 12,
                    v => v,
                };
                if n >= 2 {
                    out.push_str(&format!("{:02}", h12));
                } else {
                    out.push_str(&h12.to_string());
                }
            }
            'm' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", p.minute));
                } else {
                    out.push_str(&p.minute.to_string());
                }
            }
            's' => {
                if n >= 2 {
                    out.push_str(&format!("{:02}", p.second));
                } else {
                    out.push_str(&p.second.to_string());
                }
            }
            'S' => {
                if n >= 3 {
                    out.push_str(&format!("{:03}", p.millis));
                } else if n == 2 {
                    out.push_str(&format!("{:02}", p.millis / 10));
                } else {
                    out.push_str(&(p.millis / 100).to_string());
                }
            }
            'E' => {
                if n >= 4 {
                    out.push_str(DAYS_LONG[p.weekday as usize]);
                } else {
                    out.push_str(DAYS_SHORT[p.weekday as usize]);
                }
            }
            'a' => out.push_str(if p.hour < 12 { "AM" } else { "PM" }),
            _ => out.push_str(&pattern[start..i]),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_widths() {
        let found = find_timestamps("a=1640970000 b=1671160204198 c=1671160204198000", true, true);
        let texts: Vec<&str> = found.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts, vec!["1640970000", "1671160204198", "1671160204198000"]);
    }

    #[test]
    fn respects_micro_nano_switches() {
        let text = "1640970000000000 1640970000000000000";
        assert_eq!(find_timestamps(text, false, false).len(), 0);
        assert_eq!(find_timestamps(text, true, false).len(), 1);
        assert_eq!(find_timestamps(text, true, true).len(), 2);
    }

    #[test]
    fn accepts_suffix_and_decimal() {
        let found = find_timestamps("x=1640970000L y=1640970000.123 z=12345678901", true, true);
        let texts: Vec<&str> = found.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(texts, vec!["1640970000L", "1640970000.123"]);
    }

    #[test]
    fn rejects_ip_addresses() {
        // IP / 版本号含小数点但整数位不足 10，不应被当成时间戳（原 bug）
        let text = "ip=\"192.168.5.222\" v=1.2.3 port=8080";
        assert!(find_timestamps(text, true, true).is_empty());
    }

    #[test]
    fn plausibility_filters_1970_and_far_future() {
        // 1970 附近的秒数（IP 片段换算结果）与离谱大数都要滤掉
        assert!(!is_plausible(instant_nanos("12").unwrap()));
        assert!(!is_plausible(instant_nanos("5").unwrap()));
        assert!(!is_plausible(instant_nanos("9999999999999999999").unwrap()));
        // 正常时间戳保留
        assert!(is_plausible(instant_nanos("1640970000").unwrap()));
        assert!(is_plausible(instant_nanos("1671160204198").unwrap()));
    }

    #[test]
    fn rejects_word_boundary_violations() {
        assert!(find_timestamps("a1640970000", true, true).is_empty());
        assert!(find_timestamps("1640970000abc", true, true).is_empty());
        assert!(find_timestamps("x_1640970000", true, true).is_empty());
    }

    #[test]
    fn converts_instant_with_offset() {
        // 1640970000 = 2021-12-31 17:00:00 UTC
        let nanos = instant_nanos("1640970000").unwrap();
        let p = parts(nanos, 0);
        assert_eq!((p.year, p.month, p.day, p.hour, p.minute, p.second), (2021, 12, 31, 17, 0, 0));
        assert_eq!(p.weekday, 5); // Friday

        let p8 = parts(nanos, 8 * 3600);
        assert_eq!((p8.year, p8.month, p8.day, p8.hour), (2022, 1, 1, 1));
    }

    #[test]
    fn formats_default_pattern() {
        let nanos = instant_nanos("1640970000").unwrap();
        assert_eq!(format(nanos, 0, "dd MMM yyyy HH:mm:ss"), "31 Dec 2021 17:00:00");
        assert_eq!(format(nanos, 8 * 3600, "yyyy-MM-dd HH:mm:ss EEE"), "2022-01-01 01:00:00 Sat");
        assert_eq!(format(nanos, 0, "yyyy/MM/dd hh:mm:ss a"), "2021/12/31 05:00:00 PM");
    }

    #[test]
    fn parses_timezone() {
        assert_eq!(parse_tz("UTC"), Some(0));
        assert_eq!(parse_tz("+08:00"), Some(8 * 3600));
        assert_eq!(parse_tz("-05:30"), Some(-5 * 3600 - 30 * 60));
        assert_eq!(parse_tz("Asia/Shanghai"), None);
    }

    #[test]
    fn drops_long_suffix() {
        assert_eq!(instant_nanos("1640970000L"), instant_nanos("1640970000"));
    }
}
