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
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

/// dsh 默认版本:首次在 runtime 目录安装时使用。
const DEFAULT_DSH_VERSION: &str = "0.1.0-rc.7";
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

/// 为子进程构造包含 node/npm 所在目录的 PATH。
/// GUI(访达/程序坞)启动应用时,子进程 PATH 往往极简(不含 Homebrew/nvm 目录)。
/// npm 是 `#!/usr/bin/env node` 的脚本,会回退到 PATH 找 node;PATH 里没有就报
/// `env: node: No such file or directory` 并以 127 退出。把 node/npm 目录并到 PATH 最前即可。
fn path_with_node(node: &str, npm: &str) -> String {
    let mut dirs: Vec<String> = Vec::new();
    for p in [node, npm] {
        if let Some(dir) = Path::new(p).parent() {
            if let Some(s) = dir.to_str() {
                if !dirs.iter().any(|d| d == s) {
                    dirs.push(s.to_string());
                }
            }
        }
    }
    let mut path = dirs.join(":");
    if let Ok(existing) = std::env::var("PATH") {
        if !existing.trim().is_empty() {
            if !path.is_empty() {
                path.push(':');
            }
            path.push_str(&existing);
        }
    }
    path
}

// ---------- 复用本机已装的 dsh 引擎 ----------

/// 引擎来源,用于面板展示与打开安装位置。
#[derive(Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Debug)]
enum EngineSource {
    /// 全局 npm 安装(`npm i -g @deepseek-ai/dsh`),复用,不重复装。
    Global,
    /// 本应用私有 runtime 目录里的那一份。
    App,
    /// 经 npx 可用(`npx @deepseek-ai/dsh`,用户本机常用),没有其它已装引擎时兜底复用。
    Npx,
}

/// 一个已定位到磁盘的 dsh 引擎。
struct Engine {
    /// 展示用的"安装位置"目录(面板/在 Finder 打开用)。
    dir: PathBuf,
    source: EngineSource,
}

impl Engine {
    /// 实际入口 `lib/bin.js`。Npx 走 `npx` 命令,没有固定 bin。
    fn bin(&self) -> PathBuf {
        match self.source {
            EngineSource::Global => self.dir.join("lib").join("bin.js"),
            EngineSource::App => dsh_bin_path(&self.dir),
            EngineSource::Npx => PathBuf::new(),
        }
    }

    fn version(&self) -> String {
        match self.source {
            EngineSource::Global => read_pkg_version(&self.dir.join("package.json")),
            EngineSource::App => read_pkg_version(
                &self.dir.join("node_modules").join("@deepseek-ai").join("dsh").join("package.json"),
            ),
            // npx 引擎目录若直接带 package.json(找到真实缓存包)就读它,否则按启动所钉版本。
            EngineSource::Npx => {
                if self.dir.join("package.json").exists() {
                    read_pkg_version(&self.dir.join("package.json"))
                } else {
                    DEFAULT_DSH_VERSION.to_string()
                }
            }
        }
    }
}

/// 读某个 package.json 的 version。
fn read_pkg_version(pkg: &Path) -> String {
    std::fs::read_to_string(pkg)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("version").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_else(|| "未知".to_string())
}

/// 由解析出的 node 绝对路径推出"全局 npm 安装"的 `@deepseek-ai/dsh` 目录。
/// 路径模式与 node 安装方式一致:nvm `~/.nvm/.../vXX/lib/node_modules/@deepseek-ai/dsh`、
/// Homebrew `/opt/homebrew/lib/node_modules/@deepseek-ai/dsh` 都是 node_dir 上两级的 lib/node_modules。
fn resolve_global_dsh(node: &str) -> Option<PathBuf> {
    let node_path = Path::new(node);
    let pkg = node_path
        .parent()?
        .parent()?
        .join("lib")
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh");
    if pkg.join("lib").join("bin.js").exists() {
        Some(pkg)
    } else {
        None
    }
}

