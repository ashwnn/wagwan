//! Startup configuration read from the process environment.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_MODEL_URL: &str = "https://huggingface.co/minishlab/potion-base-8M";

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    /// IANA time-zone name used by the presentation layer.
    pub tz: String,
    pub retention_days: u64,
    pub min_score: u8,
    pub fetch_concurrency: usize,
    pub fetch_min_interval: Duration,
    pub fetch_max_interval: Duration,
    pub opml_seed: Option<PathBuf>,
    pub topics_seed: Vec<String>,
    pub model_url: String,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub llm_concurrency: usize,
    pub log: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind", &self.bind)
            .field("data_dir", &self.data_dir)
            .field("tz", &self.tz)
            .field("retention_days", &self.retention_days)
            .field("min_score", &self.min_score)
            .field("fetch_concurrency", &self.fetch_concurrency)
            .field("fetch_min_interval", &self.fetch_min_interval)
            .field("fetch_max_interval", &self.fetch_max_interval)
            .field("opml_seed", &self.opml_seed)
            .field("topics_seed", &self.topics_seed)
            .field("model_url", &self.model_url)
            .field("llm_base_url", &self.llm_base_url)
            .field(
                "llm_api_key",
                &self.llm_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("llm_model", &self.llm_model)
            .field("llm_concurrency", &self.llm_concurrency)
            .field("log", &self.log)
            .finish()
    }
}

impl Config {
    /// Read all supported settings once. A `*_FILE` value takes precedence over
    /// its direct environment variable, as is conventional for Docker secrets.
    pub fn from_env() -> Result<Self, String> {
        warn_unknown_variables();

        let bind = value("WAGWAN_BIND")?.unwrap_or_else(|| "0.0.0.0:8080".into());
        let bind = bind
            .parse::<SocketAddr>()
            .map_err(|e| format!("invalid WAGWAN_BIND: {e}"))?;
        let data_dir = PathBuf::from(value("WAGWAN_DATA_DIR")?.unwrap_or_else(|| "/data".into()));
        let tz = value("WAGWAN_TZ")?.unwrap_or_else(|| "UTC".into());
        if time_tz::timezones::get_by_name(&tz).is_none() {
            return Err(format!("WAGWAN_TZ is not an IANA time zone: {tz}"));
        }

        let retention_days = number("WAGWAN_RETENTION_DAYS", 90u64)?;
        if retention_days == 0 {
            return Err("WAGWAN_RETENTION_DAYS must be greater than zero".into());
        }
        let min_score = number("WAGWAN_MIN_SCORE", 40u8)?;
        if min_score > 99 {
            return Err("WAGWAN_MIN_SCORE must be between 0 and 99".into());
        }
        let fetch_concurrency = number("WAGWAN_FETCH_CONCURRENCY", 16usize)?;
        if fetch_concurrency == 0 {
            return Err("WAGWAN_FETCH_CONCURRENCY must be greater than zero".into());
        }
        let fetch_min_interval = duration(
            "WAGWAN_FETCH_MIN_INTERVAL",
            Duration::from_secs(crate::tuning::FETCH_MIN_INTERVAL_SECS),
        )?;
        let fetch_max_interval = duration(
            "WAGWAN_FETCH_MAX_INTERVAL",
            Duration::from_secs(crate::tuning::FETCH_MAX_INTERVAL_SECS),
        )?;
        if fetch_min_interval.is_zero() {
            return Err("WAGWAN_FETCH_MIN_INTERVAL must be greater than zero".into());
        }
        if fetch_max_interval.is_zero() {
            return Err("WAGWAN_FETCH_MAX_INTERVAL must be greater than zero".into());
        }
        if fetch_min_interval > fetch_max_interval {
            return Err(
                "WAGWAN_FETCH_MIN_INTERVAL must not exceed WAGWAN_FETCH_MAX_INTERVAL".into(),
            );
        }

        let opml_seed = value("WAGWAN_OPML_SEED")?.map(PathBuf::from);
        let topics_seed = value("WAGWAN_TOPICS_SEED")?
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let model_url = value("WAGWAN_MODEL_URL")?.unwrap_or_else(|| DEFAULT_MODEL_URL.into());
        let llm_base_url = value("WAGWAN_LLM_BASE_URL")?;
        let llm_api_key = value("WAGWAN_LLM_API_KEY")?;
        let llm_model = value("WAGWAN_LLM_MODEL")?;
        if llm_base_url.is_some() && llm_model.as_deref().unwrap_or("").trim().is_empty() {
            return Err("WAGWAN_LLM_MODEL is required when WAGWAN_LLM_BASE_URL is set".into());
        }
        let llm_concurrency = number("WAGWAN_LLM_CONCURRENCY", 2usize)?;
        if llm_concurrency == 0 {
            return Err("WAGWAN_LLM_CONCURRENCY must be greater than zero".into());
        }
        let log = value("WAGWAN_LOG")?.unwrap_or_else(|| "info".into());

        Ok(Self {
            bind,
            data_dir,
            tz,
            retention_days,
            min_score,
            fetch_concurrency,
            fetch_min_interval,
            fetch_max_interval,
            opml_seed,
            topics_seed,
            model_url,
            llm_base_url,
            llm_api_key,
            llm_model,
            llm_concurrency,
            log,
        })
    }
}

