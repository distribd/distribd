use std::path::PathBuf;

use anyhow::{Context, Result};
use figment::{
    providers::{Env, Format, Serialized, Yaml},
    Figment,
};
use jwt_simple::prelude::ES256PublicKey;
use platform_dirs::AppDirs;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use x509_parser::prelude::Pem;

use crate::RegistryNodeId;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TlsConfig {
    pub key: String,
    pub chain: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RaftConfig {
    pub address: String,
    pub port: u16,
    pub tls: Option<TlsConfig>,
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            address: "0.0.0.0".to_string(),
            port: 8080,
            tls: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegistryConfig {
    pub address: String,
    pub port: u16,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            address: "0.0.0.0".to_string(),
            port: 8000,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrometheusConfig {
    pub address: String,
    pub port: u16,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            address: "0.0.0.0".to_string(),
            port: 7080,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PublicKey {
    pub path: String,
    pub public_key: ES256PublicKey,
}

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.path)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        let mut p = PathBuf::from(s.clone());
        if p.is_relative() {
            let app_dirs = AppDirs::new(Some("distribd"), true).unwrap();
            let config_dir = app_dirs.config_dir;
            p = config_dir.join(p);
        }
        let pem = std::fs::read_to_string(&p).unwrap();

        let public_key = match ES256PublicKey::from_pem(&pem) {
            Ok(public_key) => public_key,
            Err(_) => {
                let pem = Pem::iter_from_buffer(pem.as_bytes())
                    .next()
                    .unwrap()
                    .unwrap();
                let x509 = pem.parse_x509().expect("X.509: decoding DER failed");
                let raw = &x509.public_key().subject_public_key.data;

                ES256PublicKey::from_bytes(raw).unwrap()
            }
        };

        Ok(PublicKey {
            path: s,
            public_key,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenConfig {
    pub issuer: String,
    pub service: String,
    pub realm: String,
    pub public_key: PublicKey,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WebhookConfig {
    pub url: String,

    #[serde(with = "serde_regex")]
    pub matcher: Regex,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct ScrubberConfig {
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SentryConfig {
    pub endpoint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Configuration {
    pub identifier: String,
    pub raft: RaftConfig,
    pub registry: RegistryConfig,
    pub prometheus: PrometheusConfig,
    pub token_server: Option<TokenConfig>,
    pub storage: String,
    pub webhooks: Vec<WebhookConfig>,
    pub scrubber: ScrubberConfig,
    pub sentry: Option<SentryConfig>,
}

impl Configuration {
    pub fn id(&self) -> Result<RegistryNodeId> {
        let (_, node_id) = self
            .identifier
            .rsplit_once('-')
            .context("Invalid identifier name")?;

        let mut node_id = node_id
            .parse()
            .context("Identifier must end with a number")?;

        node_id += 1;

        Ok(node_id)
    }
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            identifier: "localhost-0".to_string(),
            raft: RaftConfig::default(),
            registry: RegistryConfig::default(),
            prometheus: PrometheusConfig::default(),
            token_server: None,
            storage: "var".to_string(),
            webhooks: vec![],
            scrubber: ScrubberConfig::default(),
            sentry: None,
        }
    }
}

pub fn config(config: Option<PathBuf>) -> Configuration {
    let mut fig = Figment::from(Serialized::defaults(Configuration::default()));

    fig = match config {
        Some(config_path) => fig.merge(Yaml::file(config_path)),
        None => {
            let app_dirs = AppDirs::new(Some("distribd"), true).unwrap();
            let config_dir = app_dirs.config_dir;
            let config_path = config_dir.join("config.yaml");
            match config_path.exists() {
                true => fig.merge(Yaml::file(config_path)),
                false => fig,
            }
        }
    };

    fig.merge(Env::prefixed("DISTRIBD_"))
        .extract()
        .expect("Failed to load config.yaml")
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn entrypoint() {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::env::current_dir()
                .unwrap()
                .join("fixtures/etc")
                .as_os_str(),
        );

        let config = config(None);
        assert_eq!(config.identifier, "localhost-0".to_string());
    }

    #[test]
    fn defaults() {
        let defaults: Configuration = Figment::from(Serialized::defaults(Configuration::default()))
            .extract()
            .unwrap();
        assert_eq!(defaults.raft.address, "0.0.0.0");
    }

    #[test]
    fn token_config() {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::env::current_dir()
                .unwrap()
                .join("fixtures/etc")
                .as_os_str(),
        );

        let data = r#"
        {
            "issuer": "Test Issuer",
            "realm": "testrealm",
            "service": "myservice",
            "public_key": "token.pub"
        }"#;

        let t: TokenConfig = serde_json::from_str(data).unwrap();

        assert_eq!(t.issuer, "Test Issuer");
        assert_eq!(t.realm, "testrealm");
        assert_eq!(t.service, "myservice");
        assert_eq!(t.public_key.path, "token.pub");
        assert_eq!(t.public_key.public_key.to_pem().unwrap(), "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEPEUDSJJ2ThQmq1py0QUp1VHfLxOS\nGjl1uDis2P2rq3YWN96TDWgYbmk4v1Fd3sznlgTnM7cZ22NrrdKvM4TmVg==\n-----END PUBLIC KEY-----\n");
    }

    #[test]
    fn token_confi_cert() {
        std::env::set_var(
            "XDG_CONFIG_HOME",
            std::env::current_dir()
                .unwrap()
                .join("fixtures/etc")
                .as_os_str(),
        );

        let data = r#"
        {
            "issuer": "Test Issuer",
            "realm": "testrealm",
            "service": "myservice",
            "public_key": "token.crt"
        }"#;

        let t: TokenConfig = serde_json::from_str(data).unwrap();

        assert_eq!(t.issuer, "Test Issuer");
        assert_eq!(t.realm, "testrealm");
        assert_eq!(t.service, "myservice");
        assert_eq!(t.public_key.path, "token.crt");
        assert_eq!(t.public_key.public_key.to_pem().unwrap(), "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEPEUDSJJ2ThQmq1py0QUp1VHfLxOS\nGjl1uDis2P2rq3YWN96TDWgYbmk4v1Fd3sznlgTnM7cZ22NrrdKvM4TmVg==\n-----END PUBLIC KEY-----\n");
    }

    #[test]
    fn webhook_config() {
        let data = r#"
        {
            "url": "http://localhost:1234",
            "matcher": "matcher.*"
        }"#;

        let t: WebhookConfig = serde_json::from_str(data).unwrap();

        assert_eq!(t.url, "http://localhost:1234");
        assert!(!t.matcher.is_match("testrealm"));
        assert!(t.matcher.is_match("matcherZ"));
    }
}