/// 取 node 同目录的 npx 可执行文件(npx 随 npm/node 一起分发)。
fn resolve_npx(node: &str) -> String {
    if let Some(dir) = Path::new(node).parent() {
        let p = dir.join("npx");
        if p.exists() {
            return p.to_string_lossy().to_string();
        }
    }
    "npx".to_string()
}

/// 本机曾用 `npx @deepseek-ai/dsh` 跑过时,包落在 `~/.npm/_npx/<hash>/node_modules/@deepseek-ai/dsh`。
/// 找到真实缓存包目录则返回它;否则退回 `~/.npm/_npx` 根目录;再否则 None。
fn resolve_npx_engine_dir(home: &std::path::Path) -> Option<PathBuf> {
    let npx_root = home.join(".npm").join("_npx");
    if let Ok(entries) = std::fs::read_dir(&npx_root) {
        for e in entries.flatten() {
            let pkg = e.path().join("node_modules").join("@deepseek-ai").join("dsh");
            if pkg.join("package.json").exists() {
                return Some(pkg);
            }
        }
    }
    if npx_root.exists() {
        Some(npx_root)
    } else {
        None
    }
}

/// 选择实际使用的引擎:优先全局 npm 安装,其次本应用私有 runtime,最后 npx 兜底。均无则 None。
fn pick_engine(app: &tauri::AppHandle, node: &str) -> Option<Engine> {
    if let Some(global) = resolve_global_dsh(node) {
        let bin = global.join("lib").join("bin.js");
        if bin.exists() {
            return Some(Engine { dir: global, source: EngineSource::Global });
        }
    }
    let runtime = runtime_dir(app).ok()?;
    if dsh_bin_path(&runtime).exists() {
        return Some(Engine { dir: runtime, source: EngineSource::App });
    }
    // npx 兜底:只有 node 同目录确实有 npx 才视为可用(避免误把 PATH 里的 npx 当准)。
    let npx = resolve_npx(node);
    if Path::new(&npx).exists() {
        let home = std::env::var("HOME").unwrap_or_default();
        let dir = resolve_npx_engine_dir(std::path::Path::new(&home)).unwrap_or_else(|| {
            std::path::Path::new(&npx)
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from(home))
        });
        return Some(Engine { dir, source: EngineSource::Npx });
    }
    None
}

/// 当前应使用的引擎(node 解析失败时退化为仅看私有 runtime)。
fn active_engine(app: &tauri::AppHandle) -> Option<Engine> {
    resolve_node().ok().and_then(|n| pick_engine(app, &n))
}

// ---- 在跑实例的持久化与探活(dsh 用随机端口且不留状态文件,靠自记 {pid,url}) ----

/// dsh-live.json 内容:最近一次由本应用启动且在跳出应用后仍存活的 web 实例。
/// 探活靠 url 发请求;pid 供"重启服务"终结跨会话孤儿实例用。
#[derive(Deserialize)]
struct LiveFile {
    pid: u32,
    url: String,
}

/// 应用数据目录下记录在跑 dsh 实例的文件。
fn live_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    Ok(data.join("dsh-live.json"))
}

/// 启动 dsh 成功后记录 pid + url(自动确保数据目录存在,避免首次 npx 模式下目录未创建而写失败)。
fn persist_live(app: &tauri::AppHandle, pid: u32, url: &str) {
    let Ok(data) = app.path().app_data_dir() else { return };
    if std::fs::create_dir_all(&data).is_err() {
        return;
    }
    let path = data.join("dsh-live.json");
    let _ = std::fs::write(&path, serde_json::json!({ "pid": pid, "url": url }).to_string());
}

