mod agent_logs;
mod blender_runtime;
mod blender_service;
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
mod update;
mod usage;
mod workflows;

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
        "cache" => serde_json::to_value(cyapi_routes::read_cache(&data))
            .map_err(|_| "无法读取线路缓存".into()),
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
    app: tauri::AppHandle,
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
            | "integration-services-read"
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
            let result = if action == "open-url" {
                crate::update::open(request.url.as_deref().unwrap_or_default())
            } else if action == "integration-services-read" {
                let workspace = request.workspace.as_deref().unwrap_or("codex");
                // Snapshot only local endpoint settings under the lock. DNS and
                // addon I/O must not block settings, MCP or other local pages.
                let probes = service
                    .lock()
                    .map_err(|_| "安装服务暂不可用".to_string())
                    .and_then(|state| {
                        state
                            .as_ref()
                            .map_err(|error| error.clone())?
                            .integration_service_probes(workspace)
                    });
                probes.map(|probes| {
                    tauri::async_runtime::block_on(blender_service::read(workspace, probes))
                })
            } else if matches!(action.as_str(), "update-download" | "update-install") {
                // Copy only immutable update settings, then release the global
                // lock before any network request or large file operation.
                let context = service
                    .lock()
                    .map_err(|_| "安装服务暂不可用".to_string())
                    .and_then(|state| {
                        state
                            .as_ref()
                            .map(update::UpdateContext::snapshot)
                            .map_err(|error| error.clone())
                    });
                context.and_then(|context| {
                    if action == "update-download" {
                        use tauri::Emitter;
                        update::download(&context, |stage, downloaded, total| {
                            let _ = app.emit(
                                "toolbox-update-progress",
                                update::UpdateProgress {
                                    operation_id: request.operation_id.clone().unwrap_or_default(),
                                    stage,
                                    downloaded,
                                    total,
                                },
                            );
                        })
                    } else {
                        update::install(&context, |path| {
                            // Wait for existing Service writes to finish. Keep
                            // the mutex through launch, then replace the state
                            // with a shutdown barrier before releasing it, so
                            // no new write can start during the exit delay.
                            let mut state =
                                service.lock().map_err(|_| "安装服务暂不可用".to_string())?;
                            state.as_ref().map_err(|error| error.clone())?;
                            update::launch_and_exit(path)?;
                            *state =
                                Err("更新安装程序已启动，应用正在关闭，请稍后重新打开。".into());
                            Ok(())
                        })
                    }
                })
            } else {
                service
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
                    })
            };
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
    if let Err(error) = &service {
        logging::event(
            "error",
            "bootstrap",
            format!("安装服务初始化失败；仍可使用排查与日志功能：{error}"),
        );
    }
    let builder = tauri::Builder::default().manage(AppService(Arc::new(Mutex::new(service))));
    let builder = builder.manage(workflows::Workflows::default());
    let handler: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![
            toolbox,
            toolbox_cancel,
            cyapi_routes,
            cyapi_balance::cyapi_balance,
            usage::usage_stats,
            logging::logs_snapshot,
            logging::log_frontend_error,
            diagnostics::diagnostics,
            codex_logs::codex_logs,
            agent_logs::agent_logs,
            workflows::workflows,
            workflows::reload_workflows
        ]);
    builder
        .invoke_handler(move |invoke| {
            // App commands have no remote caller. A child WebView shares the
            // OS window, so checking only the window label is insufficient.
            if !workflows::trusted_caller(invoke.message.webview_ref()) {
                invoke.resolver.reject("远程页面没有本机操作权限");
                return true;
            }
            handler(invoke)
        })
        .build(tauri::generate_context!())
        .expect("Failed to launch CYAPI Toolbox")
        .run(|_app, _event| {});
}
