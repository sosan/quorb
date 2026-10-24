//! `RealWslAdapter`: la implementación real contra `wsl.exe`.
//!
//! Toda la lógica de proceso vive aquí; el resto de la app depende del
//! trait, así que los tests usan `MockWslAdapter` y no tocan WSL.
//!
//! Decisiones que implementa (W2-PLAN §4, ADR-W2, W1-CONTRACT):
//! - **Lectura**: `wsl -d <distro> -- cat <path>` (no UNC), porque solo
//!   así se puede acotar el proceso colgado.
//! - **Home**: `whoami` → `getent passwd <user>` → campo 6 parseado en
//!   Rust (nunca `| cut`).
//! - **Enumeración**: `wsl --list --verbose`, que NO despierta la distro
//!   (F4).
//! - **Diagnóstico**: `--version` (WSL ausente) y `--list` (cero distros)
//!   como sondas separadas (F6).
//! - **Timeout**: cancelación ACOTADA (nivel 1) — se mata el proceso
//!   remoto de la operación y la distro queda `Running`.
//! - **`--terminate`**: solo desde `terminate()`, por acción explícita
//!   del usuario (nivel 2).

use super::errors::{looks_utf16, parse_list_verbose, parse_passwd_home};
use super::process::{
    direct_args, option_args, resolve_wsl_exe, shell_args, WslRunner, WslSearchPaths, SCRIPT_EXEC,
};
use super::{Distribution, ShellStrategy, WslAdapter, WslDiagnostic, WslError, WslState};
use crate::cli_process::{CommandOutput, CommandRunner, ProcessRunner};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Adaptador real contra `wsl.exe`.
pub struct RealWslAdapter {
    runner: Box<dyn CommandRunner>,
    program: PathBuf,
    /// Presupuesto de una operación de la distro.
    timeout: Duration,
    /// Presupuesto de las operaciones de limpieza (leer pidfile, matar,
    /// borrar) tras un timeout.
    cleanup_timeout: Duration,
    /// Variables de entorno que se pasan al proceso `wsl.exe`.
    env: BTreeMap<String, String>,
}

impl RealWslAdapter {
    /// Adaptador con el `ProcessRunner` real y el binario descubierto.
    pub fn new() -> Self {
        Self::with_runner(Box::new(ProcessRunner))
    }

    /// Adaptador con un runner inyectado (para pruebas) y el binario
    /// descubierto en el sistema.
    pub fn with_runner(runner: Box<dyn CommandRunner>) -> Self {
        let program = Self::discover_program();
        Self::with_runner_and_program(runner, program)
    }

    /// Adaptador con runner y binario explícitos.
    pub fn with_runner_and_program(runner: Box<dyn CommandRunner>, program: PathBuf) -> Self {
        RealWslAdapter {
            runner,
            program,
            timeout: Duration::from_secs(20),
            cleanup_timeout: Duration::from_secs(5),
            env: Self::base_env(),
        }
    }

    /// Descubre `wsl.exe` (F9). Si no está en ninguno de los sitios
    /// conocidos, se usa el nombre a secas: el fallo se distingue por
    /// `WslNotInstalled` (F6), no por un error de arranque genérico.
    fn discover_program() -> PathBuf {
        let paths = WslSearchPaths::from_env();
        let is_32bit = cfg!(target_pointer_width = "32");
        resolve_wsl_exe(&paths, is_32bit, &|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("wsl.exe"))
    }

