use std::usize;

use crate::codec::{bits::BitUnpacker, common::clamp_i16, lms::LMS_LEN};

use super::{
    bits::BitPacker,
    common::{SeaDequantTab, SeaError, SeaResidualSize},
    encoder::EncoderSettings,
    file::SeaFileHeader,
    lms::SeaLMS,
};

#[derive(Debug, Clone, Copy)]
pub enum SeaChunkType {
    CBR = 0x01,
    VBR = 0x02,
}

#[derive(Debug)]
pub struct SeaChunk {
    file_header: SeaFileHeader,
    chunk_type: SeaChunkType,

    pub scale_factor_bits: u8,
    pub scale_factor_frames: u8,
    pub residual_size: SeaResidualSize,

    pub lms: Vec<SeaLMS>,

    pub scale_factors: Vec<u8>,
    pub vbr_residual_sizes: Vec<u8>,
    pub residuals: Vec<u8>,
}

impl SeaChunk {
    pub fn new(
        file_header: &SeaFileHeader,
        lms: &Vec<SeaLMS>,
        encoder_settings: &EncoderSettings,
        scale_factors: Vec<u8>,
        vbr_residual_sizes: Vec<u8>,
        residuals: Vec<u8>,
    ) -> SeaChunk {
        let is_vbr = vbr_residual_sizes.len() > 0;

        SeaChunk {
            file_header: file_header.clone(),
            chunk_type: if is_vbr {
                SeaChunkType::VBR
            } else {
                SeaChunkType::CBR
            },
            scale_factor_bits: encoder_settings.scale_factor_bits,
            scale_factor_frames: encoder_settings.scale_factor_frames,
            residual_size: SeaResidualSize::from(encoder_settings.residual_bits.floor() as u8),

            lms: lms.clone(),
            scale_factors,
            vbr_residual_sizes,
            residuals,
        }
    }

    pub fn from_slice(
        encoded: &[u8],
        file_header: &SeaFileHeader,
        remaining_frames: Option<usize>,
    ) -> Result<Self, SeaError> {
        assert!(encoded.len() <= file_header.chunk_size as usize);

        // we cannot calculate last frame size in streaming mode
        if remaining_frames.is_none() && encoded.len() < file_header.chunk_size as usize {
            return Err(SeaError::InvalidFrame);
        }

        let chunk_type: SeaChunkType = match encoded[0] {
            0x01 => SeaChunkType::CBR,
            0x02 => SeaChunkType::VBR,
            _ => return Err(SeaError::InvalidFile),
        };

        let scale_factor_bits = encoded[1] >> 4;
        let residual_size = SeaResidualSize::from(encoded[1] & 0b1111);
        let scale_factor_frames = encoded[2];
        let _reserved = encoded[3];

        let mut encoded_index = 4;

        let mut lms: Vec<SeaLMS> = vec![];
        for _ in 0..file_header.channels as usize {
            lms.push(SeaLMS::from_bytes(
                &encoded[encoded_index..encoded_index + LMS_LEN * 4]
                    .try_into()
                    .unwrap(),
            ));
            encoded_index += LMS_LEN * 4;
        }

        let frames_in_this_chunk =
            (file_header.frames_per_chunk as usize).min(remaining_frames.unwrap_or(usize::MAX));

        let scale_factor_items = frames_in_this_chunk.div_ceil(scale_factor_frames as usize)
            * file_header.channels as usize;

        let scale_factors = {
            let packed_scale_factor_bytes =
                (scale_factor_items * scale_factor_bits as usize).div_ceil(8);

            let packed_scale_factors =
                &encoded[encoded_index..encoded_index + packed_scale_factor_bytes];
            encoded_index += packed_scale_factor_bytes;

            let mut unpacker = BitUnpacker::new_const_bits(scale_factor_bits as u8);
            unpacker.process_bytes(&packed_scale_factors);
            let mut res = unpacker.finish();
            res.resize(scale_factor_items, 0);
            res
        };

        let vbr_residual_sizes: Vec<u8> = if matches!(chunk_type, SeaChunkType::VBR) {
            let packed_vbr_residual_sizes_bytes = (scale_factor_items * 2).div_ceil(8);
            let packed_vbr_residual_sizes =
                &encoded[encoded_index..encoded_index + packed_vbr_residual_sizes_bytes];
            encoded_index += packed_vbr_residual_sizes_bytes;

            let mut unpacker: BitUnpacker = BitUnpacker::new_const_bits(2);
            unpacker.process_bytes(&packed_vbr_residual_sizes);
            let mut res = unpacker.finish();
            res.resize(scale_factor_items, 0);
            for i in 0..res.len() {
                res[i] += residual_size as u8 - 1;
            }
            res
        } else {
            Vec::new()
        };

        let residuals: Vec<u8> = {
            let mut unpacker = if matches!(chunk_type, SeaChunkType::VBR) {
                let mut bitlengths = Vec::new();
                for vbr_chunk in vbr_residual_sizes.chunks_exact(file_header.channels as usize) {
                    for _ in 0..scale_factor_frames {
                        for channel_index in 0..file_header.channels as usize {
                            bitlengths.push(vbr_chunk[channel_index] as u8);
                        }
                    }
                }

                BitUnpacker::new_var_bits(&bitlengths)
            } else {
                BitUnpacker::new_const_bits(residual_size as u8)
            };

            let packed_residuals_bytes = if matches!(chunk_type, SeaChunkType::VBR) {
                let mut residual_bits: u32 = vbr_residual_sizes
                    [..vbr_residual_sizes.len() - file_header.channels as usize]
                    .iter()
                    .map(|x| *x as u32)
                    .sum();

                residual_bits *= scale_factor_frames as u32;

                let last_frame_samples = frames_in_this_chunk as u32 % scale_factor_frames as u32;
                let multiplier = if last_frame_samples == 0 {
                    scale_factor_frames as u32
                } else {
                    last_frame_samples
                };

                for size in vbr_residual_sizes
                    [(vbr_residual_sizes.len() - file_header.channels as usize)..]
                    .iter()
                {
                    residual_bits += *size as u32 * multiplier;
                }

                let residual_bytes = residual_bits.div_ceil(8);
                residual_bytes as usize
            } else {
                (frames_in_this_chunk * residual_size as usize * file_header.channels as usize)
                    .div_ceil(8)
            };

            let packed_residuals = &encoded[encoded_index..encoded_index + packed_residuals_bytes];

            unpacker.process_bytes(&packed_residuals);

            let mut res = unpacker.finish();
            res.resize(frames_in_this_chunk * file_header.channels as usize, 0);
            res
        };

        Ok(Self {
            file_header: file_header.clone(),
            chunk_type,
            scale_factor_bits,
            scale_factor_frames,
            residual_size,

            lms,
            scale_factors,
            vbr_residual_sizes,
            residuals,
        })
    }

