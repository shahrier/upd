// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use which::which;
use sysinfo::System;
use regex::Regex;
use serde::Serialize;
use serde_json::json;
use tokio::process::Command;
use tokio::io::{AsyncBufReadExt, BufReader};
use tauri::{AppHandle, Emitter};
use futures::future::join_all;

#[derive(Serialize)]
struct PackageManagerResult {
    name: String,
    detected: bool,
    error: Option<String>,
    packages: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct PackageActionResult {
    manager: String,
    action: String,
    package: String,
    success: bool,
    error: Option<String>,
    output: Option<String>,
}

#[derive(Serialize)]
struct ManagerCommandResult {
    manager: String,
    args: Vec<String>,
    success: bool,
    output: String,
    error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
struct PackageEntry {
    name: String,
    version: String,
    latest_version: Option<String>,
}

#[derive(Serialize, Debug)]
struct PackageDetails {
    name: String,
    description: Option<String>,
    homepage: Option<String>,
    changelog: Option<String>,
    repository: Option<String>,
    license: Option<String>,
    author: Option<String>,
    dependencies: Option<serde_json::Value>,
    raw: Option<serde_json::Value>,
}

#[tauri::command]
async fn scan_package_managers() -> Result<Vec<PackageManagerResult>, String> {
    let managers = vec![
        ("npm", vec!["npm", "list", "-g", "--depth=0", "--json"]),
        ("pip", vec!["pip", "list", "--format=json"]),
        ("pip3", vec!["pip3", "list", "--format=json"]),
        ("pipx", vec!["pipx", "list", "--json"]),
        ("conda", vec!["conda", "list", "--json"]),
        ("pixi", vec!["pixi", "list", "--json"]),
        ("cargo", vec!["cargo", "install", "--list"]),
        ("brew", vec!["brew", "list", "--versions"]),
        ("mas", vec!["mas", "list"]),
    ];

    let mut results = Vec::new();
    for (name, cmd) in managers {
        let detected = which(cmd[0]).is_ok();
        if !detected {
            continue; // Only add installed managers
        }
        let mut command = Command::new(cmd[0]);
        command.args(&cmd[1..]);
        let output = command.output().await;
        match output {
            Ok(out) => {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let mut packages = None;
                    // Parse and fetch latest version for npm and pip
                    if name == "npm" {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                            if let Some(deps) = json.get("dependencies") {
                                let mut pkgs = Vec::new();
                                for (pkg_name, pkg_info) in deps.as_object().unwrap() {
                                    let version = pkg_info.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    // Fetch latest version
                                    let latest = get_npm_latest_version(pkg_name).await;
                                    pkgs.push(PackageEntry {
                                        name: pkg_name.clone(),
                                        version,
                                        latest_version: latest,
                                    });
                                }
                                packages = Some(serde_json::to_value(pkgs).unwrap());
                            }
                        }
                    } else if name == "pip" {
                        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                            let mut pkgs = Vec::new();
                            for pkg in list {
                                let pkg_name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
                                let latest = get_pip_latest_version(pkg_name).await;
                                pkgs.push(PackageEntry {
                                    name: pkg_name.to_string(),
                                    version: version.to_string(),
                                    latest_version: latest,
                                });
                            }
                            packages = Some(serde_json::to_value(pkgs).unwrap());
                        }
                    } else if name == "pip3" {
                        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                            let mut pkgs = Vec::new();
                            let mut futures = Vec::new();
                            for pkg in &list {
                                let pkg_name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                futures.push(get_pip_latest_version_pypi(pkg_name.to_string()));
                            }
                            let latest_versions = join_all(futures).await;
                            for (idx, pkg) in list.iter().enumerate() {
                                let pkg_name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
                                pkgs.push(PackageEntry {
                                    name: pkg_name.to_string(),
                                    version: version.to_string(),
                                    latest_version: latest_versions.get(idx).cloned().unwrap_or(None),
                                });
                            }
                            packages = Some(serde_json::to_value(pkgs).unwrap());
                        }
                    } else if name == "cargo" {
                        // cargo output is a string, parse lines and fetch latest version for each package
                        let lines = stdout.split('\n');
                        let mut pkgs = Vec::new();
                        let mut names = Vec::new();
                        let mut versions = Vec::new();
                        for line in lines {
                            let parts: Vec<&str> = line.split(' ').collect();
                            if parts.len() >= 2 && !parts[0].is_empty() {
                                let mut version = parts[1].trim();
                                if version.starts_with('v') {
                                    version = &version[1..];
                                }
                                if version.ends_with(':') {
                                    version = &version[..version.len()-1];
                                }
                                names.push(parts[0].to_string());
                                versions.push(version.to_string());
                            }
                        }
                        let mut futures = Vec::new();
                        for name in &names {
                            futures.push(get_cargo_latest_version(name.clone()));
                        }
                        let latest_versions = join_all(futures).await;
                        for (idx, (name, version)) in names.iter().zip(versions.iter()).enumerate() {
                            pkgs.push(PackageEntry {
                                name: name.clone(),
                                version: version.clone(),
                                latest_version: latest_versions.get(idx).cloned().unwrap_or(None),
                            });
                        }
                        packages = Some(serde_json::to_value(pkgs).unwrap());
                    } else if name == "brew" {
                        // brew output is a string, parse lines and fetch latest version for each package
                        let blines = stdout.split('\n');
                        let mut pkgs = Vec::new();
                        let mut names = Vec::new();
                        let mut versions = Vec::new();
                        for line in blines {
                            let idx = line.find(' ');
                            if let Some(idx) = idx {
                                names.push(line[..idx].to_string());
                                let mut version = line[(idx + 1)..].to_string();
                                if let Some((base, _)) = version.split_once('_') {
                                    version = base.to_string();
                                }
                                versions.push(version);
                            }
                        }
                        let mut futures = Vec::new();
                        for name in &names {
                            futures.push(get_brew_latest_version(name.clone()));
                        }
                        let latest_versions = join_all(futures).await;
                        for (idx, (name, version)) in names.iter().zip(versions.iter()).enumerate() {
                            pkgs.push(PackageEntry {
                                name: name.clone(),
                                version: version.clone(),
                                latest_version: latest_versions.get(idx).cloned().unwrap_or(None),
                            });
                        }
                        packages = Some(serde_json::to_value(pkgs).unwrap());
                    } else {
                        // Fallback: keep original logic
                        let packages_val = serde_json::from_str(&stdout)
                            .map(|v| v)
                            .unwrap_or(serde_json::Value::String(stdout.to_string()));
                        packages = Some(packages_val);
                    }
                    results.push(PackageManagerResult {
                        name: name.to_string(),
                        detected: true,
                        error: None,
                        packages,
                    });
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    results.push(PackageManagerResult {
                        name: name.to_string(),
                        detected: true,
                        error: Some(stderr.to_string()),
                        packages: None,
                    });
                }
            }
            Err(e) => {
                results.push(PackageManagerResult {
                    name: name.to_string(),
                    detected: true,
                    error: Some(e.to_string()),
                    packages: None,
                });
            }
        }
    }
    Ok(results)
}

