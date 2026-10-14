//! The lighting / settings page (command 0x04).
//!
//! 128 bytes, ending in a `5A A5` marker. The interesting part is a table of
//! per-effect settings: the keyboard remembers brightness, speed and colour
//! separately for every lighting effect, and a single byte near the front says
//! which effect is live.
//!
//! Layout confirmed against an F75 (`aula dump`):
//!
//! ```text
//! 0000  00 03 03 01 00 00 04 04 07 00 01 20 01 00 00 00
//! 0010  00 00 02 00 02 01 00 ff 0a 00 00 00 01 00 03 01
//! 0020  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
//! 0030  00 00 00 00 00 00 00 00 ff ff 09 40 09 47 09 47
//! ...                                    ^^^^^ ^^^^^ per-effect settings
//! 0070  07 44 07 44 04 09 04 04 04 04 04 04 04 04 5a a5
//! ```
//!
//! Nothing here builds a page from nothing. The page is always read from the
//! keyboard first, then the fields the user touched are patched and the page is
//! written back — so bytes this code does not understand survive a round trip
//! untouched.

use crate::ini::Ini;
use std::path::Path;

/// Size of the settings page, fixed by the `5A A5` marker at 0x7e..0x7f.
pub const PAGE_LEN: usize = 128;

/// Byte offsets inside the settings page.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Which lighting effect is live, as its 1-based position in the profile's
    /// effect list (the first field of `LedOptN=`, i.e. `ui_index`).
    ///
    /// Confirmed: reads back 0x01 on an F75 set to "Fixed on", the first entry.
    pub effect: usize,
    /// Idle timeout, in the units of `SleepHW=`.
    ///
    /// Confirmed: reads back 0x0a on an F75, and `SleepTime=300` maps to 10.
    pub sleep: usize,
    /// First byte of the per-effect settings table.
    ///
    /// Confirmed: the table starts right after the `FF FF` at 0x38..0x39 and
    /// runs to the end marker, which is 34 slots of [`EFFECT_STRIDE`] bytes.
    /// Slots are addressed by `ui_index - 1`, *not* by the hardware effect
    /// number — pressing Fn+Tab on an F75 showing the first effect moves the
    /// byte at 0x3b, which is slot 0.
    pub effect_table: usize,
    /// Key debounce time. **Unconfirmed** — derived from the Windows driver's
    /// own log line, not yet checked on hardware.
    pub debounce: usize,
}

impl Default for Layout {
    fn default() -> Layout {
        Layout {
            effect: 0x0a,
            sleep: 0x18,
            effect_table: 0x3a,
            debounce: 0x02,
        }
    }
}

/// Bytes per entry in the per-effect settings table.
pub const EFFECT_STRIDE: usize = 2;
/// Offset of the brightness byte within an entry.
const ENTRY_BRIGHTNESS: usize = 0;
/// Offset of the packed speed/colour byte within an entry.
const ENTRY_SPEED_COLOR: usize = 1;

impl Layout {
    /// How many effect slots fit between the table and the end marker.
    pub fn effect_slots(&self) -> usize {
        (PAGE_LEN - 2).saturating_sub(self.effect_table) / EFFECT_STRIDE
    }

    /// Read a per-model override, if the profile ships one.
    ///
    /// ```ini
    /// [layout]
    /// effect = 0x0a
    /// sleep = 0x18
    /// effect_table = 0x3a
    /// ```
    pub fn load_override(dir: &Path) -> Layout {
        let mut l = Layout::default();
        let path = dir.join("layout.ini");
        let Ok(ini) = Ini::load(&path) else { return l };
        let set = |field: &mut usize, key: &str| {
            if let Some(v) = ini.num("layout", key) {
                if (0..PAGE_LEN as i64).contains(&v) {
                    *field = v as usize;
                }
            }
        };
        set(&mut l.effect, "effect");
        set(&mut l.sleep, "sleep");
        set(&mut l.effect_table, "effect_table");
        set(&mut l.debounce, "debounce");
        l
    }
}

/// The fixed colour slots the firmware understands, in firmware order.
///
/// The driver stores them as Windows `COLORREF` (0x00BBGGRR) and maps them onto
/// these indices; index 7 asks the firmware to cycle colours itself.
pub const COLORS: [(&str, [u8; 3]); 7] = [
    ("Red", [255, 0, 0]),
    ("Green", [0, 255, 0]),
    ("Blue", [0, 0, 255]),
    ("Yellow", [255, 255, 0]),
    ("Magenta", [255, 0, 255]),
    ("Cyan", [0, 255, 255]),
    ("White", [255, 255, 255]),
];

/// Value of the colour field that means "let the firmware pick".
pub const COLOR_RANDOM: u8 = 7;

/// Nearest firmware colour slot for an arbitrary RGB triple.
pub fn nearest_color(rgb: [u8; 3]) -> u8 {
    let mut best = 0u8;
    let mut best_d = i32::MAX;
    for (i, (_, c)) in COLORS.iter().enumerate() {
        let d: i32 = (0..3)
            .map(|k| {
                let d = rgb[k] as i32 - c[k] as i32;
                d * d
            })
            .sum();
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
    }
    best
}

