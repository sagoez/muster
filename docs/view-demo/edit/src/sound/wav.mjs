// The mix as a 16-bit stereo PCM WAV file, with triangular dither.
import { RATE } from "./dsp.mjs";

/** The PCM full scale, bit depth and channel count the file is written at. */
const PCM_FULL_SCALE = 32767;
const BITS_PER_SAMPLE = 16;
const CHANNELS = 2;
/** The RIFF header's size, its fmt chunk's size and the PCM format tag. */
const HEADER_BYTES = 44;
const FMT_BYTES = 16;
const PCM_FORMAT = 1;

/** The mix as WAV bytes, dithered with `random`. */
export function wav(mix, random) {
  const bytes = BITS_PER_SAMPLE / 8;
  const dataSize = mix.length * CHANNELS * bytes;
  const buffer = Buffer.alloc(HEADER_BYTES + dataSize);
  buffer.write("RIFF", 0);
  buffer.writeUInt32LE(HEADER_BYTES - 8 + dataSize, 4);
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
