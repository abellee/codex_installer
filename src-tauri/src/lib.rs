use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, State};

const CODEX_DESKTOP_X64_URL: &str =
    "https://persistent.oaistatic.com/codex-app-prod/ChatGPT-x64.msix";
const CODEX_DESKTOP_ARM64_URL: &str =
    "https://persistent.oaistatic.com/codex-app-prod/ChatGPT-arm64.msix";
const CODEX_PLUSPLUS_REPO: &str = "BigPizzaV3/CodexPlusPlus";
const CC_SWITCH_REPO: &str = "farion1231/cc-switch";
const QQ_NUMBER: &str = "751077517";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallRequest {
    companion: String,
    run_id: u64,
}

#[derive(Default)]
struct InstallControl {
    current_run: AtomicU64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallResult {
    codex_version: String,
    companion_name: String,
    install_directory: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    run_id: u64,
    percent: u8,
    phase: String,
    title: String,
    detail: String,
    component: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize, Clone)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexDesktopInstallation {
    MicrosoftStore,
    Executable(PathBuf),
}

impl CodexDesktopInstallation {
    fn description(&self) -> String {
        match self {
            Self::MicrosoftStore => "Desktop（Microsoft Store / MSIX）".to_string(),
            Self::Executable(path) => format!("Desktop（{}）", path.display()),
        }
    }
}

fn emit_progress(
    app: &AppHandle,
    run_id: u64,
    percent: u8,
    phase: &str,
    title: &str,
    detail: &str,
    component: &str,
) {
    let _ = app.emit(
        "install-progress",
        ProgressPayload {
            run_id,
            percent,
            phase: phase.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            component: component.to_string(),
        },
    );
}

fn ensure_active(control: &InstallControl, run_id: u64) -> Result<()> {
    if control.current_run.load(Ordering::SeqCst) == run_id {
        Ok(())
    } else {
        Err(anyhow!("安装任务已取消"))
    }
}

async fn latest_release(client: &Client, repo: &str) -> Result<Release> {
    client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .send()
        .await
        .with_context(|| format!("无法连接 {repo} 的发布页面"))?
        .error_for_status()
        .with_context(|| format!("{repo} 没有可用的稳定版本"))?
        .json::<Release>()
        .await
        .context("解析上游版本信息失败")
}

fn is_windows_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.contains("windows") || lower.contains("win") || lower.contains("pc-windows"))
        && (lower.contains("x64")
            || lower.contains("x86_64")
            || lower.contains("amd64")
            || lower.contains("win64"))
}

fn codex_desktop_asset() -> Result<ReleaseAsset> {
    let (name, browser_download_url) = match std::env::consts::ARCH {
        "x86_64" => ("ChatGPT-x64.msix", CODEX_DESKTOP_X64_URL),
        "aarch64" => ("ChatGPT-arm64.msix", CODEX_DESKTOP_ARM64_URL),
        architecture => return Err(anyhow!("Codex 桌面版暂不支持当前架构：{architecture}")),
    };
    Ok(ReleaseAsset {
        name: name.to_string(),
        browser_download_url: browser_download_url.to_string(),
        digest: None,
    })
}

fn choose_companion_asset(release: &Release, name: &str) -> Result<ReleaseAsset> {
    release
        .assets
        .iter()
        .filter(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            is_windows_asset(&asset.name)
                && (lower.ends_with(".exe") || lower.ends_with(".msi"))
                && !lower.contains("portable")
                && !lower.contains("debug")
                && !lower.contains("source")
        })
        .max_by_key(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            (
                lower.contains("setup") as u8,
                lower.contains("installer") as u8,
                lower.ends_with(".exe") as u8,
            )
        })
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "{name} 的最新版本中没有找到 Windows x64 安装文件（{}）",
                release.tag_name
            )
        })
}

