# Target Environment Matrix (Phase II)

Status: Accepted (2026-01-03)

## Operating systems
- Linux (x86_64) only

## Filesystems
- Local filesystems only (no NFS/SMB/CIFS)
- Shared-store scope: same-host, same-group

## Runtime
- CPU-only (no CUDA/GPU required)

## Process model
- Many readers, single writer
- Multi-process daemons allowed on the same host