/// 终止跨会话持久化的 live 实例:读 dsh-live.json 拿 pid 并 kill,随后删文件。
/// 用于"重启服务"——把上一会话留下的孤儿 dsh 也一并结束,再拉起全新的。
/// 注意:dsh-live.json 只记录最近一次实例的 pid,且 npx 模式下它是 npx 的包装进程 pid,
/// 而非真实 `node .../.bin/dsh` 服务器——所以光靠它杀不干净,restart 还必须调用 kill_all_dsh。
fn kill_live_instance(app: &tauri::AppHandle) {
    let Ok(path) = live_file(app) else { return };
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    if let Ok(rec) = serde_json::from_str::<LiveFile>(&raw) {
        let _ = Command::new("kill").arg(rec.pid.to_string()).status();
    }
    let _ = std::fs::remove_file(&path);
}

/// 终止所有在跑的 dsh web 服务器进程(含历史会话遗留的孤儿、npx 间接启动的真实 node 进程)。
///
/// 为什么必须"全杀":dsh 的 task-board ledger 是单所有权锁(`~/.dsh/task-board/ledger-v2.lock`),
/// 只要有一个残留 dsh 占着它,新起的实例就会在插件加载时报
/// `task-board ledger is already owned by process <pid>` 并退出,stdout 永远打不出
/// `dsh web: http://...`,于是 restart 只能卡到 30s 超时。掉单 pid 永远清不掉这种孤儿,
/// 所以 restart 用这里按命令行(而非 pid)把匹配的 dsh 都结束。
///
/// 双模式匹配:
///   - npx/兜底: `node .../node_modules/.bin/dsh --profile web --port 0`(记在 live 里的其实是 npx 包装 pid)
///   - 全局/本应用: `node .../@deepseek-ai/dsh/lib/bin.js --profile web --port 0`
fn kill_all_dsh() {
    #[cfg(unix)]
    {
        let _ = Command::new("pkill").args(["-f", r"\.bin/dsh --profile web"]).status();
        let _ = Command::new("pkill").args(["-f", r"lib/bin\.js --profile web"]).status();
    }
    // Windows 无 pkill;直接子进程与记录 pid 已被 restart 另行 kill,这里先不强杀全部 node(会误伤)。
    #[cfg(windows)]
    {
        // TODO(windows): 若要根治 ledger 单所有权锁下的孤儿,需枚举命令行含 --profile web 的 node 进程再逐一 taskkill。
    }
}

