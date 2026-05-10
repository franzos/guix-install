# Changelog

## [0.1.1] - 2026-05-10

### Fixed
- `cow-store` overlay failure ("filesystem on /mnt/tmp/guix-inst not supported as upperdir"): mounts now happen in the host mount namespace so shepherd (PID 1) sees `/mnt`.
- `swapon` failing on btrfs with `EINVAL`: btrfs swap files are now created with `btrfs filesystem mkswapfile` (NOCOW + contiguous extents).

### Added
- `.claude/skills/guix-install-test` — interactive testing assistant for cycling install scenarios in a QEMU VM.
- `udevadm settle` between partition/format and format/mount to avoid label-resolution races.
