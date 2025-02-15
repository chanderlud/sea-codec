use super::{
    chunk::{SeaChunk, SeaChunkType},
    common::clamp_i16,
    dqt::SeaDequantTab,
};

pub struct Decoder {
    channels: usize,
    scale_factor_bits: usize,

    dequant_tab: SeaDequantTab,
}

impl Decoder {
    pub fn init(channels: usize, scale_factor_bits: usize) -> Self {
        Self {
            channels,
            scale_factor_bits,

            dequant_tab: SeaDequantTab::init(scale_factor_bits),
        }
    }

    pub fn decode(&self, chunk: &SeaChunk) -> Vec<i16> {
        assert_eq!(chunk.scale_factor_bits as usize, self.scale_factor_bits);

        let mut output: Vec<i16> =
            Vec::with_capacity(chunk.file_header.frames_per_chunk as usize * self.channels);

        let mut lms = chunk.lms.clone();

        let dqts: Vec<Vec<Vec<i32>>> = (1..=8)
            .map(|i| self.dequant_tab.get_dqt(i).clone())
            .collect();

        for (frame_index, channel_residuals) in
            chunk.residuals.chunks_exact(self.channels).enumerate()
        {
            let scale_factor_index =
                (frame_index / chunk.scale_factor_frames as usize) * self.channels;

            for (channel_index, residual) in channel_residuals.iter().enumerate() {
                let residual_size: usize = if matches!(chunk.chunk_type, SeaChunkType::VBR) {
                    chunk.vbr_residual_sizes[scale_factor_index + channel_index] as usize
                } else {
                    chunk.residual_size as usize
                };

                let scale_factor = chunk.scale_factors[scale_factor_index + channel_index];

                let predicted = lms[channel_index].predict();

                let quantized: usize = *residual as usize;

                let dequantized =
                    dqts[residual_size as usize - 1][scale_factor as usize][quantized];

                let reconstructed = clamp_i16(predicted + dequantized);
                output.push(reconstructed);
                lms[channel_index].update(reconstructed as i16, dequantized);
            }
        }

        output
    }
}
