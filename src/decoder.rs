use std::io;

use bytemuck::cast_slice;

use crate::codec::{
    common::SeaError,
    file::{SeaFile, SeaFileHeader},
};

pub struct SeaDecoder<R, W> {
    reader: R,
    writer: W,
    file: SeaFile,
    frames_read: usize,
}

impl<R, W> SeaDecoder<R, W>
where
    R: io::Read,
    W: io::Write,
{
    pub fn new(mut reader: R, writer: W) -> Result<Self, SeaError> {
        let file = SeaFile::from_reader(&mut reader)?;

        Ok(Self {
            reader,
            writer,
            file,
            frames_read: 0,
        })
    }

    pub fn decode_frame(&mut self) -> Result<bool, SeaError> {
        if self.file.header.total_frames != 0
            && (self.file.header.total_frames as usize) <= self.frames_read
        {
            return Ok(false);
        }

        let remaining_frames = if self.file.header.total_frames > 0 {
            Some(self.file.header.total_frames as usize - self.frames_read)
        } else {
            None
        };

        let reader_res = self
            .file
            .samples_from_reader(&mut self.reader, remaining_frames)?;

        match reader_res {
            Some(samples) => {
                self.frames_read += samples.len() / self.file.header.channels as usize;
                let samples_u8: &[u8] = cast_slice(&samples);
                self.writer.write_all(samples_u8)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn flush(&mut self) {
        let _ = self.writer.flush();
    }

    pub fn finalize(&mut self) -> Result<(), SeaError> {
        self.writer.flush()?;
        Ok(())
    }

    pub fn get_header(&self) -> SeaFileHeader {
        self.file.header.clone()
    }
}
