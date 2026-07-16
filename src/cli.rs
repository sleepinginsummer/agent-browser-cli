use crate::{config, server};
use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use fs2::FileExt;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const HOST: &str = "127.0.0.1";
const PORT: u16 = 18767;

#[derive(Debug, Parser)]
#[command(name = "agent-browser-cli")]
pub struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    Tabs(TabsArgs),
    Tabtree(TabtreeArgs),
    Lookup(LookupCommand),
    #[command(name = "profile-label")]
    ProfileLabel(ProfileLabelCommand),
    Scan(ScanArgs),
    Exec(ExecArgs),
    Snapshot(SnapshotArgs),
    Click(TargetCommandArgs),
    Fill(FillCommandArgs),
    #[command(name = "send-keys")]
    SendKeys(SendKeysCommandArgs),
    #[command(name = "mouse-click")]
    MouseClick(TargetCommandArgs),
    Screenshot(ScreenshotArgs),
    #[command(name = "save-pdf")]
    SavePdf(SavePdfArgs),
    Network(NetworkCommand),
    Console(ConsoleCommand),
    Open(OpenArgs),
    Close(CloseArgs),
    Status,
    Logs(LogsArgs),
    Doctor,
    #[command(name = "install-skill")]
    InstallSkill(InstallSkillArgs),
    #[command(name = "set-extension-port")]
    SetExtensionPort(SetExtensionPortArgs),
    Stop,
    Restart,
    Daemon,
}

#[derive(Debug, Args)]
struct TabsArgs {
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Debug, Args)]
struct TabtreeArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    /// 输出完整 URL 和 session_key；默认使用 compact 输出以减少 token 消耗。
    #[arg(long)]
    full: bool,
}

#[derive(Debug, Args)]
struct ScanArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    tabs_only: bool,
    #[arg(long)]
    text_only: bool,
    #[arg(long, default_value_t = 60.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct ExecArgs {
    #[arg(default_value = "")]
    script: String,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    monitor: bool,
    #[arg(long)]
    wait_js: Option<String>,
    #[arg(long, default_value_t = 3.0)]
    wait_timeout: f64,
    #[arg(long, default_value_t = 0.1)]
    wait_interval: f64,
    #[arg(long, default_value_t = 60.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct SnapshotArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long, default_value_t = 0)]
    offset: usize,
    #[arg(long, default_value_t = 200)]
    limit: usize,
    #[arg(long)]
    details: bool,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct TargetCommandArgs {
    target: String,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    monitor: bool,
    #[arg(long)]
    wait_js: Option<String>,
    #[arg(long, default_value_t = 3.0)]
    wait_timeout: f64,
    #[arg(long, default_value_t = 0.1)]
    wait_interval: f64,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct FillCommandArgs {
    target: String,
    value: Option<String>,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    append: bool,
    #[arg(long)]
    clear: bool,
    #[arg(long)]
    monitor: bool,
    #[arg(long)]
    wait_js: Option<String>,
    #[arg(long, default_value_t = 3.0)]
    wait_timeout: f64,
    #[arg(long, default_value_t = 0.1)]
    wait_interval: f64,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct SendKeysCommandArgs {
    keys: String,
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    monitor: bool,
    #[arg(long)]
    wait_js: Option<String>,
    #[arg(long, default_value_t = 3.0)]
    wait_timeout: f64,
    #[arg(long, default_value_t = 0.1)]
    wait_interval: f64,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct ScreenshotArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    selector: Option<String>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value = "png")]
    format: String,
    #[arg(long)]
    quality: Option<u8>,
    #[arg(long)]
    full_page: bool,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct SavePdfArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value = "a4")]
    paper: String,
    #[arg(long)]
    landscape: bool,
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    #[arg(long = "no-print-background")]
    no_print_background: bool,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Subcommand)]
enum NetworkAction {
    Start(TabOnlyArgs),
    List(NetworkListArgs),
    Detail(NetworkDetailArgs),
    Clear(TabOnlyArgs),
    Stop(TabOnlyArgs),
}

#[derive(Debug, Args)]
struct NetworkCommand {
    #[command(subcommand)]
    action: NetworkAction,
}

#[derive(Debug, Subcommand)]
enum ConsoleAction {
    Start(TabOnlyArgs),
    List(ConsoleListArgs),
    Clear(TabOnlyArgs),
    Stop(TabOnlyArgs),
}

