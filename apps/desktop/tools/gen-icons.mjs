import { deflateSync } from 'node:zlib';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const iconsDir = join(here, '..', 'src-tauri', 'icons');
mkdirSync(iconsDir, { recursive: true });

const BG = [0x0e, 0x11, 0x16]; // #0e1116
const ACCENT = [0x4f, 0x8c, 0xff]; // #4f8cff

let crc32;
try {
  const zlib = await import('node:zlib');
  if (typeof zlib.crc32 === 'function') {
    crc32 = (buf) => zlib.crc32(buf) >>> 0;
  }
} catch {
  crc32 = null;
}
if (!crc32) {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  crc32 = (buf) => {
    let c = 0xffffffff;
    for (let i = 0; i < buf.length; i++) {
      c = table[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
    }
    return (c ^ 0xffffffff) >>> 0;
  };
}

function pngChunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'ascii');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

function encodePng(width, height, rgb) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // color type 2 = RGB
  const raw = Buffer.alloc(height * (1 + width * 3));
  for (let y = 0; y < height; y++) {
    const row = y * (1 + width * 3);
    raw[row] = 0; // filter byte
    for (let x = 0; x < width; x++) {
      const p = (y * width + x) * 3;
      const o = row + 1 + x * 3;
      raw[o] = rgb[p];
      raw[o + 1] = rgb[p + 1];
      raw[o + 2] = rgb[p + 2];
    }
  }
  const idat = deflateSync(raw, { level: 9 });
  return Buffer.concat([sig, pngChunk('IHDR', ihdr), pngChunk('IDAT', idat), pngChunk('IEND', Buffer.alloc(0))]);
}

function distToSegment(px, py, x1, y1, x2, y2) {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const lenSq = dx * dx + dy * dy;
  let t = lenSq === 0 ? 0 : ((px - x1) * dx + (py - y1) * dy) / lenSq;
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  const cx = x1 + t * dx;
  const cy = y1 + t * dy;
  return Math.hypot(px - cx, py - cy);
}

function drawA(size) {
  const rgb = Buffer.alloc(size * size * 3);
  for (let i = 0; i < rgb.length; i += 3) {
    rgb[i] = BG[0];
    rgb[i + 1] = BG[1];
    rgb[i + 2] = BG[2];
  }
  const cx = size / 2;
  const topY = size * 0.14;
  const bottomY = size * 0.8;
  const leftX = size * 0.26;
  const rightX = size * 0.74;
  const barY = size * 0.55;
  const barHalf = size * 0.1;
  const stroke = Math.max(2, Math.round(size / 10));
  const barStroke = Math.max(2, Math.round(size / 13));

  const strokes = [
    [leftX, bottomY, cx, topY, stroke],
    [rightX, bottomY, cx, topY, stroke],
    [leftX + barHalf, barY, rightX - barHalf, barY, barStroke],
  ];

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const px = x + 0.5;
      const py = y + 0.5;
      let on = false;
      for (let k = 0; k < strokes.length && !on; k++) {
        const [x1, y1, x2, y2, w] = strokes[k];
        if (distToSegment(px, py, x1, y1, x2, y2) <= w / 2) on = true;
      }
      if (on) {
        const o = (y * size + x) * 3;
        rgb[o] = ACCENT[0];
        rgb[o + 1] = ACCENT[1];
        rgb[o + 2] = ACCENT[2];
      }
    }
  }
  return rgb;
}

function makeIco(png) {
  const header = Buffer.alloc(22);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(1, 4); // count
  header[6] = 32; // width
  header[7] = 32; // height
  header[8] = 0; // colors
  header[9] = 0; // reserved
  header.writeUInt16LE(1, 10); // planes
  header.writeUInt16LE(32, 12); // bit count
  header.writeUInt32LE(png.length, 14); // bytes in resource
  header.writeUInt32LE(22, 18); // image offset
  return Buffer.concat([header, png]);
}

const png32 = encodePng(32, 32, drawA(32));
const png128 = encodePng(128, 128, drawA(128));

writeFileSync(join(iconsDir, '32x32.png'), png32);
writeFileSync(join(iconsDir, '128x128.png'), png128);
writeFileSync(join(iconsDir, 'icon.png'), png128);
writeFileSync(join(iconsDir, 'icon.ico'), makeIco(png32));

console.log('Generated placeholder icons in ' + iconsDir);
