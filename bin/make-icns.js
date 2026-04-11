#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

const [inputDir, outputFile] = process.argv.slice(2);

if (!inputDir || !outputFile) {
  console.error('usage: make-icns.js <input-dir> <output-file>');
  process.exit(1);
}

const iconTypeByFile = [
  ['16x16.png', 'icp4'],
  ['32x32.png', 'icp5'],
  ['64x64.png', 'icp6'],
  ['128x128.png', 'ic07'],
  ['256x256.png', 'ic08'],
  ['512x512.png', 'ic09'],
  ['1024x1024.png', 'ic10'],
];

const chunks = [];

for (const [fileName, iconType] of iconTypeByFile) {
  const filePath = path.join(inputDir, fileName);
  if (!fs.existsSync(filePath)) {
    continue;
  }

  const png = fs.readFileSync(filePath);
  const header = Buffer.alloc(8);
  header.write(iconType, 0, 4, 'ascii');
  header.writeUInt32BE(png.length + 8, 4);
  chunks.push(Buffer.concat([header, png]));
}

if (chunks.length === 0) {
  console.error(`no PNG icon sizes found in ${inputDir}`);
  process.exit(1);
}

const payload = Buffer.concat(chunks);
const fileHeader = Buffer.alloc(8);
fileHeader.write('icns', 0, 4, 'ascii');
fileHeader.writeUInt32BE(payload.length + 8, 4);

fs.writeFileSync(outputFile, Buffer.concat([fileHeader, payload]));
