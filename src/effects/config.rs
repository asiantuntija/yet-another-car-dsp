use std::fs;
use crate::effects::types::{EffectType, BackendError};

pub fn write_filter_config(device: &str, effect: EffectType, value: f32) -> Result<String, BackendError> {
    let node_name = format!("yacd_filter_{}_{}", device, effect.as_str());
    let filter_type = effect.filter_type_id();
    
    let config_content = format!(
        r#"bypass = false
        g_in = 0.00 db
        g_out = 0.00 db
        mode = 0
        react = 0.20000
        shift = 0.00 db
        zoom = -36.00 db
        ife_l = true
        ofe_l = true
        ife_r = true
        ofe_r = true
        bal = 0.00000
        ft = {filter_type}
        fm = 2
        s = 1
        f = {value}
        w = 4.00000
        g = 0.00 db
        q = 0.00000"#,
        filter_type = filter_type,
        value = value
    );

    let config_path = crate::paths::get_effects_dir().join(format!("{}.cfg", node_name));
    fs::write(&config_path, config_content).map_err(BackendError::IoError)?;
    
    Ok(config_path.to_string_lossy().to_string())
}

pub fn write_eq_config(device: &str, bands: &[f32]) -> Result<String, BackendError> {
    let node_name = format!("yacd_filter_{}_EQ", device);
    
    let mut bands_config = String::new();
    for (i, val) in bands.iter().enumerate() {
        bands_config.push_str(&format!("xe_{} = true\n", i));
        bands_config.push_str(&format!("g_{} = {:.2} db\n", i, val));
    }

    let config_content = format!(
r#"bypass = false
        g_in = 0.00 db
        g_out = 0.00 db
        mode = 0
        slope = 0
        react = 0.20000
        shift = 0.00 db
        zoom = -36.00 db
        send = ""
        return = ""
        ife_l = true
        ofe_l = true
        rfe_l = true
        ife_r = true
        ofe_r = true
        rfe_r = true
        bal = 0.00000
{} "#, bands_config);

    let config_path = crate::paths::get_effects_dir().join(format!("{}.cfg", node_name));
    fs::write(&config_path, config_content).map_err(BackendError::IoError)?;
    
    Ok(config_path.to_string_lossy().to_string())
}

