# Changelog

## [0.1.8] - 2026-06-09

### Added
- GUI About panel: version, authors, source, license, and credits.

### Fixed
- Online check probes substitute servers concurrently and fails fast on unreachable hosts, so a dead server no longer stalls the check.

## [0.1.7] - 2026-06-08

### Fixed
- Wi-Fi connect waits for the link to settle before checking the internet, so a freshly-connected network is no longer falsely reported offline.

### Changed
- Online check now probes the selected mode's substitute servers, not just bordeaux.

## [0.1.6] - 2026-06-07

### Added
- Network step shows a "(connected)" marker next to the network you're already on, and selecting it skips the passphrase prompt.
- The network list shows each Wi-Fi network once (deduplicated by name), so the same SSID seen by two adapters is no longer an ambiguous duplicate; the adapter is chosen automatically when you connect.

### Fixed
- Wi-Fi connect now confirms the *chosen* network actually connected and the internet is reachable, instead of trusting overall connman state. On machines with a second Wi-Fi adapter already online this previously reported a false "connected" while leaving you offline.

## [0.1.5] - 2026-06-06

### Added
- Network connection step (Ethernet/Wi-Fi via connmanctl) in both CLI and GUI installers; auto-skips when a substitute server is already reachable.
- Keyboard layout selection in the GUI installer (first step), applied live by relaunching the compositor; revisitable until passwords are entered.

## [0.1.4] - 2026-06-04

### Added
- "Shut down" button on the installation-complete screen.

## [0.1.3] - 2026-06-03

### Added
- Graphical installer (`guix-install-gui`), an iced frontend over the same logic.
- Live, structured progress for `guix pull` / `guix system init`, in both frontends.
- In-app `system.scm` editor in the GUI.

### Changed
- Project split into a Cargo workspace; the CLI builds with no GUI dependencies.
- LUKS passphrase entered during setup, fed to `cryptsetup` without a TTY prompt.

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
