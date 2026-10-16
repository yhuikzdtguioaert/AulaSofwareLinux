use std::io;
use std::time::Duration;

use crate::hid::HidDevice;

/// Payload bytes per package.
pub const PAGE: usize = 512;
/// Header bytes in front of the payload, including the report id.
pub const HEADER: usize = 8;
/// Full feature report size for this device family.
pub const REPORT_LEN: usize = HEADER + PAGE;

/// Delay the vendor driver inserts around every transfer.
pub const DEFAULT_DELAY: Duration = Duration::from_millis(20);
/// Delay before retrying a failed transfer.
const RETRY_DELAY: Duration = Duration::from_millis(70);
const RETRIES: u32 = 3;

/// Commands, as issued by `CDevG5KB`.
///
/// Writes and reads are separate opcodes; for the bulk data commands the read
/// opcode is the write opcode with bit 7 set, but the identity and battery
/// reads have their own numbers, so they are all listed explicitly.
pub mod cmd {
    /// Write the key matrix for one layer (parameter = layer index).
    pub const SET_MATRIX: u8 = 0x03;
    /// Read the key matrix for one layer (parameter = layer index).
    pub const GET_MATRIX: u8 = 0x83;
    /// Write the lighting / configuration page.
    pub const SET_LED: u8 = 0x04;
    /// Read the lighting / configuration page.
    pub const GET_LED: u8 = 0x84;
    /// Write the per-key RGB table (3 bytes per LED).
    pub const SET_RGB_TABLE: u8 = 0x06;
    /// Read the per-key RGB table.
    pub const GET_RGB_TABLE: u8 = 0x86;
    /// Reset the device to its factory configuration.
    pub const RESET: u8 = 0x11;

    /// Identity read; parameter 1 returns the 6-byte model password.
    pub const GET_INFO: u8 = 0x82;
    /// Parameter for [`GET_INFO`] that returns the model password.
    pub const INFO_PASSWORD: u8 = 0x01;
    /// Battery read; used with parameter 2.
    pub const GET_POWER: u8 = 0x87;
    /// Parameter for [`GET_POWER`].
    pub const POWER_PARAM: u8 = 0x02;
}

/// Builds one package header.
///
/// `payload` is copied in and the rest of the report is zero padded, which is
/// what the firmware expects — short reports are rejected.
pub fn frame(
    report_id: u8,
    command: u8,
    parameter: u8,
    packages: u8,
    index: u8,
    payload: &[u8],
) -> Vec<u8> {
    assert!(payload.len() <= PAGE, "payload must fit one package");
    let mut buf = vec![0u8; REPORT_LEN];
    let len = payload.len() as u16;
    buf[0] = report_id;
    buf[1] = command;
    buf[2] = parameter;
    buf[3] = 0;
    buf[4] = packages;
    buf[5] = index;
    buf[6] = len as u8;
    buf[7] = (len >> 8) as u8;
    buf[HEADER..HEADER + payload.len()].copy_from_slice(payload);
    buf
}

/// Number of packages a transfer of `len` bytes is split into.
pub fn package_count(len: usize) -> u8 {
    if len == 0 {
        0
    } else {
        len.div_ceil(PAGE) as u8
    }
}

/// Framing for the 2.4 GHz dongle (USB 3554:fa09).
///
/// The dongle has no 520-byte feature report. Commands travel as 20-byte
/// interrupt reports on report id 0x13, in chunks of at most 14 data bytes:
///
/// ```text
/// byte 0      0x13
/// byte 1      command           wired opcode with bit 6 set for reads, e.g.
///                               0x04 writes the settings page, 0x44 reads it
/// byte 2      total packages
/// byte 3      package index
/// byte 4      (operation << 4) | chunk length    operation 1 = write
/// byte 5..18  data
/// byte 19     sum of bytes 0..18, modulo 256
/// ```
///
/// A read is one request (no data); the keyboard streams every package back.
/// A write is acknowledged package by package. The keyboard dozes after a few
/// idle minutes, so a timeout usually means "asleep" rather than "broken".
pub mod wireless {
    use std::io;
    use std::time::{Duration, Instant};

    use crate::hid::HidDevice;

    pub const REPORT_ID: u8 = 0x13;
    pub const PACKET_LEN: usize = 20;
    /// Bytes of data per packet.
    pub const CHUNK: usize = 14;
    const REPLY_TIMEOUT: Duration = Duration::from_millis(1500);
    const DRAIN_TIMEOUT: Duration = Duration::from_millis(40);
    const RETRIES: u32 = 4;

