use crate::color::ColorSpec;

#[derive(Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub stride_y: usize,
    pub stride_uv: usize,
    pub color: ColorSpec,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pack_yuv420p_into(
    out: &mut Frame,
    y: &[u8],
    u: &[u8],
    v: &[u8],
    width: u32,
    height: u32,
    y_stride: usize,
    uv_stride: usize,
) {
    let w = width as usize;
    let h = height as usize;
    let stride_y = w;
    let stride_uv = w / 2;
    let y_len = stride_y * h;
    let uv_len = stride_uv * (h / 2);
    out.width = width;
    out.height = height;
    out.stride_y = stride_y;
    out.stride_uv = stride_uv;
    out.y.resize(y_len, 0);
    out.u.resize(uv_len, 0);
    out.v.resize(uv_len, 0);
    copy_plane(y, y_stride, &mut out.y, stride_y, w, h);
    copy_plane(u, uv_stride, &mut out.u, stride_uv, w / 2, h / 2);
    copy_plane(v, uv_stride, &mut out.v, stride_uv, w / 2, h / 2);
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
