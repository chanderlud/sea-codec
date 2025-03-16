import { downloadFile, formatNumber, readFile } from "./utils.mjs";

const worker = (() => {
  const w = new Worker("worker.mjs", {
    type: "module",
  });
  const callbacks = {};
  let id = 1;
  w.onmessage = (e) => {
    const [id, ...args] = e.data;
    if (id === "error") {
      document.getElementById("error").innerHTML = args[0];
      return;
    }
    const cb = callbacks[id];
    setTimeout(cb(...args), 0);
    delete callbacks[id];
  };

  const call = (fn, ...args) =>
    new Promise((resolve, reject) => {
      callbacks[id] = resolve;
      w.postMessage([id, fn, ...args]);
    });

  return {
    decodeAudioFile: (...args) => call("decodeAudioFile", ...args),
    encodeSEA: (...args) => call("encodeSEA", ...args),
    decodeSEA: (...args) => call("decodeSEA", ...args),
  };
})();

const DOM_ENCODE_DROP = document.getElementById("encode_drop");
const DOM_ENCODE_FILE = document.getElementById("encode_input");
const DOM_RESIDUAL_SIZE = document.getElementById("residual_size");
const DOM_VBR_TARGET_BITRATE = document.getElementById("vbr_target_bitrate");
const DOM_VBR_TARGET_BITRATE_LABEL = document.getElementById("vbr_target_bitrate_label");
const DOM_ENCODE_SUBMIT = document.getElementById("encode_submit");
const DOM_ENCODE_RESULT = document.getElementById("encode_result");

const DOM_DECODE_DROP = document.getElementById("decode_drop");
const DOM_DECODE_FILE = document.getElementById("decode_input");
const DOM_DECODE_SUBMIT = document.getElementById("decode_submit");
const DOM_DECODE_RESULT = document.getElementById("decode_result");

DOM_RESIDUAL_SIZE.addEventListener("input", () => {
  DOM_VBR_TARGET_BITRATE.disabled = DOM_RESIDUAL_SIZE.value === "vbr" ? "" : "disabled";
});

DOM_VBR_TARGET_BITRATE.addEventListener("input", () => {
  const value = parseFloat(DOM_VBR_TARGET_BITRATE.value);
  DOM_VBR_TARGET_BITRATE_LABEL.textContent = value.toFixed(1);
});

DOM_ENCODE_SUBMIT.addEventListener("click", async () => {
  const fileInput = DOM_ENCODE_FILE;
  if (!fileInput.files.length) return alert("Please select a file.");

  DOM_ENCODE_SUBMIT.disabled = true;
  DOM_ENCODE_RESULT.innerHTML = "<p>Encoding...</p>";

  const inputArrayBuffer = await readFile(fileInput.files[0]);
  const {
    samples: interleavedInput,
    sampleRate,
    channels,
  } = await worker.decodeAudioFile(inputArrayBuffer);

  const vbr = DOM_RESIDUAL_SIZE.value === "vbr";
  const residual_size = vbr
    ? parseFloat(DOM_VBR_TARGET_BITRATE.value)
    : parseInt(DOM_RESIDUAL_SIZE.value);

  const { encoded, duration: encodeDuration } = await worker.encodeSEA(
    interleavedInput,
    sampleRate,
    channels,
    residual_size,
    vbr
  );

  const {
    differenceFromOriginal,
    wave: decodedWav,
    duration: decodeDuration,
    psnr,
  } = await worker.decodeSEA(encoded, interleavedInput);

  const pcm16Size = interleavedInput.length * 2;
  const compressedSize = (encoded.length / pcm16Size) * 100;
  const status = [
    `PCM 16 bit size ${formatNumber(pcm16Size)} bytes`,
    `Compressed size: ${formatNumber(encoded.length)} bytes (${compressedSize.toFixed(2)} %)`,
    `Bits per sample: ${((encoded.length * 8) / interleavedInput.length).toFixed(2)} bps`,
    `Encoding took ${encodeDuration.toFixed(2)} ms`,
    `Decoding took ${decodeDuration.toFixed(2)} ms`,
    `PSNR ${psnr.toFixed(2)} dB`,
  ].join("<br />");

  const audioUrl = URL.createObjectURL(new Blob([decodedWav], { type: "audio/wav" }));
  const differenceFromOriginalUrl = URL.createObjectURL(
    new Blob([differenceFromOriginal], { type: "audio/wav" })
  );

  DOM_ENCODE_RESULT.innerHTML = `
  Encode result:
  <a href="#" id="download_sea">Download Sea File</a>
  <audio controls src="${audioUrl}"></audio>
  <br />
  Difference from original:
  <audio controls src="${differenceFromOriginalUrl}"></audio>
  <pre>${status}</pre>
  `;

  const fileName = fileInput.files[0].name.split(".")[0] + ".sea";
  document.getElementById("download_sea").addEventListener("click", (e) => {
    downloadFile(encoded, fileName, "application/octet-stream");
    e.preventDefault();
  });

  DOM_ENCODE_SUBMIT.disabled = false;
});

DOM_DECODE_SUBMIT.addEventListener("click", async () => {
  const fileInput = DOM_DECODE_FILE;
  if (!fileInput.files.length) return alert("Please select a file.");

  DOM_DECODE_SUBMIT.disabled = true;
  DOM_DECODE_RESULT.innerHTML = "<p>Decoding...</p>";

  const file = fileInput.files[0];
  const encodedArrayBuffer = new Uint8Array(await readFile(file));

  const { wave: decodedWav, duration: decodeDuration } = await worker.decodeSEA(encodedArrayBuffer);

  const audioUrl = URL.createObjectURL(new Blob([decodedWav], { type: "audio/wav" }));

  const status = [`Decoding took ${decodeDuration.toFixed(2)} ms`].join("<br />");

  DOM_DECODE_RESULT.innerHTML = `
  Decode result:
  <audio controls src="${audioUrl}"></audio>
    <a href="#" id="download_wav">Download WAV file</a>
  <br />
  <pre>${status}</pre>
  `;

  const fileName = fileInput.files[0].name.replace(".sea", ".wav");
  document.getElementById("download_wav").addEventListener("click", (e) => {
    downloadFile(decodedWav, fileName, "application/octet-stream");
    e.preventDefault();
  });

  DOM_DECODE_SUBMIT.disabled = false;
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
    if (e.dataTransfer.files.length === 1) {
      fileInput.files = e.dataTransfer.files;
      dropZone.textContent = fileInput.files[0].name;
    }
  });
  dropZone.addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    dropZone.textContent = fileInput.files[0].name;
  });
}

setupDragAndDrop(DOM_ENCODE_DROP, DOM_ENCODE_FILE);
setupDragAndDrop(DOM_DECODE_DROP, DOM_DECODE_FILE);
