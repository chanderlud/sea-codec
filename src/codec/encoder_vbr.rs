use crate::{
    codec::{common::SeaResidualSize, lms::LMS_LEN},
    encoder::EncoderSettings,
};

use super::{
    common::{EncodedSamples, SeaEncoderTrait, SEA_MAX_CHANNELS},
    dqt::SeaDequantTab,
    encoder_base::EncoderBase,
    file::SeaFileHeader,
    lms::SeaLMS,
    qt::SeaQuantTab,
};

pub struct VbrEncoder {
    file_header: SeaFileHeader,
    scale_factor_bits: u8,
    scale_factor_frames: u8,
    vbr_target_bitrate: f32,
    prev_scalefactor: [i32; SEA_MAX_CHANNELS as usize],
    base_encoder: EncoderBase,
    pub lms: Vec<SeaLMS>,
}

// const TARGET_RESIDUAL_DISTRIBUTION: [f32; 6] = [0.00, 0.09, 0.82, 0.07, 0.02, 0.00]; // ([0, target-1, target, target+1, target+2, 0])
const TARGET_RESIDUAL_DISTRIBUTION: [f32; 6] = [0.00, 0.00, 0.95, 0.05, 0.00, 0.00]; // TODO: it needs tuning

impl VbrEncoder {
    pub fn new(file_header: &SeaFileHeader, encoder_settings: &EncoderSettings) -> Self {
        VbrEncoder {
            file_header: file_header.clone(),
            scale_factor_bits: encoder_settings.scale_factor_bits,
            prev_scalefactor: [0; SEA_MAX_CHANNELS as usize],
            lms: SeaLMS::init_vec(file_header.channels as u32),
            scale_factor_frames: encoder_settings.scale_factor_frames,
            base_encoder: EncoderBase::new(
                file_header.channels as usize,
                encoder_settings.scale_factor_bits as usize,
            ),
            vbr_target_bitrate: Self::get_normalized_vbr_bitrate(encoder_settings),
        }
    }

    fn get_normalized_vbr_bitrate(encoder_settings: &EncoderSettings) -> f32 {
        let mut vbr_bitrate = encoder_settings.residual_bits as f32;

        // compensate lms
        vbr_bitrate -= (LMS_LEN as f32 * 16.0 * 2.0) / encoder_settings.frames_per_chunk as f32;

        // compensate scale factor data
        vbr_bitrate -=
            encoder_settings.scale_factor_bits as f32 / encoder_settings.scale_factor_frames as f32;

        // compensate vbr data
        vbr_bitrate -= 2.0 / encoder_settings.scale_factor_frames as f32;

        // compensate with target distribution
        let base_residuals = encoder_settings.residual_bits.floor();
        let new_bitrate = TARGET_RESIDUAL_DISTRIBUTION[1] * (base_residuals - 1.0)
            + TARGET_RESIDUAL_DISTRIBUTION[2] * base_residuals
            + TARGET_RESIDUAL_DISTRIBUTION[3] * (base_residuals + 1.0)
            + TARGET_RESIDUAL_DISTRIBUTION[4] * (base_residuals + 2.0);
        let diff = new_bitrate - base_residuals;
        vbr_bitrate -= diff;

        vbr_bitrate
    }

    // returns items count [target-1, target, target+1, target+2]
    fn interpolate_distribution(items: usize, target_rate: f32) -> [usize; 4] {
        let frac = target_rate.fract();
        let om_frac = 1.0 - frac;

        let mut percentages = [0f32; 4];
        for i in 0..4 {
            percentages[i] = TARGET_RESIDUAL_DISTRIBUTION[i] * frac
                + TARGET_RESIDUAL_DISTRIBUTION[i + 1] * om_frac;
        }

        let mut res = [0usize; 4];
        let mut sum = 0usize;

        // distribute remaining using TARGET_RESIDUAL_DISTRIBUTION
        while sum < items {
            let remaining = items - sum;
            for i in 0..4 {
                let value = (remaining as f32 * percentages[i]) as usize;
                sum += value;
                res[i] += value;
            }

            // if remaining is not enough to distribute based on TARGET_RESIDUAL_DISTRIBUTION
            if items - sum == remaining {
                sum += remaining;
                res[1] += remaining
            }
        }

        res
    }

