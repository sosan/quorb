# W1 — Contrato de entorno, activación, shell y errores (Decidir)

> Borrador para aceptación del owner (issue #90, quorb-wsl-v1, ODD W1).
> Materializa las decisiones que las secciones a continuación deben contrastar
> antes de W2 (implementar). Referencia: `docs/w0-wsl-lab/W0-EVIDENCE.md`.
> Nivel de detalle: borrador operativo — cada sección propone un comando
> canónico, la falla que cubre y la decisión que pide aceptar.

---

## 1. Contrato de entorno

El modo describe el origen de los datos de sesión CLI de Quorb:

| Modo | Significado | Home usado para sesiones |
|---|---|---|
| `native` (hoy, default) | CLI corre en Windows nativo (PowerShell o cmd) | `%USERPROFILE%` vía `dirs::home_dir()` |
| `wsl/<distro>` | CLI corre en WSL, distro `<distro>`, **user por defecto** de la distro | Home del default user de la distro |

**Regla crítica**: Quorb **no asume que el user Windows coincide con el user
Linux**. El user Linux se resuelve de la distro, no del `whoami` Windows (F5).

### Resolver el home en modo WSL

Comando canónico (basado en F5):

```bash
wsl.exe -d <distro> -- getent passwd <user> | cut -d: -f6
```

- `wsl.exe` desde `C:\Windows\system32\wsl.exe` (F9) — buscarlo ahí primero,
  luego `%ProgramFiles%\WSL\wsl.exe` (caveat Win10).
- **F7**: este comando **despierta la distro** (Stopped → Running). Es una
  **acción explícita del usuario** (p. ej. botón "Detectar home WSL" o
  equivalente), **nunca** efecto secundario de refrescar la lista de cuentas
  (F4: `wsl --list` no despierta).

**Decisión W1 que pide aceptar**:
- [ ] El modo `wsl/<distro>` es la única ruta de entorno nueva (sin `wsl/<vm>`,
      `wsl/<terminal>` ni `docker`).
- [ ] El resolver de home via `wsl -d <distro> -- getent passwd` (F7) es la ruta canónica.

---

## 2. Política de activación de distribuciones

Tabla de operaciones y su efecto sobre una distro Stopped (F4 + F7):

| Operación | ¿Despierta? | Cuándo la usa Quorb |
|---|---|---|
| `wsl --list` / `--list --verbose` | **NO** | Enumerar distros en background (Ajustes › WSL, sin acción del usuario) |
| `wsl --list --quiet` | **NO** | Idem (más rápido) |
| `wsl --version` | **NO** | Detectar "WSL ausente" (sonda separada de cero-distros, F6) |
| `wsl -d <distro> -- <cmd>` | **SÍ** | Resolver home, abrir sesión, probar login — siempre acción explícita |
| `wsl --terminate <distro>` | No (para) | Primitive de timeout: detener una distro bloqueada |

**Primitive de timeout** (F4, observado): matar el proceso hijo de `wsl` NO
detiene la distro (queda Running). El contrato es:

1. kill del launcher (libera el PID del shell de Quorb),
2. `wsl --terminate <distro>` (detiene la distro realmente).

**F6 — Diagnóstico de 4 estados** (con sondas separadas):

| Estado | Sonda | Señal |
|---|---|---|
| WSL ausente | `wsl --version` | Falla (no instalado / no en PATH) |
| Cero distros | `wsl --list` | Imprime `"no installed distributions"`, exit -1 |
| Distro ausente (nombre inválido) | `wsl -d <missing> -- ...` | Output `Wsl/Service/WSL_E_DISTRO_NOT_FOUND`, exit -1 |
| Salida inválida / timeout | `wsl -d <distro> -- <cmd>` | exit no 0, stderr con token |

Todos son recuperables: la app puede reintentar sin recargar.

**Decisión W1 que pide aceptar**:
- [ ] La enumeración usa `wsl --list --verbose` (F4, no despierta).
- [ ] El resolver de home usa `wsl -d <distro> --` (F7, sí despierta), solo en
      acción explícita del usuario.
- [ ] El primitive de timeout es `wsl --terminate <distro>` tras kill del
      launcher (F4).

---

## 3. Estrategia de shell

`wsl -d <distro> -- <cmd>` corre `<cmd>` vía el shell por defecto de la
distro (F8 observado):

| Distro | Shell por defecto | PATH por defecto |
|---|---|---|
| Alpine (lab) / busybox | `/bin/sh` | Base Linux + interop Windows (`/mnt/c/...`) + `/usr/lib/wsl/lib`. **Sin** nvm/volta |
| Ubuntu / store distros | `/bin/bash` (o `/bin/sh` por `$SHELL`) | Con bash, `nvm`/`volta` se cargan en login shell (`bash -lc` o `bash -ic`) |

**Contrato propuesto**:

- Quorb **prefiere** `-e bash -lc <cmd>` cuando bash existe.
- Fallback a `-e sh -c <cmd>` con el PATH que la distro le da (Alpine/busybox).
- **Detección única** de bash:

```bash
wsl.exe -d <distro> -- bash -lc 'command -v bash'   # exit 0 = bash presente
```

**Implicación para login**: un login CLI instalado en `nvm`/`volta` (que
requiere bash) no se ve en el PATH sh plano. El contrato debe forzar
`-e bash -lc` cuando exista, o documentar que el PATH sh plano es el aceptado.

**Decisión W1 que pide aceptar**:
- [ ] El contrato es `-e bash -lc` con fallback `-e sh -c`.

---

## 4. Manejo de errores (F2 + F6)

Exit codes solos no bastan: `wsl --list` (con cero distros) y `wsl -d <missing>`
devuelven **ambos -1** con texto similar (F2). Quorb debe parsear el token del
output:

| Token en el output | Significado |
|---|---|
| `Wsl/Service/WSL_E_DISTRO_NOT_FOUND` | Distro ausente (nombre inválido) |
| `"no installed distributions"` | Cero distros |
| `"Invalid command line argument"` | Argumento inválido del CLI |
| (timeout) | kill launcher → `--terminate <distro>` |

**Decisión W1 que pide aceptar**:
- [ ] La app parsea el token `Wsl/Service/...` y usa sondas separadas para
      `WSL ausente` vs `cero distros` (F6).

---

## 5. Precedencia de `credentials_path` (F3 + lectura del archivo de sesión)

Precedencia de resolución del archivo de credenciales de un CLI:

1. `credentials_path` explícito del usuario → ese path (sin cambio respecto a hoy).
2. En modo `wsl/<distro>` sin `credentials_path`: home del default user vía
   `wsl -d <distro> -- getent passwd <user>` (F5), luego el path del proveedor
   (`.claude/.credentials.json`, `.codex/auth.json`, `.gemini/oauth_creds.json`).
3. En modo `native`: default Windows (hoy).

**Nota F8**: en WSL, `wsl -d <distro> -- <cmd>` corre en el directorio de la
distro, no en el home. El login CLI debe ejecutarse con el shell que carga el
PATH del user (ver §3).

**Decisión W1 que pide aceptar**:
- [ ] La precedencia es `credentials_path` explícito > `wsl -d ... getent passwd` > default.

---

## 6. Defensa de `--import` (F3)

`wsl --import` rechaza nombres con espacios y no-ASCII (run `39124241492`):

| Nombre | Resultado |
|---|---|
| `Hello World` (espacio) | `E_INVALIDARG` |
| `PruebaÑ` (latin) | `E_INVALIDARG` |
| `Quorb-Проверка` (cyrillic) | `E_INVALIDARG` |
| `Quorb-世界` (CJK) | `E_INVALIDARG` |
| `CmdSpaceName_test` (puntos/guiones/baja) | Aceptado |

- El parser de distros debe tolerar el rechazo (el import falla; la app no
  debe afirmar que esa distro existe).
- La enumeración real devuelve nombres de store (`Ubuntu-22.04`, `Debian`).
- Los tests de W2 pueden incluir casos de rechazo (edge cases), **no
  asumirlos en la práctica normal**.

**Decisión W1 que pide aceptar**:
- [ ] Los tests de W2 incluyen casos de rechazo de nombres en `--import`
      (edge cases), pero el parseo normal asume nombres de store.

---

## 7. Criterios de aceptación (consolidados del issue)

- [ ] Config ausente → modo `native`, todos los demás datos preservados.
- [ ] Modo y distro persisten; se sanea config inválida; se distingue distro
      previamente elegida pero ya no disponible (F6).
- [ ] Enumerar (sin despertar, F4) y resolver home (con despertar, F7) son dos
      operaciones distintas; la app no las mezcla.
- [ ] Login y lectura usan el mismo contrato de entorno/carpeta/variables.
- [ ] Cambiar de distro invalida cachés dependientes de la sesión.
- [ ] Tests deterministas con adaptadores inyectables (W2).
- [ ] Sin tokens en UI/logs/errores; sin fallback a Windows cuando la distro falla.

## 8. Fuera de alcance (W2/W3)

- Persistencia/distcovery con adaptadores inyectables (W2).
- Suite pertinente y evidencia real Windows/WSL (W3).
- Cambiar la ubicación de datos propios de Quorb (issue #91, fuera de este W1).

## 9. Decisiones abiertas delegadas a W2

1. **UNC vs `wsl -d ... cat`** para leer el archivo de credenciales dentro de
   la distro. UNC despierta (F7); `wsl -d <distro> -- cat <path>` también.
   Ambos despiertan; la decisión es cuál usa la implementación (probablemente
   `wsl -d ... cat` para poder aplicar el timeout con `--terminate`).
2. **Representación de `wsl/<distro>` en config** (sintaxis, validación).
3. **Migración de distro "seleccionada pero no disponible"** (F6) — la app debe
   distinguir este caso (mostrar el error de F2) sin crashear ni caer al home
   de otra distro.

## 10. Validación de este borrador

- El owner debe marcar (aceptar) cada checkbox de las secciones 1–6 y la 7.
- Cualquier checkbox sin marcar → W2 no arranca hasta aceptarlo.
- El borrador es un doc en la rama `investigacion/w0-wsl-lab` del fork
  `sosan/quorb`, enlazado desde el issue #90.
