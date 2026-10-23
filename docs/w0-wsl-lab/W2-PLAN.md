# W2 — Plan de desarrollo: persistencia/discovery WSL con adaptadores inyectables (Implementar)

> Esqueleto de trabajo para la fase W2 (ODD, issue #90, quorb-wsl-v1).
> **Todo el contenido de este doc queda condicionado al checklist §1–§7 de
> `W1-CONTRACT.md`** (aceptado por el owner el 2026-10-21T06:31 en la issue
> #90). Este doc planifica CÓMO, no redecide el QUÉ de W1; cualquier decisión
> que W2 deba reabrir se negocia con el owner antes de implementar.
> Referencias: `W1-CONTRACT.md`, `W0-EVIDENCE.md`, issue #90 (tabla ODD).

---

## 0. Decisiones delegadas de W1 (paso 0 obligatorio — se resuelven antes de codificar)

W1 §9 dejó 3 decisiones abiertas. Se cierran como primera tarea — ver
**`ADR-W2.md`** (decisión, contexto, evidencia y consecuencias de cada una) —
porque condicionan la API y los tests:

| # | Decisión | Opciones | Recomendación W1 | Impacto si no se decide |
|---|---|---|---|---|
| 1 | Lectura de credenciales dentro de la distro | UNC (`\\wsl$\<distro>\...`) vs `wsl -d <distro> -- cat <path>` | `wsl -d ... cat` (aplica timeout con cancelación acotada) | Forma del adaptador de lectura |
| 2 | Sintaxis de `wsl/<distro>` en config | `"wsl/Ubuntu-22.04"`, struct `{mode, distro}`, enum | — | Serialización y saneado (§7) |
| 3 | Distro elegida pero no disponible | Error F2 explícito vs fallback a native | Error F2 (sin fallback, W1 §7) | Comportamiento de `home()` y sesión |

**Nota de verificación (decisión 1)**: la doc de Microsoft
([Working across file systems](https://learn.microsoft.com/en-us/windows/wsl/filesystems))
documenta `\\wsl$\<distro>\` y `\\wsl.localhost\<distro>\` como vía de acceso,
y la **evidencia real del lab (W0-EVIDENCE.md, F7) ya confirmó** que
`Test-Path \\wsl.localhost\quorb_w0\home\…` despierta la distro
(Stopped → Running), igual que `wsl -d ... -- <cmd>`. No hay gap: **ambos
despiertan**. La decisión W1 §9 se mantiene por la razón correcta: `wsl -d ...
cat` es la única opción que permite acotar un proceso colgado (timeout con
kill del launcher + del PID remoto, §4), mientras que la UNC no ofrece forma
de acotar un proceso colgado.

**Decisiones del paso 0 — CERRADAS (2026-10-21, con evidencia del lab)**:

| # | Decisión | Resultado | Base |
|---|---|---|---|
| 1 | Lectura de credenciales | **`wsl -d <distro> -- cat <path>`** (no UNC) | F7: UNC despierta igual que el comando; `wsl ... cat` es la única que permite acotar el proceso colgado (timeout: kill launcher + PID remoto; `--terminate` solo como recuperación explícita, ver §4). Ruta resuelta vía `getent passwd` (F5) |
| 2 | Sintaxis de `wsl/<distro>` en config | **String `"wsl/<distro>"`** con enum de modo + distro saneada | Sencillo de serializar, validar y migrar; el parser valida el nombre contra la **enumeración real** (F4), no contra un whitelist de caracteres — el rechazo de F3 era de `wsl --import` al crear distros, no aplica a aceptar una existente (revisión owner 2026-10-23). Coherente con el formato actual de `credentials_path` |
| 3 | Distro elegida pero no disponible | **Error F2 explícito, SIN fallback a native** | W1 §7 (sin fallback); el diagnóstico F6 distingue el caso y F14 permite resolver el error sin caer al home de otra distro |

Con las 3 decisiones cerradas y F14 incorporado, la **Fase A puede arrancar**:
definir `WslAdapter` trait + `MockWslAdapter` con los estados programables y
los tests deterministas de W1 §7.

---

## 1. Objetivo

Implementar el modo `wsl/<distro>` definido en W1: el home y las sesiones CLI
de Quorb se resuelven dentro de una distro WSL elegida, con enumeración que no
despierta (F4), resolución explícita que sí despierta (F7), diagnóstico de 4
estados (F6), estrategia de shell `-e bash -lc` con fallback `-e sh -c` (F8) y
manejo de errores por token (F2).

## 2. Superficie de entrada (mapeada contra el código real)

| Archivo | Rol hoy | Cambio en W2 |
|---|---|---|
| `src-tauri/src/providers/mod.rs` | `home()` (L279) = `dirs::home_dir()`, `session_file()` (L134), `session_stamp()` (L251) | `home()` pasa a ser dependiente del modo vía adaptador; sesiones resueltas con el home del modo activo |
| `src-tauri/src/config.rs` | `AppConfig` con `SubscriptionConfig.credentials_path` por proveedor | Nuevo campo de modo WSL (decisión #2) + saneado |
| `src-tauri/src/lib.rs` | Comandos Tauri (`session_stamp` L423, `account_session` L453, login/start L466+) | Comandos de Ajustes › WSL: enumerar, resolver home, diagnóstico; comandos de sesión pasan por el adaptador |
| `src/lib/types.ts` | Tipos de config/UI | Tipos del modo + estado de diagnóstico para Ajustes |
| `src/Settings.tsx` | UI Ajustes | Bloque «Entorno de sesión» (Windows/WSL) + selector de distro (solo lectura en W2 si el alcance lo permite) |

## 3. Arquitectura: adaptador inyectable (clave)

Nuevo módulo `src-tauri/src/wsl/` (aislado, como exige el issue: no tocar
Windows nativo ni el futuro port macOS):

```
src-tauri/src/wsl/
├── mod.rs          # trait WslAdapter (enumerable, resolver, diagnosticar, leer, terminar)
├── real.rs         # impl RealWslAdapter: corre wsl.exe (F9: system32, luego ProgramFiles\WSL)
├── process.rs      # lanzador con timeout: kill launcher → kill remoto acotado (pidfile)
├── errors.rs       # parseo de tokens F2: WSL_E_DISTRO_NOT_FOUND / "no installed distributions" / "Invalid command line argument"
└── env.rs          # detección shell: -e bash -lc vs -e sh -c (W1 §3)
```

- La app depende de `dyn WslAdapter` (inyectado en `AppState`).
- En tests: `MockWslAdapter` (sin procesos reales) → tests deterministas.
- En producción: `RealWslAdapter` envuelve `wsl.exe`.

**Decisión de arquitectura — lanzamiento de procesos (verificado)**: el plugin
`tauri-plugin-process` de Tauri v2 **no spawn** procesos: solo expone `exit` y
`relaunch` de la propia app ([docs](https://v2.tauri.app/plugin/process/)). El
spawn de procesos externos vive en el **Shell plugin**
(`tauri_plugin_shell::ShellExt`, permisos `shell:allow-spawn/kill/execute`,
requiere Rust 1.90+; [docs](https://v2.tauri.app/plugin/shell/)).

Implicación para W2: `RealWslAdapter` **no introduce el plugin shell** — debe
reutilizar el patrón que ya existe en `src-tauri/src/cli_process.rs`
(`ProcessRunner` + `Containment` con `attach`/`terminate`, y `RunError`), que
hoy ya lanza procesos externos con captura de output y límites de
contención. El shell plugin solo se añadiría si W2 necesitara comandos de UI
que escapen el trait (no es el caso: todo pasa por `dyn WslAdapter`).

**Ruta de `wsl.exe` (F9, matiz 32-bit)**: Microsoft Learn documenta que un
proceso de 32 bits que invoque `wsl.exe` (64-bit) debe usar
`C:\Windows\Sysnative\wsl.exe` ([basic commands](https://learn.microsoft.com/en-us/windows/wsl/basic-commands)).
`RealWslAdapter` debe resolver: `C:\Windows\System32\wsl.exe` →
`C:\Windows\Sysnative\wsl.exe` (si el proceso es 32-bit) →
`%ProgramFiles%\WSL\wsl.exe` (Win10 antiguo), y fallback a `wsl.exe` en PATH
último.

## 4. Fases de implementación (RED/GREEN)

### Fase A — Contrato y mock (sin WSL real)
- Definir `WslAdapter` trait + `MockWslAdapter` con estados programables
  (cero distros, distro OK, distro ausente, timeout, salida inválida).
- Tests deterministas de: enumeración (F4), resolución home (F7), diagnóstico
  4 estados (F6), precedencia credentials_path (W1 §5), defensa --import (W1 §6).
- **Gate**: `cargo test --locked` verde con el mock.

### Fase B — `RealWslAdapter` (procesos reales)
- Implementar lanzador `wsl.exe` (ruta F9 con matiz Sysnative, ver §3), parseo
  de tokens F2, cancelación acotada en timeout (W1 §2 + nivel 1 de §4) con
  `--terminate` reservado a la acción explícita del usuario, detección de
  shell (W1 §3).
- **cwd (verificado lab F11 + MS)**: el lab F11 (run `39509129152`) confirmó
  en el runner real que `wsl -d X -- pwd` → `/mnt/d/a/quorb/quorb` (cwd de
  Windows), y que **`wsl -d X --cd ~ -- pwd` → `/mnt/c/Users/runneradmin`
  (home de WINDOWS, no de la distro)**: el flag `--cd ~` se ignora o se
  interpreta hacia el home de Windows, con exit 0 (no falla, pero no hace lo
  esperado). **`--cd <path>` queda DESCARTADO por evidencia** — el login no
  puede depender de él. Para forzar el home de la distro hay que usar el shell
  de la distro (`wsl -d X -- sh -c 'cd $HOME && ...'` — funciona incluso sin
  bash, como en Alpine) o `bash -lc 'cd ~ && ...'` solo cuando bash existe.
  F12 (lab) instala bash en la Alpine importada (`apk add bash`) y valida la
  estrategia `-e bash -lc` con bash real, cerrando el gap del run anterior.
- **Sondas F6 ampliadas**: `wsl --status` (estado general WSL) y `wsl --version`
  son ambas válidas para «WSL ausente»; se elige `wsl --version` por simplicidad
  de parseo. **`wsl --shutdown` está prohibido como primitive de timeout**:
  termina TODAS las distros y la VM ligera, no solo la objetivo
  ([basic commands](https://learn.microsoft.com/en-us/windows/wsl/basic-commands)).
- **Primitive de timeout VALIDADO (lab F13, run `39509129152`)**: el lab
  confirmó en runner real que `Stop-Process` del launcher (PID) → la distro
  **sigue Running** (F4), y `wsl --terminate quorb_w0` → **Stopped**
  ("The operation completed successfully"). F13 demuestra que `--terminate`
  **funciona**, no que sea una cancelación segura.

  **Cancelación en dos niveles (punto 1 de la revisión del owner, 2026-10-23)**
  — `--terminate <distro>` detiene TODA la distribución: procesos y sesiones
  ajenos a Quorb Included, no solo la operación que colgaba. W2 define:

  1. **Cancelación ACOTADA (automática, en timeout)**: kill del árbol del
     launcher local + `wsl -d <distro> -- kill <pid_remoto>` sobre el PID que
     el propio comando escribió en un pidfile determinista (el script
     constante de F14 hace `echo $$ > <pidfile>; exec "$1" "$@"` — el `$$` es
     del shell dentro de la distro, no del `wsl.exe` de Windows). Tras un
     `-KILL` al remoto si no responde. **La distro queda `Running`** y no se
     toca nada ajeno a la operación.
  2. **Recuperación EXPLICITA del usuario**: `wsl --terminate <distro>`
     (el método `terminate()` del trait) solo desde una acción de UI
     deliberada («Forzar cierre de la distro» en el diagnóstico), con aviso
     previo de que detiene todo lo que corra en esa distro. **Nunca** como
     primitive automático de timeout.

  F13 respalda el nivel 2 (que funciona) y el nivel 1 queda como gap de
  evidencia de lab: validar en W3 que el pidfile se escribe y que el kill
  remoto alcanza al proceso colgado sin tumbar la distro.
- **F14 — quoting/escapado de argv hacia `sh -c`/`bash -lc` (incorporado por
  el owner, 2026-10-21)**: el cruce Windows→WSL no quedó cubierto por F1–F13
  (que probaron comandos simples). La validación de W2 debe incorporar F14:
  cómo se pasan los argv del login CLI (rutas con espacios/no ASCII, args del
  proveedor) a `wsl -d <distro> -- sh -c '...'` / `-e bash -lc '...'` sin
  romper por quoting. Reglas a testear en Fase B (RED/GREEN):
  - El comando se construye como argv, no como string interpolado. **Precedente
    verificado en `main` (revisión, 2026-10-21)**: `split_login_command` SÍ
    existe en `login.rs` (L312, con tests de fail-closed y backslash), y la
    variante Linux de `spawn_terminal` ya usa `terminal_invocation` =
    `sh -c <SHELL_WRAP> sh <program> <args...>` con argv — no interpola. El
    precedente que citaba originalmente el plan es correcto; la nota previa
    que lo negaba se basó en un grep sobre una rama obsoleta
    (`investigacion/w0-wsl-lab` con `login.rs` anterior al PR #122) y queda
    descartada. F14 extiende ese patrón argv a la resolución de home y a la
    detección de shell en WSL.
  - La ruta del shell (`sh -c`/`bash -lc`) se pasa como argv separado, nunca
    concatenada.
  - **Doble frontera (punto 3 de la revisión del owner, 2026-10-23)**:
    `Command::args` protege la línea de Windows, pero **no** lo que llega
    dentro de `sh -c`/`bash -lc` — ahí vuelve a haber intérprete. El contrato
    lo cierra así:
    - El **script es una constante del código** (allowlist fijo), nunca una
      plantilla construida con datos: p. ej. `exec "$1" "$@"` para
      comando+args, o `cd "$1" && exec "$2" "$@"` para resolver el home.
    - Los **datos entran como parámetros posicionales** (`$1..$n`,
      referenciados con `"$1"`/`"$@"` entrecomillados por el propio script),
      nunca interpolados en el texto del script.
    - Tests de **preservación literal a través de las DOS fronteras**
      (argv de Windows → `sh -c` → `exec`): ruta con espacio, nombre no-ASCII,
      comilla simple y doble, `$`, backticks, glob (`*`/`?`), salto de línea,
      string vacío y argumento que empieza por `-` — el valor debe llegar al
      proceso final **byte a byte idéntico** a como salió de Rust.
  - Los caracteres con significado para el shell (espacios, comillas, `$`,
    backticks, globs) se escapan/entrecomillan de forma determinista y se
    cubren con tests de casos: ruta con espacio, nombre no-ASCII, argumento
    con comilla, path con `$HOME`-like, string vacío.
  - Preferir pasar `$HOME`/ruta resuelta como variable de entorno
    (`WSLENV`/env del proceso) en lugar de interpolarla en el script, cuando
    sea viable (F10: stdin no se reenvía; argv/env es la vía).
  - F14 queda como **gap de evidencia de lab** hasta validarlo con casos
    reales en W3 (o un run F14 dedicado); el diseño lo incorpora desde ahora.
- Tests de integración con `wsl.exe` **solo** si hay WSL disponible; si no,
  skip documentado (determinismo garantizado por mock de fase A).
- **Gate**: tests verdes + `npm run typecheck`.

### Fase C — Config + persistencia
- Campo de modo en `AppConfig` (decisión #2), saneado (config inválida →
  `native` + aviso), distro "no disponible" distinguida (decisión #3).
- Conservación de configuración antigua (default `native`, sin migración de
  datos; issue #91 fuera de alcance).
- **Gate**: tests de persistencia + saneado verdes.

### Fase D — Comandos Tauri + UI Ajustes
- Comandos: `wsl_list` (F4, no despierta), `wsl_detect_home` (F7, explícito),
  `wsl_diagnose` (4 estados F6), sesión por el home del modo activo.
- UI: bloque env en Ajustes (nativo/WSL), selector de distro, botón «Detectar
  home WSL», estado de diagnóstico; sin tokens en UI/logs (W1 §7).
- **Gate**: `npm test` + `npm run build`.

## 5. Criterios de aceptación de W2 (de W1 §7)

- [ ] Config ausente → `native`, datos preservados.
- [ ] Modo/distro persisten; saneado; distro no disponible distinguida.
- [ ] Enumerar (F4) y resolver home (F7) son operaciones separadas.
- [ ] Login y lectura con el mismo contrato de entorno/carpeta/variables.
- [ ] Cambiar de distro invalida cachés dependientes de sesión.
- [ ] Tests deterministas con adaptadores inyectables.
- [ ] Sin tokens en UI/logs/errores; sin fallback a Windows si la distro falla.

## 6. Fuera de alcance de W2 (→ W3)

- Evidencia real Windows/WSL (suite pertinente) — fase W3.
- Cambiar ubicación de datos propios de Quorb (issue #91).
- Instalación/gestión de WSL o CLI; Linux nativo; port macOS.

**Nota de defensa `--import` (verificado MS)**: Microsoft Learn confirma que
las **distros importadas no tienen ejecutable launcher** (el cambio de default
user de una distro importada va por `/etc/wsl.conf`, no por
`<distro> config --default-user`)
([basic commands](https://learn.microsoft.com/en-us/windows/wsl/basic-commands)).
Refuerza W1 §6: la app no debe asumir operaciones de launcher sobre distros
importadas; la enumeración real devuelve nombres de store (`Ubuntu-22.04`,
`Debian`) y el parser tolera el rechazo de `--import`.

## 7. Dependencias y gate

- **Bloqueante**: W1 aceptado por el owner (checkboxes §1–§7 en la issue #90).
- **Después de la aceptación**: confirmar milestone quorb-wsl-v1 y PR revisable
  desde el inicio (tests y docs con cada unidad, según CONTRIBUTING).

## 8. Fuentes verificadas (deep-research, 2026-10-20)

Investigación web sobre integración WSL/Tauri v2 para contrastar el diseño
(sin MCP firecrawl/exa disponibles en la sesión; WebFetch sobre fuentes
primarias). Sub-preguntas: (a) CLI de `wsl.exe` (flags, salidas, errores),
(b) UNC `\\wsl$\` vs `wsl.exe` para leer archivos, (c) spawn de procesos en
Tauri v2 sin bloquear la UI.

1. [Basic commands for WSL — Microsoft Learn](https://learn.microsoft.com/en-us/windows/wsl/basic-commands)
   — Referencia oficial del CLI `wsl.exe`. Confirma: `--list --verbose`
   (incluye estado Running/Stopped y versión WSL 1/2), `--list --quiet`,
   `--version`, `--status`, `--terminate <distro>`, `--import --vhd`,
   `--shutdown` (termina TODAS las distros + VM), `wsl -d <distro> --user`
   (error si el user no existe), `wsl ~` (arranca en home), y el caveat
   `C:\Windows\Sysnative\wsl.exe` para procesos 32-bit. Nota: distros
   importadas sin launcher; default user vía `/etc/wsl.conf`. `wslconfig.exe`
   y `bash` quedan deprecados.
2. [Working across file systems — Microsoft Learn](https://learn.microsoft.com/en-us/windows/wsl/filesystems)
   — Interop y UNC `\\wsl$\<distro>` / `\\wsl.localhost\<distro>`. Confirma
   que `wsl <cmd>` corre en el cwd de Windows y con el default user; los
   comandos se pasan sin modificar y las rutas van en formato WSL. **Gap**: no
   confirma explícitamente que *acceder* a la UNC despierte una distro Stopped
   (validar en W3). `WSLENV` (`/p /l /u /w`) como vía de compartir env vars.
3. [Process plugin — Tauri v2](https://v2.tauri.app/plugin/process/) — Confirma
   que el plugin de proceso NO spawn: solo `exit`/`relaunch` de la app.
   Descartado como vía de lanzamiento para `RealWslAdapter`.
4. [Shell plugin — Tauri v2](https://v2.tauri.app/plugin/shell/) — El spawn de
   procesos externos es del Shell plugin (`ShellExt::shell().command()`,
   `shell:allow-spawn/kill/execute`, requiere Rust 1.90+). Solo se añadiría si
   W2 necesitara comandos de UI fuera del trait (no es el caso).

**Gaps declarados**: (1) «acceder a `\\wsl$\` despierta la distro» no está
verificado en fuente primaria → test de W3. (2) `wsl --cd <path>` no está en
la doc oficial referenciada (builds recientes) → validar en W3 si se adopta.
(3) El comportamiento exacto de `--user` con distros importadas no está
documentado en estas fuentes → cubierto por la defensa de W1 §6.