/// Brightness, speed and colour as stored for one effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectSettings {
    pub brightness: u8,
    /// High nibble of the packed byte; indexes `SpeedHW=`.
    pub speed: u8,
    /// Low nibble of the packed byte; see [`COLORS`], 7 means "colourful".
    pub color: u8,
}

/// A settings page read from the device, ready to be patched and written back.
#[derive(Debug, Clone)]
pub struct ConfigPage {
    pub bytes: Vec<u8>,
    pub layout: Layout,
    /// Set once any field has been changed since the page was read.
    pub dirty: bool,
}

impl ConfigPage {
    pub fn new(bytes: Vec<u8>, layout: Layout) -> ConfigPage {
        let mut bytes = bytes;
        bytes.resize(PAGE_LEN, 0);
        ConfigPage {
            bytes,
            layout,
            dirty: false,
        }
    }

    /// A blank page, for when the device cannot be read (no permission yet).
    pub fn blank(layout: Layout) -> ConfigPage {
        ConfigPage {
            bytes: vec![0; PAGE_LEN],
            layout,
            dirty: false,
        }
    }

    /// Does this look like a page the keyboard actually sent?
    pub fn looks_valid(&self) -> bool {
        self.bytes.len() == PAGE_LEN
            && self.bytes[PAGE_LEN - 2] == 0x5a
            && self.bytes[PAGE_LEN - 1] == 0xa5
    }

    fn get(&self, at: usize) -> u8 {
        self.bytes.get(at).copied().unwrap_or(0)
    }

    fn set(&mut self, at: usize, v: u8) {
        if at < self.bytes.len() && self.bytes[at] != v {
            self.bytes[at] = v;
            self.dirty = true;
        }
    }

    /// Hardware number of the live effect.
    pub fn effect(&self) -> u8 {
        self.get(self.layout.effect)
    }

    pub fn set_effect(&mut self, v: u8) {
        self.set(self.layout.effect, v);
    }

    pub fn sleep(&self) -> u8 {
        self.get(self.layout.sleep)
    }

    pub fn set_sleep(&mut self, v: u8) {
        self.set(self.layout.sleep, v);
    }

    pub fn debounce(&self) -> u8 {
        self.get(self.layout.debounce)
    }

    pub fn set_debounce(&mut self, v: u8) {
        self.set(self.layout.debounce, v);
    }

    /// Table slot for the effect listed at `ui_index` in the device profile.
    ///
    /// The list is 1-based in `LedOptN=`, the table is 0-based.
    pub fn slot_of(ui_index: u8) -> u8 {
        ui_index.saturating_sub(1)
    }

    /// Byte offset of settings slot `slot`, if it fits before the end marker.
    fn entry(&self, slot: u8) -> Option<usize> {
        let at = self.layout.effect_table + slot as usize * EFFECT_STRIDE;
        (at + EFFECT_STRIDE <= PAGE_LEN - 2).then_some(at)
    }

    /// Settings stored in one slot.
    pub fn slot_settings(&self, slot: u8) -> EffectSettings {
        let Some(at) = self.entry(slot) else {
            return EffectSettings::default();
        };
        let packed = self.get(at + ENTRY_SPEED_COLOR);
        EffectSettings {
            brightness: self.get(at + ENTRY_BRIGHTNESS),
            speed: packed >> 4,
            color: packed & 0x0f,
        }
    }

    /// Settings for whichever effect is live.
    pub fn current(&self) -> EffectSettings {
        self.slot_settings(Self::slot_of(self.effect()))
    }

    pub fn set_brightness(&mut self, slot: u8, v: u8) {
        if let Some(at) = self.entry(slot) {
            self.set(at + ENTRY_BRIGHTNESS, v);
        }
    }

    pub fn set_speed(&mut self, slot: u8, v: u8) {
        if let Some(at) = self.entry(slot) {
            let packed = self.get(at + ENTRY_SPEED_COLOR);
            self.set(at + ENTRY_SPEED_COLOR, (v << 4) | (packed & 0x0f));
        }
    }

    pub fn set_color(&mut self, slot: u8, v: u8) {
        if let Some(at) = self.entry(slot) {
            let packed = self.get(at + ENTRY_SPEED_COLOR);
            self.set(at + ENTRY_SPEED_COLOR, (packed & 0xf0) | (v & 0x0f));
        }
    }

    /// Patch a single byte directly. Used by the probe UI when confirming an
    /// offset against real hardware.
    pub fn poke(&mut self, at: usize, v: u8) {
        self.set(at, v);
    }

