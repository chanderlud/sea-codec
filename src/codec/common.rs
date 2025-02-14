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

            // zig zag pattern decreases quantization error
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

#[derive(Debug, PartialEq)]
pub struct SeaQuantTab {
    pub offsets: [usize; 9],
    pub quant_tab: [u8; 5 + 9 + 17 + 33 + 65 + 129 + 257 + 513],
}

impl SeaQuantTab {
    // use zig-zag pattern to decrease quantization error
    fn fill_dqt_table(slice: &mut [u8], items: usize) {
        let midpoint = items / 2;
        let mut x = (items / 2 - 1) as i32;
        slice[0] = x as u8;
        for i in (1..midpoint).step_by(2) {
            slice[i] = x as u8;
            slice[i + 1] = x as u8;
            x -= 2;
        }
        x = 0;
        for i in (midpoint..(items - 1)).step_by(2) {
            slice[i] = x as u8;
            slice[i + 1] = x as u8;
            x += 2;
        }
        slice[items - 1] = (x - 2) as u8;

        // special case when residual_size = 2
        if items == 9 {
            slice[2] = 1;
            slice[6] = 0;
        }
    }

    pub fn init() -> Self {
        let mut offsets = [0; 9];
        let mut quant_tab = [0; 5 + 9 + 17 + 33 + 65 + 129 + 257 + 513];

        let mut current_offset = 0;
        for shift in 2..=9 {
            offsets[shift - 1] = current_offset;

            let items = (1 << shift) + 1;

            Self::fill_dqt_table(
                &mut quant_tab[current_offset..current_offset + items],
                items,
            );

            current_offset += items;
        }

        Self { offsets, quant_tab }
    }
}

fn calculate_residuals(
    channels: usize,
    dequant_tab: &[i32],
    quant_tab: &SeaQuantTab,
    samples: &[i16],
    scalefactor: i32,
    lms: &mut SeaLMS,
    best_rank: u64, // provided as optimization, can be u64::MAX if omitted
    residual_size: SeaResidualSize,
    scalefactor_reciprocals: &[i32],
) -> (Vec<u8>, u64) {
    let mut current_rank: u64 = 0;

    let clamp_limit = residual_size.to_binary_combinations() as i32;

    let quant_tab_offset = clamp_limit + quant_tab.offsets[residual_size as usize] as i32;

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
        let quantized = quant_tab.quant_tab[(quant_tab_offset + clamped) as usize];

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
    quant_tab: &SeaQuantTab,
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
            quant_tab,
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
    fn encode(
        &mut self,
        input_slice: &[i16],
        quant_tab: &SeaQuantTab,
        dequant_tab: &mut SeaDequantTab,
    ) -> EncodedSamples;
}
