/*
    SEA - Simple Embedded Audio Codec
    Copyright (C) 2025 Dani Biró
    MIT License

    WARNING: This is just a proof of concept code, without error checking. Use it at your own risk.
    The Rust version is much more robust and has error checking.
*/

#ifndef SEA_H
#define SEA_H

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define SEA_MIN(a, b) ((a) < (b) ? (a) : (b))
#define SEAC_MAGIC_REV 0x63616573 // 'seac' in little endian

static inline uint8_t read_u8(const uint8_t** buffer)
{
    uint8_t value = **buffer;
    (*buffer)++;
    return value;
}

static inline int16_t read_i16_le(const uint8_t** buffer)
{
    int16_t value = (*buffer)[0] | ((*buffer)[1] << 8);
    *buffer += 2;
    return value;
}

static inline uint16_t read_u16_le(const uint8_t** buffer)
{
    uint16_t value = (*buffer)[0] | ((*buffer)[1] << 8);
    *buffer += 2;
    return value;
}

static inline uint32_t read_u32_le(const uint8_t** buffer)
{
    uint32_t value = (*buffer)[0] | ((*buffer)[1] << 8) | ((*buffer)[2] << 16) | ((*buffer)[3] << 24);
    *buffer += 4;
    return value;
}

typedef struct {
    int32_t history[4];
    int32_t weights[4];
} SEA_LMS;

static void sea_read_unpack_bits(uint8_t bit_size, const uint8_t** encoded, uint32_t bytes_to_read, uint8_t* output)
{
    const uint32_t MASKS[9] = { 0, 1, 3, 7, 15, 31, 63, 127, 255 };
    uint32_t bits_stored = 0, carry = 0;
    uint32_t output_len = 0;

    for (int i = 0; i < bytes_to_read; i++) {
        uint32_t v = (carry << 8) | read_u8(encoded);
        bits_stored += 8;
        while (bits_stored >= bit_size) {
            output[output_len++] = (v >> (bits_stored - bit_size)) & MASKS[bit_size];
            bits_stored -= bit_size;
        }
        carry = v & ((1 << bits_stored) - 1);
    }
}

static inline uint32_t div_ceil(uint32_t a, uint32_t b)
{
    return (a + b - 1) / b;
}

static int32_t* SEA_DQT = NULL;
static uint32_t SEA_DQT_MULTIPLIER = 0;

static void alloc_prepare_dqt(uint32_t scale_factor_bits, uint32_t residual_bits)
{
    static const float IDEAL_POW_FACTOR[8] = { 12.0f, 11.65f, 11.20f, 10.58f, 9.64f, 8.75f, 7.66f, 6.63f };

    uint32_t scale_factor_items = 1 << scale_factor_bits;
    uint32_t dqt_len = 1 << (residual_bits - 1);

    float power_factor = IDEAL_POW_FACTOR[residual_bits - 1] / (float)scale_factor_bits;
    int32_t* scale_factors = (int32_t*)malloc(scale_factor_items * sizeof(int32_t));
    for (uint32_t i = 0; i < scale_factor_items; ++i) {
        scale_factors[i] = (int32_t)powf((float)(i + 1), power_factor);
    }

    // Generate DQT table
    float* dqt = (float*)malloc(dqt_len * sizeof(float));
    if (residual_bits == 1) {
        dqt[0] = 2.0f;
    } else if (residual_bits == 2) {
        dqt[0] = 1.115f;
        dqt[1] = 4.0f;
    } else {
        float start = 0.75f;
        float end = (float)((1 << residual_bits) - 1);
        float step = floorf((end - start) / (dqt_len - 1));
        dqt[0] = start;
        for (uint32_t i = 1; i < dqt_len - 1; ++i) {
            dqt[i] = 0.5f + i * step;
        }
        dqt[dqt_len - 1] = end;
    }

    SEA_DQT = (int32_t*)malloc(scale_factor_items * dqt_len * 2 * sizeof(int32_t));
    uint32_t idx = 0;
    for (uint32_t s = 0; s < scale_factor_items; ++s) {
        for (uint32_t q = 0; q < dqt_len; ++q) {
            int32_t val = (int32_t)roundf(scale_factors[s] * dqt[q]);
            SEA_DQT[idx++] = val;
            SEA_DQT[idx++] = -val;
        }
    }

    SEA_DQT_MULTIPLIER = dqt_len * 2;

    // Clean up
    free(scale_factors);
    free(dqt);
}

static inline int16_t clamp_i16(int32_t value)
{
    if (value > INT16_MAX) {
        return INT16_MAX;
    } else if (value < INT16_MIN) {
        return INT16_MIN;
    }
    return (int16_t)value;
}

static inline int32_t sea_lms_predict(const SEA_LMS* lms)
{
    int32_t prediction = 0;

    for (int i = 0; i < 4; ++i) {
        prediction += lms->weights[i] * lms->history[i];
    }

    return prediction >> 13;
}

