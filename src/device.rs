use crate::hid::{self, HidDevice, HidNode};
use crate::profile::DeviceProfile;
use crate::proto::{self, Link, REPORT_LEN};

/// A hidraw node that looks like it speaks the vendor protocol.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub node: HidNode,
    /// Report id carrying the 519-byte vendor feature report.
    pub report_id: u8,
    /// True for the 2.4 GHz dongle, which carries the same commands as 20-byte
    /// interrupt reports instead of 520-byte feature reports.
    pub wireless: bool,
}

/// Does this node expose the vendor command channel?
pub fn candidate_for(node: &HidNode) -> Option<Candidate> {
    // The command channel is the feature report whose payload is one page plus
    // the 7 header bytes that follow the report id.
    let want = REPORT_LEN - 1;
    if let Some((id, _)) = node.feature_reports.iter().find(|(_, len)| *len == want) {
        return Some(Candidate {
            node: node.clone(),
            report_id: *id,
            wireless: false,
        });
    }
    // The dongle has no big feature report; its vendor channel is a 19-byte
    // output report.
    node.output_reports
        .iter()
        .find(|(id, len)| *id == proto::WIRELESS_REPORT_ID && *len == proto::WIRELESS_PAYLOAD)
        .map(|(id, _)| Candidate {
            node: node.clone(),
            report_id: *id,
            wireless: true,
        })
}

/// All attached nodes that expose the vendor command channel.
///
/// `profiles` is used to restrict the search to known USB ids; pass an empty
/// slice to accept any device that has the right report shape.
pub fn find_candidates(profiles: &[DeviceProfile]) -> Vec<Candidate> {
    let ids: Vec<(u16, u16)> = profiles
        .iter()
        .flat_map(|p| {
            let mut v = vec![(p.vid, p.pid)];
            if let (Some(vid), Some(pid)) = (p.wireless_vid, p.wireless_pid) {
                v.push((vid, pid));
            }
            v
        })
        .collect();

    hid::enumerate()
        .into_iter()
        .filter(|n| {
            ids.is_empty()
                || ids.contains(&(n.vid, n.pid))
                || n.name.to_ascii_lowercase().contains("aula")
        })
        .filter_map(|n| candidate_for(&n))
        .collect()
}

/// An open keyboard together with the profile that describes it.
pub struct Keyboard {
    pub link: Link,
    pub profile: DeviceProfile,
    pub password: [u8; 6],
}

/// Open `candidate` and work out which profile it is.
pub fn open(candidate: &Candidate, profiles: &[DeviceProfile]) -> anyhow::Result<Keyboard> {
    let dev = HidDevice::open(&candidate.node)
        .map_err(|e| anyhow::anyhow!("{}", hid::permission_hint(&candidate.node.dev_path, &e)))?;
    let link = Link::for_candidate(dev, candidate);

    let password = link
        .password()
        .map_err(|e| anyhow::anyhow!("reading the device password failed: {e}"))?;

    let profile = profiles
        .iter()
        .find(|p| p.password == password)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no profile matches the device password {}. \
                 Add a profile for this model under assets/devices/",
                fmt_password(&password)
            )
        })?;

    Ok(Keyboard {
        link,
        profile,
        password,
    })
}

pub fn fmt_password(p: &[u8; 6]) -> String {
    p.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Best-effort scan: returns the first keyboard that identifies successfully,
/// along with any errors met on the way (useful for the status line).
pub fn autodetect(profiles: &[DeviceProfile]) -> (Option<Keyboard>, Vec<String>) {
    let mut problems = Vec::new();
    for c in find_candidates(profiles) {
        match open(&c, profiles) {
            Ok(kb) => return (Some(kb), problems),
            Err(e) => problems.push(format!("{}: {e}", c.node.dev_path.display())),
        }
    }
    (None, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn node(features: Vec<(u8, usize)>) -> HidNode {
        HidNode {
            dev_path: PathBuf::from("/dev/hidraw9"),
            sys_path: PathBuf::from("/sys/class/hidraw/hidraw9"),
            vid: 0x258a,
            pid: 0x010c,
            name: "BY Tech Gaming Keyboard".into(),
            feature_reports: features,
            output_reports: vec![],
        }
    }

    #[test]
    fn picks_the_node_with_the_vendor_report() {
        let c = candidate_for(&node(vec![(5, 5), (6, 519)])).expect("should match");
        assert_eq!(c.report_id, 6);
    }

    #[test]
    fn ignores_the_plain_keyboard_interface() {
        assert!(candidate_for(&node(vec![])).is_none());
        assert!(candidate_for(&node(vec![(5, 5)])).is_none());
    }

    #[test]
    fn accepts_the_alternate_report_id() {
        let c = candidate_for(&node(vec![(9, 519)])).expect("should match");
        assert_eq!(c.report_id, 9);
    }

    #[test]
    fn accepts_the_dongle_vendor_interface() {
        let mut n = node(vec![(6, 7)]);
        n.vid = 0x3554;
        n.pid = 0xfa09;
        n.output_reports = vec![(0x13, 19)];
        let c = candidate_for(&n).expect("should match");
        assert!(c.wireless);
        assert_eq!(c.report_id, 0x13);
    }

    #[test]
    fn ignores_the_dongle_keyboard_interface() {
        let mut n = node(vec![]);
        n.output_reports = vec![(1, 1)];
        assert!(candidate_for(&n).is_none());
    }

    #[test]
    fn formats_passwords_like_the_profiles() {
        assert_eq!(fmt_password(&[3, 0, 0, 0, 0, 0xcd]), "03,00,00,00,00,cd");
    }
}
