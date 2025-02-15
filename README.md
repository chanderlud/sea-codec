# SEA - Simple Embedded Audio Codec - WIP

This is a low complexity, lossy audio codec for embedded devices, inspired by the QOA codec.

Features:

- Main use case in low power embedded devices, game assets, live streaming
- Low complexity, time domain compression
- No low pass filtering, flat frequency response
- 1.3 - 8.5 bits per sample
- CBR and VBR
- Fixed frame length -> seekable in constant time
- Can store metadata
- MIT License

SEA file specification:

All values are stored in little-endian order.

# File header

- char[4] magic; // "SEAC";
- uint8_t version; // currently 0x01
- uint8_t number_of_channels; // 1 - 255
- uint16_t chunk_size; // in bytes
- uint16_t frames_per_chunk; // frame = the samples for all channels
- uint32_t sample_rate; // Hz
- uint32_t total_frames; // total samples per channel, 0 means streaming data until EOF
- uint32_t metadata_size; // metadata size in bytes, can be zero
- char\* metadata[metadata_size]; // metadata

# Metadata

- UTF-8 encoded string
- contains key=value pairs, separated by newline character('\n')
- key and value are separated by '='
- key is case insensitive, it cannot contain chars '=' or '\n'
- value is case sensitive, it can contain any chars, except newline character('\n')
- example:

```
author=John Doe
title=My Song
```

# Chunk

- Has fixed size + fixed number of frames stored to make the file seekable
- Slice count is variable
- Must be zero padded to match the chunk size

- uint8_t type; // CBR(0x01) || VBR(0x02)
- uint8_t scale_factor_and_residual_size; // scale_factor (4 bits) | residual_size (4 bits)
- uint8_t scale_factor_frames; // distance between scalefactor values
- uint8_t reserved; // currently = 0x5A

- struct {
  int16_t history[4]; // most recent last
  int16_t weights[4]; // most recent last
  } lms_state[channels_count];

- uint8_t packed_scale_factors[(scale_factor_bits * channels_count) / 8]; // interleaved samples

- VBR residual lengths - 2 bits per residual length. offset = -1 (0 = chunk_residual - 1, 1 = chunk_residual, 2 = chunk_residual + 1, 3 = chunk_residual + 2)

- uint8_t packed_residuals[] // interleaved samples

# License

MIT License
Copyright (c) 2025 Dani Biró
