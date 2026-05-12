use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use crate::backend::PipeWireBackend;
use crate::session::{SessionManager, SessionData};
use log::{error, debug, info};

#[derive(Debug)]
pub enum BackendCmd {
    UpdateVolume(String, f32),
    SetVirtualSinkVolume(f32),
    UpdateEffectActive(String, crate::effects::EffectType, bool),
    UpdateEffectValue(String, crate::effects::EffectType, f32),
    UpdateEQ(String, Vec<f32>),
    UpdateRouting(String, bool),
    UpdateEffectOrder(String),
}


#[derive(Clone)]
pub struct SessionState {
    pub current: SessionData,
    pub saved: SessionData,
}

pub struct AppState {
    pub backend: PipeWireBackend,
    pub session_mgr: SessionManager,
    pub state: Mutex<SessionState>,
    pub is_syncing: AtomicBool,
    pub tx: mpsc::UnboundedSender<BackendCmd>,
}

impl AppState {
    pub fn new(backend: PipeWireBackend, session_path: &str, tx: mpsc::UnboundedSender<BackendCmd>) -> Self {
        let session_mgr = SessionManager::new(session_path);
        let data = session_mgr.load();
        Self {
            backend,
            session_mgr,
            state: Mutex::new(SessionState {
                current: data.clone(),
                saved: data,
            }),
            is_syncing: AtomicBool::new(false),
            tx,
        }
    }

    pub async fn backend_worker(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<BackendCmd>) {
        info!("Backend worker started");
        while let Some(cmd) = rx.recv().await {
            match cmd {
                BackendCmd::UpdateVolume(dev, vol) => {
                    if let Err(e) = self.backend.set_device_volume(&dev, vol).await {
                        error!("Worker: Failed to set volume for {}: {}", dev, e);
                    }
                }
                BackendCmd::SetVirtualSinkVolume(vol) => {
                    if let Err(e) = self.backend.set_virtual_sink_volume(vol).await {
                        error!("Worker: Failed to set virtual sink volume: {}", e);
                    }
                }
                BackendCmd::UpdateEffectActive(dev, eff, active) => {
                    if !active {
                        let _ = self.backend.remove_filter(&dev, eff).await;
                    }
                    self.apply_backend_state(&dev, true).await;
                }
                BackendCmd::UpdateEffectValue(dev, eff, val) => {
                    if let Err(e) = self.backend.set_effect_value(&dev, eff, val).await {
                        error!("Worker: Failed to update effect value for {}: {}", dev, e);
                    }
                    self.apply_backend_state(&dev, true).await;
                }
                BackendCmd::UpdateEQ(dev, bands) => {
                    if let Err(e) = self.backend.update_eq_filter(&dev, &bands).await {
                        error!("Worker: Failed to update EQ for {}: {}", dev, e);
                    }
                    self.apply_backend_state(&dev, true).await;
                }
                BackendCmd::UpdateRouting(dev, active) => {
                    self.apply_backend_state(&dev, active).await;
                }
                BackendCmd::UpdateEffectOrder(dev) => {
                    self.apply_backend_state(&dev, true).await;
                }
            }
        }
    }

    pub fn save_all(&self) {
        let mut state = self.state.lock().unwrap();
        self.session_mgr.save(&state.current);
        state.saved = state.current.clone();
    }

    pub fn save_device(&self, device: &str) {
        let mut state = self.state.lock().unwrap();
        
        let settings = match state.current.devices.get(device) {
            Some(s) => s.clone(),
            None => return,
        };

        // Load existing disk data to ensure we only update one device
        let mut disk_data = self.session_mgr.load();
        disk_data.devices.insert(device.to_string(), settings.clone());
        
        self.session_mgr.save(&disk_data);
        
        // Update the saved mirror so is_device_dirty(device) returns false
        state.saved.devices.insert(device.to_string(), settings);
    }

    pub fn is_any_dirty(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.current.devices != state.saved.devices 
            || state.current.virtual_sink_volume != state.saved.virtual_sink_volume
    }

    pub fn is_virtual_sink_dirty(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.current.virtual_sink_volume != state.saved.virtual_sink_volume
    }

    pub fn save_virtual_sink_volume(&self) {
        let mut state = self.state.lock().unwrap();
        self.session_mgr.save(&state.current);
        state.saved.virtual_sink_volume = state.current.virtual_sink_volume;
    }

    pub fn revert_virtual_sink_volume(&self) -> f32 {
        let mut state = self.state.lock().unwrap();
        state.current.virtual_sink_volume = state.saved.virtual_sink_volume;
        state.current.virtual_sink_volume
    }

