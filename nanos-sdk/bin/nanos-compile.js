#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { execSync, spawnSync } = require('child_process');

console.log("\x1B[38;2;34;211;238m⚡ nanos compile — JS/TS → WASM Toolchain v1.0.0\x1B[0m");

const args = process.argv.slice(2);
if (args.length === 0 || args.includes('--help') || args.includes('-h')) {
  console.log(`
Usage:
  nanos-compile <entry_file.js|.ts> [options]

Options:
  --out, -o    Output .wasm bundle path (default: dist/agent.wasm)
  --engine     WASM compiler engine: 'javy' (default) | 'bundle'
  --optimize   Apply optimization flags to javy compilation

Engines:
  javy    — Compile JS to self-contained WASM via Javy (QuickJS-based, recommended)
  bundle  — Package JS source as a WASM-loadable bundle for nanos eval_js runtime

Requirements:
  'javy' engine requires: cargo install javy-cli
  'bundle' engine requires: Node.js >= 18
`);
  process.exit(0);
}

const entryFile = args[0];
if (!fs.existsSync(entryFile)) {
  console.error(`\x1B[31m✘ Error: Entry file '${entryFile}' not found.\x1B[0m`);
  process.exit(1);
}

let outFile = 'dist/agent.wasm';
const outIdx = args.indexOf('--out') !== -1 ? args.indexOf('--out') : args.indexOf('-o');
if (outIdx !== -1 && args[outIdx + 1]) {
  outFile = args[outIdx + 1];
}

const engine = args.includes('--engine') ? args[args.indexOf('--engine') + 1] : 'javy';
const optimize = args.includes('--optimize');

// Ensure output directory exists
const outDir = path.dirname(outFile);
if (!fs.existsSync(outDir)) {
  fs.mkdirSync(outDir, { recursive: true });
}

console.log(`  Entry:    \x1B[38;2;167;139;250m${entryFile}\x1B[0m`);
console.log(`  Output:   \x1B[38;2;167;139;250m${outFile}\x1B[0m`);
console.log(`  Engine:   ${engine}`);
console.log('');

// Handle TypeScript — transpile to JS first
let jsFile = entryFile;
if (entryFile.endsWith('.ts') || entryFile.endsWith('.tsx')) {
  console.log('Detected TypeScript input, transpiling to JavaScript...');
  try {
    // Try esbuild first (fastest)
    execSync(`npx -y esbuild ${entryFile} --bundle --platform=node --format=esm --outfile=/tmp/nanos_compiled.js`, { stdio: 'pipe' });
    jsFile = '/tmp/nanos_compiled.js';
    console.log('  ✔ TypeScript transpiled via esbuild');
  } catch (e) {
    console.error('\x1B[31m✘ TypeScript transpilation failed. Install esbuild: npm i -g esbuild\x1B[0m');
    process.exit(1);
  }
}

if (engine === 'javy') {
  // ── Javy Engine: Real JS → WASM compilation via QuickJS ──
  console.log('Compiling JavaScript to WASM via Javy (QuickJS engine)...');
  
  // Check if javy is installed
  const javyCheck = spawnSync('javy', ['--version'], { stdio: 'pipe' });
  if (javyCheck.status !== 0) {
    console.error('\x1B[31m✘ Javy CLI not found.\x1B[0m');
    console.error('  Install it with: cargo install javy-cli');
    console.error('  Or use --engine bundle for the eval_js fallback.');
    process.exit(1);
  }
  
  const javyVersion = javyCheck.stdout.toString().trim();
  console.log(`  Using Javy ${javyVersion}`);
  
  // Build javy compile command
  const javyArgs = ['compile', jsFile, '-o', outFile];
  if (optimize) {
    javyArgs.push('-d'); // Dynamic linking for smaller output
  }
  
  console.log(`  Running: javy ${javyArgs.join(' ')}`);
  const result = spawnSync('javy', javyArgs, { stdio: 'inherit' });
  
  if (result.status !== 0) {
    console.error('\x1B[31m✘ Javy compilation failed.\x1B[0m');
    process.exit(1);
  }
  
  const stats = fs.statSync(outFile);
  const sizeKB = (stats.size / 1024).toFixed(2);
  console.log(`\n\x1B[32m✔ Compiled to ${outFile} (${sizeKB} KB)\x1B[0m`);

} else if (engine === 'bundle') {
  // ── Bundle Engine: Package JS for nanos eval_js runtime ──
  console.log('Packaging JavaScript source as nanos eval_js bundle...');
  
  const sourceCode = fs.readFileSync(jsFile, 'utf-8');
  
  // Create a minimal WASM module that exports the JS source as a data segment
  // The nanos runtime will extract and execute it via eval_js
  const header = Buffer.from([
    0x00, 0x61, 0x73, 0x6d, // WASM magic
    0x01, 0x00, 0x00, 0x00, // Version 1
  ]);
  
  // Custom section (section id 0) containing the JS source
  const nameBytes = Buffer.from('nanos_js_bundle');
  const sourceBytes = Buffer.from(sourceCode, 'utf-8');
  const sectionPayload = Buffer.concat([
    Buffer.from([nameBytes.length]), nameBytes,
    sourceBytes
  ]);
  
  // Section header: id=0 (custom), then LEB128 length
  const sectionHeader = Buffer.from([0x00]); // Custom section
  const sectionLen = encodeLEB128(sectionPayload.length);
  
  const wasmBundle = Buffer.concat([header, sectionHeader, sectionLen, sectionPayload]);
  fs.writeFileSync(outFile, wasmBundle);
  
  const sizeKB = (wasmBundle.length / 1024).toFixed(2);
  console.log(`\n\x1B[32m✔ Bundled to ${outFile} (${sizeKB} KB)\x1B[0m`);
  console.log('\x1B[38;2;100;116;139mNote: This bundle requires the nanos eval_js runtime. For standalone WASM, use --engine javy.\x1B[0m');

} else {
  console.error(`\x1B[31m✘ Unknown engine: '${engine}'. Use 'javy' or 'bundle'.\x1B[0m`);
  process.exit(1);
}

console.log("\x1B[38;2;52;211;153mnanos WASM target ready.\x1B[0m");

// ── Helpers ──

function encodeLEB128(value) {
  const bytes = [];
  do {
    let byte = value & 0x7f;
    value >>= 7;
    if (value !== 0) byte |= 0x80;
    bytes.push(byte);
  } while (value !== 0);
  return Buffer.from(bytes);
}
