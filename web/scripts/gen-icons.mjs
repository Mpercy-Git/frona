/**
 * Generates PWA icon PNGs with no npm dependencies.
 * Uses Node.js built-in zlib to produce valid PNG files.
 *
 * Output (web/public/):
 *   icon-192.png, icon-512.png
 *   icon-maskable-192.png, icon-maskable-512.png
 *   apple-touch-icon.png (180×180)
 *   badge-72.png (72×72, white bell on transparent bg)
 */

import { deflateSync } from "zlib";
import { writeFileSync, mkdirSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dir = dirname(fileURLToPath(import.meta.url));
const OUT = join(__dir, "..", "public");
mkdirSync(OUT, { recursive: true });

// Brand colours
const BG   = [0x22, 0x28, 0x31, 0xff]; // #222831
const FG   = [0xff, 0xff, 0xff, 0xff]; // white
const NONE = [0x00, 0x00, 0x00, 0x00]; // transparent

// ─── PNG helpers ────────────────────────────────────────────────────────────

function crc32(buf) {
  const table = (() => {
    const t = new Uint32Array(256);
    for (let i = 0; i < 256; i++) {
      let c = i;
      for (let k = 0; k < 8; k++) c = (c & 1) ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      t[i] = c;
    }
    return t;
  })();
  let crc = 0xffffffff;
  for (const b of buf) crc = table[(crc ^ b) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const typeBytes = Buffer.from(type, "ascii");
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const crcInput = Buffer.concat([typeBytes, data]);
  const crcBuf = Buffer.alloc(4); crcBuf.writeUInt32BE(crc32(crcInput));
  return Buffer.concat([len, typeBytes, data, crcBuf]);
}

function makePng(pixels, width, height) {
  // pixels: Uint8Array of RGBA, row-major
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;  // bit depth
  ihdr[9] = 6;  // RGBA
  // bytes 10–12: compression, filter, interlace = 0

  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0; // filter None
    for (let x = 0; x < width; x++) {
      const src = (y * width + x) * 4;
      const dst = y * (width * 4 + 1) + 1 + x * 4;
      raw[dst]     = pixels[src];
      raw[dst + 1] = pixels[src + 1];
      raw[dst + 2] = pixels[src + 2];
      raw[dst + 3] = pixels[src + 3];
    }
  }

  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ─── Drawing primitives ──────────────────────────────────────────────────────

function newCanvas(w, h, fill = NONE) {
  const buf = new Uint8Array(w * h * 4);
  if (fill[3] > 0) {
    for (let i = 0; i < w * h; i++) {
      buf[i * 4]     = fill[0];
      buf[i * 4 + 1] = fill[1];
      buf[i * 4 + 2] = fill[2];
      buf[i * 4 + 3] = fill[3];
    }
  }
  return buf;
}

function setPixel(buf, w, x, y, color) {
  if (x < 0 || y < 0 || x >= w || y >= Math.floor(buf.length / (w * 4))) return;
  const i = (y * w + x) * 4;
  buf[i] = color[0]; buf[i + 1] = color[1]; buf[i + 2] = color[2]; buf[i + 3] = color[3];
}

function fillRect(buf, w, x0, y0, x1, y1, color) {
  for (let y = y0; y < y1; y++)
    for (let x = x0; x < x1; x++)
      setPixel(buf, w, x, y, color);
}

/** Filled circle with anti-alias via distance */
function fillCircle(buf, w, h, cx, cy, r, color) {
  const x0 = Math.max(0, Math.floor(cx - r - 1));
  const x1 = Math.min(w - 1, Math.ceil(cx + r + 1));
  const y0 = Math.max(0, Math.floor(cy - r - 1));
  const y1 = Math.min(h - 1, Math.ceil(cy + r + 1));
  for (let y = y0; y <= y1; y++) {
    for (let x = x0; x <= x1; x++) {
      const dist = Math.sqrt((x - cx) ** 2 + (y - cy) ** 2);
      const alpha = Math.max(0, Math.min(1, r + 0.5 - dist));
      if (alpha <= 0) continue;
      const a = Math.round(color[3] * alpha);
      setPixel(buf, w, x, y, [color[0], color[1], color[2], a]);
    }
  }
}

/** Filled rounded rectangle */
function fillRoundRect(buf, w, h, x0, y0, x1, y1, radius, color) {
  // Corners
  fillCircle(buf, w, h, x0 + radius, y0 + radius, radius, color);
  fillCircle(buf, w, h, x1 - radius, y0 + radius, radius, color);
  fillCircle(buf, w, h, x0 + radius, y1 - radius, radius, color);
  fillCircle(buf, w, h, x1 - radius, y1 - radius, radius, color);
  // Edges
  fillRect(buf, w, x0 + radius, y0, x1 - radius, y1, color);
  fillRect(buf, w, x0, y0 + radius, x0 + radius, y1 - radius, color);
  fillRect(buf, w, x1 - radius, y0 + radius, x1, y1 - radius, color);
}

// ─── Icon designs ────────────────────────────────────────────────────────────

/** Standard icon: dark rounded-rect background + white "F" */
function drawIcon(size, maskable = false) {
  const buf = newCanvas(size, size);
  const inset = maskable ? Math.round(size * 0.1) : 0; // maskable safe zone
  const radius = maskable ? Math.round(size * 0.15) : Math.round(size * 0.22);

  fillRoundRect(buf, size, size, inset, inset, size - inset, size - inset, radius, BG);

  // "F" lettermark — proportional to size
  const s = size;
  const lx = Math.round(s * 0.27);  // left of vertical bar
  const rx = Math.round(s * 0.40);  // right of vertical bar
  const ty = Math.round(s * 0.22);  // top
  const by = Math.round(s * 0.78);  // bottom
  const mx = Math.round(s * 0.68);  // right of top arm
  const mx2= Math.round(s * 0.60);  // right of mid arm
  const m1y= Math.round(s * 0.44);  // top of mid bar
  const m2y= Math.round(s * 0.56);  // bottom of mid bar
  const th = Math.round(s * 0.12);  // arm thickness

  // Vertical stroke
  fillRect(buf, s, lx, ty, rx, by, FG);
  // Top horizontal arm
  fillRect(buf, s, lx, ty, mx, ty + th, FG);
  // Mid horizontal arm
  fillRect(buf, s, lx, m1y, mx2, m2y, FG);

  return buf;
}

/** Badge: 72×72 white notification dot on transparent background */
function drawBadge(size) {
  const buf = newCanvas(size, size);
  const r = Math.round(size * 0.38);
  fillCircle(buf, size, size, size / 2, size / 2, r, FG);
  return buf;
}

// ─── Write files ─────────────────────────────────────────────────────────────

const specs = [
  { file: "icon-192.png",          size: 192, fn: (s) => drawIcon(s, false) },
  { file: "icon-512.png",          size: 512, fn: (s) => drawIcon(s, false) },
  { file: "icon-maskable-192.png", size: 192, fn: (s) => drawIcon(s, true)  },
  { file: "icon-maskable-512.png", size: 512, fn: (s) => drawIcon(s, true)  },
  { file: "apple-touch-icon.png",  size: 180, fn: (s) => drawIcon(s, false) },
  { file: "badge-72.png",          size:  72, fn: (s) => drawBadge(s)       },
];

for (const { file, size, fn } of specs) {
  const pixels = fn(size);
  const png = makePng(pixels, size, size);
  writeFileSync(join(OUT, file), png);
  console.log(`  wrote ${file} (${size}×${size}, ${png.length} bytes)`);
}

console.log("done.");