#[derive(Debug, Args)]
struct ConsoleCommand {
    #[command(subcommand)]
    action: ConsoleAction,
}

#[derive(Debug, Args)]
struct TabOnlyArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct NetworkListArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    filter: Option<String>,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct NetworkDetailArgs {
    request_id: String,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct ConsoleListArgs {
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    level: Option<String>,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct OpenArgs {
    target: String,
    #[arg(long)]
    background: bool,
    #[arg(long)]
    window: bool,
    #[arg(long)]
    focus: bool,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long = "group-title")]
    group_title: Option<String>,
    #[arg(long, default_value_t = 10.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct CloseArgs {
    #[arg(long)]
    tab: String,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct LogsArgs {
    #[arg(long, default_value_t = 100)]
    tail: usize,
}

#[derive(Debug, Args)]
struct InstallSkillArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Args)]
struct SetExtensionPortArgs {
    port: u16,
}

#[derive(Debug, Args)]
struct LookupCommand {
    #[command(subcommand)]
    target: LookupTarget,
}

#[derive(Debug, Subcommand)]
enum LookupTarget {
    Tab(LookupTabArgs),
    Browser(LookupBrowserArgs),
    Profile(LookupProfileArgs),
}

#[derive(Debug, Args)]
struct LookupTabArgs {
    tab: String,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Debug, Args)]
struct LookupBrowserArgs {
    browser: String,
}

#[derive(Debug, Args)]
struct LookupProfileArgs {
    profile: String,
}

#[derive(Debug, Args)]
struct ProfileLabelCommand {
    #[command(subcommand)]
    action: ProfileLabelAction,
}

#[derive(Debug, Subcommand)]
enum ProfileLabelAction {
    Set(ProfileLabelSetArgs),
    Clear(ProfileLabelClearArgs),
}

