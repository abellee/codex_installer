use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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
const APP_REPO: &str = "abellee/codex_installer";
const CODEX_PLUSPLUS_UPDATE_URL: &str =
    "https://github.com/BigPizzaV3/CodexPlusPlus/releases/latest/download/latest.json";
const CC_SWITCH_UPDATE_URL: &str =
    "https://github.com/farion1231/cc-switch/releases/latest/download/latest.json";
const VC_REDIST_X64_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";
const VC_REDIST_ARM64_URL: &str = "https://aka.ms/vs/17/release/vc_redist.arm64.exe";
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
struct AppRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    html_url: String,
    assets: Vec<AppReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct AppReleaseAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AppUpdateInfo {
    current_version: String,
    latest_version: String,
    available: bool,
    release_notes: String,
    release_url: String,
    asset_name: String,
    download_url: String,
    digest: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AppUpdateProgress {
    percent: u8,
    downloaded: u64,
    total: u64,
}

#[derive(Debug, Deserialize)]
struct Release {
    #[serde(rename = "tag_name")]
    _tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize, Clone)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublishedAsset {
    name: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct CodexPlusPlusUpdate {
    version: String,
    assets: Vec<PublishedAsset>,
}

#[derive(Debug, Deserialize)]
struct PublishedPlatform {
    url: String,
}

#[derive(Debug, Deserialize)]
struct CcSwitchUpdate {
    version: String,
    platforms: HashMap<String, PublishedPlatform>,
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

fn choose_app_update_asset(
    release: &AppRelease,
    architecture: &str,
) -> Result<AppReleaseAsset> {
    let selected = release
        .assets
        .iter()
        .filter(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            let is_installer = lower.ends_with(".exe")
                && !lower.contains("uninstall")
                && !lower.contains("portable");
            let matches_architecture = match architecture {
                "x86_64" => {
                    (lower.contains("x64")
                        || lower.contains("x86_64")
                        || lower.contains("win64"))
                        && !lower.contains("arm64")
                        && !lower.contains("aarch64")
                }
                "aarch64" => lower.contains("arm64") || lower.contains("aarch64"),
                _ => false,
            };
            is_installer && matches_architecture
        })
        .max_by_key(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            (
                lower.contains("setup") as u8,
                lower.contains("installer") as u8,
            )
        })
        .ok_or_else(|| anyhow!("最新版本中没有适用于 {architecture} Windows 的安装包"))?;
    Ok(AppReleaseAsset {
        name: selected.name.clone(),
        browser_download_url: selected.browser_download_url.clone(),
        digest: selected.digest.clone(),
    })
}

fn is_newer_version(current: &str, latest_tag: &str) -> Result<bool> {
    let current_version = Version::parse(current).context("当前应用版本号格式无效")?;
    let latest_text = latest_tag.trim_start_matches(['v', 'V']);
    let latest_version = Version::parse(latest_text)
        .with_context(|| format!("Release 版本号格式无效：{latest_tag}"))?;
    Ok(latest_version > current_version)
}

fn current_app_update() -> AppUpdateInfo {
    let version = env!("CARGO_PKG_VERSION").to_string();
    AppUpdateInfo {
        current_version: version.clone(),
        latest_version: version,
        available: false,
        release_notes: String::new(),
        release_url: String::new(),
        asset_name: String::new(),
        download_url: String::new(),
        digest: None,
    }
}

async fn fetch_app_update() -> Result<AppUpdateInfo> {
    let client = Client::builder()
        .user_agent(format!("codex-installer/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("无法初始化更新服务")?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{APP_REPO}/releases/latest"
        ))
        .send()
        .await
        .context("无法连接 GitHub 更新服务")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(current_app_update());
    }
    let release = response
        .error_for_status()
        .context("GitHub 更新服务返回异常")?
        .json::<AppRelease>()
        .await
        .context("解析版本信息失败")?;

    let current_version_text = env!("CARGO_PKG_VERSION");
    let latest_version_text = release
        .tag_name
        .trim_start_matches(['v', 'V']);
    let latest_version = Version::parse(latest_version_text)
        .with_context(|| format!("Release 版本号格式无效：{}", release.tag_name))?;
    let available = is_newer_version(current_version_text, &release.tag_name)?;
    let asset = if available {
        Some(choose_app_update_asset(&release, std::env::consts::ARCH)?)
    } else {
        None
    };

    Ok(AppUpdateInfo {
        current_version: current_version_text.to_string(),
        latest_version: latest_version.to_string(),
        available,
        release_notes: release.body,
        release_url: release.html_url,
        asset_name: asset.as_ref().map(|item| item.name.clone()).unwrap_or_default(),
        download_url: asset
            .as_ref()
            .map(|item| item.browser_download_url.clone())
            .unwrap_or_default(),
        digest: asset.and_then(|item| item.digest),
    })
}

