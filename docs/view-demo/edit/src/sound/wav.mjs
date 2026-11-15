// 16-bit stereo PCM WAV at the output rate: reading the inputs the user
// provides, and writing the mix, with triangular dither.
import { readFileSync } from "node:fs";
import { RATE } from "./dsp.mjs";

/** The PCM full scale, bit depth and channel count every file here has. */
const PCM_FULL_SCALE = 32767;
const BITS_PER_SAMPLE = 16;
const CHANNELS = 2;
/** The RIFF header's size, its fmt chunk's size, the PCM format tag and where the chunks start. */
const HEADER_BYTES = 44;
const FMT_BYTES = 16;
const PCM_FORMAT = 1;
const FIRST_CHUNK = 12;
/** A chunk's id and size take this many bytes before its body. */
const CHUNK_HEAD = 8;

/** The 16-bit stereo WAV at `path` as `{ left, right, seconds }`. */
export function readWav(path) {
  const buffer = readFileSync(path);
  if (buffer.toString("ascii", 0, 4) !== "RIFF" || buffer.toString("ascii", 8, 12) !== "WAVE") throw new Error(`${path} is not a WAV file`);
  let format = null;
  let data = null;
  for (let offset = FIRST_CHUNK; offset + CHUNK_HEAD <= buffer.length; ) {
    const id = buffer.toString("ascii", offset, offset + 4);
    const size = buffer.readUInt32LE(offset + 4);
    const body = buffer.subarray(offset + CHUNK_HEAD, offset + CHUNK_HEAD + size);
    if (id === "fmt ") format = body;
    if (id === "data") data = body;
    offset += CHUNK_HEAD + size + (size % 2);
  }
  if (!format || !data) throw new Error(`${path} has no fmt or data chunk`);
  const [tag, channels, rate, bits] = [format.readUInt16LE(0), format.readUInt16LE(2), format.readUInt32LE(4), format.readUInt16LE(14)];
  if (tag !== PCM_FORMAT || channels !== CHANNELS || rate !== RATE || bits !== BITS_PER_SAMPLE) {
    throw new Error(`${path} must be ${BITS_PER_SAMPLE}-bit PCM, ${CHANNELS} channels at ${RATE} Hz (convert it with ffmpeg -ar ${RATE} -ac ${CHANNELS})`);
  }
  const bytes = BITS_PER_SAMPLE / 8;
  const length = Math.floor(data.length / (CHANNELS * bytes));
  const left = new Float64Array(length);
  const right = new Float64Array(length);
  for (let index = 0; index < length; index += 1) {
    left[index] = data.readInt16LE(index * CHANNELS * bytes) / PCM_FULL_SCALE;
    right[index] = data.readInt16LE(index * CHANNELS * bytes + bytes) / PCM_FULL_SCALE;
  }
  return { left, right, seconds: length / RATE };
}

/** The mix as WAV bytes, dithered with `random`. */
export function wav(mix, random) {
  const bytes = BITS_PER_SAMPLE / 8;
  const dataSize = mix.length * CHANNELS * bytes;
  const buffer = Buffer.alloc(HEADER_BYTES + dataSize);
  buffer.write("RIFF", 0);
  buffer.writeUInt32LE(HEADER_BYTES - CHUNK_HEAD + dataSize, 4);
  buffer.write("WAVEfmt ", 8);
  buffer.writeUInt32LE(FMT_BYTES, 16);
  buffer.writeUInt16LE(PCM_FORMAT, 20);
  buffer.writeUInt16LE(CHANNELS, 22);
  buffer.writeUInt32LE(RATE, 24);
  buffer.writeUInt32LE(RATE * CHANNELS * bytes, 28);
  buffer.writeUInt16LE(CHANNELS * bytes, 32);
  buffer.writeUInt16LE(BITS_PER_SAMPLE, 34);
  buffer.write("data", 36);
  buffer.writeUInt32LE(dataSize, 40);
  let offset = HEADER_BYTES;
  for (let index = 0; index < mix.length; index += 1) {
    for (const channel of [mix.left, mix.right]) {
      const dither = (random() + random()) / 2;
      const value = Math.round(channel[index] * PCM_FULL_SCALE + dither);
      buffer.writeInt16LE(Math.max(-PCM_FULL_SCALE, Math.min(PCM_FULL_SCALE, value)), offset);
      offset += bytes;
    }
  }
  return buffer;
}
