use std::{array, io};

use super::lms::{SeaLMS, LMS_LEN};

pub const SEAC_MAGIC: u32 = u32::from_be_bytes(*b"seac"); // 0x73 0x65 0x61 0x63
pub const SEA_MAX_CHANNELS: u8 = 8;

#[inline(always)]
pub fn clamp_i16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum SeaResidualSize {
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
}

impl SeaResidualSize {
    #[inline(always)]
    pub fn from(len: u8) -> Self {
        match len {
            1 => SeaResidualSize::One,
            2 => SeaResidualSize::Two,
            3 => SeaResidualSize::Three,
            4 => SeaResidualSize::Four,
            5 => SeaResidualSize::Five,
            6 => SeaResidualSize::Six,
            7 => SeaResidualSize::Seven,
            8 => SeaResidualSize::Eight,
            _ => panic!("Invalid residual length"),
        }
    }

    #[inline(always)]
    pub fn to_binary_combinations(self) -> usize {
        match self {
            SeaResidualSize::One => 2,
            SeaResidualSize::Two => 4,
            SeaResidualSize::Three => 8,
            SeaResidualSize::Four => 16,
            SeaResidualSize::Five => 32,
            SeaResidualSize::Six => 64,
            SeaResidualSize::Seven => 128,
            SeaResidualSize::Eight => 256,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct SeaDequantTab {
    scale_factor_bits: usize,

    cached_dqt: [Vec<Vec<i32>>; 9],
}

// scale_factors along with residuals should cover all potential values
// we try to calcualte an exponent for max scalefactor that is efficient given the range ot residuals
// theoretically [12, 11, 10, 9, 8, 7] should be fine, but these numbers perform better over a diverse dataset
pub static IDEAL_POW_FACTOR: [f32; 8] = [12.0, 11.65, 11.20, 10.58, 9.64, 8.75, 7.66, 6.63]; // were found experimentally

impl SeaDequantTab {
    pub fn init(scale_factor_bits: usize) -> Self {
        SeaDequantTab {
            scale_factor_bits,
            cached_dqt: array::from_fn(|_| Vec::new()),
        }
    }

    fn calculate_ideal_pow_factors() -> [[f32; 8]; 5] {
        let mut ideal_power_factors: [[f32; 8]; 5] = [[0.0; 8]; 5];

        for scale_factor_bits in 2..=6 {
            for residual_bits in 1..=8 {
                ideal_power_factors[scale_factor_bits - 2][residual_bits - 1] =
                    IDEAL_POW_FACTOR[residual_bits - 1] / (scale_factor_bits as f32)
            }
        }
        ideal_power_factors
    }

    fn calculate_scale_factors(residual_bits: usize, scale_factor_bits: usize) -> Vec<i32> {
        let ideal_pow_factors = Self::calculate_ideal_pow_factors();

        let mut output: Vec<i32> = Vec::new();
        let power_factor = ideal_pow_factors[scale_factor_bits - 2][residual_bits - 1];

        let scale_factor_items = 1 << scale_factor_bits;
        for index in 1..=scale_factor_items {
            let value: f32 = (index as f32).powf(power_factor);
            output.push(value as i32);
        }

        output
    }

    fn get_scalefactor_reciprocals(residual_bits: usize, scale_factor_bits: usize) -> Vec<i32> {
        let scale_factors = Self::calculate_scale_factors(residual_bits, scale_factor_bits);
        let mut output: Vec<i32> = Vec::new();
        for sf in scale_factors {
            let value = ((1 << 16) as f32 / sf as f32) as i32;
            output.push(value);
        }
        output
    }

    fn gen_dqt_table(residual_bits: usize) -> Vec<f32> {
        match residual_bits {
            1 => return vec![2.0],
            2 => return vec![1.115, 4.0],
            _ => (),
        }

        let start: f32 = 0.75f32;
        let steps = 1 << (residual_bits - 1);
        let end = ((1 << residual_bits) - 1) as f32;
        let step = (end - start) / (steps - 1) as f32;
        let step_floor = step.floor();

        let mut curve = vec![0.0; steps];
        for i in 1..steps {
            let y = 0.5 + i as f32 * step_floor;
            curve[i] = y;
        }

        curve[0] = start;
        curve[steps - 1] = end;
        curve
    }

    fn generate_dqt(&self, scale_factor_bits: usize, residual_bits: usize) -> Vec<Vec<i32>> {
        let dqt = Self::gen_dqt_table(residual_bits);

        let scalefactor_items = 1 << scale_factor_bits;

        let mut output: Vec<Vec<i32>> = Vec::new();

        let dqt_items = 2usize.pow(residual_bits as u32 - 1);

        let scale_factors = Self::calculate_scale_factors(residual_bits, scale_factor_bits);

        for s in 0..scalefactor_items {
            output.push(Vec::with_capacity(dqt.len()));

            for q in 0..dqt_items {
                let val = (scale_factors[s] as f32 * dqt[q]).round() as i32;
                output[s].push(val);
                output[s].push(-val);
            }
        }

        output
    }

    pub fn get_dqt(&mut self, scale_factor_bits: usize, residual_bits: usize) -> &Vec<Vec<i32>> {
        if scale_factor_bits != self.scale_factor_bits {
            self.cached_dqt = array::from_fn(|_| Vec::new());
        }

        let cached_dqt = &self.cached_dqt[residual_bits as usize];
        if cached_dqt.len() == 0 {
            let new_dqt = self.generate_dqt(scale_factor_bits, residual_bits);
            self.cached_dqt[residual_bits as usize] = new_dqt;
        }

        &self.cached_dqt[residual_bits as usize]
    }
}

#[derive(Debug)]
pub enum SeaError {
    ReadError,
    InvalidParameters,
    InvalidFile,
    InvalidFrame,
    EncoderClosed,
    UnsupportedVersion,
    TooManyFrames,
    MetadataTooLarge,
    IoError(io::Error),
}

impl From<io::Error> for SeaError {
    fn from(error: io::Error) -> Self {
        SeaError::IoError(error)
    }
}

#[inline(always)]
pub fn sea_div(v: i32, scalefactor_reciprocal: i64) -> i32 {
    let n = (v as i64 * scalefactor_reciprocal + (1 << 15)) >> 16;
    (n + (v.signum() as i64 - n.signum() as i64)) as i32
}

const SEA_QUANT_TAB_OFFSET: [i32; 9] = [
    0,
    0,
    5,
    5 + 9,
    5 + 9 + 17,
    5 + 9 + 17 + 33,
    5 + 9 + 17 + 33 + 65,
    5 + 9 + 17 + 33 + 65 + 129,
    5 + 9 + 17 + 33 + 65 + 129 + 257,
];
const SEA_QUANT_TAB: [u8; 5 + 9 + 17 + 33 + 65 + 129 + 257 + 513] = [
    /* QUANT_TAB 1 */
    1, 1, /* -4..-1 */
    0, /*  0     */
    0, 0, /*  1.. 4 */
    /* QUANT_TAB 2 */
    3, 3, 1, 1, /* -4..-1 */
    0, /*  0     */
    0, 0, 2, 2, /*  1.. 4 */
    /* QUANT_TAB 3 */
    7, 7, 7, 5, 5, 3, 3, 1, /* -8..-1 */
    0, /*  0     */
    0, 2, 2, 4, 4, 6, 6, 6, /*  1.. 8 */
    /* QUANT_TAB 4 */
    15, 15, 15, 13, 13, 11, 11, 9, /* -16..-9 */
    9, 7, 7, 5, 5, 3, 3, 1, /* -8..-1 */
    0, /*  0     */
    0, 2, 2, 4, 4, 6, 6, 8, /*  1.. 8 */
    8, 10, 10, 12, 12, 14, 14, 14, /* 9..16 */
    /* QUANT_TAB 5 */
    31, 31, 31, 29, 29, 27, 27, 25, 25, 23, 23, 21, 21, 19, 19, 17, /* -32..-17 */
    17, 15, 15, 13, 13, 11, 11, 9, 9, 7, 7, 5, 5, 3, 3, 1, /* -16..-1 */
    0, /*  0     */
    0, 2, 2, 4, 4, 6, 6, 8, 8, 10, 10, 12, 12, 14, 14, 16, /*  1.. 16 */
    16, 18, 18, 20, 20, 22, 22, 24, 24, 26, 26, 28, 28, 30, 30, 30, /* 17..32 */
    /* QUANT_TAB 6 */
    63, 63, 63, 61, 61, 59, 59, 57, 57, 55, 55, 53, 53, 51, 51, 49, /* -64..-49 */
    49, 47, 47, 45, 45, 43, 43, 41, 41, 39, 39, 37, 37, 35, 35, 33, /* -48..-33 */
    33, 31, 31, 29, 29, 27, 27, 25, 25, 23, 23, 21, 21, 19, 19, 17, /* -32..-17 */
    17, 15, 15, 13, 13, 11, 11, 9, 9, 7, 7, 5, 5, 3, 3, 1, /* -16..-1 */
    0, /*  0     */
    0, 2, 2, 4, 4, 6, 6, 8, 8, 10, 10, 12, 12, 14, 14, 16, /*  1.. 16 */
    16, 18, 18, 20, 20, 22, 22, 24, 24, 26, 26, 28, 28, 30, 30, 32, /* 17..32 */
    32, 34, 34, 36, 36, 38, 38, 40, 40, 42, 42, 44, 44, 46, 46, 48, /* 33..48 */
    48, 50, 50, 52, 52, 54, 54, 56, 56, 58, 58, 60, 60, 62, 62, 62, /* 49..64 */
    /* QUANT_TAB 7 */
    127, 127, 127, 125, 125, 123, 123, 121, 121, 119, 119, 117, 117, 115, 115,
    113, /* -128..-113 */
    113, 111, 111, 109, 109, 107, 107, 105, 105, 103, 103, 101, 101, 99, 99,
    97, /* -112..-97 */
    97, 95, 95, 93, 93, 91, 91, 89, 89, 87, 87, 85, 85, 83, 83, 81, /* -96..-81 */
    81, 79, 79, 77, 77, 75, 75, 73, 73, 71, 71, 69, 69, 67, 67, 65, /* -80..-65 */
    65, 63, 63, 61, 61, 59, 59, 57, 57, 55, 55, 53, 53, 51, 51, 49, /* -64..-49 */
    49, 47, 47, 45, 45, 43, 43, 41, 41, 39, 39, 37, 37, 35, 35, 33, /* -48..-33 */
    33, 31, 31, 29, 29, 27, 27, 25, 25, 23, 23, 21, 21, 19, 19, 17, /* -32..-17 */
    17, 15, 15, 13, 13, 11, 11, 9, 9, 7, 7, 5, 5, 3, 3, 1, /* -16..-1 */
    0, /*  0     */
    0, 2, 2, 4, 4, 6, 6, 8, 8, 10, 10, 12, 12, 14, 14, 16, /*  1.. 16 */
    16, 18, 18, 20, 20, 22, 22, 24, 24, 26, 26, 28, 28, 30, 30, 32, /* 17..32 */
    32, 34, 34, 36, 36, 38, 38, 40, 40, 42, 42, 44, 44, 46, 46, 48, /* 33..48 */
    48, 50, 50, 52, 52, 54, 54, 56, 56, 58, 58, 60, 60, 62, 62, 64, /* 49..64 */
    64, 66, 66, 68, 68, 70, 70, 72, 72, 74, 74, 76, 76, 78, 78, 80, /* 65..80 */
    80, 82, 82, 84, 84, 86, 86, 88, 88, 90, 90, 92, 92, 94, 94, 96, /* 81..96 */
    96, 98, 98, 100, 100, 102, 102, 104, 104, 106, 106, 108, 108, 110, 110, 112, /* 97..112 */
    112, 114, 114, 116, 116, 118, 118, 120, 120, 122, 122, 124, 124, 126, 126,
    126, /* 113..128 */
    /* QUANT_TAB 8 */
    255, 255, 255, 253, 253, 251, 251, 249, 249, 247, 247, 245, 245, 243, 243,
    241, /* -256..-241 */
    241, 239, 239, 237, 237, 235, 235, 233, 233, 231, 231, 229, 229, 227, 227,
    225, /* -240..-225 */
    225, 223, 223, 221, 221, 219, 219, 217, 217, 215, 215, 213, 213, 211, 211,
    209, /* -224..-209 */
    209, 207, 207, 205, 205, 203, 203, 201, 201, 199, 199, 197, 197, 195, 195,
    193, /* -208..-193 */
    193, 191, 191, 189, 189, 187, 187, 185, 185, 183, 183, 181, 181, 179, 179,
    177, /* -192..-177 */
    177, 175, 175, 173, 173, 171, 171, 169, 169, 167, 167, 165, 165, 163, 163,
    161, /* -176..-161 */
    161, 159, 159, 157, 157, 155, 155, 153, 153, 151, 151, 149, 149, 147, 147,
    145, /* -160..-145 */
    145, 143, 143, 141, 141, 139, 139, 137, 137, 135, 135, 133, 133, 131, 131,
    129, /* -144..-129 */
    129, 127, 127, 125, 125, 123, 123, 121, 121, 119, 119, 117, 117, 115, 115,
    113, /* -128..-113 */
    113, 111, 111, 109, 109, 107, 107, 105, 105, 103, 103, 101, 101, 99, 99,
    97, /* -112..-97 */
    97, 95, 95, 93, 93, 91, 91, 89, 89, 87, 87, 85, 85, 83, 83, 81, /* -96..-81 */
    81, 79, 79, 77, 77, 75, 75, 73, 73, 71, 71, 69, 69, 67, 67, 65, /* -80..-65 */
    65, 63, 63, 61, 61, 59, 59, 57, 57, 55, 55, 53, 53, 51, 51, 49, /* -64..-49 */
    49, 47, 47, 45, 45, 43, 43, 41, 41, 39, 39, 37, 37, 35, 35, 33, /* -48..-33 */
    33, 31, 31, 29, 29, 27, 27, 25, 25, 23, 23, 21, 21, 19, 19, 17, /* -32..-17 */
    17, 15, 15, 13, 13, 11, 11, 9, 9, 7, 7, 5, 5, 3, 3, 1, /* -16..-1 */
    0, /*  0     */
    0, 2, 2, 4, 4, 6, 6, 8, 8, 10, 10, 12, 12, 14, 14, 16, /*  1.. 16 */
    16, 18, 18, 20, 20, 22, 22, 24, 24, 26, 26, 28, 28, 30, 30, 32, /* 17..32 */
    32, 34, 34, 36, 36, 38, 38, 40, 40, 42, 42, 44, 44, 46, 46, 48, /* 33..48 */
    48, 50, 50, 52, 52, 54, 54, 56, 56, 58, 58, 60, 60, 62, 62, 64, /* 49..64 */
    64, 66, 66, 68, 68, 70, 70, 72, 72, 74, 74, 76, 76, 78, 78, 80, /* 65..80 */
    80, 82, 82, 84, 84, 86, 86, 88, 88, 90, 90, 92, 92, 94, 94, 96, /* 81..96 */
    96, 98, 98, 100, 100, 102, 102, 104, 104, 106, 106, 108, 108, 110, 110, 112, /* 97..112 */
    112, 114, 114, 116, 116, 118, 118, 120, 120, 122, 122, 124, 124, 126, 126,
    128, /* 113..128 */
    128, 130, 130, 132, 132, 134, 134, 136, 136, 138, 138, 140, 140, 142, 142,
    144, /* 129..144 */
    144, 146, 146, 148, 148, 150, 150, 152, 152, 154, 154, 156, 156, 158, 158,
    160, /* 145..160 */
    160, 162, 162, 164, 164, 166, 166, 168, 168, 170, 170, 172, 172, 174, 174,
    176, /* 161..176 */
    176, 178, 178, 180, 180, 182, 182, 184, 184, 186, 186, 188, 188, 190, 190,
    192, /* 177..192 */
    192, 194, 194, 196, 196, 198, 198, 200, 200, 202, 202, 204, 204, 206, 206,
    208, /* 193..208 */
    208, 210, 210, 212, 212, 214, 214, 216, 216, 218, 218, 220, 220, 222, 222,
    224, /* 209..224 */
    224, 226, 226, 228, 228, 230, 230, 232, 232, 234, 234, 236, 236, 238, 238,
    240, /* 225..240 */
    240, 242, 242, 244, 244, 246, 246, 248, 248, 250, 250, 252, 252, 254, 254,
    254, /* 241..256 */
];

fn calculate_residuals(
    channels: usize,
    dequant_tab: &[i32],
    samples: &[i16],
    scalefactor: i32,
    lms: &mut SeaLMS,
    best_rank: u64, // provided as optimization, can be u64::MAX if omitted
    residual_size: SeaResidualSize,
    scalefactor_reciprocals: &[i32],
) -> (Vec<u8>, u64) {
    let mut current_rank: u64 = 0;

    let clamp_limit = residual_size.to_binary_combinations() as i32;

    let quant_tab_offset = clamp_limit + SEA_QUANT_TAB_OFFSET[residual_size as usize];

    let mut residuals: Vec<u8> = Vec::new();

    for sample_i16 in samples.iter().step_by(channels as usize) {
        let sample = *sample_i16 as i32;
        let predicted = lms.predict();
        let residual = sample - predicted;
        let scaled = sea_div(
            residual,
            scalefactor_reciprocals[scalefactor as usize] as i64,
        );
        let clamped = scaled.clamp(-clamp_limit, clamp_limit);
        let quantized = SEA_QUANT_TAB[(quant_tab_offset + clamped) as usize];

        let dequantized = dequant_tab[quantized as usize];
        let reconstructed = clamp_i16(predicted + dequantized);

        let error: i64 = sample as i64 - reconstructed as i64;

        let error_sq = error.pow(2) as u64;

        current_rank += error_sq + lms.get_weights_penalty();
        if current_rank > best_rank {
            break;
        }

        lms.update(reconstructed, dequantized);
        residuals.push(quantized);
    }

    (residuals, current_rank)
}

pub fn get_residuals_with_best_scalefactor(
    channels: usize,
    dequant_tab: &Vec<Vec<i32>>,
    samples: &[i16],
    prev_scalefactor: i32, // provided as optimization, can be 0
    ref_lms: &SeaLMS,
    residual_size: SeaResidualSize,
    scale_factor_bits: u8,
) -> (u64, Vec<u8>, SeaLMS, i32) {
    let mut best_rank: u64 = u64::MAX;
    let mut best_residuals: Vec<u8> = Vec::new();
    let mut best_lms = SeaLMS {
        history: [0; LMS_LEN],
        weights: [0; LMS_LEN],
    };
    let mut best_scalefactor: i32 = 0;

    let mut lms: SeaLMS = ref_lms.clone();

    let scalefactor_reciprocals = SeaDequantTab::get_scalefactor_reciprocals(
        residual_size as usize,
        scale_factor_bits as usize,
    );

    let scalefactor_end = 1 << scale_factor_bits;

    for sfi in 0..scalefactor_end {
        let scalefactor: i32 = (sfi + prev_scalefactor) % scalefactor_end;

        lms.clone_from(&ref_lms);

        let dqt = &dequant_tab[scalefactor as usize];

        let (residuals, current_rank) = calculate_residuals(
            channels,
            dqt,
            &samples,
            scalefactor,
            &mut lms,
            best_rank,
            residual_size,
            &scalefactor_reciprocals,
        );

        if current_rank < best_rank {
            best_rank = current_rank;
            best_residuals = residuals;
            best_lms.clone_from(&lms);
            best_scalefactor = scalefactor;
        }
    }

    (best_rank, best_residuals, best_lms, best_scalefactor)
}

#[inline(always)]
pub fn read_bytes<R: io::Read, const BYTES: usize>(mut reader: R) -> io::Result<[u8; BYTES]> {
    let mut buf = [0_u8; BYTES];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

#[inline(always)]
pub fn read_u8<R: io::Read>(reader: R) -> io::Result<u8> {
    let data: [u8; 1] = read_bytes(reader)?;
    Ok(data[0])
}

#[inline(always)]
pub fn read_u16_le<R: io::Read>(reader: R) -> io::Result<u16> {
    let data = read_bytes(reader)?;
    Ok(u16::from_le_bytes(data))
}

#[inline(always)]
pub fn read_u32_be<R: io::Read>(reader: R) -> io::Result<u32> {
    let data = read_bytes(reader)?;
    Ok(u32::from_be_bytes(data))
}

#[inline(always)]
pub fn read_u32_le<R: io::Read>(reader: R) -> io::Result<u32> {
    let data = read_bytes(reader)?;
    Ok(u32::from_le_bytes(data))
}

pub fn read_max_or_zero<R: io::Read>(mut reader: R, at_least_bytes: usize) -> io::Result<Vec<u8>> {
    let mut buffer = vec![0u8; at_least_bytes];
    let mut total_bytes_read = 0;

    while total_bytes_read < at_least_bytes {
        let bytes_read = reader.read(&mut buffer[total_bytes_read..])?;

        // EOF
        if bytes_read == 0 {
            break;
        }

        total_bytes_read += bytes_read;
    }

    if total_bytes_read == 0 {
        return Ok(Vec::new());
    }

    return Ok(buffer[..total_bytes_read].to_vec());
}

#[derive(Debug)]
pub struct EncodedSamples {
    pub scale_factors: Vec<u8>,
    pub residuals: Vec<u8>,
    pub residual_bits: Vec<u8>,
}

pub trait SeaEncoderTrait {
    fn encode(&mut self, input_slice: &[i16], dequant_tab: &mut SeaDequantTab) -> EncodedSamples;
}
