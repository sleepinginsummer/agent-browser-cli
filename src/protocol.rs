use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub browser_id: String,
    #[serde(default)]
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_label: Option<String>,
    #[serde(default)]
    pub session_key: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "type", default = "default_tab_type")]
    pub tab_type: String,
    #[serde(default)]
    pub connected_at: Option<f64>,
}

fn default_tab_type() -> String {
    "ext_ws".to_string()
}

#[derive(Debug, Clone)]
pub struct Session {
    pub session_key: String,
    pub tab_id: String,
    pub browser_id: String,
    pub profile_id: String,
    pub profile_label: Option<String>,
    pub extension_version: Option<String>,
    pub file_scheme_access: Option<bool>,
    pub info: TabInfo,
    pub sender: mpsc::UnboundedSender<String>,
    pub disconnected_at: Option<Instant>,
}

impl Session {
    pub fn is_active(&self) -> bool {
        self.disconnected_at.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "newTabs")]
    pub new_tabs: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RectInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementDomInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<RectInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dom_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementRef {
    pub ref_id: String,
    pub backend_dom_node_id: i64,
    pub index: usize,
    pub role: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dom: Option<ElementDomInfo>,
}

#[derive(Debug, Clone)]
pub struct SnapshotCache {
    pub generation: u64,
    pub url: String,
    pub refs: HashMap<String, ElementRef>,
}

#[derive(Debug)]
pub struct PendingExec {
    pub delivered_at: Option<Instant>,
    pub tx: oneshot::Sender<anyhow::Result<ExecResult>>,
}

#[derive(Default)]
pub struct DriverState {
    pub sessions: HashMap<String, Session>,
    pub snapshots: HashMap<String, SnapshotCache>,
    pub pending: HashMap<String, PendingExec>,
    pub default_session_key: Option<String>,
    pub latest_session_key: Option<String>,
    pub preferred_default_session_key: Option<String>,
    pub active_exec_sessions: HashMap<String, String>,
    pub acked: HashSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum WsIncoming {
    #[serde(rename = "ext_ready")]
    ExtReady {
        #[serde(default = "default_browser_id")]
        browser_id: String,
        #[serde(default = "default_profile_id")]
        profile_id: String,
        #[serde(default)]
        profile_label: Option<String>,
        #[serde(default)]
        extension_version: Option<String>,
        #[serde(default)]
        file_scheme_access: Option<bool>,
        tabs: Vec<ExtTab>,
    },
    #[serde(rename = "tabs_update")]
    TabsUpdate {
        #[serde(default = "default_browser_id")]
        browser_id: String,
        #[serde(default = "default_profile_id")]
        profile_id: String,
        #[serde(default)]
        profile_label: Option<String>,
        #[serde(default)]
        extension_version: Option<String>,
        #[serde(default)]
        file_scheme_access: Option<bool>,
        tabs: Vec<ExtTab>,
    },
    #[serde(rename = "ack")]
    Ack { id: String },
    #[serde(rename = "result")]
    Result {
        id: String,
        result: Value,
        #[serde(rename = "newTabs")]
        new_tabs: Option<Value>,
    },
    #[serde(rename = "error")]
    Error {
        id: String,
        error: Value,
        #[serde(rename = "newTabs")]
        new_tabs: Option<Value>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct ExtTab {
    pub id: Value,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
}

impl ExtTab {
    pub fn into_tab_info(
        self,
        browser_id: &str,
        profile_id: &str,
        profile_label: Option<String>,
    ) -> TabInfo {
        let tab_id = match self.id {
            Value::String(s) => s,
            other => other.to_string(),
        };
        let session_key = make_session_key(browser_id, profile_id, &tab_id);
        TabInfo {
            id: tab_id.clone(),
            tab_id,
            browser_id: browser_id.to_string(),
            profile_id: profile_id.to_string(),
            profile_label,
            session_key,
            url: self.url,
            title: self.title,
            tab_type: "ext_ws".to_string(),
            connected_at: None,
        }
    }
}

pub fn make_session_key(browser_id: &str, profile_id: &str, tab_id: &str) -> String {
    format!("{browser_id}:{profile_id}:{tab_id}")
}

fn default_browser_id() -> String {
    "browser-default".to_string()
}

fn default_profile_id() -> String {
    "profile-default".to_string()
}
