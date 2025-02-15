use super::{
    base_encoder::BaseEncoder,
    common::{EncodedSamples, SeaEncoderTrait, SeaResidualSize},
    dqt::SeaDequantTab,
    encoder::EncoderSettings,
    file::SeaFileHeader,
    lms::SeaLMS,
};

pub struct CbrEncoder {
    file_header: SeaFileHeader,
    residual_size: SeaResidualSize,
    scale_factor_frames: u8,
    scale_factor_bits: u8,
    base_encoder: BaseEncoder,
}

impl CbrEncoder {
    pub fn new(file_header: &SeaFileHeader, encoder_settings: &EncoderSettings) -> Self {
        CbrEncoder {
            file_header: file_header.clone(),
            residual_size: SeaResidualSize::from(encoder_settings.residual_bits.floor() as u8),
            scale_factor_frames: encoder_settings.scale_factor_frames,
            scale_factor_bits: encoder_settings.scale_factor_bits,
            base_encoder: BaseEncoder::new(
                file_header.channels as usize,
                encoder_settings.scale_factor_bits as usize,
            ),
        }
    }

    pub fn get_lms(&self) -> &Vec<SeaLMS> {
        &self.base_encoder.lms
    }
}

impl SeaEncoderTrait for CbrEncoder {
    fn encode(&mut self, samples: &[i16], dequant_tab: &mut SeaDequantTab) -> EncodedSamples {
        let mut scale_factors =
            vec![0u8; samples.len().div_ceil(self.scale_factor_frames as usize)];
        let mut residuals = vec![0u8; samples.len()];

        let channels = self.file_header.channels as usize;

        let slice_size = self.scale_factor_frames as usize * channels;

        let residual_sizes = vec![self.residual_size; channels];

        for (slice_index, input_slice) in samples.chunks(slice_size).enumerate() {
            self.base_encoder.get_residuals_for_chunk(
                input_slice,
                &residual_sizes,
                &mut scale_factors[slice_index * channels..],
                &mut residuals[slice_index * slice_size..],
            );
        }

        EncodedSamples {
            scale_factors,
            residuals,
            residual_bits: vec![],
        }
    }
}
