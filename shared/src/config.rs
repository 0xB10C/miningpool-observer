use std::convert::TryFrom;
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
const VALID_POOL_IDENTIFICATION_NETWORKS: &str =
    "\"bitcoin\", \"testnet\", \"signet\", \"regtest\"";

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
    // The option used to be spelled "pool_identificatoin". The alias keeps
    // configuration files using the misspelled name working.
    #[serde(alias = "pool_identificatoin")]
    pool_identification: Option<PoolIdentificationTomlConfig>,
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

impl TryFrom<PoolIdentificationTomlConfig> for PoolIdentificationConfig {
    type Error = ConfigError;

    fn try_from(toml: PoolIdentificationTomlConfig) -> Result<Self, Self::Error> {
        let network_name = toml
            .network
            .unwrap_or(DEFAULT_POOL_IDENTIFICATOIN_NETWORK.to_string());
        let network = Network::from_str(&network_name.to_lowercase())
            .map_err(|_| ConfigError::InvalidNetwork(network_name))?;
        Ok(PoolIdentificationConfig {
            dataset_url: toml
                .dataset_url
                .unwrap_or(DEFAULT_POOL_IDENTIFICATOIN_DATASET_URL.to_string()),
            network,
        })
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
    parse_daemon_config(&config_string)
}

pub fn parse_daemon_config(toml_str: &str) -> Result<DaemonConfig, ConfigError> {
    let config: DaemonTomlConfig = toml::from_str(toml_str)?;

    let rpc_auth: Auth = if let Some(rpc_cookie_file) = config.rpc_cookie_file {
        if !rpc_cookie_file.exists() {
            return Err(ConfigError::CookieFileDoesNotExist);
        }
        Auth::CookieFile(rpc_cookie_file)
    } else if config.rpc_user.is_some() && config.rpc_password.is_some() {
        Auth::UserPass(config.rpc_user.unwrap(), config.rpc_password.unwrap())
    } else {
        return Err(ConfigError::NoRpcAuth);
    };

    let log_level = LevelFilter::from_str(&config.log_level)?;

    Ok(DaemonConfig {
        rpc_url: format!("http://{}:{}", config.rpc_host, config.rpc_port),
        rpc_auth,
        database_url: config.database_url,
        log_level,
        retag_transactions: config.retag_transactions,
        prometheus: config.prometheus,
        sanctioned_addresses_url: config
            .sanctioned_addresses_url
            .unwrap_or(DEFAULT_SANCTIONED_ADDRESSES_URL.to_string()),
        pool_identification: PoolIdentificationConfig::try_from(
            config.pool_identification.unwrap_or_default(),
        )?,
    })
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
    parse_web_config(&config_string)
}

pub fn parse_web_config(toml_str: &str) -> Result<WebConfig, ConfigError> {
    let config: WebTomlConfig = toml::from_str(toml_str)?;
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
    InvalidNetwork(String),
    TomlError(toml::de::Error),
    ReadError(io::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ConfigError::CookieFileDoesNotExist => write!(f, "the .cookie file path set via rpc_cookie_file does not exist"),
            ConfigError::NoRpcAuth => write!(f, "please specify a Bitcoin Core RPC .cookie file (option: 'rpc_cookie_file') or a rpc_user and rpc_password"),
            ConfigError::InvalidLogLevel(e) => write!(f, "the specified log level is invalid: {}", e),
            ConfigError::InvalidNetwork(network) => write!(f, "the pool identification network '{}' is invalid: valid networks are {}", network, VALID_POOL_IDENTIFICATION_NETWORKS),
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
            ConfigError::InvalidNetwork(_) => None,
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
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const MINIMAL_DAEMON_CONFIG: &str = r#"
        rpc_host = "127.0.0.1"
        rpc_port = 8332
        rpc_user = "user"
        rpc_password = "password"
        database_url = "postgres://user:password@localhost:5432/db"
        log_level = "info"
        retag_transactions = false

        [prometheus]
        enable = false
        address = "127.0.0.1:23842"
    "#;

    const MINIMAL_WEB_CONFIG: &str = r#"
        address = "127.0.0.1:8080"
        database_url = "postgres://user:password@localhost:5432/db"
        log_level = "info"
        www_dir_path = "www"

        [site]
        title = "title"
        footer = "footer"
        base_url = "https://example.com"
    "#;

    /// A file in the temporary directory that is removed again when it goes
    /// out of scope. Used as a stand-in for a Bitcoin Core .cookie file.
    struct TempFile {
        path: PathBuf,
    }

    impl TempFile {
        fn new() -> TempFile {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = env::temp_dir().join(format!(
                "miningpool-observer-test-cookie-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let mut file =
                fs::File::create(&path).expect("we should be able to create a temp file");
            file.write_all(b"__cookie__:password")
                .expect("we should be able to write to the temp file");
            TempFile { path }
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    /// Prepends a top-level key to the minimal daemon config.
    fn daemon_config_with(extra: &str) -> String {
        format!("{}\n{}", extra, MINIMAL_DAEMON_CONFIG)
    }

    /// Appends a section to the minimal daemon config.
    fn daemon_config_with_section(section: &str) -> String {
        format!("{}\n{}", MINIMAL_DAEMON_CONFIG, section)
    }

    #[test]
    fn minimal_daemon_config_parses() {
        let config = parse_daemon_config(MINIMAL_DAEMON_CONFIG)
            .expect("the minimal daemon config should parse");
        assert_eq!(config.rpc_url, "http://127.0.0.1:8332");
        assert_eq!(
            config.database_url,
            "postgres://user:password@localhost:5432/db"
        );
        assert_eq!(config.log_level, LevelFilter::Info);
        assert!(!config.retag_transactions);
        assert!(!config.prometheus.enable);
        assert_eq!(config.prometheus.address, "127.0.0.1:23842");
        match config.rpc_auth {
            Auth::UserPass(user, password) => {
                assert_eq!(user, "user");
                assert_eq!(password, "password");
            }
            other => panic!("expected a user and password auth, got {:?}", other),
        }
    }

    #[test]
    fn daemon_config_rpc_url_uses_the_configured_host_and_port() {
        let config = parse_daemon_config(
            r#"
            rpc_host = "node.example.com"
            rpc_port = 18443
            rpc_user = "user"
            rpc_password = "password"
            database_url = "postgres://localhost/db"
            log_level = "info"
            retag_transactions = true

            [prometheus]
            enable = true
            address = "127.0.0.1:1234"
        "#,
        )
        .expect("the daemon config should parse");
        assert_eq!(config.rpc_url, "http://node.example.com:18443");
        assert!(config.retag_transactions);
        assert!(config.prometheus.enable);
    }

    #[test]
    fn daemon_config_applies_the_defaults() {
        let config = parse_daemon_config(MINIMAL_DAEMON_CONFIG)
            .expect("the minimal daemon config should parse");
        assert_eq!(
            config.sanctioned_addresses_url,
            DEFAULT_SANCTIONED_ADDRESSES_URL
        );
        assert_eq!(
            config.pool_identification.dataset_url,
            DEFAULT_POOL_IDENTIFICATOIN_DATASET_URL
        );
        assert_eq!(config.pool_identification.network, Network::Bitcoin);
    }

    #[test]
    fn daemon_config_sanctioned_addresses_url_can_be_set() {
        let config = parse_daemon_config(&daemon_config_with(
            r#"sanctioned_addresses_url = "https://example.com/list.txt""#,
        ))
        .expect("the daemon config should parse");
        assert_eq!(
            config.sanctioned_addresses_url,
            "https://example.com/list.txt"
        );
    }

    #[test]
    fn daemon_config_without_any_rpc_auth_is_rejected() {
        let config = r#"
            rpc_host = "127.0.0.1"
            rpc_port = 8332
            database_url = "postgres://localhost/db"
            log_level = "info"
            retag_transactions = false

            [prometheus]
            enable = false
            address = "127.0.0.1:23842"
        "#;
        assert!(matches!(
            parse_daemon_config(config),
            Err(ConfigError::NoRpcAuth)
        ));
    }

    #[test]
    fn daemon_config_with_a_rpc_user_but_no_password_is_rejected() {
        let config = r#"
            rpc_host = "127.0.0.1"
            rpc_port = 8332
            rpc_user = "user"
            database_url = "postgres://localhost/db"
            log_level = "info"
            retag_transactions = false

            [prometheus]
            enable = false
            address = "127.0.0.1:23842"
        "#;
        assert!(matches!(
            parse_daemon_config(config),
            Err(ConfigError::NoRpcAuth)
        ));
    }

    #[test]
    fn daemon_config_with_a_rpc_password_but_no_user_is_rejected() {
        let config = r#"
            rpc_host = "127.0.0.1"
            rpc_port = 8332
            rpc_password = "password"
            database_url = "postgres://localhost/db"
            log_level = "info"
            retag_transactions = false

            [prometheus]
            enable = false
            address = "127.0.0.1:23842"
        "#;
        assert!(matches!(
            parse_daemon_config(config),
            Err(ConfigError::NoRpcAuth)
        ));
    }

    #[test]
    fn daemon_config_with_a_missing_cookie_file_is_rejected() {
        let config = daemon_config_with(r#"rpc_cookie_file = "/this/path/does/not/exist/.cookie""#);
        assert!(matches!(
            parse_daemon_config(&config),
            Err(ConfigError::CookieFileDoesNotExist)
        ));
    }

    #[test]
    fn daemon_config_with_an_existing_cookie_file_uses_it() {
        let cookie_file = TempFile::new();
        let config = daemon_config_with(&format!(
            "rpc_cookie_file = {:?}",
            cookie_file.path.display().to_string()
        ));
        let config = parse_daemon_config(&config).expect("the daemon config should parse");
        match config.rpc_auth {
            // The cookie file is used even though the minimal config also sets
            // a rpc_user and a rpc_password.
            Auth::CookieFile(path) => assert_eq!(path, cookie_file.path),
            other => panic!("expected a cookie file auth, got {:?}", other),
        }
    }

    #[test]
    fn daemon_config_log_level_is_case_insensitive() {
        let config = parse_daemon_config(
            r#"
            rpc_host = "127.0.0.1"
            rpc_port = 8332
            rpc_user = "user"
            rpc_password = "password"
            database_url = "postgres://localhost/db"
            log_level = "INFO"
            retag_transactions = false

            [prometheus]
            enable = false
            address = "127.0.0.1:23842"
        "#,
        )
        .expect("the daemon config should parse");
        assert_eq!(config.log_level, LevelFilter::Info);
    }

    #[test]
    fn daemon_config_with_an_invalid_log_level_is_rejected() {
        let config = r#"
            rpc_host = "127.0.0.1"
            rpc_port = 8332
            rpc_user = "user"
            rpc_password = "password"
            database_url = "postgres://localhost/db"
            log_level = "shout"
            retag_transactions = false

            [prometheus]
            enable = false
            address = "127.0.0.1:23842"
        "#;
        assert!(matches!(
            parse_daemon_config(config),
            Err(ConfigError::InvalidLogLevel(_))
        ));
    }

    #[test]
    fn invalid_toml_is_rejected() {
        assert!(matches!(
            parse_daemon_config("this is not TOML"),
            Err(ConfigError::TomlError(_))
        ));
        assert!(matches!(
            parse_web_config("this is not TOML"),
            Err(ConfigError::TomlError(_))
        ));
    }

    #[test]
    fn a_missing_required_field_is_rejected() {
        // the rpc_host is missing
        let config = r#"
            rpc_port = 8332
            rpc_user = "user"
            rpc_password = "password"
            database_url = "postgres://localhost/db"
            log_level = "info"
            retag_transactions = false

            [prometheus]
            enable = false
            address = "127.0.0.1:23842"
        "#;
        assert!(matches!(
            parse_daemon_config(config),
            Err(ConfigError::TomlError(_))
        ));
    }

    #[test]
    fn pool_identification_network_is_case_insensitive() {
        let config = parse_daemon_config(&daemon_config_with_section(
            "[pool_identification]\nnetwork = \"Signet\"\n",
        ))
        .expect("the daemon config should parse");
        assert_eq!(config.pool_identification.network, Network::Signet);
    }

    #[test]
    fn pool_identification_section_is_read() {
        let config = parse_daemon_config(&daemon_config_with_section(
            "[pool_identification]\nnetwork = \"regtest\"\ndataset_url = \"https://example.com/pools.json\"\n",
        ))
        .expect("the daemon config should parse");
        assert_eq!(config.pool_identification.network, Network::Regtest);
        assert_eq!(
            config.pool_identification.dataset_url,
            "https://example.com/pools.json"
        );
    }

    #[test]
    fn the_misspelled_pool_identification_section_still_works() {
        let config = parse_daemon_config(&daemon_config_with_section(
            "[pool_identificatoin]\nnetwork = \"testnet\"\n",
        ))
        .expect("the daemon config should parse");
        assert_eq!(config.pool_identification.network, Network::Testnet);
    }

    #[test]
    fn pool_identification_section_falls_back_to_the_defaults() {
        let config = parse_daemon_config(&daemon_config_with_section("[pool_identification]\n"))
            .expect("the daemon config should parse");
        assert_eq!(config.pool_identification.network, Network::Bitcoin);
        assert_eq!(
            config.pool_identification.dataset_url,
            DEFAULT_POOL_IDENTIFICATOIN_DATASET_URL
        );
    }

    #[test]
    fn an_unknown_pool_identification_network_is_rejected() {
        let config = daemon_config_with_section("[pool_identification]\nnetwork = \"mainnet\"\n");
        match parse_daemon_config(&config) {
            Err(ConfigError::InvalidNetwork(network)) => assert_eq!(network, "mainnet"),
            other => panic!("expected an InvalidNetwork error, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn minimal_web_config_parses() {
        let config =
            parse_web_config(MINIMAL_WEB_CONFIG).expect("the minimal web config should parse");
        assert_eq!(config.address, "127.0.0.1:8080");
        assert_eq!(
            config.database_url,
            "postgres://user:password@localhost:5432/db"
        );
        assert_eq!(config.log_level, LevelFilter::Info);
        assert_eq!(config.www_dir_path, "www");
        assert_eq!(config.site.title, "title");
        assert_eq!(config.site.footer, "footer");
        assert_eq!(config.site.base_url, "https://example.com");
        // debug_pages is optional and off by default
        assert!(!config.debug_pages);
    }

    #[test]
    fn web_config_debug_pages_can_be_enabled() {
        let config = parse_web_config(&format!("debug_pages = true\n{}", MINIMAL_WEB_CONFIG))
            .expect("the web config should parse");
        assert!(config.debug_pages);
        let config = parse_web_config(&format!("debug_pages = false\n{}", MINIMAL_WEB_CONFIG))
            .expect("the web config should parse");
        assert!(!config.debug_pages);
    }

    #[test]
    fn web_config_with_an_invalid_log_level_is_rejected() {
        let config = r#"
            address = "127.0.0.1:8080"
            database_url = "postgres://localhost/db"
            log_level = "shout"
            www_dir_path = "www"

            [site]
            title = "title"
            footer = "footer"
            base_url = "https://example.com"
        "#;
        assert!(matches!(
            parse_web_config(config),
            Err(ConfigError::InvalidLogLevel(_))
        ));
    }

    #[test]
    fn web_config_log_level_is_case_insensitive() {
        let config = parse_web_config(
            r#"
            address = "127.0.0.1:8080"
            database_url = "postgres://localhost/db"
            log_level = "DEBUG"
            www_dir_path = "www"

            [site]
            title = "title"
            footer = "footer"
            base_url = "https://example.com"
        "#,
        )
        .expect("the web config should parse");
        assert_eq!(config.log_level, LevelFilter::Debug);
    }

    #[test]
    fn every_config_error_has_a_useful_message() {
        let toml_error = match parse_daemon_config("not TOML") {
            Err(ConfigError::TomlError(e)) => e,
            _ => panic!("expected a TOML error"),
        };
        let log_level_error = LevelFilter::from_str("shout").unwrap_err();
        let errors = vec![
            ConfigError::CookieFileDoesNotExist,
            ConfigError::NoRpcAuth,
            ConfigError::InvalidLogLevel(log_level_error),
            ConfigError::InvalidNetwork("mainnet".to_string()),
            ConfigError::TomlError(toml_error),
            ConfigError::ReadError(io::Error::new(io::ErrorKind::NotFound, "no such file")),
        ];
        for error in errors.iter() {
            let message = error.to_string();
            assert!(!message.is_empty(), "{:?} has an empty message", error);
            // the message should be a sentence and not just the variant name
            assert!(message.len() > 20, "{:?} has a very short message", error);
        }
        assert!(errors[0].to_string().contains("rpc_cookie_file"));
        assert!(errors[1].to_string().contains("rpc_user"));
        assert!(errors[2].to_string().contains("log level"));
        let invalid_network = errors[3].to_string();
        assert!(invalid_network.contains("mainnet"));
        for valid in ["bitcoin", "testnet", "signet", "regtest"].iter() {
            assert!(
                invalid_network.contains(valid),
                "the InvalidNetwork message should list '{}'",
                valid
            );
        }
        assert!(errors[4].to_string().contains("TOML"));
        assert!(errors[5].to_string().contains("no such file"));
    }

    #[test]
    fn config_errors_expose_their_source() {
        use std::error::Error;

        let log_level_error = LevelFilter::from_str("shout").unwrap_err();
        assert!(ConfigError::InvalidLogLevel(log_level_error)
            .source()
            .is_some());
        assert!(
            ConfigError::ReadError(io::Error::new(io::ErrorKind::NotFound, "no such file"))
                .source()
                .is_some()
        );
        assert!(ConfigError::NoRpcAuth.source().is_none());
        assert!(ConfigError::CookieFileDoesNotExist.source().is_none());
        assert!(ConfigError::InvalidNetwork("mainnet".to_string())
            .source()
            .is_none());
    }

    #[test]
    fn load_example_config() {
        use crate::config;
        use std::env;

        // Note: env::set_var is process global. This should stay the only test
        // using the CONFIG_FILE environment variable.
        const EXAMPLE_DAEMON_CONFIG: &str = "../daemon-config.toml.example";
        env::set_var(config::ENVVAR_CONFIG_FILE, EXAMPLE_DAEMON_CONFIG);
        let cfg = config::load_daemon_config().unwrap_or_else(|e| {
            panic!(
                "We should be able to load the deamon config file '{}': {}",
                EXAMPLE_DAEMON_CONFIG, e
            )
        });
        assert_eq!(cfg.pool_identification.network, Network::Bitcoin);

        const EXAMPLE_WEB_CONFIG: &str = "../web-config.toml.example";
        env::set_var(config::ENVVAR_CONFIG_FILE, EXAMPLE_WEB_CONFIG);
        let _cfg = config::load_web_config().unwrap_or_else(|e| {
            panic!(
                "We should be able to load the web config file '{}': {}",
                EXAMPLE_WEB_CONFIG, e
            )
        });
    }
}
