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

### F7 — UNC access to home: does it wake a stopped distro?

*Run 39129860995, `raw/unc-wake.txt` (2026-10-18, this branch).*

Observed (terminated `quorb_w0` → Stopped, then accessed the UNC home):

| Operation | State before | State after |
| --- | --- | --- |
| `Test-Path \\wsl.localhost\quorb_w0\home\quorbuser` | Stopped | **Running** |
| `Get-ChildItem \\wsl.localhost\quorb_w0\home\quorbuser` | Running | Running (count 0) |
| `wsl -d quorb_w0 -- whoami` (control) | Running | Running (`quorbuser`) |

**Finding**: `Test-Path` over the UNC namespace **wakes the distro** (Stopped →
Running). Unlike `wsl --list` (F4, does not wake), reading the home path via
`\\wsl.localhost\...` activates the distro. So the "enumerar no debe despertar"
policy must NOT include UNC home access: resolving the home this way has the
same activation cost as running a command. W1 must choose another home-resolution
strategy that does not wake (e.g. `wsl --list` has no home info; `wsl -d X -- getent passwd`
wakes too — same class as F4's "a command wakes the distro").

### F8 — Shell semantics for detection and login

*Run 39129860995, `raw/shell-semantics.txt` (2026-10-18, this branch).*

| Invocation | Output |
| --- | --- |
| `wsl -d X -- <cmd>` | `<cmd>` runs via **`/bin/sh`** (BusyBox in the Alpine lab distro); an expression with `()` gives `sh: syntax error: unexpected "("` |
| `wsl -d X -e sh -c 'echo SHELL=$SHELL'` | `SHELL=/bin/sh`, `0=20` (PID) |
| `wsl -d X -- whoami` | `quorbuser` (default user per `/etc/wsl.conf`) |
| `wsl -d X -- bash -lc` | `/bin/sh: bash: not found` |
| `wsl -d X -- bash -c` | `/bin/sh: bash: not found` |
| `wsl -d X -- /bin/sh -lc` | `sh: syntax error` (the `(` in the expression) |
| default PATH | Windows interop (`/mnt/c/...`) + Linux base (`/usr/local/sbin:...:/usr/lib/wsl/lib`) — **no nvm/volta paths** |

**Findings**:
- The distro default shell is **`/bin/sh` (BusyBox on Alpine)**; `bash` is NOT
  installed by default in this lab distro. Any CLI that needs bash
  (nvm/volta-installed `claude`/`codex`) will not be on the plain PATH and not
  runnable via `wsl -d X -- <cmd>`.
- A plain `wsl -d X -- <cmd>` sends the command to `/bin/sh` — so shell
  metacharacters in the command are executed by the distro shell (W1 must decide
  whether Quorb passes the CLI as argv to an explicit `-e bash -lc` when present,
  or accepts the plain sh PATH).
- The Windows PATH is appended by WSL interop; it does NOT contain nvm/volta.

### F9 — wsl.exe location in the hosted runner

*Run 39129860995, `raw/wsl-path.txt` (2026-10-18, this branch).*

`(Get-Command wsl.exe).Source` → **`C:\Windows\system32\wsl.exe`**.

On `windows-latest` (Server 2025, 10.0.26100) wsl.exe is in System32. The W1
lookup order should be: `System32\wsl.exe` first, then `%ProgramFiles%\WSL\wsl.exe`
for older images; "WSL absent" must be a distinct diagnostic from "not in PATH"
(the file can exist even with zero distros — F6).

### F10 — stdin non-TTY from a GUI-like context

*Run 39129860995, `raw/stdin-tty.txt` (2026-10-18, this branch).*

| Command | Result |
| --- | --- |
| `wsl -d X -- whoami` via `pwsh` (redirect) | `quorbuser` — works |
| `wsl -d X -- whoami` via `cmd /c` (redirect) | `quorbuser` — works |
| `"hi-from-pwsh" | wsl -d X -- sh -c 'read x; echo got=[$x]'` | `got=[]` — **stdin NOT forwarded** |
| `cmd /c "echo hi-from-cmd | wsl -d X -- sh -c 'read x; echo got=[$x]'"` | `got=[]` — **stdin NOT forwarded** |

**Finding**: launching `wsl -d X -- <cmd>` from a redirected-stdin context (GUI
app / pwsh / cmd) works for output, but **Windows stdin is NOT forwarded to the
Linux command's stdin** (`read x` gets nothing). W1/W2 cannot rely on stdin to
pass data into WSL commands; use arguments or a temp file instead.

### F11 — `wsl --cd` is NOT reliable for forcing the distro home

*Run 39509129152, `raw/f11-cd.txt` (2026-10-20, this branch, after the backtick fix).*

| Command | Output |
| --- | --- |
| `wsl -d quorb_w0 -- pwd` (cwd de Windows esperado) | `/mnt/d/a/quorb/quorb` — el cwd de Windows del runner |
| `wsl -d quorb_w0 --cd ~ -- pwd` (si soporta `--cd`) | `/mnt/c/Users/runneradmin` — **home de Windows**, no el de la distro |
| exit code del intento `--cd` | `0` (no falla, pero NO hace lo esperado) |
| control: `wsl -d quorb_w0 -- bash -lc 'cd ~ && pwd'` | `/bin/sh: bash: not found` (Alpine, esperado) |

**Finding**: `wsl --cd ~` **se ignora o se interpreta hacia el home de
Windows** (`/mnt/c/Users/runneradmin`), no hacia la distro. Devuelve exit 0,
así que no es detectable por el exit code. **El login NO puede usar `--cd`
para forzar el home de la distro.** La vía fiable es pasar el `cd` al shell de
la distro: `wsl -d X -- sh -c 'cd $HOME && ...'` (funciona sin bash, Alpine
incluido) o `bash -lc 'cd ~ && ...'` solo cuando bash existe (F8/F12).

### F12 — Shell strategy with bash installed on Alpine (real)

*Run 39513898413 (2026-10-20, `raw/f12-bash-distro.txt`, con el fix `-u root`).
El run anterior (39512692244) documentó el hallazgo de permisos; este run
captura la evidencia final con bash instalado.*

**Hallazgo del primer intento (permisos, run 39512692244)**: el `apk add
bash` falló con

> ERROR: Unable to lock database: Permission denied
> ERROR: Failed to open apk database: Permission denied
> exit code de apk add bash: 99

**Causa**: el paso F5 configuró `/etc/wsl.conf` con `default=quorbuser`
(no-root). El `apk add` corrió como **quorbuser** y la base de datos de apk
(`/var/lib/apk`) solo es escribible por root. Como bash no se instaló,
`command -v bash` quedó vacío y `-e bash -lc`/`-e bash -c`/`cd ~` fallaron
con `WSL (Relay) ERROR: CreateProcessCommon:875: execvpe(bash) failed:
No such file or directory`. El `-e sh -c` (fallback) sí funcionó y mostró el
PATH del default user (interop Windows + Linux base, sin nvm/volta).

**Fix aplicado**: instalar con `-u root`
(`wsl -d quorb_w0 -u root -- sh -c 'apk add --no-cache bash'`), manteniendo
las pruebas de la estrategia como default user (el que Quorb usaría).

**Evidencia final (run 39513898413)**:

| Paso | Resultado |
| --- | --- |
| `-u root apk add --no-cache bash` | `(4/4) Installing bash (5.2.26-r0)`, `OK: 10 MiB in 18 packages`, exit 0 |
| `command -v bash` (default user) | `/bin/bash` |
| `-e bash -lc` (login) | `BASH=/bin/bash`, `PATH=/usr/local/sbin:...` (Linux base, **sin** interop Windows — el login shell carga nvm/volta si existen) |
| `-e bash -c` (non-login) | `BASH=/bin/bash`, PATH con interop Windows + Linux (contraste claro) |
| `-e sh -c` (fallback) | `SHELL=/bin/sh`, PATH con interop + Linux |
| `-e bash -lc 'cd ~ && pwd'` | `/home/quorbuser` — **confirma F11**: con bash el home se fuerza bien (no hace falta `--cd`) |
| PATH por defecto (sin login) | interop Windows + Linux base, sin nvm/volta (refuerza F8) |

**Conclusión F12**: cierra el gap de W1 §3 — la estrategia `-e bash -lc`
(con `PATH` limpio del login shell) con fallback `-e sh -c` queda **validada
en la MISMA distro con bash real**. Implicación práctica: la detección
`command -v bash` + `-e bash -lc` es la vía correcta cuando bash existe.

**Nota de procedencia (matiz del owner, 2026-10-21T06:31)**: hay una
circularidad parcial en F12 — el bash validado fue **instalado por el propio
lab** (`apk add -u root`), no venía con la distro. En distros store
(Ubuntu/Debian) bash ya viene preinstalado; en distros importadas (la Alpine
del lab) no, por lo que **el fallback `-e sh -c` sigue siendo necesario** en
el contrato: `-e bash -lc` solo cuando `command -v bash` confirme su
existencia, `-e sh -c` en caso contrario.

### F13 — Launcher kill vs terminate: state verified (F4 amplified)

*Run 39509129152, `raw/f13-launcher-vs-terminate.txt`.*

| Step | State (`--list --verbose`) |
| --- | --- |
| Distro Running (control `wsl -d quorb_w0 -- true`) | `quorb_w0  Running  2` |
| Lanzar `sleep 60` en background | launcher alive after 3s: True |
| `Stop-Process` del launcher (PID 3728) | `quorb_w0  Running  2` — **sigue Running** |
| `wsl --terminate quorb_w0` | `The operation completed successfully.` |
| Estado tras `--terminate` | `quorb_w0  Stopped  2` |

**Finding**: confirmado con estado explícito lo que F4 asumía: **matar el
launcher NO detiene la distro** (sigue Running), y **`wsl --terminate` SÍ la
deja Stopped**. El primitive de timeout del W1 §2 (kill launcher →
`--terminate <distro>`) queda validado por evidencia de estado, no por
suposición.

## Evidencia cruda — outputs del run `39513898413` (F11/F12/F13)

Para facilitar la revisión del owner, aquí los outputs reales de los artefactos
`raw/f11-cd.txt`, `raw/f12-bash-distro.txt` y `raw/f13-launcher-vs-terminate.txt`
del run final (artifact `w0-evidence.zip` ID `12307021102`).
Verbatim, sin edición (solo se trunca el PATH de `-e bash -c`/`-e sh -c`, idéntico
al PATH por defecto ya citado en F8/F12).

### `raw/f11-cd.txt`

```
>> plain [wsl -d X -- pwd] (cwd de Windows esperado)
/mnt/d/a/quorb/quorb
>> [wsl -d X --cd ~ -- pwd] (si soporta --cd)
/mnt/c/Users/runneradmin
>> exit code of --cd attempt: 0
>> control: [wsl -d X -- bash -lc 'cd ~ && pwd']
/bin/sh: bash: not found
```

### `raw/f12-bash-distro.txt`

```
>> estado inicial (-l -v)
  NAME        STATE           VERSION
* quorb_w0    Stopped         2
>> instalar bash como root (-u root apk add bash)
fetch https://dl-cdn.alpinelinux.org/alpine/v3.20/main/x86_64/APKINDEX.tar.gz
fetch https://dl-cdn.alpinelinux.org/alpine/v3.20/community/x86_64/APKINDEX.tar.gz
(1/4) Installing ncurses-terminfo-base (6.4_p20240420-r2)
(2/4) Installing libncursesw (6.4_p20240420-r2)
(3/4) Installing readline (8.2.10-r0)
(4/4) Installing bash (5.2.26-r0)
Executing bash-5.2.26-r0.post-install
Executing busybox-1.36.1-r29.trigger
OK: 10 MiB in 18 packages
>> exit code de apk add bash: 0
>> command -v bash como default user (quorbuser)
/bin/bash
>> -e bash -lc (login shell) — PATH y BASH
BASH=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
>> -e bash -c (non-login) — PATH
BASH=/bin/bash
PATH=<interop Windows + Linux base, idéntico al PATH por defecto de F8>
>> control -e sh -c (fallback) — SHELL
SHELL=/bin/sh
PATH=<interop Windows + Linux base, idéntico al PATH por defecto de F8>
>> home con bash -lc (cd ~ && pwd)
/home/quorbuser
>> PATH de la distro por defecto (sin login) para contraste
<interop Windows + Linux base, sin nvm/volta — ver F8>
```

### `raw/f13-launcher-vs-terminate.txt`

```
>> estado inicial (Running esperado)
  NAME        STATE           VERSION
* quorb_w0    Running         2
>> lanzar 'sleep 60' en background
>> launcher alive after 3s: True
>> kill SOLO el launcher (PID 4104)
>> estado tras kill del launcher (F4: distro sigue Running)
  NAME        STATE           VERSION
* quorb_w0    Running         2
>> --terminate quorb_w0 (debe dejar Stopped)
The operation completed successfully.
>> estado tras --terminate (Stopped esperado)
  NAME        STATE           VERSION
* quorb_w0    Stopped         2
```

Los PATH largos completos están en el artefacto descargable
(`w0-evidence.zip`, ID `12307021102`); el doc solo resume los que son
redundantes con F8.

## Limits of this evidence

Hosted runners prove real wsl.exe behavior on Windows Server 2025 image, but they do not cover:
- store-installed distros and first-boot OOBE user creation (declared: if the first
  read of the home wakes the distro via UNC, OOBE completes at that moment — input
  for the W2 test plan, not for the W1 contract).
- WSL not installed at all (couldn't be tested without removing system components).
- developer machine setups with multiple distros and `wsl.conf` variations (F12:
  no store distro with bash on the runner; a single Alpine + a unicode-named
  distro is the current coverage).
- GUI session / interactive login flows.

Dates: evidence captured 2026-10-17 (F1–F10) and 2026-10-20 (F11–F13).

## Captured runs (F7–F10)

The evidence for F7/F8/F9/F10 was produced by re-running
`.github/workflows/w0-evidence.yml` on `sosan/quorb` (branch
`investigacion/w0-wsl-lab`):

- Run **39129860995** (this branch, fix applied): 39 files, 13.3KB artifact
  `w0-evidence.zip` (ID 12137935927). F7–F10 sections above cite the real
  outputs from this run.
- Run **39509129152** (2026-10-20, after the F11 backtick fix): 44 files,
  15KB artifact `w0-evidence.zip` (ID 12304408250). F11–F13 sections above
  cite the real outputs from this run.
- Run **39512692244** (2026-10-20, workflow ampliado — F12 instala bash):
  44 files, 16KB artifact `w0-evidence.zip` (ID 12306019281). Documenta el
  hallazgo de permisos: el default user no-root no puede `apk add` (exit 99).
- Run **39513898413** (2026-10-20, con el fix `-u root`): 44 files, artifact
  `w0-evidence.zip` (ID 12307021102). F12 evidencia final con bash instalado
  vía root y estrategia `-e bash -lc` validada.
- The re-run uses `Add-Content -Value` for annotation writes (so they
  append, not overwrite) — the previous Set-Content bug truncated F7–F10
  outputs to the last annotation only.

> **Mutaciones de estado entre runs (matiz del owner, 2026-10-21T06:31)**:
> a partir del run `39512692244`, el paso F12 instala bash en la distro
> (`apk add -u root`), lo que **muta el estado de `quorb_w0`** de forma
> persistente (rootfs modificado). Los runs posteriores (`39513898413` y
> siguientes) ejecutan ese mismo paso sobre distros re-importadas en cada
> run nuevo (runner efímero), así que la mutación **no se arrastra entre
> runs** — cada run parte de una Alpine limpia re-importada. Sin embargo,
> dentro de un mismo run, F12 (instala bash) corre ANTES que F13: F13 no
> depende de bash (solo `sleep 60` + `--list --verbose` + `--terminate`),
> pero queda anotado que el orden F12→F13 del workflow implica que la distro
> ya trae bash cuando F13 la toca. Para reproducibilidad limpia se mantiene
> este orden documentado.

The F7–F10 runs answered the questions the W1 contract needs: home
resolution does wake the distro (decide the home strategy explicitly),
the distro default shell is `/bin/sh` with no bash (decide the login
strategy), wsl.exe is in System32 (lookup order for "WSL absent"), and
Windows stdin is not forwarded to WSL commands (input must be argv
or temp file, not stdin). The F11–F13 run answered the remaining
W2 questions: `wsl --cd` is not reliable for forcing the distro home
(login must `cd $HOME` via the distro shell), the shell strategy with
bash could not be tested on this runner (no store distro — gap for W3),
and the timeout primitive (kill launcher → `--terminate`) is confirmed
with explicit state.

## W1 decisions that now have evidence

| Decision | Evidence (run 39129860995) | Implication |
| --- | --- | --- |
| Home resolution: UNC vs alternative | F7 — `Test-Path \\wsl.localhost\…` wakes the distro (Stopped → Running) | Must NOT use UNC to resolve home; pick another path (`wsl -d X -- getent passwd` wakes too — alternative needed) |
| Login shell strategy: sh vs bash | F8 — distro shell is `/bin/sh` (BusyBox), `bash` not found | Plain `wsl -d X -- <cmd>` may not see nvm/volta-installed CLIs; `-e bash -lc` only if bash is present (e.g. on Ubuntu store distros) |
| wsl.exe lookup | F9 — wsl.exe is at `C:\Windows\system32\wsl.exe` | W1 contract should name the lookup order (System32 first, then `%ProgramFiles%\WSL\wsl.exe`) |
| Stdin propagation | F10 — Windows stdin is NOT forwarded to `wsl -d X -- <cmd>` (read x gets nothing) | Quorb must not rely on stdin; use argv or temp file |
| Existing F4 (no wake on list) | 20x `--list --quiet` does not wake | `wsl --list` is the only confirmed non-waking call |
| Forcing the distro home (cwd) | F11 — `wsl -d X --cd ~ -- pwd` → `/mnt/c/Users/runneradmin` (home de Windows); `--cd` no es fiable (exit 0 pero no hace lo esperado) | Login must `cd $HOME` via the distro shell (`sh -c 'cd $HOME && ...'` o `bash -lc 'cd ~ && ...'` si bash existe); NO usar `--cd` |
| Shell strategy with bash | F12 (run 39512692244) — `apk add bash` como root (`-u root`, el default user no-root no puede escribir `/var/lib/apk`: exit 99); `-e bash -lc` vs `-e sh -c` como default user | W1 §3 — `-e bash -lc` cuando bash existe (instalado vía apk), fallback `-e sh -c`; la instalación de bash requiere root en distros importadas |
| Timeout primitive | F13 — kill del launcher → distro sigue Running; `wsl --terminate` → Stopped | Contrato W1 §2 validado por estado: kill launcher → `--terminate <distro>` |
