use std::io::Cursor;

use bytemuck::cast_slice;
use codec::{
    decoder::SeaDecoder,
    encoder::{EncoderSettings, SeaEncoder},
};

pub mod codec;
pub mod wasm_api;

pub fn sea_encode(
    input_samples: &[i16],
    sample_rate: u32,
    channels: u32,
    settings: EncoderSettings,
) -> Vec<u8> {
    let u8_input_samples: &[u8] = cast_slice(input_samples);
    let mut cursor: Cursor<_> = Cursor::new(u8_input_samples);
    let mut sea_encoded = Vec::<u8>::with_capacity(input_samples.len());
    let mut sea_encoder = SeaEncoder::new(
        channels as u8,
        sample_rate,
        Some(input_samples.len() as u32 / channels),
        settings,
        &mut cursor,
        &mut sea_encoded,
    )
    .unwrap();

    while sea_encoder.encode_frame().unwrap() {}
    sea_encoder.finalize().unwrap();

    sea_encoded
}

pub struct SeaDecodeInfo {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u32,
}

pub fn sea_decode(encoded: &[u8]) -> SeaDecodeInfo {
    let mut cursor: Cursor<&[u8]> = Cursor::new(encoded);
    let mut sea_decoded = Vec::<u8>::with_capacity(encoded.len() * 8);

    let mut sea_decoder: SeaDecoder<&mut Cursor<&[u8]>, &mut Vec<u8>> =
        SeaDecoder::new(&mut cursor, &mut sea_decoded).unwrap();

    while sea_decoder.decode_frame().unwrap() {}
    sea_decoder.finalize().unwrap();

    let header = sea_decoder.get_header();

    let decoded: &[i16] = cast_slice(&sea_decoded);

    SeaDecodeInfo {
        samples: decoded.to_vec(),
        sample_rate: header.sample_rate,
        channels: header.channels as u32,
    }
}
