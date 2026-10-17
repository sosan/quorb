# W0 evidence — Windows/WSL discovery (lab for Quorb #90)

Evidence collected on hosted `windows-latest` runners in the temporary lab
`sosan/wsl-w0-lab`; the two workflows that produced it (`w0-evidence.yml`,
`w0-names.yml`) are included in this branch for reproducibility. Raw artifacts
live in the lab runs; this document consolidates only their outcomes.

- Baseline run: <https://github.com/sosan/wsl-w0-lab/actions/runs/38990859346>
- Names run: <https://github.com/sosan/wsl-w0-lab/actions/runs/38991093656>

## Environment observed

| Item | Value | Source |
| --- | --- | --- |
| WSL version | 3.0.1.0 | `version-default.txt` |
| Kernel | 6.18.40.1-1 | `version-default.txt` |
| Windows | 10.0.26100 (Server 2025 image) | `version-default.txt`, `host-info.txt` |
| Initial distro state | zero installed | `list-verbose-default.txt` |

## Findings for Quorb contract (W1 input)

### F1 — Output encoding is dual-behavior

Without `WSL_UTF8=1`, `wsl.exe --status / --version / --list` writes **UTF-16LE without BOM**
to a redirected file (captured raw bytes + hex head in `hex-head-list-verbose.txt`,
`encoding-shape.txt` from the baseline run). With `WSL_UTF8=1` the same outputs are plain UTF-8.
Quorb must pin output encoding handling: either always set `WSL_UTF8=1` for child processes or
decode both shapes; naive per-line parsing of default output produces `\x00`-interleaved text.

### F2 — Exit codes are noisy and undocumented in practice

| Case | Exit code | Evidence |
| --- | --- | --- |
| `wsl --list --all` with installed distros | 0 | `exit-codes.txt` |
| `wsl --list --all` with zero distros (message, no stderr) | -1 | run 38991093656 step 2 + `exit-codes.txt` contrast |
| `wsl --list --verbose` zero distros | -1 | `list-verbose-default.txt` (baseline run) |
| `wsl -d <missing>` | -1, text `Wsl/Service/WSL_E_DISTRO_NOT_FOUND` | `zero-distro-missing.txt`, `exit-codes.txt` |
| invalid flag | -1, `Invalid command line argument` | `exit-codes.txt` |
| empty name (`-d " "`) | -1, same DISTRO_NOT_FOUND text | `exit-codes.txt` |

Quorb cannot rely on exit codes alone; parsing the `Wsl/Service/...` error token in the output
text is required to distinguish missing-distro from invalid-args and zero-distro states.

### F3 — Distro names: registration rejects spaces and non-ASCII

`wsl --import` with wsl 3.0.1 rejects:
- `Hello World` (ASCII + space) → `Wsl/Service/RegisterDistro/E_INVALIDARG`
- `PruebaÑ` (latin unicode) → same, via pwsh and via cmd + `chcp 65001`
- `Quorb-Проверка` (cyrillic), `Quorb-世界` (CJK) → same

The error message echoes the name back intact in every shell, which shows the rejection comes
from WSL's own validation, not from pwsh argument mangling. Names with dots/dashes/underscores
were accepted (`CmdSpaceName_test` import ok; `--set-default` ok; `wsl -d` target ok), and
`--list --verbose` prints the name correctly under `WSL_UTF8=1`.

Implication for #90: through supported paths, listed distro names are usually store names
(`Ubuntu-22.04`, `Debian`), but defensive parsing and simulation tests for spaces/non-ASCII
still make sense for legacy/edge states; registration-level rejection is now evidenced.

### F4 — Wake semantics are exactly what the ODD assumed

| Operation | Effect | Evidence |
| --- | --- | --- |
| 20x `wsl --list --quiet` on Stopped distro | **does not wake** (state stays Stopped) | `list-does-not-wake.txt` |
| a single `wsl -d X -- uname` | wakes → Running | `wake-and-run.txt`, `running-list.txt` |
| `wsl --terminate X` | Running → Stopped | `terminated-list.txt` |
| killing the launcher process of a sleeping `wsl` command | **does not stop the distro** (stays Running) | `timeout-hang.txt` |

Quorb timeouts cannot rely on killing the spawned process: terminating a hung invocation needs
`wsl --terminate <distro>` (or its session terminator), which the W1 contract must state as the
stop primitive.

### F5 — Default user / home resolution works without name assumptions

Baseline run `user-default.txt`:
- imported distro default user = `root` until `/etc/wsl.conf` `[user] default=quorbuser` + terminate
- after config: `whoami` → `quorbuser`, `HOME=/home/quorbuser`, `getent passwd` resolves home
- Windows user (`runneradmin`) is unrelated to the Linux user — the "do not infer Windows
  username" assumption is confirmed as required
- `-u root` override works independently of `wsl.conf`

### F6 — Zero-distro diagnostics are distinct and reproducible

Fresh runner with zero distros: `--list` prints
`Windows Subsystem for Linux has no installed distributions.` and exits -1; targeting any distro
name returns `Wsl/Service/WSL_E_DISTRO_NOT_FOUND` -1. WSL itself may be installed with zero
distros (this exact state was the lab environment), so "WSL absent" vs "zero distros" need
separate probes (`--version` vs `--list`).

## Limits of this evidence

Hosted runners prove real wsl.exe behavior on Windows Server 2025 image, but they do not cover:
- store-installed distros and first-boot OOBE user creation,
- WSL not installed at all (couldn't be tested without removing system components),
- developer machine setups with multiple distros and `wsl.conf` variations,
- GUI session / interactive login flows.

Dates: evidence captured 2026-10-17.
