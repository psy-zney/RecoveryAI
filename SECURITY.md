# Security Policy

## 🔒 Physical Data Safety Guarantee

RecoveryAI is intentionally engineered to prevent accidental or intentional data loss on target physical storage:

1. **Win32 Read-Only Enforcement:**
   - Physical drives (`\\.\PhysicalDriveX`), partitions (`\\.\D:`), and disk image files are opened strictly with `GENERIC_READ` and `FILE_SHARE_READ | FILE_SHARE_WRITE`.
   - The engine contains zero code paths that request `GENERIC_WRITE`.
2. **Copy-on-Write Sandbox:**
   - All partition edits, MBR restorations, and cluster rewrites are simulated inside RAM (`VirtualDiskOverlay`).
   - The physical storage is completely untouched.
3. **Progressive Disclosure:**
   - Forensic payloads are capped at small sizes (< 5KB for interactive queries) to prevent buffer overflows and prompt injection vectors.

## 🛡️ Reporting a Vulnerability

If you discover any security issue, potential data safety violation, or memory unsafety bug in RecoveryAI:
- Please report it responsibly via GitHub Security Advisories or email: `lequangkhanh295@gmail.com`.
- We aim to acknowledge reports within 24 hours and patch critical security defects within 72 hours.
