use std::io;
use std::time::{Duration, Instant};

use crate::hid::{self, HidDevice, HidNode};

pub const NAME: &str = "AULA HERO 68 HE";
pub const UID: [u8; 6] = [0x11, 0, 0, 0, 0, 3];
const REPORT_ID: u8 = 9;
const REPORT_LEN: usize = 64;
const PAYLOAD: usize = 56;

// LED order follows the calibrated physical layout, including Fn and Left.
// https://github.com/SlumberMin/aula-hero68-protocol/blob/main/docs/KEYMAP.md
pub const KEY_ROWS: &[&[(u8, &str)]] = &[
    &[
        (1, "Esc"),
        (15, "1"),
        (16, "2"),
        (17, "3"),
        (18, "4"),
        (19, "5"),
        (20, "6"),
        (21, "7"),
        (22, "8"),
        (23, "9"),
        (24, "0"),
        (25, "-"),
        (26, "="),
        (27, "Backspace"),
        (98, "Ins"),
    ],
    &[
        (28, "Tab"),
        (29, "Q"),
        (30, "W"),
        (31, "E"),
        (32, "R"),
        (33, "T"),
        (34, "Y"),
        (35, "U"),
        (36, "I"),
        (37, "O"),
        (38, "P"),
        (39, "["),
        (40, "]"),
        (41, "\\"),
        (99, "Del"),
    ],
    &[
        (42, "Caps"),
        (43, "A"),
        (44, "S"),
        (45, "D"),
        (46, "F"),
        (47, "G"),
        (48, "H"),
        (49, "J"),
        (50, "K"),
        (51, "L"),
        (52, ";"),
        (53, "'"),
        (54, "Enter"),
        (102, "PgUp"),
    ],
    &[
        (55, "Shift"),
        (56, "Z"),
        (57, "X"),
        (58, "C"),
        (59, "V"),
        (60, "B"),
        (61, "N"),
        (62, "M"),
        (63, ","),
        (64, "."),
        (65, "/"),
        (66, "Shift"),
        (74, "Up"),
        (103, "PgDn"),
    ],
    &[
        (67, "Ctrl"),
        (68, "Super"),
        (69, "Alt"),
        (70, "Space"),
        (71, "Alt"),
        (72, "Fn"),
        (73, "Ctrl"),
        (76, "Left"),
        (75, "Down"),
        (77, "Right"),
    ],
];

pub const EFFECTS: &[&str] = &[
    "Steady",
    "Breathing",
    "Dream rainbow",
    "Reactive",
    "Rain",
    "Rainbow wheel",
    "Ripple",
    "Stars",
    "Fade",
    "Stream",
    "Wave",
    "Trail",
    "Sine wave",
    "Sweep",
    "Pinwheel",
    "Rainbow waterfall",
    "Bloom",
    "Vortex",
];

pub const KEY_COUNT: usize = 68;

pub enum Lighting {
    Effect { id: u8, color: [u8; 3] },
    PerKey(Box<[[u8; 3]; KEY_COUNT]>),
}

pub struct Hero {
    dev: HidDevice,
}

pub fn candidate(node: &HidNode) -> bool {
    (node.vid, node.pid) == (0x372e, 0x103e)
        && node.application_usages.contains(&(0xff60, 0x61))
        && node.output_reports.contains(&(REPORT_ID, REPORT_LEN - 1))
        && node.input_reports.contains(&(REPORT_ID, REPORT_LEN - 1))
}

fn report(command: u8, parameter: u8, total: u8, index: u8, payload: &[u8]) -> [u8; REPORT_LEN] {
    let mut frame = [0; REPORT_LEN];
    frame[..7].copy_from_slice(&[
        REPORT_ID,
        command,
        parameter,
        0,
        total,
        index,
        payload.len() as u8,
    ]);
    frame[7..7 + payload.len()].copy_from_slice(payload);
    frame[REPORT_LEN - 1] = !frame[..REPORT_LEN - 1]
        .iter()
        .fold(0u8, |sum, &byte| sum.wrapping_add(byte));
    frame
}

impl Hero {
    pub fn detect() -> (Option<Self>, Vec<String>) {
        let mut problems = Vec::new();
        for node in hid::enumerate().into_iter().filter(candidate) {
            match HidDevice::open(&node).and_then(Self::identify) {
                Ok(hero) => return (Some(hero), problems),
                Err(err) => problems.push(hid::permission_hint(&node.dev_path, &err)),
            }
        }
        (None, problems)
    }