#[tauri::command]
async fn check_app_update() -> Result<AppUpdateInfo, String> {
    fetch_app_update().await.map_err(|error| error.to_string())
}

fn verify_sha256(path: &Path, digest: &str) -> Result<()> {
    let Some(expected) = digest.strip_prefix("sha256:") else {
        return Ok(());
    };
    let mut verify_file = File::open(path).context("无法读取下载文件")?;
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
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(anyhow!("下载校验失败，文件可能已损坏"))
    }
}

async fn perform_app_update(app: &AppHandle) -> Result<()> {
    let update = fetch_app_update().await?;
    if !update.available {
        return Err(anyhow!("当前已经是最新版本"));
    }
    let expected_prefix = format!("https://github.com/{APP_REPO}/releases/download/");
    if !update.download_url.starts_with(&expected_prefix) {
        return Err(anyhow!("更新下载地址不属于本项目的 GitHub Release"));
    }

    let client = Client::builder()
        .user_agent(format!("codex-installer/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("无法初始化更新下载服务")?;
    let response = client
        .get(&update.download_url)
        .send()
        .await
        .context("更新下载请求失败")?
        .error_for_status()
        .context("GitHub 拒绝了更新下载请求")?;
    let total = response.content_length().unwrap_or(0);
    let update_dir = std::env::temp_dir().join("CodexInstaller").join("updates");
    fs::create_dir_all(&update_dir).context("无法创建更新缓存目录")?;
    let safe_name = Path::new(&update.asset_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.to_ascii_lowercase().ends_with(".exe"))
        .ok_or_else(|| anyhow!("更新安装包名称无效"))?;
    let destination = update_dir.join(safe_name);
    let mut file = File::create(&destination).context("无法创建更新安装包")?;
    let mut stream = response.bytes_stream();
    let mut downloaded = 0_u64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("下载更新时连接中断")?;
        file.write_all(&chunk).context("写入更新安装包失败")?;
        downloaded += chunk.len() as u64;
        let percent = downloaded
            .saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0)
            .min(100) as u8;
        let _ = app.emit(
            "app-update-progress",
            AppUpdateProgress {
                percent,
                downloaded,
                total,
            },
        );
    }
    file.flush().context("刷新更新安装包失败")?;
    drop(file);
    if let Some(digest) = &update.digest {
        verify_sha256(&destination, digest)?;
    }

    Command::new(&destination)
        .spawn()
        .context("无法启动新版安装程序")?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn install_app_update(app: AppHandle) -> Result<(), String> {
    perform_app_update(&app)
        .await
        .map_err(|error| error.to_string())
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

fn release_asset_with_digest(
    release: &Release,
    name: String,
    browser_download_url: String,
) -> ReleaseAsset {
    let digest = release
        .assets
        .iter()
        .find(|asset| asset.browser_download_url == browser_download_url || asset.name == name)
        .and_then(|asset| asset.digest.clone());
    ReleaseAsset {
        name,
        browser_download_url,
        digest,
    }
}

fn choose_codex_plus_plus_asset(
    update: &CodexPlusPlusUpdate,
    release: &Release,
    architecture: &str,
) -> Result<ReleaseAsset> {
    let selected = update
        .assets
        .iter()
        .filter(|asset| {
            let lower = format!("{} {}", asset.name, asset.url).to_ascii_lowercase();
            let is_installer = lower.ends_with(".exe") || lower.ends_with(".msi");
            let is_other_platform =
                lower.contains("macos") || lower.contains("darwin") || lower.contains("linux");
            let matches_architecture = match architecture {
                "x86_64" => !lower.contains("arm64") && !lower.contains("aarch64"),
                "aarch64" => lower.contains("arm64") || lower.contains("aarch64"),
                _ => false,
            };
            is_installer && !is_other_platform && matches_architecture
        })
        .max_by_key(|asset| {
            let lower = format!("{} {}", asset.name, asset.url).to_ascii_lowercase();
            (
                lower.contains("setup") as u8,
                lower.contains("installer") as u8,
                lower.ends_with(".exe") as u8,
            )
        })
        .ok_or_else(|| {
            anyhow!(
                "Codex++ 官方更新清单中没有适用于 {architecture} Windows 的安装文件（{}）",
                update.version
            )
        })?;
    Ok(release_asset_with_digest(
        release,
        selected.name.clone(),
        selected.url.clone(),
    ))
}

fn choose_cc_switch_asset(
    update: &CcSwitchUpdate,
    release: &Release,
    architecture: &str,
) -> Result<ReleaseAsset> {
    let platform = match architecture {
        "x86_64" => "windows-x86_64",
        "aarch64" => "windows-aarch64",
        _ => return Err(anyhow!("CC Switch 暂不支持当前架构：{architecture}")),
    };
    let published = update.platforms.get(platform).ok_or_else(|| {
        anyhow!(
            "CC Switch 官方更新清单中没有 {platform} 安装文件（{}）",
            update.version
        )
    })?;
    let name = reqwest::Url::parse(&published.url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(str::to_string)
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "CC-Switch-Windows.msi".to_string());
    Ok(release_asset_with_digest(
        release,
        name,
        published.url.clone(),
    ))
}

async fn latest_companion_asset(client: &Client, companion: &str) -> Result<ReleaseAsset> {
    let (repo, update_url) = if companion == "cc-switch" {
        (CC_SWITCH_REPO, CC_SWITCH_UPDATE_URL)
    } else {
        (CODEX_PLUSPLUS_REPO, CODEX_PLUSPLUS_UPDATE_URL)
    };
    let release = latest_release(client, repo).await?;
    let response = client
        .get(update_url)
        .send()
        .await
        .with_context(|| format!("无法获取 {repo} 官方更新清单"))?
        .error_for_status()
        .with_context(|| format!("{repo} 官方更新清单不可用"))?;

    if companion == "cc-switch" {
        let update = response
            .json::<CcSwitchUpdate>()
            .await
            .context("解析 CC Switch 官方更新清单失败")?;
        choose_cc_switch_asset(&update, &release, std::env::consts::ARCH)
    } else {
        let update = response
            .json::<CodexPlusPlusUpdate>()
            .await
            .context("解析 Codex++ 官方更新清单失败")?;
        choose_codex_plus_plus_asset(&update, &release, std::env::consts::ARCH)
    }
}

#[allow(clippy::too_many_arguments, clippy::manual_checked_ops)]
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
    let elevated_script = if lower.ends_with(".msi") {
        "$arguments = @('/i', ('\"' + $env:CODEX_COMPANION_INSTALLER + '\"'), '/qn', '/norestart'); $process = Start-Process -FilePath 'msiexec.exe' -ArgumentList $arguments -Verb RunAs -PassThru -Wait; exit $process.ExitCode"
    } else {
        "$process = Start-Process -FilePath $env:CODEX_COMPANION_INSTALLER -ArgumentList '/S' -Verb RunAs -PassThru -Wait; exit $process.ExitCode"
    };
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            elevated_script,
        ])
        .env("CODEX_COMPANION_INSTALLER", path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| anyhow!("无法启动 {component} 安装程序：{error}"))?;
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

