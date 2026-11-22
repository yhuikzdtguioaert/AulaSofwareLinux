use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::config_page::{
    ConfigPage, Layout, CUSTOM_BRIGHTNESS_OFFSET, KNOB_MODE_OFFSET, KNOB_TICKS_OFFSET, PAGE_LEN,
};
use crate::device::{self, Keyboard};
use crate::profile::DeviceProfile;

pub const KNOB_ACTIONS: [(&str, [u8; 4]); 8] = [
    ("Default", [0x07, 0, 0, 0x1d]),
    ("Play / Pause", [0x02, 0, 0, 0xcd]),
    ("Mute", [0x02, 0, 0, 0xe2]),
    ("Next track", [0x02, 0, 0, 0xb5]),
    ("Previous track", [0x02, 0, 0, 0xb6]),
    ("Stop", [0x02, 0, 0, 0xb7]),
    ("Volume up", [0x02, 0, 0, 0xe9]),
    ("Volume down", [0x02, 0, 0, 0xea]),
];

pub enum Request {
    Connect,
    WriteConfig(Vec<u8>),
    WriteRgbTable(Vec<u8>),
    SetKnob([u8; 4]),
    ExitKnobLightingMode,
    FactoryReset,
}

pub enum Event {
    Connected { profile_id: String, wireless: bool },
    Disconnected(String),
    Config(Vec<u8>),
    SavedConfig { sent: Vec<u8>, bytes: Vec<u8> },
    RgbTable(Vec<u8>),
    Power(Vec<u8>),
    Knob(Option<[u8; 4]>),
    Notice(String),
    Error(String),
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(profiles: Vec<DeviceProfile>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        thread::Builder::new()
            .name("aula-hid".into())
            .spawn(move || run(profiles, requests, events))
            .expect("failed to start device thread");
        Self { tx, rx }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

struct Session {
    keyboard: Keyboard,
    knob_slot: Option<usize>,
}

fn run(profiles: Vec<DeviceProfile>, rx: Receiver<Request>, tx: Sender<Event>) {
    let mut session: Option<Session> = None;
    loop {
        let request = match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(request) => request,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if session
                    .as_ref()
                    .is_some_and(|s| !s.keyboard.link.is_present())
                {
                    session = None;
                    let _ = tx.send(Event::Disconnected(String::new()));
                }
                if session.is_some() {
                    continue;
                }
                Request::Connect
            }
        };
        if matches!(request, Request::Connect) {
            session = connect(&profiles, &tx);
            continue;
        }
        let Some(active) = &mut session else { continue };
        let result = handle(active, request, &tx);
        if let Err(error) = result {
            eprintln!("aula: {error}");
            let lost = matches!(
                error.raw_os_error(),
                Some(libc::ENODEV | libc::EIO | libc::ENOENT)
            );
            let message = match error.kind() {
                io::ErrorKind::TimedOut => "Wake the keyboard with a key press.",
                io::ErrorKind::PermissionDenied => "Run the installer to allow device access.",
                io::ErrorKind::Unsupported => "Connect with USB to use this setting.",
                _ => "Could not save settings. Reconnect the keyboard.",
            };
            let _ = tx.send(Event::Error(message.into()));
            if lost {
                session = None;
                let _ = tx.send(Event::Disconnected("Reconnect the keyboard.".into()));
            }
        }
    }
}

fn connect(profiles: &[DeviceProfile], tx: &Sender<Event>) -> Option<Session> {
    let (found, problems) = device::autodetect(profiles);
    let Some(keyboard) = found else {
        let message = if problems.iter().any(|p| p.contains("permission denied")) {
            "Run the installer to allow device access."
        } else if !problems.is_empty() {
            "Wake the keyboard with a key press."
        } else if crate::hid::enumerate()
            .iter()
            .any(|n| n.is_aula_bluetooth())
        {
            "Connect with USB or the 2.4 GHz receiver to change settings."
        } else {
            "Connect your keyboard."
        };
        let _ = tx.send(Event::Disconnected(message.into()));
        return None;
    };
    open_session(keyboard, tx)
}

fn open_session(keyboard: Keyboard, tx: &Sender<Event>) -> Option<Session> {
    let page = match keyboard.link.read_config(PAGE_LEN).and_then(valid_config) {
        Ok(page) => page,
        Err(error) => {
            eprintln!("aula: reading settings: {error}");
            let message = if crate::hid::enumerate()
                .iter()
                .any(|n| n.is_aula_bluetooth())
            {
                "Connect with USB or the 2.4 GHz receiver to change settings."
            } else {
                "Wake the keyboard with a key press."
            };
            let _ = tx.send(Event::Disconnected(message.into()));
            return None;
        }
    };
    let _ = tx.send(Event::Connected {
        profile_id: keyboard.profile.id.clone(),
        wireless: keyboard.link.is_wireless(),
    });
    let _ = tx.send(Event::Config(page));
    if !keyboard.link.is_wireless() || keyboard.profile.is_f99_pro() {
        if let Ok(rgb) = keyboard.link.read_rgb_table() {
            let _ = tx.send(Event::RgbTable(rgb));
        }
    }
    if keyboard.profile.show_power || keyboard.link.is_wireless() {
        if let Ok(power) = keyboard.link.power() {
            let _ = tx.send(Event::Power(power));
        }
    }
    let knob = read_knob(&keyboard);
    let _ = tx.send(Event::Knob(knob.map(|(_, action)| action)));
    let knob_slot = knob.map(|(slot, _)| slot);
    Some(Session {
        keyboard,
        knob_slot,
    })
}

