use tokio::process::{Command};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::Mutex as TokioMutex;
use log::{debug, info};
use crate::effects::{EffectType, FilterInstance, BackendError, write_filter_config, write_eq_config, types::FilterConfigValue};

const VIRTUAL_SINK_NAME: &str = "YACD";


pub struct PipeWireBackend {
    pub(crate) active_filters: Mutex<HashMap<(String, EffectType), FilterInstance>>,
    pub(crate) routing_lock: TokioMutex<()>,
}

impl PipeWireBackend {
    pub fn new() -> Self {
        Self {
            active_filters: Mutex::new(HashMap::new()),
            routing_lock: TokioMutex::new(()), // Initialize this
        }
    }

    fn filter_node_name(device: &str, effect: EffectType) -> String {
        format!("yacd_filter_{}_{}", device, effect.as_str())
    }
    
    async fn run_cmd(&self, cmd: &str, args: &[&str]) -> Result<String, BackendError> {
        let cmd_str = format!("{} {}", cmd, args.join(" "));
        debug!("Executing: {}", cmd_str);

        let output = Command::new(cmd)
            .args(args)
            .output()
            .await
            .map_err(BackendError::IoError)?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            debug!("Command success: {}. Output: {}", cmd_str, stdout);
            Ok(stdout)
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            Err(BackendError::CommandFailed(err_msg))
        }
    }

   async fn link(&self, src: &str, dst: &str) -> Result<(), BackendError> {
        debug!("Linking: {} -> {}", src, dst);
        self.run_cmd("pw-link", &[src, dst]).await.map(|_| ())
    }

    async fn unlink(&self, src: &str, dst: &str) -> Result<(), BackendError> {
        debug!("Unlinking: {} -> {}", src, dst);
        self.run_cmd("pw-link", &["-d", src, dst]).await.map(|_| ())
    }


    pub async fn start_virtual_sink(&self) -> Result<(), BackendError> {
        if let Err(e) = self.run_cmd("pactl", &["unload-module", "module-null-sink"]).await {
            debug!("Could not unload existing null-sink (it might not be loaded): {}", e);
        }

        self.run_cmd("pactl", &[
            "load-module", 
            "module-null-sink", 
            &format!("sink_name={}", VIRTUAL_SINK_NAME), 
            &format!("sink_properties=device.description={}", VIRTUAL_SINK_NAME)
        ]).await?;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        Ok(())
    }

    pub fn cleanup_sync(&self) -> Result<(), BackendError> {
        debug!("Performing synchronous cleanup...");
        
        let filters: Vec<FilterInstance> = {
            let mut lock = self.active_filters.lock().unwrap();
            lock.drain().map(|(_, instance)| instance).collect()
        };

        for filter in filters {
            if let Some(pid) = filter.child.id() {
                debug!("Killing filter process {} (PID: {})", filter.node_name, pid);
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .status();
            }
        }

        // Force kill any remaining YACD filter orphans
        let _ = std::process::Command::new("pkill")
            .args(["-f", "yacd_filter"])
            .status();

        let _ = std::process::Command::new("pactl")
            .args(["unload-module", "module-null-sink"])
            .status();

        Ok(())
    }

    pub async fn spawn_filter(&self, device: &str, effect: EffectType, value: FilterConfigValue) -> Result<(), BackendError> {
        let node_name = Self::filter_node_name(device, effect);
        
        self.remove_filter(device, effect).await?;

        let (binary, config_path) = match (effect, value) {
            (EffectType::EQ, FilterConfigValue::Bands(bands)) => {
                ("lsp-plugins-graph-equalizer-x16-stereo", write_eq_config(device, &bands)?)
            }
            (EffectType::LPF | EffectType::HPF, FilterConfigValue::Single(val)) => {
                ("lsp-plugins-filter-stereo", write_filter_config(device, effect, val)?)
            }
            _ => return Err(BackendError::CommandFailed("Invalid value type for effect".to_string())),
        };
        
        debug!("Spawning {} filter for {}: node={}, config={}", effect.as_str(), device, node_name, config_path);
        let child = Command::new(binary)
            .args(["-hl", "-c", &config_path, "-n", &node_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(BackendError::IoError)?;

        self.active_filters.lock().unwrap().insert(
            (device.to_string(), effect), 
            FilterInstance { child, node_name }
        );
        Ok(())
    }

    pub async fn update_eq_filter(&self, device: &str, bands: &[f32]) -> Result<(), BackendError> {
        self.remove_filter(device, EffectType::EQ).await?;
        self.spawn_filter(device, EffectType::EQ, FilterConfigValue::Bands(bands.to_vec())).await?;
        
        Ok(())
    }

    pub async fn remove_filter(&self, device: &str, effect: EffectType) -> Result<(), BackendError> {
        let node_name = Self::filter_node_name(device, effect);

        let instance_opt = {
            let mut filters = self.active_filters.lock().unwrap();
            filters.remove(&(device.to_string(), effect))
        };
        
        if let Some(instance) = instance_opt {
            debug!("Removing tracked filter {} by killing process", instance.node_name);
            let _ = instance.kill().await;
        }

        // Kill any orphan processes matching this node name (e.g. from a previous crash)
        let _ = Command::new("pkill")
            .args(["-f", &node_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        Ok(())
    }


    pub async fn get_output_devices(&self) -> Result<Vec<String>, BackendError> {
        let stdout = self.run_cmd("pw-link", &["-i"]).await?;
        let mut node_ports: std::collections::HashMap<String, std::collections::HashSet<String>> = 
            std::collections::HashMap::new();
        
        for line in stdout.lines() {
            if let Some((node_name, port_name)) = line.rsplit_once(':') {
                node_ports
                    .entry(node_name.to_string())
                    .or_default()
                    .insert(port_name.to_string());
            }
        }
        
        let mut devices: Vec<String> = node_ports
            .into_iter()
            .filter(|(node, ports)| {
                !node.contains(VIRTUAL_SINK_NAME) 
                && !node.contains("Virtual") 
                && !node.contains("Null")
                && !node.contains("Dummy")
                && ports.iter().any(|p| p.contains("playback_FL")) 
                && ports.iter().any(|p| p.contains("playback_FR"))
            })
            .map(|(node, _)| node)
            .collect();
        
        devices.sort();
        info!("Backend detected {} output devices: {:?}", devices.len(), devices);
        Ok(devices)
    }

    async fn clear_channel_links(
        &self,
        device: &str,
        _device_filters: &[EffectType], // Unused in new implementation
        sink_out: &str,
        filter_in: &str,
        filter_out: &str,
        dev_in: &str,
    ) -> Result<(), BackendError> {
        let src_root = format!("{}:{}", VIRTUAL_SINK_NAME, sink_out);
        let dst_root = format!("{}:{}", device, dev_in);
        let effects = [EffectType::EQ, EffectType::LPF, EffectType::HPF];

        debug!("Routing channel [{}]: Cleaning up all potential links...", sink_out);
        
        // 1. Unlink direct path
        let _ = self.unlink(&src_root, &dst_root).await;

        // 2. Unlink everything related to filters for this device
        for effect_src in &effects {
            let src_node = Self::filter_node_name(device, *effect_src);
            let src_port = format!("{}:{}", src_node, filter_out);

            // Unlink Filter -> Device
            let _ = self.unlink(&src_port, &dst_root).await;

            // Unlink Filter -> Other Filters
            for effect_dst in &effects {
                if effect_src == effect_dst { continue; }
                let dst_node = Self::filter_node_name(device, *effect_dst);
                let dst_port = format!("{}:{}", dst_node, filter_in);
                let _ = self.unlink(&src_port, &dst_port).await;
            }
        }

        // 3. Unlink Virtual Sink -> All Filters
        for effect in &effects {
            let node = Self::filter_node_name(device, *effect);
            let in_port = format!("{}:{}", node, filter_in);
            let _ = self.unlink(&src_root, &in_port).await;
        }

        Ok(())
    }

    async fn apply_channel_links(
    &self,
    device: &str,
    active_effects: &[&str],
    order: &[EffectType],
    sink_out: &str,
    filter_in: &str,
    filter_out: &str,
    dev_in: &str,
    ) -> Result<(), BackendError> {
    let src_root = format!("{}:{}", VIRTUAL_SINK_NAME, sink_out);
    let dst_root = format!("{}:{}", device, dev_in);

    if active_effects.is_empty() {
        debug!("Routing channel [{}]: Direct link (no effects)", sink_out);
        self.link(&src_root, &dst_root).await?;
    } else {
        debug!("Routing channel [{}]: Effect chain: {} -> ...", sink_out, src_root);
        let mut current_src = src_root;
        
        for effect in order {
            if active_effects.contains(&effect.as_str()) {
                let node = Self::filter_node_name(device, *effect);
                let filter_in_port = format!("{}:{}", node, filter_in);
                let filter_out_port = format!("{}:{}", node, filter_out);

                debug!("  -> Adding filter: {} ({} -> {})", effect.as_str(), current_src, filter_in_port);
                self.link(&current_src, &filter_in_port).await?;
                current_src = filter_out_port;
            }
        }
        debug!("  -> Final link: {} -> {}", current_src, dst_root);
        self.link(&current_src, &dst_root).await?;
    }
    Ok(())
}

    pub async fn set_routing(&self, device: &str, activate: bool, active_effects: &[&str], order: &[EffectType]) -> Result<(), BackendError> {
        let channels = [
            ("monitor_FL", "in_l", "out_l", "playback_FL"),
            ("monitor_FR", "in_r", "out_r", "playback_FR"),
        ];

        debug!("set_routing: device={}, activate={}, effects={:?}", device, activate, active_effects);

        let device_filters: Vec<EffectType> = {
            let filters = self.active_filters.lock().unwrap();
            filters.keys().filter(|(dev, _)| dev == device).map(|(_, e)| *e).collect()
        };

        for (sink_out, filter_in, filter_out, dev_in) in channels {
        self.clear_channel_links(device, &device_filters, sink_out, filter_in, filter_out, dev_in).await?;

        if activate {
            self.apply_channel_links(device, active_effects, order, sink_out, filter_in, filter_out, dev_in).await?;
        } else {
            debug!("Routing channel [{}]: Deactivating (links cleared)", sink_out);
        }
    }
    Ok(())
}

    pub async fn set_device_volume(&self, device: &str, volume: f32) -> Result<(), BackendError> {
        let vol_percent = format!("{:.0}%", volume * 100.0);
        debug!("Backend: Setting volume for {} to {}", device, vol_percent);
        self.run_cmd("pactl", &["set-sink-volume", device, &vol_percent]).await.map(|_| ())
    }

    pub async fn set_virtual_sink_volume(&self, volume: f32) -> Result<(), BackendError> {
        self.set_device_volume(VIRTUAL_SINK_NAME, volume).await
    }

    pub async fn set_effect_value(&self, device: &str, effect: EffectType, value: f32) -> Result<(), BackendError> {
    debug!("Backend: Updating {:?} for {} to {}Hz", effect, device, value);
    
        self.remove_filter(device, effect).await?;
        self.spawn_filter(device, effect, FilterConfigValue::Single(value)).await?;
        Ok(())
    }
}