    /// Wireless read opcodes.
    pub const GET_IDENTITY: u8 = 0x05;
    pub const GET_BATTERY: u8 = 0x4a;

    pub fn checksum(packet: &[u8]) -> u8 {
        packet[..PACKET_LEN - 1]
            .iter()
            .fold(0u8, |a, b| a.wrapping_add(*b))
    }

    /// Build one packet. `operation` is 0 for a request and 1 for a write.
    pub fn packet(
        command: u8,
        operation: u8,
        data: &[u8],
        packages: u8,
        index: u8,
    ) -> [u8; PACKET_LEN] {
        assert!(data.len() <= CHUNK, "chunk too large");
        let mut p = [0u8; PACKET_LEN];
        p[0] = REPORT_ID;
        p[1] = command;
        p[2] = packages;
        p[3] = index;
        p[4] = (operation << 4) | data.len() as u8;
        p[5..5 + data.len()].copy_from_slice(data);
        p[PACKET_LEN - 1] = checksum(&p);
        p
    }

    fn timed_out(what: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "no answer to {what} over the dongle — the keyboard may be asleep, press a key"
            ),
        )
    }

    /// Next well-formed packet for `command` (reads and acks echo the opcode,
    /// with bit 7 flagging an error on acks).
    fn next_reply(dev: &HidDevice, command: u8) -> io::Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + REPLY_TIMEOUT;
        while Instant::now() < deadline {
            match dev.read_report(deadline.saturating_duration_since(Instant::now()))? {
                None => return Ok(None),
                Some(r) if r.len() >= PACKET_LEN && r[0] == REPORT_ID => {
                    if r[1] & 0x7f == command & 0x7f && r[PACKET_LEN - 1] == checksum(&r) {
                        return Ok(Some(r));
                    }
                }
                Some(_) => {}
            }
        }
        Ok(None)
    }

    pub fn drain(dev: &HidDevice) {
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            if !matches!(dev.read_report(DRAIN_TIMEOUT), Ok(Some(_))) {
                break;
            }
        }
    }

    /// Issue read opcode `command` and reassemble the packages it streams back.
    pub fn read(dev: &HidDevice, command: u8) -> io::Result<Vec<u8>> {
        for _ in 0..RETRIES {
            drain(dev);
            dev.write_report(&packet(command, 0, &[], 1, 0))?;
            let mut chunks: Vec<Option<Vec<u8>>> = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                let Some(r) = next_reply(dev, command)? else {
                    break;
                };
                if r[1] & 0x80 != 0 {
                    return Err(io::Error::other("receiver rejected the read"));
                }
                let total = r[2] as usize;
                let (idx, len) = (r[3] as usize, (r[4] & 0x0f) as usize);
                if total == 0 || idx >= total || len > CHUNK {
                    continue;
                }
                if chunks.is_empty() {
                    chunks = vec![None; total];
                }
                if chunks.len() != total {
                    continue;
                }
                chunks[idx] = Some(r[5..5 + len].to_vec());
                if chunks.iter().all(Option::is_some) {
                    return Ok(chunks.into_iter().flatten().flatten().collect());
                }
            }
        }
        Err(timed_out(&format!("read 0x{command:02x}")))
    }

    /// Send `data` under write opcode `command`, waiting for each ack.
    pub fn write(dev: &HidDevice, command: u8, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        drain(dev);
        let packages = data.len().div_ceil(CHUNK);
        if packages > u8::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload too large",
            ));
        }
        for (index, chunk) in data.chunks(CHUNK).enumerate() {
            dev.write_report(&packet(command, 1, chunk, packages as u8, index as u8))?;
            // Opcodes with bit 7 set are fire-and-forget.
            if command & 0x80 != 0 {
                continue;
            }
            match next_reply(dev, command)? {
                None => {
                    return Err(timed_out(&format!(
                        "package {index} of write 0x{command:02x}"
                    )))
                }
                Some(ack) if ack[1] & 0x80 != 0 => {
                    return Err(io::Error::other(format!(
                        "keyboard rejected package {index} (checksum error)"
                    )))
                }
                Some(ack) if ack[3] as usize != index => {
                    return Err(io::Error::other(format!(
                        "ack index mismatch on package {index}"
                    )))
                }
                Some(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::net::UnixDatagram;

        #[test]
        fn ignores_corrupt_checksums_and_unrelated_packets() {
            let (socket, peer) = UnixDatagram::pair().unwrap();
            let dev = HidDevice::from_test_socket(socket);
            let mut corrupt = packet(0x44, 0, &[9], 1, 0);
            corrupt[19] ^= 1;
            peer.send(&corrupt).unwrap();
            peer.send(&packet(0x4a, 0, &[20], 1, 0)).unwrap();
            let expected = packet(0x44, 0, &[1, 2], 1, 0);
            peer.send(&expected).unwrap();
            assert_eq!(next_reply(&dev, 0x44).unwrap().unwrap(), expected);
        }

        #[test]
        fn unrelated_traffic_cannot_extend_the_reply_deadline() {
            let (socket, peer) = UnixDatagram::pair().unwrap();
            let dev = HidDevice::from_test_socket(socket);
            let producer = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(1750);
                while Instant::now() < deadline {
                    if peer.send(&packet(0x4a, 0, &[], 1, 0)).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            });
            let start = Instant::now();
            assert!(next_reply(&dev, 0x44).unwrap().is_none());
            assert!(start.elapsed() < Duration::from_secs(2));
            drop(dev);
            producer.join().unwrap();
        }
    }
}