async fn install_visual_cpp_runtime(
    app: &AppHandle,
    control: &InstallControl,
    run_id: u64,
    path: &Path,
) -> Result<()> {
    ensure_active(control, run_id)?;
    emit_progress(
        app,
        run_id,
        61,
        "install",
        "正在安装 Microsoft Visual C++ 运行库",
        "这是 Codex++ 和 CC Switch 运行所需的系统组件",
        "系统依赖",
    );
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$process = Start-Process -FilePath $env:VC_REDIST_INSTALLER -ArgumentList @('/install', '/quiet', '/norestart') -Verb RunAs -PassThru -Wait; exit $process.ExitCode",
        ])
        .env("VC_REDIST_INSTALLER", path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| anyhow!("无法启动 Microsoft Visual C++ 运行库安装程序：{error}"))?;
    let mut current = 61_u8;
    loop {
        if ensure_active(control, run_id).is_err() {
            let _ = child.kill();
            return Err(anyhow!("安装任务已取消"));
        }
        if let Some(status) = child.try_wait().context("读取 Visual C++ 安装状态失败")? {
            if !status.success() {
                return Err(anyhow!(
                    "Microsoft Visual C++ 运行库安装程序返回错误（{status}）"
                ));
            }
            break;
        }
        current = (current + 1).min(62);
        emit_progress(
            app,
            run_id,
            current,
            "install",
            "正在安装 Microsoft Visual C++ 运行库",
            "Windows 正在注册运行时组件",
            "系统依赖",
        );
        tokio::time::sleep(std::time::Duration::from_millis(420)).await;
    }
    for _ in 0..20 {
        if visual_cpp_runtime_installed() {
            return Ok(());
        }
        ensure_active(control, run_id)?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(anyhow!(
        "Microsoft Visual C++ 运行库安装程序已退出，但 VCRUNTIME140.dll / VCRUNTIME140_1.dll 验证没有通过；请在权限提示中选择“是”后重试"
    ))
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

#[cfg(windows)]
fn visual_cpp_runtime_installed() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let subkey = match std::env::consts::ARCH {
        "aarch64" => "SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\ARM64",
        _ => "SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64",
    };
    let registry_installed = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(subkey)
        .ok()
        .and_then(|key: RegKey| key.get_value::<u32, _>("Installed").ok())
        .is_some_and(|installed| installed == 1);
    let Some(windows_dir) = std::env::var_os("WINDIR") else {
        return false;
    };
    let system_directory = PathBuf::from(windows_dir).join("System32");
    registry_installed
        && system_directory.join("VCRUNTIME140.dll").is_file()
        && system_directory.join("VCRUNTIME140_1.dll").is_file()
}