    fn identify(dev: HidDevice) -> io::Result<Self> {
        dev.write_report(&report(0x82, 1, 1, 0, &[]))?;
        let deadline = Instant::now() + Duration::from_millis(750);
        while let Some(reply) =
            dev.read_report(deadline.saturating_duration_since(Instant::now()))?
        {
            if reply.len() != REPORT_LEN
                || reply[..4] != [REPORT_ID, 0x82, 1, 0]
                || reply[6] < UID.len() as u8
                || reply[6] as usize > PAYLOAD
            {
                continue;
            }
            if reply[7..13] == UID {
                return Ok(Self { dev });
            }
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported AULA HERO model",
            ));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "no HERO 68 HE identity reply",
        ))
    }

    pub fn is_present(&self) -> bool {
        self.dev.node.sys_path.exists() && self.dev.node.dev_path.exists() && self.dev.is_present()
    }

    pub fn path(&self) -> &std::path::Path {
        &self.dev.node.dev_path
    }

    fn send(&self, command: u8, parameter: u8, payload: &[u8]) -> io::Result<()> {
        self.dev
            .write_report(&report(command, parameter, 1, 0, payload))?;
        Ok(())
    }

    pub fn set_lighting(&self, setting: Lighting) -> io::Result<()> {
        match setting {
            Lighting::Effect {
                id,
                color: [r, g, b],
            } => {
                if id == 0 || id as usize > EFFECTS.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "unknown HERO effect",
                    ));
                }
                self.send(0x04, 1, &[id, 7, r, g, b, 0x14, 1])
            }
            Lighting::PerKey(colors) => {
                self.send(0x01, 0x80, &[0x10])?;
                self.send(0x04, 0x02, &[0x13, 0])?;
                self.send(0x04, 0x04, &[100, 4])?;
                // The one-shot mixed-color command handles all 68 colors in
                // seven packets. It is unsuitable for continuous animation.
                let entries: Vec<_> = KEY_ROWS
                    .iter()
                    .flat_map(|row| row.iter())
                    .zip(colors.iter())
                    .map(|(&(id, _), &[r, g, b])| [0, id, r, g, b])
                    .collect();
                let per_report = PAYLOAD / 5;
                let total = entries.len().div_ceil(per_report) as u8;
                for (index, chunk) in entries.chunks(per_report).enumerate() {
                    self.dev.write_report(&report(
                        0x06,
                        0,
                        total,
                        index as u8,
                        chunk.as_flattened(),
                    ))?;
                }
                self.send(0x01, 0, &[])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixDatagram;

    #[test]
    fn lights_all_68_keys_and_checks_identity_before_writing() {
        let (socket, keyboard) = UnixDatagram::pair().unwrap();
        keyboard
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let emulator = std::thread::spawn(move || {
            let read = || {
                let mut packet = [0; 64];
                assert_eq!(keyboard.recv(&mut packet).unwrap(), packet.len());
                assert_eq!(packet.iter().copied().fold(0u8, u8::wrapping_add), 0xff);
                assert_eq!(packet[0], 9);
                packet
            };
            assert_eq!(&read()[..7], &[9, 0x82, 1, 0, 1, 0, 0]);
            keyboard.send(&[0x01, 0]).unwrap();
            keyboard.send(&report(0x82, 1, 1, 0, &UID)).unwrap();
            assert_eq!(&read()[..14], &[9, 4, 1, 0, 1, 0, 7, 2, 7, 1, 2, 3, 20, 1]);
            assert_eq!(&read()[..8], &[9, 1, 0x80, 0, 1, 0, 1, 0x10]);
            assert_eq!(&read()[..9], &[9, 4, 2, 0, 1, 0, 2, 0x13, 0]);
            assert_eq!(&read()[..9], &[9, 4, 4, 0, 1, 0, 2, 100, 4]);

            let mut lit = std::collections::BTreeMap::new();
            for index in 0..7 {
                let packet = read();
                assert_eq!(&packet[..6], &[9, 6, 0, 0, 7, index]);
                let payload = &packet[7..7 + packet[6] as usize];
                assert_eq!(payload.len(), if index == 6 { 10 } else { 55 });
                for &[hi, id, red, green, blue] in payload.as_chunks::<5>().0 {
                    assert_eq!(hi, 0);
                    assert!(lit.insert(id, [red, green, blue]).is_none());
                }
            }
            assert_eq!(&read()[..7], &[9, 1, 0, 0, 1, 0, 0]);
            let expected_ids = std::iter::once(1).chain(15..=77).chain([98, 99, 102, 103]);
            assert!(lit.keys().copied().eq(expected_ids));
            assert_eq!(lit[&1], [4, 5, 6]);
            assert_eq!(lit[&72], [0, 0, 0]);
        });
        let hero = Hero::identify(HidDevice::from_test_socket(socket)).unwrap();
        assert!(hero
            .set_lighting(Lighting::Effect {
                id: 0,
                color: [0; 3]
            })
            .is_err());
        hero.set_lighting(Lighting::Effect {
            id: 2,
            color: [1, 2, 3],
        })
        .unwrap();
        let mut colors = [[0; 3]; KEY_COUNT];
        colors[0] = [4, 5, 6];
        hero.set_lighting(Lighting::PerKey(Box::new(colors)))
            .unwrap();
        emulator.join().unwrap();
    }

    #[test]
    fn refuses_another_model_sharing_the_vid_and_pid() {
        let (socket, keyboard) = UnixDatagram::pair().unwrap();
        keyboard
            .send(&report(0x82, 1, 1, 0, &[0x11, 0, 0, 0, 0, 0x14]))
            .unwrap();
        let result = Hero::identify(HidDevice::from_test_socket(socket));
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::Unsupported);
    }
}
