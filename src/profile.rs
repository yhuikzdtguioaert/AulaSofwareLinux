//! Device descriptions, read from the vendor `KB.ini` shipped with each
//! AULA package (see `assets/devices/<id>/device.ini`).
//!
//! One `KB.ini` fully describes a keyboard: USB ids, the 6-byte password that
//! identifies the model behind the shared 258a:010c pair, the pixel geometry of
//! every key on `keyimg.png`, the LED index of every key, and which hardware
//! lighting effects the firmware exposes.

use std::path::{Path, PathBuf};

use crate::ini::{self, Ini};

/// A key as drawn on `keyimg.png` and addressed in the key matrix.
#[derive(Debug, Clone)]
pub struct KeyDef {
    /// 1-based index as written in the ini (`K1`, `K2`, …).
    pub index: u32,
    /// Comment above the entry, e.g. `Esc`. Used as the on-screen caption.
    pub name: String,
    pub rect: [i32; 4],
    /// Function class: 0x02 = plain key, 0x09 = multimedia / system action.
    pub major: u8,
    pub minor: u8,
    pub param: u32,
    /// Position of this key in the firmware LED table.
    pub led: i32,
}

impl KeyDef {
    pub fn width(&self) -> i32 {
        self.rect[2] - self.rect[0]
    }
    pub fn height(&self) -> i32 {
        self.rect[3] - self.rect[1]
    }
    pub fn is_drawn(&self) -> bool {
        self.width() > 0 && self.height() > 0
    }
    /// The 32-bit function id the firmware stores for this key.
    pub fn function_id(&self) -> u32 {
        u32::from_be_bytes([
            self.major,
            self.minor,
            (self.param >> 8) as u8,
            self.param as u8,
        ])
    }
}

/// One entry of the hardware lighting-effect list (`LedOptN=`).
///
/// Fields are, in the vendor's own comment order:
/// `hw effect speed light direct random color`.
#[derive(Debug, Clone, Copy)]
pub struct LedOpt {
    /// Index used by the UI effect list.
    pub ui_index: u8,
    /// Value written to the firmware.
    pub hw_effect: u8,
    pub has_speed: bool,
    pub has_light: bool,
    pub has_direction: bool,
    pub has_random: bool,
    pub has_color: bool,
}

impl LedOpt {
    /// The row every model ends its list with: hardware effect 0 and no
    /// controls at all, i.e. lighting off.
    pub fn is_off(&self) -> bool {
        self.hw_effect == 0
    }
}

/// A layer of alternate key functions (`[FN1]`, `[FN2]`, …).
#[derive(Debug, Clone)]
pub struct FnLayer {
    pub name: String,
    /// Keyed by the same `K<n>` index as [`KeyDef::index`].
    pub entries: Vec<(u32, u8, u8, u32)>,
}

#[derive(Debug, Clone)]
pub struct DeviceProfile {
    pub id: String,
    pub dir: PathBuf,

    pub name: String,
    pub short_name: String,
    pub vid: u16,
    pub pid: u16,
    pub wireless_vid: Option<u16>,
    pub wireless_pid: Option<u16>,
    /// The 6 bytes returned by the `read password` command; this is what tells
    /// one AULA model from another, since they all share 258a:010c.
    pub password: [u8; 6],
    pub fw: i64,
    pub crc: bool,
    pub matrix_len: usize,
    pub channel_mask: u8,
    pub show_power: bool,
    pub led_mask: u32,
    pub default_led_index: u8,

    pub light_ui: Vec<i64>,
    pub light_hw: Vec<i64>,
    pub speed_ui: Vec<i64>,
    pub speed_hw: Vec<i64>,
    pub sleep_ui: Vec<i64>,
    pub sleep_hw: Vec<i64>,
    pub led_opts: Vec<LedOpt>,

    pub color_key_hover: [u8; 3],
    pub color_key_down: [u8; 3],
    pub color_key_has_func: [u8; 3],

    pub keys: Vec<KeyDef>,
    pub fn_layers: Vec<FnLayer>,
    pub keyimg: Option<PathBuf>,
}

