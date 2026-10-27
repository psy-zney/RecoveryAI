#!/usr/bin/env node

/**
 * resc MCP Server Launcher & Bridge
 * Provides cross-platform npx execution for the native Rust MCP server.
 *
 * Usage:
 *   npx resc-mcp [flags]
 *   node bin/resc-mcp.js [flags]
 */

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const isWindows = process.platform === 'win32';
const binaryName = isWindows ? 'mcp-server.exe' : 'mcp-server';

function findBinary() {
  if (process.env.RESC_MCP_BIN && fs.existsSync(process.env.RESC_MCP_BIN)) {
    return process.env.RESC_MCP_BIN;
  }
  if (process.env.RECOVERYAI_MCP_BIN && fs.existsSync(process.env.RECOVERYAI_MCP_BIN)) {
    return process.env.RECOVERYAI_MCP_BIN;
  }

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

function run() {
  let binaryPath = findBinary();

  if (!binaryPath) {
    console.error('[resc] NotImplemented: automatic execution of unverified downloads is disabled.');
    console.error('Build locally with cargo build --release or set RESC_MCP_BIN to a trusted binary.');
    process.exit(1);
  }

  const userArgs = process.argv.slice(2);

  const child = spawn(binaryPath, userArgs, {
    stdio: ['pipe', 'pipe', 'inherit'],
    env: {
      ...process.env,
      RUST_LOG: process.env.RUST_LOG || 'info'
    }
  });

  process.stdin.pipe(child.stdin);
  child.stdout.pipe(process.stdout);

  child.on('error', (err) => {
    console.error(`[resc MCP Launcher] Failed to start native process (${binaryPath}):`, err);
    process.exit(1);
  });

  child.on('close', (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    } else {
      process.exit(code ?? 0);
    }
  });

  process.on('SIGINT', () => {
    if (!child.killed) child.kill('SIGINT');
  });

  process.on('SIGTERM', () => {
    if (!child.killed) child.kill('SIGTERM');
  });
}

run();
