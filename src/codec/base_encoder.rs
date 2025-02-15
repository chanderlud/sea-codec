use super::{
    common::{clamp_i16, SeaResidualSize},
    lms::{SeaLMS, LMS_LEN},
    qt::SeaQuantTab,
};

pub struct BaseEncoder {
    current_residuals: Vec<u8>,
}

#[inline(always)]
pub fn sea_div(v: i32, scalefactor_reciprocal: i64) -> i32 {
    let n = (v as i64 * scalefactor_reciprocal + (1 << 15)) >> 16;
    (n + (v.signum() as i64 - n.signum() as i64)) as i32
}

impl BaseEncoder {
    pub fn new() -> Self {
        Self {
            current_residuals: Vec::new(),
        }
    }

    fn calculate_residuals(
        &mut self,
        channels: usize,
        dequant_tab: &[i32],
        quant_tab: &SeaQuantTab,
        samples: &[i16],
        scalefactor: i32,
        lms: &mut SeaLMS,
        best_rank: u64, // provided as optimization, can be u64::MAX if omitted
        residual_size: SeaResidualSize,
        scalefactor_reciprocals: &[i32],
    ) -> u64 {
        let mut current_rank: u64 = 0;

        let clamp_limit = residual_size.to_binary_combinations() as i32;

        let quant_tab_offset = clamp_limit + quant_tab.offsets[residual_size as usize] as i32;

        for (index, sample_i16) in samples.iter().step_by(channels as usize).enumerate() {
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
            self.current_residuals[index] = quantized;
        }

        current_rank
    }

    pub fn get_residuals_with_best_scalefactor(
        &mut self,
        channels: usize,
        quant_tab: &SeaQuantTab,
        dequant_tab: &Vec<Vec<i32>>,
        scalefactor_reciprocals: &[i32],
        samples: &[i16],
        prev_scalefactor: i32, // provided as optimization, can be 0
        ref_lms: &SeaLMS,
        residual_size: SeaResidualSize,
        scale_factor_bits: u8,
        best_residual_bits: &mut [u8],
    ) -> (u64, SeaLMS, i32) {
        let mut best_rank: u64 = u64::MAX;

        self.current_residuals.resize(best_residual_bits.len(), 0);

        let mut best_lms = SeaLMS::new();
        let mut best_scalefactor: i32 = 0;

        let mut current_lms: SeaLMS = ref_lms.clone();

        let scalefactor_end = 1 << scale_factor_bits;

        for sfi in 0..scalefactor_end {
            let scalefactor: i32 = (sfi + prev_scalefactor) % scalefactor_end;

            current_lms.clone_from(&ref_lms);

            let dqt = &dequant_tab[scalefactor as usize];

            let current_rank = self.calculate_residuals(
                channels,
                dqt,
                quant_tab,
                &samples,
                scalefactor,
                &mut current_lms,
                best_rank,
                residual_size,
                &scalefactor_reciprocals,
            );

            if current_rank < best_rank {
                best_rank = current_rank;
                best_residual_bits[..self.current_residuals.len()]
                    .clone_from_slice(&self.current_residuals);
                best_lms.clone_from(&current_lms);
                best_scalefactor = scalefactor;
            }
        }

        (best_rank, best_lms, best_scalefactor)
    }
}
