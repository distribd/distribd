use figment::{
    providers::{Env, Format, Serialized, Yaml},
    Figment,
};
use platform_dirs::AppDirs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct RaftConfig {
    pub address: String,
    pub port: u32,
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8080,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct RegistryConfig {
    pub address: String,
    pub port: u32,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8000,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PrometheusConfig {
    pub address: String,
    pub port: u32,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 7080,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TokenConfig {
    pub issuer: String,
    pub service: String,
    pub realm: String,
    pub public_key: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PeerConfig {
    pub name: String,
    pub raft: RaftConfig,
    pub registry: RegistryConfig,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct MintConfig {
    pub realm: String,
    pub service: String,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Configuration {
    pub raft: RaftConfig,
    pub registry: RegistryConfig,
    pub prometheus: PrometheusConfig,
    pub token_server: Option<TokenConfig>,
    pub mirroring: Option<MintConfig>,
    pub storage: String,
    pub peers: Vec<PeerConfig>,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            raft: RaftConfig::default(),
            registry: RegistryConfig::default(),
            prometheus: PrometheusConfig::default(),
            token_server: None,
            mirroring: None,
            storage: "var".to_string(),
            peers: vec![],
        }
    }
}

pub fn config() -> Configuration {
    let mut config = Figment::from(Serialized::defaults(Configuration::default()));

    let app_dirs = AppDirs::new(Some("distribd"), false).unwrap();
    let config_dir = app_dirs.config_dir;
    let config_path = config_dir.join("config.yaml");

    if config_path.exists() {
        config = config.merge(Yaml::file(config_path));
    }

    config
        .merge(Env::prefixed("DISTRIBD_"))
        .extract()
        .expect("Failed to load config.yaml")
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn defaults() {
        let defaults: Configuration = Figment::from(Serialized::defaults(Configuration::default()))
            .extract()
            .unwrap();
        assert_eq!(defaults.raft.address, "127.0.0.1");
        assert!(defaults.peers.is_empty());
    }
}
