# ADR-W2 — Decisiones del paso 0 (issue #90, quorb-wsl-v1)

- **Estado**: Aceptado (2026-10-21) — cierra las 3 decisiones delegadas de W1 §9.
- **Contexto**: W1-CONTRACT §9 dejó 3 decisiones abiertas que condicionan la
  API y los tests de W2. Se cierran aquí con registro de trazabilidad
  (contexto → decisión → consecuencia), enlazadas desde W2-PLAN.
- **Base**: evidencia del lab W0 (W0-EVIDENCE.md, F1–F13) y decisiones del
  checklist §1–§7 de W1-CONTRACT (aceptado por el owner).

---

## Decisión 1 — Lectura de credenciales dentro de la distro

**Contexto**: para leer el archivo de credenciales de un CLI en modo
`wsl/<distro>` hay dos vías: la UNC `\\wsl$\<distro>\...` (o
`\\wsl.localhost\<distro>\...`) y `wsl -d <distro> -- cat <path>`.

**Evidencia**: F7 (W0-EVIDENCE) confirmó que acceder a la UNC **despierta** la
distro (Stopped → Running), igual que `wsl -d ... -- <cmd>`. Ambos despiertan;
no hay una vía "que no despierta". La diferencia está en el control del
proceso: la UNC es una operación de filesystem sin proceso al que aplicar
timeout; `wsl ... cat` es un proceso `wsl.exe` que se puede matar (kill del
launcher y del PID remoto que escribe el comando, F13).

**Decisión**: **`wsl -d <distro> -- cat <path>`** (no UNC).

**Consecuencias**:
- El adaptador puede aplicar el timeout (kill launcher + kill acotado del PID
  remoto, nivel 1 de W2-PLAN §4) sobre la lectura, como con cualquier comando.
- La ruta se resuelve previamente con `getent passwd` (F5) — no con la UNC.
- `read_file` en el trait devuelve `String` (UTF-8 lossy), no bytes, porque
  el runner reutilizable (cli_process.rs) ya aplica `from_utf8_lossy`.
- **Matiz (revisión del owner, 2026-10-23)**: `--terminate` se reserva como
  recuperación explícita del usuario (nivel 2); no es el primitive de timeout
  automático, porque mata procesos y sesiones ajenas a Quorb dentro de la
  distro.

## Decisión 2 — Sintaxis de `wsl/<distro>` en config

**Contexto**: cómo representar el modo en `AppConfig`: string
`"wsl/Ubuntu-22.04"`, struct `{ mode, distro }`, o enum.

**Evidencia**: el parser debe tolerar nombres de store (`Ubuntu-22.04`,
`Debian`) y aceptar la distro **elegida por el usuario**, sea cual sea su
nombre real. F3 documenta el rechazo de `wsl --import` al **crear** distros
con espacios/no-ASCII (`E_INVALIDARG`), pero eso no acredita descartar
globalmente un nombre enumerado o legacy que ya exista en la máquina
(revisión del owner, 2026-10-23). La validación correcta es **contra la
enumeración real (F4)**, no contra un whitelist de caracteres.

**Decisión**: **string `"wsl/<distro>"`** con un enum de modo (`Native` /
`Wsl`) + la distro como string saneado.

**Consecuencias**:
- Serialización simple y coherente con el formato actual de
  `credentials_path` (string por proveedor).
- **Validación por enumeración, no por whitelist (punto 2 del owner,
  2026-10-23)**: un nombre configurado es válido si aparece en `wsl --list
  --quiet` (F4) con comparación normalizada (trim + case-insensitive). Un
  nombre enumerado con espacios o no-ASCII es válido; uno que NO aparezca
  (borrada, renombrada) se reporta `DistroUnavailable` con la lista de
  disponibles (decisión 3, sin fallback), **conservando el valor en config**
  para que el usuario pueda recuperarlo. Solo un valor vacío/blanco se
  sanea a `native` + aviso (W1 §7).
- Migración: default `native` sin tocar configs antiguas (issue #91 fuera de
  alcance).

## Decisión 3 — Distro elegida pero no disponible

**Contexto**: si el usuario eligió `wsl/Ubuntu-22.04` y esa distro ya no
existe (desinstalada, desregistrada), qué hace la app: mostrar el error F2
o caer silenciosamente a `native`/a otra distro.

**Evidencia**: W1 §7 exige **sin fallback a Windows cuando la distro falla**.
El diagnóstico F6 distingue el estado con sondas separadas; los tokens F2
permiten identificar `WSL_E_DISTRO_NOT_FOUND`.

**Decisión**: **error F2 explícito, SIN fallback a `native`**.

**Consecuencias**:
- `diagnose(target)` del trait produce `WslState::DistroUnavailable` para
  este caso (por eso el trait recibe `target`).
- La app muestra el error accionable sin crashear ni resolver el home contra
  otra distro (p. ej. el default).
- El saneado de config distingue "config inválida" (→ `native` + aviso) de
  "distro elegida pero ausente" (→ error, se mantiene el modo).

---

## Cierre

Estas 3 decisiones quedan reflejadas en:
- `W2-PLAN.md` — paso 0 (tabla de decisiones cerradas).
- `W2-WSLADAPTER.md` — trait `WslAdapter` (firmas que las materializan:
  `read_file` vía `wsl cat`, sintaxis string, `diagnose(target)`).

Cualquier reapertura es negociable con el owner antes de implementar.