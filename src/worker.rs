use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::config_page::{ConfigPage, Layout, PAGE_LEN};
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
    FactoryReset,
}

pub enum Event {
    Connected { profile_id: String, wireless: bool },
    Disconnected(String),
    Config(Vec<u8>),
    SavedConfig { sent: Vec<u8>, bytes: Vec<u8> },
    RgbTable(Vec<u8>),
    Power(Vec<u8>),
    Knob([u8; 4]),
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
    if !keyboard.link.is_wireless() {
        if let Ok(rgb) = keyboard.link.read_rgb_table() {
            let _ = tx.send(Event::RgbTable(rgb));
        }
    }
    if keyboard.profile.show_power || keyboard.link.is_wireless() {
        if let Ok(power) = keyboard.link.power() {
            let _ = tx.send(Event::Power(power));
        }
    }
    let knob = (!keyboard.link.is_wireless())
        .then_some(&keyboard)
        .filter(|keyboard| {
            keyboard
                .profile
                .keys
                .iter()
                .any(|key| key.name.eq_ignore_ascii_case("wheel"))
        })
        .and_then(|keyboard| {
            keyboard
                .link
                .read_matrix(0, keyboard.profile.matrix_len)
                .ok()
        })
        .and_then(|matrix| crate::remap::locate_knob(&keyboard.profile, &matrix));
    if let Some((_, action)) = knob {
        let _ = tx.send(Event::Knob(action));
    }
    let knob_slot = knob.map(|(slot, _)| slot);
    Some(Session {
        keyboard,
        knob_slot,
    })
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
            kb.link.write_config(&sent)?;
            let bytes = valid_config(kb.link.read_config(PAGE_LEN)?)?;
            let _ = tx.send(Event::SavedConfig { sent, bytes });
        }
        Request::WriteRgbTable(rgb) => {
            let mut page = valid_config(kb.link.read_config(PAGE_LEN)?)?;
            kb.link.write_rgb_table(&rgb)?;
            page[0x09] = 1;
            page[Layout::load_override(&kb.profile.dir).effect] = 21;
            if page[0x75] == 0 {
                page[0x75] = 4;
            }
            kb.link.write_config(&page)?;
            let _ = tx.send(Event::Config(valid_config(kb.link.read_config(PAGE_LEN)?)?));
        }
        Request::SetKnob(action) => {
            let slot = session
                .knob_slot
                .ok_or_else(|| io::Error::other("knob not found"))?;
            let mut matrix = kb.link.read_matrix(0, kb.profile.matrix_len)?;
            let page = valid_config(kb.link.read_config(PAGE_LEN)?)?;
            // Store the slot before writing so it remains discoverable after a restart.
            crate::remap::set_knob(&mut matrix, slot, action)?;
            crate::remap::remember_knob(&kb.profile, slot, action)?;
            kb.link.write_matrix(0, &matrix)?;
            kb.link.write_config(&page)?;
            let back = kb.link.read_matrix(0, kb.profile.matrix_len)?;
            if back.get(slot * 4..slot * 4 + 4) != Some(action.as_slice()) {
                return Err(io::Error::other("knob setting did not persist"));
            }
            let _ = tx.send(Event::Knob(action));
        }
        Request::FactoryReset => {
            kb.link.reset()?;
            let _ = tx.send(Event::Config(valid_config(kb.link.read_config(PAGE_LEN)?)?));
        }
        Request::Connect => return Ok(()),
    }
    let _ = tx.send(Event::Notice(String::new()));
    Ok(())
}