// Fetch latest npm version using 'npm view <pkg> version'
async fn get_npm_latest_version(pkg: &str) -> Option<String> {
    let output = Command::new("npm")
        .args(["view", pkg, "version"])
        .output()
        .await;
    match output {
        Ok(out) if out.status.success() => {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !v.is_empty() { Some(v) } else { None }
        }
        _ => None,
    }
}

// Fetch latest pip version using 'pip index versions <pkg>' or fallback to PyPI API
async fn get_pip_latest_version(pkg: &str) -> Option<String> {
    let output = Command::new("pip")
        .args(["index", "versions", pkg])
        .output()
        .await;
    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if line.contains("AVAILABLE VERSIONS") {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() > 1 {
                        let versions = parts[1].trim();
                        let latest = versions.split(',').next().unwrap_or("").trim();
                        if !latest.is_empty() { return Some(latest.to_string()); }
                    }
                }
            }
        }
    }
    // Fallback: fetch from PyPI JSON API
    let url = format!("https://pypi.org/pypi/{}/json", pkg);
    if let Ok(resp) = reqwest::get(&url).await {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            if let Some(v) = json.get("info").and_then(|i| i.get("version")).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
    }
    None
}

async fn get_pip_latest_version_pypi(pkg: String) -> Option<String> {
    let url = format!("https://pypi.org/pypi/{}/json", pkg);
    if let Ok(resp) = reqwest::get(&url).await {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            if let Some(v) = json.get("info").and_then(|i| i.get("version")).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
    }
    None
}

