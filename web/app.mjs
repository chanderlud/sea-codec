import { decodeAudioFile } from "./dist/deps.modern.js";

const worker = (() => {
  const w = new Worker("worker.js");
  const callbacks = {};
  let id = 1;
  w.onmessage = (e) => {
    const [id, ...args] = e.data;
    if (id === "error") {
      document.getElementById("error").textContent = e.message;
      return;
    }
    const cb = callbacks[id];
    setTimeout(cb(...args), 0);
    delete callbacks[id];
  };
  return {
    call: (fn, ...args) => {
      return new Promise((resolve, reject) => {
        callbacks[id] = resolve;
        w.postMessage([id, fn, ...args]);
      });
    },
  };
})();

function readFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error);
    reader.readAsArrayBuffer(file);
  });
}

function downloadFile(data, filename, mimeType) {
  const blob = new Blob([data], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

function formatNumber(x) {
  return x.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

document.getElementById("encode-submit").addEventListener("click", async () => {
  const fileInput = document.getElementById("encode-input");
  if (!fileInput.files.length) return alert("Please select a file.");

  const {
    samples: interleavedInput,
    sampleRate,
    channels,
  } = await (async () => {
    const file = fileInput.files[0];
    const arrayBuffer = await readFile(file);
    const audioBuffer = await decodeAudioFile(arrayBuffer);
    const samplesPerChannel = [...Array(audioBuffer.numberOfChannels).keys()].map((i) =>
      audioBuffer.getChannelData(i)
    );

    return {
      samples: await worker.call(
        "channelsToInterleaved",
        samplesPerChannel,
        audioBuffer.numberOfChannels
      ),
      sampleRate: audioBuffer.sampleRate,
      channels: audioBuffer.numberOfChannels,
    };
  })();

  const quality = parseInt(document.querySelector('select[name="quality"]').value);
  const vbr = false;

  document.getElementById("encode-submit").disabled = true;
  document.getElementById("encode-processing").classList.remove("hidden");
  document.getElementById("encode-details").innerHTML = "";

  const { encoded, duration: encodeDuration } = await worker.call(
    "encode",
    interleavedInput,
    sampleRate,
    channels,
    quality,
    vbr
  );

  const {
    wave: decodedWave,
    duration: decodeDuration,
    psnr,
  } = await worker.call("decode", encoded, interleavedInput);

  const status = `
  PCM 16 bit size ${formatNumber(interleavedInput.length * 2)} bytes <br />
  Compressed size: ${formatNumber(encoded.length)} bytes (${(
    (encoded.length / (interleavedInput.length * 2)) *
    100
  ).toFixed(2)} %) <br />
  Encoding took ${encodeDuration.toFixed(2)} ms <br/>
  Decoding took ${decodeDuration.toFixed(2)} ms <br />
  PSNR ${psnr.toFixed(2)} dB <br/>`;

  document.getElementById("encode-details").innerHTML = status;

  const audioUrl = URL.createObjectURL(new Blob([decodedWave], { type: "audio/wav" }));
  const audioElement = document.getElementById("encode-audio");
  audioElement.src = audioUrl;
  audioElement.classList.remove("hidden");

  const encodeDownloadLink = document.getElementById("encode-download");
  encodeDownloadLink.href = URL.createObjectURL(new Blob([encoded]));
  encodeDownloadLink.classList.remove("hidden");
  encodeDownloadLink.onclick = (e) => {
    downloadFile(encoded, "output.sea", "application/octet-stream");
    e.preventDefault();
  };

  const encodeWavDownloadLink = document.getElementById("encode-wav-download");
  encodeWavDownloadLink.href = URL.createObjectURL(new Blob([decodedWave], { type: "audio/wav" }));
  encodeWavDownloadLink.classList.remove("hidden");
  encodeWavDownloadLink.onclick = (e) => {
    downloadFile(decodedWave, "output.wav", "audio/wav");
    e.preventDefault();
  };

  document.getElementById("encode-processing").classList.add("hidden");
  document.getElementById("encode-submit").disabled = false;
});

document.getElementById("decode-submit").addEventListener("click", async () => {
  const fileInput = document.getElementById("decode-input");
  if (!fileInput.files.length) return alert("Please select a file.");

  const file = fileInput.files[0];
  const arrayBuffer = await readFile(file);
  const encodedData = new Uint8Array(arrayBuffer);

  const decodeStart = performance.now();
  const decodedData = sea.wasm_sea_decode(encodedData);
  const decodeEnd = performance.now();
  document.getElementById("decode-time").textContent = `Decoding time: ${(
    decodeEnd - decodeStart
  ).toFixed(2)} ms`;

  const wavBuffer = encodeWAV(decodedData, 44100, 1);
  downloadFile(wavBuffer, "output.wav", "audio/wav");

  const audioUrl = URL.createObjectURL(new Blob([wavBuffer], { type: "audio/wav" }));
  const audioElement = document.getElementById("decode-audio");
  audioElement.src = audioUrl;
  audioElement.classList.remove("hidden");
});

function setupDragAndDrop(dropZone, fileInput) {
  dropZone.addEventListener("dragover", (e) => {
    e.preventDefault();
    dropZone.classList.add("dragover");
  });
  dropZone.addEventListener("dragleave", () => {
    dropZone.classList.remove("dragover");
  });
  dropZone.addEventListener("drop", (e) => {
    e.preventDefault();
    dropZone.classList.remove("dragover");
    fileInput.files = e.dataTransfer.files;
  });
  dropZone.addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    dropZone.textContent = fileInput.files[0].name;
  });
}

setupDragAndDrop(document.getElementById("encode-drop"), document.getElementById("encode-input"));
setupDragAndDrop(document.getElementById("decode-drop"), document.getElementById("decode-input"));
