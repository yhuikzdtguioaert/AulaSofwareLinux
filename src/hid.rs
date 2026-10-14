//! Linux `hidraw` transport.
//!
//! The vendor driver talks to the keyboard with HID *feature* reports on a
//! vendor-defined usage page (0xFF00). On Linux that is `/dev/hidrawN` plus the
//! `HIDIOCGFEATURE` / `HIDIOCSFEATURE` ioctls — no libusb, no kernel module.
//!
//! A keyboard exposes several hidraw nodes (the boot keyboard, the consumer
//! controls, the vendor collection). Only one of them carries the feature
//! reports we need, so the descriptor of each candidate is parsed and the node
//! that actually declares them is selected.

use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

const HIDRAW_CLASS: &str = "/sys/class/hidraw";

// include/uapi/asm-generic/ioctl.h
const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_READ_WRITE: u32 = 3; // _IOC_WRITE | _IOC_READ

const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> libc::c_ulong {
    ((dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT))
        as libc::c_ulong
}

fn hidioc_set_feature(len: usize) -> libc::c_ulong {
    ioc(IOC_READ_WRITE, b'H' as u32, 0x06, len as u32)
}

fn hidioc_get_feature(len: usize) -> libc::c_ulong {
    ioc(IOC_READ_WRITE, b'H' as u32, 0x07, len as u32)
}

/// One `/dev/hidrawN` node together with what its descriptor declares.
#[derive(Debug, Clone)]
pub struct HidNode {
    pub dev_path: PathBuf,
    pub sys_path: PathBuf,
    pub vid: u16,
    pub pid: u16,
    pub name: String,
    /// `(report id, feature payload size in bytes)`, excluding the id byte.
    pub feature_reports: Vec<(u8, usize)>,
    /// `(report id, payload size in bytes)` of every declared output report.
    pub output_reports: Vec<(u8, usize)>,
}

impl HidNode {
    pub fn feature_len(&self, report_id: u8) -> Option<usize> {
        self.feature_reports
            .iter()
            .find(|(id, _)| *id == report_id)
            .map(|(_, len)| *len)
    }
}

/// Enumerate every hidraw node on the system.
pub fn enumerate() -> Vec<HidNode> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(HIDRAW_CLASS) else {
        return out;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for sys in entries {
        let Some(node) = read_node(&sys) else {
            continue;
        };
        out.push(node);
    }
    out
}

fn read_node(sys: &Path) -> Option<HidNode> {
    let name = sys.file_name()?.to_string_lossy().into_owned();
    let uevent = std::fs::read_to_string(sys.join("device/uevent")).ok()?;
    let mut vid = 0u16;
    let mut pid = 0u16;
    let mut hid_name = String::new();
    for line in uevent.lines() {
        if let Some(v) = line.strip_prefix("HID_ID=") {
            // bus:vendor:product, each zero-padded hex
            let mut it = v.split(':');
            it.next();
            vid = it
                .next()
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                .unwrap_or(0) as u16;
            pid = it
                .next()
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                .unwrap_or(0) as u16;
        } else if let Some(v) = line.strip_prefix("HID_NAME=") {
            hid_name = v.to_string();
        }
    }
    let desc = std::fs::read(sys.join("device/report_descriptor")).unwrap_or_default();
    Some(HidNode {
        dev_path: PathBuf::from("/dev").join(&name),
        sys_path: sys.to_path_buf(),
        vid,
        pid,
        name: hid_name,
        feature_reports: feature_reports(&desc),
        output_reports: output_reports(&desc),
    })
}

/// Walk a HID report descriptor and return the declared feature report sizes.
///
/// Only the items that matter here are tracked: report id, report size (bits
/// per field), report count (number of fields) and the Feature main item.
pub fn feature_reports(desc: &[u8]) -> Vec<(u8, usize)> {
    reports(desc, 0xb)
}

/// Like [`feature_reports`], for output reports.
pub fn output_reports(desc: &[u8]) -> Vec<(u8, usize)> {
    reports(desc, 0x9)
}

