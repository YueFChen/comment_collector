use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{Value, json};
use wonderland_plugin_sdk::{
    HostClient, PluginError, PluginFailure, RequestTracker, serve,
};
use wonderland_comment_collector::{
    ArchiveSummary, CommentArchive, CommentCollector, CommentQuery, ExportFormat, ExportOutcome,
    PublicHttpClient,
};

const CONTRACT: &str = include_str!("../package/contract.json");

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
    if let Err(error) = run() {
        eprintln!("comment_collector backend stopped: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let data_dir = std::env::var_os("WONDERLAND_PLUGIN_DATA_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "Core did not provide a plugin data directory".to_owned())?;
    let http = Arc::new(CoreHttp::default());
    let collector = Arc::new(CommentCollector::new(data_dir.join("archive-v1"), http.clone())
        .map_err(|error| error.to_string())?);
    let active_collection = RequestTracker::default();
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()
        .map_err(|error| error.to_string())?);
    serve("comment_collector", "0.1.0", CONTRACT, move |host, method, params, request_id| {
        http.set_host(host.clone());
        dispatch(&host, &collector, &runtime, &active_collection, &method, params, request_id)
            .map_err(|error| PluginError::new(
                format!("PLUGIN_COMMENT_COLLECTOR_{}", error.code().to_ascii_uppercase()),
                error.to_string(),
            ))
    }).map_err(|error| error.to_string())
}

fn dispatch(
    host: &HostClient,
    collector: &CommentCollector,
    runtime: &tokio::runtime::Runtime,
    active_collection: &RequestTracker,
    method: &str,
    params: Value,
    request_id: Option<String>,
) -> Result<Value, PluginFailure> {
    match method {
        "archives" => serde_json::to_value(collector.archives()?).map_err(|_| PluginFailure::InvalidResponse),
        "archive" => {
            let level_id = string_param(&params, "level_id")?;
            serde_json::to_value(collector.archive(level_id)?).map_err(|_| PluginFailure::InvalidResponse)
        }
        "collect" => {
            let request_id = request_id.as_deref().ok_or(PluginFailure::InvalidInput)?;
            let _active = active_collection.begin(request_id).ok_or(PluginFailure::Busy)?;
            let query: CommentQuery = value_param(&params, "query")?;
            let event_host = host.clone();
            let event_id = Some(request_id.to_owned());
            let archive = runtime.block_on(collector.collect(query, move |progress| {
                let payload = serde_json::to_value(progress).unwrap_or_else(|_| json!({}));
                let _ = event_host.emit("collect.progress", event_id.as_deref(), payload);
            }))?;
            serde_json::to_value(archive).map_err(|_| PluginFailure::InvalidResponse)
        }
        "export" => {
            let level_id = string_param(&params, "level_id")?;
            let format: ExportFormat = value_param(&params, "format")?;
            let payload = collector.export_payload(level_id, format)?;
            let response = host.call_core("core.files.export", json!({
                "name": payload.name,
                "contentBase64": base64::engine::general_purpose::STANDARD.encode(payload.content),
            })).map_err(host_error)?;
            let path = response.get("path").and_then(Value::as_str).ok_or(PluginFailure::InvalidResponse)?;
            serde_json::to_value(ExportOutcome { path: path.to_owned(), format: payload.format, rows: payload.rows })
                .map_err(|_| PluginFailure::InvalidResponse)
        }
        "export_dir" => {
            let result = host.call_core("core.files.export_dir", json!({})).map_err(host_error)?;
            Ok(json!(result.get("path").and_then(Value::as_str).ok_or(PluginFailure::InvalidResponse)?))
        }
        "reveal_dir" => host.call_core("core.files.reveal_own", json!({ "scope": "exports" }))
            .map_err(host_error),
        "__cancel" => {
            if params.get("requestId").and_then(Value::as_str)
                .is_some_and(|request_id| active_collection.matches(request_id))
            {
                collector.cancel();
            }
            Ok(Value::Null)
        }
        _ => Err(PluginFailure::InvalidInput),
    }
}

fn value_param<T: serde::de::DeserializeOwned>(params: &Value, key: &str) -> Result<T, PluginFailure> {
    serde_json::from_value(params.get(key).cloned().ok_or(PluginFailure::InvalidInput)?)
        .map_err(|_| PluginFailure::InvalidInput)
}

fn string_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, PluginFailure> {
    params.get(key).and_then(Value::as_str).ok_or(PluginFailure::InvalidInput)
}

#[derive(Default)]
struct CoreHttp { host: RwLock<Option<HostClient>> }

impl CoreHttp {
    fn set_host(&self, host: HostClient) {
        if let Ok(mut current) = self.host.write() { *current = Some(host); }
    }
    fn host(&self) -> Result<HostClient, PluginFailure> {
        self.host.read().ok().and_then(|value| value.clone()).ok_or(PluginFailure::NotInitialized)
    }
    fn request(&self, method: &str, url: &str, headers: &[(&str, &str)], query: &[(String, String)], body: Option<&str>) -> Result<Vec<u8>, PluginFailure> {
        let headers: Vec<[&str; 2]> = headers.iter().map(|(name, value)| [*name, *value]).collect();
        let query: Vec<[&str; 2]> = query.iter().map(|(name, value)| [name.as_str(), value.as_str()]).collect();
        let response = self.host()?.call_core("core.network.public", json!({
            "method": method, "url": url, "headers": headers, "query": query, "body": body,
        })).map_err(host_error)?;
        let content = response.get("contentBase64").and_then(Value::as_str).ok_or(PluginFailure::InvalidResponse)?;
        base64::engine::general_purpose::STANDARD.decode(content).map_err(|_| PluginFailure::InvalidResponse)
    }
}

#[async_trait]
impl PublicHttpClient for CoreHttp {
    async fn get_bytes(&self, url: &str, headers: &[(&str, &str)], query: &[(String, String)]) -> Result<Vec<u8>, PluginFailure> {
        self.request("GET", url, headers, query, None)
    }
    async fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &str) -> Result<Vec<u8>, PluginFailure> {
        self.request("POST", url, headers, &[], Some(body))
    }
}

fn host_error(error: PluginError) -> PluginFailure {
    match error.code.as_str() {
        "TIMEOUT" => PluginFailure::Timeout,
        "UNAUTHORIZED" => PluginFailure::InvalidInput,
        code if code.contains("HTTP_ERROR") => PluginFailure::Http(500),
        _ => PluginFailure::Transport(error.message),
    }
}