impl DeviceProfile {
    pub fn load(dir: &Path) -> anyhow::Result<DeviceProfile> {
        let path = dir.join("device.ini");
        let raw = std::fs::read(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let text = ini::decode(&raw);
        let id = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::from_text(&id, dir, &text)
    }

    pub fn from_text(id: &str, dir: &Path, text: &str) -> anyhow::Result<DeviceProfile> {
        let ini = Ini::parse(text);

        let password = parse_password(ini.get("OPT", "Psd").unwrap_or(""))
            .ok_or_else(|| anyhow::anyhow!("{id}: missing or malformed Psd="))?;

        let keyimg = ["keyimg.png", "keyimg.PNG"]
            .iter()
            .map(|f| dir.join(f))
            .find(|p| p.exists());

        let mut profile = DeviceProfile {
            id: id.to_string(),
            dir: dir.to_path_buf(),
            name: ini.str("OPT", "Name").unwrap_or_else(|| id.to_string()),
            short_name: ini
                .str("OPT", "ShortName")
                .unwrap_or_else(|| id.to_string()),
            vid: ini.num("OPT", "VID").unwrap_or(0) as u16,
            pid: ini.num("OPT", "PID").unwrap_or(0) as u16,
            // Wired-only models write `VID_Wireless=0` rather than leaving the
            // key out, so zero means "no dongle", not USB id 0000.
            wireless_vid: ini
                .num("OPT", "VID_Wireless")
                .map(|v| v as u16)
                .filter(|v| *v != 0),
            wireless_pid: ini
                .num("OPT", "PID_Wireless")
                .map(|v| v as u16)
                .filter(|v| *v != 0),
            password,
            fw: ini.num("OPT", "Fw").unwrap_or(0),
            crc: ini.num("OPT", "CRC").unwrap_or(0) != 0,
            matrix_len: ini.num("OPT", "MatrixLen").unwrap_or(128) as usize,
            channel_mask: ini.num("OPT", "ChannelMask").unwrap_or(1) as u8,
            show_power: ini.num("OPT", "ShowPower").unwrap_or(0) != 0,
            led_mask: ini.num("OPT", "LedMask").unwrap_or(0) as u32,
            default_led_index: ini.num("OPT", "DefLedIndex").unwrap_or(0) as u8,
            light_ui: ini.nums("OPT", "Light"),
            light_hw: ini.nums("OPT", "LightHW"),
            speed_ui: ini.nums("OPT", "Speed"),
            speed_hw: ini.nums("OPT", "SpeedHW"),
            sleep_ui: ini.nums("OPT", "SleepUI"),
            sleep_hw: ini.nums("OPT", "SleepHW"),
            led_opts: Vec::new(),
            color_key_hover: ini.rgb("OPT", "clrKeyOv").unwrap_or([255, 255, 255]),
            color_key_down: ini.rgb("OPT", "clrKeyDn").unwrap_or([255, 0, 0]),
            color_key_has_func: ini.rgb("OPT", "clrKeyHasFunc").unwrap_or([10, 147, 255]),
            keys: Vec::new(),
            fn_layers: Vec::new(),
            keyimg,
        };

        for n in 1..=64 {
            let Some(raw) = ini.get("OPT", &format!("LedOpt{n}")) else {
                continue;
            };
            let v: Vec<i64> = raw.split(',').filter_map(ini::parse_num).collect();
            if v.len() < 7 {
                continue;
            }
            // The all-zero row terminates the list. The row before it is
            // usually `21,0,0,0,0,0,0`, which is a real entry — hardware
            // effect 0, lighting off.
            if v[0] == 0 {
                break;
            }
            profile.led_opts.push(LedOpt {
                ui_index: v[0] as u8,
                hw_effect: v[1] as u8,
                has_speed: v[2] != 0,
                has_light: v[3] != 0,
                has_direction: v[4] != 0,
                has_random: v[5] != 0,
                has_color: v[6] != 0,
            });
        }

        profile.keys = parse_keys(text);
        for n in 1..=8 {
            let sect = format!("FN{n}");
            let Some(map) = ini.section(&sect) else {
                continue;
            };
            let mut entries = Vec::new();
            for (k, v) in map {
                let Some(idx) = k.strip_prefix('K').and_then(|s| s.parse::<u32>().ok()) else {
                    continue;
                };
                let f: Vec<i64> = v.split(',').filter_map(ini::parse_num).collect();
                if f.len() >= 3 {
                    entries.push((idx, f[0] as u8, f[1] as u8, f[2] as u32));
                }
            }
            entries.sort_by_key(|e| e.0);
            profile.fn_layers.push(FnLayer {
                name: sect,
                entries,
            });
        }

        Ok(profile)
    }

    /// Bounding box of the drawn keys, in `keyimg.png` pixel space.
    pub fn key_bounds(&self) -> [i32; 4] {
        let mut b = [i32::MAX, i32::MAX, 0, 0];
        for k in self.keys.iter().filter(|k| k.is_drawn()) {
            b[0] = b[0].min(k.rect[0]);
            b[1] = b[1].min(k.rect[1]);
            b[2] = b[2].max(k.rect[2]);
            b[3] = b[3].max(k.rect[3]);
        }
        if b[0] == i32::MAX {
            b = [0, 0, 1, 1];
        }
        b
    }

    /// Highest LED index referenced by any key, i.e. the size of the per-key
    /// RGB table this device expects.
    pub fn led_count(&self) -> usize {
        self.keys
            .iter()
            .map(|k| k.led)
            .max()
            .map(|m| (m + 1).max(0) as usize)
            .unwrap_or(0)
    }

    pub fn brightness_levels(&self) -> usize {
        self.light_ui.len().max(1)
    }

    pub fn speed_levels(&self) -> usize {
        self.speed_ui.len().max(1)
    }

    /// Map a UI brightness step onto the value the firmware wants.
    pub fn brightness_hw(&self, ui: usize) -> u8 {
        *self
            .light_hw
            .get(ui)
            .or_else(|| self.light_hw.last())
            .unwrap_or(&0) as u8
    }

    pub fn speed_hw(&self, ui: usize) -> u8 {
        *self
            .speed_hw
            .get(ui)
            .or_else(|| self.speed_hw.last())
            .unwrap_or(&0) as u8
    }

    /// Inverse of [`Self::brightness_hw`], for showing what the device stores.
    pub fn brightness_ui(&self, hw: u8) -> usize {
        self.light_hw
            .iter()
            .position(|v| *v == hw as i64)
            .unwrap_or(0)
    }

    /// Inverse of [`Self::speed_hw`].
    pub fn speed_ui(&self, hw: u8) -> usize {
        self.speed_hw
            .iter()
            .position(|v| *v == hw as i64)
            .unwrap_or(0)
    }

    /// Sleep timeout in seconds -> firmware units (minutes-ish, per `SleepHW`).
    pub fn sleep_hw(&self, ui_seconds: i64) -> u8 {
        self.sleep_ui
            .iter()
            .position(|s| *s == ui_seconds)
            .and_then(|i| self.sleep_hw.get(i))
            .copied()
            .unwrap_or(0) as u8
    }
}

fn parse_password(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 6 {
        return None;
    }
    let mut out = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        // Written as bare hex without a prefix: `3,0,0,0,0,cd`.
        out[i] = ini::parse_hex_byte(p)?;
    }
    Some(out)
}

