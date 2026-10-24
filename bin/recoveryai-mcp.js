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
const https = require('https');

const isWindows = process.platform === 'win32';
const binaryName = isWindows ? 'mcp-server.exe' : 'mcp-server';

function findBinary() {
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

function downloadBinary(targetPath) {
  const url = `https://github.com/psy-zney/resc/releases/latest/download/${binaryName}`;
  process.stderr.write(`[resc MCP Launcher] Native binary not found locally.\n`);
  process.stderr.write(`[resc MCP Launcher] Fetching prebuilt engine from GitHub Releases:\n  ${url}\n\n`);

  return new Promise((resolve, reject) => {
    function fetchUrl(currentUrl) {
      https.get(currentUrl, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return fetchUrl(res.headers.location);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`Server responded with status code ${res.statusCode}`));
        }

        const file = fs.createWriteStream(targetPath);
        res.pipe(file);

        file.on('finish', () => {
          file.close(() => {
            try {
              fs.chmodSync(targetPath, 0o755);
            } catch (_) {}
            process.stderr.write(`[RecoveryAI MCP Launcher] Successfully installed native engine to ${targetPath}\n\n`);
            resolve(targetPath);
          });
        });
      }).on('error', (err) => {
        fs.unlink(targetPath, () => {});
        reject(err);
      });
    }

    fetchUrl(url);
  });
}

async function run() {
  let binaryPath = findBinary();

  if (!binaryPath) {
    const installTarget = path.resolve(__dirname, binaryName);
    try {
      binaryPath = await downloadBinary(installTarget);
    } catch (err) {
      console.error('[RecoveryAI MCP Launcher] Error: Native binary not found and automatic download failed.');
      console.error('Details:', err.message);
      console.error('\nPlease compile the native engine manually with:');
      console.error('  cargo build --release\n');
      process.exit(1);
    }
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

  process.on('SIGINT', () => {
    if (!child.killed) child.kill('SIGINT');
  });

  process.on('SIGTERM', () => {
    if (!child.killed) child.kill('SIGTERM');
  });
}

run();
