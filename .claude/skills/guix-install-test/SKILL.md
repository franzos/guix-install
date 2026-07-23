---
name: guix-install-test
description: Walk the user through testing the guix-install Rust installer in a QEMU VM. Builds the binary, launches a VM with the chosen ISO, copies the binary over SSH, and cycles through install scenarios. Use `/guix-install-test` to start a session.
---

You are an interactive testing assistant for the `guix-install` Rust installer at `/home/franz/git/guix-install`. You drive QEMU on the host, deploy the binary into the live ISO over SSH, and coordinate with the user (who handles the GUI/installer prompts).

The user knows their installer well — they tell you which scenario to test; you handle the plumbing. **Your job is to remove friction from the test cycle, not to second-guess the user.**

## Step 1 — Gather inputs

Ask the user for:

1. **ISO path** — likely under `/gnu/store/...-image.iso`. If they don't have one handy, suggest they build it first; do not guess.
2. **First scenario** — give them concrete options (mode × filesystem × encryption × firmware), but accept anything. Use AskUserQuestion if helpful.

Do not ask about disk size, hostname, locale, etc. — those use sane defaults (20G disk, defaults from CLAUDE.md).

## Step 2 — Build & prep

Build the release binary using the project's wrapper:

```bash
guix shell rust rust:cargo gcc-toolchain -- sh -c "CC=gcc cargo build --release"
```

Patch the RPATH so the binary can find `libgcc_s.so.1` next to itself (the Guix store path baked in by rustc's gcc isn't always present in the live ISO):

```bash
guix shell patchelf -- patchelf --set-rpath '/root:$ORIGIN' target/release/guix-install
```

Find a `libgcc_s.so.1` to copy alongside (gcc-14 lib works fine):

```bash
find /gnu/store -maxdepth 3 -name libgcc_s.so.1 2>/dev/null | grep gcc-14 | head -1
```

Create the qcow2 target disk:

```bash
rm -f /tmp/guix-target.qcow2
qemu-img create -f qcow2 /tmp/guix-target.qcow2 20G
```

## Step 3 — Launch VM

For BIOS:

```bash
qemu-system-x86_64 \
  -enable-kvm -m 4096 -smp 2 \
  -cdrom <ISO_PATH> \
  -drive file=/tmp/guix-target.qcow2,format=qcow2,if=virtio \
  -boot d \
  -device e1000,netdev=net0 \
  -netdev user,id=net0,hostfwd=tcp::2222-:22 \
  -display gtk -vga virtio \
  -name "guix-installer-test"
```

Run this in the **background** (`run_in_background: true`). **Use `-vga virtio`** — `-vga std` (default) sometimes blanks the screen on the Guix installer's libre-graphics check.

For UEFI, add `-bios /run/current-system/profile/share/qemu/edk2-x86_64-code.fd` (or wherever OVMF is on the host) and the user's installer will detect EFI via `/sys/firmware/efi`.

## Step 4 — User sets up sshd

Tell the user (concisely):

1. In the boot menu pick the **graphical/dialog installer** — **NOT** "install using shell based process" (that path blanks the screen).
2. At the shell: `passwd` → `pantherx`, then `herd start ssh-daemon`.
3. Tell you when ready.

## Step 5 — Copy binary

Use `sshpass` (via guix shell) so password prompts don't block:

```bash
guix shell sshpass openssh -- sshpass -p pantherx scp \
  -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2222 \
  target/release/guix-install <libgcc_path> root@localhost:/root/

guix shell sshpass openssh -- sshpass -p pantherx ssh \
  -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2222 root@localhost \
  'chmod +x /root/guix-install && /root/guix-install --help | head -1'
```

Last line should print the installer's banner — confirms it can dlopen libgcc and runs.

## Step 6 — Tell user settings

Print a compact table of what to pick at each REPL step. Match the user's chosen scenario. Always include:
- Mode (Guix / Nonguix / Panther / Enterprise)
- Filesystem (ext4 / btrfs)
- Encryption (yes/no — the passphrase is entered once and piped to cryptsetup via `--key-file -`; `luksFormat --batch-mode` means no extra confirmation prompt)
- Disk: `/dev/vda`
- Username: `panther`, password: `test1234`
- Skip SSH key, no desktop, defaults for locale/timezone/hostname/swap

Tell them to run the installer with output capture:

```
/root/guix-install 2>&1 | tee /tmp/install.log
```

If it errors, pull the log: `sshpass -p pantherx ssh -p 2222 root@localhost 'cat /tmp/install.log'`.

## Step 7 — Boot the result

When the user says they shut down the installer, boot the disk **without the ISO** to verify:

