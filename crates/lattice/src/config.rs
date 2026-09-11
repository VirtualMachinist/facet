//! `[lattice]` configuration: workspace file, machine file, built-in default.
//! Precedence is applied by the caller: CLI flag > workspace file > machine
//! file > default.

use std::{fmt, fs, path::Path};

use serde::Deserialize;

/// Default inline body threshold: 64 KiB. A guess until measured.
pub const DEFAULT_INLINE_BODY_MAX: u64 = 64 * 1024;
/// Default `busy_timeout` in milliseconds.
pub const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;

/// Run-history retention window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retention {
    /// Never expire runs.
    Unlimited,
    /// Expire runs older than this many days.
    Days(u32),
}

impl Retention {
    /// Milliseconds before "now" at which runs expire, if bounded.
    #[must_use]
    pub fn window_ms(self) -> Option<i64> {
        match self {
            Self::Unlimited => None,
            Self::Days(days) => Some(i64::from(days) * 86_400_000),
        }
    }
}

impl fmt::Display for Retention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unlimited => f.write_str("unlimited"),
            Self::Days(days) => write!(f, "{days}d"),
        }
    }
}

/// Engine used for both workspace history and machine state.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// Bundled SQLite, the default for existing installations.
    #[default]
    Sqlite,
    /// Reserved; selecting Turso is rejected until a driver is wired in.
    Turso,
}

impl Engine {
    /// Stable engine identifier used by configuration and database markers.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Turso => "turso",
        }
    }
}

/// Effective Lattice configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatticeConfig {
    /// Explicit storage engine; compilation alone never changes it.
    pub engine: Engine,
    /// Bodies at or under this many bytes are stored inline in the row.
    pub inline_body_max: u64,
    /// Run retention window applied by `gc`.
    pub history_retention: Retention,
    /// Open connections in WAL journal mode.
    pub wal: bool,
    /// SQLite busy timeout applied at connection open.
    pub busy_timeout_ms: u64,
}

impl Default for LatticeConfig {
    fn default() -> Self {
        Self {
            engine: Engine::Sqlite,
            inline_body_max: DEFAULT_INLINE_BODY_MAX,
            history_retention: Retention::Unlimited,
            wal: true,
            busy_timeout_ms: DEFAULT_BUSY_TIMEOUT_MS,
        }
    }
}

/// Configuration parse failure.
#[derive(Debug)]
pub enum ConfigError {
    /// A byte-size string such as `64KiB` could not be parsed.
    InvalidByteSize(String),
    /// A retention string such as `30d` could not be parsed.
    InvalidRetention(String),
    /// The TOML document could not be parsed.
    Toml(String),
    /// The configuration file could not be read.
    Io(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByteSize(value) => {
                write!(
                    f,
                    "invalid byte size {value:?}; expected e.g. 64KiB, 1MiB, or 0"
                )
            }
            Self::InvalidRetention(value) => {
                write!(
                    f,
                    "invalid retention {value:?}; expected unlimited or e.g. 30d"
                )
            }
            Self::Toml(message) => write!(f, "invalid config.toml: {message}"),
            Self::Io(message) => write!(f, "cannot read config.toml: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    lattice: LatticeSection,
}

#[derive(Debug, Default, Deserialize)]
struct LatticeSection {
    engine: Option<Engine>,
    inline_body_max: Option<String>,
    history_retention: Option<String>,
    wal: Option<bool>,
    busy_timeout_ms: Option<u64>,
}

impl LatticeConfig {
    /// Loads the machine file (if any) and then the workspace file (if any)
    /// over the built-in default.
    pub fn load(workspace_root: &Path) -> Result<Self, ConfigError> {
        let mut config = Self::default();
        if let Some(dir) = crate::machine_config_dir() {
            config.apply_file(&dir.join(crate::CONFIG_FILE))?;
        }
        config.apply_file(
            &workspace_root
                .join(crate::FACET_DIR)
                .join(crate::CONFIG_FILE),
        )?;
        Ok(config)
    }

    /// Applies a config file when it exists. A missing file is not an error.
    pub fn apply_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        match fs::read_to_string(path) {
            Ok(source) => self.apply_source(&source),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConfigError::Io(format!("{}: {error}", path.display()))),
        }
    }