async fn download_asset(
    app: &AppHandle,
    control: &InstallControl,
    run_id: u64,
    client: &Client,
    asset: &ReleaseAsset,
    destination: &Path,
    start: u8,
    end: u8,
    component: &str,
) -> Result<PathBuf> {
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("下载请求失败")?
        .error_for_status()
        .context("上游拒绝了下载请求")?;
    let total = response.content_length().unwrap_or(0);
    let mut stream = response.bytes_stream();
    let mut file = File::create(destination).context("无法创建临时安装文件")?;
    let mut downloaded = 0_u64;

    while let Some(chunk) = stream.next().await {
        ensure_active(control, run_id)?;
        let chunk = chunk.context("下载过程中连接中断")?;
        file.write_all(&chunk).context("写入安装文件失败")?;
        downloaded += chunk.len() as u64;
        let percent = if total > 0 {
            start.saturating_add((((end - start) as u64 * downloaded) / total) as u8)
        } else {
            start
        };
        emit_progress(
            app,
            run_id,
            percent,
            "download",
            &format!("正在下载 {component}"),
            &asset.name,
            component,
        );
    }

    file.flush().context("刷新下载文件失败")?;
    if let Some(digest) = &asset.digest {
        if let Some(expected) = digest.strip_prefix("sha256:") {
            let mut verify_file = File::open(destination).context("无法读取下载文件")?;
            let mut hasher = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = verify_file.read(&mut buffer).context("校验下载文件失败")?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            let actual = hex::encode(hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(anyhow!("下载校验失败，文件可能已损坏"));
            }
        }
    }

    Ok(destination.to_path_buf())
}

fn local_install_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|path| path.join("CodexInstaller").join("bin"))
        .ok_or_else(|| anyhow!("无法确定当前用户的本地安装目录"))
}

async fn install_codex_desktop(
    app: &AppHandle,
    control: &InstallControl,
    run_id: u64,
    path: &Path,
) -> Result<()> {
    ensure_active(control, run_id)?;
    emit_progress(
        app,
        run_id,
        50,
        "install",
        "正在安装 Codex 桌面版",
        "正在部署 OpenAI 官方签名的 MSIX 应用包",
        "Codex",
    );
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Add-AppxPackage -Path $env:CODEX_INSTALLER_MSIX_PATH",
        ])
        .env("CODEX_INSTALLER_MSIX_PATH", path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().context("无法启动 Codex 桌面版安装程序")?;
    let mut current = 51_u8;
    loop {
        if ensure_active(control, run_id).is_err() {
            let _ = child.kill();
            return Err(anyhow!("安装任务已取消"));
        }
        if let Some(status) = child.try_wait().context("读取 Codex 安装状态失败")? {
            if !status.success() {
                return Err(anyhow!("Codex 桌面版安装程序返回错误（{status}）"));
            }
            break;
        }
        current = (current + 1).min(58);
        emit_progress(
            app,
            run_id,
            current,
            "install",
            "正在安装 Codex 桌面版",
            "Windows 正在注册应用文件",
            "Codex",
        );
        tokio::time::sleep(std::time::Duration::from_millis(420)).await;
    }
    Ok(())
}

async fn install_release_installer(
    app: &AppHandle,
    control: &InstallControl,
    run_id: u64,
    path: &Path,
    component: &str,
) -> Result<()> {
    ensure_active(control, run_id)?;
    emit_progress(
        app,
        run_id,
        78,
        "install",
        &format!("正在安装 {component}"),
        "正在运行官方安装程序",
        component,
    );
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let mut command = if lower.ends_with(".msi") {
        let mut cmd = Command::new("msiexec.exe");
        cmd.args(["/i", &path.to_string_lossy(), "/qn", "/norestart"]);
        cmd
    } else {
        let mut cmd = Command::new(path);
        cmd.arg("/S");
        cmd
    };
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command
        .spawn()
        .with_context(|| format!("无法启动 {component} 安装程序"))?;
    let mut current = 80_u8;
    loop {
        if ensure_active(control, run_id).is_err() {
            let _ = child.kill();
            return Err(anyhow!("安装任务已取消"));
        }
        if let Some(status) = child.try_wait().context("读取安装程序状态失败")? {
            if !status.success() {
                return Err(anyhow!("{component} 安装程序返回错误（{status}）"));
            }
            break;
        }
        current = (current + 1).min(94);
        emit_progress(
            app,
            run_id,
            current,
            "install",
            &format!("正在安装 {component}"),
            "安装程序正在处理文件",
            component,
        );
        tokio::time::sleep(std::time::Duration::from_millis(420)).await;
    }
    Ok(())
}