/// `main_tag` is the HID main item to total up: 0x9 Output, 0xB Feature.
fn reports(desc: &[u8], main_tag: u8) -> Vec<(u8, usize)> {
    let mut out: Vec<(u8, usize)> = Vec::new();
    let mut report_id: u8 = 0;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;
    let mut bits: Vec<(u8, u64)> = Vec::new();

    let mut i = 0usize;
    while i < desc.len() {
        let head = desc[i];
        i += 1;
        if head == 0xfe {
            // Long item: [len][tag][data...]
            let len = *desc.get(i).unwrap_or(&0) as usize;
            i += 2 + len;
            continue;
        }
        let size = match head & 0x03 {
            0 => 0usize,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if i + size > desc.len() {
            break;
        }
        let mut data: u32 = 0;
        for (n, b) in desc[i..i + size].iter().enumerate() {
            data |= (*b as u32) << (8 * n);
        }
        i += size;

        let ty = (head >> 2) & 0x03;
        let tag = head >> 4;
        match (ty, tag) {
            // Global items
            (1, 0x8) => report_id = data as u8,
            (1, 0x7) => report_size = data,
            (1, 0x9) => report_count = data,
            // Main items: 0x8 Input, 0x9 Output, 0xB Feature
            (0, t) if t == main_tag => {
                let add = (report_size as u64) * (report_count as u64);
                match bits.iter_mut().find(|(id, _)| *id == report_id) {
                    Some((_, total)) => *total += add,
                    None => bits.push((report_id, add)),
                }
            }
            // Collection / End collection reset nothing we care about.
            _ => {}
        }
    }

    for (id, total_bits) in bits {
        out.push((id, total_bits.div_ceil(8) as usize));
    }
    out.sort_by_key(|(id, _)| *id);
    out
}

/// An opened hidraw node.
pub struct HidDevice {
    file: File,
    pub node: HidNode,
}

impl HidDevice {
    pub fn open(node: &HidNode) -> io::Result<HidDevice> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&node.dev_path)?;
        Ok(HidDevice {
            file,
            node: node.clone(),
        })
    }

    fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// Send an output report (`write(2)`); `buf[0]` is the report id.
    pub fn write_report(&self, buf: &[u8]) -> io::Result<usize> {
        let ret = unsafe { libc::write(self.fd(), buf.as_ptr() as *const libc::c_void, buf.len()) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }

    /// Wait up to `timeout` for one input report. `None` on timeout.
    pub fn read_report(&self, timeout: std::time::Duration) -> io::Result<Option<Vec<u8>>> {
        let mut pfd = libc::pollfd {
            fd: self.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            return Ok(None);
        }
        let mut buf = [0u8; 64];
        let n = unsafe { libc::read(self.fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(buf[..n as usize].to_vec()))
    }

    /// `HIDIOCSFEATURE`. `buf[0]` must be the report id.
    pub fn set_feature(&self, buf: &[u8]) -> io::Result<usize> {
        let ret = unsafe {
            libc::ioctl(
                self.fd(),
                hidioc_set_feature(buf.len()),
                buf.as_ptr() as *mut libc::c_void,
            )
        };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }

    /// `HIDIOCGFEATURE`. `buf[0]` must be set to the report id on entry and is
    /// overwritten with the reply.
    pub fn get_feature(&self, buf: &mut [u8]) -> io::Result<usize> {
        let ret = unsafe {
            libc::ioctl(
                self.fd(),
                hidioc_get_feature(buf.len()),
                buf.as_mut_ptr() as *mut libc::c_void,
            )
        };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }
}

/// Human-readable hint for the most common failure: no permission on the node.
pub fn permission_hint(path: &Path, err: &io::Error) -> String {
    if err.kind() == io::ErrorKind::PermissionDenied {
        format!(
            "{}: permission denied.\n\
             Install the udev rule and replug the keyboard:\n\
             \x20 sudo cp udev/60-aula.rules /etc/udev/rules.d/ && sudo udevadm control --reload",
            path.display()
        )
    } else {
        format!("{}: {err}", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real descriptor read from an AULA F75 (258a:010c, second interface).
    const F75_IFACE1: &[u8] = &[
        0x06, 0x01, 0x00, 0x09, 0x80, 0xa1, 0x01, 0x85, 0x01, 0x19, 0x81, 0x29, 0x83, 0x15, 0x00,
        0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05, 0x81, 0x01, 0xc0,
        0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x02, 0x19, 0x00, 0x2a, 0xff, 0x02, 0x15, 0x00,
        0x26, 0xff, 0x7f, 0x95, 0x01, 0x75, 0x10, 0x81, 0x00, 0xc0, 0x06, 0x00, 0xff, 0x09, 0x01,
        0xa1, 0x01, 0x85, 0x03, 0x15, 0x00, 0x26, 0xff, 0x00, 0x09, 0x2f, 0x75, 0x08, 0x95, 0x03,
        0x81, 0x02, 0xc0, 0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x04, 0x05, 0x07, 0x19, 0x04,
        0x29, 0x70, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x78, 0x81, 0x02, 0xc0, 0x06, 0x00,
        0xff, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x05, 0x15, 0x00, 0x26, 0xff, 0x00, 0x19, 0x01, 0x29,
        0x02, 0x75, 0x08, 0x95, 0x05, 0xb1, 0x02, 0xc0, 0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01,
        0x85, 0x06, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x07, 0x09, 0x02, 0x81, 0x00,
        0x15, 0x00, 0x26, 0xff, 0x00, 0x19, 0x01, 0x29, 0x02, 0x75, 0x08, 0x96, 0x07, 0x02, 0xb1,
        0x02, 0xc0,
    ];

    #[test]
    fn finds_the_vendor_feature_reports() {
        let f = feature_reports(F75_IFACE1);
        assert_eq!(f, vec![(5, 5), (6, 519)]);
    }

    /// Vendor interface of the 2.4 GHz dongle (3554:fa09, second interface),
    /// trimmed to the collections that matter.
    const DONGLE_IFACE1: &[u8] = &[
        0x06, 0x02, 0xff, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x13, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75,
        0x08, 0x95, 0x13, 0x09, 0x02, 0x81, 0x00, 0x09, 0x02, 0x91, 0x00, 0xc0, 0x06, 0x04, 0xff,
        0x09, 0x02, 0xa1, 0x02, 0x85, 0x06, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x07,
        0xb1, 0x02, 0xc0,
    ];

    #[test]
    fn dongle_has_a_19_byte_output_report_and_no_big_feature_report() {
        assert_eq!(output_reports(DONGLE_IFACE1), vec![(0x13, 19)]);
        assert_eq!(feature_reports(DONGLE_IFACE1), vec![(6, 7)]);
    }

    #[test]
    fn boot_keyboard_declares_no_feature_reports() {
        let boot = &[
            0x05u8, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
            0x25, 0x01, 0x95, 0x08, 0x75, 0x01, 0x81, 0x02, 0xc0,
        ];
        assert!(feature_reports(boot).is_empty());
    }

    #[test]
    fn ioctl_numbers_match_the_kernel_macros() {
        // HIDIOCSFEATURE(len) = _IOC(_IOC_WRITE|_IOC_READ, 'H', 0x06, len)
        assert_eq!(hidioc_set_feature(520), 0xC2084806);
        assert_eq!(hidioc_get_feature(520), 0xC2084807);
    }
}