    fn choose_residual_len_from_errors(&self, input_len: usize, errors: &[u64]) -> Vec<u8> {
        // we need to ensure that last partial frames are not touched (it would debalance the frame size)
        let sortable_items = input_len / self.scale_factor_frames as usize;

        let mut indices: Vec<u16> = (0..sortable_items as u16).collect();
        indices.sort_unstable_by(|&a, &b| errors[a as usize].cmp(&errors[b as usize]));

        let [minus_one_items, _, plus_one_items, plus_two_items] =
            Self::interpolate_distribution(sortable_items, self.vbr_target_bitrate);

        let base_residual_bits = self.vbr_target_bitrate as u8;

        let mut residual_sizes = vec![base_residual_bits; errors.len()];

        for index in indices.iter().take(minus_one_items) {
            residual_sizes[*index as usize] = base_residual_bits - 1;
        }

        for index in indices[(sortable_items - plus_two_items - plus_one_items)..]
            .iter()
            .take(plus_one_items)
        {
            residual_sizes[*index as usize] = base_residual_bits + 1;
        }

        for index in indices[sortable_items - plus_two_items..]
            .iter()
            .take(plus_two_items)
        {
            residual_sizes[*index as usize] = base_residual_bits + 2;
        }

        // count how many times each residual size appears
        let mut residual_size_counts = vec![0; 9];
        for i in 0..errors.len() {
            residual_size_counts[residual_sizes[i] as usize] += 1;
        }

        residual_sizes
    }

    fn analyze(&mut self, input_slice: &[i16]) -> Vec<u8> {
        let mut errors: Vec<u64> = Vec::with_capacity(input_slice.len());

        let analyze_residual_size = SeaResidualSize::from(self.vbr_target_bitrate as u8 + 1);

        let slice_size = self.scale_factor_frames as usize * self.file_header.channels as usize;

        todo!();

        // let dqt: &Vec<Vec<i32>> = dequant_tab.get_dqt(analyze_residual_size as usize);

        // let scalefactor_reciprocals =
        //     dequant_tab.get_scalefactor_reciprocals(analyze_residual_size as usize);

        // let mut lms = self.lms.clone();
        // let mut prev_scalefactor = self.prev_scalefactor.clone();

        // let best_residual_bits: &mut [u8] =
        //     &mut vec![0u8; input_slice.len() / self.file_header.channels as usize];

        // for (_, input_slice) in input_slice.chunks(slice_size).enumerate() {
        //     for channel_offset in 0..self.file_header.channels as usize {
        // let (_best_rank, best_lms, best_scalefactor) =
        //     self.base_encoder.get_residuals_with_best_scalefactor(
        //         self.file_header.channels as usize,
        //         dqt,
        //         scalefactor_reciprocals,
        //         &input_slice[channel_offset..],
        //         prev_scalefactor[channel_offset],
        //         &lms[channel_offset],
        //         analyze_residual_size,
        //         best_residual_bits,
        //     );

        // prev_scalefactor[channel_offset] = best_scalefactor;
        // lms[channel_offset] = best_lms;
        // errors.push(_best_rank);
        //     }
        // }

        // self.choose_residual_len_from_errors(input_slice.len(), &errors)
    }
}

impl SeaEncoderTrait for VbrEncoder {
    fn encode(&mut self, samples: &[i16]) -> EncodedSamples {
        let mut scale_factors = Vec::<u8>::new();
        let mut residuals = vec![0u8; samples.len()];

        let residual_bits = self.analyze(samples);

        let slice_size = self.scale_factor_frames as usize * self.file_header.channels as usize;

        let best_residual_bits: &mut [u8] =
            &mut vec![0u8; samples.len() / self.file_header.channels as usize];

        for (slice_index, input_slice) in samples.chunks(slice_size).enumerate() {
            for channel_offset in 0..self.file_header.channels as usize {
                let residual_size = residual_bits
                    [slice_index * self.file_header.channels as usize + channel_offset]
                    as usize;

                // let dqt: &Vec<Vec<i32>> = dequant_tab.get_dqt(residual_size);
                // let scalefactor_reciprocals: &Vec<i32> =
                //     dequant_tab.get_scalefactor_reciprocals(residual_size);

                // let (_best_rank, best_lms, best_scalefactor) =
                //     self.base_encoder.get_residuals_with_best_scalefactor(
                //         self.file_header.channels as usize,
                //         dqt,
                //         scalefactor_reciprocals,
                //         &input_slice[channel_offset..],
                //         self.prev_scalefactor[channel_offset] as i32,
                //         &self.lms[channel_offset],
                //         SeaResidualSize::from(
                //             residual_bits
                //                 [slice_index * self.file_header.channels as usize + channel_offset],
                //         ),
                //         best_residual_bits,
                //     );

                // self.prev_scalefactor[channel_offset] = best_scalefactor;
                // self.lms[channel_offset] = best_lms;

                // scale_factors.push(best_scalefactor as u8);
                // residuals need to be interleaved
                for i in 0..best_residual_bits.len() {
                    residuals[slice_index * slice_size
                        + i * self.file_header.channels as usize
                        + channel_offset] = best_residual_bits[i];
                }
            }
        }

        EncodedSamples {
            scale_factors,
            residuals,
            residual_bits,
        }
    }
}
