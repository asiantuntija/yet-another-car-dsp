use std::path::PathBuf;
use std::fs;
use std::env;

const APP_NAME: &str = "car-dsp";
const EFFECTS_DIR_NAME: &str = "effects";
const SESSION_FILE_NAME: &str = "session.json";

fn get_base_config_dir() -> PathBuf {
    let mut path = PathBuf::from(
        env::var("XDG_CONFIG_HOME")
            .map(|v| v)
            .unwrap_or_else(|_| {
                let home = env::var("HOME").expect("HOME environment variable not set");
                format!("{}/.config", home)
            }),
    );
    path.push(APP_NAME);
    path
}

pub fn get_session_path() -> String {
    let mut path = get_base_config_dir();
    path.push(SESSION_FILE_NAME);
    path.to_string_lossy().to_string()
}

pub fn get_effects_dir() -> PathBuf {
    let mut path = get_base_config_dir();
    path.push(EFFECTS_DIR_NAME);
    path
}

pub fn ensure_config_dirs() -> std::io::Result<()> {
    let base = get_base_config_dir();
    fs::create_dir_all(&base)?;
    
    let effects = get_effects_dir();
    fs::create_dir_all(&effects)?;
    
    Ok(())
}

pub fn get_systemd_service_path() -> PathBuf {
    let mut path = PathBuf::from(
        env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| {
                let home = env::var("HOME").expect("HOME environment variable not set");
                format!("{}/.config", home)
            }),
    );
    path.push("systemd/user/car-dsp.service");
    path
}

pub fn ensure_systemd_service() -> std::io::Result<bool> {
    let service_path = get_systemd_service_path();
    
    // Ensure the directory exists
    if let Some(parent) = service_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe_path = env::current_exe()?;
    let exe_path_str = exe_path.to_string_lossy();

    let service_content = format!(
        r#"[Unit]
Description=Yet Another Car DSP
After=pipewire.service

[Service]
Type=simple
# ExecStart={exe}
ExecStopPost=pactl unload-module module-null-sink
KillMode=control-group

[Install]
WantedBy=default.target"#,
        exe = exe_path_str
    );


    let should_write = if let Ok(existing) = fs::read_to_string(&service_path) {
        existing != service_content
    } else {
        true
    };

    if should_write {
        fs::write(&service_path, service_content)?;
    }

    Ok(should_write)
}

pub fn restart_systemd_service() -> std::io::Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()?;
    
    if !status.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to reload systemd daemon"));
    }

    let status = std::process::Command::new("systemctl")
        .args(["--user", "restart", "car-dsp.service"])
        .status()?;

    if !status.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to restart car-dsp service"));
    }

    Ok(())
}

