use std::path::PathBuf;
use std::str::FromStr;
use std::{env, error, fmt, fs, io};

use bitcoincore_rpc::bitcoin::Network;
use bitcoincore_rpc::Auth;
use log::LevelFilter;
use serde::{Deserialize, Serialize};

const ENVVAR_CONFIG_FILE: &str = "CONFIG_FILE";
const DEFAULT_DAEMON_CONFIG: &str = "daemon-config.toml";
const DEFAULT_WEB_CONFIG: &str = "web-config.toml";
const DEFAULT_SANCTIONED_ADDRESSES_URL: &str = "https://raw.githubusercontent.com/0xB10C/ofac-sanctioned-digital-currency-addresses/lists/sanctioned_addresses_XBT.txt";
const DEFAULT_POOL_IDENTIFICATOIN_DATASET_URL: &str =
    "https://raw.githubusercontent.com/bitcoin-data/mining-pools/generated/pool-list.json";
const DEFAULT_POOL_IDENTIFICATOIN_NETWORK: Network = Network::Bitcoin;

#[derive(Deserialize)]
struct DaemonTomlConfig {
    rpc_host: String,
    rpc_port: u16,
    rpc_cookie_file: Option<PathBuf>,
    rpc_user: Option<String>,
    rpc_password: Option<String>,
    database_url: String,
    log_level: String,
    retag_transactions: bool,
    prometheus: PrometheusConfig,
    sanctioned_addresses_url: Option<String>,
    pool_identificatoin: Option<PoolIdentificationTomlConfig>,
}

#[derive(Serialize, Deserialize)]
pub struct PrometheusConfig {
    pub enable: bool,
    pub address: String,
}

#[derive(Serialize, Deserialize)]
pub struct PoolIdentificationTomlConfig {
    pub dataset_url: Option<String>,
    pub network: Option<String>,
}