fn codex_desktop_common_candidates() -> Vec<PathBuf> {
    const RELATIVE_PATHS: &[&str] = &[
        "Codex\\Codex.exe",
        "OpenAI\\Codex\\Codex.exe",
        "OpenAI Codex\\Codex.exe",
        "ChatGPT\\ChatGPT.exe",
        "OpenAI\\ChatGPT\\ChatGPT.exe",
    ];
    let mut roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local_app_data).join("Programs"));
    }
    roots
        .into_iter()
        .flat_map(|root| {
            RELATIVE_PATHS
                .iter()
                .map(move |relative| root.join(relative))
        })
        .collect()
}

fn registered_executable_path(value: &str) -> PathBuf {
    let trimmed = value.trim();
    let executable = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        trimmed.split(',').next().unwrap_or(trimmed)
    };
    PathBuf::from(executable.trim())
}

#[cfg(windows)]
fn codex_desktop_app_path_candidates() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .flat_map(|root| {
            ["Codex.exe", "ChatGPT.exe"]
                .into_iter()
                .filter_map(move |name| {
                    let key = RegKey::predef(root)
                        .open_subkey(format!(
                            "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{name}"
                        ))
                        .ok()?;
                    let value: String = key.get_value("").ok()?;
                    Some(registered_executable_path(&value))
                })
        })
        .collect()
}

#[cfg(not(windows))]
fn codex_desktop_app_path_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn microsoft_store_codex_installed() -> bool {
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "if (Get-AppxPackage -Name 'OpenAI.Codex') { exit 0 } else { exit 1 }",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(windows))]
fn microsoft_store_codex_installed() -> bool {
    false
}

fn detect_codex_desktop() -> Option<CodexDesktopInstallation> {
    codex_desktop_app_path_candidates()
        .into_iter()
        .chain(codex_desktop_common_candidates())
        .find(|path| path.is_file())
        .map(CodexDesktopInstallation::Executable)
        .or_else(|| {
            microsoft_store_codex_installed().then_some(CodexDesktopInstallation::MicrosoftStore)
        })
}

fn companion_is_installed(companion: &str) -> bool {
    let Some(local_data) = dirs::data_local_dir() else {
        return false;
    };
    let programs = local_data.join("Programs");
    match companion {
        "cc-switch" => programs.join("CC Switch").join("cc-switch.exe").is_file(),
        _ => {
            let directory = programs.join("Codex++");
            directory.join("codex-plus-plus-manager.exe").is_file()
                && directory.join("codex-plus-plus.exe").is_file()
        }
    }
}

