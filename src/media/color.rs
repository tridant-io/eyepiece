//! turning stored y'cbcr samples into r'g'b': the numbers the shader needs, worked out here where they can be tested.
//! v1 does no color management: the conversion follows the file's tags, or the stated assumption, and the result
//! goes to the display unchanged.

/// which y'cbcr matrix a file uses. the constants are kr and kb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl Matrix {
    fn kr_kb(self) -> (f64, f64) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
        }
    }

    /// what to assume when a file does not say: bt.709 for hd and above, bt.601 below, as players have always done.
    pub fn assumed(height: usize) -> Matrix {
        if height >= 720 { Matrix::Bt709 } else { Matrix::Bt601 }
    }
}

/// how samples sit in the texture the shader reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stored {
    /// bits per sample in the file
    pub bits: u32,
    /// bits per texel in the texture: 8 or 16
    pub texel_bits: u32,
    /// true when the sample sits in the top bits of the texel (hardware decoders), false when in the low bits
    pub top_aligned: bool,
    /// true for full range, false for video range (16 to 235 at 8 bits)
    pub full_range: bool,
}

/// what the shader multiplies and adds. a texel read as 0..1 becomes y' by `texel * y_scale + y_offset` and a color
/// difference by `texel * c_scale + c_offset`; then r' = y' + cr_to_r * cr, g' = y' + cb_to_g * cb + cr_to_g * cr,
/// b' = y' + cb_to_b * cb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conversion {
    pub y_scale: f32,
    pub y_offset: f32,
    pub c_scale: f32,
    pub c_offset: f32,
    pub cr_to_r: f32,
    pub cb_to_g: f32,
    pub cr_to_g: f32,
    pub cb_to_b: f32,
}

pub fn conversion(stored: Stored, matrix: Matrix) -> Conversion {
    let max_texel = ((1u64 << stored.texel_bits) - 1) as f64;
    // one code value of the file's depth, as a fraction of the texel's range
    let code = if stored.top_aligned { (1u64 << (stored.texel_bits - stored.bits)) as f64 / max_texel } else { 1.0 / max_texel };
    let step = (1u64 << (stored.bits - 8)) as f64;
    let (y_black, y_span, c_mid, c_span) = if stored.full_range {
        let max = ((1u64 << stored.bits) - 1) as f64;
        (0.0, max, (1u64 << (stored.bits - 1)) as f64, max)
    } else {
        (16.0 * step, 219.0 * step, 128.0 * step, 224.0 * step)
    };
    let (kr, kb) = matrix.kr_kb();
    let kg = 1.0 - kr - kb;
    Conversion {
        y_scale: (1.0 / (code * y_span)) as f32,
        y_offset: (-y_black / y_span) as f32,
        c_scale: (1.0 / (code * c_span)) as f32,
        c_offset: (-c_mid / c_span) as f32,
        cr_to_r: (2.0 * (1.0 - kr)) as f32,
        cb_to_g: (-2.0 * kb * (1.0 - kb) / kg) as f32,
        cr_to_g: (-2.0 * kr * (1.0 - kr) / kg) as f32,
        cb_to_b: (2.0 * (1.0 - kb)) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the same arithmetic the shader does, for one sample.
    fn rgb(c: Conversion, texel: [f64; 3]) -> [f64; 3] {
        let y = texel[0] * c.y_scale as f64 + c.y_offset as f64;
        let cb = texel[1] * c.c_scale as f64 + c.c_offset as f64;
        let cr = texel[2] * c.c_scale as f64 + c.c_offset as f64;
        [y + c.cr_to_r as f64 * cr, y + c.cb_to_g as f64 * cb + c.cr_to_g as f64 * cr, y + c.cb_to_b as f64 * cb]
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn ten_bit_video_range_from_a_hardware_decoder() {
        // p010: the ten bits sit at the top of sixteen
        let c = conversion(Stored { bits: 10, texel_bits: 16, top_aligned: true, full_range: false }, Matrix::Bt709);
        let texel = |code: u32| (code << 6) as f64 / 65535.0;
        assert!(close(rgb(c, [texel(64), texel(512), texel(512)]), [0.0, 0.0, 0.0]));
        assert!(close(rgb(c, [texel(940), texel(512), texel(512)]), [1.0, 1.0, 1.0]));
        // the constants the gate's shader had by hand
        assert!((c.cr_to_r - 1.5748).abs() < 1e-4 && (c.cb_to_b - 1.8556).abs() < 1e-4);
        assert!((c.cb_to_g + 0.1873).abs() < 1e-4 && (c.cr_to_g + 0.4681).abs() < 1e-4);
    }

    #[test]
    fn ten_bit_video_range_from_a_software_decoder() {
        // yuv422p10le: the ten bits sit at the bottom of sixteen
        let c = conversion(Stored { bits: 10, texel_bits: 16, top_aligned: false, full_range: false }, Matrix::Bt709);
        let texel = |code: u32| code as f64 / 65535.0;
        assert!(close(rgb(c, [texel(64), texel(512), texel(512)]), [0.0, 0.0, 0.0]));
        assert!(close(rgb(c, [texel(940), texel(512), texel(512)]), [1.0, 1.0, 1.0]));
        // pure red in bt.709: y' = kr, cb = -kr / (2 (1 - kb)), cr = 0.5
        let red = [texel(64 + (0.2126f64 * 876.0).round() as u32), texel(512 - (0.2126f64 / 1.8556 * 896.0).round() as u32), texel(960)];
        let got = rgb(c, red);
        assert!((got[0] - 1.0).abs() < 0.003 && got[1].abs() < 0.003 && got[2].abs() < 0.003, "{got:?}");
    }

    #[test]
    fn eight_bit_video_range() {
        let c = conversion(Stored { bits: 8, texel_bits: 8, top_aligned: false, full_range: false }, Matrix::Bt601);
        assert!(close(rgb(c, [16.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0]), [0.0, 0.0, 0.0]));
        assert!(close(rgb(c, [235.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0]), [1.0, 1.0, 1.0]));
        assert!((c.cr_to_r - 1.402).abs() < 1e-4 && (c.cb_to_b - 1.772).abs() < 1e-4);
    }

    #[test]
    fn full_range_uses_every_code() {
        let c = conversion(Stored { bits: 8, texel_bits: 8, top_aligned: false, full_range: true }, Matrix::Bt709);
        assert!(close(rgb(c, [0.0, 128.0 / 255.0, 128.0 / 255.0]), [0.0, 0.0, 0.0]));
        assert!(close(rgb(c, [1.0, 128.0 / 255.0, 128.0 / 255.0]), [1.0, 1.0, 1.0]));
    }

    #[test]
    fn twelve_bit_software_planes() {
        let c = conversion(Stored { bits: 12, texel_bits: 16, top_aligned: false, full_range: false }, Matrix::Bt2020);
        let texel = |code: u32| code as f64 / 65535.0;
        assert!(close(rgb(c, [texel(256), texel(2048), texel(2048)]), [0.0, 0.0, 0.0]));
        assert!(close(rgb(c, [texel(3760), texel(2048), texel(2048)]), [1.0, 1.0, 1.0]));
    }

    #[test]
    fn untagged_files_get_the_usual_assumption() {
        assert_eq!(Matrix::assumed(2160), Matrix::Bt709);
        assert_eq!(Matrix::assumed(720), Matrix::Bt709);
        assert_eq!(Matrix::assumed(576), Matrix::Bt601);
    }
}