    /// Offsets that differ from `other`, for showing what a change did.
    pub fn diff(&self, other: &ConfigPage) -> Vec<(usize, u8, u8)> {
        self.bytes
            .iter()
            .zip(other.bytes.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i, *a, *b))
            .collect()
    }

    pub fn hex_dump(&self) -> String {
        let mut s = String::new();
        for (row, chunk) in self.bytes.chunks(16).enumerate() {
            s.push_str(&format!("{:04x}  ", row * 16));
            for b in chunk {
                s.push_str(&format!("{b:02x} "));
            }
            s.push('\n');
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page an F75 returns, straight off the wire.
    const F75: [u8; PAGE_LEN] = [
        0x00, 0x03, 0x03, 0x01, 0x00, 0x00, 0x04, 0x04, 0x07, 0x00, 0x01, 0x20, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x01, 0x00, 0xff, 0x0a, 0x00, 0x00, 0x00, 0x01, 0x00,
        0x03, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0x09, 0x40,
        0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09,
        0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47,
        0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x07, 0x47, 0x07, 0x47, 0x07, 0x44, 0x07,
        0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x04, 0x09, 0x04, 0x04,
        0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x5a, 0xa5,
    ];

    fn f75() -> ConfigPage {
        ConfigPage::new(F75.to_vec(), Layout::default())
    }

    #[test]
    fn recognises_a_real_page() {
        assert!(f75().looks_valid());
        assert!(!ConfigPage::blank(Layout::default()).looks_valid());
    }

    #[test]
    fn reads_the_live_effect_and_sleep_timeout() {
        let p = f75();
        assert_eq!(p.effect(), 0x01, "F75 was on Fixed_on");
        assert_eq!(p.sleep(), 0x0a, "SleepTime=300 maps to SleepHW 10");
    }

    #[test]
    fn the_live_effect_reads_from_slot_ui_index_minus_one() {
        // The page says effect 1, the first entry of the profile's list, so the
        // live settings are slot 0 at 0x3a: 09 40.
        assert_eq!(ConfigPage::slot_of(1), 0);
        assert_eq!(
            f75().current(),
            EffectSettings {
                brightness: 9,
                speed: 4,
                color: 0
            }
        );
    }

    #[test]
    fn every_effect_has_its_own_slot() {
        let p = f75();
        assert_eq!(
            p.slot_settings(1),
            EffectSettings {
                brightness: 9,
                speed: 4,
                color: 7
            }
        );
        assert_eq!(
            p.slot_settings(17),
            EffectSettings {
                brightness: 9,
                speed: 3,
                color: 7
            }
        );
        assert_eq!(
            p.slot_settings(20),
            EffectSettings {
                brightness: 7,
                speed: 4,
                color: 7
            }
        );
    }

    /// Pressing Fn+Tab on the keyboard, which cycles the colour of the live
    /// effect, moved exactly one byte: 0x3b from 0x40 to 0x41. That is slot 0's
    /// packed speed/colour byte, and 1 is green in [`COLORS`].
    #[test]
    fn matches_what_the_keyboard_does_on_fn_tab() {
        let mut p = f75();
        p.set_color(ConfigPage::slot_of(p.effect()), 1);
        assert_eq!(p.diff(&f75()), vec![(0x3b, 0x41, 0x40)]);
        assert_eq!(COLORS[1].0, "Green");
    }

    #[test]
    fn the_table_stops_before_the_end_marker() {
        let l = Layout::default();
        assert_eq!(l.effect_slots(), 34);
        let p = f75();
        // Slot 34 would land on the marker, so it is refused rather than
        // corrupting the page.
        let mut q = p.clone();
        q.set_brightness(34, 1);
        assert!(!q.dirty);
        assert_eq!(q.bytes[PAGE_LEN - 2..], [0x5a, 0xa5]);
    }

    #[test]
    fn changing_speed_keeps_the_colour_nibble() {
        let mut p = f75();
        p.set_speed(1, 2);
        assert_eq!(p.bytes[0x3d], 0x27);
        assert_eq!(
            p.slot_settings(1),
            EffectSettings {
                brightness: 9,
                speed: 2,
                color: 7
            }
        );
    }

    #[test]
    fn changing_colour_keeps_the_speed_nibble() {
        let mut p = f75();
        p.set_color(1, 2);
        assert_eq!(p.bytes[0x3d], 0x42);
    }

    #[test]
    fn a_change_touches_only_its_own_bytes() {
        let mut p = f75();
        p.set_brightness(1, 5);
        p.set_effect(3);
        let changed: Vec<usize> = p.diff(&f75()).into_iter().map(|(i, _, _)| i).collect();
        assert_eq!(changed, vec![0x0a, 0x3c]);
    }

    #[test]
    fn writing_the_same_value_does_not_dirty_the_page() {
        let mut p = f75();
        p.set_effect(0x01);
        assert!(!p.dirty);
        p.set_effect(0x02);
        assert!(p.dirty);
    }

    #[test]
    fn short_reads_are_padded_to_a_full_page() {
        let page = ConfigPage::new(vec![1, 2, 3], Layout::default());
        assert_eq!(page.bytes.len(), PAGE_LEN);
        assert_eq!(page.bytes[3], 0);
    }

    #[test]
    fn colour_slots_are_matched_by_distance() {
        assert_eq!(nearest_color([250, 5, 5]), 0);
        assert_eq!(nearest_color([0, 0, 240]), 2);
        assert_eq!(nearest_color([250, 250, 250]), 6);
    }
}