async fn get_brew_latest_version(pkg: String) -> Option<String> {
    let output = Command::new("brew")
        .args(["info", &pkg, "--json"])
        .output()
        .await;
    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                if let Some(arr) = json.as_array() {
                    if let Some(pkg_info) = arr.get(0) {
                        if let Some(v) = pkg_info.get("versions").and_then(|v| v.get("stable")).and_then(|v| v.as_str()) {
                            return Some(v.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

async fn get_cargo_latest_version(pkg: String) -> Option<String> {
    println!("[cargo] Querying crates.io for: '{}'", pkg);
    let url = format!("https://crates.io/api/v1/crates/{}", pkg);
    if let Ok(resp) = reqwest::get(&url).await {
        if let Ok(json) = resp.json::<serde_json::Value>().await {
            let latest = json.get("crate").and_then(|c| c.get("newest_version")).and_then(|v| v.as_str());
            println!("[cargo] crates.io response for '{}': {:?}", pkg, latest);
            if let Some(v) = latest {
                return Some(v.to_string());
            }
        }
    }
    println!("[cargo] crates.io lookup failed for '{}'", pkg);
    None
}

#[tauri::command]
async fn package_action(manager: String, action: String, package: String) -> Result<PackageActionResult, String> {
    // Map action to command for each manager
    let (cmd, args) = match (manager.as_str(), action.as_str()) {
        ("npm", "install") => ("npm", vec!["install", "-g", &package]),
        ("npm", "uninstall") => ("npm", vec!["uninstall", "-g", &package]),
        ("npm", "update") => ("npm", vec!["update", "-g", &package]),
        ("pip", "install") => ("pip", vec!["install", &package]),
        ("pip", "uninstall") => ("pip", vec!["uninstall", "-y", &package]),
        ("pip", "update") => ("pip", vec!["install", "--upgrade", &package]),
        ("pipx", "install") => ("pipx", vec!["install", &package]),
        ("pipx", "uninstall") => ("pipx", vec!["uninstall", &package]),
        ("pipx", "update") => ("pipx", vec!["upgrade", &package]),
        ("conda", "install") => ("conda", vec!["install", "-y", &package]),
        ("conda", "uninstall") => ("conda", vec!["remove", "-y", &package]),
        ("conda", "update") => ("conda", vec!["update", "-y", &package]),
        ("pixi", "install") => ("pixi", vec!["add", &package]),
        ("pixi", "uninstall") => ("pixi", vec!["remove", &package]),
        ("pixi", "update") => ("pixi", vec!["update", &package]),
        ("cargo", "install") => ("cargo", vec!["install", &package]),
        ("cargo", "uninstall") => ("cargo", vec!["uninstall", &package]), // cargo uninstall is not standard, may need cargo uninstall crate
        ("cargo", "update") => ("cargo", vec!["install", "--force", &package]),
        ("brew", "install") => ("brew", vec!["install", &package]),
        ("brew", "uninstall") => ("brew", vec!["uninstall", &package]),
        ("brew", "update") => ("brew", vec!["upgrade", &package]),
        ("mas", "install") => ("mas", vec!["install", &package]),
        ("mas", "uninstall") => ("mas", vec!["uninstall", &package]),
        ("mas", "update") => ("mas", vec!["upgrade", &package]),
        _ => return Err("Unsupported manager/action combination".to_string()),
    };
    let mut command = Command::new(cmd);
    command.args(&args);
    let output = command.output().await;
    match output {
        Ok(out) => {
            let success = out.status.success();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            Ok(PackageActionResult {
                manager,
                action,
                package,
                success,
                error: if success { None } else { Some(stderr.clone()) },
                output: Some(stdout + &stderr),
            })
        }
        Err(e) => Ok(PackageActionResult {
            manager,
            action,
            package,
            success: false,
            error: Some(e.to_string()),
            output: None,
        }),
    }
}

#[tauri::command]
async fn run_manager_command(manager: String, args: Vec<String>) -> Result<ManagerCommandResult, String> {
    let output = Command::new(&manager).args(&args).output().await;
    match output {
        Ok(out) => {
            let success = out.status.success();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            Ok(ManagerCommandResult {
                manager,
                args,
                success,
                output: stdout + &stderr,
                error: if success { None } else { Some(stderr) },
            })
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn run_manager_command_stream(app: AppHandle, manager: String, args: Vec<String>, event_id: String) -> Result<(), String> {
    let mut cmd = Command::new(&manager);
    cmd.args(&args);
    match cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let app_clone = app.clone();
            let event_id_clone = event_id.clone();
            // Stream stdout
            if let Some(out) = stdout {
                let mut reader = BufReader::new(out).lines();
                let app_clone = app_clone.clone();
                let event_id_clone = event_id_clone.clone();
                tauri::async_runtime::spawn(async move {
                    while let Ok(Some(line)) = reader.next_line().await {
                        let _ = app_clone.emit(&format!("cli://{}", event_id_clone), line);
                    }
                });
            }
            // Stream stderr
            if let Some(err) = stderr {
                let mut reader = BufReader::new(err).lines();
                let app_clone = app_clone.clone();
                let event_id_clone = event_id_clone.clone();
                tauri::async_runtime::spawn(async move {
                    while let Ok(Some(line)) = reader.next_line().await {
                        let _ = app_clone.emit(&format!("cli://{}", event_id_clone), line);
                    }
                });
            }
            // Wait for process to finish
            let status = child.wait().await;
            let _ = app.emit(&format!("cli://{}:done", event_id), status.map(|s| s.code()).unwrap_or(None));
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn get_package_details(manager: String, package: String) -> Result<PackageDetails, String> {
    match manager.as_str() {
        "npm" => {
            let url = format!("https://registry.npmjs.org/{}", package);
            let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
            let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let latest = json.get("dist-tags").and_then(|d| d.get("latest")).and_then(|v| v.as_str()).unwrap_or("");
            let version_info = json.get("versions").and_then(|v| v.get(latest)).cloned();
            let description = version_info.as_ref().and_then(|v| v.get("description")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = version_info.as_ref().and_then(|v| v.get("homepage")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let changelog = version_info.as_ref().and_then(|v| v.get("changelog")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let repository = version_info.as_ref().and_then(|v| v.get("repository")).and_then(|v| v.get("url")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let license = version_info.as_ref().and_then(|v| v.get("license")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let author = version_info.as_ref().and_then(|v| v.get("author")).and_then(|v| v.get("name")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let dependencies = version_info.as_ref().and_then(|v| v.get("dependencies")).cloned();
            Ok(PackageDetails {
                name: package,
                description,
                homepage,
                changelog,
                repository,
                license,
                author,
                dependencies,
                raw: version_info,
            })
        }
        "pip" | "pip3" => {
            let url = format!("https://pypi.org/pypi/{}/json", package);
            let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
            let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let info = json.get("info");
            let description = info.and_then(|v| v.get("summary")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = info.and_then(|v| v.get("home_page")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let changelog = info.and_then(|v| v.get("project_urls")).and_then(|v| v.get("Changelog")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let repository = info.and_then(|v| v.get("project_urls")).and_then(|v| v.get("Source")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let license = info.and_then(|v| v.get("license")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let author = info.and_then(|v| v.get("author")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let dependencies = json.get("info").and_then(|v| v.get("requires_dist")).cloned();
            Ok(PackageDetails {
                name: package,
                description,
                homepage,
                changelog,
                repository,
                license,
                author,
                dependencies,
                raw: info.cloned(),
            })
        }
        "cargo" => {
            let url = format!("https://crates.io/api/v1/crates/{}", package);
            let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
            let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let crate_info = json.get("crate");
            let description = crate_info.and_then(|v| v.get("description")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = crate_info.and_then(|v| v.get("homepage")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let repository = crate_info.and_then(|v| v.get("repository")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let license = crate_info.and_then(|v| v.get("license")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let author = None; // Not directly available
            let dependencies = json.get("versions").and_then(|v| v.get(0)).and_then(|v| v.get("dependencies")).cloned();
            let changelog = None; // Not directly available
            Ok(PackageDetails {
                name: package,
                description,
                homepage,
                changelog,
                repository,
                license,
                author,
                dependencies,
                raw: crate_info.cloned(),
            })
        }
        "brew" => {
            let output = Command::new("brew").args(["info", &package, "--json"]).output().await.map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).to_string());
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let json: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| e.to_string())?;
            let info = json.get(0);
            let description = info.and_then(|v| v.get("desc")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = info.and_then(|v| v.get("homepage")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let changelog = info.and_then(|v| v.get("urls")).and_then(|v| v.get("changelog")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let repository = info.and_then(|v| v.get("urls")).and_then(|v| v.get("stable")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let license = info.and_then(|v| v.get("license")).and_then(|v| v.as_str()).map(|s| s.to_string());
            let author = None; // Not directly available
            let dependencies = info.and_then(|v| v.get("dependencies")).cloned();
            Ok(PackageDetails {
                name: package,
                description,
                homepage,
                changelog,
                repository,
                license,
                author,
                dependencies,
                raw: info.cloned(),
            })
        }
        _ => Err("Manager not supported yet".to_string()),
    }
}

#[tauri::command]
async fn get_system_info() -> Result<serde_json::Value, String> {
    let mut sys = System::new_all();
    sys.refresh_all();
    let os = System::name().unwrap_or_default();
    let os_version = System::os_version().unwrap_or_default();
    let kernel_version = System::kernel_version().unwrap_or_default();
    let cpu_brand = sys.cpus().get(0).map(|c| c.brand().to_string()).unwrap_or_default();
    let cpu_cores = sys.cpus().len();
    let total_memory = sys.total_memory();
    let used_memory = sys.used_memory();
    let total_swap = sys.total_swap();
    let used_swap = sys.used_swap();
    let disks: Vec<serde_json::Value> = Vec::new();
    // Get versions for Python, Node, Rust, etc.
    async fn get_version(cmd: &str, arg: &str) -> Option<String> {
        let output = Command::new(cmd).arg(arg).output().await.ok()?;
        if !output.status.success() { return None; }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Some(stdout.trim().to_string())
    }
    let python = get_version("python3", "--version").await
        .or(get_version("python", "--version").await);
    let node = get_version("node", "--version").await;
    let rustc = get_version("rustc", "--version").await;
    let cargo = get_version("cargo", "--version").await;
    let npm = get_version("npm", "--version").await;
    let pip = get_version("pip", "--version").await
        .or(get_version("pip3", "--version").await);
    let conda = get_version("conda", "--version").await;
    let brew = get_version("brew", "--version").await;
    let shell = std::env::var("SHELL").ok();
    let env_vars: std::collections::HashMap<_, _> = std::env::vars().collect();
    // Clean up version strings
    let python = python.as_deref().and_then(|s| extract_version("python", s)).unwrap_or_default();
    let node = node.as_deref().and_then(|s| extract_version("node", s)).unwrap_or_default();
    let rustc = rustc.as_deref().and_then(|s| extract_version("rustc", s)).unwrap_or_default();
    let cargo = cargo.as_deref().and_then(|s| extract_version("cargo", s)).unwrap_or_default();
    let npm = npm.as_deref().and_then(|s| extract_version("npm", s)).unwrap_or_default();
    let pip = pip.as_deref().and_then(|s| extract_version("pip", s)).unwrap_or_default();
    let conda = conda.as_deref().and_then(|s| extract_version("conda", s)).unwrap_or_default();
    let brew = brew.as_deref().and_then(|s| extract_version("brew", s)).unwrap_or_default();
    Ok(json!({
        "os": os,
        "os_version": os_version,
        "kernel_version": kernel_version,
        "cpu_brand": cpu_brand,
        "cpu_cores": cpu_cores,
        "total_memory": total_memory,
        "used_memory": used_memory,
        "total_swap": total_swap,
        "used_swap": used_swap,
        "disks": disks,
        "versions": {
            "python": python,
            "node": node,
            "rustc": rustc,
            "cargo": cargo,
            "npm": npm,
            "pip": pip,
            "conda": conda,
            "brew": brew,
        },
        "shell": shell,
        "env_vars": env_vars,
    }))
}

fn extract_version(tool: &str, output: &str) -> Option<String> {
    let patterns = [
        ("pip", r"pip ([\d.]+)"),
        ("python", r"Python ([\d.]+)"),
        ("node", r"v?([\d.]+)"),
        ("npm", r"([\d.]+)"),
        ("cargo", r"cargo ([\d.]+)"),
        ("rustc", r"rustc ([\d.]+)"),
        ("brew", r"Homebrew ([\d.]+)"),
        ("conda", r"conda ([\d.]+)"),
    ];
    let output = output.trim();
    for (name, pat) in patterns.iter() {
        if tool == *name {
            if let Ok(re) = Regex::new(pat) {
                if let Some(caps) = re.captures(output) {
                    if let Some(ver) = caps.get(1) {
                        return Some(ver.as_str().to_string());
                    }
                }
            }
        }
    }
    // fallback: just return the first word that looks like a version
    let fallback = Regex::new(r"\d+(\.\d+)+").unwrap();
    if let Some(caps) = fallback.captures(output) {
        if let Some(ver) = caps.get(0) {
            return Some(ver.as_str().to_string());
        }
    }
    None
}

fn fix_path() {
    let mut paths = vec![
        "/usr/local/bin".to_string(),
        "/opt/homebrew/bin".to_string(),
        "/usr/bin".to_string(),
        "/bin".to_string(),
        "/usr/sbin".to_string(),
        "/sbin".to_string(),
        // Add user's cargo bin
        format!("{}/.cargo/bin", std::env::var("HOME").unwrap_or_default()),
    ];
    if let Ok(existing) = env::var("PATH") {
        paths.push(existing);
    }
    let new_path = paths.join(":");
    env::set_var("PATH", new_path);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    fix_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![scan_package_managers, package_action, run_manager_command, run_manager_command_stream, get_package_details, get_system_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