fn value(name: &str) -> Result<Option<String>, String> {
    let file_name = format!("{name}_FILE");
    match env::var_os(&file_name) {
        Some(path) => {
            let path = PathBuf::from(path);
            let contents = std::fs::read_to_string(&path)
                .map_err(|e| format!("could not read {file_name} {}: {e}", path.display()))?;
            Ok(Some(contents.trim_end_matches(['\r', '\n']).to_owned()))
        }
        None => match env::var(name) {
            Ok(v) => Ok(Some(v)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(e) => Err(format!("{name} is not valid Unicode: {e}")),
        },
    }
}

fn number<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    match value(name)? {
        Some(v) => v.parse().map_err(|_| format!("invalid {name}: {v:?}")),
        None => Ok(default),
    }
}

/// Parse the documented duration syntax: integer seconds, minutes, hours, or days.
fn duration(name: &str, default: Duration) -> Result<Duration, String> {
    let Some(raw) = value(name)? else {
        return Ok(default);
    };
    parse_duration(name, &raw)
}

fn parse_duration(name: &str, raw: &str) -> Result<Duration, String> {
    let trimmed = raw.trim();
    let (digits, unit) = trimmed.split_at(
        trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len()),
    );
    let amount = digits
        .parse::<u64>()
        .map_err(|_| format!("invalid {name}: {raw:?}"))?;
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => {
            return Err(format!(
                "invalid {name}: expected a duration such as 30s, 15m, 6h, or 1d"
            ));
        }
    };
    let seconds = amount
        .checked_mul(multiplier)
        .ok_or_else(|| format!("{name} is too large"))?;
    Ok(Duration::from_secs(seconds))
}

fn warn_unknown_variables() {
    const KNOWN: &[&str] = &[
        "WAGWAN_BIND",
        "WAGWAN_DATA_DIR",
        "WAGWAN_TZ",
        "WAGWAN_RETENTION_DAYS",
        "WAGWAN_MIN_SCORE",
        "WAGWAN_FETCH_CONCURRENCY",
        "WAGWAN_FETCH_MIN_INTERVAL",
        "WAGWAN_FETCH_MAX_INTERVAL",
        "WAGWAN_OPML_SEED",
        "WAGWAN_TOPICS_SEED",
        "WAGWAN_MODEL_URL",
        "WAGWAN_LLM_BASE_URL",
        "WAGWAN_LLM_API_KEY",
        "WAGWAN_LLM_MODEL",
        "WAGWAN_LLM_CONCURRENCY",
        "WAGWAN_LOG",
    ];
    for (key, _) in env::vars_os() {
        let Some(key) = key.to_str() else { continue };
        if key.starts_with("WAGWAN_")
            && !KNOWN.contains(&key)
            && !(key
                .strip_suffix("_FILE")
                .is_some_and(|base| KNOWN.contains(&base)))
        {
            eprintln!("warning: unknown configuration variable {key}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_units_and_invalid_values() {
        assert_eq!(
            parse_duration("TEST", "30s").unwrap(),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_duration("TEST", "15m").unwrap(),
            Duration::from_secs(900)
        );
        assert_eq!(
            parse_duration("TEST", "6h").unwrap(),
            Duration::from_secs(21600)
        );
        assert_eq!(
            parse_duration("TEST", "1d").unwrap(),
            Duration::from_secs(86400)
        );
        assert!(parse_duration("TEST", "1w").is_err());
        assert!(parse_duration("TEST", "-1h").is_err());
        assert!(parse_duration("TEST", "18446744073709551615d").is_err());
    }

    #[test]
    fn debug_redacts_llm_api_key() {
        let config = Config {
            bind: "127.0.0.1:8080".parse().unwrap(),
            data_dir: PathBuf::from("/tmp/wagwan"),
            tz: "UTC".into(),
            retention_days: 90,
            min_score: 40,
            fetch_concurrency: 16,
            fetch_min_interval: Duration::from_secs(900),
            fetch_max_interval: Duration::from_secs(21600),
            opml_seed: None,
            topics_seed: vec![],
            model_url: DEFAULT_MODEL_URL.into(),
            llm_base_url: None,
            llm_api_key: Some("secret-token".into()),
            llm_model: None,
            llm_concurrency: 2,
            log: "info".into(),
        };
        let formatted = format!("{config:?}");
        assert!(formatted.contains("<redacted>"));
        assert!(!formatted.contains("secret-token"));
    }
}
