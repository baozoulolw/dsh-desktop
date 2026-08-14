// 移植自 Electron 版 main.js:子进程管理、版本检测、升级。
//
// 关键差异(相对 Electron 版):
//  - dsh 引擎不再随应用打包,而是装在"每用户 runtime 目录"(app_data_dir/dsh-runtime),
//    首次启动用系统 npm 安装,升级就在该目录 npm install 新版本。
//  - 用系统 Node 运行 dsh(dsh 的原生模块 node-pty/koffi 按本机 Node 编译,
//    必须用系统 node 而非内置运行时,否则 ABI 不匹配)。
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

/// dsh 默认版本:首次在 runtime 目录安装时使用。
const DEFAULT_DSH_VERSION: &str = "0.1.0-rc.6";
/// dsh web 启动后打印的地址,如 `dsh web: http://127.0.0.1:50721`
const URL_RE: &str = r"dsh web: (http://\d+\.\d+\.\d+\.\d+:\d+)";
const STARTUP_TIMEOUT_MS: u64 = 30_000;
const HTTP_TIMEOUT_MS: u64 = 10_000;
/// 引擎(WebView 运行时)交互界面上展示的版本。
const ENGINE_VERSION: &str = "2";

/// 全局状态:持有 dsh 子进程与最近一次拿到的 URL。
pub struct DshState {
    child: Mutex<Option<Child>>,
    url: Mutex<Option<String>>,
}

impl Default for DshState {
    fn default() -> Self {
        Self {
            child: Mutex::new(None),
            url: Mutex::new(None),
        }
    }
}

// ---------- 路径与工具 ----------

/// dsh 引擎所在的"每用户 runtime 目录"。
fn runtime_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    Ok(data.join("dsh-runtime"))
}

/// dsh web 启动时的工作目录:用户主目录,避免污染应用目录。
fn workspace_home() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/".to_string())
}

/// 解析系统 node 绝对路径(移植 Electron 版 resolveNodeBin)。
fn resolve_node() -> Result<String, String> {
    if let Ok(p) = std::env::var("npm_node_execpath") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    if let Ok(p) = std::env::var("NODE") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates = vec![
        "/opt/homebrew/bin/node".into(), // Apple Silicon Homebrew
        "/usr/local/bin/node".into(),    // Intel Homebrew / 全局
        "/usr/bin/node".into(),          // 系统自带旧 node
    ];
    // nvm 各版本,从新到旧
    let nvm_dir = Path::new(&home).join(".nvm").join("versions").join("node");
    if let Ok(entries) = std::fs::read_dir(&nvm_dir) {
        let mut vers: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        vers.sort();
        vers.reverse();
        for v in vers {
            candidates.push(nvm_dir.join(&v).join("bin").join("node").to_string_lossy().to_string());
        }
    }
    for p in &candidates {
        if Path::new(p).exists() {
            return Ok(p.clone());
        }
    }
    Ok("node".to_string()) // 最终回退:依赖 PATH
}

