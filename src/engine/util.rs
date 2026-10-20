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
        if bytes % size == 0 {
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
