use std::{io, rc::Rc};

use bytemuck::cast_slice;

use crate::codec::{
    common::{read_max_or_zero, SeaError},
    file::{SeaFile, SeaFileHeader},
};

pub enum SeaEncoderState {
    Start,
    WritingFrames,
    Finished,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncoderSettings {
    pub scale_factor_bits: u8,
    pub scale_factor_frames: u8,
    pub residual_bits: f32, // 1-8
    pub frames_per_chunk: u16,
    pub vbr: bool,
}

impl Default for EncoderSettings {
    fn default() -> Self {
        Self {
            frames_per_chunk: 5120,
            scale_factor_bits: 4,
            scale_factor_frames: 20,
            residual_bits: 3.0,
            vbr: false,
        }
    }
}

pub struct SeaEncoder<R, W> {
    reader: R,
    writer: W,
    file: SeaFile,
    state: SeaEncoderState,
    written_frames: u32,
}

impl<R, W> SeaEncoder<R, W>
where
    R: io::Read,
    W: io::Write,
{
    pub fn new(
        channels: u8,
        sample_rate: u32,
        total_frames: Option<u32>,
        settings: EncoderSettings,
        reader: R,
        mut writer: W,
    ) -> Result<Self, SeaError> {
        let header = SeaFileHeader {
            version: 1,
            channels,
            chunk_size: 0, // will be set later by the first chunk
            frames_per_chunk: settings.frames_per_chunk,
            sample_rate,
            total_frames: total_frames.unwrap_or(0),
            metadata: Rc::new(String::new()),
        };

        let file = SeaFile::new(header, &settings)?;

        let mut state = SeaEncoderState::Start;

        if let Some(total_frames) = total_frames {
            if total_frames == 0 {
                writer.write_all(&file.header.serialize())?;
                state = SeaEncoderState::WritingFrames;
            }
        }

        Ok(SeaEncoder {
            file,
            state,
            reader,
            writer,
            written_frames: 0,
        })
    }

    fn read_samples(&mut self, max_sample_count: usize) -> Result<Vec<i16>, SeaError> {
        let buffer_size = max_sample_count * std::mem::size_of::<i16>();
        let buffer = read_max_or_zero(&mut self.reader, buffer_size)?;

        if buffer.is_empty() {
            return Ok(Vec::new());
        }

        if buffer.len() % (std::mem::size_of::<i16>() * self.file.header.channels as usize) != 0 {
            return Err(SeaError::IoError(io::Error::from(
                io::ErrorKind::UnexpectedEof,
            )));
        }

        let samples: &[i16] = cast_slice(&buffer);
        Ok(samples.to_vec())
    }

    pub fn encode_frame(&mut self) -> Result<bool, SeaError> {
        if matches!(self.state, SeaEncoderState::Finished) {
            return Err(SeaError::EncoderClosed);
        }

        let channels = self.file.header.channels;
        let frames = if self.file.header.total_frames > 0 {
            (self.file.header.frames_per_chunk as usize)
                .min(self.file.header.total_frames as usize - self.written_frames as usize)
        } else {
            self.file.header.frames_per_chunk as usize
        };

        let full_size_samples =
            self.file.header.frames_per_chunk as usize * self.file.header.channels as usize;
        let samples_to_read = frames * channels as usize;
        let samples: Vec<i16> = self.read_samples(samples_to_read)?;
        let eof: bool = samples.is_empty() || samples.len() < full_size_samples;

        if !samples.is_empty() {
            let encoded_chunk = self.file.make_chunk(&samples)?;

            if eof {
                assert!(encoded_chunk.len() <= self.file.header.chunk_size as usize);
            } else {
                assert_eq!(encoded_chunk.len(), self.file.header.chunk_size as usize);
            }

            // we need to write file header after the first chunk is generated
            if matches!(self.state, SeaEncoderState::Start) {
                self.writer.write_all(&self.file.header.serialize())?;
                self.state = SeaEncoderState::WritingFrames;
            }

            self.writer.write_all(&encoded_chunk)?;
            self.written_frames += frames as u32;
        }

        if eof {
            self.state = SeaEncoderState::Finished;
        }

        Ok(!eof)
    }

    pub fn flush(&mut self) {
        let _ = self.writer.flush();
    }

    pub fn finalize(&mut self) -> Result<(), SeaError> {
        self.writer.flush()?;
        self.state = SeaEncoderState::Finished;
        Ok(())
    }
}