fn read_knob(kb: &Keyboard) -> Option<(usize, [u8; 4])> {
    if !kb
        .profile
        .keys
        .iter()
        .any(|key| key.name.eq_ignore_ascii_case("wheel"))
    {
        return None;
    }
    let matrix = read_matrix(kb).ok()?;
    crate::remap::locate_knob(&kb.profile, &matrix)
}

fn read_matrix(kb: &Keyboard) -> io::Result<Vec<u8>> {
    let bytes = kb.link.read_matrix(0, kb.profile.matrix_len)?;
    if kb.profile.is_f99_pro()
        && !kb.link.is_wireless()
        && !bytes.ends_with(&crate::remap::MATRIX_TRAILER)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid key matrix",
        ));
    }
    Ok(bytes)
}

fn verify_config(kb: &Keyboard, sent: &[u8]) -> io::Result<Vec<u8>> {
    let bytes = valid_config(kb.link.read_config(PAGE_LEN)?)?;
    if kb.profile.is_f99_pro() {
        let layout = Layout::load_override(&kb.profile.dir);
        let expected = ConfigPage::new(sent.to_vec(), layout);
        let actual = ConfigPage::new(bytes.clone(), layout);
        if expected.effect() != actual.effect()
            || expected.is_custom() != actual.is_custom()
            || expected.current() != actual.current()
            || expected.sleep() != actual.sleep()
            || expected.bytes[KNOB_MODE_OFFSET] != actual.bytes[KNOB_MODE_OFFSET]
        {
            return Err(io::Error::other("the keyboard did not retain the settings"));
        }
    }
    Ok(bytes)
}

fn valid_config(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if bytes.len() != PAGE_LEN || !ConfigPage::new(bytes.clone(), Layout::default()).looks_valid() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid settings page",
        ));
    }
    Ok(bytes)
}