#[cfg(not(windows))]
fn visual_cpp_runtime_installed() -> bool {
    true
}

fn visual_cpp_runtime_asset() -> Result<ReleaseAsset> {
    let (name, url) = match std::env::consts::ARCH {
        "x86_64" => ("vc_redist.x64.exe", VC_REDIST_X64_URL),
        "aarch64" => ("vc_redist.arm64.exe", VC_REDIST_ARM64_URL),
        architecture => {
            return Err(anyhow!(
                "Microsoft Visual C++ 运行库暂不支持当前架构：{architecture}"
            ))
        }
    };
    Ok(ReleaseAsset {
        name: name.to_string(),
        browser_download_url: url.to_string(),
        digest: None,
    })
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

    if detected_codex.is_none() || !companion_installed || !visual_cpp_runtime_installed() {
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
    if visual_cpp_runtime_installed() {
        emit_progress(
            app,
            run_id,
            62,
            "skip",
            "检测到 Microsoft Visual C++ 运行库",
            "已确认系统依赖存在，跳过重复安装",
            "系统依赖",
        );
    } else {
        emit_progress(
            app,
            run_id,
            59,
            "download",
            "正在下载 Microsoft Visual C++ 运行库",
            "从 Microsoft 官方下载服务获取系统组件",
            "系统依赖",
        );
        let runtime_asset = visual_cpp_runtime_asset()?;
        let runtime_path = cache_dir.join(&runtime_asset.name);
        download_asset(
            app,
            control,
            run_id,
            &client,
            &runtime_asset,
            &runtime_path,
            59,
            60,
            "系统依赖",
        )
        .await?;
        install_visual_cpp_runtime(app, control, run_id, &runtime_path).await?;
    }

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
        let companion_asset = latest_companion_asset(&client, &request.companion).await?;
        ensure_active(control, run_id)?;
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
#[cfg(windows)]
fn copy_qq() -> Result<(), String> {
    use std::ptr::null_mut;
    use std::thread;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };

    const CF_UNICODETEXT: u32 = 13;

    struct ClipboardGuard;
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }

    let mut opened = false;
    for _ in 0..10 {
        if unsafe { OpenClipboard(null_mut()) } != 0 {
            opened = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if !opened {
        return Err(format!(
            "无法访问系统剪贴板：{}",
            std::io::Error::last_os_error()
        ));
    }
    let _clipboard = ClipboardGuard;

    if unsafe { EmptyClipboard() } == 0 {
        return Err(format!(
            "无法清空系统剪贴板：{}",
            std::io::Error::last_os_error()
        ));
    }

    let wide_text: Vec<u16> = QQ_NUMBER.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide_text.len() * std::mem::size_of::<u16>();
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len) };
    if memory.is_null() {
        return Err(format!(
            "无法分配剪贴板内存：{}",
            std::io::Error::last_os_error()
        ));
    }

    let destination = unsafe { GlobalLock(memory) };
    if destination.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return Err(format!(
            "无法写入系统剪贴板：{}",
            std::io::Error::last_os_error()
        ));
    }

    unsafe {
        std::ptr::copy_nonoverlapping(
            wide_text.as_ptr(),
            destination.cast::<u16>(),
            wide_text.len(),
        );
        GlobalUnlock(memory);
    }

    if unsafe { SetClipboardData(CF_UNICODETEXT, memory) }.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return Err(format!(
            "复制 QQ 号失败：{}",
            std::io::Error::last_os_error()
        ));
    }

    Ok(())
}

