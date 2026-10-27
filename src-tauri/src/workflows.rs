//! Remote workflow UI owned by the existing Service; no engine or Key handling.
use serde::{Deserialize, Serialize};
use std::{sync::Mutex, time::Duration};
use tauri::{
    webview::{DownloadEvent, NewWindowResponse, PermissionResponse, WebviewBuilder},
    Manager,
};

const VIEW: &str = "workflows-service";

#[derive(Default)]
pub struct Workflows {
    // Serializes child creation/layout. Never holds the main toolbox Service lock.
    layout: Mutex<()>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Layout {
    visible: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    section: String,
    theme: String,
    navigation_id: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ViewState {
    ready: bool,
    section: Option<String>,
    locked: bool,
}

fn section_allowed(section: &str) -> bool {
    matches!(section, "library" | "drafts" | "queue" | "results")
}

impl Layout {
    fn validate(&self) -> Result<(), String> {
        if ![self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite() && *n >= 0. && *n <= 32_768.)
            || (self.visible && (self.width < 1. || self.height < 1.))
            || !section_allowed(&self.section)
            || !matches!(self.theme.as_str(), "dark" | "light")
            || self.navigation_id.len() > 80
            || !self
                .navigation_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("工作流显示参数无效".into());
        }
        Ok(())
    }
}

fn local_app_url(url: &tauri::Url) -> bool {
    let packaged = (url.scheme() == "http"
        && url.host_str() == Some("tauri.localhost")
        && url.port().is_none())
        || (url.scheme() == "tauri" && url.host_str() == Some("localhost"));
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(1420);
    (packaged || development) && url.username().is_empty() && url.password().is_none()
}

pub(crate) fn trusted_caller(view: &tauri::Webview) -> bool {
    view.label() == "main" && view.url().is_ok_and(|url| local_app_url(&url))
}

fn service_origin(url: &tauri::Url) -> bool {
    url.origin() == crate::service::Service::workflow_url().origin()
        && url.username().is_empty()
        && url.password().is_none()
}

fn allowed_navigation(url: &tauri::Url) -> bool {
    service_origin(url) && (url.path().starts_with("/workflows/") || allowed_download(url))
}

fn allowed_download(url: &tauri::Url) -> bool {
    if !service_origin(url) {
        return false;
    }
    let path = url.path();
    (path.starts_with("/api/workflows/results/") && url.query() == Some("download=1"))
        || (path.starts_with("/api/workflows/templates/")
            && path.ends_with("/share")
            && url.query().is_none())
}

impl Workflows {
    fn sync(
        &self,
        app: &tauri::AppHandle,
        layout: &Layout,
    ) -> Result<Option<tauri::Webview>, String> {
        layout.validate()?;
        let _guard = self.layout.lock().map_err(|_| "工作流窗口暂不可用")?;
        let mut view = app.get_webview(VIEW);
        if !layout.visible {
            if let Some(view) = &view {
                view.hide().map_err(|e| e.to_string())?;
            }
            return Ok(None);
        }
        if view.is_none() {
            let window = app.get_window("main").ok_or("找不到主窗口")?;
            let builder = WebviewBuilder::new(
                VIEW,
                tauri::WebviewUrl::External(crate::service::Service::workflow_url()),
            )
            .focused(false)
            .disable_drag_drop_handler()
            .initialization_script(include_str!("workflows-ui.js"))
            .on_navigation(allowed_navigation)
            .on_new_window(|_, _| NewWindowResponse::Deny)
            .on_permission_request(|_, _| PermissionResponse::Deny)
            .on_download(|view, event| match event {
                DownloadEvent::Requested { url, destination } => {
                    if !allowed_download(&url) { return false; }
                    let Ok(downloads) = view.app_handle().path().download_dir() else { return false; };
                    let directory = downloads.join("CYAPI-workflows");
                    if crate::configuration::no_links(&directory).is_err()
                        || std::fs::create_dir_all(&directory).is_err() { return false; }
                    // Do not trust a server-supplied filename or overwrite an
                    // existing file. Result format is PNG; shares are JSON.
                    let extension = if url.path().ends_with("/share") { "json" } else { "png" };
                    *destination = directory.join(format!("workflow-{}.{}", uuid::Uuid::new_v4(), extension));
                    true
                }
                DownloadEvent::Finished { success, .. } => {
                    let message = if success { "已保存到系统下载目录 / CYAPI-workflows" } else { "下载失败，请在结果页重新下载" };
                    let text = serde_json::to_string(message).unwrap();
                    let _ = view.eval(format!("(() => {{ const el=document.querySelector('#toast'); if(el) {{el.textContent={text};el.hidden=false;setTimeout(()=>el.hidden=true,6000);}} }})()"));
                    true
                }
                _ => true,
            });
            view = Some(
                window
                    .add_child(
                        builder,
                        tauri::LogicalPosition::new(layout.x, layout.y),
                        tauri::LogicalSize::new(layout.width, layout.height),
                    )
                    .map_err(|e| e.to_string())?,
            );
        }
        let view = view.unwrap();
        view.set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(layout.x, layout.y).into(),
            size: tauri::LogicalSize::new(layout.width, layout.height).into(),
        })
        .map_err(|e| e.to_string())?;
        view.show().map_err(|e| e.to_string())?;
        Ok(Some(view))
    }
}

