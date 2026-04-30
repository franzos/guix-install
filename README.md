# guix-install

<p align="center">
  <img src="assets/logo.svg" alt="guix-install" width="480">
</p>
<p align="center">
  A mode-aware Guix System installer. Boot a Guix ISO, run one binary, end up with a working system — libre Guix, Nonguix with non-free firmware, PantherX, or an enterprise tarball you point it at.
</p>

## Why

I've been running PantherX on a few machines and the existing Python installer (`px-install`) had grown awkward — partitioning logic tangled with config generation, no resume on failure, four modes glued on with conditionals. So I rewrote it in Rust, with the four installation modes as the central axis everything else fans out from.

The thing is, "install Guix" means very different things depending on whether you want libre purity, working Wi-Fi, the Panther desktop, or a fleet config pulled from a server. This installer treats those as first-class instead of afterthoughts.

## Status

Pre-1.0. It runs end-to-end on the machines I've tested, but **read what it's about to do before letting it touch your disk**. `--dry-run` prints the generated `system.scm` (+ `channels.scm`) without partitioning anything. Do take this with a grain of salt — installers that nuke disks deserve scrutiny.

## Modes

| Mode | Channels | Kernel | Notes |
|------|----------|--------|-------|
| `guix` | `%default-channels` | linux-libre | Runs a hardware preflight — warns about Wi-Fi/GPU/Ethernet that need non-free firmware. |
| `nonguix` | + nonguix | linux + microcode | Substitute key for `substitutes.nonguix.org` is compiled in. |
| `panther` *(default)* | + panther (pulls nonguix) | linux + microcode | Inherits `%os-base` from `(px system os)`. Substitute key for `substitutes.guix.gofranz.com` compiled in. |
| `enterprise` | from remote | from remote | Fetches a tarball over HTTPS by config ID. Skips locale/timezone/hostname/users/desktop — those come from the tarball. |

## Install

This is meant to be run from a Guix live ISO. Outside that, you can build it on any Guix system:

```bash
guix shell rust rust:cargo gcc-toolchain -- sh -c "CC=gcc cargo build --release"
```

There's a `manifest.scm` in the repo if you'd rather:

```bash
guix shell -m manifest.scm -- cargo build --release
```

## Usage

Interactive — what you'd run from the ISO:

```bash
sudo ./target/release/guix-install
```

It walks you through Mode → Locale → Timezone → Hostname → Disk → Encryption → Users → Desktop → Summary. Escape goes back a step. Enterprise mode collapses the middle to just Disk + Encryption — the rest comes from the remote config.

Non-interactive preview (no disk touched):

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
guix-install wifi          # connmanctl-driven WiFi setup, useful from ISO
```

## What it does, step by step

The install runs in **8 phases**, persisted to `/tmp/.guix-install-state` after each:

1. Partition (parted, BIOS or EFI auto-detected from `/sys/firmware/efi`)
2. Format (ext4/btrfs, optional LUKS)
3. Mount under `/mnt`
4. Swap file
5. Generate `system.scm` + `channels.scm` (or fetch enterprise tarball)
6. Authorize substitute servers
7. `guix pull` (skipped for plain Guix)
8. `guix system init`, then set the user password

If a phase fails or the box dies mid-install, re-running picks up where it left off — completed phases are skipped, only the failed one re-runs. If you change disk, mode, or firmware before resuming, the state is discarded and you start fresh.

## Notes worth flagging

- **Passwords never land in `system.scm`.** They're SHA-512-crypted in-process and atomically written to `/mnt/etc/shadow` (sibling-write + fsync + rename + dir fsync). Plaintext is held in `Zeroizing<String>` so it's wiped on drop. No `chroot`/`chpasswd` subprocess.
- **Substitute keys are compiled in** via `include_str!` — not fetched at install time.
- **Enterprise tarballs are streamed** through `ureq → flate2 → tar`. No intermediate file on disk.
- **Disk partition naming** handles NVMe/MMC `p` separators (`/dev/nvme0n1p1`) vs SATA (`/dev/sda1`) automatically.
- **UI is a trait** (`UserInterface`). The current REPL impl uses `dialoguer`; a TUI/GUI could plug in without touching step logic.

## Development

Tests, including golden tests for the 2×2×2×4 scheme matrix (firmware × encryption × filesystem × mode):

```bash
guix shell rust rust:cargo gcc-toolchain -- sh -c "CC=gcc cargo test"
```

There's also a tier-2 validation that pipes each rendered `system.scm` through `guix time-machine ... system build -d` to catch drift in the modules and records we depend on. It's `#[ignore]`d by default because it's slow on the first run:

```bash
guix shell guix -- cargo test --test scheme_validate -- --ignored
```

Format:

```bash
podman run --rm -v $PWD:/work -w /work rust:latest \
  sh -c "rustup component add rustfmt && cargo fmt"
```

`PLAN.md` and `RESEARCH.md` capture the original design intent and the upstream Guix/Nonguix/Panther references I worked from. They're useful when scoping bigger changes, but the code is the source of truth.