/// 系统 Node 是否可用。resolve_node 能解析到确定路径时直接判存;回退到 PATH("node")
/// 时用 `node --version` 实测,避免 PATH 里其实没有 node 却误判可用。
fn node_available() -> bool {
    let Ok(node) = resolve_node() else {
        return false;
    };
    if node == "node" {
        return Command::new("node")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    Path::new(&node).exists()
}

/// 取 node 同目录的 npm 可执行文件(npm 随 node 一起分发)。
fn resolve_npm(node: &str) -> String {
    if let Some(dir) = Path::new(node).parent() {
        let p = dir.join("npm");
        if p.exists() {
            return p.to_string_lossy().to_string();
        }
    }
    "npm".to_string()
}

/// 读 runtime 目录里已安装的 dsh 版本,未装则返回"未知"。
fn read_dsh_version(runtime: &Path) -> String {
    let pkg = runtime.join("node_modules").join("@deepseek-ai").join("dsh").join("package.json");
    std::fs::read_to_string(&pkg)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("version").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_else(|| "未知".to_string())
}

/// 轻量语义化版本比较:a > b 返回 true,忽略 v/V 前缀,按 major.minor.patch 逐段比较。
fn semver_gt(a: &str, b: &str) -> bool {
    let nums = |s: &str| {
        s.trim_start_matches(|c| c == 'v' || c == 'V')
            .split('.')
            .take(3)
            .map(|n| n.parse::<i32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let pa = nums(a);
    let pb = nums(b);
    for i in 0..3 {
        if pa[i] != pb[i] {
            return pa[i] > pb[i];
        }
    }
    false
}

// ---------- dsh 引擎安装与启动 ----------

/// dsh 引擎可执行文件路径。
fn dsh_bin_path(runtime: &Path) -> PathBuf {
    runtime.join("node_modules").join("@deepseek-ai").join("dsh").join("lib").join("bin.js")
}

/// 引擎是否已安装(runtime 目录里存在 bin.js)。
fn dsh_installed(runtime: &Path) -> bool {
    dsh_bin_path(runtime).exists()
}

/// 真正拉起 dsh 子进程并等其打印 URL。
/// 阻塞至拿到 URL(由调用方负责包一层,避免卡 UI)。调用前需确保引擎已安装。
async fn start_dsh_inner(app: &tauri::AppHandle, state: &State<'_, DshState>) -> Result<String, String> {
    // 先终止可能残留的旧进程
    if let Some(mut old) = state.child.lock().unwrap().take() {
        let _ = old.kill();
    }
    let runtime = runtime_dir(app)?;
    let node = resolve_node()?;
    let dsh_bin = dsh_bin_path(&runtime);
    if !dsh_bin.exists() {
        return Err("dsh 未安装".to_string());
    }

    let mut child = Command::new(&node)
        .arg(&dsh_bin)
        .arg("--profile")
        .arg("web")
        .arg("--port")
        .arg("0")
        .current_dir(workspace_home())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 dsh 失败: {e}"))?;

    let re = Regex::new(URL_RE).unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let out = child.stdout.take().ok_or("无法读取 dsh stdout")?;
    let err = child.stderr.take().ok_or("无法读取 dsh stderr")?;
    // 两个线程分别读 stdout/stderr,谁先打印 URL 谁胜出。
    // 进程持续运行时线程会阻塞在读上,进程被杀后随管道关闭而结束。
    let re_out = re.clone();
    let tx_out = tx.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(out).lines().flatten() {
            if let Some(c) = re_out.captures(&line) {
                let _ = tx_out.send(c[1].to_string());
                break;
            }
        }
    });
    let re_err = re;
    let tx_err = tx;
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(err).lines().flatten() {
            if let Some(c) = re_err.captures(&line) {
                let _ = tx_err.send(c[1].to_string());
                break;
            }
        }
    });

    *state.child.lock().unwrap() = Some(child);
    match rx.recv_timeout(Duration::from_millis(STARTUP_TIMEOUT_MS)) {
        Ok(url) => {
            *state.url.lock().unwrap() = Some(url.clone());
            Ok(url)
        }
        Err(_) => {
            if let Some(mut c) = state.child.lock().unwrap().take() {
                let _ = c.kill();
            }
            Err("启动超时: 30 秒内未获取到 dsh web 地址".to_string())
        }
    }
}

// ---------- 命令 ----------

#[derive(Serialize)]
struct VersionInfo {
    app_version: String,
    engine_version: String,
    dsh_version: String,
    platform: String,
}

#[derive(Serialize)]
struct UpdateInfo {
    current: String,
    latest: String,
    is_outdated: bool,
}

#[derive(Serialize)]
struct AppUpdateInfo {
    ok: bool,
    current: String,
    latest: String,
    is_outdated: bool,
    url: String,
    error: Option<String>,
}

/// 启动 dsh 的结果:成功返回地址;失败则区分原因,供前端给出针对性提示。
#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
enum DshStartup {
    Ready { url: String },
    /// 系统缺少 Node.js,无法安装/运行 dsh 引擎。
    NodeMissing,
    /// 引擎尚未安装,需要先点"安装"。
    NotInstalled,
    /// dsh 已启动但未在时限内就绪(超时/进程退出)。
    StartupFailed { message: String },
}