    /// Applies a TOML document over the current values.
    pub fn apply_source(&mut self, source: &str) -> Result<(), ConfigError> {
        let file: ConfigFile =
            toml::from_str(source).map_err(|error| ConfigError::Toml(error.to_string()))?;
        if let Some(value) = file.lattice.engine {
            self.engine = value;
        }
        if let Some(value) = file.lattice.inline_body_max {
            self.inline_body_max = parse_byte_size(&value)?;
        }
        if let Some(value) = file.lattice.history_retention {
            self.history_retention = parse_retention(&value)?;
        }
        if let Some(value) = file.lattice.wal {
            self.wal = value;
        }
        if let Some(value) = file.lattice.busy_timeout_ms {
            self.busy_timeout_ms = value;
        }
        Ok(())
    }
}

/// Parses `64KiB`, `1MiB`, `2GiB`, `4096`, or `0` into bytes.
pub fn parse_byte_size(value: &str) -> Result<u64, ConfigError> {
    let trimmed = value.trim();
    let split = trimmed
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (digits, unit) = trimmed.split_at(split);
    let number: u64 = digits
        .parse()
        .map_err(|_| ConfigError::InvalidByteSize(value.to_owned()))?;
    let multiplier: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kib" => 1024,
        "m" | "mib" => 1024 * 1024,
        "g" | "gib" => 1024 * 1024 * 1024,
        _ => return Err(ConfigError::InvalidByteSize(value.to_owned())),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| ConfigError::InvalidByteSize(value.to_owned()))
}

/// Parses `unlimited` or `<days>d`.
pub fn parse_retention(value: &str) -> Result<Retention, ConfigError> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("unlimited") {
        return Ok(Retention::Unlimited);
    }
    trimmed
        .strip_suffix('d')
        .and_then(|days| days.parse::<u32>().ok())
        .filter(|days| *days > 0)
        .map(Retention::Days)
        .ok_or_else(|| ConfigError::InvalidRetention(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{LatticeConfig, Retention, parse_byte_size, parse_retention};

    #[test]
    fn parses_human_byte_sizes() {
        assert_eq!(parse_byte_size("64KiB").unwrap(), 65_536);
        assert_eq!(parse_byte_size("1 MiB").unwrap(), 1_048_576);
        assert_eq!(parse_byte_size("0").unwrap(), 0);
        assert_eq!(parse_byte_size("4096").unwrap(), 4096);
        assert!(parse_byte_size("lots").is_err());
        assert!(parse_byte_size("1TB").is_err());
    }

    #[test]
    fn parses_retention() {
        assert_eq!(parse_retention("unlimited").unwrap(), Retention::Unlimited);
        assert_eq!(parse_retention("30d").unwrap(), Retention::Days(30));
        assert!(parse_retention("0d").is_err());
        assert!(parse_retention("30").is_err());
    }

    #[test]
    fn applies_toml_over_defaults() {
        let mut config = LatticeConfig::default();
        config
            .apply_source(
                "[lattice]\ninline_body_max = \"1MiB\"\nhistory_retention = \"90d\"\nwal = false\n",
            )
            .unwrap();
        assert_eq!(config.inline_body_max, 1_048_576);
        assert_eq!(config.history_retention, Retention::Days(90));
        assert!(!config.wal);
        assert_eq!(config.busy_timeout_ms, 5_000);
    }

    #[test]
    fn empty_document_keeps_defaults() {
        let mut config = LatticeConfig::default();
        config.apply_source("").unwrap();
        assert_eq!(config, LatticeConfig::default());
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    #[test]
    fn engine_is_explicit_and_unknown_names_are_rejected() {
        let mut config = LatticeConfig::default();
        assert_eq!(config.engine, Engine::Sqlite);
        config
            .apply_source("[lattice]\nengine = \"turso\"")
            .unwrap();
        assert_eq!(config.engine, Engine::Turso);
        assert!(
            config
                .apply_source("[lattice]\nengine = \"libsql\"")
                .is_err()
        );
    }

    #[test]
    fn unavailable_engine_never_creates_a_sqlite_database() {
        let dir = tempfile::tempdir().unwrap();
        let config = LatticeConfig {
            engine: Engine::Turso,
            ..LatticeConfig::default()
        };
        assert!(matches!(
            crate::WorkspaceStore::open(dir.path(), config),
            Err(crate::LatticeError::Engine(_))
        ));
        assert!(!dir.path().join(".facet/lattice.db").exists());
        assert!(!dir.path().join(".facet/lattice.db.engine").exists());
    }
}
