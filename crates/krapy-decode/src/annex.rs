// (start code offset, nal type) for every nal in an annex-b buffer
pub(crate) fn nal_starts(data: &[u8]) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                out.push((i, data[i + 3] & 0x1f));
                i += 3;
                continue;
            }
            if data[i + 2] == 0 && i + 4 < data.len() && data[i + 3] == 1 {
                out.push((i, data[i + 4] & 0x1f));
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    out
}