    pub fn decode(&self, dequant_tab: &mut SeaDequantTab) -> Vec<i16> {
        let mut output: Vec<i16> = Vec::with_capacity(
            self.file_header.frames_per_chunk as usize * self.file_header.channels as usize,
        );

        let mut lms = self.lms.clone();

        let dqts: Vec<Vec<Vec<i32>>> = (1..=8)
            .map(|i| {
                dequant_tab
                    .get_dqt(self.scale_factor_bits as usize, i)
                    .clone()
            })
            .collect();

        for (frame_index, channel_residuals) in self
            .residuals
            .chunks_exact(self.file_header.channels as usize)
            .enumerate()
        {
            let scale_factor_index = (frame_index / self.scale_factor_frames as usize)
                * self.file_header.channels as usize;

            for (channel_index, residual) in channel_residuals.iter().enumerate() {
                let residual_size: usize = if matches!(self.chunk_type, SeaChunkType::VBR) {
                    self.vbr_residual_sizes[scale_factor_index + channel_index] as usize
                } else {
                    self.residual_size as usize
                };

                let scale_factor = self.scale_factors[scale_factor_index + channel_index];

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

    fn serialize_header(&self) -> [u8; 4] {
        assert!(self.scale_factor_bits > 0);
        assert!(self.scale_factor_frames > 0);
        assert!(
            self.file_header.frames_per_chunk as usize % self.scale_factor_frames as usize == 0
        );

        [
            self.chunk_type as u8,
            (self.scale_factor_bits << 4) as u8 | self.residual_size as u8,
            self.scale_factor_frames,
            0x5A,
        ]
    }

    fn serialize_lms(&self) -> Vec<u8> {
        assert_eq!(self.file_header.channels as usize, self.lms.len());

        self.lms
            .iter()
            .flat_map(|lms| lms.serialize())
            .collect::<Vec<_>>()
    }

    fn serialize_scale_factors(&self) -> Vec<u8> {
        let mut packer = BitPacker::new();
        for scale_factor in self.scale_factors.iter() {
            packer.push(*scale_factor as u32, self.scale_factor_bits);
        }
        packer.finish()
    }

    fn serialize_vbr_residual_sizes(&self) -> Vec<u8> {
        let mut packer = BitPacker::new();
        for vbr_residual_size in self.vbr_residual_sizes.iter() {
            let relative_size = *vbr_residual_size as i32 - self.residual_size as i32 + 1;
            packer.push(relative_size as u32, 2);
        }
        packer.finish()
    }

    fn serialize_residuals(&self) -> Vec<u8> {
        let mut packer = BitPacker::new();
        if matches!(self.chunk_type, SeaChunkType::VBR) {
            let mut vbr_residual_index = 0;
            let mut frames_written_since_update = 0;
            for residual in self
                .residuals
                .chunks_exact(self.file_header.channels as usize)
            {
                for channel_index in 0..self.file_header.channels as usize {
                    packer.push(
                        residual[channel_index] as u32,
                        self.vbr_residual_sizes[vbr_residual_index + channel_index] as u8,
                    );
                }
                frames_written_since_update += 1;
                if frames_written_since_update == self.scale_factor_frames {
                    vbr_residual_index += self.file_header.channels as usize;
                    frames_written_since_update = 0;
                }
            }
        } else {
            for residual in self.residuals.iter() {
                packer.push(*residual as u32, self.residual_size as u8);
            }
        }
        let res = packer.finish();
        res
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut output = Vec::new();

        output.extend_from_slice(&self.serialize_header());
        output.extend_from_slice(&self.serialize_lms());
        output.extend_from_slice(&self.serialize_scale_factors());
        if matches!(self.chunk_type, SeaChunkType::VBR) {
            output.extend_from_slice(&self.serialize_vbr_residual_sizes());
        }
        output.extend_from_slice(&self.serialize_residuals());

        output
    }
}
