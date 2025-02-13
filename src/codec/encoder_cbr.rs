use super::{
    common::{
        get_residuals_with_best_scalefactor, EncodedSamples, SeaDequantTab, SeaEncoderTrait,
        SeaResidualSize, SEA_MAX_CHANNELS,
    },
    encoder::EncoderSettings,
    file::SeaFileHeader,
    lms::SeaLMS,
};

pub struct CbrEncoder {
    file_header: SeaFileHeader,
    residual_size: SeaResidualSize,
    scale_factor_frames: u8,
    scale_factor_bits: u8,
    prev_scalefactor: [i32; SEA_MAX_CHANNELS as usize],
    pub lms: Vec<SeaLMS>,
}

impl CbrEncoder {
    pub fn new(file_header: &SeaFileHeader, encoder_settings: &EncoderSettings) -> Self {
        CbrEncoder {
            file_header: file_header.clone(),
            residual_size: SeaResidualSize::from(encoder_settings.residual_bits.floor() as u8),
            scale_factor_frames: encoder_settings.scale_factor_frames,
            scale_factor_bits: encoder_settings.scale_factor_bits,
            prev_scalefactor: [0; SEA_MAX_CHANNELS as usize],
            lms: SeaLMS::init_vec(file_header.channels as u32),
        }
    }
}

impl SeaEncoderTrait for CbrEncoder {
    fn encode(&mut self, samples: &[i16], dequant_tab: &mut SeaDequantTab) -> EncodedSamples {
        let mut scale_factors = Vec::<u8>::new();
        let mut residuals = vec![0u8; samples.len()];

        let dqt: &Vec<Vec<i32>> =
            dequant_tab.get_dqt(self.scale_factor_bits as usize, self.residual_size as usize);

        let slice_size = self.scale_factor_frames as usize * self.file_header.channels as usize;

        for (slice_index, input_slice) in samples.chunks(slice_size).enumerate() {
            for channel_offset in 0..self.file_header.channels as usize {
                let (_best_rank, best_residuals, best_lms, best_scalefactor) =
                    get_residuals_with_best_scalefactor(
                        self.file_header.channels as usize,
                        dqt,
                        &input_slice[channel_offset..],
                        self.prev_scalefactor[channel_offset] as i32,
                        &self.lms[channel_offset],
                        self.residual_size,
                        self.scale_factor_bits,
                    );

                self.prev_scalefactor[channel_offset] = best_scalefactor;
                self.lms[channel_offset] = best_lms;

                scale_factors.push(best_scalefactor as u8);
                // residuals need to be interleaved
                for i in 0..best_residuals.len() {
                    residuals[slice_index * slice_size
                        + i * self.file_header.channels as usize
                        + channel_offset] = best_residuals[i];
                }
            }
        }

        EncodedSamples {
            scale_factors,
            residuals,
            residual_bits: vec![],
        }
    }
}
