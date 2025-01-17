mod env;

use crate::error::Error;
use config::Config;
use serde::Deserialize;

use self::env::ENV;

const CONFIG_FILE_PATH: &str = "./config/main";
const CONFIG_FILE_PATH_PREFIX: &str = "./config/";

lazy_static! {
    /// If `AWS_SECRET_NAME` detected in runtime `ENV`, config will be
    /// parsed using AWS Secret.
    /// Otherwise, read config file.
    pub static ref C: KVConfig = {
        if !std::env::var("AWS_SECRET_NAME").unwrap_or_default().is_empty() {
            from_aws_secret().unwrap()
        } else {
            parse().unwrap()
        }
    };
}

#[derive(Clone, Deserialize, Default)]
pub struct KVConfig {
    pub db: ConfigDB,
    pub web: ConfigWeb,
    pub upstream: Upstream,
}

#[derive(Clone, Deserialize, Default)]
pub struct Upstream {
    pub proof_service: ConfigProofService,
    pub aggregation_service: ConfigAggregationService,
    pub sybil_service: ConfigSybilService,
    pub keybase_service: ConfigKeybaseService,
    pub knn3_service: ConfigKnn3Service,
    pub rss3_service: ConfigRss3Service,
    pub the_graph: ConfigUpstreamTheGraph,
    pub ens_reverse: ConfigENSReverse,
    pub dotbit_service: ConfigDotbitService,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigDB {
    pub host: String,
    pub username: String,
    pub password: String,
    pub db: String,
    pub schema_path: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigWeb {
    pub listen: String,
    pub port: u16,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigProofService {
    pub url: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigKeybaseService {
    pub url: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigAggregationService {
    pub url: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigSybilService {
    pub url: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigKnn3Service {
    pub url: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigRss3Service {
    pub url: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigUpstreamTheGraph {
    pub ens: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigENSReverse {
    pub url: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct ConfigDotbitService {
    pub url: String,
}

#[derive(Clone, Deserialize)]
pub enum ConfigCategory {
    File,
    AWSSecret,
}
impl Default for ConfigCategory {
    fn default() -> Self {
        Self::File
    }
}

/// Fetch and parse runtime ENV.
pub fn app_env() -> ENV {
    if cfg!(test) {
        return ENV::Testing;
    }

    std::env::var("RELATION_SERVER_ENV")
        .unwrap_or_else(|_| "development".into())
        .into()
}

/// Parse config from local file or ENV.
pub fn parse() -> Result<KVConfig, Error> {
    let s = Config::builder()
        // Default
        .add_source(config::File::with_name(CONFIG_FILE_PATH).required(false))
        // app-env-based config
        .add_source(
            config::File::with_name(&format!("{}{}.toml", CONFIG_FILE_PATH_PREFIX, app_env()))
                .required(false),
        )
        // runtime-ENV-based config
        .add_source(
            config::Environment::with_prefix("KV")
                .separator("__")
                .ignore_empty(true),
        )
        .build()?;

    s.try_deserialize().map_err(|e| e.into())
}

/// `AWS_SECRET_NAME` and `AWS_SECRET_REGION` is needed.
pub fn from_aws_secret() -> Result<KVConfig, Error> {
    todo!()
}