/// 前端启动时调用:确保引擎已就绪并拿到地址,返回 dsh web URL。
/// 已有则直接返回缓存 URL;未安装则告知前端"需要先安装",由用户触发安装,不再自动装。
#[tauri::command]
async fn get_dsh_url(app: tauri::AppHandle, state: State<'_, DshState>) -> Result<DshStartup, String> {
    if !node_available() {
        return Ok(DshStartup::NodeMissing);
    }
    if !dsh_installed(&runtime_dir(&app)?) {
        return Ok(DshStartup::NotInstalled);
    }
    let running = {
        let mut g = state.child.lock().unwrap();
        match g.as_mut() {
            Some(c) => c.try_wait().map(|s| s.is_none()).unwrap_or(false),
            None => false,
        }
    };
    if running {
        if let Some(u) = state.url.lock().unwrap().clone() {
            return Ok(DshStartup::Ready { url: u });
        }
    }
    match start_dsh_inner(&app, &state).await {
        Ok(url) => Ok(DshStartup::Ready { url }),
        Err(message) => Ok(DshStartup::StartupFailed { message }),
    }
}

#[tauri::command]
async fn get_version_info(app: tauri::AppHandle) -> VersionInfo {
    let runtime = runtime_dir(&app).unwrap_or_default();
    VersionInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_version: ENGINE_VERSION.to_string(),
        dsh_version: read_dsh_version(&runtime),
        platform: std::env::consts::OS.to_string(),
    }
}

/// 查 npmmirror registry 拿 dsh 最新版。
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current = read_dsh_version(&runtime_dir(&app).unwrap_or_default());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
        .build()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = client
        .get("https://registry.npmmirror.com/@deepseek-ai/dsh")
        .send()
        .await
        .map_err(|e| format!("查询 dsh 版本失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("解析 dsh 版本失败: {e}"))?;
    let latest = v
        .get("dist-tags")
        .and_then(|d| d.get("latest"))
        .and_then(|l| l.as_str())
        .unwrap_or("未知")
        .to_string();
    // 引擎未安装(current=="未知")时不算"过时",避免顶栏误显示"升级到 X"。
    let is_outdated = current != "未知" && latest != "未知" && latest != current;
    Ok(UpdateInfo {
        current,
        latest,
        is_outdated,
    })
}

/// 查 GitHub API 拿应用(DeepSeek Harness 本体)最新 Release。
#[tauri::command]
async fn check_app_update() -> AppUpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return AppUpdateInfo {
                ok: false,
                current,
                latest: "".into(),
                is_outdated: false,
                url: "".into(),
                error: Some(e.to_string()),
            }
        }
    };
    let resp = match client
        .get("https://api.github.com/repos/baozoulolw/dsh-desktop/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "dsh-desktop")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return AppUpdateInfo {
                ok: false,
                current,
                latest: "".into(),
                is_outdated: false,
                url: "".into(),
                error: Some(e.to_string()),
            }
        }
    };
    let v: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return AppUpdateInfo {
                ok: false,
                current,
                latest: "".into(),
                is_outdated: false,
                url: "".into(),
                error: Some(e.to_string()),
            }
        }
    };
    let tag = v.get("tag_name").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let latest = tag.trim_start_matches(|c| c == 'v' || c == 'V').to_string();
    let url = v.get("html_url").and_then(|u| u.as_str()).unwrap_or("").to_string();
    let is_outdated = !latest.is_empty() && semver_gt(&latest, &current);
    AppUpdateInfo {
        ok: true,
        current,
        latest,
        is_outdated,
        url,
        error: None,
    }
}

