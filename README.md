# guix-install

<p align="center">
  <img src="assets/logo.svg" alt="guix-install" width="480">
</p>
<p align="center">
  Guix System installer. Boot a Guix ISO, run one binary, get a working system — libre Guix, Nonguix, PantherX, or an enterprise config from a server.
</p>

## Why

The existing Python installer (`px-install`) had gotten hard to live with — partitioning tangled into config generation, no resume on failure, four modes bolted on with conditionals. Rewrote it in Rust with the install mode as the central axis.

## Status

Pre-1.0. Runs end-to-end on the machines I've tested. **Read what it's about to do before you let it touch your disk.** `--dry-run` prints the generated `system.scm` (+ `channels.scm`) without partitioning anything.

## Modes

| Mode | Channels | Kernel | Notes |
|------|----------|--------|-------|
| `guix` | `%default-channels` | linux-libre | Hardware preflight warns about Wi-Fi/GPU/Ethernet needing non-free firmware. |
| `nonguix` | + nonguix | linux + microcode | `substitutes.nonguix.org` key compiled in. |
| `panther` *(default)* | + panther (pulls nonguix) | linux + microcode | Inherits `%os-base` from `(px system os)`. `substitutes.guix.gofranz.com` key compiled in. |
| `enterprise` | from remote | from remote | Fetches a tarball over HTTPS by config ID. Skips locale/timezone/hostname/users/desktop. |

## Build

```bash
guix shell rust rust:cargo gcc-toolchain -- sh -c "CC=gcc cargo build --release"
# or
guix shell -m manifest.scm -- cargo build --release
```

## Usage

From the ISO:

```bash
sudo ./target/release/guix-install
```

Walks through Mode → Locale → Timezone → Hostname → Disk → Encryption → Users → Desktop → Summary. Escape goes back a step. Enterprise mode collapses the middle to just Disk + Encryption.

Dry run (no disk touched):

```bash
guix-install --dry-run --mode nonguix --hostname mybox --disk /dev/sda \
             --filesystem btrfs --encrypt --desktop gnome
```

Common flags:

| Flag | Default | |
|------|---------|---|
| `--mode` | `panther` | `guix`, `nonguix`, `panther`, `enterprise` |
| `--hostname` | `<mode>-<6 random>` | |
| `--timezone` | `Europe/Berlin` | |
| `--locale` | `en_US.utf8` | |
| `--disk` | `/dev/sda` | |
| `--filesystem` | `ext4` | or `btrfs` |
| `--encrypt` | off | LUKS on `/` |
| `--desktop` | none | `gnome`, `kde`, `xfce`, `mate`, `sway`, `i3`, `lxqt` |
| `--swap` | `4096` MB | swap file size |
| `--ssh-key` | none | dropped into the user's `authorized_keys` |
| `--config <ID>` | | implies `--mode enterprise` |
| `--config-url` | `https://temp.pantherx.org/install` | enterprise base URL |
| `--dry-run` | off | print scheme, do nothing |

Subcommands:

```bash
guix-install list-disks    # lsblk-style summary
guix-install wifi          # connmanctl WiFi setup
```

## Phases

8 phases, state persisted to `/tmp/.guix-install-state` after each:

1. Partition (parted, BIOS/EFI auto-detected from `/sys/firmware/efi`)
2. Format (ext4/btrfs, optional LUKS)
3. Mount under `/mnt`
4. Swap file
5. Generate `system.scm` + `channels.scm` (or fetch enterprise tarball)
6. Authorize substitute servers
7. `guix pull` (skipped for plain Guix)
8. `guix system init`, set user password

Re-running picks up at the failed phase. Change disk/mode/firmware and state is discarded.

## Notes

- **Passwords never land in `system.scm`.** SHA-512-crypted in-process, atomically written to `/mnt/etc/shadow` (sibling-write + fsync + rename + dir fsync). Plaintext held in `Zeroizing<String>`. No `chroot`/`chpasswd`.
- **Substitute keys compiled in** via `include_str!`.
- **Enterprise tarballs streamed** through `ureq → flate2 → tar`. No intermediate file.
- **Partition naming** handles NVMe/MMC (`/dev/nvme0n1p1`) vs SATA (`/dev/sda1`) via `disk::partition_path`.
- **UI is a trait** (`UserInterface`). REPL uses `dialoguer`; TUI/GUI plug in without touching step logic.

## Development

Tests (golden tests cover the 2×2×2×4 scheme matrix: firmware × encryption × filesystem × mode):

```bash
guix shell rust rust:cargo gcc-toolchain -- sh -c "CC=gcc cargo test"
```

Tier-2 validation pipes each rendered `system.scm` through `guix time-machine ... system build -d`. `#[ignore]`d by default — first run is slow:

```bash
guix shell guix -- cargo test --test scheme_validate -- --ignored
```

Format:

```bash
podman run --rm -v $PWD:/work -w /work rust:latest \
  sh -c "rustup component add rustfmt && cargo fmt"
```

`PLAN.md` and `RESEARCH.md` have the original design intent and upstream references. Code is the source of truth.