```bash
qemu-system-x86_64 \
  -enable-kvm -m 4096 -smp 2 \
  -drive file=/tmp/guix-target.qcow2,format=qcow2,if=virtio \
  -boot c \
  -device e1000,netdev=net0 \
  -netdev user,id=net0,hostfwd=tcp::2222-:22 \
  -display gtk -vga virtio \
  -name "guix-installed-test"
```

Login: `panther` / `test1234`. For encrypted installs, GRUB prompts for the LUKS passphrase first.

## Step 8 — Recycle for next scenario

When the user wants another test:

1. `pkill -TERM -f "qemu-system-x86_64.*guix-install"` — note `pkill` will return exit 144 because the foreground bash gets killed too; that's expected, ignore it.
2. `rm -f /tmp/guix-target.qcow2 && qemu-img create -f qcow2 /tmp/guix-target.qcow2 20G`
3. Relaunch VM (Step 3) and copy the binary again (Step 5) — passwords/sshd reset on each fresh boot.

If the installer code changed between cycles, **rebuild + re-patchelf** before copying.

## GUI installer variant (cage + Wayland)

The `guix-install-gui` (iced) binary needs a Wayland compositor. In the VM it runs under **cage** on a free VT, with the QEMU GPU exposed via virtio-gpu GL. Steps 1–2 are the same (`cargo build --release` builds all workspace members, GUI included; patchelf the GUI binary the same way). The differences:

### Launch the VM with GL

The GUI needs a GL-capable display, so use UEFI (OVMF) + `virtio-vga-gl`, **not** the CLI's `-vga virtio`. Find the host OVMF dir with `ls /gnu/store/*ovmf*/share/firmware/`:

```bash
OVMF=<host OVMF dir>   # e.g. /gnu/store/...-ovmf-x86-64-*/share/firmware
cp "$OVMF/ovmf_vars_x64.bin" /tmp/ovmf_vars_gui.bin && chmod u+w /tmp/ovmf_vars_gui.bin
qemu-system-x86_64 \
  -enable-kvm -machine q35 -cpu host -m 4096 -smp 2 \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF/ovmf_code_x64.bin" \
  -drive if=pflash,format=raw,file=/tmp/ovmf_vars_gui.bin \
  -drive file=/tmp/guix-target.qcow2,format=qcow2,if=virtio \
  -drive file=<ISO_PATH>,media=cdrom,readonly=on \
  -boot d \
  -device virtio-vga-gl -display gtk,gl=on \
  -device virtio-net,netdev=net0 -netdev user,id=net0,hostfwd=tcp::2223-:22 \
  -name "guix-gui"
```

Run it in the background. Use a **distinct SSH port** (e.g. 2223) so a GUI VM can coexist with a CLI VM on 2222.

### Deploy the GUI binary + launcher

Ship `guix-install-gui` plus `libgcc_s.so.1` plus the `run-gui.sh` launcher below:

```bash
guix shell sshpass openssh -- sshpass -p pantherx scp \
  -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2223 \
  target/release/guix-install-gui <libgcc_path> /path/to/run-gui.sh root@localhost:/root/
guix shell sshpass openssh -- sshpass -p pantherx ssh -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=/dev/null -p 2223 root@localhost \
  'chmod +x /root/guix-install-gui /root/run-gui.sh'
```

`run-gui.sh` is self-contained (safe to run over SSH):

```sh
#!/bin/sh
# Bring up guix-install-gui under cage on this VM.
export XDG_RUNTIME_DIR=/run/user/0
mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"

# The gui binary dlopens libwayland-*/libxkbcommon at runtime; point at a store
# profile that has them. This hash is ISO-specific — if the binary panics on a
# missing lib, find one on the VM: find /gnu/store -name 'libwayland-client.so*'
export LD_LIBRARY_PATH=/gnu/store/47pk6li1mfa0wgy9qzlizxrsc08b5fhb-profile/lib

# virtio-gpu KMS quirks: fails wlroots' atomic test-commit, and its hardware
# cursor plane rejects drmModeSetCursor. Use legacy modeset + software cursor.
export WLR_DRM_NO_ATOMIC=1
export WLR_NO_HARDWARE_CURSORS=1

# The ISO runs kmscon on tty1 with --no-switchvt, holding DRM master forever;
# stop it or cage can never take the GPU.
herd stop term-tty1 >/dev/null 2>&1 || true

pkill -9 cage 2>/dev/null || true
pkill -9 seatd 2>/dev/null || true
rm -f /run/seatd.sock

# Run cage on a free VT (7) and switch to it, so it becomes the active VT and
# gets DRM master. Logs to /root/gui.log for inspection over SSH.
exec openvt -c 7 -s -w -- sh -c 'seatd-launch -- cage -- /root/guix-install-gui > /root/gui.log 2>&1'
```

