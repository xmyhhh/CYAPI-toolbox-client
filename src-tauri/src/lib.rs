mod agent_logs;
mod cancel;
mod catalog_cache;
mod claude_config;
mod codex_config;
mod codex_logs;
mod codex_models;
mod configuration;
mod cyapi_balance;
mod cyapi_routes;
mod diagnostics;
mod external_tools;
mod installed;
mod integrations;
mod logging;
mod mcp;
mod model_list;
mod model_selection;
mod models;
mod opencode_config;
mod platform;
mod python_dependencies;
mod python_runtime;
mod runtime;
mod service;
mod software;
mod telemetry;
mod update;
mod usage;

use models::Request;
use service::Service;
use std::sync::{Arc, Mutex};

struct AppService(Arc<Mutex<Result<Service, String>>>);

/// Network payload fetched before the service lock is taken, keyed by action.
enum Prefetched {
    Catalog(&'static str, Result<service::Fetched, String>),
    Update(Result<Vec<u8>, String>),
}

#[tauri::command]
async fn cyapi_routes(
    action: String,
    route_id: Option<String>,
    state: tauri::State<'_, AppService>,
) -> Result<serde_json::Value, String> {
    // Release the toolbox lock before network I/O, so probing never blocks local configuration.
    let data = state
        .0
        .lock()
        .map_err(|_| "线路服务暂不可用")?
        .as_ref()
        .map_err(|e| e.clone())?
        .data
        .clone();
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "read" => {
            serde_json::to_value(cyapi_routes::read(&data)).map_err(|_| "无法读取线路清单".into())
        }
        "probe" => serde_json::to_value(cyapi_routes::probe(
            &data,
            route_id.as_deref().ok_or("缺少线路标识")?,
        )?)
        .map_err(|_| "无法读取测速结果".into()),
        _ => Err("未知线路操作".into()),
    })
    .await
    .map_err(|_| "线路操作未完成".to_string())?
}

/// Stop a running installation. The command does not take the service lock, so
/// it reaches the download that is in flight instead of waiting for it.
#[tauri::command]
fn toolbox_cancel() {
    crate::cancel::cancel();
}