impl Default for PoolIdentificationTomlConfig {
    fn default() -> Self {
        PoolIdentificationTomlConfig {
            dataset_url: Some(DEFAULT_POOL_IDENTIFICATOIN_DATASET_URL.to_string()),
            network: Some(DEFAULT_POOL_IDENTIFICATOIN_NETWORK.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct PoolIdentificationConfig {
    pub dataset_url: String,
    pub network: Network,
}

impl From<PoolIdentificationTomlConfig> for PoolIdentificationConfig {
    fn from(toml: PoolIdentificationTomlConfig) -> Self {
        PoolIdentificationConfig {
            dataset_url: toml
                .dataset_url
                .unwrap_or(DEFAULT_POOL_IDENTIFICATOIN_DATASET_URL.to_string()),
            network: Network::from_str(
                &toml
                    .network
                    .unwrap_or(DEFAULT_POOL_IDENTIFICATOIN_NETWORK.to_string())
                    .to_lowercase(),
            )
            .expect("invalid pool identification network"),
        }
    }
}

pub struct DaemonConfig {
    pub rpc_url: String,
    pub rpc_auth: Auth,
    pub database_url: String,
    pub log_level: LevelFilter,
    pub retag_transactions: bool,
    pub prometheus: PrometheusConfig,
    pub sanctioned_addresses_url: String,
    pub pool_identification: PoolIdentificationConfig,
}

pub fn load_daemon_config() -> Result<DaemonConfig, ConfigError> {
    let config_file_path =
        env::var(ENVVAR_CONFIG_FILE).unwrap_or_else(|_| DEFAULT_DAEMON_CONFIG.to_string());
    println!("Reading configuration file from {}.", config_file_path);
    let config_string = fs::read_to_string(config_file_path)?;
    let config: DaemonTomlConfig = toml::from_str(&config_string)?;

    let rpc_auth: Auth;
    if config.rpc_cookie_file.is_some() {
        let rpc_cookie_file = config.rpc_cookie_file.unwrap();

        if !rpc_cookie_file.exists() {
            return Err(ConfigError::CookieFileDoesNotExist);
        }

        rpc_auth = Auth::CookieFile(rpc_cookie_file);
    } else if config.rpc_user.is_some() && config.rpc_password.is_some() {
        rpc_auth = Auth::UserPass(config.rpc_user.unwrap(), config.rpc_password.unwrap());
    } else {
        return Err(ConfigError::NoRpcAuth);
    }

    let log_level = LevelFilter::from_str(&config.log_level)?;

    return Ok(DaemonConfig {
        rpc_url: format!("http://{}:{}", config.rpc_host, config.rpc_port),
        rpc_auth,
        database_url: config.database_url,
        log_level,
        retag_transactions: config.retag_transactions,
        prometheus: config.prometheus,
        sanctioned_addresses_url: config
            .sanctioned_addresses_url
            .unwrap_or(DEFAULT_SANCTIONED_ADDRESSES_URL.to_string()),
        pool_identification: config.pool_identificatoin.unwrap_or_default().into(),
    });
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WebSiteConfig {
    pub title: String,
    pub footer: String,
    pub base_url: String,
}

#[derive(Clone, Deserialize)]
pub struct WebTomlConfig {
    address: String,
    database_url: String,
    log_level: String,
    debug_pages: Option<bool>,
    www_dir_path: String,
    site: WebSiteConfig,
}

#[derive(Clone)]
pub struct WebConfig {
    pub address: String,
    pub database_url: String,
    pub log_level: LevelFilter,
    pub debug_pages: bool,
    pub www_dir_path: String,
    pub site: WebSiteConfig,
}

pub fn load_web_config() -> Result<WebConfig, ConfigError> {
    let config_string = fs::read_to_string(
        env::var(ENVVAR_CONFIG_FILE).unwrap_or_else(|_| DEFAULT_WEB_CONFIG.to_string()),
    )?;
    let config: WebTomlConfig = toml::from_str(&config_string)?;
    let log_level = LevelFilter::from_str(&config.log_level)?;
    Ok(WebConfig {
        address: config.address,
        database_url: config.database_url,
        log_level,
        debug_pages: config.debug_pages.unwrap_or(false),
        www_dir_path: config.www_dir_path,
        site: config.site,
    })
}

#[derive(Debug)]
pub enum ConfigError {
    CookieFileDoesNotExist,
    NoRpcAuth,
    InvalidLogLevel(log::ParseLevelError),
    TomlError(toml::de::Error),
    ReadError(io::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ConfigError::CookieFileDoesNotExist => write!(f, "the .cookie file path set via rpc_cookie_file does not exist"),
            ConfigError::NoRpcAuth => write!(f, "please specify a Bitcoin Core RPC .cookie file (option: 'rpc_cookie_file') or a rpc_user and rpc_password"),
            ConfigError::InvalidLogLevel(e) => write!(f, "the specified log level is invalid: {}", e),
            ConfigError::TomlError(e) => write!(f, "the TOML in the configuration file could not be parsed: {}", e),
            ConfigError::ReadError(e) => write!(f, "the configuration file could not be read: {}", e),
        }
    }
}

impl error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            ConfigError::NoRpcAuth => None,
            ConfigError::CookieFileDoesNotExist => None,
            ConfigError::TomlError(ref e) => Some(e),
            ConfigError::ReadError(ref e) => Some(e),
            ConfigError::InvalidLogLevel(ref e) => Some(e),
        }
    }
}

impl From<io::Error> for ConfigError {
    fn from(err: io::Error) -> ConfigError {
        ConfigError::ReadError(err)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> ConfigError {
        ConfigError::TomlError(err)
    }
}

impl From<log::ParseLevelError> for ConfigError {
    fn from(err: log::ParseLevelError) -> ConfigError {
        ConfigError::InvalidLogLevel(err)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn load_example_config() {
        use crate::config;
        use std::env;

        const EXAMPLE_DAEMON_CONFIG: &str = "../daemon-config.toml.example";
        env::set_var(config::ENVVAR_CONFIG_FILE, EXAMPLE_DAEMON_CONFIG);
        let _cfg = config::load_daemon_config().expect(&format!(
            "We should be able to load the deamon config file '{}'",
            EXAMPLE_DAEMON_CONFIG
        ));

        const EXAMPLE_WEB_CONFIG: &str = "../web-config.toml.example";
        env::set_var(config::ENVVAR_CONFIG_FILE, EXAMPLE_WEB_CONFIG);
        let _cfg = config::load_web_config().expect(&format!(
            "We should be able to load the web config file '{}'",
            EXAMPLE_DAEMON_CONFIG
        ));
    }
}
