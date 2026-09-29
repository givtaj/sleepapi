use std::{env, io, net::SocketAddr, str::FromStr};

use axum::http::{HeaderValue, Uri};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub max_duration_ms: u64,
    pub max_in_flight: usize,
    pub shutdown_grace_ms: u64,
    pub cors_allowed_origins: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 3000)),
            max_duration_ms: 30_000,
            max_in_flight: 64,
            shutdown_grace_ms: 5_000,
            cors_allowed_origins: Vec::new(),
        }
    }
}

impl Config {
    pub fn from_env() -> io::Result<Self> {
        let defaults = Self::default();
        let config = Self {
            bind_addr: read_env("SLEEPAPI_ADDR", defaults.bind_addr)?,
            max_duration_ms: read_env("SLEEPAPI_MAX_DURATION_MS", defaults.max_duration_ms)?,
            max_in_flight: read_env("SLEEPAPI_MAX_IN_FLIGHT", defaults.max_in_flight)?,
            shutdown_grace_ms: read_env("SLEEPAPI_SHUTDOWN_GRACE_MS", defaults.shutdown_grace_ms)?,
            cors_allowed_origins: read_env("SLEEPAPI_CORS_ORIGINS", String::new())?
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_owned)
                .collect(),
        };
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        if !(1..=3_600_000).contains(&self.max_duration_ms) {
            return Err(invalid("SLEEPAPI_MAX_DURATION_MS must be in 1..=3600000"));
        }
        if !(1..=1024).contains(&self.max_in_flight) {
            return Err(invalid("SLEEPAPI_MAX_IN_FLIGHT must be in 1..=1024"));
        }
        if !(1..=60_000).contains(&self.shutdown_grace_ms) {
            return Err(invalid("SLEEPAPI_SHUTDOWN_GRACE_MS must be in 1..=60000"));
        }
        self.cors_origins()?;
        Ok(())
    }

    pub(crate) fn cors_origins(&self) -> io::Result<Vec<HeaderValue>> {
        self.cors_allowed_origins.iter().map(|origin| {
            let error = || invalid("SLEEPAPI_CORS_ORIGINS requires exact http(s) origins such as http://localhost:5173, without paths, credentials, or wildcards");
            let uri: Uri = origin.parse().map_err(|_| error())?;
            let scheme = uri.scheme_str().ok_or_else(error)?;
            let authority = uri.authority().ok_or_else(error)?;
            let port = if authority.as_str().starts_with('[') {
                authority.as_str().split_once(']').and_then(|(_, suffix)| suffix.strip_prefix(':'))
            } else {
                authority.as_str().split_once(':').map(|(_, port)| port)
            };
            if !matches!(scheme, "http" | "https")
                || authority.as_str().contains(['@', '*'])
                || uri.host().is_none_or(str::is_empty)
                || port.is_some_and(|port| port.parse::<u16>().is_err())
                || origin != &format!("{scheme}://{authority}")
            {
                return Err(error());
            }
            origin.parse().map_err(|_| error())
        }).collect()
    }
}

fn read_env<T: FromStr>(name: &str, default: T) -> io::Result<T> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| invalid(format!("invalid value for {name}"))),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(invalid(format!("{name} must be valid Unicode"))),
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_accepts_only_exact_http_origins() {
        for origin in [
            "http://localhost:5173",
            "https://example.com",
            "http://[::1]:5173",
        ] {
            assert!(
                Config {
                    cors_allowed_origins: vec![origin.into()],
                    ..Config::default()
                }
                .validate()
                .is_ok(),
                "{origin}"
            );
        }
        for origin in [
            "*",
            "null",
            "localhost:5173",
            "ftp://example.com",
            "https://example.com/",
            "https://example.com/path",
            "https://example.com?x=1",
            "https://example.com#x",
            "https://user@example.com",
            "https://*.example.com",
            "http://localhost:abc",
            "http://localhost:99999",
            "http://localhost:",
            "http://[::1]:abc",
        ] {
            assert!(
                Config {
                    cors_allowed_origins: vec![origin.into()],
                    ..Config::default()
                }
                .validate()
                .is_err(),
                "{origin}"
            );
        }
    }

    #[test]
    fn shutdown_grace_rejects_zero_and_excessive_values() {
        for shutdown_grace_ms in [0, 60_001] {
            assert!(
                Config {
                    shutdown_grace_ms,
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
    }
}