#[tauri::command]
#[cfg(not(windows))]
fn copy_qq() -> Result<(), String> {
    Err("当前系统暂不支持自动复制 QQ 号".to_string())
}

fn launch_executable(path: &Path, label: &str) -> Result<(), String> {
    match Command::new(path).spawn() {
        Ok(_) => Ok(()),
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(740) => launch_elevated(path, label),
        Err(error) => Err(format!("无法启动 {label}：{error}")),
    }
}

#[cfg(windows)]
fn launch_elevated(path: &Path, label: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Start-Process -FilePath $env:CODEX_LAUNCH_TARGET -Verb RunAs",
        ])
        .env("CODEX_LAUNCH_TARGET", path)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("无法请求管理员权限启动 {label}：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} 需要管理员权限，权限确认已取消"))
    }
}

#[tauri::command]
fn launch_installed_app(target: String) -> Result<(), String> {
    if target == "codex" {
        return match detect_codex_desktop() {
            Some(CodexDesktopInstallation::Executable(path)) => launch_executable(&path, "Codex"),
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
    launch_executable(&path, label)
}

pub fn run() {
    tauri::Builder::default()
        .manage(InstallControl::default())
        .invoke_handler(tauri::generate_handler![
            install_components,
            cancel_installation,
            check_app_update,
            install_app_update,
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

    #[test]
    fn codex_plus_plus_uses_official_metadata_after_asset_rename() {
        let url = "https://example.test/downloads/renamed-installer.exe";
        let update = CodexPlusPlusUpdate {
            version: "v-next".to_string(),
            assets: vec![PublishedAsset {
                name: "renamed-installer.exe".to_string(),
                url: url.to_string(),
            }],
        };
        let release = Release {
            _tag_name: "v-next".to_string(),
            assets: vec![ReleaseAsset {
                name: "renamed-installer.exe".to_string(),
                browser_download_url: url.to_string(),
                digest: Some("sha256:abc".to_string()),
            }],
        };

        let selected = choose_codex_plus_plus_asset(&update, &release, "x86_64").unwrap();
        assert_eq!(selected.browser_download_url, url);
        assert_eq!(selected.digest.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn cc_switch_uses_platform_mapping_without_x64_in_filename() {
        let url = "https://example.test/downloads/CC-Switch-Windows.msi";
        let update = CcSwitchUpdate {
            version: "3.19.2".to_string(),
            platforms: HashMap::from([(
                "windows-x86_64".to_string(),
                PublishedPlatform {
                    url: url.to_string(),
                },
            )]),
        };
        let release = Release {
            _tag_name: "v3.19.2".to_string(),
            assets: vec![ReleaseAsset {
                name: "CC-Switch-Windows.msi".to_string(),
                browser_download_url: url.to_string(),
                digest: Some("sha256:def".to_string()),
            }],
        };

        let selected = choose_cc_switch_asset(&update, &release, "x86_64").unwrap();
        assert_eq!(selected.name, "CC-Switch-Windows.msi");
        assert_eq!(selected.digest.as_deref(), Some("sha256:def"));
    }

    #[test]
    fn visual_cpp_runtime_asset_uses_microsoft_official_download() {
        let asset = visual_cpp_runtime_asset().unwrap();
        assert!(asset
            .browser_download_url
            .starts_with("https://aka.ms/vs/17/release/"));
        assert!(asset.name.starts_with("vc_redist."));
    }

    #[test]
    fn app_update_selects_setup_for_current_architecture() {
        let release = AppRelease {
            tag_name: "v0.2.0".to_string(),
            body: "notes".to_string(),
            html_url: "https://github.com/abellee/codex_installer/releases/tag/v0.2.0"
                .to_string(),
            assets: vec![
                AppReleaseAsset {
                    name: "Codex.Setup_0.2.0_arm64-setup.exe".to_string(),
                    browser_download_url: "https://example.test/arm64.exe".to_string(),
                    digest: None,
                },
                AppReleaseAsset {
                    name: "Codex.Setup_0.2.0_x64-setup.exe".to_string(),
                    browser_download_url: "https://example.test/x64.exe".to_string(),
                    digest: Some("sha256:abc".to_string()),
                },
            ],
        };

        let selected = choose_app_update_asset(&release, "x86_64").unwrap();
        assert!(selected.name.contains("x64-setup.exe"));
        assert_eq!(selected.digest.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn app_update_only_accepts_newer_semantic_versions() {
        assert!(is_newer_version("0.1.0", "v0.2.0").unwrap());
        assert!(!is_newer_version("0.2.0", "v0.2.0").unwrap());
        assert!(!is_newer_version("0.3.0", "v0.2.0").unwrap());
    }

    #[test]
    fn no_release_means_current_version_is_latest() {
        let update = current_app_update();
        assert!(!update.available);
        assert_eq!(update.current_version, update.latest_version);
    }

    #[cfg(windows)]
    #[test]
    fn elevation_required_is_windows_error_740() {
        let error = std::io::Error::from_raw_os_error(740);
        assert_eq!(error.raw_os_error(), Some(740));
    }
}