const RGB_SLOTS: usize = 126;

fn rgb_planes(rgb: &[u8]) -> io::Result<Vec<u8>> {
    if rgb.len() != RGB_SLOTS * 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid per-key colour table",
        ));
    }
    let mut planes = vec![0; rgb.len()];
    for (slot, color) in rgb.chunks(3).enumerate() {
        for channel in 0..3 {
            planes[channel * RGB_SLOTS + slot] = color[channel];
        }
    }
    Ok(planes)
}

fn check_transfer_len(len: usize) -> io::Result<()> {
    if len > PAGE * u8::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transfer too large",
        ));
    }
    Ok(())
}

/// Report id of the dongle's vendor channel.
pub const WIRELESS_REPORT_ID: u8 = wireless::REPORT_ID;
/// Payload bytes (excluding the report id) of that report.
pub const WIRELESS_PAYLOAD: usize = wireless::PACKET_LEN - 1;

fn unsupported_wireless(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} is not available over the 2.4 GHz dongle; connect the cable"),
    )
}

/// A keyboard speaking this protocol.
pub struct Link {
    dev: HidDevice,
    report_id: u8,
    delay: Duration,
    wireless: bool,
}

impl Link {
    /// Pick the transport the candidate node calls for.
    pub fn for_candidate(dev: HidDevice, candidate: &crate::device::Candidate) -> Link {
        if candidate.wireless {
            Link::wireless(dev)
        } else {
            Link::new(dev, candidate.report_id)
        }
    }

    /// Link over the 2.4 GHz dongle.
    pub fn wireless(dev: HidDevice) -> Link {
        Link {
            dev,
            report_id: wireless::REPORT_ID,
            delay: DEFAULT_DELAY,
            wireless: true,
        }
    }

    pub fn is_wireless(&self) -> bool {
        self.wireless
    }

    /// `report_id` is normally 6; firmwares that do not identify as 0x18 use 9.
    /// [`report_id_for_firmware`] and the HID descriptor both tell you which.
    pub fn new(dev: HidDevice, report_id: u8) -> Link {
        Link {
            dev,
            report_id,
            delay: DEFAULT_DELAY,
            wireless: false,
        }
    }

    pub fn report_id(&self) -> u8 {
        self.report_id
    }

    /// `/dev/hidrawN` this link is bound to, for status messages.
    pub fn node_path(&self) -> String {
        self.dev.node.dev_path.display().to_string()
    }

    pub fn is_present(&self) -> bool {
        self.dev.node.sys_path.exists() && self.dev.node.dev_path.exists() && self.dev.is_present()
    }

    /// Send `data`, split into packages, with the given command/parameter.
    pub fn write(&self, command: u8, parameter: u8, data: &[u8]) -> io::Result<()> {
        if self.wireless {
            return self.write_wireless(command, parameter, data);
        }
        let packages = package_count(data.len());
        for (index, chunk) in data.chunks(PAGE).enumerate() {
            let buf = frame(
                self.report_id,
                command,
                parameter,
                packages,
                index as u8,
                chunk,
            );
            std::thread::sleep(self.delay);
            self.set_feature_retrying(&buf)?;
        }
        Ok(())
    }

