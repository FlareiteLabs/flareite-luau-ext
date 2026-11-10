//! Luau for Flareite. Highlighting needs no code; this configures luau-lsp
//! (which Flareite runs) from the extension's settings, and keeps a Rojo
//! project's `sourcemap.json` up to date.
//!
//! - **The server.** With the Roblox platform, luau-lsp gets Roblox's types
//!   (for the chosen security level) and API docs as `--definitions` and
//!   `--docs`, files Flareite downloads and keeps (`${download:...}`
//!   arguments); with standard Luau, neither. Its `platform.type` setting
//!   follows. The manifest's arguments and settings are the default
//!   settings' ones, so with the defaults nothing changes here, and the
//!   server, which waits for this extension to activate, starts once.
//! - **Sourcemaps.** luau-lsp resolves requires and instance paths with a
//!   `sourcemap.json` Rojo writes. In a trusted folder with the Rojo
//!   project file, this runs `rojo sourcemap --watch`, which rewrites it as
//!   files change; Flareite tells luau-lsp when it does.
//!
//! Both are looked at again when the settings change, or the folder (or
//! whether it's trusted) does.

use std::cell::RefCell;

use flareite_extension_api::processes::{self, ProcessError, ProcessOptions};
use flareite_extension_api::types::{
  Activation, CommandContext, ConfigValue, DocumentChange, DocumentId, DocumentInfo, LogLevel,
  MessageLevel,
};
use flareite_extension_api::{Guest, export_extension, files, host, language_servers, workspace};

const SERVER: &str = "luau-lsp";
const RESTART_SERVER: &str = "flareite.luau.restartServer";
const REGENERATE_SOURCEMAP: &str = "flareite.luau.regenerateSourcemap";
const CONFIGURE: &str = "flareite.luau.configure";
const SOURCEMAP_STOPPED: &str = "flareite.luau.sourcemapStopped";

/// Roblox's types for a security level are at this address, then
/// `<level>.d.luau`.
const ROBLOX_TYPES: &str = "https://luau-lsp.pages.dev/type-definitions/globalTypes.";
const ROBLOX_DOCS: &str = "https://luau-lsp.pages.dev/api-docs/en-us.json";
const SECURITY_LEVELS: [&str; 4] = [
  "None",
  "LocalUserSecurity",
  "PluginSecurity",
  "RobloxScriptSecurity",
];

/// What the settings ask for.
#[derive(Debug, Clone, PartialEq)]
struct Config {
  roblox: bool,
  security_level: String,
  sourcemap: bool,
  project_file: String,
  include_non_scripts: bool,
}

impl Config {
  fn read() -> Self {
    let text = |key: &str, default: &str| match host::configuration(key) {
      Some(ConfigValue::Text(text)) => text,
      _ => default.to_owned(),
    };
    let flag = |key: &str| !matches!(host::configuration(key), Some(ConfigValue::Boolean(false)));
    let security_level = text("roblox_security_level", "PluginSecurity");
    Self {
      roblox: text("platform", "roblox") != "standard",
      security_level: if SECURITY_LEVELS.contains(&security_level.as_str()) {
        security_level
      } else {
        "PluginSecurity".into()
      },
      sourcemap: flag("sourcemap"),
      project_file: text("rojo_project_file", "default.project.json"),
      include_non_scripts: flag("sourcemap_include_non_scripts"),
    }
  }

  /// luau-lsp's arguments.
  fn server_args(&self) -> Vec<String> {
    let mut args = vec!["lsp".to_owned()];
    if self.roblox {
      args.push(format!(
        "--definitions:@roblox=${{download:{ROBLOX_TYPES}{}.d.luau}}",
        self.security_level
      ));
      args.push(format!("--docs=${{download:{ROBLOX_DOCS}}}"));
    }
    args
  }

