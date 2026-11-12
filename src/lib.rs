//! Luau for Flareite. Highlighting needs no code; this configures luau-lsp
//! (which Flareite runs) from the extension's settings.
//!
//! With the Roblox platform, luau-lsp gets Roblox's types (for the chosen
//! security level) and API docs as `--definitions` and `--docs`, files
//! Flareite downloads and keeps (`${download:...}` arguments); with standard
//! Luau, neither. Its `platform.type` setting follows. The manifest's
//! arguments and settings are the default settings' ones, so with the
//! defaults nothing changes here, and the server, which waits for this
//! extension to activate, starts once. They're set again when the settings
//! change.
//!
//! In Rojo projects, luau-lsp resolves requires and instance paths with the
//! `sourcemap.json` the Rojo extension keeps up to date; Flareite tells
//! luau-lsp when it changes.
//!
//! It also types `end` for blocks as Enter opens them (see [`blocks`]).

mod blocks;

use flareite_extension_api::events::Event;
use flareite_extension_api::language_features::{AutoInsertRequest, Insertion};
use flareite_extension_api::types::{Activation, CommandContext, ConfigValue, LogLevel};
use flareite_extension_api::{Extension, host, language_servers, register};

const SERVER: &str = "luau-lsp";
const RESTART_SERVER: &str = "flareite.luau.restartServer";
/// Most lines one read returns.
const READ_LINES: u32 = 10_000;

/// `end` to insert after Enter, if a block needs one.
fn auto_insert(request: &AutoInsertRequest) -> Option<Insertion> {
  if request.trigger != "\n" {
    return None;
  }
  let document = host::document(request.document).ok()?;
  // The whole document, as it was when Enter was pressed.
  let mut lines = Vec::new();
  let mut start = 0;
  while start < document.line_count {
    let slice = host::read_lines(request.document, start, start + READ_LINES).ok()?;
    if slice.version != request.version || slice.lines.is_empty() {
      return None;
    }
    start += u32::try_from(slice.lines.len()).ok()?;
    lines.extend(slice.lines);
  }
  let (line, column) = (
    request.position.line as usize,
    request.position.column as usize,
  );
  let (text, cursor) = blocks::after_enter(&lines, line, column, &request.indent)?;
  Some(Insertion {
    text,
    cursor: u32::try_from(cursor).ok()?,
  })
}

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
}

impl Config {
  fn read() -> Self {
    let text = |key: &str, default: &str| match host::configuration(key) {
      Some(ConfigValue::Text(text)) => text,
      _ => default.to_owned(),
    };
    let security_level = text("roblox_security_level", "PluginSecurity");
    Self {
      roblox: text("platform", "roblox") != "standard",
      security_level: if SECURITY_LEVELS.contains(&security_level.as_str()) {
        security_level
      } else {
        "PluginSecurity".into()
      },
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
}

/// Sets luau-lsp's arguments and settings from the settings. They're the
/// manifest's for the default settings, which changes nothing then.
fn configure() {
  let config = Config::read();
  if let Err(error) = language_servers::set_arguments(SERVER, &config.server_args()) {
    host::log(
      LogLevel::Warn,
      &format!("couldn't set luau-lsp's arguments: {error:?}"),
    );
  }
  if let Err(error) = language_servers::set_settings(SERVER, &config.server_settings()) {
    host::log(
      LogLevel::Warn,
      &format!("couldn't set luau-lsp's settings: {error:?}"),
    );
  }
}

struct Luau;

impl Extension for Luau {
  fn activate(_: Activation) -> Result<Self, String> {
    // Before luau-lsp starts: it waits for this.
    configure();
    Ok(Luau)
  }

  fn run_command(&mut self, command: &str, _: CommandContext) -> Result<(), String> {
    match command {
      RESTART_SERVER => language_servers::restart(SERVER).map_err(|error| format!("{error:?}")),
      _ => Err(format!("unknown command {command}")),
    }
  }

  /// The settings changed (the manifest's `events`): configure luau-lsp again.
  fn handle_event(&mut self, event: Event) -> Result<(), String> {
    if let Event::ConfigurationChanged = event {
      configure();
    }
    Ok(())
  }

  fn provide_auto_insert(
    &mut self,
    request: AutoInsertRequest,
  ) -> Result<Option<Insertion>, String> {
    Ok(auto_insert(&request))
  }
}

register!(Luau);

#[cfg(test)]
mod tests {
  use super::*;

  fn config(roblox: bool) -> Config {
    Config {
      roblox,
      security_level: "PluginSecurity".into(),
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
}