#[tauri::command]
async fn toolbox(
    request: Request,
    state: tauri::State<'_, AppService>,
) -> Result<serde_json::Value, String> {
    let operation = request
        .operation_id
        .clone()
        .filter(|id| {
            id.len() <= 80
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let action = request.action.clone();
    let visible = !matches!(
        action.as_str(),
        "status"
            | "catalog-read"
            | "preview"
            | "codex-read"
            | "claude-read"
            | "opencode-read"
            | "configuration-read"
            | "models-list"
            | "update-check"
    );
    let started = std::time::Instant::now();
    if visible {
        logging::with_operation(operation.clone(), || {
            logging::event("info", "toolbox", format!("开始操作：{action}"));
        });
    }
    // Network-bound reads are fetched before taking the service lock, so local
    // configuration pages stay usable while the server is slow or offline.
    let prefetch_kind = match action.as_str() {
        "catalog-read" | "refresh" => Some("/api/catalog"),
        "update-check" => Some("update"),
        "mcp-catalog-read" | "mcp-refresh" => Some("/api/mcp"),
        "integrations-read" | "integrations-refresh" => Some("/api/integrations"),
        _ => None,
    };
    let prefetched = if let Some(kind) = prefetch_kind {
        let (server_url, data, needed) = {
            let guard = state.0.lock().map_err(|_| "安装服务暂不可用")?;
            let service = guard.as_ref().map_err(|error| error.clone())?;
            let needed = kind == "update"
                || service::claim_catalog_refresh(
                    &service.data,
                    kind,
                    matches!(
                        action.as_str(),
                        "refresh" | "mcp-refresh" | "integrations-refresh"
                    ),
                );
            (
                service.server_url().to_string(),
                service.data.clone(),
                needed,
            )
        };
        if needed {
            let operation = operation.clone();
            if kind == "update" {
                let fetched = tauri::async_runtime::spawn_blocking(move || {
                    logging::with_operation(operation, || {
                        crate::update::fetch_manifest(&server_url)
                    })
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
                Some(Prefetched::Update(fetched))
            } else {
                let data = data.clone();
                let fetched = tauri::async_runtime::spawn_blocking(move || {
                    logging::with_operation(operation, || {
                        service::fetch_catalog_conditional(&server_url, &data, kind)
                    })
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
                Some(Prefetched::Catalog(kind, fetched))
            }
        } else {
            None
        }
    } else {
        None
    };
    let service = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        logging::with_operation(operation, || {
            if let Some(input) = &request.codex {
                logging::register_secret(&input.api_key);
            }
            if let Some(input) = &request.claude {
                logging::register_secret(&input.auth_token);
            }
            if let Some(input) = &request.opencode {
                logging::register_secret(&input.api_key);
            }
            if let Some(key) = &request.api_key {
                logging::register_secret(key);
            }
            if request.action == "configuration-save" || request.action == "integration-action" {
                if let Some(values) = &request.values {
                    for value in values.values().filter_map(|v| v.as_str()) {
                        logging::register_secret(value);
                    }
                }
            }
            let result = service
                .lock()
                .map_err(|_| "安装服务暂不可用".to_string())
                .and_then(|mut state| {
                    let service = state.as_mut().map_err(|error| error.clone())?;
                    match prefetched {
                        Some(Prefetched::Catalog(path, fetched)) => {
                            let automatic = matches!(
                                request.action.as_str(),
                                "catalog-read" | "mcp-catalog-read" | "integrations-read"
                            );
                            service.apply_catalog_read(path, fetched, automatic)?;
                            if path == "/api/catalog" {
                                Ok(service.status())
                            } else {
                                service.dispatch(request)
                            }
                        }
                        Some(Prefetched::Update(bytes)) => service.apply_update_check(bytes),
                        None => service.dispatch(request),
                    }
                });
            if let Ok(value) = &result {
                for field in ["apiKey", "authToken"] {
                    if let Some(key) = value.get(field).and_then(|v| v.as_str()) {
                        logging::register_secret(key);
                    }
                }
            }
            match &result {
                Ok(_) if visible => logging::event(
                    "info",
                    "toolbox",
                    format!("操作完成：{action}（{} ms）", started.elapsed().as_millis()),
                ),
                // A failed background update check is expected while offline.
                Err(error) if action != "update-check" => logging::event(
                    "error",
                    "toolbox",
                    format!(
                        "操作失败：{action}；{error}（{} ms）",
                        started.elapsed().as_millis()
                    ),
                ),
                _ => {}
            }
            result
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::initialize();
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        logging::event("error", "bootstrap", format!("原生异常：{info}"));
        previous_hook(info);
    }));
    logging::event(
        "info",
        "bootstrap",
        format!("CYAPI Toolbox {} 启动", env!("CARGO_PKG_VERSION")),
    );
    let service = Service::from_environment();
    if let Ok(service) = &service {
        telemetry::start(service.data.clone(), service.server_url().to_string());
    }
    if let Err(error) = &service {
        logging::event(
            "error",
            "bootstrap",
            format!("安装服务初始化失败；仍可使用排查与日志功能：{error}"),
        );
    }
    tauri::Builder::default()
        .manage(AppService(Arc::new(Mutex::new(service))))
        .invoke_handler(tauri::generate_handler![
            toolbox,
            toolbox_cancel,
            cyapi_routes,
            cyapi_balance::cyapi_balance,
            usage::usage_stats,
            logging::logs_snapshot,
            logging::log_frontend_error,
            diagnostics::diagnostics,
            codex_logs::codex_logs,
            agent_logs::agent_logs
        ])
        .run(tauri::generate_context!())
        .expect("Failed to launch CYAPI Toolbox");
}
