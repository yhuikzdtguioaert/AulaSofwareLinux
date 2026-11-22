use std::io;

const DEFAULT_KNOB: [u8; 4] = [0x07, 0, 0, 0x1d];
pub const MATRIX_TRAILER: [u8; 4] = [0, 0, 0x5a, 0xa5];

// Find the actual knob assignment; its slot varies between layouts.
pub fn find_knob(matrix: &[u8]) -> Option<usize> {
    let limit = matrix.len() - usize::from(matrix.ends_with(&MATRIX_TRAILER)) * 4;
    let mut slots = matrix[..limit]
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, entry)| **entry == DEFAULT_KNOB)
        .map(|(slot, _)| slot);
    let slot = slots.next()?;
    slots.next().is_none().then_some(slot)
}

pub fn set_knob(matrix: &mut [u8], slot: usize, action: [u8; 4]) -> io::Result<()> {
    if !crate::worker::KNOB_ACTIONS
        .iter()
        .any(|(_, allowed)| *allowed == action)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid knob action",
        ));
    }
    let range = slot_range(matrix, slot).ok_or_else(|| io::Error::other("invalid knob slot"))?;
    matrix[range].copy_from_slice(&action);
    Ok(())
}

fn slot_range(matrix: &[u8], slot: usize) -> Option<std::ops::Range<usize>> {
    let start = slot.checked_mul(4)?;
    let end = start.checked_add(4)?;
    let limit = matrix.len() - usize::from(matrix.ends_with(&MATRIX_TRAILER)) * 4;
    (end <= limit).then_some(start..end)
}

pub fn locate_knob(
    profile: &crate::profile::DeviceProfile,
    matrix: &[u8],
) -> Option<(usize, [u8; 4])> {
    let wheel = profile
        .keys
        .iter()
        .find(|key| key.name.eq_ignore_ascii_case("wheel"))?;
    // Wheel has no LED or geometry. The last field of its INI row is its
    // matrix slot, so it remains identifiable even after an external remap.
    let slot = usize::try_from(wheel.led)
        .ok()
        .filter(|slot| slot_range(matrix, *slot).is_some())
        .or_else(|| find_knob(matrix))?;
    let range = slot_range(matrix, slot)?;
    Some((slot, matrix[range].try_into().ok()?))
}

/// Describe an existing assignment even when it isn't one of the presets.
pub fn describe([kind, a, b, c]: [u8; 4]) -> String {
    match kind {
        0 => {
            let modifiers = [
                "Ctrl", "Shift", "Alt", "Super", "RCtrl", "RShift", "RAlt", "RSuper",
            ];
            let mut parts: Vec<String> = modifiers
                .iter()
                .enumerate()
                .filter(|(bit, _)| a & (1 << bit) != 0)
                .map(|(_, name)| (*name).into())
                .collect();
            parts.push(format!("Key 0x{c:02X}"));
            parts.join(" + ")
        }
        1 => format!("Command {a}"),
        2 => format!("Media usage 0x{:04X}", u16::from_be_bytes([b, c])),
        3 => format!("Macro {}", u16::from(c) + 1),
        5 => format!("Mouse buttons 0x{a:02X}"),
        7 => format!("Keyboard function 0x{c:02X}"),
        8 => format!("Keyboard function {a:02X} {b:02X} {c:02X}"),
        0x0d => "Fn".into(),
        _ => format!("Assignment {kind:02X} {a:02X} {b:02X} {c:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_knob_in_different_layouts_and_preserves_other_keys() {
        for slot in [84, 91, 108] {
            let mut matrix = vec![0; 512];
            matrix[slot * 4..slot * 4 + 4].copy_from_slice(&DEFAULT_KNOB);
            assert_eq!(find_knob(&matrix), Some(slot));
            let before = matrix.clone();
            set_knob(&mut matrix, slot, [2, 0, 0, 0xcd]).unwrap();
            assert_eq!(&matrix[..slot * 4], &before[..slot * 4]);
            assert_eq!(&matrix[slot * 4 + 4..], &before[slot * 4 + 4..]);
        }
    }

    #[test]
    fn finds_a_remapped_f99_pro_knob_and_protects_the_matrix_trailer() {
        let profile = crate::profile::DeviceProfile::load(std::path::Path::new(
            "assets/devices/f99-pro-f99pro",
        ))
        .unwrap();
        let mut matrix = vec![0; 512];
        matrix[508..].copy_from_slice(&MATRIX_TRAILER);
        let mute = [2, 0, 0, 0xe2];
        matrix[432..436].copy_from_slice(&mute);
        assert_eq!(locate_knob(&profile, &matrix), Some((108, mute)));
        assert!(set_knob(&mut matrix, 127, DEFAULT_KNOB).is_err());
        assert_eq!(matrix[508..], MATRIX_TRAILER);
        set_knob(&mut matrix, 108, DEFAULT_KNOB).unwrap();
        assert_eq!(locate_knob(&profile, &matrix), Some((108, DEFAULT_KNOB)));
        assert_eq!(describe([0, 0x60, 0, 4]), "RShift + RAlt + Key 0x04");
        assert_eq!(describe([2, 0, 0x02, 0x23]), "Media usage 0x0223");
    }

    #[test]
    fn refuses_ambiguous_and_missing_knobs() {
        assert_eq!(find_knob(&[0; 512]), None);
        assert_eq!(find_knob(&DEFAULT_KNOB.repeat(2)), None);
    }
}
