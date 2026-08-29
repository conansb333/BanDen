// Generates deterministic BanDen icons (PNG + ICO) without external tools.
// The mark: a rounded dark tile with a white network glyph (three nodes,
// two links) and a red stop bar, hinting at "control + emergency stop".
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "apps", "desktop", "src-tauri", "icons");
mkdirSync(outDir, { recursive: true });

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePng(size, draw) {
  const raw = Buffer.alloc(size * (size * 4 + 1));
  const px = (x, y, [r, g, b, a]) => {
    if (x < 0 || y < 0 || x >= size || y >= size) return;
    const row = y * (size * 4 + 1) + 1;
    const i = row + x * 4;
    const sa = a / 255;
    const da = raw[i + 3] / 255;
    const oa = sa + da * (1 - sa);
    if (oa === 0) return;
    raw[i] = Math.round((r * sa + raw[i] * da * (1 - sa)) / oa);
    raw[i + 1] = Math.round((g * sa + raw[i + 1] * da * (1 - sa)) / oa);
    raw[i + 2] = Math.round((b * sa + raw[i + 2] * da * (1 - sa)) / oa);
    raw[i + 3] = Math.round(oa * 255);
  };
  draw(px, size);

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function icoWrap(png) {
  // ICO with a single PNG-encoded entry.
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2); // type icon
  header.writeUInt16LE(1, 4); // count
  const entry = Buffer.alloc(16);
  entry[0] = 0; // 256 -> 0
  entry[1] = 0;
  entry[2] = 0;
  entry[3] = 0;
  entry.writeUInt16LE(1, 4); // planes
  entry.writeUInt16LE(32, 6); // bpp
  entry.writeUInt32LE(png.length, 8);
  entry.writeUInt32LE(22, 12); // offset after header+entry
  return Buffer.concat([header, entry, png]);
}

const BG = [24, 27, 33, 255]; // slate tile
const BG_LIGHT = [35, 39, 47, 255];
const FG = [235, 238, 243, 255]; // node white
const ACCENT = [225, 63, 63, 255]; // stop red
const LINK = [120, 133, 154, 255];

function drawMark(px, s) {
  const r = s * 0.19; // tile corner radius
  const inset = Math.round(s * 0.04);
  for (let y = 0; y < s; y++) {
    for (let x = 0; x < s; x++) {
      // rounded-rect tile
      const cx = Math.min(Math.max(x, r), s - 1 - r);
      const cy = Math.min(Math.max(y, r), s - 1 - r);
      const dx = x - cx;
      const dy = y - cy;
      const d = Math.sqrt(dx * dx + dy * dy);
      const edge = d - r;
      if (edge > 1 || x < inset || y < inset || x > s - 1 - inset || y > s - 1 - inset) continue;
      const base = edge > 0 ? [...BG, Math.round(255 * (1 - edge))] : [...(y < s * 0.5 ? BG_LIGHT : BG)];
      px(x, y, base);
    }
  }

  const c = (s - 1) / 2;
  const nodeR = s * 0.09;
  const nodes = [
    [c, c], // hub
    [s * 0.26, s * 0.7],
    [s * 0.74, s * 0.7],
    [s * 0.74, s * 0.3],
  ];

  const line = (x1, y1, x2, y2, color, w) => {
    const steps = Math.ceil(Math.hypot(x2 - x1, y2 - y1) * 2);
    for (let i = 0; i <= steps; i++) {
      const t = i / steps;
      const lx = x1 + (x2 - x1) * t;
      const ly = y1 + (y2 - y1) * t;
      for (let oy = -w; oy <= w; oy++)
        for (let ox = -w; ox <= w; ox++) {
          if (ox * ox + oy * oy <= w * w + 0.1) px(Math.round(lx + ox), Math.round(ly + oy), color);
        }
    }
  };

  line(nodes[0][0], nodes[0][1], nodes[1][0], nodes[1][1], LINK, Math.max(1, s * 0.012));
  line(nodes[0][0], nodes[0][1], nodes[2][0], nodes[2][1], LINK, Math.max(1, s * 0.012));
  line(nodes[0][0], nodes[0][1], nodes[3][0], nodes[3][1], LINK, Math.max(1, s * 0.012));

  for (const [nx, ny] of nodes) {
    for (let oy = -nodeR; oy <= nodeR; oy++)
      for (let ox = -nodeR; ox <= nodeR; ox++) {
        const dd = ox * ox + oy * oy;
        if (dd <= nodeR * nodeR) {
          const aa = dd > (nodeR - 1) * (nodeR - 1) ? FG[3] * (nodeR - Math.sqrt(dd)) : 255;
          px(Math.round(nx + ox), Math.round(ny + oy), [...FG.slice(0, 3), Math.max(0, Math.min(255, aa))]);
        }
      }
  }

  // Emergency-stop bar bottom-right.
  const bw = s * 0.3;
  const bh = s * 0.075;
  const bx = s - inset - bw - s * 0.06;
  const by = s - inset - bh - s * 0.06;
  for (let y = 0; y < bh; y++)
    for (let x = 0; x < bw; x++) px(Math.round(bx + x), Math.round(by + y), ACCENT);
}

for (const size of [32, 128]) {
  const png = encodePng(size, drawMark);
  writeFileSync(join(outDir, `${size}x${size}.png`), png);
  if (size === 32) writeFileSync(join(outDir, "icon.ico"), icoWrap(png));
  console.log(`wrote ${size}x${size}.png`);
}
writeFileSync(join(outDir, "icon.png"), encodePng(128, drawMark));
console.log("wrote icon.ico / icon.png");