#[derive(Debug, Args)]
struct ProfileLabelSetArgs {
    label: String,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

#[derive(Debug, Args)]
struct ProfileLabelClearArgs {
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    browser: Option<String>,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        CommandKind::Daemon => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(server::run_daemon())
        }
        CommandKind::Tabs(args) => {
            ensure_server()?;
            let mut path = "/tabs".to_string();
            let mut query = Vec::new();
            if let Some(browser) = args.browser {
                query.push(format!("browser={}", query_escape(&browser)));
            }
            if let Some(profile) = args.profile {
                query.push(format!("profile={}", query_escape(&profile)));
            }
            if !query.is_empty() {
                path.push('?');
                path.push_str(&query.join("&"));
            }
            print_json(request("GET", &path, None, 30.0)?);
            Ok(())
        }
        CommandKind::Tabtree(args) => {
            ensure_server()?;
            let mut path = "/tabtree".to_string();
            let mut query = Vec::new();
            if let Some(tab) = args.tab {
                query.push(format!("tab={}", query_escape(&tab)));
            }
            if let Some(browser) = args.browser {
                query.push(format!("browser={}", query_escape(&browser)));
            }
            if let Some(profile) = args.profile {
                query.push(format!("profile={}", query_escape(&profile)));
            }
            if args.full {
                query.push("full=true".to_string());
            }
            if !query.is_empty() {
                path.push('?');
                path.push_str(&query.join("&"));
            }
            print_json(request("GET", &path, None, 30.0)?);
            Ok(())
        }
        CommandKind::Lookup(args) => run_lookup_command(args),
        CommandKind::ProfileLabel(args) => run_profile_label_command(args),
        CommandKind::Scan(args) => {
            ensure_server()?;
            print_json(request(
                "POST",
                "/scan",
                Some(json!({
                    "tabs_only": args.tabs_only,
                    "text_only": args.text_only,
                    "switch_tab_id": args.tab,
                    "browser": args.browser,
                    "profile": args.profile,
                })),
                args.timeout,
            )?);
            Ok(())
        }
        CommandKind::Exec(args) => {
            ensure_server()?;
            let script = if let Some(file) = args.file {
                std::fs::read_to_string(file)?
            } else {
                args.script
            };
            print_json(request(
                "POST",
                "/exec",
                Some(json!({
                    "script": script,
                    "switch_tab_id": args.tab,
                    "browser": args.browser,
                    "profile": args.profile,
                    "no_monitor": !args.monitor,
                    "wait_js": args.wait_js,
                    "wait_timeout": args.wait_timeout,
                    "wait_interval": args.wait_interval,
                })),
                args.timeout,
            )?);
            Ok(())
        }
        CommandKind::Snapshot(args) => {
            ensure_server()?;
            print_json(request(
                "POST",
                "/snapshot",
                Some(json!({
                    "switch_tab_id": args.tab,
                    "browser": args.browser,
                    "profile": args.profile,
                    "offset": args.offset,
                    "limit": args.limit,
                    "details": args.details,
                    "timeout": args.timeout,
                })),
                args.timeout,
            )?);
            Ok(())
        }
        CommandKind::Click(args) => run_target_command("/click", args),
        CommandKind::MouseClick(args) => run_target_command("/mouse-click", args),
        CommandKind::Fill(args) => run_fill_command(args),
        CommandKind::SendKeys(args) => run_send_keys_command(args),
        CommandKind::Screenshot(args) => run_screenshot_command(args),
        CommandKind::SavePdf(args) => run_save_pdf_command(args),
        CommandKind::Network(args) => run_network_command(args),
        CommandKind::Console(args) => run_console_command(args),
        CommandKind::Open(args) => {
            if !args.timeout.is_finite() || args.timeout <= 0.0 {
                return Err(anyhow!("open --timeout 必须是大于 0 的有限秒数"));
            }
            ensure_server()?;
            let cwd = env::current_dir().context("无法读取当前工作目录")?;
            let request_timeout = (args.timeout + 20.0).max(30.0);
            print_json(request(
                "POST",
                "/open",
                Some(json!({
                    "target": args.target.clone(),
                    "url": args.target,
                    "cwd": cwd,
                    "active": !args.background,
                    "window": args.window,
                    "allow_focus": args.focus,
                    "switch_tab_id": args.tab,
                    "browser": args.browser,
                    "profile": args.profile,
                    "session": args.session,
                    "group_title": args.group_title,
                    "readiness_timeout": args.timeout,
                })),
                request_timeout,
            )?);
            Ok(())
        }
        CommandKind::Close(args) => {
            ensure_server()?;
            print_json(request(
                "POST",
                "/close",
                Some(
                    json!({ "tab_id": args.tab, "browser": args.browser, "profile": args.profile }),
                ),
                args.timeout,
            )?);
            Ok(())
        }
        CommandKind::Status => {
            print_json(status_value()?);
            Ok(())
        }
        CommandKind::Logs(args) => print_logs(args.tail),
        CommandKind::Doctor => {
            print_json(doctor_value()?);
            Ok(())
        }
        CommandKind::InstallSkill(args) => install_skill(args),
        CommandKind::SetExtensionPort(args) => {
            let was_running = is_server_alive();
            let config = config::set_extension_port(args.port)?;
            let mut restarted = false;
            if was_running {
                let _ = request("POST", "/shutdown", Some(json!({})), 3.0);
                wait_server_stopped(Duration::from_secs(5));
                ensure_server()?;
                restarted = true;
            }
            print_json(json!({
                "ok": true,
                "extension_port": config.extension_port,
                "restarted": restarted
            }));
            Ok(())
        }
        CommandKind::Stop => {
            match request("POST", "/shutdown", Some(json!({})), 3.0) {
                Ok(value) => print_json(value),
                Err(_) => print_json(json!({ "ok": true, "status": "not_running" })),
            }
            Ok(())
        }
        CommandKind::Restart => {
            let _ = request("POST", "/shutdown", Some(json!({})), 3.0);
            wait_server_stopped(Duration::from_secs(5));
            ensure_server()?;
            print_json(request("GET", "/health", None, 3.0)?);
            Ok(())
        }
    }
}

fn query_escape(input: &str) -> String {
    input
        .bytes()
        .flat_map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => vec![b as char],
            _ => format!("%{b:02X}").chars().collect(),
        })
        .collect()
}