async fn perform_installation(
    app: &AppHandle,
    control: &InstallControl,
    request: InstallRequest,
) -> Result<InstallResult> {
    let run_id = request.run_id;
    ensure_active(control, run_id)?;
    let companion_name = match request.companion.as_str() {
        "cc-switch" => "CC Switch",
        _ => "Codex++",
    };
    let companion_repo = if request.companion == "cc-switch" {
        CC_SWITCH_REPO
    } else {
        CODEX_PLUSPLUS_REPO
    };
    let client = Client::builder()
        .user_agent("codex-installer/0.1")
        .build()
        .context("无法初始化下载服务")?;
    let install_dir = local_install_dir()?;
    emit_progress(
        app,
        run_id,
        4,
        "check",
        "正在检查电脑环境",
        "确认系统版本与安装目录",
        "环境检查",
    );

    let cache_dir = install_dir.join(".cache");
    let detected_codex = detect_codex_desktop();
    let companion_installed = companion_is_installed(&request.companion);
    let check_detail = match (detected_codex.is_some(), companion_installed) {
        (true, true) => format!("已检测到 Codex 和 {companion_name}，无需重复下载"),
        (true, false) => format!("已检测到 Codex，仅需安装 {companion_name}"),
        (false, true) => format!("已检测到 {companion_name}，仅需安装 Codex"),
        (false, false) => format!("未检测到 Codex 和 {companion_name}，准备下载安装"),
    };
    emit_progress(
        app,
        run_id,
        7,
        "check",
        "已完成安装检测",
        &check_detail,
        "环境检查",
    );

    if detected_codex.is_none() || !companion_installed {
        fs::create_dir_all(&cache_dir).context("无法创建下载缓存目录")?;
    }

    let codex_version = if let Some(installation) = detected_codex {
        emit_progress(
            app,
            run_id,
            58,
            "skip",
            "检测到 Codex 桌面版已安装",
            "已确认桌面应用存在，跳过重复下载与安装",
            "Codex",
        );
        installation.description()
    } else {
        emit_progress(
            app,
            run_id,
            8,
            "download",
            "正在获取 Codex 桌面版",
            "连接 OpenAI 官方桌面应用下载服务",
            "Codex",
        );
        let codex_asset = codex_desktop_asset()?;
        let codex_path = cache_dir.join(&codex_asset.name);
        download_asset(
            app,
            control,
            run_id,
            &client,
            &codex_asset,
            &codex_path,
            10,
            46,
            "Codex",
        )
        .await?;
        ensure_active(control, run_id)?;
        install_codex_desktop(app, control, run_id, &codex_path).await?;
        detect_codex_desktop()
            .map(|installation| installation.description())
            .ok_or_else(|| anyhow!("Codex 桌面版安装完成，但应用注册验证没有通过"))?
    };

    ensure_active(control, run_id)?;
    if companion_installed {
        emit_progress(
            app,
            run_id,
            94,
            "skip",
            &format!("检测到 {companion_name} 已安装"),
            "跳过重复下载与安装，可在完成页直接启动",
            companion_name,
        );
    } else {
        emit_progress(
            app,
            run_id,
            63,
            "download",
            &format!("正在获取 {companion_name} 版本"),
            "选择适用于 Windows 的官方安装包",
            companion_name,
        );
        let companion_release = latest_release(&client, companion_repo).await?;
        ensure_active(control, run_id)?;
        let companion_asset = choose_companion_asset(&companion_release, companion_name)?;
        let companion_path = cache_dir.join(&companion_asset.name);
        download_asset(
            app,
            control,
            run_id,
            &client,
            &companion_asset,
            &companion_path,
            64,
            77,
            companion_name,
        )
        .await?;
        install_release_installer(app, control, run_id, &companion_path, companion_name).await?;
    }
    ensure_active(control, run_id)?;
    emit_progress(
        app,
        run_id,
        97,
        "verify",
        "正在做最后检查",
        "确认桌面应用与配置工具可以正常使用",
        "最终检查",
    );
    emit_progress(
        app,
        run_id,
        100,
        "done",
        "全部安装完成",
        "组件已经可以使用",
        "完成",
    );
    Ok(InstallResult {
        codex_version,
        companion_name: companion_name.to_string(),
        install_directory: install_dir.to_string_lossy().to_string(),
    })
}

#[cfg(target_os = "macos")]
async fn simulate_installation(
    app: &AppHandle,
    control: &InstallControl,
    request: &InstallRequest,
) -> Result<InstallResult> {
    let run_id = request.run_id;
    let companion_name = if request.companion == "cc-switch" {
        "CC Switch"
    } else {
        "Codex++"
    };
    let stages = [
        (
            5,
            "check",
            "正在检查电脑环境",
            "macOS 测试环境仅模拟安装流程",
            "环境检查",
        ),
        (
            18,
            "download",
            "正在下载 Codex",
            "模拟接收 Codex 安装文件",
            "Codex",
        ),
        (
            50,
            "install",
            "正在安装 Codex",
            "模拟部署 Codex 桌面应用",
            "Codex",
        ),
        (
            64,
            "verify",
            "正在验证 Codex",
            "模拟检查 Codex 版本",
            "Codex",
        ),
        (
            76,
            "download",
            "正在获取配置工具",
            "模拟下载配置工具安装包",
            companion_name,
        ),
        (
            94,
            "install",
            "正在安装配置工具",
            "模拟执行官方安装程序",
            companion_name,
        ),
        (
            100,
            "done",
            "全部安装完成",
            "macOS 流程测试已完成，未修改系统",
            "完成",
        ),
    ];

    for (percent, phase, title, detail, component) in stages {
        ensure_active(control, run_id)?;
        emit_progress(app, run_id, percent, phase, title, detail, component);
        tokio::time::sleep(std::time::Duration::from_millis(620)).await;
    }

    Ok(InstallResult {
        codex_version: "macOS 流程测试".to_string(),
        companion_name: companion_name.to_string(),
        install_directory: "模拟模式（未写入文件）".to_string(),
    })
}

