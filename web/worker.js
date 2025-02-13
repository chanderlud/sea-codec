let wasm;

(async () => {
  const wasmModule = await WebAssembly.instantiateStreaming(fetch("codec.wasm"), {
    env: {
      memory: new WebAssembly.Memory({ initial: 256 }), // 16 MB
      js_error: (start) => {
        const view = new Uint8Array(wasmModule.instance.exports.memory.buffer);
        let end = view.indexOf(0, start);
        if (end === -1) throw new Error("Got invalid string from wasm");
        const str = new TextDecoder().decode(view.subarray(start, end));
        throw new Error(str);
      },
    },
  });
  const wasmExports = wasmModule.instance.exports;
  wasmExports.setup();

  wasm = {
    wasm_sea_encode: (inputSamples, sampleRate, channels, quality, vbr) => {
      if (!(inputSamples instanceof Int16Array))
        throw new Error("inputSamples should be Int16Array");

      let wasmInputBufferSize;
      let wasmInputBuffer;
      let wasmOutputBufferSize;
      let wasmOutputBuffer;

      try {
        wasmInputBufferSize = inputSamples.byteLength;
        wasmInputBuffer = wasmExports.allocate(wasmInputBufferSize);
        const inputSamplesU8 = new Uint8Array(
          inputSamples.buffer,
          inputSamples.byteOffset,
          inputSamples.byteLength
        ).slice();

        new Uint8Array(wasmExports.memory.buffer).set(inputSamplesU8, wasmInputBuffer);

        wasmOutputBufferSize = Math.floor(inputSamples.byteLength / 2);
        wasmOutputBuffer = wasmExports.allocate(wasmOutputBufferSize);

        const outputLength = wasmExports.wasm_sea_encode(
          wasmInputBuffer,
          wasmInputBufferSize,
          sampleRate,
          channels,
          quality,
          vbr,
          wasmOutputBuffer,
          wasmOutputBufferSize
        );

        if (outputLength === 0) throw new Error("Encoding failed: Output buffer too small.");

        const output = new Uint8Array(
          wasmExports.memory.buffer,
          wasmOutputBuffer,
          outputLength
        ).slice();

        return output;
      } catch (e) {
        postMessage(["error", e.message]);
        throw e;
      } finally {
        wasmExports.deallocate(wasmInputBuffer, wasmInputBufferSize);
        wasmExports.deallocate(wasmOutputBuffer, wasmOutputBufferSize);
      }
    },
    wasm_sea_decode: (encodedData) => {
      if (!(encodedData instanceof Uint8Array)) throw new Error("encodedData should be Uint8Array");

      let wasmInputBufferSize;
      let wasmInputBuffer;
      let wasmOutputBufferSize;
      let wasmOutputBuffer;

      try {
        wasmInputBufferSize = encodedData.byteLength;
        wasmInputBuffer = wasmExports.allocate(wasmInputBufferSize);
        new Uint8Array(wasmExports.memory.buffer).set(encodedData, wasmInputBuffer);

        wasmOutputBufferSize = wasmInputBufferSize * 10;
        wasmOutputBuffer = wasmExports.allocate(wasmOutputBufferSize);

        wasmSampleRateBuffer = wasmExports.allocate(4);
        wasmChannelsBuffer = wasmExports.allocate(4);

        const outputLength = wasmExports.wasm_sea_decode(
          wasmInputBuffer,
          wasmInputBufferSize,
          wasmOutputBuffer,
          wasmOutputBufferSize,
          wasmSampleRateBuffer,
          wasmChannelsBuffer
        );

        if (outputLength === 0) throw new Error("Decoding failed: Got zero output length.");

        const output = new Int16Array(
          wasmExports.memory.buffer,
          wasmOutputBuffer,
          outputLength / 2
        ).slice();

        const sampleRate = new Uint32Array(wasmExports.memory.buffer, wasmSampleRateBuffer, 1)[0];
        const channels = new Uint32Array(wasmExports.memory.buffer, wasmChannelsBuffer, 1)[0];

        if (sampleRate === 0 || channels === 0)
          throw new Error("Decoding failed: Got invalid output.");

        return {
          samples: output,
          sampleRate,
          channels,
        };
      } catch (e) {
        postMessage(["error", e.message]);
        throw e;
      } finally {
        wasmExports.deallocate(wasmInputBuffer, wasmInputBufferSize);
        wasmExports.deallocate(wasmOutputBuffer, wasmOutputBufferSize);
        wasmExports.deallocate(wasmSampleRateBuffer, 4);
        wasmExports.deallocate(wasmChannelsBuffer, 4);
      }
    },
  };

  // warm up JIT
  const encodedData = wasm.wasm_sea_encode(new Int16Array(1024 * 1024), 44100, 1, 3, false);
  wasm.wasm_sea_decode(encodedData);
})();

function encodeWAV(samples, sampleRate, channels) {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);

  const writeString = (offset, string) => {
    for (let i = 0; i < string.length; i++) {
      view.setUint8(offset + i, string.charCodeAt(i));
    }
  };

  writeString(0, "RIFF");
  view.setUint32(4, 36 + samples.length * 2, true);
  writeString(8, "WAVE");
  writeString(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, channels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * channels * 2, true);
  view.setUint16(32, channels * 2, true);
  view.setUint16(34, 16, true);
  writeString(36, "data");
  view.setUint32(40, samples.length * 2, true);

  let sampleBuffer = new Int16Array(buffer, 44, samples.length);
  sampleBuffer.set(samples);

  return new Uint8Array(buffer);
}

function channelSamplesToInterleavedInt16(channelSamples, channels) {
  const interleavedSamples = new Int16Array(channelSamples[0].length * channels);

  for (let channel = 0; channel < channels; channel++) {
    for (let i = 0; i < channelSamples[0].length; i++) {
      let clamped = Math.max(-1, Math.min(1, channelSamples[channel][i]));
      let i16 = Math.floor(clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff);
      interleavedSamples[i * channels + channel] = i16;
    }
  }

  return interleavedSamples;
}

function getPSNR(a, b) {
  if (a.length !== b.length) {
    throw new Error("Size mismatch");
  }
  let sum = 0;
  for (let i = 0; i < a.length; i++) {
    const diff = a[i] / 32768 - b[i] / 32768;
    sum += diff * diff;
  }

  let rms = Math.sqrt(sum / a.length);
  let psnr = -20 * Math.log10(2.0 / rms);
  return psnr;
}

let exports = {
  channelsToInterleaved(channelSamples, channels) {
    const interleavedSamples = channelSamplesToInterleavedInt16(channelSamples, channels);
    return interleavedSamples;
  },

  encode(interleavedSamples, sampleRate, channels, quality, vbr) {
    const start = performance.now();
    const encoded = wasm.wasm_sea_encode(interleavedSamples, sampleRate, channels, quality, vbr);
    const end = performance.now();

    return {
      encoded,
      duration: end - start,
    };
  },

  decode(encodedData, originalData) {
    const start = performance.now();
    const { samples, sampleRate, channels } = wasm.wasm_sea_decode(encodedData);
    const end = performance.now();
    const wave = encodeWAV(samples, sampleRate, channels);
    let psnr = 0;
    if (originalData) {
      psnr = getPSNR(originalData, samples);
    }

    return {
      wave,
      sampleRate,
      channels,
      duration: end - start,
      psnr,
    };
  },
};

addEventListener("message", (e) => {
  const { data } = e;
  const [id, fn, ...rest] = data;

  if (fn in exports) {
    const res = exports[fn](...rest);
    postMessage([id, res]);
  } else {
    throw new Error(`Unknown function: ${fn}`);
  }
});
