use std::io;

const DEFAULT_KNOB: [u8; 4] = [0x07, 0, 0, 0x1d];

// Find the actual knob assignment; its slot varies between layouts.
pub fn find_knob(matrix: &[u8]) -> Option<usize> {
    let mut slots = matrix
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
    let start = slot
        .checked_mul(4)
        .ok_or_else(|| io::Error::other("invalid knob slot"))?;
    let end = start
        .checked_add(4)
        .ok_or_else(|| io::Error::other("invalid knob slot"))?;
    let limit = matrix.len() - usize::from(matrix.ends_with(&[0x5a, 0xa5])) * 2;
    if end > limit {
        return Err(io::Error::other("invalid knob slot"));
    }
    let entry = matrix
        .get_mut(start..end)
        .ok_or_else(|| io::Error::other("invalid knob slot"))?;
    entry.copy_from_slice(&action);
    Ok(())
}

fn setting_path(profile: &crate::profile::DeviceProfile) -> Option<std::path::PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    let identity = profile
        .password
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Some(root.join("aula").join(format!("knob-{identity}")))
}

pub fn locate_knob(
    profile: &crate::profile::DeviceProfile,
    matrix: &[u8],
) -> Option<(usize, [u8; 4])> {
    if let Some(slot) = find_knob(matrix) {
        return Some((slot, DEFAULT_KNOB));
    }
    let text = std::fs::read_to_string(setting_path(profile)?).ok()?;
    let values = text
        .split_whitespace()
        .map(str::parse::<usize>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let [slot, a, b, c, d] = values.as_slice() else {
        return None;
    };
    let action = [
        u8::try_from(*a).ok()?,
        u8::try_from(*b).ok()?,
        u8::try_from(*c).ok()?,
        u8::try_from(*d).ok()?,
    ];
    let start = slot.checked_mul(4)?;
    (matrix.get(start..start.checked_add(4)?) == Some(action.as_slice())).then_some((*slot, action))
}

pub fn remember_knob(
    profile: &crate::profile::DeviceProfile,
    slot: usize,
    action: [u8; 4],
) -> io::Result<()> {
    let path = setting_path(profile)
        .ok_or_else(|| io::Error::other("configuration directory unavailable"))?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension("tmp");
    std::fs::write(
        &temporary,
        format!(
            "{slot} {} {} {} {}\n",
            action[0], action[1], action[2], action[3]
        ),
    )?;
    std::fs::rename(temporary, path)
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
    fn refuses_ambiguous_and_missing_knobs() {
        assert_eq!(find_knob(&[0; 512]), None);
        assert_eq!(find_knob(&DEFAULT_KNOB.repeat(2)), None);
    }
}
