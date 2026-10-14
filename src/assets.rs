//! Locating and loading the vendor artwork and UI strings.
//!
//! The `assets/` tree holds everything taken from the AULA installers:
//!
//! ```text
//! assets/skins/…            the original bitmaps, shared by every model
//! assets/text/en.xml        the original UI strings
//! assets/devices/<id>/      device.ini, keyimg.png, effect definitions
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ini::decode;

/// Where the asset tree lives, searched in the order a user would expect.
pub fn root() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("AULA_ASSETS") {
        candidates.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        // target/debug/aula -> repo root
        for up in [1usize, 2, 3] {
            let mut p = exe.clone();
            for _ in 0..up {
                p.pop();
            }
            candidates.push(p.join("assets"));
        }
    }
    candidates.push(PathBuf::from("assets"));
    candidates.push(PathBuf::from("/usr/share/aula/assets"));
    candidates.push(PathBuf::from("/usr/local/share/aula/assets"));

    candidates.into_iter().find(|p| p.join("skins").is_dir())
}

/// The vendor UI strings, keyed by their original tag (`tc_msg22`, …).
#[derive(Debug, Default, Clone)]
pub struct Strings {
    map: BTreeMap<String, String>,
}

impl Strings {
    /// The vendor files are flat `<tag>value</tag>` XML, so a scanner is enough
    /// and avoids dragging in an XML parser for 250 lines of text.
    pub fn parse(text: &str) -> Strings {
        let mut map = BTreeMap::new();
        let bytes: Vec<char> = text.chars().collect();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] != '<' {
                i += 1;
                continue;
            }
            let Some(close) = find(&bytes, i + 1, '>') else {
                break;
            };
            let tag: String = bytes[i + 1..close].iter().collect();
            if tag.starts_with('/') || tag.starts_with('?') || tag.starts_with('!') {
                i = close + 1;
                continue;
            }
            // Self-closing or a container: skip.
            if tag.ends_with('/') {
                i = close + 1;
                continue;
            }
            let end_marker: Vec<char> = format!("</{tag}>").chars().collect();
            let Some(end) = find_seq(&bytes, close + 1, &end_marker) else {
                i = close + 1;
                continue;
            };
            let value: String = bytes[close + 1..end].iter().collect();
            if !value.contains('<') {
                map.insert(tag.clone(), unescape(value.trim()));
            }
            i = close + 1;
        }
        Strings { map }
    }

    pub fn load(path: &Path) -> Strings {
        match std::fs::read(path) {
            Ok(raw) => Strings::parse(&decode(&raw)),
            Err(_) => Strings::default(),
        }
    }

    /// The string for `key`, falling back to `default` when the vendor file
    /// does not have it (some tags are empty in the shipped translations).
    pub fn get<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        match self.map.get(key) {
            Some(v) if !v.is_empty() => v.as_str(),
            _ => default,
        }
    }

    /// Name of hardware lighting effect `ui_index`, as shown by the original.
    pub fn effect_name(&self, ui_index: u8) -> String {
        let key = format!("tc_kb_led{ui_index}");
        match self.map.get(&key) {
            Some(v) if !v.is_empty() => v.replace('_', " "),
            _ => format!("Effect {ui_index}"),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

fn find(hay: &[char], from: usize, needle: char) -> Option<usize> {
    (from..hay.len()).find(|i| hay[*i] == needle)
}

fn find_seq(hay: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|i| &hay[*i..*i + needle.len()] == needle)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Decode a PNG into raw RGBA plus its size.
pub fn load_png(path: &Path) -> anyhow::Result<(egui::ColorImage, [usize; 2])> {
    let img = image::open(path)
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
        .to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let pixels = img.into_raw();
    Ok((
        egui::ColorImage::from_rgba_unmultiplied(size, &pixels),
        size,
    ))
}

/// Lazily-loaded skin bitmaps, addressed by their path under `assets/skins`.
pub struct Skins {
    dir: PathBuf,
    cache: BTreeMap<String, Option<egui::TextureHandle>>,
}

impl Skins {
    pub fn new(root: &Path) -> Skins {
        Skins {
            dir: root.join("skins"),
            cache: BTreeMap::new(),
        }
    }

    /// `get(ctx, "navi50x50/navi1_nr.png")`
    pub fn get(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        if let Some(hit) = self.cache.get(name) {
            return hit.clone();
        }
        let path = self.dir.join(name);
        let loaded = load_png(&path)
            .ok()
            .map(|(img, _)| ctx.load_texture(name, img, egui::TextureOptions::LINEAR));
        self.cache.insert(name.to_string(), loaded.clone());
        loaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="utf-16"?>
<root>
	<config>
		<tc_apply>Save</tc_apply>
		<tc_msg22>Brightness</tc_msg22>
		<tc_kb_led1>Fixed_on</tc_kb_led1>
		<tc_kb_led21></tc_kb_led21>
		<tc_web></tc_web>
		<tc_amp>a &amp; b</tc_amp>
	</config>
</root>"#;

    #[test]
    fn reads_flat_tags_and_skips_containers() {
        let s = Strings::parse(XML);
        assert_eq!(s.get("tc_apply", "?"), "Save");
        assert_eq!(s.get("tc_msg22", "?"), "Brightness");
        assert_eq!(s.get("root", "fallback"), "fallback");
        assert_eq!(s.get("config", "fallback"), "fallback");
    }

    #[test]
    fn empty_tags_fall_back_to_the_default() {
        let s = Strings::parse(XML);
        assert_eq!(s.get("tc_web", "Website"), "Website");
        assert_eq!(s.get("tc_missing", "Default"), "Default");
    }

    #[test]
    fn effect_names_drop_the_vendor_underscores() {
        let s = Strings::parse(XML);
        assert_eq!(s.effect_name(1), "Fixed on");
        assert_eq!(s.effect_name(21), "Effect 21", "blank tags fall back");
        assert_eq!(s.effect_name(99), "Effect 99");
    }

    #[test]
    fn unescapes_entities() {
        let s = Strings::parse(XML);
        assert_eq!(s.get("tc_amp", ""), "a & b");
    }
}