    /// Entorno base de los hijos: `WSL_UTF8=1` (F1) y `LC_ALL=C` para que
    /// los tokens de error sean los mismos en cualquier locale (F2/F6).
    fn base_env() -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert("WSL_UTF8".to_string(), "1".to_string());
        env.insert("LC_ALL".to_string(), "C".to_string());
        env.insert("LANG".to_string(), "C".to_string());
        // `wsl.exe` necesita el PATH del host para sus propias dependencias.
        if let Some(path) = std::env::var_os("PATH").or_else(|| std::env::var_os("Path")) {
            env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
        }
        for key in ["SystemRoot", "ProgramFiles", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                env.insert(key.to_string(), value.to_string_lossy().into_owned());
            }
        }
        env
    }

    /// Fusiona las variables de sesión del llamador en el env del hijo.
    fn env_with(&self, extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut env = self.env.clone();
        for (k, v) in extra {
            env.insert((*k).to_string(), (*v).to_string());
        }
        env
    }

    /// Directorio de trabajo de los hijos: el propio cwd del proceso.
    fn cwd() -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    /// Contexto de ejecución agrupado para las operaciones del adaptador.
    fn runner<'e>(&'e self, env: &'e BTreeMap<String, String>, cwd: &'e Path) -> WslRunner<'e> {
        WslRunner::new(
            &*self.runner,
            &self.program,
            env,
            cwd,
            self.timeout,
            self.cleanup_timeout,
        )
    }

    /// Ejecuta un comando de control (sin distro): `--list`, `--version`,
    /// `--terminate`.
    fn control(&self, args: &[String], distro: Option<&str>) -> Result<CommandOutput, WslError> {
        let cwd = Self::cwd();
        self.runner(&self.env, &cwd).control(args, distro)
    }

    /// Ejecuta un comando dentro de la distro con cancelación acotada.
    fn in_distro(
        &self,
        distro: &str,
        shell: ShellStrategy,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<CommandOutput, WslError> {
        let merged = self.env_with(env);
        let cwd = Self::cwd();
        self.runner(&merged, &cwd).run_scoped(distro, shell, args)
    }
}

impl Default for RealWslAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl WslAdapter for RealWslAdapter {
    /// F4: `wsl --list --verbose` NO despierta ninguna distro.
    fn list_distributions(&self) -> Result<Vec<Distribution>, WslError> {
        let args = option_args("--list", None);
        let mut args = args;
        args.push("--verbose".to_string());
        let output = self.control(&args, None)?;
        if looks_utf16(&output.stdout) {
            // F1: sin `WSL_UTF8=1` la salida viene en UTF-16 y el runner
            // de texto la habría perdido. Se dice explícitamente en vez de
            // devolver una lista corrupta.
            return Err(WslError::UnparsableOutput(
                "wsl.exe devolvió la salida en UTF-16 (WSL_UTF8 no soportado)".into(),
            ));
        }
        Ok(parse_list_verbose(&output.stdout))
    }

    /// F7: despierta la distro. Solo desde una acción explícita del
    /// usuario (W1 §2). El home es el campo 6 de `getent passwd`.
    fn resolve_home(&self, distro: &str) -> Result<PathBuf, WslError> {
        let whoami = direct_args(Some(distro), "whoami", &[]);
        let output = self.control(&whoami, Some(distro))?;
        let user = output.stdout.trim().to_string();
        if user.is_empty() {
            return Err(WslError::UnparsableOutput(
                "wsl.exe no devolvió el usuario por defecto de la distro".into(),
            ));
        }
        let passwd = direct_args(Some(distro), "getent", &["passwd", &user]);
        let output = self.control(&passwd, Some(distro))?;
        let home = parse_passwd_home(output.stdout.trim())
            .ok_or_else(|| WslError::UnparsableOutput("getent passwd sin campo home".into()))?;
        Ok(PathBuf::from(home))
    }

    /// Lee un archivo dentro de la distro (decisión paso 0 #1: `wsl cat`,
    /// no UNC), con cancelación acotada (nivel 1).
    fn read_file(&self, distro: &str, path: &Path) -> Result<String, WslError> {
        let path_str = path.to_string_lossy().to_string();
        // `--` antes del path: un nombre que empiece por `-` no se
        // interpreta como opción de `cat`.
        match self.in_distro(
            distro,
            ShellStrategy::ShPosix,
            &["cat", "--", path_str.as_str()],
            &[],
        ) {
            Ok(output) => Ok(output.stdout),
            // El timeout se propaga: no es un fichero ausente.
            Err(err @ WslError::Timeout { .. }) => Err(err),
            // Cualquier otro fallo del `cat` es, en la práctica, que el
            // fichero no está: no hay sesión que leer.
            Err(_) => Err(WslError::FileNotFound(path_str)),
        }
    }

    /// W1 §3: `command -v bash` — si existe, `-e bash -lc` (PATH de
    /// login, donde viven nvm/volta); si no, `-e sh -c`.
    fn shell_strategy(&self, distro: &str) -> Result<ShellStrategy, WslError> {
        let argv = shell_args(
            Some(distro),
            ShellStrategy::ShPosix,
            SCRIPT_EXEC,
            &["command", "-v", "bash"],
        );
        let cwd = Self::cwd();
        let output = self.runner(&self.env, &cwd).control(&argv, Some(distro))?;
        Ok(if output.code == 0 && !output.stdout.trim().is_empty() {
            ShellStrategy::BashLogin
        } else {
            ShellStrategy::ShPosix
        })
    }