    pub fn revert_device(&self, device: &str) -> (crate::session::DeviceSettings, bool, String) {
        let mut state = self.state.lock().unwrap();
        
        // Clone settings first to avoid overlapping borrow of 'state'
        let saved_settings = state.saved.devices.get(device).cloned();
        
        let is_active = saved_settings.as_ref().map(|s| s.active).unwrap_or(false);
        let name = saved_settings.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| device.to_string());
        
        let settings = state.current.devices.entry(device.to_string()).or_default();
        settings.active = is_active;
        settings.name = name.clone();

        if let Some(saved) = saved_settings {
            settings.volume = saved.volume;
            settings.eq = saved.eq.clone();
            settings.lpf = saved.lpf.clone();
            settings.hpf = saved.hpf.clone();
        }
        
        (settings.clone(), is_active, name)
    }

    pub fn set_device_routing(self: &Arc<Self>, device: &str, active: bool) {
        if self.is_syncing.load(Ordering::SeqCst) {
            let mut state = self.state.lock().unwrap();
            state.current.devices.entry(device.to_string()).or_default().active = active;
            return;
        }

        info!("Queueing routing for {}: active={}", device, active);
        let device_str = device.to_string();
        
        let mut state = self.state.lock().unwrap();
        state.current.devices.entry(device_str.clone()).or_default().active = active;
        
        let _ = self.tx.send(BackendCmd::UpdateRouting(device_str, active));
    }

    pub async fn apply_backend_state(&self, device: &str, active: bool) {
        let _lock = self.backend.routing_lock.lock().await;
        
        let (active_effects, values, volume) = {
            let state = self.state.lock().unwrap();
            let dev_settings = state.current.devices.get(device);
            
            let mut effects: Vec<crate::effects::EffectType> = Vec::new();
            let mut vals: std::collections::HashMap<crate::effects::EffectType, crate::effects::types::FilterConfigValue> = std::collections::HashMap::new();
            let vol = dev_settings.map(|d| d.volume).unwrap_or(0.5);

            if let Some(d) = dev_settings {
                if d.eq.active {
                    effects.push(crate::effects::EffectType::EQ);
                    vals.insert(crate::effects::EffectType::EQ, crate::effects::types::FilterConfigValue::Bands(d.eq.bands.clone()));
                }
                if d.lpf.active {
                    effects.push(crate::effects::EffectType::LPF);
                    vals.insert(crate::effects::EffectType::LPF, crate::effects::types::FilterConfigValue::Single(d.lpf.value));
                }
                if d.hpf.active {
                    effects.push(crate::effects::EffectType::HPF);
                    vals.insert(crate::effects::EffectType::HPF, crate::effects::types::FilterConfigValue::Single(d.hpf.value));
                }
            }
            (effects, vals, vol)
        };
        
        let order = {
            let state = self.state.lock().unwrap();
            state.current.devices.get(device)
                .map(|d| d.effect_order.clone())
                .unwrap_or_else(|| vec![crate::effects::EffectType::EQ, crate::effects::EffectType::LPF, crate::effects::EffectType::HPF])
        };

        debug!("Applying backend state for {}: active={}, volume={}, effects={:?}, order={:?}", device, active, volume, active_effects, order);

        // Sync volume to backend
        if let Err(e) = self.backend.set_device_volume(device, volume).await {
            error!("Error syncing volume for {}: {}", device, e);
        }

        // Remove any filters currently running in the backend that should NOT be active
        let current_backend_effects: Vec<crate::effects::EffectType> = {
            let filters = self.backend.active_filters.lock().unwrap();
            filters.keys()
                .filter(|(dev, _)| dev == device)
                .map(|(_, eff)| *eff)
                .collect()
        };

        for eff in current_backend_effects {
            if !active_effects.contains(&eff) {
                info!("Backend sync: Removing inactive filter {:?} for {}", eff, device);
                if let Err(e) = self.backend.remove_filter(device, eff).await {
                    error!("Error removing filter {:?} for {}: {}", eff, device, e);
                }
            }
        }

        if active {
            for effect in &active_effects {
                if let Some(val) = values.get(effect) {
                    let is_running = self.backend.active_filters.lock().unwrap()
                        .contains_key(&(device.to_string(), *effect));

                    if !is_running {
                        info!("Backend sync: Spawning missing filter {:?} for {}", effect, device);
                        if let Err(e) = self.backend.spawn_filter(device, *effect, val.clone()).await {
                            error!("Error spawning filter {:?} for {}: {}", effect, device, e);
                        }
                    }
                }
            }
            // Give PipeWire a moment to register the new nodes before linking
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        } else {
            for effect in &active_effects {
                info!("Backend sync: Deactivating device {}, removing filter {:?}", device, effect);
                if let Err(e) = self.backend.remove_filter(device, *effect).await {
                    error!("Error removing filter {:?} for {}: {}", effect, device, e);
                }
            }
        }

        let effects_refs: Vec<&str> = active_effects.iter().map(|e| e.as_str()).collect();
        debug!("Triggering set_routing for {}: active={}, chain={:?}", device, active, effects_refs);
        if let Err(e) = self.backend.set_routing(device, active, &effects_refs, &order).await {
            error!("Error setting routing for {}: {}", device, e);
        }
    }

    pub fn set_effect_order(self: &Arc<Self>, device: &str, order_str: String) {
        let order: Vec<crate::effects::EffectType> = order_str
            .split(',')
            .filter_map(|s| crate::effects::EffectType::from_str(s.trim()))
            .collect();

        if order.is_empty() { return; }

        {
            let mut state = self.state.lock().unwrap();
            state.current.devices.entry(device.to_string()).or_default().effect_order = order.clone();
        }

        let _ = self.tx.send(BackendCmd::UpdateEffectOrder(device.to_string()));
    }

    pub fn update_device_name(&self, device: &str, new_name: String) {
        let mut state = self.state.lock().unwrap();
        state.current.devices.entry(device.to_string()).or_default().name = new_name;
    }

   pub fn set_device_volume(self: &Arc<Self>, device: &str, volume: f32) {
        if self.is_syncing.load(Ordering::SeqCst) {
            return;
        }
        info!("Queueing volume for {} to {:.2}", device, volume);
        let device_str = device.to_string();
        {
            let mut state = self.state.lock().unwrap();
            state.current.devices.entry(device_str.clone()).or_default().volume = volume;
        }
        let _ = self.tx.send(BackendCmd::UpdateVolume(device_str, volume));
    }

    pub fn set_effect_active(self: &Arc<Self>, device: &str, effect: crate::effects::EffectType, active: bool) {
        if self.is_syncing.load(Ordering::SeqCst) {
            return;
        }

        info!("Queueing {:?} for {} to active={}", effect, device, active);
        let is_device_active = {
            let mut state = self.state.lock().unwrap();
            let dev_settings = state.current.devices.entry(device.to_string()).or_default();
            match effect {
                crate::effects::EffectType::EQ => dev_settings.eq.active = active,
                crate::effects::EffectType::LPF => dev_settings.lpf.active = active,
                crate::effects::EffectType::HPF => dev_settings.hpf.active = active,
            }
            dev_settings.active
        };

        if is_device_active {
            let _ = self.tx.send(BackendCmd::UpdateEffectActive(device.to_string(), effect, active));
        }
    }

    pub fn set_effect_value(self: &Arc<Self>, device: &str, effect: crate::effects::EffectType, value: f32) {
        if self.is_syncing.load(Ordering::SeqCst) {
            return;
        }

        info!("Queueing {:?} value for {} to {:.2}", effect, device, value);
        let device_str = device.to_string();

        let is_device_active = {
            let mut state = self.state.lock().unwrap();
            let dev_settings = state.current.devices.entry(device_str.clone()).or_default();
            match effect {
                crate::effects::EffectType::EQ => dev_settings.eq.value = value,
                crate::effects::EffectType::LPF => dev_settings.lpf.value = value,
                crate::effects::EffectType::HPF => dev_settings.hpf.value = value,
            }
            dev_settings.active
        };

        if is_device_active {
            let _ = self.tx.send(BackendCmd::UpdateEffectValue(device_str, effect, value));
        }
    }

    pub fn set_eq_values(self: &Arc<Self>, device: &str, values: Vec<f32>) {
        if self.is_syncing.load(Ordering::SeqCst) {
            return;
        }

        info!("Queueing EQ bands for {}", device);
        let device_str = device.to_string();
        let is_active = {
            let mut state = self.state.lock().unwrap();
            let dev_settings = state.current.devices.entry(device_str.clone()).or_default();
            dev_settings.eq.bands = values.clone();
            dev_settings.active
        };

        if is_active {
            let _ = self.tx.send(BackendCmd::UpdateEQ(device_str, values));
        }
    }

    pub fn set_virtual_sink_volume(self: &Arc<Self>, volume: f32) {
        {
            let mut state = self.state.lock().unwrap();
            state.current.virtual_sink_volume = volume;
        }
        let _ = self.tx.send(BackendCmd::SetVirtualSinkVolume(volume));
    }


    pub fn is_device_dirty(&self, device: &str) -> bool {

        let state = self.state.lock().unwrap();
        let current = state.current.devices.get(device);
        let saved = state.saved.devices.get(device);
        current != saved
    }
}