fn request(method: &str, path: &str, payload: Option<Value>, timeout_secs: f64) -> Result<Value> {
    let client = Client::builder()
        .timeout(Duration::from_secs_f64(timeout_secs.max(0.1)))
        .build()?;
    let url = format!("http://{HOST}:{PORT}{path}");
    let token = config::load_api_token()?;
    let with_token = |request: reqwest::blocking::RequestBuilder| {
        if let Some(token) = token.as_deref() {
            request.header(config::API_TOKEN_HEADER, token)
        } else {
            request
        }
    };
    let response = match method {
        "GET" => with_token(client.get(url)).send()?,
        "POST" => with_token(client.post(url))
            .json(&payload.unwrap_or_else(|| json!({})))
            .send()?,
        _ => return Err(anyhow!("不支持的 HTTP 方法: {method}")),
    };
    Ok(response.json()?)
}

fn is_server_alive() -> bool {
    request("GET", "/health", None, 1.0)
        .ok()
        .and_then(|v| v.get("running").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn ensure_server() -> Result<()> {
    if is_server_alive() {
        return Ok(());
    }
    let lock_path = project_dir().join(".agent-browser-cli.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let result = ensure_server_locked();
    let _ = lock.unlock();
    result
}

fn ensure_server_locked() -> Result<()> {
    if is_server_alive() {
        return Ok(());
    }
    start_server()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if is_server_alive() {
            return Ok(());
        }
        sleep(Duration::from_millis(200));
    }
    Err(anyhow!(
        "agent-browser-cli server 启动超时，查看 agent-browser-cli logs --tail 100"
    ))
}

fn start_server() -> Result<()> {
    let exe = env::current_exe()?;
    let log_path = config::ensure_log_file()?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("打开 daemon 日志失败: {}", log_path.display()))?;
    let log_err = log.try_clone()?;
    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .current_dir(project_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 后台 daemon 必须脱离当前终端会话，否则 CLI 退出后子进程会被一起回收。
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command.spawn()?;
    Ok(())
}

fn wait_server_stopped(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !is_server_alive() {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    !is_server_alive()
}

fn project_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn print_json(value: Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    );
}

fn status_value() -> Result<Value> {
    let configured = config::load_or_create()?.extension_port;
    let value = match request("GET", "/health", None, 1.0) {
        Ok(value) => enrich_status(value),
        Err(_) => json!({
            "ok": true,
            "healthy": false,
            "summary": "daemon_not_running",
            "message": "daemon 未运行；执行 tabs/open/exec/scan 会自动启动，或运行 agent-browser-cli restart 手动启动",
            "running": false,
            "ready": false,
            "ports": {
                "api": PORT,
                "extension": {
                    "configured": configured,
                    "listening": null,
                    "matched": false
                }
            },
            "connection": {
                "extension_connected": false,
                "active_tabs": 0
            }
        }),
    };
    Ok(value)
}

fn enrich_status(mut value: Value) -> Value {
    let running = value
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let ready = value.get("ready").and_then(Value::as_bool).unwrap_or(false);
    let matched = value
        .pointer("/ports/extension/matched")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let extension_connected = value
        .pointer("/connection/extension_connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let active_tabs = value
        .pointer("/connection/active_tabs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let (healthy, summary, message) = if !running {
        (
            false,
            "daemon_not_running",
            "daemon 未运行；执行 tabs/open/exec/scan 会自动启动，或运行 agent-browser-cli restart 手动启动",
        )
    } else if !matched {
        (
            false,
            "port_mismatch",
            "CLI 配置的插件端口和 daemon 实际监听端口不一致；运行 agent-browser-cli restart 让配置生效",
        )
    } else if !extension_connected {
        (
            false,
            "extension_not_connected",
            "浏览器插件未连接；确认 Chrome 已打开、插件已启用，且插件端口和 CLI 配置一致",
        )
    } else if active_tabs == 0 || !ready {
        (
            false,
            "no_active_tabs",
            "没有可用的普通网页标签页；请在 Chrome 打开一个 http/https 页面后重试",
        )
    } else {
        (true, "ready", "daemon、插件连接和浏览器标签页均可用")
    };
    value["ok"] = json!(true);
    value["healthy"] = json!(healthy);
    value["summary"] = json!(summary);
    value["message"] = json!(message);
    value
}

fn print_logs(tail: usize) -> Result<()> {
    let path = config::daemon_log_path()?;
    if !path.exists() {
        return Err(anyhow!(
            "日志文件不存在或尚无日志: {}；先运行 agent-browser-cli tabs/open/exec/scan 或 restart 启动 daemon",
            path.display()
        ));
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("读取日志文件失败: {}", path.display()))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(tail);
    for line in &lines[start..] {
        println!("{line}");
    }
    Ok(())
}

fn doctor_value() -> Result<Value> {
    let exe_ok = env::current_exe().map(|p| p.is_file()).unwrap_or(false);
    let config_path = config::config_path()?;
    let config_exists = config_path.exists();
    let config_result = config::load_existing();
    let (configured_port, config_ok, config_hint) = match config_result {
        Ok(cfg) => (Some(cfg.extension_port), true, Value::Null),
        Err(err) => (None, false, json!(err.to_string())),
    };
    let api_reachable = tcp_listening(PORT);
    let health = request("GET", "/health", None, 1.0).ok();
    let listening_port = health
        .as_ref()
        .and_then(|v| v.pointer("/ports/extension/listening"))
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok());
    let extension_port_to_check = listening_port.or(configured_port);
    let extension_port_ok = extension_port_to_check.map(tcp_listening).unwrap_or(false);
    let extension_connected = health
        .as_ref()
        .and_then(|v| v.pointer("/connection/extension_connected"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let active_tabs = health
        .as_ref()
        .and_then(|v| v.pointer("/connection/active_tabs"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let file_scheme_profiles = health
        .as_ref()
        .and_then(|v| v.pointer("/connection/profiles"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let running = health
        .as_ref()
        .and_then(|v| v.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let ready = health
        .as_ref()
        .and_then(|v| v.get("ready"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let port_conflict = configured_port == Some(PORT);
    let port_matched =
        configured_port.is_some() && listening_port.is_some() && configured_port == listening_port;
    let log_path = config::daemon_log_path()?;

    let mut checks = Vec::new();
    checks.push(json!({
        "name": "cli",
        "ok": exe_ok,
        "hint": if exe_ok { Value::Null } else { json!("当前 CLI 二进制不可执行或无法定位") }
    }));
    checks.push(json!({
        "name": "api",
        "ok": api_reachable,
        "host": HOST,
        "port": PORT,
        "hint": if api_reachable { Value::Null } else { json!("daemon API 未监听；doctor 不会自动启动 daemon，可运行 agent-browser-cli restart") }
    }));
    checks.push(json!({
        "name": "config",
        "ok": config_ok,
        "path": config_path,
        "exists": config_exists,
        "extension_port": configured_port,
        "hint": config_hint
    }));
    checks.push(json!({
        "name": "extension_port",
        "ok": extension_port_ok && !port_conflict,
        "configured_port": configured_port,
        "listening_port": listening_port,
        "port_matched": port_matched,
        "hint": extension_port_hint(configured_port, listening_port, extension_port_ok, port_conflict)
    }));
    checks.push(json!({
        "name": "health",
        "ok": running,
        "running": running,
        "ready": ready,
        "extension_connected": extension_connected,
        "active_tabs": active_tabs,
        "hint": if running { Value::Null } else { json!("daemon 未运行；运行 agent-browser-cli restart 或执行 tabs/open/exec/scan 自动启动") }
    }));
    checks.push(json!({
        "name": "extension_connected",
        "ok": extension_connected,
        "configured_port": configured_port,
        "listening_port": listening_port,
        "connected": extension_connected,
        "hint": if extension_connected { Value::Null } else { json!(format!("浏览器插件未连接；确认插件已启用，且插件端口为 {}", configured_port.unwrap_or(config::DEFAULT_EXTENSION_PORT))) }
    }));
    checks.push(json!({
        "name": "active_tabs",
        "ok": active_tabs > 0,
        "active_tabs": active_tabs,
        "hint": if active_tabs > 0 { Value::Null } else { json!("Chrome 需要至少打开一个普通 http/https/file 网页标签页") }
    }));
    checks.push(json!({
        "name": "file_scheme_access",
        "ok": true,
        "optional": true,
        "profiles": file_scheme_profiles,
        "hint": "文件网址访问是可选能力；关闭时不影响普通 http/https 页面控制"
    }));

    let ok = checks
        .iter()
        .all(|c| c.get("ok").and_then(Value::as_bool).unwrap_or(false));
    Ok(json!({
        "ok": ok,
        "checks": checks,
        "log_path": log_path,
        "log_exists": log_path.exists()
    }))
}

fn extension_port_hint(
    configured: Option<u16>,
    listening: Option<u16>,
    extension_port_ok: bool,
    port_conflict: bool,
) -> Value {
    if port_conflict {
        json!("插件端口不能和 API 端口 18767 相同；运行 agent-browser-cli set-extension-port <port> 修改")
    } else if configured.is_some() && listening.is_some() && configured != listening {
        json!("CLI 配置和 daemon 实际监听端口不一致；运行 agent-browser-cli restart 让配置生效")
    } else if !extension_port_ok {
        json!("当前插件端口未监听；daemon 未启动或 WebSocket 端口启动失败")
    } else {
        Value::Null
    }
}

fn tcp_listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("{HOST}:{port}").parse().expect("固定地址应可解析"),
        Duration::from_millis(300),
    )
    .is_ok()
}

#[derive(Debug)]
struct SkillPlanItem {
    path: PathBuf,
    action: String,
    will_write: bool,
    note: String,
}

fn install_skill(args: InstallSkillArgs) -> Result<()> {
    let source = resolve_skill_source()?;
    let home = home_dir()?;
    let main_target = home.join(".agents/skills/agent-browser-cli");
    let mut plan = Vec::new();
    plan.push(SkillPlanItem {
        path: main_target.clone(),
        action: if main_target.exists() {
            "update_main"
        } else {
            "create_main"
        }
        .to_string(),
        will_write: true,
        note: format!("复制内置 skill 目录: {}", source.display()),
    });

    for parent in [
        home.join(".codex/skills"),
        home.join(".claude/skills"),
        home.join(".config/agents/skills"),
        home.join(".cursor/skills"),
        home.join(".gemini/skills"),
    ] {
        let target = parent.join("agent-browser-cli");
        if !parent.exists() {
            plan.push(SkillPlanItem {
                path: target,
                action: "skip_missing_parent".to_string(),
                will_write: false,
                note: "父目录不存在，跳过软链接".to_string(),
            });
            continue;
        }
        let meta = fs::symlink_metadata(&target).ok();
        if let Some(meta) = meta {
            if meta.file_type().is_symlink() {
                plan.push(SkillPlanItem {
                    path: target,
                    action: "update_symlink".to_string(),
                    will_write: true,
                    note: format!("更新软链接到 {}", main_target.display()),
                });
            } else {
                plan.push(SkillPlanItem {
                    path: target,
                    action: "skip_existing_entity".to_string(),
                    will_write: false,
                    note: "目标已存在且不是软链接，必须手动处理，--yes 也不会覆盖".to_string(),
                });
            }
        } else {
            plan.push(SkillPlanItem {
                path: target,
                action: "create_symlink".to_string(),
                will_write: true,
                note: format!("创建软链接到 {}", main_target.display()),
            });
        }
    }

    print_skill_plan(&plan, args.dry_run);
    if args.dry_run {
        return Ok(());
    }
    if !args.yes && !confirm_install()? {
        println!("已取消安装");
        return Ok(());
    }

    copy_dir_clean(&source, &main_target)?;
    for item in plan
        .iter()
        .filter(|item| item.action.ends_with("symlink") && item.will_write)
    {
        if let Some(parent) = item.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(&item.path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            remove_path(&item.path)?;
        }
        create_symlink_dir(&main_target, &item.path)?;
    }
    println!("安装完成: {}", main_target.join("SKILL.md").display());
    Ok(())
}

fn print_skill_plan(plan: &[SkillPlanItem], dry_run: bool) {
    println!("agent-browser-cli skill 安装计划");
    if dry_run {
        println!("模式: dry-run，不写文件、不创建软链接");
    }
    for item in plan {
        println!(
            "- [{}] {} -> {} ({})",
            if item.will_write { "write" } else { "skip" },
            item.action,
            item.path.display(),
            item.note
        );
    }
}

fn confirm_install() -> Result<bool> {
    print!("确认执行以上安装计划？输入 yes 继续: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim() == "yes")
}

fn resolve_skill_source() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(package_dir) = env::var_os("AGENT_BROWSER_CLI_PACKAGE_DIR") {
        candidates.push(PathBuf::from(package_dir).join("skills/agent-browser-cli"));
    }
    if let Ok(exe) = env::current_exe() {
        for ancestor in exe.ancestors().take(6) {
            candidates.push(ancestor.join("skills/agent-browser-cli"));
        }
    }
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd.join("skills/agent-browser-cli"));
    }
    for candidate in candidates {
        if candidate.join("SKILL.md").is_file() {
            return Ok(candidate);
        }
    }
    Err(anyhow!("找不到内置 skill 目录 skills/agent-browser-cli"))
}

fn copy_dir_clean(source: &Path, target: &Path) -> Result<()> {
    if target.exists() || fs::symlink_metadata(target).is_ok() {
        if fs::symlink_metadata(target)?.file_type().is_symlink() {
            return Err(anyhow!("主安装目录不能是软链接: {}", target.display()));
        }
        fs::remove_dir_all(target)
            .with_context(|| format!("清理主安装目录失败: {}", target.display()))?;
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src = entry.path();
        let dst = target.join(entry.file_name());
        let meta = entry.file_type()?;
        if meta.is_dir() {
            copy_dir_clean(&src, &dst)?;
        } else if meta.is_file() {
            fs::copy(&src, &dst).with_context(|| format!("复制文件失败: {}", src.display()))?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(path)?;
    } else if meta.is_dir() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn create_symlink_dir(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target)
        .with_context(|| format!("创建软链接失败: {}", target.display()))
}

#[cfg(windows)]
fn create_symlink_dir(source: &Path, target: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
        .with_context(|| format!("创建软链接失败: {}", target.display()))
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("无法定位用户主目录"))
}

fn run_lookup_command(args: LookupCommand) -> Result<()> {
    ensure_server()?;
    let path = match args.target {
        LookupTarget::Tab(args) => {
            let mut path = format!("/lookup/tab/{}", query_escape(&args.tab));
            let mut query = Vec::new();
            if let Some(browser) = args.browser {
                query.push(format!("browser={}", query_escape(&browser)));
            }
            if let Some(profile) = args.profile {
                query.push(format!("profile={}", query_escape(&profile)));
            }
            if !query.is_empty() {
                path.push('?');
                path.push_str(&query.join("&"));
            }
            path
        }
        LookupTarget::Browser(args) => format!("/lookup/browser/{}", query_escape(&args.browser)),
        LookupTarget::Profile(args) => format!("/lookup/profile/{}", query_escape(&args.profile)),
    };
    print_json(request("GET", &path, None, 30.0)?);
    Ok(())
}

fn run_profile_label_command(args: ProfileLabelCommand) -> Result<()> {
    ensure_server()?;
    match args.action {
        ProfileLabelAction::Set(args) => {
            print_json(request(
                "POST",
                "/profile-label",
                Some(json!({
                    "label": args.label,
                    "profile": args.profile,
                    "browser": args.browser,
                    "switch_tab_id": args.tab,
                })),
                args.timeout,
            )?);
        }
        ProfileLabelAction::Clear(args) => {
            print_json(request(
                "POST",
                "/profile-label",
                Some(json!({
                    "label": null,
                    "profile": args.profile,
                    "browser": args.browser,
                    "switch_tab_id": args.tab,
                })),
                args.timeout,
            )?);
        }
    }
    Ok(())
}

fn run_target_command(path: &str, args: TargetCommandArgs) -> Result<()> {
    ensure_server()?;
    print_json(request(
        "POST",
        path,
        Some(json!({
            "target": args.target,
            "switch_tab_id": args.tab,
            "browser": args.browser,
            "profile": args.profile,
            "monitor": args.monitor,
            "wait_js": args.wait_js,
            "wait_timeout": args.wait_timeout,
            "wait_interval": args.wait_interval,
            "timeout": args.timeout,
        })),
        args.timeout + args.wait_timeout + 5.0,
    )?);
    Ok(())
}

fn run_fill_command(args: FillCommandArgs) -> Result<()> {
    ensure_server()?;
    let has_value = args.value.is_some();
    let value = args.value.unwrap_or_default();
    print_json(request(
        "POST",
        "/fill",
        Some(json!({
            "target": args.target,
            "value": value,
            "has_value": has_value,
            "switch_tab_id": args.tab,
            "browser": args.browser,
            "profile": args.profile,
            "append": args.append,
            "clear": args.clear,
            "monitor": args.monitor,
            "wait_js": args.wait_js,
            "wait_timeout": args.wait_timeout,
            "wait_interval": args.wait_interval,
            "timeout": args.timeout,
        })),
        args.timeout + args.wait_timeout + 5.0,
    )?);
    Ok(())
}

fn run_send_keys_command(args: SendKeysCommandArgs) -> Result<()> {
    ensure_server()?;
    print_json(request(
        "POST",
        "/send-keys",
        Some(json!({
            "keys": args.keys,
            "target": args.target,
            "switch_tab_id": args.tab,
            "browser": args.browser,
            "profile": args.profile,
            "monitor": args.monitor,
            "wait_js": args.wait_js,
            "wait_timeout": args.wait_timeout,
            "wait_interval": args.wait_interval,
            "timeout": args.timeout,
        })),
        args.timeout + args.wait_timeout + 5.0,
    )?);
    Ok(())
}

fn run_network_command(args: NetworkCommand) -> Result<()> {
    ensure_server()?;
    match args.action {
        NetworkAction::Start(args) => print_json(request(
            "POST",
            "/network/start",
            Some(
                json!({ "switch_tab_id": args.tab, "browser": args.browser, "profile": args.profile }),
            ),
            args.timeout,
        )?),
        NetworkAction::List(args) => print_json(request(
            "POST",
            "/network/list",
            Some(json!({
                "switch_tab_id": args.tab,
                "browser": args.browser,
                "profile": args.profile,
                "filter": args.filter,
                "limit": args.limit,
            })),
            args.timeout,
        )?),
        NetworkAction::Detail(args) => print_json(request(
            "POST",
            "/network/detail",
            Some(json!({
                "switch_tab_id": args.tab,
                "browser": args.browser,
                "profile": args.profile,
                "request_id": args.request_id,
            })),
            args.timeout,
        )?),
        NetworkAction::Clear(args) => print_json(request(
            "POST",
            "/network/clear",
            Some(
                json!({ "switch_tab_id": args.tab, "browser": args.browser, "profile": args.profile }),
            ),
            args.timeout,
        )?),
        NetworkAction::Stop(args) => print_json(request(
            "POST",
            "/network/stop",
            Some(
                json!({ "switch_tab_id": args.tab, "browser": args.browser, "profile": args.profile }),
            ),
            args.timeout,
        )?),
    }
    Ok(())
}

fn run_console_command(args: ConsoleCommand) -> Result<()> {
    ensure_server()?;
    match args.action {
        ConsoleAction::Start(args) => print_json(request(
            "POST",
            "/console/start",
            Some(
                json!({ "switch_tab_id": args.tab, "browser": args.browser, "profile": args.profile }),
            ),
            args.timeout,
        )?),
        ConsoleAction::List(args) => print_json(request(
            "POST",
            "/console/list",
            Some(json!({
                "switch_tab_id": args.tab,
                "browser": args.browser,
                "profile": args.profile,
                "level": args.level,
                "limit": args.limit,
            })),
            args.timeout,
        )?),
        ConsoleAction::Clear(args) => print_json(request(
            "POST",
            "/console/clear",
            Some(
                json!({ "switch_tab_id": args.tab, "browser": args.browser, "profile": args.profile }),
            ),
            args.timeout,
        )?),
        ConsoleAction::Stop(args) => print_json(request(
            "POST",
            "/console/stop",
            Some(
                json!({ "switch_tab_id": args.tab, "browser": args.browser, "profile": args.profile }),
            ),
            args.timeout,
        )?),
    }
    Ok(())
}

fn run_screenshot_command(args: ScreenshotArgs) -> Result<()> {
    ensure_server()?;
    print_json(request(
        "POST",
        "/screenshot",
        Some(json!({
            "switch_tab_id": args.tab,
            "browser": args.browser,
            "profile": args.profile,
            "target": args.target,
            "selector": args.selector,
            "out": args.out,
            "format": args.format,
            "quality": args.quality,
            "full_page": args.full_page,
            "timeout": args.timeout,
        })),
        args.timeout + 5.0,
    )?);
    Ok(())
}

fn run_save_pdf_command(args: SavePdfArgs) -> Result<()> {
    ensure_server()?;
    print_json(request(
        "POST",
        "/save-pdf",
        Some(json!({
            "switch_tab_id": args.tab,
            "browser": args.browser,
            "profile": args.profile,
            "out": args.out,
            "paper": args.paper,
            "landscape": args.landscape,
            "scale": args.scale,
            "print_background": !args.no_print_background,
            "timeout": args.timeout,
        })),
        args.timeout + 5.0,
    )?);
    Ok(())
}