  /// luau-lsp's settings, as JSON.
  fn server_settings(&self) -> String {
    let platform = if self.roblox { "roblox" } else { "standard" };
    format!(r#"{{"luau-lsp.platform.type": "{platform}"}}"#)
  }

  /// `rojo`'s arguments for making sourcemaps of `project`.
  fn rojo_args(&self) -> Vec<String> {
    let mut args = vec![
      "sourcemap".to_owned(),
      self.project_file.clone(),
      "--watch".to_owned(),
      "--output".to_owned(),
      "sourcemap.json".to_owned(),
    ];
    if self.include_non_scripts {
      args.push("--include-non-scripts".to_owned());
    }
    args
  }
}

/// The `rojo sourcemap --watch` running, and what for.
struct Rojo {
  process: u64,
  folder: String,
  args: Vec<String>,
}

#[derive(Default)]
struct State {
  rojo: Option<Rojo>,
  /// Rojo's failures are shown once, until it runs again.
  failure_shown: bool,
}

thread_local! {
  static STATE: RefCell<State> = RefCell::default();
}

fn log(level: LogLevel, message: &str) {
  host::log(level, message);
}

/// Sets luau-lsp's arguments and settings for `config`. They're the
/// manifest's for the default settings, which changes nothing then.
fn configure_server(config: &Config) {
  if let Err(error) = language_servers::set_arguments(SERVER, &config.server_args()) {
    log(
      LogLevel::Warn,
      &format!("couldn't set luau-lsp's arguments: {error:?}"),
    );
  }
  if let Err(error) = language_servers::set_settings(SERVER, &config.server_settings()) {
    log(
      LogLevel::Warn,
      &format!("couldn't set luau-lsp's settings: {error:?}"),
    );
  }
}

/// The folder Rojo should make sourcemaps in, if any: the open folder, when
/// it's trusted, has the project file, and the settings want sourcemaps.
fn sourcemap_folder(config: &Config) -> Option<String> {
  if !(config.roblox && config.sourcemap) {
    return None;
  }
  let folder = workspace::folder()?;
  if !folder.trusted {
    return None;
  }
  let project = format!(
    "{}/{}",
    folder.path.trim_end_matches(['/', '\\']),
    config.project_file
  );
  files::read_file(&project).ok()?;
  Some(folder.path)
}

/// Starts, stops, or restarts Rojo as `config` and the folder call for;
/// `again` restarts it even if it's running as asked.
fn configure_sourcemap(config: &Config, again: bool) {
  let wanted = sourcemap_folder(config).map(|folder| (folder, config.rojo_args()));
  let running = STATE.with_borrow_mut(|state| {
    let same = match (&state.rojo, &wanted) {
      (Some(rojo), Some((folder, args))) => &rojo.folder == folder && &rojo.args == args,
      _ => false,
    };
    if same && !again {
      return true;
    }
    if let Some(rojo) = state.rojo.take() {
      // Its stopped command finds it isn't the running one, and says
      // nothing.
      processes::kill(rojo.process);
    }
    false
  });
  let Some((folder, args)) = wanted.filter(|_| !running) else {
    return;
  };
  let options = ProcessOptions {
    cwd: Some(folder.clone()),
    env: Vec::new(),
    stdin: None,
    timeout_ms: None,
  };
  match processes::spawn("rojo", &args, &options, SOURCEMAP_STOPPED) {
    Ok(process) => {
      log(
        LogLevel::Info,
        &format!("making sourcemaps: rojo {}", args.join(" ")),
      );
      STATE.with_borrow_mut(|state| {
        state.rojo = Some(Rojo {
          process,
          folder,
          args,
        });
      });
    }
    Err(error) => rojo_failed(&describe(&error)),
  }
}

fn describe(error: &ProcessError) -> String {
  match error {
    ProcessError::NotFound(why) => format!(
      "Rojo isn't installed, or isn't on the PATH ({why}). Install it to keep sourcemap.json up to date, or turn off the Luau extension's Sourcemap setting."
    ),
    ProcessError::PermissionDenied => {
      "Running Rojo isn't allowed: allow the Luau extension to run programs in the Extensions view."
        .into()
    }
    ProcessError::Untrusted => "Rojo runs only in folders you trust.".into(),
    other => format!("Rojo couldn't make sourcemaps: {other:?}"),
  }
}

/// Tells the user why Rojo isn't making sourcemaps, once.
fn rojo_failed(message: &str) {
  log(LogLevel::Warn, message);
  let first = STATE.with_borrow_mut(|state| !std::mem::replace(&mut state.failure_shown, true));
  if first {
    host::show_message(MessageLevel::Warning, message);
  }
}

/// Rojo stopped: says why, unless it was stopped on purpose.
fn sourcemap_stopped() {
  let Some(finished) = processes::finished() else {
    return;
  };
  let current = STATE.with_borrow_mut(|state| {
    let current = state
      .rojo
      .as_ref()
      .is_some_and(|rojo| rojo.process == finished.process);
    if current {
      state.rojo = None;
    }
    current
  });
  if !current {
    return;
  }
  match finished.outcome {
    Ok(output) => {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let why = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("it exited");
      rojo_failed(&format!(
        "Rojo stopped making sourcemaps ({}): {why}",
        output
          .exit_code
          .map_or_else(|| "stopped".to_owned(), |code| format!("exit code {code}"))
      ));
    }
    Err(ProcessError::Killed) => {}
    Err(error) => rojo_failed(&describe(&error)),
  }
}

fn configure(again: bool) {
  let config = Config::read();
  configure_server(&config);
  configure_sourcemap(&config, again);
}

struct Luau;

impl Guest for Luau {
  fn activate(_context: Activation) -> Result<(), String> {
    // Before luau-lsp starts: it waits for this.
    configure(false);
    Ok(())
  }