#[tauri::command]
pub async fn reload_workflows(
    app: tauri::AppHandle,
    webview: tauri::Webview,
) -> Result<(), String> {
    if !trusted_caller(&webview) {
        return Err("远程页面没有本机工作流权限".into());
    }
    if let Some(view) = app.get_webview(VIEW) {
        view.navigate(crate::service::Service::workflow_url())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn workflows(
    layout: Layout,
    app: tauri::AppHandle,
    webview: tauri::Webview,
) -> Result<serde_json::Value, String> {
    if !trusted_caller(&webview) {
        return Err("远程页面没有本机工作流权限".into());
    }
    let next = layout.clone();
    // add_child on Windows must not run inside a synchronous UI command.
    let view =
        tauri::async_runtime::spawn_blocking(move || app.state::<Workflows>().sync(&app, &next))
            .await
            .map_err(|_| "工作流窗口操作未完成")??;
    let mut state = ViewState::default();
    if let Some(view) = view {
        let message = serde_json::json!({
            "section": layout.section, "theme": layout.theme, "navigationId": layout.navigation_id
        });
        // The remote adapter returns only three validated UI fields. No Key,
        // Cookie, paths, page HTML or arbitrary command is read from the page.
        let script = format!("(() => window.__cyapiWorkflowUi?.sync({message}) ?? {{ready:false,section:null,locked:false}})()");
        let (send, receive) = tokio::sync::oneshot::channel();
        let send = Mutex::new(Some(send));
        view.eval_with_callback(script, move |result| {
            if let Ok(mut send) = send.lock() {
                if let Some(send) = send.take() {
                    let _ = send.send(result);
                }
            }
        })
        .map_err(|e| e.to_string())?;
        if let Ok(Ok(result)) = tokio::time::timeout(Duration::from_secs(2), receive).await {
            if result.len() < 512 {
                if let Ok(next) = serde_json::from_str::<ViewState>(&result) {
                    if next.section.as_deref().is_none_or(section_allowed) {
                        state = next;
                    }
                }
            }
        }
    }
    Ok(serde_json::json!({ "url": crate::service::Service::workflow_url(), "view": state }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_origin_and_routes_are_pinned() {
        let url = crate::service::Service::workflow_url();
        assert_eq!(url.as_str(), "https://api.cyapi.cyou/workflows/");
        for candidate in [
            "http://api.cyapi.cyou/workflows/",
            "https://example.com/workflows/",
            "http://127.0.0.1:8188/",
            "https://api.cyapi.cyou/v1/usage",
            "https://key@api.cyapi.cyou/workflows/",
        ] {
            assert!(
                !allowed_navigation(&candidate.parse().unwrap()),
                "{candidate}"
            );
        }
        assert!(allowed_navigation(&url));
        assert!(allowed_navigation(&url.join("canvas/").unwrap()));
    }

    #[test]
    fn downloads_are_only_service_results_and_shares() {
        for candidate in [
            "https://api.cyapi.cyou/api/workflows/results/id?download=1",
            "https://api.cyapi.cyou/api/workflows/templates/id/share",
        ] {
            assert!(allowed_download(&candidate.parse().unwrap()));
        }
        for candidate in [
            "https://example.com/api/workflows/results/id?download=1",
            "https://api.cyapi.cyou/api/workflows/results/id",
            "https://api.cyapi.cyou/releases/app.exe",
            "file:///C:/Windows/win.ini",
        ] {
            assert!(!allowed_download(&candidate.parse().unwrap()));
        }
    }

    #[test]
    fn remote_and_other_local_pages_are_not_privileged() {
        for candidate in [
            "https://api.cyapi.cyou/workflows/",
            "http://tauri.localhost.evil/",
            "https://tauri.localhost/",
            "http://key@tauri.localhost/",
            "file:///C:/index.html",
        ] {
            assert!(!local_app_url(&candidate.parse().unwrap()));
        }
        assert!(local_app_url(&"http://tauri.localhost/".parse().unwrap()));
    }

    #[test]
    fn layout_rejects_injection_invalid_numbers_and_unknown_fields() {
        let valid = serde_json::json!({"visible":true,"x":216,"y":64,"width":1100,"height":760,"section":"library","theme":"light","navigationId":"nav-1"});
        assert!(serde_json::from_value::<Layout>(valid.clone())
            .unwrap()
            .validate()
            .is_ok());
        for (key, value) in [
            ("section", serde_json::json!("editor")),
            ("theme", serde_json::json!("');alert(1)//")),
            ("width", serde_json::json!(-1)),
            ("navigationId", serde_json::json!("<script>")),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(serde_json::from_value::<Layout>(invalid)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut invalid = valid;
        invalid["command"] = serde_json::json!("toolbox");
        assert!(serde_json::from_value::<Layout>(invalid).is_err());
    }
}
