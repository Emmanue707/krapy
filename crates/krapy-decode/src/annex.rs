pub(crate) fn nal_type_at(data: &[u8], mut i: usize) -> Option<u8> {
    if data[i..].starts_with(&[0, 0, 0, 1]) {
        i += 4;
    } else if data[i..].starts_with(&[0, 0, 1]) {
        i += 3;
    } else {
        return None;
    }
    data.get(i).map(|b| b & 0x1f)
}

pub(crate) fn parameter_sets_only(data: &[u8]) -> bool {
    let mut i = 0;
    let mut saw = false;
    while i + 3 < data.len() {
        let Some(ty) = nal_type_at(data, i) else {
            i += 1;
            continue;
        };
        if ty != 7 && ty != 8 {
            return false;
        }
        saw = true;
        i += 1;
    }
    saw
}

pub(crate) fn has_idr(data: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 < data.len() {
        if let Some(ty) = nal_type_at(data, i)
            && ty == 5
        {
            return true;
        }
        i += 1;
    }
    false
}
