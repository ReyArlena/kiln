//! Small helpers shared across the engine.

use std::path::Path;

/// Read a sysfs or procfs file and trim it. `None` when unreadable.
pub fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Format a byte count as a short human string, e.g. `20G`, `512M`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("T", 1 << 40),
        ("G", 1 << 30),
        ("M", 1 << 20),
        ("K", 1 << 10),
    ];
    for (suffix, size) in UNITS {
        if bytes < size {
            continue;
        }
        if bytes.is_multiple_of(size) {
            return format!("{}{suffix}", bytes / size);
        }
        return format!("{:.1}{suffix}", bytes as f64 / size as f64);
    }
    format!("{bytes}")
}

/// Parse `20G`, `512M`, `1024`, `1.5G` into bytes. Suffixes are binary (1024-based).
pub fn parse_bytes(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (number, suffix) = match text.find(|c: char| c.is_ascii_alphabetic()) {
        Some(idx) => text.split_at(idx),
        None => (text, ""),
    };
    let value: f64 = number.trim().parse().ok()?;
    let multiplier: u64 = match suffix.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1 << 10,
        "M" | "MB" | "MIB" => 1 << 20,
        "G" | "GB" | "GIB" => 1 << 30,
        "T" | "TB" | "TIB" => 1 << 40,
        _ => return None,
    };
    if value < 0.0 {
        return None;
    }
    Some((value * multiplier as f64) as u64)
}

/// Parse `90`, `90s`, `5m`, `2h`, `1h30m` into seconds.
pub fn parse_duration_secs(text: &str) -> Option<u64> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }
    if let Ok(secs) = text.parse::<u64>() {
        return Some(secs);
    }
    let mut total = 0u64;
    let mut number = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let value: u64 = number.parse().ok()?;
        number.clear();
        total += match c {
            's' => value,
            'm' => value * 60,
            'h' => value * 3600,
            _ => return None,
        };
    }
    if !number.is_empty() {
        total += number.parse::<u64>().ok()?;
    }
    Some(total)
}

/// Whether a line of tool output reports an actual problem. Keywords such as
/// `error`, `fail` and `mismatch` count only when they are not negated or
/// zero: `mismatches=0`, `0 errors`, `no errors found` and `err=0` are fine.
pub fn looks_like_problem(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("kiln_metric") {
        return false;
    }
    ["fail", "error", "mismatch", "corrupt", "diff "]
        .iter()
        .any(|kw| keyword_is_bad(&lower, kw))
}

fn keyword_is_bad(lower: &str, keyword: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = lower[from..].find(keyword) {
        let at = from + pos;
        let before = lower[..at].trim_end();
        let after =
            lower[at + keyword.len()..].trim_start_matches(|c: char| c.is_ascii_alphabetic());
        // Collapse runs of spaces so `summary:    no errors` reads as `summary: no errors`.
        let after = after.split_whitespace().collect::<Vec<_>>().join(" ");
        let after = after.trim_start_matches([':', '=', ' ']);
        let count_is_zero =
            after.starts_with('0') && !after[1..].starts_with(|c: char| c.is_ascii_digit());
        let negated = before.ends_with("no") || before.ends_with(" 0") || before == "0";
        let benign = count_is_zero
            || negated
            || after.starts_with("none")
            || after.starts_with("summary: no");
        if !benign {
            return true;
        }
        from = at + keyword.len();
    }
    false
}

/// Format seconds as `1h02m03s`, `5m00s` or `45s`.
pub fn format_duration_secs(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_parse_and_format() {
        assert_eq!(parse_bytes("20G"), Some(20 << 30));
        assert_eq!(parse_bytes("512m"), Some(512 << 20));
        assert_eq!(
            parse_bytes("1.5G"),
            Some((1.5 * (1u64 << 30) as f64) as u64)
        );
        assert_eq!(parse_bytes("4096"), Some(4096));
        assert_eq!(parse_bytes("abc"), None);
        assert_eq!(format_bytes(20 << 30), "20G");
        assert_eq!(format_bytes(1536 << 20), "1.5G");
    }

    #[test]
    fn problem_detection_ignores_zero_and_negated_counts() {
        assert!(!looks_like_problem(
            "KILN_METRIC passes=1 mb_per_sec=2113 mismatches=0"
        ));
        assert!(!looks_like_problem("pass 2/20 ok, mismatches=0"));
        assert!(!looks_like_problem("Error summary:    no errors found"));
        assert!(!looks_like_problem("fio: err= 0"));
        assert!(!looks_like_problem("0 errors, 0 failures"));
        assert!(looks_like_problem(
            "stress-ng: fail: [1] vm: data mismatch at 0x1000"
        ));
        assert!(looks_like_problem("MISMATCH on pass 3: expected ab got cd"));
        assert!(looks_like_problem("Error summary:    csum=12"));
        assert!(looks_like_problem("DIFF ./lib/libc.so"));
    }

    #[test]
    fn durations_parse_and_format() {
        assert_eq!(parse_duration_secs("90"), Some(90));
        assert_eq!(parse_duration_secs("5m"), Some(300));
        assert_eq!(parse_duration_secs("1h30m"), Some(5400));
        assert_eq!(parse_duration_secs("2h5s"), Some(7205));
        assert_eq!(parse_duration_secs("x"), None);
        assert_eq!(format_duration_secs(5400), "1h30m00s");
        assert_eq!(format_duration_secs(300), "5m00s");
        assert_eq!(format_duration_secs(45), "45s");
    }
}
