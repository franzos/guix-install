# Changelog

## [0.1.2] - 2026-05-10

### Added
- GitHub Actions workflow that builds a static `x86_64-unknown-linux-musl` binary on push, PR, and tag, and attaches it to a GitHub release on `v*` tags.
- README screenshot of the installation summary, plus a fresh terminal-motif logo.

### Changed
- README Usage now distinguishes the PantherX ISO (binary pre-installed) from plain Guix (download the static binary from a release).
- README documents `--username` and `--keyboard` flags that were missing from the table.

## [0.1.1] - 2026-05-10

### Fixed
- `cow-store` overlay failure ("filesystem on /mnt/tmp/guix-inst not supported as upperdir"): mounts now happen in the host mount namespace so shepherd (PID 1) sees `/mnt`.
- `swapon` failing on btrfs with `EINVAL`: btrfs swap files are now created with `btrfs filesystem mkswapfile` (NOCOW + contiguous extents).

### Added
- `.claude/skills/guix-install-test` — interactive testing assistant for cycling install scenarios in a QEMU VM.
- `udevadm settle` between partition/format and format/mount to avoid label-resolution races.
