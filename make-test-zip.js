// Minimal STORED-method ZIP writer: forward-slash paths only, no directory
// marker entries. Use this instead of PowerShell's Compress-Archive when
// rebuilding a test content pack for this viewer — Compress-Archive writes
// backslash path separators and trailing-backslash directory-marker entries
// for non-empty directories that this viewer's zip reader (index.html's
// downloadAndExtractZip) mishandles, turning them into bogus zero-byte
// files that then make FS.mkdirTree() fail with ENOTDIR for real files
// under that path. See docs/web_preview_viewer.md "Step 3 findings" #3.
//
// Usage: node make-test-zip.js <source-dir> <out.zip>
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const srcDir = process.argv[2];
const outZip = process.argv[3];

if (!srcDir || !outZip) {
  console.error('Usage: node make-test-zip.js <source-dir> <out.zip>');
  process.exit(1);
}

function walk(dir, base, out) {
  for (const name of fs.readdirSync(dir)) {
    const full = path.join(dir, name);
    const rel = base ? `${base}/${name}` : name;
    const stat = fs.statSync(full);
    if (stat.isDirectory()) {
      walk(full, rel, out);
    } else {
      out.push({ full, rel });
    }
  }
}

const files = [];
walk(srcDir, '', files);
console.log(`packing ${files.length} files`);

function crc32(buf) {
  return zlib.crc32(buf) >>> 0;
}

function dosDateTime() {
  // Fixed epoch-ish timestamp; exact value doesn't matter for local testing.
  return { time: 0, date: 0x21 };
}

const localChunks = [];
const centralChunks = [];
let offset = 0;

for (const { full, rel } of files) {
  const data = fs.readFileSync(full);
  const nameBuf = Buffer.from(rel, 'utf8');
  const crc = crc32(data);
  const { time, date } = dosDateTime();

  const localHeader = Buffer.alloc(30);
  localHeader.writeUInt32LE(0x04034b50, 0);
  localHeader.writeUInt16LE(20, 4); // version needed
  localHeader.writeUInt16LE(0x0800, 6); // flags: UTF-8 name
  localHeader.writeUInt16LE(0, 8); // method: stored
  localHeader.writeUInt16LE(time, 10);
  localHeader.writeUInt16LE(date, 12);
  localHeader.writeUInt32LE(crc, 14);
  localHeader.writeUInt32LE(data.length, 18); // compressed size
  localHeader.writeUInt32LE(data.length, 22); // uncompressed size
  localHeader.writeUInt16LE(nameBuf.length, 26);
  localHeader.writeUInt16LE(0, 28); // extra len

  localChunks.push(localHeader, nameBuf, data);

  const centralHeader = Buffer.alloc(46);
  centralHeader.writeUInt32LE(0x02014b50, 0);
  centralHeader.writeUInt16LE(20, 4); // version made by
  centralHeader.writeUInt16LE(20, 6); // version needed
  centralHeader.writeUInt16LE(0x0800, 8); // flags
  centralHeader.writeUInt16LE(0, 10); // method: stored
  centralHeader.writeUInt16LE(time, 12);
  centralHeader.writeUInt16LE(date, 14);
  centralHeader.writeUInt32LE(crc, 16);
  centralHeader.writeUInt32LE(data.length, 20);
  centralHeader.writeUInt32LE(data.length, 24);
  centralHeader.writeUInt16LE(nameBuf.length, 28);
  centralHeader.writeUInt16LE(0, 30); // extra len
  centralHeader.writeUInt16LE(0, 32); // comment len
  centralHeader.writeUInt16LE(0, 34); // disk number
  centralHeader.writeUInt16LE(0, 36); // internal attrs
  centralHeader.writeUInt32LE(0, 38); // external attrs
  centralHeader.writeUInt32LE(offset, 42); // local header offset

  centralChunks.push(centralHeader, nameBuf);

  offset += localHeader.length + nameBuf.length + data.length;
}

const centralStart = offset;
let centralSize = 0;
for (const c of centralChunks) centralSize += c.length;

const eocd = Buffer.alloc(22);
eocd.writeUInt32LE(0x06054b50, 0);
eocd.writeUInt16LE(0, 4);
eocd.writeUInt16LE(0, 6);
eocd.writeUInt16LE(files.length, 8);
eocd.writeUInt16LE(files.length, 10);
eocd.writeUInt32LE(centralSize, 12);
eocd.writeUInt32LE(centralStart, 16);
eocd.writeUInt16LE(0, 20);

const out = Buffer.concat([...localChunks, ...centralChunks, eocd]);
fs.writeFileSync(outZip, out);
console.log(`wrote ${outZip} (${(out.length / 1024 / 1024).toFixed(1)} MB)`);