static inline void sea_lms_update(SEA_LMS* lms, int16_t sample, int32_t residual)
{
    int32_t delta = residual >> 4;

    for (int i = 0; i < 4; ++i) {
        if (lms->history[i] < 0) {
            lms->weights[i] -= delta;
        } else {
            lms->weights[i] += delta;
        }
    }

    for (int i = 1; i < 4; ++i) {
        lms->history[i - 1] = lms->history[i];
    }
    lms->history[3] = (int32_t)sample;
}

static int sea_read_chunk(const uint8_t** encoded, uint32_t channels, uint32_t frames_in_this_chunk, int16_t** output)
{
    uint8_t type = read_u8(encoded);
    if (type != 0x01) {
        fprintf(stderr, "Only CBR supported\n");
        return 1;
    }
    uint8_t scale_factor_and_residual_size = read_u8(encoded);
    uint8_t scale_factor_bits = scale_factor_and_residual_size >> 4;
    uint8_t residual_size = scale_factor_and_residual_size & 0xF;
    uint8_t scale_factor_frames = read_u8(encoded);
    uint8_t reserved = read_u8(encoded);
    if (reserved != 0x5A) {
        fprintf(stderr, "Invalid file\n");
        return 1;
    }

    alloc_prepare_dqt(scale_factor_bits, residual_size);

    SEA_LMS* lms = (SEA_LMS*)malloc(channels * sizeof(SEA_LMS));
    for (int channel_id = 0; channel_id < channels; channel_id++) {
        for (int j = 0; j < 4; j++) {
            lms[channel_id].history[j] = read_i16_le(encoded);
        }
        for (int j = 0; j < 4; j++) {
            lms[channel_id].weights[j] = read_i16_le(encoded);
        }
    }

    uint32_t scale_factor_items = div_ceil(frames_in_this_chunk, scale_factor_frames) * channels;
    uint8_t* scale_factors = (uint8_t*)malloc(scale_factor_items + 8);
    uint32_t scale_factor_bytes = div_ceil(scale_factor_items * scale_factor_bits, 8);
    sea_read_unpack_bits(scale_factor_bits, encoded, scale_factor_bytes, scale_factors);

    uint32_t residual_bytes = div_ceil(frames_in_this_chunk * residual_size * channels, 8);
    uint8_t* residuals = (uint8_t*)malloc(frames_in_this_chunk * channels + 8);
    sea_read_unpack_bits(residual_size, encoded, residual_bytes, residuals);

    for (int scale_factor_offset = 0; scale_factor_offset < scale_factor_items; scale_factor_offset += channels) {
        uint8_t* scale_factor_residuals = &residuals[scale_factor_offset * scale_factor_frames];

        for (int frame_index = 0; frame_index < scale_factor_frames; frame_index++) {
            const uint8_t* subchunk_residuals = &scale_factor_residuals[frame_index * channels];

            for (int channel_index = 0; channel_index < channels; ++channel_index) {
                uint8_t scale_factor = scale_factors[scale_factor_offset + channel_index];
                int32_t predicted = sea_lms_predict(&lms[channel_index]);
                uint32_t quantized = (uint32_t)subchunk_residuals[channel_index];
                int32_t dequantized = SEA_DQT[scale_factor * SEA_DQT_MULTIPLIER + quantized];
                int32_t reconstructed = clamp_i16(predicted + dequantized);

                **output = reconstructed;
                *output += 1;

                sea_lms_update(&lms[channel_index], reconstructed, dequantized);
            }
        }
    }

    free(lms);
    free(SEA_DQT);
    free(residuals);
    free(scale_factors);

    return 0;
}

int sea_decode(uint8_t* encoded, uint32_t encoded_len, uint32_t* sample_rate, uint32_t* channels, int16_t* output, uint32_t* total_frames)
{
    const uint8_t** encoded_ptr = (const uint8_t**)&encoded;

    uint32_t magic = read_u32_le(encoded_ptr);
    uint8_t version = read_u8(encoded_ptr);

    if (magic != SEAC_MAGIC_REV || version != 1) {
        fprintf(stderr, "Invalid file\n");
        return 1;
    }

    *channels = read_u8(encoded_ptr);
    uint16_t chunk_size = read_u16_le(encoded_ptr);
    uint16_t frames_per_chunk = read_u16_le(encoded_ptr);
    *sample_rate = read_u32_le(encoded_ptr);
    *total_frames = read_u32_le(encoded_ptr);
    uint32_t metadata_len = read_u32_le(encoded_ptr);
    encoded_ptr += metadata_len;

    if (output == NULL) {
        return 0;
    }

    uint32_t read_frames = 0;
    int16_t** output_ptr = (int16_t**)&output;
    while (read_frames < *total_frames) {
        uint32_t frames_in_chunk = SEA_MIN(frames_per_chunk, *total_frames - read_frames);
        uint32_t written_samples = sea_read_chunk(encoded_ptr, *channels, frames_in_chunk, output_ptr);
        if (written_samples != 0) {
            fprintf(stderr, "Decode error\n");
            return 2;
        }
        read_frames += frames_in_chunk;
    }

    return 0;
}

#endif
