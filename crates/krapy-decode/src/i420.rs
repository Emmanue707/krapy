pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub stride_y: usize,
    pub stride_uv: usize,
}

pub(crate) fn pack_yuv420p(
    y: &[u8],
    u: &[u8],
    v: &[u8],
    width: u32,
    height: u32,
    y_stride: usize,
    uv_stride: usize,
) -> Frame {
    let w = width as usize;
    let h = height as usize;
    let stride_y = w;
    let stride_uv = w / 2;
    let mut out_y = vec![0u8; stride_y * h];
    let mut out_u = vec![0u8; stride_uv * (h / 2)];
    let mut out_v = vec![0u8; stride_uv * (h / 2)];
    copy_plane(y, y_stride, &mut out_y, stride_y, w, h);
    copy_plane(u, uv_stride, &mut out_u, stride_uv, w / 2, h / 2);
    copy_plane(v, uv_stride, &mut out_v, stride_uv, w / 2, h / 2);
    Frame {
        width,
        height,
        y: out_y,
        u: out_u,
        v: out_v,
        stride_y,
        stride_uv,
    }
}

fn copy_plane(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    w: usize,
    h: usize,
) {
    for row in 0..h {
        let s = row * src_stride;
        let d = row * dst_stride;
        dst[d..d + w].copy_from_slice(&src[s..s + w]);
    }
}
