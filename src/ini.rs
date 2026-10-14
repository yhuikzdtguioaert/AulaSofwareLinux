//! Minimal INI reader for the vendor `KB.ini` / `Cfg.ini` files.
//!
//! The originals ship as UTF-16LE (with and without BOM) and occasionally as
//! GBK, so decoding is sniffed rather than assumed. Keys are case-insensitive,
//! `;` starts a comment, and duplicate keys keep the first occurrence — that is
//! what OemDrv.exe does.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct Ini {
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl Ini {
    pub fn parse(text: &str) -> Ini {
        let mut ini = Ini::default();
        let mut current = String::from("");
        for raw in text.lines() {
            let line = raw.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with("//") {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current = line[1..line.len() - 1].trim().to_ascii_uppercase();
                ini.sections.entry(current.clone()).or_default();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            ini.sections
                .entry(current.clone())
                .or_default()
                .entry(k.trim().to_ascii_uppercase())
                .or_insert_with(|| v.trim().to_string());
        }
        ini
    }

    pub fn load(path: &Path) -> std::io::Result<Ini> {
        Ok(Ini::parse(&decode(&std::fs::read(path)?)))
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(&section.to_ascii_uppercase())?
            .get(&key.to_ascii_uppercase())
            .map(String::as_str)
    }

    pub fn section(&self, section: &str) -> Option<&BTreeMap<String, String>> {
        self.sections.get(&section.to_ascii_uppercase())
    }

    pub fn has_section(&self, section: &str) -> bool {
        self.sections.contains_key(&section.to_ascii_uppercase())
    }

    pub fn str(&self, section: &str, key: &str) -> Option<String> {
        self.get(section, key).map(|s| s.to_string())
    }

    /// Numbers appear as `24`, `0x258a` and `0X010C` in the same file.
    pub fn num(&self, section: &str, key: &str) -> Option<i64> {
        parse_num(self.get(section, key)?)
    }

    pub fn nums(&self, section: &str, key: &str) -> Vec<i64> {
        self.get(section, key)
            .map(|v| v.split(',').filter_map(parse_num).collect())
            .unwrap_or_default()
    }

    /// `clrKeyOv = 255,255,255`
    pub fn rgb(&self, section: &str, key: &str) -> Option<[u8; 3]> {
        let v = self.nums(section, key);
        (v.len() >= 3).then(|| [v[0] as u8, v[1] as u8, v[2] as u8])
    }
}

pub fn parse_num(s: &str) -> Option<i64> {
    let s = s.trim();
    let s = s.split(';').next()?.trim();
    if s.is_empty() {
        return None;
    }
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()?
    } else {
        s.parse::<i64>().ok()?
    };
    Some(if neg { -v } else { v })
}

/// Hex without the `0x` prefix, as used by `Psd=3,0,0,0,0,cd`.
pub fn parse_hex_byte(s: &str) -> Option<u8> {
    u8::from_str_radix(s.trim(), 16).ok()
}

/// Sniff UTF-16LE / UTF-8 / GBK-ish and return UTF-8.
pub fn decode(raw: &[u8]) -> String {
    if raw.starts_with(&[0xff, 0xfe]) {
        return utf16le(&raw[2..]);
    }
    if raw.starts_with(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(&raw[3..]).into_owned();
    }
    // Unmarked UTF-16LE: every second byte of ASCII text is NUL.
    let odd_nul = raw.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    if raw.len() > 8 && odd_nul * 4 > raw.len() {
        return utf16le(raw);
    }
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        // Vendor comments are GBK; the data we need is ASCII, so lossy is fine.
        Err(_) => raw.iter().map(|b| *b as char).collect(),
    }
}

fn utf16le(raw: &[u8]) -> String {
    let units: Vec<u16> = raw
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_start_matches('\u{feff}')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sections_and_numbers() {
        let ini = Ini::parse(
            "[OPT]\nVID=0x258a\nPsd=3,0,0,0,0,cd\n;c\n[KEY]\nK1=27,33,53,61, 0x02,0x1B,0x00,0\n",
        );
        assert_eq!(ini.num("opt", "vid"), Some(0x258a));
        assert_eq!(ini.get("KEY", "K1").unwrap().split(',').count(), 8);
        assert!(ini.has_section("key"));
    }

    #[test]
    fn decodes_utf16() {
        let mut raw = vec![0xff, 0xfe];
        for b in "[OPT]\nFw=24\n".bytes() {
            raw.push(b);
            raw.push(0);
        }
        assert_eq!(Ini::parse(&decode(&raw)).num("OPT", "Fw"), Some(24));
    }
}