/// 打开外部链接(交系统浏览器)。
#[tauri::command]
async fn open_external(app: tauri::AppHandle, url: String) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// 升级 dsh 到指定版本:在 runtime 目录 npm install,流式回传进度,完成后重启 dsh。
#[tauri::command]
async fn upgrade_dsh(
    app: tauri::AppHandle,
    state: State<'_, DshState>,
    target_version: String,
) -> Result<VersionInfo, String> {
    if let Some(mut c) = state.child.lock().unwrap().take() {
        let _ = c.kill();
    }
    let runtime = runtime_dir(&app)?;
    let node = resolve_node()?;
    let npm = resolve_npm(&node);

    let _ = app.emit("upgrade-progress", serde_json::json!({ "phase": "开始升级…", "done": false }));

    let mut cmd = Command::new(&npm)
        .arg("install")
        .arg("--prefix")
        .arg(&runtime)
        .arg(format!("@deepseek-ai/dsh@{target_version}"))
        .arg("--loglevel=info") // 非终端下默认只打警告;降到 info 让下载/安装过程可见
        .current_dir(&runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 npm install 失败: {e}"))?;

    let out = cmd.stdout.take().ok_or("无法读取 npm stdout")?;
    let err = cmd.stderr.take().ok_or("无法读取 npm stderr")?;
    let app2 = app.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(out).lines().flatten() {
            let _ = app2.emit("upgrade-progress", serde_json::json!({ "phase": line, "done": false }));
        }
    });
    let app3 = app.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(err).lines().flatten() {
            let _ = app3.emit("upgrade-progress", serde_json::json!({ "phase": line, "done": false }));
        }
    });

    let status = cmd.wait().map_err(|e| format!("等待 npm install 失败: {e}"))?;
    if !status.success() {
        return Err(format!("升级失败 (exit = {status})"));
    }

    // 升级完成后重启 dsh
    let url = start_dsh_inner(&app, &state).await?;
    let _ = app.emit("dsh-url-updated", url);
    let _ = app.emit("upgrade-progress", serde_json::json!({ "phase": "升级完成, dsh 已重启", "done": true }));

    Ok(VersionInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_version: ENGINE_VERSION.to_string(),
        dsh_version: read_dsh_version(&runtime),
        platform: std::env::consts::OS.to_string(),
    })
}

/// 手动安装 dsh 引擎:在 runtime 目录 npm install 默认版本,实时回传进度,完成后不重启,
/// 由前端收到 done 事件后重新调用 boot 拉起引擎。
#[tauri::command]
async fn install_dsh(app: tauri::AppHandle) -> Result<VersionInfo, String> {
    if !node_available() {
        return Err("未检测到 Node.js,无法安装 dsh 引擎。请先前往 nodejs.org 安装 Node.js。".to_string());
    }
    let runtime = runtime_dir(&app)?;
    std::fs::create_dir_all(&runtime).map_err(|e| format!("创建 runtime 目录失败: {e}"))?;
    let pkg = runtime.join("package.json");
    if !pkg.exists() {
        std::fs::write(&pkg, "{\"private\":true}").map_err(|e| format!("写 package.json 失败: {e}"))?;
    }
    let node = resolve_node()?;
    let npm = resolve_npm(&node);

    let _ = app.emit("install-progress", serde_json::json!({ "phase": "准备安装 dsh 引擎…", "done": false }));

    let mut cmd = Command::new(&npm)
        .arg("install")
        .arg("--prefix")
        .arg(&runtime)
        .arg(format!("@deepseek-ai/dsh@{DEFAULT_DSH_VERSION}"))
        .arg("--loglevel=info") // 非终端下默认只打警告;降到 info 让下载/安装过程可见
        .current_dir(&runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 npm install 失败: {e}"))?;

    let out = cmd.stdout.take().ok_or("无法读取 npm stdout")?;
    let err = cmd.stderr.take().ok_or("无法读取 npm stderr")?;
    let app2 = app.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(out).lines().flatten() {
            let _ = app2.emit("install-progress", serde_json::json!({ "phase": line, "done": false }));
        }
    });
    let app3 = app.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(err).lines().flatten() {
            let _ = app3.emit("install-progress", serde_json::json!({ "phase": line, "done": false }));
        }
    });

    let status = cmd.wait().map_err(|e| format!("等待 npm install 失败: {e}"))?;
    if !status.success() {
        return Err(format!("安装失败 (exit = {status})"));
    }

    let _ = app.emit("install-progress", serde_json::json!({ "phase": "安装完成", "done": true }));
    Ok(VersionInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_version: ENGINE_VERSION.to_string(),
        dsh_version: read_dsh_version(&runtime),
        platform: std::env::consts::OS.to_string(),
    })
}

// ---------- 入口 ----------

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(DshState::default())
        .invoke_handler(tauri::generate_handler![
            get_dsh_url,
            get_version_info,
            check_update,
            check_app_update,
            open_external,
            upgrade_dsh,
            install_dsh
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // 退出时终止 dsh 子进程,避免残留
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<DshState>() {
                    if let Some(mut c) = state.child.lock().unwrap().take() {
                        let _ = c.kill();
                    }
                }
            }
        });
}