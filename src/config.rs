use std::{collections::HashMap, env, fs::File, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Config {
    pub web: WebConfig,
    pub inputs: HashMap<String, InputConfig>,
    pub jack: JackConfig,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct WebConfig {
    pub host: String,
    pub port: u16,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            host: "::1".into(),
            port: 5005,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct InputConfig {
    pub name: String,
    #[serde(default = "default_channels")]
    pub channels: usize,
}

fn default_channels() -> usize { 2 }

#[derive(Clone, Deserialize, Serialize)]
pub struct JackConfig {
    pub client_name: String,
}

impl Default for JackConfig {
    fn default() -> Self {
        Self {
            client_name: "loudness_meter".into(),
        }
    }
}

pub(super) fn load() -> Config {
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "config.json".to_owned());
    let path = Path::new(&config_path);
    if path.exists() {
        let mut file = File::open(path).expect("failed to open config");
        serde_json::from_reader(&mut file).expect("failed to parse config")
    } else {
        let config = Config::default();

        let mut file = File::create(path).expect("failed to create config");
        serde_json::to_writer_pretty(&mut file, &config).expect("failed to write config");

        config
    }
}