/// `[KEY]` needs a hand-rolled pass: the key caption lives in the `;comment`
/// line above each entry, which a normal ini parser throws away.
fn parse_keys(text: &str) -> Vec<KeyDef> {
    let mut keys = Vec::new();
    let mut in_key = false;
    let mut pending_name = String::new();

    for raw in text.lines() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_key = line[1..line.len() - 1].trim().eq_ignore_ascii_case("KEY");
            continue;
        }
        if !in_key {
            continue;
        }
        if let Some(c) = line.strip_prefix(';') {
            pending_name = c.trim().to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let Some(index) = k
            .trim()
            .strip_prefix(['K', 'k'])
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let f: Vec<i64> = v.split(',').filter_map(ini::parse_num).collect();
        if f.len() < 8 {
            continue;
        }
        keys.push(KeyDef {
            index,
            name: std::mem::take(&mut pending_name),
            rect: [f[0] as i32, f[1] as i32, f[2] as i32, f[3] as i32],
            major: f[4] as u8,
            minor: f[5] as u8,
            param: f[6] as u32,
            led: f[7] as i32,
        });
    }
    keys.sort_by_key(|k| k.index);
    keys
}

/// Load every profile under `assets/devices`.
pub fn load_all(root: &Path) -> Vec<DeviceProfile> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("device.ini").is_file())
        .collect();
    dirs.sort();
    for d in dirs {
        match DeviceProfile::load(&d) {
            Ok(p) => out.push(p),
            Err(e) => eprintln!("aula: skipping {}: {e}", d.display()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[OPT]
Fw=24
VID=0x258a
PID=0x010C
Psd=3,0,0,0,0,cd
CRC=1
MatrixLen=128
Name=AULA F75
ShortName=F75
Light=0,1,2,3
LightHW=0,1,2,3
Speed=0,1,2,3,4
SpeedHW=4,3,2,1,0
SleepUI=60,90
SleepHW=2,3
LedOpt1=  1,1,0,1,0,1,1
LedOpt2=  2,3,1,1,0,1,1
LedOpt19=21,0,0,0,0,0,0
LedOpt20=0,0,0,0,0,0,0
[KEY]
;Esc
K1=27,33,53,61, 0x02,0x1B,0x00,0
;Wheel
K82=0,0,0,0, 0x09,0x01,0x0700001D,84
[FN1]
;Esc
K1=0x09,0x01,0x07000004
";

    fn sample() -> DeviceProfile {
        DeviceProfile::from_text("f75", Path::new("."), SAMPLE).unwrap()
    }

    #[test]
    fn parses_identity() {
        let p = sample();
        assert_eq!(p.vid, 0x258a);
        assert_eq!(p.pid, 0x010c);
        assert_eq!(p.password, [3, 0, 0, 0, 0, 0xcd]);
        assert_eq!(p.matrix_len, 128);
    }

    #[test]
    fn parses_keys_with_captions() {
        let p = sample();
        assert_eq!(p.keys.len(), 2);
        assert_eq!(p.keys[0].name, "Esc");
        assert_eq!(p.keys[0].rect, [27, 33, 53, 61]);
        assert_eq!(p.keys[0].minor, 0x1b);
        assert!(p.keys[0].is_drawn());
        assert_eq!(p.keys[1].name, "Wheel");
        assert!(!p.keys[1].is_drawn());
        assert_eq!(p.keys[1].led, 84);
        assert_eq!(p.led_count(), 85);
    }

    #[test]
    fn stops_at_the_terminator_row() {
        let p = sample();
        // LedOpt1, LedOpt2 and the `21,0,…` off row; LedOpt20 terminates.
        assert_eq!(p.led_opts.len(), 3);
        assert_eq!(p.led_opts[1].hw_effect, 3);
        assert!(p.led_opts[1].has_speed);
        assert!(!p.led_opts[0].has_speed);
        assert!(p.led_opts[2].is_off());
        assert!(!p.led_opts[0].is_off());
    }

    #[test]
    fn maps_ui_steps_to_hardware_values() {
        let p = sample();
        assert_eq!(p.speed_hw(0), 4);
        assert_eq!(p.speed_hw(4), 0);
        assert_eq!(p.speed_hw(99), 0, "out of range clamps to the last entry");
        assert_eq!(p.sleep_hw(90), 3);
    }

    #[test]
    fn a_zero_wireless_id_means_there_is_no_dongle() {
        // The wired-only profiles ship `VID_Wireless=0`; taking that literally
        // would put a rule for USB 0000:0000 in the generated udev file.
        let text = "[OPT]\nVID=0x258a\nPID=0x010C\nPsd=3,0,0,0,0,cd\n\
                    VID_Wireless=0\nPID_Wireless=0\n";
        let p = DeviceProfile::from_text("wired", Path::new("."), text).unwrap();
        assert_eq!(p.wireless_vid, None);
        assert_eq!(p.wireless_pid, None);

        let text = text
            .replace("VID_Wireless=0", "VID_Wireless=0x3554")
            .replace("PID_Wireless=0", "PID_Wireless=0xfa09");
        let p = DeviceProfile::from_text("wireless", Path::new("."), &text).unwrap();
        assert_eq!(p.wireless_vid, Some(0x3554));
        assert_eq!(p.wireless_pid, Some(0xfa09));
    }

    #[test]
    fn maps_hardware_values_back_to_ui_steps() {
        let p = sample();
        // SpeedHW=4,3,2,1,0 so the mapping is reversed.
        assert_eq!(p.speed_ui(4), 0);
        assert_eq!(p.speed_ui(0), 4);
        assert_eq!(p.brightness_ui(2), 2);
        assert_eq!(
            p.speed_ui(200),
            0,
            "unknown values fall back to the first step"
        );
    }

    #[test]
    fn reads_fn_layer() {
        let p = sample();
        assert_eq!(p.fn_layers.len(), 1);
        assert_eq!(p.fn_layers[0].entries, vec![(1, 0x09, 0x01, 0x07000004)]);
    }
}