    /// Request `len` bytes and return them.
    pub fn read(&self, command: u8, parameter: u8, len: usize) -> io::Result<Vec<u8>> {
        if self.wireless {
            return self.read_wireless(command, parameter, len);
        }
        check_transfer_len(len)?;
        let packages = package_count(len);
        let mut out = Vec::with_capacity(len);
        for index in 0..packages {
            let chunk = (len - out.len()).min(PAGE);
            let request = frame(self.report_id, command, parameter, packages, index, &[]);
            // The length field describes the reply, so patch it in directly.
            let mut request = request;
            request[6] = chunk as u8;
            request[7] = (chunk >> 8) as u8;

            std::thread::sleep(self.delay);
            self.set_feature_retrying(&request)?;

            std::thread::sleep(self.delay);
            let mut reply = vec![0u8; REPORT_LEN];
            reply[0] = self.report_id;
            let received = self.dev.get_feature(&mut reply)?;
            if received < HEADER + chunk {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short HID feature report",
                ));
            }
            if reply[0] != self.report_id
                || reply[1] != command
                || reply[2] != parameter
                || reply[5] != index
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "device answered command 0x{:02x}, expected 0x{command:02x}",
                        reply[1]
                    ),
                ));
            }
            out.extend_from_slice(&reply[HEADER..HEADER + chunk]);
        }
        Ok(out)
    }

    /// Wired commands mapped onto the dongle's opcodes. Only what has been
    /// seen working over the dongle is mapped; everything else is refused
    /// rather than guessed at.
    fn read_wireless(&self, command: u8, parameter: u8, len: usize) -> io::Result<Vec<u8>> {
        let (opcode, want) = match (command, parameter) {
            (cmd::GET_INFO, cmd::INFO_PASSWORD) => (wireless::GET_IDENTITY, 6),
            (cmd::GET_POWER, _) => (wireless::GET_BATTERY, 2),
            (cmd::GET_LED, 0) => (0x44, len),
            (cmd::GET_MATRIX, 0) => (0x41, len),
            _ => return Err(unsupported_wireless(&format!("read 0x{command:02x}"))),
        };
        let mut data = wireless::read(&self.dev, opcode)?;
        if command == cmd::GET_MATRIX
            && !data.is_empty()
            && data.len() <= want
            && data.len().is_multiple_of(4)
        {
            return Ok(data);
        }
        if data.len() < want {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("dongle returned {} of {want} bytes", data.len()),
            ));
        }
        data.truncate(want);
        Ok(data)
    }

    fn write_wireless(&self, command: u8, parameter: u8, data: &[u8]) -> io::Result<()> {
        match (command, parameter) {
            (cmd::SET_LED, 0) => wireless::write(&self.dev, cmd::SET_LED, data),
            (cmd::SET_MATRIX, _) => Err(unsupported_wireless("key assignments")),
            (cmd::SET_RGB_TABLE, _) => Err(unsupported_wireless("per-key colour")),
            (cmd::RESET, _) => Err(unsupported_wireless("factory reset")),
            _ => Err(unsupported_wireless(&format!("write 0x{command:02x}"))),
        }
    }

    fn set_feature_retrying(&self, buf: &[u8]) -> io::Result<()> {
        let mut last = None;
        for _ in 0..RETRIES {
            match self.dev.set_feature(buf) {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(RETRY_DELAY);
                }
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("set_feature failed")))
    }

    /// The 6 identity bytes. Every model in this family shares USB
    /// 258a:010c, so this is what says *which* keyboard is attached; it is
    /// matched against `Psd=` in the device profile.
    pub fn password(&self) -> io::Result<[u8; 6]> {
        let v = self.read(cmd::GET_INFO, cmd::INFO_PASSWORD, 6)?;
        let mut out = [0u8; 6];
        out.copy_from_slice(&v[..6]);
        Ok(out)
    }

    /// Battery state, for the wireless models. The vendor driver logs two
    /// values here; the first is the charge percentage.
    pub fn power(&self) -> io::Result<Vec<u8>> {
        self.read(cmd::GET_POWER, cmd::POWER_PARAM, 2)
    }

    /// Read the lighting / configuration page.
    pub fn read_config(&self, len: usize) -> io::Result<Vec<u8>> {
        self.read(cmd::GET_LED, 0, len)
    }

    /// Write the lighting / configuration page.
    pub fn write_config(&self, page: &[u8]) -> io::Result<()> {
        if page.len() != crate::config_page::PAGE_LEN || !page.ends_with(&[0x5a, 0xa5]) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid settings page",
            ));
        }
        self.write(cmd::SET_LED, 0, page)
    }

    /// Per-key colours use three planes of 126 slots, independent of key count.
    pub fn write_rgb_table(&self, rgb: &[u8]) -> io::Result<()> {
        self.write(cmd::SET_RGB_TABLE, 0, &rgb_planes(rgb)?)
    }

    pub fn read_rgb_table(&self) -> io::Result<Vec<u8>> {
        let planes = self.read(cmd::GET_RGB_TABLE, 0, RGB_SLOTS * 3)?;
        let mut rgb = vec![0; RGB_SLOTS * 3];
        for (slot, color) in rgb.chunks_mut(3).enumerate() {
            for channel in 0..3 {
                color[channel] = planes[channel * RGB_SLOTS + slot];
            }
        }
        Ok(rgb)
    }

    /// Key matrix for one Fn layer; 4 bytes per key slot.
    pub fn write_matrix(&self, layer: u8, matrix: &[u8]) -> io::Result<()> {
        self.write(cmd::SET_MATRIX, layer, matrix)
    }

    pub fn read_matrix(&self, layer: u8, slots: usize) -> io::Result<Vec<u8>> {
        self.read(cmd::GET_MATRIX, layer, slots * 4)
    }

    /// Restore the factory configuration.
    pub fn reset(&self) -> io::Result<()> {
        self.write(cmd::RESET, 0, &[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_key_colours_use_planes_and_preserve_all_slots() {
        let mut rgb = vec![0; RGB_SLOTS * 3];
        rgb[..3].copy_from_slice(&[1, 2, 3]);
        rgb[125 * 3..].copy_from_slice(&[4, 5, 6]);
        let planes = rgb_planes(&rgb).unwrap();
        assert_eq!((planes[0], planes[126], planes[252]), (1, 2, 3));
        assert_eq!((planes[125], planes[251], planes[377]), (4, 5, 6));
        assert!(rgb_planes(&rgb[..270]).is_err());
    }

    #[test]
    fn oversized_transfers_do_not_wrap_the_package_count() {
        assert!(check_transfer_len(PAGE * 255).is_ok());
        assert!(check_transfer_len(PAGE * 255 + 1).is_err());
    }

    #[test]
    fn header_matches_the_vendor_layout() {
        let f = frame(6, cmd::SET_LED, 0, 1, 0, &[0xaa, 0xbb]);
        assert_eq!(f.len(), 520);
        assert_eq!(&f[..8], &[6, 0x04, 0, 0, 1, 0, 2, 0]);
        assert_eq!(&f[8..10], &[0xaa, 0xbb]);
        assert!(f[10..].iter().all(|b| *b == 0), "tail must be zero padded");
    }

    #[test]
    fn password_request_matches_the_disassembly() {
        // OemDrv.exe builds exactly this: 06 82 01 00 01 00 06 00
        let f = frame(6, cmd::GET_INFO, cmd::INFO_PASSWORD, 1, 0, &[]);
        let mut f = f;
        f[6] = 6;
        assert_eq!(&f[..8], &[0x06, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06, 0x00]);
    }

    #[test]
    fn wireless_packets_carry_a_checksum() {
        // Captured from a live F99 Pro: battery request and its answer.
        let req = wireless::packet(0x4a, 0, &[], 1, 0);
        assert_eq!(req[..5], [0x13, 0x4a, 1, 0, 0]);
        assert_eq!(req[19], 0x13 + 0x4a + 1);
        let reply = [
            0x13, 0x4a, 0x01, 0x00, 0x02, 0x47, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xa8,
        ];
        assert_eq!(wireless::checksum(&reply), 0xa8);
    }

    #[test]
    fn wireless_write_packet_layout() {
        let p = wireless::packet(0x04, 1, &[1, 2, 3], 10, 2);
        assert_eq!(p[..8], [0x13, 0x04, 10, 2, 0x13, 1, 2, 3]);
        assert_eq!(wireless::checksum(&p), p[19]);
    }

    #[test]
    fn splits_into_512_byte_packages() {
        assert_eq!(package_count(0), 0);
        assert_eq!(package_count(1), 1);
        assert_eq!(package_count(512), 1);
        assert_eq!(package_count(513), 2);
        assert_eq!(package_count(2048), 4);
    }

    #[test]
    fn last_package_carries_the_remainder() {
        let data: Vec<u8> = (0..600).map(|i| i as u8).collect();
        let packages = package_count(data.len());
        let chunks: Vec<&[u8]> = data.chunks(PAGE).collect();
        assert_eq!(packages, 2);
        let second = frame(6, cmd::SET_MATRIX, 1, packages, 1, chunks[1]);
        assert_eq!(second[4], 2, "total packages");
        assert_eq!(second[5], 1, "package index");
        assert_eq!(u16::from_le_bytes([second[6], second[7]]), 88);
    }
}
