#!/usr/bin/env node

/**
 * RecoveryAI MCP Server Launcher & Bridge
 * Provides cross-platform npx execution for the native Rust MCP server.
 *
 * Usage:
 *   npx recoveryai-mcp [flags]
 *   node bin/recoveryai-mcp.js [flags]
 */

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const isWindows = process.platform === 'win32';
const binaryName = isWindows ? 'mcp-server.exe' : 'mcp-server';

function findBinary() {
  // 1. Check custom environment variable
  if (process.env.RECOVERYAI_MCP_BIN && fs.existsSync(process.env.RECOVERYAI_MCP_BIN)) {
    return process.env.RECOVERYAI_MCP_BIN;
  }

  // 2. Candidate search locations
  const searchPaths = [
    path.resolve(__dirname, '..', 'target', 'release', binaryName),
    path.resolve(__dirname, '..', 'target', 'debug', binaryName),
    path.resolve(__dirname, '..', 'binaries', binaryName),
    path.resolve(__dirname, binaryName)
  ];

  for (const candidate of searchPaths) {
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  return null;
}

const binaryPath = findBinary();

if (!binaryPath) {
  console.error('[RecoveryAI MCP Launcher] Error: Native binary not found.');
  console.error('Looked in:');
  console.error(`  - ${path.resolve(__dirname, '..', 'target', 'release', binaryName)}`);
  console.error(`  - ${path.resolve(__dirname, '..', 'binaries', binaryName)}`);
  console.error('\nPlease compile the native engine first with:');
  console.error('  cargo build --release\n');
  process.exit(1);
}

// Forward CLI arguments
const userArgs = process.argv.slice(2);

// Launch the native MCP server process
const child = spawn(binaryPath, userArgs, {
  stdio: ['pipe', 'pipe', 'inherit'],
  env: {
    ...process.env,
    RUST_LOG: process.env.RUST_LOG || 'info'
  }
});

// Clean stdio pipeline for JSON-RPC 2.0 communication
process.stdin.pipe(child.stdin);
child.stdout.pipe(process.stdout);

child.on('error', (err) => {
  console.error(`[RecoveryAI MCP Launcher] Failed to start native process (${binaryPath}):`, err);
  process.exit(1);
});

child.on('close', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});

// Graceful signal forwarding
process.on('SIGINT', () => {
  if (!child.killed) child.kill('SIGINT');
});

process.on('SIGTERM', () => {
  if (!child.killed) child.kill('SIGTERM');
});