#[tauri::command]
async fn install_components(
    app: AppHandle,
    control: State<'_, InstallControl>,
    request: InstallRequest,
) -> Result<InstallResult, String> {
    control.current_run.store(request.run_id, Ordering::SeqCst);
    #[cfg(windows)]
    {
        perform_installation(&app, &control, request)
            .await
            .map_err(|error| error.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        simulate_installation(&app, &control, &request)
            .await
            .map_err(|error| error.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, control, request);
        Err("当前操作系统不受支持".to_string())
    }
}

#[tauri::command]
fn cancel_installation(control: State<'_, InstallControl>, run_id: u64) {
    let _ = control
        .current_run
        .compare_exchange(run_id, 0, Ordering::SeqCst, Ordering::SeqCst);
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    open::that_detached(url).map_err(|error| error.to_string())
}

#[tauri::command]
fn copy_qq() -> Result<(), String> {
    let mut child = Command::new("clip.exe")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("无法访问系统剪贴板：{error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法写入系统剪贴板".to_string())?;
    stdin
        .write_all(QQ_NUMBER.as_bytes())
        .map_err(|error| format!("复制 QQ 号失败：{error}"))?;
    drop(stdin);
    let status = child
        .wait()
        .map_err(|error| format!("复制 QQ 号失败：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("复制 QQ 号失败".to_string())
    }
}

#[tauri::command]
fn launch_installed_app(target: String) -> Result<(), String> {
    if target == "codex" {
        return match detect_codex_desktop() {
            Some(CodexDesktopInstallation::Executable(path)) => Command::new(&path)
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("无法启动 Codex：{error}")),
            Some(CodexDesktopInstallation::MicrosoftStore) => Command::new("explorer.exe")
                .arg("shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App")
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("无法启动 Codex：{error}")),
            None => Err("没有找到 Codex 桌面版，请先完成安装".to_string()),
        };
    }

    let programs = dirs::data_local_dir()
        .ok_or_else(|| "无法确定本地程序目录".to_string())?
        .join("Programs");
    let (path, label) = match target.as_str() {
        "codex-plus-plus-manager" => (
            programs.join("Codex++").join("codex-plus-plus-manager.exe"),
            "Codex++ 管理工具",
        ),
        "codex-plus-plus" => (
            programs.join("Codex++").join("codex-plus-plus.exe"),
            "Codex++",
        ),
        "cc-switch" => (
            programs.join("CC Switch").join("cc-switch.exe"),
            "CC Switch",
        ),
        _ => return Err("未知的启动目标".to_string()),
    };

    if !path.is_file() {
        return Err(format!("没有找到 {label}，请先完成安装"));
    }
    Command::new(&path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动 {label}：{error}"))
}

pub fn run() {
    tauri::Builder::default()
        .manage(InstallControl::default())
        .invoke_handler(tauri::generate_handler![
            install_components,
            cancel_installation,
            open_external,
            copy_qq,
            launch_installed_app
        ])
        .run(tauri::generate_context!())
        .expect("启动 Codex 安装助手失败");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_app_path_parser_handles_icons_and_quoted_paths() {
        assert_eq!(
            registered_executable_path(r#""C:\Program Files\OpenAI\Codex\Codex.exe" --open"#),
            PathBuf::from(r"C:\Program Files\OpenAI\Codex\Codex.exe")
        );
        assert_eq!(
            registered_executable_path(r"C:\Program Files\OpenAI\Codex\Codex.exe,0"),
            PathBuf::from(r"C:\Program Files\OpenAI\Codex\Codex.exe")
        );
    }

    #[test]
    fn desktop_asset_uses_openai_official_msix() {
        let selected = codex_desktop_asset().expect("supported Windows architecture");
        assert!(selected.name.ends_with(".msix"));
        assert!(selected
            .browser_download_url
            .starts_with("https://persistent.oaistatic.com/codex-app-prod/"));
    }
}
