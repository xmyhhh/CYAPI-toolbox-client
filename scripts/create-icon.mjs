// Generates the application icon as a PNG without any native dependency.
//
// The Windows build uses a PowerShell script with System.Drawing. This Node
// implementation produces the same artwork everywhere, so `npm run icons`
// works on macOS and Linux as well.
import { deflateSync } from 'node:zlib';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const SIZE = 1024;
const RADIUS = 240;
const BACKGROUND = [0x19, 0x3d, 0x33, 0xff];
const ACCENT = [0xce, 0xf4, 0x82, 0xff];
const BOLT = [
  [544, 159],
  [256, 576],
  [480, 576],
  [448, 865],
  [768, 416],
  [544, 416],
  [624, 159],
];

function insideRoundedRect(x, y) {
  const max = SIZE - 1;
  const cx = x < RADIUS ? RADIUS : x > max - RADIUS ? max - RADIUS : x;
  const cy = y < RADIUS ? RADIUS : y > max - RADIUS ? max - RADIUS : y;
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy <= RADIUS * RADIUS;
}

function insidePolygon(x, y) {
  let inside = false;
  for (let i = 0, j = BOLT.length - 1; i < BOLT.length; j = i++) {
    const [xi, yi] = BOLT[i];
    const [xj, yj] = BOLT[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) {
      inside = !inside;
    }
  }
  return inside;
}

// 3x3 supersampling keeps the rounded corners and the bolt edge smooth.
const SAMPLES = 3;
const pixels = Buffer.alloc(SIZE * SIZE * 4);
for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    let background = 0;
    let accent = 0;
    for (let sy = 0; sy < SAMPLES; sy++) {
      for (let sx = 0; sx < SAMPLES; sx++) {
        const px = x + (sx + 0.5) / SAMPLES;
        const py = y + (sy + 0.5) / SAMPLES;
        if (!insideRoundedRect(px, py)) continue;
        background++;
        if (insidePolygon(px, py)) accent++;
      }
    }
    const total = SAMPLES * SAMPLES;
    const offset = (y * SIZE + x) * 4;
    if (background === 0) continue;
    const coverage = accent / total;
    const shape = accent > 0 ? 0 : 1;
    const colour = shape === 1 ? ACCENT : BACKGROUND;
    // Blend the bolt over the plate, then apply the plate's own alpha.
    const alpha = background / total;
    const mixed = [
      Math.round(BACKGROUND[0] * (1 - coverage) + ACCENT[0] * coverage),
      Math.round(BACKGROUND[1] * (1 - coverage) + ACCENT[1] * coverage),
      Math.round(BACKGROUND[2] * (1 - coverage) + ACCENT[2] * coverage),
    ];
    pixels[offset] = mixed[0];
    pixels[offset + 1] = mixed[1];
    pixels[offset + 2] = mixed[2];
    pixels[offset + 3] = Math.round(colour[3] * alpha);
  }
}

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buffer) {
  let crc = -1;
  for (const byte of buffer) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ -1) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

function encodePng() {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(SIZE, 0);
  header.writeUInt32BE(SIZE, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // truecolour with alpha
  const raw = Buffer.alloc((SIZE * 4 + 1) * SIZE);
  for (let y = 0; y < SIZE; y++) {
    const rowStart = y * (SIZE * 4 + 1);
    raw[rowStart] = 0; // no filter
    pixels.copy(raw, rowStart + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const target = path.join(projectRoot, 'public', 'app-icon.png');
writeFileSync(target, encodePng());
console.log(`Created ${target}`);
