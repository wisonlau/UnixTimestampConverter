//! Unix Timestamp Converter for OpenIDEA.
//!
//! 仿 JetBrains 插件 ramustha/unix-timestamp：在编辑器中把时间戳数字
//! 就地显示成可读日期（inlay hint），并提供插入当前时间戳命令。

mod timestamp;

use openidea_plugin::inlay_hints::{Document, InlayHint, InlayHints, InlayKind};
use openidea_plugin::{Error, Invocation, Plugin, register_plugin, text};
use std::time::{SystemTime, UNIX_EPOCH};

use timestamp::{find_timestamps, format, instant_nanos, is_plausible, parse_tz};

const DEFAULT_PATTERN: &str = "dd MMM yyyy HH:mm:ss";
const DEFAULT_OFFSET: i32 = 8 * 3600; // TZ 不可用时回退 UTC+8

#[derive(Default)]
pub struct UnixTimestamp;

/// 读取设置：优先插件 schema 的值，缺省时用默认值。
fn setting_string(key: &str, default: &str) -> String {
    openidea_plugin::host::setting_string(key).unwrap_or_else(|| default.to_string())
}

fn setting_bool(key: &str, default: bool) -> bool {
    openidea_plugin::host::setting_bool(key).unwrap_or(default)
}

fn current_offset() -> i32 {
    let configured = setting_string("timezone", "");
    parse_tz(&configured).unwrap_or(DEFAULT_OFFSET)
}

/// 可见范围内生成内嵌提示。
fn hints_in_range(doc: &Document, range: openidea_plugin::Range) -> Vec<InlayHint> {
    let start = match text::offset(&doc.text, range.start) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let end = match text::offset(&doc.text, range.end) {
        Some(v) => v,
        None => doc.text.len(),
    };
    if start > end || end > doc.text.len() {
        return Vec::new();
    }

    let micros = setting_bool("supportMicros", true);
    let nanos = setting_bool("supportNanos", true);
    let at_eol = setting_bool("placeAtEndOfLine", true);
    let pattern = setting_string("datePattern", DEFAULT_PATTERN);
    let offset = current_offset();

    let slice = &doc.text[start..end];
    let mut hints = Vec::new();

    for found in find_timestamps(slice, micros, nanos) {
        let nanos_value = match instant_nanos(&found.text) {
            Some(v) if is_plausible(v) => v,
            _ => continue,
        };
        let label = format(nanos_value, offset, &pattern);

        // 行尾模式：把提示放到该行的末尾（换行符处），否则放在数字结束处。
        // 必须从数字结束处往后在「整篇文档」里找换行，不能依赖可见范围 end：
        // IDE 传入的 range 边界可能落在时间戳所在行内，若以 end 为界就找不到
        // 换行符，提示会被推到 end（截图里表现为飞到文档末尾）。
        let digit_end = start + found.end;
        let absolute = if at_eol {
            doc.text[digit_end..]
                .find('\n')
                .map_or(doc.text.len(), |i| digit_end + i)
        } else {
            digit_end
        };

        hints.push(
            InlayHint::new(text::position(&doc.text, absolute), label)
                .kind(InlayKind::Other)
                .padding_left(),
        );
    }

    hints
}

impl InlayHints for UnixTimestamp {
    fn inlay_hints(&mut self, doc: &Document, range: openidea_plugin::Range) -> Vec<InlayHint> {
        hints_in_range(doc, range)
    }
}

impl Plugin for UnixTimestamp {
    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        _arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            "insert-current-timestamp" => {
                let editor = ctx.editor().ok_or_else(|| Error::Invalid("no focused editor".into()))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| Error::Invalid(format!("system clock error: {e}")))?;
                let stamp = now.as_millis().to_string();
                // 空选区即光标，替换后等价于插入；闭包要求返回 String
                editor.transform_selections(|_| stamp.clone())?;
                Ok(None)
            }
            _ => Err(Error::NotFound(format!("command {id}"))),
        }
    }
}

register_plugin!(UnixTimestamp, inlay_hints);

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::mock::MockHost;
    use openidea_plugin::{Position, Range};

    fn doc(text: &str) -> Document {
        Document {
            path: "/p/a.txt".into(),
            language: "plaintext".into(),
            version: 1,
            text: text.into(),
        }
    }

    fn full_range(text: &str) -> Range {
        Range {
            start: Position::new(0, 0),
            end: text::position(text, text.len()),
        }
    }

    #[test]
    fn hints_on_two_lines() {
        let host = MockHost::install();
        host.set_setting("timezone", "\"UTC\""); // mock 里设置值是 JSON
        let d = doc("let t = 1640970000;\nlet ms = 1671160204198;\n");
        let hints = hints_in_range(&d, full_range(&d.text));
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].label, "31 Dec 2021 17:00:00");
        assert_eq!(hints[1].label, "16 Dec 2022 03:10:04");
    }

    #[test]
    fn ip_address_produces_no_hints() {
        let _host = MockHost::install();
        let d = doc("\"ip\":\"192.168.5.222\" \"mac\":\"84:f3:eb:dd:27:00\"\n");
        assert!(hints_in_range(&d, full_range(&d.text)).is_empty());
    }

    #[test]
    fn visible_range_is_respected() {
        let _host = MockHost::install();
        let d = doc("1640970000\n1640970000\n");
        let r = Range {
            start: Position::new(1, 0),
            end: text::position(&d.text, d.text.len()),
        };
        assert_eq!(hints_in_range(&d, r).len(), 1);
    }

    #[test]
    fn eol_hint_stays_on_its_own_line_when_range_starts_later() {
        // 回归：行尾模式下曾把 start 叠加两次，导致提示飞到错误的行。
        let _host = MockHost::install();
        let text = "let a = 1;\nlet t = 1640970000;\nlet b = 2;\n";
        let d = doc(text);
        let line2_start = text.find("let t").unwrap();
        let r = Range {
            start: text::position(&d.text, line2_start),
            end: text::position(&d.text, d.text.len()),
        };
        let hints = hints_in_range(&d, r);
        assert_eq!(hints.len(), 1);
        // 提示应落在第二行分号之后，而不是被 start 推到第三行。
        let line2_end = text.find("1640970000;").unwrap() + "1640970000;".len();
        assert_eq!(hints[0].position, text::position(&d.text, line2_end));
    }

    #[test]
    fn eol_hint_ignores_range_end_when_line_extends_beyond_it() {
        // 回归：可见范围 end 落在时间戳所在行内（未含换行符）时，
        // 提示仍应落在该行真正的行尾，而不是被推到 range.end / 文档末尾。
        let _host = MockHost::install();
        let text = "prefix\nlet t = 1640970000; more visible code\nlet b = 2;\n";
        let d = doc(text);
        let line1 = text.lines().nth(1).unwrap();
        let stamp_end_col = line1.find("1640970000").unwrap() + "1640970000".len();
        let r = Range {
            start: Position::new(0, 0),
            // range.end 恰好在时间戳结束处，同一行内没有换行符
            end: Position::new(1, stamp_end_col as u32),
        };
        let hints = hints_in_range(&d, r);
        assert_eq!(hints.len(), 1);
        // 期望落在第 1 行真正行尾（换行符处），character 为该行长度
        assert_eq!(hints[0].position.line, 1);
        assert_eq!(hints[0].position.character, line1.len() as u32);
    }
}
