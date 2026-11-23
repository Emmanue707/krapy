#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorSpec {
    pub full_range: bool,
    pub matrix: Matrix,
}

impl ColorSpec {
    pub const DEFAULT: Self = Self {
        full_range: false,
        matrix: Matrix::Bt709,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Matrix {
    Bt601,
    #[default]
    Bt709,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YuvRgb {
    pub m: [[f32; 3]; 3],
    pub y_scale: f32,
    pub y_bias: f32,
    pub u_scale: f32,
    pub u_bias: f32,
    pub v_scale: f32,
    pub v_bias: f32,
}

pub fn yuv_rgb_coeffs(spec: ColorSpec) -> YuvRgb {
    let m = match spec.matrix {
        Matrix::Bt601 => [
            [1.0, 0.0, 1.402],
            [1.0, -0.344136, -0.714136],
            [1.0, 1.772, 0.0],
        ],
        Matrix::Bt709 => [
            [1.0, 0.0, 1.5748],
            [1.0, -0.187324, -0.468124],
            [1.0, 1.8556, 0.0],
        ],
    };
    let (y_scale, y_bias, u_scale, u_bias, v_scale, v_bias) = if spec.full_range {
        (
            1.0 / 255.0,
            0.0,
            1.0 / 255.0,
            -128.0 / 255.0,
            1.0 / 255.0,
            -128.0 / 255.0,
        )
    } else {
        (
            1.0 / 219.0,
            -16.0 / 219.0,
            1.0 / 224.0,
            -128.0 / 224.0,
            1.0 / 224.0,
            -128.0 / 224.0,
        )
    };
    YuvRgb {
        m,
        y_scale,
        y_bias,
        u_scale,
        u_bias,
        v_scale,
        v_bias,
    }
}

pub fn yuv_to_rgb(spec: ColorSpec, y: u8, u: u8, v: u8) -> (f32, f32, f32) {
    let c = yuv_rgb_coeffs(spec);
    let yn = y as f32 * c.y_scale + c.y_bias;
    let un = u as f32 * c.u_scale + c.u_bias;
    let vn = v as f32 * c.v_scale + c.v_bias;
    let r = c.m[0][0] * yn + c.m[0][1] * un + c.m[0][2] * vn;
    let g = c.m[1][0] * yn + c.m[1][1] * un + c.m[1][2] * vn;
    let b = c.m[2][0] * yn + c.m[2][1] * un + c.m[2][2] * vn;
    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1.0 / 255.0 + 1e-5, "{a} vs {b}");
    }

    #[test]
    fn limited_709_white() {
        let spec = ColorSpec::DEFAULT;
        let (r, g, b) = yuv_to_rgb(spec, 235, 128, 128);
        approx(r, 1.0);
        approx(g, 1.0);
        approx(b, 1.0);
    }

    #[test]
    fn limited_709_black() {
        let spec = ColorSpec::DEFAULT;
        let (r, g, b) = yuv_to_rgb(spec, 16, 128, 128);
        approx(r, 0.0);
        approx(g, 0.0);
        approx(b, 0.0);
    }

    #[test]
    fn full_range_white() {
        let spec = ColorSpec {
            full_range: true,
            matrix: Matrix::Bt709,
        };
        let (r, g, b) = yuv_to_rgb(spec, 255, 128, 128);
        approx(r, 1.0);
        approx(g, 1.0);
        approx(b, 1.0);
    }

    #[test]
    fn limited_709_red_roundtrip() {
        let spec = ColorSpec::DEFAULT;
        let (r, g, b) = yuv_to_rgb(spec, 81, 90, 240);
        assert!(r > 0.9 && g < 0.2 && b < 0.2);
        let _ = (r, g, b);
    }
}
