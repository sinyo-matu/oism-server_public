use secrecy::{ExposeSecret, Secret};
use uuid::Uuid;

#[derive(serde::Deserialize)]
pub struct Settings {
    pub database: DatabaseSettings,
    pub application_port: u16,
    pub access_expiration: u32,
    pub refresh_expiration: u32,
    pub signup_secret: Secret<String>,
    pub access_token_secret: Secret<String>,
    pub refresh_token_secret: Secret<String>,
    pub utility: UtilitySetting,
    pub google_service: GoogleServiceSetting,
}

#[derive(serde::Deserialize)]
pub struct UtilitySetting {
    pub port: u16,
    pub host: String,
}

impl UtilitySetting {
    pub fn get_utility_url(&self) -> String {
        format!("http://{}:{}", self.host.clone(), self.port.clone())
    }
}

#[derive(serde::Deserialize)]
pub struct GoogleServiceSetting {
    pub target_user_ex_id: Uuid,
    pub host: String,
    pub port: u16,
    pub task_list_name: String,
}

impl GoogleServiceSetting {
    pub fn get_service_url(&self) -> String {
        format!("http://{}:{}", self.host.clone(), self.port.clone())
    }
}

#[derive(serde::Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    pub password: Secret<String>,
    pub port: u16,
    pub host: String,
    pub database_name: String,
}

pub fn get_configuration() -> Result<Settings, config::ConfigError> {
    let base_path = std::env::current_dir().expect("Failed to determine current directory");
    let configuration_path = base_path.join("configuration");
    let environment: Environment = std::env::var("APP_ENVIRONMENT")
        .unwrap_or_else(|_| "local".into())
        .try_into()
        .expect("failed to parse APP_ENVIRONMENT");
    let env_config = config::Environment::with_prefix("oism").separator("__");
    let settings = config::Config::builder()
        .add_source(config::File::from(configuration_path.join("base")).required(true))
        .add_source(
            config::File::from(configuration_path.join(environment.as_str())).required(true),
        )
        .add_source(env_config)
        .build()?;
    settings.try_deserialize::<Settings>()
}

impl DatabaseSettings {
    pub fn connection_string_local(&self) -> String {
        let DatabaseSettings {
            username: _,
            password: _,
            port,
            host,
            database_name: _,
        } = self;
        format!("mongodb://{host}:{port}")
    }
    pub fn connection_string_cloud(&self) -> Secret<String> {
        let DatabaseSettings {
            username,
            password,
            port: _,
            host,
            database_name: _,
        } = self;
        Secret::new(format!(
            "mongodb+srv://{username}:{password}@{host}",
            password = password.expose_secret()
        ))
    }
}

pub enum Environment {
    Local,
    Preview,
    Production,
}
impl Environment {
    fn as_str(&self) -> &str {
        match self {
            Environment::Local => "local",
            Environment::Preview => "preview",
            Environment::Production => "production",
        }
    }
}

impl TryFrom<String> for Environment {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "preview" => Ok(Self::Preview),
            "production" => Ok(Self::Production),
            other => Err(format!(
                "{} is not supported environment. use either `local`,`preview` or `production` instead.",
                other
            )),
        }
    }
}