  fn deactivate() {
    STATE.with_borrow_mut(|state| {
      if let Some(rojo) = state.rojo.take() {
        processes::kill(rojo.process);
      }
    });
  }

  fn run_command(command: String, _context: CommandContext) -> Result<(), String> {
    match command.as_str() {
      CONFIGURE => configure(false),
      REGENERATE_SOURCEMAP => {
        STATE.with_borrow_mut(|state| state.failure_shown = false);
        let config = Config::read();
        if sourcemap_folder(&config).is_none() {
          return Err(format!(
            "Sourcemaps are made for Rojo projects with the Roblox platform: the open folder needs {}, and must be trusted, and the Sourcemap setting on.",
            config.project_file
          ));
        }
        configure_sourcemap(&config, true);
      }
      RESTART_SERVER => language_servers::restart(SERVER).map_err(|error| format!("{error:?}"))?,
      SOURCEMAP_STOPPED => sourcemap_stopped(),
      _ => return Err(format!("unknown command {command}")),
    }
    Ok(())
  }

  fn document_opened(_document: DocumentInfo) {}

  fn document_changed(_change: DocumentChange) {}

  fn document_closed(_document: DocumentId) {}
}

export_extension!(Luau);

#[cfg(test)]
mod tests {
  use super::*;

  fn config(roblox: bool) -> Config {
    Config {
      roblox,
      security_level: "PluginSecurity".into(),
      sourcemap: true,
      project_file: "default.project.json".into(),
      include_non_scripts: true,
    }
  }

  /// The manifest's arguments and settings are the default settings'.
  #[test]
  fn the_manifest_matches_the_default_settings() {
    let manifest = include_str!("../extension.toml");
    let defaults = config(true);
    for arg in defaults.server_args() {
      assert!(manifest.contains(&format!("{arg:?}")), "{arg}");
    }
    assert!(manifest.contains(r#""luau-lsp.platform.type" = "roblox""#));
    assert_eq!(
      defaults.server_settings(),
      r#"{"luau-lsp.platform.type": "roblox"}"#
    );
  }

  #[test]
  fn standard_luau_gets_no_roblox_types() {
    let standard = config(false);
    assert_eq!(standard.server_args(), ["lsp"]);
    assert!(standard.server_settings().contains("standard"));
    let mut level = config(true);
    level.security_level = "None".into();
    assert!(level.server_args()[1].ends_with("globalTypes.None.d.luau}"));
  }

  #[test]
  fn rojo_watches_the_project() {
    let mut scripts_only = config(true);
    assert_eq!(
      scripts_only.rojo_args(),
      [
        "sourcemap",
        "default.project.json",
        "--watch",
        "--output",
        "sourcemap.json",
        "--include-non-scripts"
      ]
    );
    scripts_only.include_non_scripts = false;
    assert!(
      !scripts_only
        .rojo_args()
        .contains(&"--include-non-scripts".to_owned())
    );
  }
}
