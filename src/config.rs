use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;

pub const DEFAULT_EXTENSION_PORT: u16 = 18765;
pub const CLI_API_PORT: u16 = 18767;
pub const API_TOKEN_HEADER: &str = "x-agent-browser-token";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub extension_port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            extension_port: DEFAULT_EXTENSION_PORT,
        }
    }
}

pub fn load_or_create() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        let config = AppConfig::default();
        save(&config)?;
        return Ok(config);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    if content.trim().is_empty() {
        let config = AppConfig::default();
        save(&config)?;
        return Ok(config);
    }

    let config: AppConfig = serde_json::from_str(&content)
        .with_context(|| format!("解析配置文件失败: {}", path.display()))?;
    validate_port(config.extension_port)?;
    Ok(config)
}

pub fn load_existing() -> Result<AppConfig> {
    let path = config_path()?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    if content.trim().is_empty() {
        return Err(anyhow!("配置文件为空: {}", path.display()));
    }
    let config: AppConfig = serde_json::from_str(&content)
        .with_context(|| format!("解析配置文件失败: {}", path.display()))?;
    validate_port(config.extension_port)?;
    Ok(config)
}

pub fn save(config: &AppConfig) -> Result<()> {
    validate_port(config.extension_port)?;
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(config)? + "\n";
    fs::write(&path, content).with_context(|| format!("写入配置文件失败: {}", path.display()))?;
    Ok(())
}

pub fn set_extension_port(port: u16) -> Result<AppConfig> {
    validate_port(port)?;
    let mut config = load_or_create()?;
    config.extension_port = port;
    save(&config)?;
    Ok(config)
}

pub fn config_path() -> Result<PathBuf> {
    Ok(user_config_dir()?.join("config.json"))
}

pub fn user_config_dir() -> Result<PathBuf> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| anyhow!("无法定位用户主目录，不能创建 agent-browser-cli 配置文件"))?;
    Ok(PathBuf::from(home).join(".agent-browser-cli"))
}

pub fn log_dir() -> Result<PathBuf> {
    Ok(user_config_dir()?.join("logs"))
}

pub fn daemon_log_path() -> Result<PathBuf> {
    Ok(log_dir()?.join("daemon.log"))
}

pub fn api_token_path() -> Result<PathBuf> {
    Ok(user_config_dir()?.join("api-token"))
}

pub fn load_api_token() -> Result<Option<String>> {
    let path = api_token_path()?;
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| format!("读取 API token 失败: {}", path.display()))
        }
    };
    let token = content.trim();
    if token.is_empty() {
        return Err(anyhow!("API token 文件为空: {}", path.display()));
    }
    Ok(Some(token.to_string()))
}

pub fn save_api_token(token: &str) -> Result<()> {
    if token.is_empty() {
        return Err(anyhow!("API token 不能为空"));
    }
    let path = api_token_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("API token 路径缺少父目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
    let temp_path = parent.join(format!(".api-token-{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options
            .open(&temp_path)
            .with_context(|| format!("创建 API token 临时文件失败: {}", temp_path.display()))?;
        file.write_all(token.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&temp_path, &path)
            .with_context(|| format!("保存 API token 失败: {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

pub fn ensure_log_file() -> Result<PathBuf> {
    let path = daemon_log_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建日志目录失败: {}", parent.display()))?;
    }
    if !path.exists() {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("创建日志文件失败: {}", path.display()))?;
    }
    Ok(path)
}

fn validate_port(port: u16) -> Result<()> {
    if port == 0 {
        return Err(anyhow!("extension_port 必须是 1-65535"));
    }
    if port == CLI_API_PORT {
        return Err(anyhow!(
            "18767 是 agent-browser-cli API 端口，插件端口请换一个"
        ));
    }
    Ok(())
}