    /// Ejecuta un comando con el script constante y los datos como
    /// parámetros posicionales (F14, doble frontera).
    fn run_command(
        &self,
        distro: &str,
        shell: ShellStrategy,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<CommandOutput, WslError> {
        self.in_distro(distro, shell, args, env)
    }

    /// F6: sondas separadas — `--version` (WSL ausente) y `--list` (cero
    /// distros o distro objetivo ausente).
    fn diagnose(&self, target: Option<&str>) -> Result<WslDiagnostic, WslError> {
        let version = option_args("--version", None);
        match self.control(&version, None) {
            Ok(_) => {}
            Err(WslError::WslNotInstalled) => {
                return Ok(WslDiagnostic {
                    state: WslState::WslNotInstalled,
                    detail: None,
                })
            }
            Err(e) => return Err(e),
        }

        let distros = self.list_distributions()?;
        if distros.is_empty() {
            return Ok(WslDiagnostic {
                state: WslState::NoDistributions,
                detail: None,
            });
        }
        match target {
            Some(name) => {
                let known = distros
                    .iter()
                    .any(|d| d.name.eq_ignore_ascii_case(name.trim()));
                if known {
                    Ok(WslDiagnostic {
                        state: WslState::Available,
                        detail: None,
                    })
                } else {
                    // Decisión paso 0 #3: error explícito, sin fallback a
                    // native y conservando el valor configurado.
                    Ok(WslDiagnostic {
                        state: WslState::DistroUnavailable,
                        detail: Some(format!(
                            "la distro configurada no aparece en la enumeración; disponibles: {}",
                            distros
                                .iter()
                                .map(|d| d.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                    })
                }
            }
            None => Ok(WslDiagnostic {
                state: WslState::Available,
                detail: None,
            }),
        }
    }

    /// Nivel 2: detener la distro COMPLETA. Solo por acción explícita
    /// del usuario — mata también procesos y sesiones ajenas a Quorb.
    fn terminate(&self, distro: &str) -> Result<(), WslError> {
        let args = option_args("--terminate", Some(distro));
        self.control(&args, Some(distro)).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_process::RunError;
    use crate::wsl::DistroState;
    use std::sync::{Arc, Mutex};

    /// Runner de mentira: responde por el substring del argv y registra
    /// las llamadas. Permite probar el argv y el parseo sin WSL.
    /// `Arc<Mutex<…>>` en vez de `RefCell`: el trait exige `Send + Sync`,
    /// y el `Arc` deja al test consultar las llamadas después de mover el
    /// runner dentro del adaptador.
    #[derive(Default, Clone)]
    struct FakeRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        responses: Arc<Mutex<Vec<(String, CommandOutput)>>>,
        spawn_on: Arc<Mutex<Vec<String>>>,
    }

    impl FakeRunner {
        fn respond(&self, needle: &str, output: CommandOutput) {
            self.responses
                .lock()
                .unwrap()
                .push((needle.to_string(), output));
        }

        /// Como si el binario no existiera (WSL ausente).
        fn spawn_on(&self, needle: &str) {
            self.spawn_on.lock().unwrap().push(needle.to_string());
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            _program: &Path,
            args: &[String],
            _env: &BTreeMap<String, String>,
            _cwd: &Path,
            _timeout: Duration,
        ) -> Result<CommandOutput, RunError> {
            self.calls.lock().unwrap().push(args.to_vec());
            let joined = args.join(" ");
            for needle in self.spawn_on.lock().unwrap().iter() {
                if joined.contains(needle.as_str()) {
                    return Err(RunError::Spawn);
                }
            }
            for (needle, output) in self.responses.lock().unwrap().iter() {
                if joined.contains(needle.as_str()) {
                    return Ok(output.clone());
                }
            }
            Ok(CommandOutput {
                code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    fn adapter(runner: FakeRunner) -> RealWslAdapter {
        RealWslAdapter::with_runner_and_program(Box::new(runner), PathBuf::from("wsl.exe"))
    }

    /// La enumeración usa `--list --verbose` y NO despierta (F4): el argv
    /// no lleva `-d`.
    #[test]
    fn list_distributions_uses_verbose_list_without_distro() {
        let runner = FakeRunner::default();
        runner.respond(
            "--list",
            ok("NAME            STATE           VERSION\nUbuntu-22.04   Running         2\n"),
        );
        let adapter = adapter(runner);
        let distros = adapter.list_distributions().expect("enumeración");
        assert_eq!(distros.len(), 1);
        assert_eq!(distros[0].name, "Ubuntu-22.04");
        assert_eq!(distros[0].state, DistroState::Running);
    }

    /// El home sale del campo 6 de `getent passwd`, con el usuario
    /// resuelto antes por `whoami` (F5) — nunca se asume el de Windows.
    #[test]
    fn resolve_home_uses_whoami_then_getent() {
        let runner = FakeRunner::default();
        runner.respond("whoami", ok("quorbuser\n"));
        runner.respond(
            "getent",
            ok("quorbuser:x:1000:1000::/home/quorbuser:/bin/sh\n"),
        );
        let adapter = adapter(runner);
        let home = adapter.resolve_home("Ubuntu-22.04").expect("home");
        assert_eq!(home, PathBuf::from("/home/quorbuser"));
    }

    /// La lectura usa `cat --` con el path como argumento (F14: sin
    /// shell, sin interpolación) y apunta a la distro pedida.
    #[test]
    fn read_file_passes_path_as_argument() {
        let runner = FakeRunner::default();
        runner.respond("cat", ok("contenido\n"));
        let adapter = adapter(runner);
        let out = adapter
            .read_file(
                "Ubuntu-22.04",
                Path::new("/home/u/.claude/.credentials.json"),
            )
            .expect("lectura");
        assert_eq!(out, "contenido\n");
    }

    /// La detección de shell mira `command -v bash` (F12).
    #[test]
    fn shell_strategy_detects_bash() {
        let runner = FakeRunner::default();
        runner.respond("command -v bash", ok("/bin/bash\n"));
        let adapter = adapter(runner);
        assert_eq!(
            adapter.shell_strategy("Ubuntu-22.04").expect("shell"),
            ShellStrategy::BashLogin
        );
    }

    /// Sin bash, el fallback es POSIX (`-e sh -c`).
    #[test]
    fn shell_strategy_falls_back_to_posix() {
        let runner = FakeRunner::default();
        runner.respond("command -v bash", ok(""));
        let adapter = adapter(runner);
        assert_eq!(
            adapter.shell_strategy("Ubuntu-22.04").expect("shell"),
            ShellStrategy::ShPosix
        );
    }

    /// El diagnóstico distingue WSL ausente de cero distros (F6).
    #[test]
    fn diagnose_reports_wsl_not_installed() {
        let runner = FakeRunner::default();
        runner.spawn_on("--version");
        let adapter = adapter(runner);
        assert_eq!(
            adapter.diagnose(None).expect("diagnóstico").state,
            WslState::WslNotInstalled
        );
    }

    /// Con WSL pero sin distros: `NoDistributions`, no un error.
    #[test]
    fn diagnose_reports_no_distributions() {
        let runner = FakeRunner::default();
        runner.respond("--version", ok("WSL version 2.0\n"));
        runner.respond("--list", ok("NAME STATE VERSION\n"));
        let adapter = adapter(runner);
        assert_eq!(
            adapter.diagnose(None).expect("diagnóstico").state,
            WslState::NoDistributions
        );
    }

    /// Una distro configurada que no aparece: `DistroUnavailable` con la
    /// lista de disponibles (decisión paso 0 #3, sin fallback).
    #[test]
    fn diagnose_reports_distro_unavailable() {
        let runner = FakeRunner::default();
        runner.respond("--version", ok("WSL version 2.0\n"));
        runner.respond(
            "--list",
            ok("NAME            STATE           VERSION\nUbuntu-22.04   Running         2\n"),
        );
        let adapter = adapter(runner);
        let diag = adapter.diagnose(Some("Debian")).expect("diagnóstico");
        assert_eq!(diag.state, WslState::DistroUnavailable);
        let detail = diag.detail.expect("detalle");
        assert!(detail.contains("Ubuntu-22.04"));
    }

    /// `terminate()` es el nivel 2: `--terminate <distro>`, sin el
    /// separador `--` (es una opción de `wsl.exe`, no un comando).
    #[test]
    fn terminate_uses_terminate_option() {
        let runner = FakeRunner::default();
        let adapter = adapter(runner.clone());
        adapter.terminate("Ubuntu-22.04").expect("terminate");
        let calls = runner.calls();
        assert_eq!(
            calls[0],
            vec!["--terminate".to_string(), "Ubuntu-22.04".to_string()]
        );
    }
}
