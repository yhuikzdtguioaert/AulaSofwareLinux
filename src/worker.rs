//! Background HID thread.
//!
//! Every transfer sleeps 20 ms per package (the vendor driver does the same and
//! the firmware drops packets without it), so all device access lives on its
//! own thread and the UI only ever exchanges messages with it.

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use crate::config_page::PAGE_LEN;
use crate::device::{self, Keyboard};
use crate::profile::DeviceProfile;

#[derive(Debug, Clone)]
pub enum Request {
    /// Look for a keyboard and identify it.
    Connect,
    /// Re-read the settings page, the per-key colours and the battery.
    Refresh,
    WriteConfig(Vec<u8>),
    WriteRgbTable(Vec<u8>),
    /// Write one byte of the settings page, for confirming an offset.
    PokeConfig {
        offset: usize,
        value: u8,
    },
    FactoryReset,
    /// Raw read, exposed by `aula probe` for protocol work.
    RawRead {
        command: u8,
        parameter: u8,
        len: usize,
    },
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A keyboard was identified.
    Connected {
        profile_id: String,
        display_name: String,
        password: [u8; 6],
        report_id: u8,
        node: String,
    },
    Disconnected(String),
    Config(Vec<u8>),
    RgbTable(Vec<u8>),
    Power(Vec<u8>),
    RawData {
        command: u8,
        data: Vec<u8>,
    },
    /// Something the user should see in the status line.
    Notice(String),
    Error(String),
    /// A line for the protocol log pane.
    Log(String),
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(profiles: Vec<DeviceProfile>) -> Worker {
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (ev_tx, ev_rx) = mpsc::channel::<Event>();
        thread::Builder::new()
            .name("aula-hid".into())
            .spawn(move || run(profiles, req_rx, ev_tx))
            .expect("failed to start the HID thread");
        Worker {
            tx: req_tx,
            rx: ev_rx,
        }
    }

    pub fn send(&self, r: Request) {
        // The UI keeps running even if the HID thread has gone away; the
        // missing replies surface as a stale status line rather than a panic.
        let _ = self.tx.send(r);
    }

    pub fn poll(&self) -> Vec<Event> {
        let mut out = Vec::new();
        // try_recv stops on both Empty and Disconnected, which is what we want:
        // a dead HID thread simply stops producing events.
        while let Ok(e) = self.rx.try_recv() {
            out.push(e);
        }
        out
    }
}

fn run(profiles: Vec<DeviceProfile>, rx: Receiver<Request>, tx: Sender<Event>) {
    let mut kb: Option<Keyboard> = None;
    while let Ok(req) = rx.recv() {
        match req {
            Request::Connect => match connect(&profiles, &tx) {
                Some(k) => {
                    kb = Some(k);
                    refresh(kb.as_ref().unwrap(), &tx);
                }
                None => kb = None,
            },
            Request::Refresh => match &kb {
                Some(k) => refresh(k, &tx),
                None => send_disconnected(&tx),
            },
            Request::WriteConfig(page) => with(&kb, &tx, |k| {
                k.link.write_config(&page)?;
                let _ = tx.send(Event::Log(format!("SET_LED {} bytes", page.len())));
                Ok(())
            }),
            Request::WriteRgbTable(rgb) => with(&kb, &tx, |k| {
                k.link.write_rgb_table(&rgb)?;
                let _ = tx.send(Event::Log(format!("SET_RGB_TABLE {} bytes", rgb.len())));
                Ok(())
            }),
            Request::PokeConfig { offset, value } => with(&kb, &tx, |k| {
                let mut page = k.link.read_config(PAGE_LEN)?;
                page.resize(PAGE_LEN, 0);
                if offset < page.len() {
                    page[offset] = value;
                }
                k.link.write_config(&page)?;
                let _ = tx.send(Event::Log(format!("poke [0x{offset:02x}] = 0x{value:02x}")));
                let back = k.link.read_config(PAGE_LEN)?;
                let _ = tx.send(Event::Config(back));
                Ok(())
            }),
            Request::FactoryReset => with(&kb, &tx, |k| {
                k.link.reset()?;
                let _ = tx.send(Event::Log("RESET".into()));
                Ok(())
            }),
            Request::RawRead {
                command,
                parameter,
                len,
            } => with(&kb, &tx, |k| {
                let data = k.link.read(command, parameter, len)?;
                let _ = tx.send(Event::Log(format!(
                    "read cmd=0x{command:02x} param=0x{parameter:02x} -> {} bytes",
                    data.len()
                )));
                let _ = tx.send(Event::RawData { command, data });
                Ok(())
            }),
        }
    }
}

fn connect(profiles: &[DeviceProfile], tx: &Sender<Event>) -> Option<Keyboard> {
    let (found, problems) = device::autodetect(profiles);
    for p in problems {
        let _ = tx.send(Event::Log(p));
    }
    match found {
        Some(kb) => {
            let _ = tx.send(Event::Connected {
                profile_id: kb.profile.id.clone(),
                display_name: kb.profile.name.clone(),
                password: kb.password,
                report_id: kb.link.report_id(),
                node: kb.link.node_path(),
            });
            Some(kb)
        }
        None => {
            send_disconnected(tx);
            None
        }
    }
}

fn send_disconnected(tx: &Sender<Event>) {
    let _ = tx.send(Event::Disconnected(
        "No AULA keyboard found. Check the cable, or that the udev rule is installed.".into(),
    ));
}

fn refresh(kb: &Keyboard, tx: &Sender<Event>) {
    match kb.link.read_config(PAGE_LEN) {
        Ok(page) => {
            let _ = tx.send(Event::Log(format!("GET_LED -> {} bytes", page.len())));
            let _ = tx.send(Event::Config(page));
        }
        Err(e) => {
            let _ = tx.send(Event::Error(format!(
                "reading the settings page failed: {e}"
            )));
        }
    }

    let leds = kb.profile.led_count();
    if leds > 0 {
        match kb.link.read_rgb_table(leds) {
            Ok(rgb) => {
                let _ = tx.send(Event::RgbTable(rgb));
            }
            Err(e) => {
                let _ = tx.send(Event::Log(format!("per-key colours unavailable: {e}")));
            }
        }
    }

    if kb.profile.show_power {
        match kb.link.power() {
            Ok(p) => {
                let _ = tx.send(Event::Power(p));
            }
            Err(e) => {
                let _ = tx.send(Event::Log(format!("battery unavailable: {e}")));
            }
        }
    }
}

fn with<F>(kb: &Option<Keyboard>, tx: &Sender<Event>, f: F)
where
    F: FnOnce(&Keyboard) -> std::io::Result<()>,
{
    match kb {
        None => send_disconnected(tx),
        Some(k) => {
            if let Err(e) = f(k) {
                let _ = tx.send(Event::Error(format!("{e}")));
            }
        }
    }
}