/// 在清理旧进程完成后,轮询等待所有 `--profile web` 的 dsh 进程真正退出。
/// 目的:等占着 task-board ledger 锁的旧进程释放锁,避免新实例启动时仍撞上
/// `ledger is already owned by <旧pid>` 而插件加载失败。
#[cfg(unix)]
fn wait_dsh_terminated() {
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    while std::time::Instant::now() < deadline {
        let a = Command::new("pgrep")
            .args(["-f", r"\.bin/dsh --profile web"])
            .output();
        let b = Command::new("pgrep")
            .args(["-f", r"lib/bin\.js --profile web"])
            .output();
        let any = a.map(|o| !o.stdout.is_empty()).unwrap_or(false)
            || b.map(|o| !o.stdout.is_empty()).unwrap_or(false);
        if !any {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(not(unix))]
fn wait_dsh_terminated() {}

/// 探活已持久化的在跑实例:对记录的 url 做一次短超时 GET,G及时视为在跑返回 url,
/// 否则删除陈旧文件返回 None。这是"复用已有实例、不重复拉"的关键。
async fn resolve_running_url(app: &tauri::AppHandle) -> Option<String> {
    let path = live_file(app).ok()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let rec: LiveFile = serde_json::from_str(&raw).ok()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
        .ok()?;
    let alive = match client.get(&rec.url).send().await {
        Ok(resp) => resp.status().is_success() || resp.status().is_redirection(),
        Err(_) => false,
    };
    if alive {
        Some(rec.url)
    } else {
        let _ = std::fs::remove_file(&path);
        None
    }
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

/// 真正拉起 dsh 子进程并等其打印 URL。
/// 阻塞至拿到 URL(由调用方负责包一层,避免卡 UI)。调用前需确保引擎已可用。
async fn start_dsh_inner(
    app: &tauri::AppHandle,
    state: &State<'_, DshState>,
    engine: &Engine,
) -> Result<String, String> {
    // 先终止可能残留的旧进程
    if let Some(mut old) = state.child.lock().unwrap().take() {
        let _ = old.kill();
    }
    // 再清掉所有残留 dsh 服务器并等它们真正退出(见 kill_all_dsh:task-board ledger 单所有权锁,
    // 留一个就会让新实例加载插件失败 → 启动超时)。这里统一收口,无论哪条启动路径都先洗干净。
    kill_all_dsh();
    wait_dsh_terminated();
    let node = resolve_node()?;
    let npx = resolve_npx(&node);
    // 先拼出 (程序, 参数),再统一建 Command——避免在 match 分支里直接返回借用局部的 Command。
    let (program, args): (String, Vec<String>) = match engine.source {
        // 全局与本应用都用系统 node 直接跑 bin.js(原生模块按本机 node ABI 编译)。
        EngineSource::Global | EngineSource::App => {
            let bin = engine.bin();
            if !bin.exists() {
                return Err("dsh 未安装".to_string());
            }
            (
                node.clone(),
                vec![
                    bin.to_string_lossy().into_owned(),
                    "--profile".into(),
                    "web".into(),
                    "--port".into(),
                    "0".into(),
                ],
            )
        }
        // npx 兜底:经 npx 解析运行。
        EngineSource::Npx => (
            npx.clone(),
            vec![
                "--yes".into(),
                format!("@deepseek-ai/dsh@{DEFAULT_DSH_VERSION}"),
                "--profile".into(),
                "web".into(),
                "--port".into(),
                "0".into(),
            ],
        ),
    };
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    // npx 是 `#!/usr/bin/env node` 脚本,需把 node 目录并入 PATH。
    if matches!(engine.source, EngineSource::Npx) {
        cmd.env("PATH", path_with_node(&program, &npx));
    }
    cmd.current_dir(workspace_home())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("启动 dsh 失败: {e}"))?;
    let pid = child.id();

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
            // 记录在跑实例(pid + url),供下次启动探活复用,若应用退出后仍存活则直接重连。
            persist_live(app, pid, &url);
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
    /// 引擎来源: "global"(全局 npm)/ "app"(本应用)/ "npx"(npx 兜底)/ "none"。
    engine_source: String,
    /// 引擎安装位置:全局/本应用时是目录路径;在跑实例时是它的 url;无引擎时为空。
    engine_address: String,
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
/// 优先级:①复用已在跑实例→ ②本进程内已启动且在跑→ ③定位引擎(优先全局,否则私有 runtime)并启动。
/// 未安装则告知前端"需要先安装",由用户触发安装,不再自动装。
#[tauri::command]
async fn get_dsh_url(app: tauri::AppHandle, state: State<'_, DshState>) -> Result<DshStartup, String> {
    if !node_available() {
        return Ok(DshStartup::NodeMissing);
    }
    // ① 探活并复用跨会话仍在跑的实例(应用退出后 dsh 仍存活时,不再重拉)。
    if let Some(url) = resolve_running_url(&app).await {
        *state.url.lock().unwrap() = Some(url.clone());
        return Ok(DshStartup::Ready { url });
    }
    // ② 本进程内已启动过且在跑,直接返回缓存 URL。
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
    // ③ 定位引擎并启动一个 dsh web。
    let node = match resolve_node() {
        Ok(n) => n,
        Err(e) => return Ok(DshStartup::StartupFailed { message: e }),
    };
    let engine = match pick_engine(&app, &node) {
        Some(e) => e,
        None => return Ok(DshStartup::NotInstalled),
    };
    match start_dsh_inner(&app, &state, &engine).await {
        Ok(url) => Ok(DshStartup::Ready { url }),
        Err(message) => Ok(DshStartup::StartupFailed { message }),
    }
}

/// 组装版本信息:定位引擎,给出 dsh 版本、来源与安装位置(本机目录)。
async fn build_version_info(app: &tauri::AppHandle) -> VersionInfo {
    let engine = active_engine(app);
    let (source, address, ver) = match &engine {
        Some(e) => {
            let source = match e.source {
                EngineSource::Global => "global",
                EngineSource::App => "app",
                EngineSource::Npx => "npx",
            };
            (source.to_string(), e.dir.display().to_string(), e.version())
        }
        None => ("none".to_string(), String::new(), "未知".to_string()),
    };
    VersionInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_version: ENGINE_VERSION.to_string(),
        dsh_version: ver,
        platform: std::env::consts::OS.to_string(),
        engine_source: source,
        engine_address: address,
    }
}

#[tauri::command]
async fn get_version_info(app: tauri::AppHandle) -> VersionInfo {
    build_version_info(&app).await
}

/// 查 npmmirror registry 拿 dsh 最新版。
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    // 对"当前版本"的判断用实际使用的引擎(优先全局 npm),让升级状态贴合真实运行。
    let current = active_engine(&app)
        .map(|e| e.version())
        .unwrap_or_else(|| "未知".to_string());
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

/// "快捷跳转":打开本机安装位置 —— 在文件管理器(Finder / 资源管理器)中显示引擎目录,而非浏览器。
/// 未安装时则跳到官方安装页。
#[tauri::command]
async fn reveal_engine(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(engine) = active_engine(&app) {
        return app.opener()
            .open_url(format!("file://{}", engine.dir.display()), None::<&str>)
            .map_err(|e| e.to_string());
    }
    app.opener()
        .open_url("https://www.npmjs.com/package/@deepseek-ai/dsh".to_string(), None::<&str>)
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
        .env("PATH", path_with_node(&node, &npm))
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

    // 升级完成后重启 dsh(仍按 active 引擎重启,含全局/npx 兜底)。
    let engine = active_engine(&app)
        .unwrap_or_else(|| Engine { dir: runtime.clone(), source: EngineSource::App });
    let url = start_dsh_inner(&app, &state, &engine).await?;
    let _ = app.emit("dsh-url-updated", url);
    let _ = app.emit("upgrade-progress", serde_json::json!({ "phase": "升级完成, dsh 已重启", "done": true }));

    Ok(build_version_info(&app).await)
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
        .env("PATH", path_with_node(&node, &npm))
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
    Ok(build_version_info(&app).await)
}

/// 重启 dsh 服务:终止在跑实例(含跨会话持久化的孤儿实例)后重新拉起一个全新的。
/// 与 get_dsh_url 不同:强制重启,不复用已有实例。
#[tauri::command]
async fn restart_dsh(app: tauri::AppHandle, state: State<'_, DshState>) -> Result<DshStartup, String> {
    // 先杀掉当前会话直管子进程、并移除 dsh-live.json(旧实例的 pid/url 不再有效)。
    if let Some(mut c) = state.child.lock().unwrap().take() {
        let _ = c.kill();
    }
    kill_live_instance(&app);
    if !node_available() {
        return Ok(DshStartup::NodeMissing);
    }
    let engine = match active_engine(&app) {
        Some(e) => e,
        None => return Ok(DshStartup::NotInstalled),
    };
    match start_dsh_inner(&app, &state, &engine).await {
        Ok(url) => Ok(DshStartup::Ready { url }),
        Err(message) => Ok(DshStartup::StartupFailed { message }),
    }
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
            reveal_engine,
            upgrade_dsh,
            install_dsh,
            restart_dsh
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // 退出时不杀 dsh 子进程:让它继续作为本地 web 服务存活,下次启动经 dsh-live.json
            // 探活复用,避免每次打开都拉一个新的、堆满孤儿进程。
        });
}