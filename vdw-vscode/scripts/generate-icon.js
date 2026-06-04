/**
 * Generate a 128x128 PNG icon for the VDW VS Code extension.
 * Uses only Node.js built-in modules (zlib for PNG compression).
 * Run: node scripts/generate-icon.js
 */
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const SIZE = 128;
const OUTPUT = path.join(__dirname, '..', 'resources', 'icons', 'vdw.png');

// PNG signature
const PNG_SIG = Buffer.from([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

function crc32(buf) {
    let crc = 0xFFFFFFFF;
    const table = new Uint32Array(256);
    for (let i = 0; i < 256; i++) {
        let c = i;
        for (let j = 0; j < 8; j++) {
            c = (c & 1) ? (0xEDB88320 ^ (c >>> 1)) : (c >>> 1);
        }
        table[i] = c;
    }
    for (let i = 0; i < buf.length; i++) {
        crc = table[(crc ^ buf[i]) & 0xFF] ^ (crc >>> 8);
    }
    return (crc ^ 0xFFFFFFFF) >>> 0;
}

function makeChunk(type, data) {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const typeB = Buffer.from(type, 'ascii');
    const crcData = Buffer.concat([typeB, data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(crcData));
    return Buffer.concat([len, typeB, data, crc]);
}

// Create RGBA pixel data
const rawData = Buffer.alloc((SIZE * SIZE * 4) + SIZE); // +SIZE for filter bytes per row

// Colors
const BG = [26, 26, 46, 255];       // #1a1a2e dark navy
const ACCENT = [0, 200, 255, 255];   // cyan accent
const GREEN = [0, 200, 100, 255];    // green accent
const WHITE = [220, 220, 240, 255];  // light text

function setPixel(x, y, r, g, b, a) {
    if (x < 0 || x >= SIZE || y < 0 || y >= SIZE) return;
    const rowStart = y * (SIZE * 4 + 1) + 1; // +1 for filter byte
    const idx = rowStart + x * 4;
    rawData[idx] = r;
    rawData[idx + 1] = g;
    rawData[idx + 2] = b;
    rawData[idx + 3] = a;
}

function drawRect(x1, y1, x2, y2, r, g, b, a) {
    for (let y = y1; y <= y2; y++) {
        for (let x = x1; x <= x2; x++) {
            setPixel(x, y, r, g, b, a);
        }
    }
}

function drawCircle(cx, cy, radius, r, g, b, a) {
    for (let y = cy - radius; y <= cy + radius; y++) {
        for (let x = cx - radius; x <= cx + radius; x++) {
            const dx = x - cx;
            const dy = y - cy;
            if (dx * dx + dy * dy <= radius * radius) {
                setPixel(x, y, r, g, b, a);
            }
        }
    }
}

// Fill background
drawRect(0, 0, SIZE - 1, SIZE - 1, BG[0], BG[1], BG[2], BG[3]);

// Draw a link chain symbol (two circles connected by lines) to represent "linking repos"
const cx1 = 38, cy1 = 64, r1 = 20;
const cx2 = 90, cy2 = 64, r2 = 20;

// Outer rings (accent color)
drawCircle(cx1, cy1, r1, ACCENT[0], ACCENT[1], ACCENT[2], ACCENT[3]);
drawCircle(cx2, cy2, r2, ACCENT[0], ACCENT[1], ACCENT[2], ACCENT[3]);

// Clear inner circles
drawCircle(cx1, cy1, 14, BG[0], BG[1], BG[2], BG[3]);
drawCircle(cx2, cy2, 14, BG[0], BG[1], BG[2], BG[3]);

// Connecting links (top and bottom bars)
drawRect(50, 60, 78, 68, ACCENT[0], ACCENT[1], ACCENT[2], ACCENT[3]);

// VDW text at bottom
// "V" shape using two lines
for (let i = 0; i < 6; i++) {
    drawRect(30 + i, 96 + i, 30 + i, 96 + i + 14, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
    drawRect(30 + i, 96 - i, 30 + i, 96 - i + 14, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
}
// "V" bottom point
drawRect(36, 96, 36, 110, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);

// "D" shape
drawRect(48, 96, 48, 110, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
drawRect(48, 96, 58, 96, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
drawRect(48, 106, 58, 106, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);
drawRect(58, 97, 58, 105, GREEN[0], GREEN[1], GREEN[2], GREEN[3]);

// "W" shape
for (let i = 0; i < 5; i++) {
    drawRect(66 + i, 96, 66 + i, 110, WHITE[0], WHITE[1], WHITE[2], WHITE[3]);
    const mid = Math.min(i * 3, 10);
    drawRect(66 + i + 2, 106 - mid, 66 + i + 2, 110, WHITE[0], WHITE[1], WHITE[2], WHITE[3]);
}

// Set filter bytes to 0 (None) for each row
for (let y = 0; y < SIZE; y++) {
    rawData[y * (SIZE * 4 + 1)] = 0;
}

// Compress with zlib
const compressed = zlib.deflateSync(rawData);

// Build PNG
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);   // width
ihdr.writeUInt32BE(SIZE, 4);   // height
ihdr.writeUInt8(8, 8);          // bit depth
ihdr.writeUInt8(6, 9);          // color type (RGBA)
ihdr.writeUInt8(0, 10);         // compression
ihdr.writeUInt8(0, 11);         // filter
ihdr.writeUInt8(0, 12);         // interlace

const png = Buffer.concat([
    PNG_SIG,
    makeChunk('IHDR', ihdr),
    makeChunk('IDAT', compressed),
    makeChunk('IEND', Buffer.alloc(0)),
]);

fs.mkdirSync(path.dirname(OUTPUT), { recursive: true });
fs.writeFileSync(OUTPUT, png);
console.log(`✅ Icon generated: ${OUTPUT} (${png.length} bytes)`);