### Start it (detached) and verify

`openvt -w` blocks, so launch **detached** over SSH:

```bash
guix shell sshpass openssh -- sshpass -p pantherx ssh -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=/dev/null -p 2223 root@localhost \
  'nohup /root/run-gui.sh > /root/gui-outer.log 2>&1 & sleep 7;
   echo "active VT: $(cat /sys/class/tty/tty0/active)";   # expect tty7
   pgrep -x cage >/dev/null && echo "cage up";
   grep -c "Created GL FBO" /root/gui.log'                # >0 means it rendered
```

Healthy signs: active VT `tty7`, cage running, `Created GL FBO` lines, `Direct scan-out`, no `Permission denied` / `panic` in `/root/gui.log`. The installer welcome screen then appears in the `guix-gui` window; the user drives it there. Interview/install logic is identical to the CLI, so the Step 6 scenario settings apply unchanged.

### GUI gotchas

- **`pgrep guix-install-gui` false-negatives** — the name is >15 chars, so `pgrep`/`pkill` by exact name silently match nothing. Use `pgrep -f guix-install-gui`, or match `cage` / the truncated `qemu-system-x86`.
- **Relaunch fails with `openvt: Couldn't deallocate console 7`** — the previous cage left VT7 allocated. Before relaunching: kill cage, then `chvt 1; deallocvt 7`. Or just reboot the VM.
- **Swapping a running GUI binary** — scp-overwriting the live binary fails with ETXTBSY. scp to `guix-install-gui.new`, then `mv -f` it over (rename works while cage holds the old inode), then relaunch.
- **Blank screen, cage not running** — kmscon (`term-tty1`) still holds DRM master, or the VT never switched. Confirm `herd stop term-tty1` ran and the active VT is `tty7`.
- **Kill VMs by PID, not `pkill -x`** — `pkill -x qemu-system-x86_64` matches nothing (truncated comm). Loop over `pgrep qemu-system-x86` and `kill` each PID (read `/proc/$pid/cmdline` to tell `guix-cli` from `guix-gui`).

## Gotchas worth remembering

- **No private mount namespace.** `herd start cow-store /mnt` dispatches to shepherd (PID 1, host namespace). If the installer enters a private mount namespace before mounting `/mnt`, shepherd doesn't see the mount and cow-store overlays onto the live overlayfs root — fails with "filesystem on /mnt/tmp/guix-inst not supported as upperdir". This is fixed in code; if you ever see that error again, look for `unshare(NEWNS)` regressions.
- **Btrfs swap.** Plain zero-filled file + `swapon` returns `EINVAL` because the file is CoW. Use `btrfs filesystem mkswapfile -s <size>m /mnt/swapfile`. Already fixed in code.
- **No cryptsetup confirmation.** The installer runs `cryptsetup luksFormat --batch-mode --key-file -`, so there's no "Type YES" prompt and no passphrase re-entry — it's piped in once via stdin.
- **First-boot ext4 orphan replay.** A streaming "clearing orphaned inode N" log on first boot is the kernel replaying the journal — wait it out (a few minutes). Not a bug.
- **VM exit on user shutdown.** When the user shuts down inside the VM, the QEMU background task ends with exit 0. Pkill returns 144 (the bash wrapper got killed). Both are normal.
- **libgcc_s.so.1.** The release binary links dynamically against a specific Guix store path. If that path isn't in the live ISO's store, the binary fails to load. RPATH-patching to `/root:$ORIGIN` plus copying `libgcc_s.so.1` next to it solves this.
- **Failure log survives on the live root.** A failed install unmounts `/mnt` on cleanup, taking `/mnt/var/log/guix-install.log` with it — but a live sink at **`/var/log/guix-install.log`** (on the ISO overlay) survives and holds the full session, including guix's real error. Read that one, not the `/mnt` copy.
- **Failed install can leave the store broken.** Cleanup stops cow-store but the `/gnu/store` overlay (upperdir on the now-unmounted `/mnt`) can linger, so every later `guix` command ENOENTs on a `mirrors.lock`. That's post-failure wreckage, not the original cause — reboot the VM to get a clean store before re-testing.

## What to NOT do

- Don't ask the user redundant scenario details if they already specified — accept their phrasing and infer (e.g. "guix with encryption" = Guix mode + ext4 + LUKS).
- Don't suggest they edit code unless they explicitly ask.
- Don't auto-recreate the disk between same-scenario retries — only recycle when starting a new scenario.
- Don't poll or sleep waiting for the VM. The user tells you when each step is done.
