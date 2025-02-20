use std::{
    cell::RefCell,
    io::{self, Cursor, Read, Write},
    rc::Rc,
};

use bytemuck::cast_slice;
use helpers::{encode_decode, gen_test_signal, TEST_SAMPLE_RATE};
use sea_codec::{
    decoder::SeaDecoder,
    encoder::{EncoderSettings, SeaEncoder},
};

extern crate sea_codec;

mod helpers;

#[derive(Clone)]
struct SharedBuffer {
    buffer: Rc<RefCell<Vec<u8>>>,
}

impl SharedBuffer {
    fn new(capacity: usize) -> Self {
        SharedBuffer {
            buffer: Rc::new(RefCell::new(Vec::with_capacity(capacity))),
        }
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buffer.borrow_mut().flush()
    }
}

impl Read for SharedBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut vec = self.buffer.borrow_mut();
        let amount = buf.len().min(vec.len());
        buf[..amount].copy_from_slice(&vec[..amount]);
        vec.drain(..amount);
        Ok(amount)
    }
}

#[test]
fn streaming() {
    let channels = 1;
    let input_samples = gen_test_signal(channels, TEST_SAMPLE_RATE as usize);

    let reference_samples = encode_decode(
        &input_samples,
        TEST_SAMPLE_RATE,
        channels,
        EncoderSettings::default(),
    );

    let u8_input_samples: &[u8] = cast_slice(&input_samples);
    let mut input_cursor: Cursor<_> = Cursor::new(u8_input_samples);

    let sea_encoded = SharedBuffer::new(input_samples.len());
    let mut sea_encoded_clone = sea_encoded.clone();

    let mut sea_encoder = SeaEncoder::new(
        channels as u8,
        TEST_SAMPLE_RATE,
        None,
        EncoderSettings::default(),
        &mut input_cursor,
        &mut sea_encoded_clone,
    )
    .unwrap();

    // need to encode first frame to get the header
    sea_encoder.encode_frame().unwrap();

    let mut sea_decoded = Vec::<u8>::with_capacity(input_samples.len() * 2);
    let sea_encoded_dec_clone = sea_encoded.clone();
    let mut sea_decoder = SeaDecoder::new(sea_encoded_dec_clone, &mut sea_decoded).unwrap();

    for _ in 0..3 {
        sea_encoder.encode_frame().unwrap();
        sea_decoder.decode_frame().unwrap();
    }

    let i16_sea_decoded: &[i16] = cast_slice(&sea_decoded);
    assert!(i16_sea_decoded.len() > 0);
    assert_eq!(
        reference_samples.decoded[..i16_sea_decoded.len()],
        i16_sea_decoded[..]
    );
}