fn handle(session: &mut Session, request: Request, tx: &Sender<Event>) -> io::Result<()> {
    let kb = &session.keyboard;
    match request {
        Request::WriteConfig(page) => {
            let sent = valid_config(page)?;
            let mut outgoing = sent.clone();
            if kb.profile.is_f99_pro() {
                let live = valid_config(kb.link.read_config(PAGE_LEN)?)?;
                outgoing[KNOB_MODE_OFFSET] = live[KNOB_MODE_OFFSET];
                outgoing[KNOB_TICKS_OFFSET] = live[KNOB_TICKS_OFFSET];
            }
            kb.link.write_config(&outgoing)?;
            let bytes = verify_config(kb, &outgoing)?;
            let _ = tx.send(Event::SavedConfig { sent, bytes });
        }
        Request::WriteRgbTable(rgb) => {
            let mut page = valid_config(kb.link.read_config(PAGE_LEN)?)?;
            kb.link.write_rgb_table(&rgb)?;
            page[0x09] = 1;
            page[Layout::load_override(&kb.profile.dir).effect] = kb.profile.custom_effect();
            if page[CUSTOM_BRIGHTNESS_OFFSET] == 0 {
                page[CUSTOM_BRIGHTNESS_OFFSET] = if kb.profile.is_f99_pro() { 9 } else { 4 };
            }
            kb.link.write_config(&page)?;
            let _ = tx.send(Event::Config(verify_config(kb, &page)?));
        }
        Request::SetKnob(action) => {
            let slot = session
                .knob_slot
                .ok_or_else(|| io::Error::other("knob not found"))?;
            let mut matrix = read_matrix(kb)?;
            let saved_page = if kb.profile.is_f99_pro() {
                None
            } else {
                Some(valid_config(kb.link.read_config(PAGE_LEN)?)?)
            };
            crate::remap::set_knob(&mut matrix, slot, action)?;
            kb.link.write_matrix(0, &matrix)?;
            if let Some(page) = saved_page {
                kb.link.write_config(&page)?;
            }
            let back = read_matrix(kb)?;
            if back.get(slot * 4..slot * 4 + 4) != Some(action.as_slice()) {
                return Err(io::Error::other("knob setting did not persist"));
            }
            let _ = tx.send(Event::Knob(Some(action)));
        }
        Request::ExitKnobLightingMode => {
            if !kb.profile.is_f99_pro() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "knob lighting mode",
                ));
            }
            let mut page = valid_config(kb.link.read_config(PAGE_LEN)?)?;
            if page[KNOB_MODE_OFFSET] == 1 {
                page[KNOB_MODE_OFFSET] = 0;
                kb.link.write_config(&page)?;
                verify_config(kb, &page)?;
            }
        }
        Request::FactoryReset => {
            kb.link.reset()?;
            let _ = tx.send(Event::Config(valid_config(kb.link.read_config(PAGE_LEN)?)?));
            let knob = read_knob(kb);
            session.knob_slot = knob.map(|(slot, _)| slot);
            let _ = tx.send(Event::Knob(knob.map(|(_, action)| action)));
            if let Ok(rgb) = kb.link.read_rgb_table() {
                let _ = tx.send(Event::RgbTable(rgb));
            }
        }
        Request::Connect => return Ok(()),
    }
    let _ = tx.send(Event::Notice(String::new()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::HidDevice;
    use crate::proto::{wireless, Link};
    use std::os::unix::net::UnixDatagram;

    #[test]
    fn f99_pro_receiver_identifies_reads_and_saves_without_per_packet_acknowledgements() {
        let (socket, receiver) = UnixDatagram::pair().unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let emulator = thread::spawn(move || {
            let mut config = [0; PAGE_LEN];
            config[0x0a] = 1;
            config[0x3a] = 9;
            config[0x18] = 10;
            config[KNOB_MODE_OFFSET] = 1;
            config[PAGE_LEN - 2..].copy_from_slice(&[0x5a, 0xa5]);
            let mut rgb = [0; 378];
            rgb[9] = 10;
            rgb[126 + 9] = 20;
            rgb[252 + 9] = 30;
            let mut matrix = [0; 504];
            matrix[432..436].copy_from_slice(&[2, 0, 0, 0xe2]);

            let mut output = [0; PAGE_LEN];
            let mut readbacks = 0;
            loop {
                let mut request = [0; 20];
                assert_eq!(receiver.recv(&mut request).unwrap(), 20);
                assert_eq!(request[0], 0x13);
                assert_eq!(request[19], wireless::checksum(&request));
                let data: &[u8] = match request[1] {
                    0x05 => &[3, 0, 0, 0, 1, 0x58],
                    0x4a => &[71, 1],
                    0x41 => &matrix,
                    0x42 => &rgb,
                    0x44 => {
                        readbacks += 1;
                        &config
                    }
                    0x04 => {
                        assert_eq!(request[2], 10);
                        assert_eq!(request[4] >> 4, 1);
                        let offset = request[3] as usize * 14;
                        let len = (request[4] & 0xf) as usize;
                        output[offset..offset + len].copy_from_slice(&request[5..5 + len]);
                        if offset + len == PAGE_LEN {
                            config = output;
                        }
                        continue;
                    }
                    other => panic!("unexpected receiver opcode {other:02x}"),
                };
                let total = data.len().div_ceil(14) as u8;
                // A valid settings read also works when replies are out of order.
                for (index, chunk) in data.chunks(14).enumerate().rev() {
                    receiver
                        .send(&wireless::packet(request[1], 0, chunk, total, index as u8))
                        .unwrap();
                }
                if readbacks == 5 {
                    return config;
                }
            }
        });
        let profile =
            DeviceProfile::load(std::path::Path::new("assets/devices/f99-pro-f99pro")).unwrap();
        let kb = device::identify(
            Link::wireless(HidDevice::from_test_socket(socket)),
            &[profile],
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let mut session = open_session(kb, &tx).unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::Connected { wireless: true, .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::Power(bytes) if bytes == &[71, 1])));
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::RgbTable(bytes) if bytes[27..30] == [10, 20, 30])));
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::Knob(Some([2, 0, 0, 0xe2])))));
        let mut page = events
            .into_iter()
            .find_map(|event| match event {
                Event::Config(bytes) => Some(ConfigPage::new(bytes, Layout::default())),
                _ => None,
            })
            .unwrap();
        assert_eq!(page.current().brightness, 9);
        let profile = &session.keyboard.profile;
        let off = profile
            .led_opts
            .iter()
            .find(|effect| effect.is_off())
            .unwrap();
        assert_eq!(profile.brightness_levels(), 10);
        assert_eq!(profile.sleep_hw(91), None);
        page.set_effect(profile.effect_value(*off));
        page.set_sleep(profile.sleep_hw(90).unwrap());
        // A stale UI copy of the firmware's mode must not clear it.
        page.bytes[KNOB_MODE_OFFSET] = 0;
        handle(&mut session, Request::WriteConfig(page.bytes), &tx).unwrap();
        assert!(rx
            .try_iter()
            .any(|event| matches!(event, Event::SavedConfig { bytes, .. }
            if bytes[KNOB_MODE_OFFSET] == 1 && bytes[0x0a] == 0 && bytes[0x18] == 3)));
        handle(&mut session, Request::ExitKnobLightingMode, &tx).unwrap();
        let saved = emulator.join().unwrap();
        assert_eq!(saved[KNOB_MODE_OFFSET], 0);
        assert_eq!(saved[0x0a], 0);
        assert_eq!(saved[0x18], 3);
        assert_eq!(saved[0x3a], 9);
        assert_eq!(saved[PAGE_LEN - 2..], [0x5a, 0xa5]);
    }
}
