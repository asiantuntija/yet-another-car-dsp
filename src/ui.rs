use gtk4::prelude::*;
use gtk4::{Box, Button, Orientation, Switch};
use libadwaita as adw;
use adw::prelude::*;
use glib;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::rc::Rc;
use std::cell::RefCell;
use crate::state::AppState;
use tokio;

const HPF_MIN: f32 = 2000.0;
const HPF_MAX: f32 = 25000.0;
const HPF_SUM: f32 = HPF_MIN + HPF_MAX;

const LPF_MIN: f32 = 20.0;
const LPF_MAX: f32 = 200.0;

fn hpf_to_ui(val: f32) -> f32 {
    HPF_SUM - val.max(HPF_MIN)
}

fn ui_to_hpf(ui_val: f32) -> f32 {
    HPF_MAX - (ui_val - HPF_MIN)
}

pub struct DevicePageWidgets {
    pub page: gtk4::Overlay,
    pub sidebar_btn: gtk4::Button,
    pub sidebar_btn_label: gtk4::Label,
    pub sidebar_dirty_label: gtk4::Label,
    pub name_entry: gtk4::Entry,
    pub volume_scale: gtk4::Scale,
    pub toggle: gtk4::Switch,
    pub effects_btns: Vec<gtk4::Button>,
    pub effects_stack: gtk4::Stack,
    pub eq_toggle: gtk4::Switch,
    pub lpf_toggle: gtk4::Switch,
    pub lpf_value_scale: gtk4::Scale,
    pub lpf_value_label: gtk4::Label,
    pub hpf_toggle: gtk4::Switch,
    pub hpf_value_scale: gtk4::Scale,
    pub hpf_value_label: gtk4::Label,
    pub eq_bands: Vec<gtk4::Scale>,
    pub eq_band_labels: Vec<gtk4::Label>,
    pub eq_flat_btn: gtk4::Button,
    pub order_entry: gtk4::Entry,
    pub actions_box: gtk4::Box,
    pub save_btn: gtk4::Button,
    pub cancel_btn: gtk4::Button,
}

pub fn update_device_ui_state(state: &AppState, device_name: &str, widgets: &DevicePageWidgets) {
    let is_dirty = state.is_device_dirty(device_name);
    widgets.actions_box.set_visible(is_dirty);
    widgets.sidebar_dirty_label.set_visible(is_dirty);
    
    if is_dirty {
        widgets.sidebar_btn.add_css_class("dirty-button");
    } else {
        widgets.sidebar_btn.remove_css_class("dirty-button");
    }
}

pub fn add_device_to_ui(
    state: &Arc<AppState>,
    device_name: &str,
    sidebar: &gtk4::Box,
    content_stack: &gtk4::Stack,
) -> Arc<DevicePageWidgets> {
    let state_lock = state.state.lock().unwrap();
    let settings = state_lock.current.devices.get(device_name)
        .cloned()
        .unwrap_or_default();

    let widgets = Arc::new(build_device_page_ui(device_name, &settings));
    connect_device_page_signals(state, device_name, Arc::clone(&widgets));
    
    let btn = &widgets.sidebar_btn;
    btn.set_property("name", device_name); 
    
    content_stack.add_named(&widgets.page, Some(device_name));

    let stack_clone = content_stack.clone();
    let sidebar_clone = sidebar.clone();
    let name_clone = device_name.to_string();
    let btn_clone = btn.clone();
    btn.connect_clicked(move |_| {
        stack_clone.set_visible_child_name(&name_clone);

        // Highlight the active button
        let mut child = sidebar_clone.first_child();
        while let Some(c) = child {
            if let Some(button) = c.downcast_ref::<gtk4::Button>() {
                button.remove_css_class("suggested-action");
            }
            child = c.next_sibling();
        }
        btn_clone.add_css_class("suggested-action");
    });
    sidebar.append(btn);

    widgets

}
pub fn remove_device_from_ui(    device_name: &str,
    sidebar: &gtk4::Box,
    content_stack: &gtk4::Stack,
) {
    if let Some(page) = content_stack.child_by_name(device_name) {
        content_stack.remove(&page);
    }

    let mut child = sidebar.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        
        // History of attempts:
        // 1. <gtk4::Widget as gtk4::prelude::WidgetExt>::name(c) -> Failed (E0576: method not found in trait)
        // 2. c.name() -> Failed (E0599: Trait collision with ActionExt)
        // 3. c.property::<String>("name").as_deref() == Some(device_name) -> Failed (E0599: as_deref not found on String)
        if c.property::<String>("name") == device_name {
            sidebar.remove(&c);
            break;
        }
        child = next;
    }
}

fn sync_widgets_to_settings(widgets: &DevicePageWidgets, settings: &crate::session::DeviceSettings) {
    widgets.toggle.set_active(settings.active);
    widgets.name_entry.set_text(&settings.name);
    widgets.volume_scale.set_value(settings.volume as f64);
    
    let order_text = settings.effect_order.iter()
        .map(|e| e.as_str())
        .collect::<Vec<_>>()
        .join(",");
    widgets.order_entry.set_text(&order_text);

    widgets.sidebar_btn_label.set_text(if settings.name.is_empty() { "Unknown" } else { &settings.name });
    
    widgets.eq_toggle.set_active(settings.eq.active);
    widgets.lpf_toggle.set_active(settings.lpf.active);
    widgets.hpf_toggle.set_active(settings.hpf.active);
    
    widgets.lpf_value_scale.set_value(settings.lpf.value as f64);
    widgets.lpf_value_label.set_text(&format!("{:.0} Hz", settings.lpf.value));
    
    let hpf_ui_val = hpf_to_ui(settings.hpf.value);
    widgets.hpf_value_scale.set_value(hpf_ui_val as f64);
    widgets.hpf_value_label.set_text(&format!("{:.0} Hz", settings.hpf.value));

    for (i, scale) in widgets.eq_bands.iter().enumerate() {
        let val = settings.eq.bands.get(i).cloned().unwrap_or(0.0);
        scale.set_value(val as f64);
        widgets.eq_band_labels[i].set_text(&format!("{:.1}", val));
    }
}

fn build_device_page_ui(
    device_name: &str,
    settings: &crate::session::DeviceSettings,
) -> DevicePageWidgets {
    let page = gtk4::Overlay::new();
    let display_name = if settings.name.is_empty() { device_name } else { &settings.name };

    let btn = Button::new();
    let btn_box = Box::new(Orientation::Vertical, 0);
    btn_box.set_valign(gtk4::Align::Center);
    
    let btn_name_lbl = gtk4::Label::new(Some(display_name));
    let sidebar_dirty_label = gtk4::Label::builder()
        .label("unsaved changes")
        .build();
    sidebar_dirty_label.add_css_class("dirty-indicator");
    sidebar_dirty_label.set_visible(false);

    btn_box.append(&btn_name_lbl);
    btn_box.append(&sidebar_dirty_label);
    btn.set_child(Some(&btn_box));
    btn.set_vexpand(true);

    let current_name = if settings.name.is_empty() { device_name.to_string() } else { settings.name.clone() };
    let name_entry = gtk4::Entry::builder()
        .text(&current_name)
        .placeholder_text("Custom name...")
        .build();

    let actions_box = Box::new(Orientation::Horizontal, 10);
    actions_box.add_css_class("actions-box");
    actions_box.set_halign(gtk4::Align::Center);
    actions_box.set_valign(gtk4::Align::End);
    actions_box.set_visible(false);

    let save_btn = Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    let cancel_btn = Button::with_label("Cancel");
    cancel_btn.add_css_class("destructive-action");

    actions_box.append(&save_btn);
    actions_box.append(&cancel_btn);
    actions_box.set_margin_bottom(20);
    actions_box.set_margin_top(10);

    let settings_group = adw::PreferencesGroup::builder()
        .title("Device Settings")
        .build();

    let is_active = settings.active;
    let active_row = adw::ActionRow::builder()
        .title("Activated")
        .subtitle("Route audio to this device")
        .build();

    let toggle = Switch::new();
    toggle.set_active(is_active);
    toggle.set_valign(gtk4::Align::Center);
    active_row.add_suffix(&toggle);

    let props_group = adw::PreferencesGroup::builder()
        .title("Device Properties")
        .build();

    let dev_info_row = adw::ActionRow::builder()
        .title("Node ID")
        .subtitle(device_name)
        .build();

    let name_row = adw::ActionRow::builder()
        .title("Display Name")
        .build();
    name_row.add_suffix(&name_entry);

    let vol_adj = gtk4::Adjustment::new(settings.volume as f64, 0.0, 1.0, 0.01, 0.1, 0.0);
    let volume_scale = gtk4::Scale::new(Orientation::Horizontal, Some(&vol_adj));
    volume_scale.set_draw_value(false);
    volume_scale.set_hexpand(true);
    volume_scale.set_valign(gtk4::Align::Center);

    let vol_row = adw::ActionRow::builder()
        .title("Master Volume")
        .subtitle("Adjust output level")
        .build();
    vol_row.add_suffix(&volume_scale);

    let order_entry = gtk4::Entry::builder()
        .placeholder_text("EQ,LPF,HPF")
        .build();
    let order_row = adw::ActionRow::builder()
        .title("Effect Order")
        .subtitle("Comma separated (e.g. EQ,LPF,HPF)")
        .build();
    order_row.add_suffix(&order_entry);

    props_group.add(&dev_info_row);
    props_group.add(&name_row);
    props_group.add(&vol_row);
    props_group.add(&order_row);

    settings_group.add(&active_row);
    settings_group.add(&props_group);

    // Effects Selection
    let btn_box = Box::new(Orientation::Horizontal, 5);
    btn_box.set_halign(gtk4::Align::Fill);
    btn_box.set_hexpand(true);

    let mut buttons = Vec::new();
    for effect in ["EQ", "LPF", "HPF"] {
        let btn = Button::with_label(effect);
        btn.set_hexpand(true);
        buttons.push(btn.clone());
        btn_box.append(&btn);
    }

    // Set initial state (EQ active)
    buttons[0].add_css_class("suggested-action");

    let effects_stack = gtk4::Stack::new();
    effects_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);

    let create_effect_page = |name: &str, active: bool| {
        let page = Box::new(Orientation::Vertical, 0);
        let row = adw::ActionRow::builder()
            .title(format!("{} Activated", name))
            .subtitle("Enable this effect")
            .build();
        let sw = Switch::new();
        sw.set_active(active);
        sw.set_valign(gtk4::Align::Center);
        row.add_suffix(&sw);
        page.append(&row);
        (page, sw)
    };

    let (eq_page, eq_toggle) = create_effect_page("EQ", settings.eq.active);
    
    // EQ Bands UI
    let eq_bands_box = Box::new(Orientation::Horizontal, 6);
    eq_bands_box.set_height_request(300);
    eq_bands_box.set_margin_start(24);
    eq_bands_box.set_margin_end(24);
    eq_bands_box.set_margin_top(12);
    eq_bands_box.set_margin_bottom(50);

    let eq_frequencies = [
        "20", "31.5", "40", "63", "100", "160", "250", "400", 
        "630", "1k", "1.6k", "2.5k", "4k", "6.3k", "10k", "16k"
    ];
    let mut eq_bands = Vec::new();
    let mut eq_band_labels = Vec::new();

    for (i, freq) in eq_frequencies.iter().enumerate() {
        let col = Box::new(Orientation::Vertical, 6);
        col.set_hexpand(true);
        col.set_vexpand(true);
        
        let db_lbl = gtk4::Label::builder().label("dB").build();
        let val_lbl = gtk4::Label::builder()
            .label("0.0")
            .build();
        val_lbl.add_css_class("eq-value-label");
        
        let initial_val = settings.eq.bands.get(i).cloned().unwrap_or(0.0);
        val_lbl.set_text(&format!("{:.1}", initial_val));
        let adj = gtk4::Adjustment::new(initial_val as f64, -12.0, 12.0, 0.1, 1.0, 0.0);
        let scale = gtk4::Scale::new(Orientation::Vertical, Some(&adj));
        scale.set_draw_value(false);
        scale.set_inverted(true);
        scale.set_vexpand(true);
        
        let freq_val_lbl = gtk4::Label::builder().label(*freq).build();
        let freq_unit_lbl = gtk4::Label::builder().label("Hz").build();
        
        col.append(&db_lbl);
        col.append(&val_lbl);
        col.append(&scale);
        col.append(&freq_val_lbl);
        col.append(&freq_unit_lbl);
        eq_bands_box.append(&col);
        eq_bands.push(scale);
        eq_band_labels.push(val_lbl);
    }
    eq_page.append(&eq_bands_box);

    let flat_btn = Button::with_label("Flat");
    flat_btn.set_halign(gtk4::Align::End);
    flat_btn.set_margin_top(15);
    flat_btn.set_margin_end(15);
    flat_btn.set_margin_bottom(20);
    eq_page.append(&flat_btn);

    // Custom LPF page with frequency control
    let lpf_page = Box::new(Orientation::Vertical, 0);
    let lpf_row = adw::ActionRow::builder()
        .title("LPF Activated")
        .subtitle("Enable this effect")
        .build();
    let lpf_toggle = Switch::new();
    lpf_toggle.set_active(settings.lpf.active);
    lpf_toggle.set_valign(gtk4::Align::Center);
    lpf_row.add_suffix(&lpf_toggle);
    lpf_page.append(&lpf_row);
 
    let lpf_adj = gtk4::Adjustment::new(settings.lpf.value.max(LPF_MIN) as f64, LPF_MIN as f64, LPF_MAX as f64, 1.0, 10.0, 0.0);
    let lpf_value_scale = gtk4::Scale::new(Orientation::Horizontal, Some(&lpf_adj));
    lpf_value_scale.set_draw_value(false);
    lpf_value_scale.add_css_class("lpf-scale");
    lpf_value_scale.set_hexpand(true);
    lpf_value_scale.set_margin_start(24);
    lpf_value_scale.set_margin_end(24);
    lpf_value_scale.set_margin_top(12);

    let lpf_value_label = gtk4::Label::builder()
        .label(&format!("{:.0} Hz", settings.lpf.value))
        .halign(gtk4::Align::Center)
        .build();
    lpf_value_label.add_css_class("freq-value");

    let freq_title = gtk4::Label::builder()
        .label("Cutoff Frequency")
        .halign(gtk4::Align::Center)
        .build();
    freq_title.set_margin_top(16);

    let freq_control_box = Box::new(Orientation::Vertical, 0);
    freq_control_box.set_margin_bottom(20);
    freq_control_box.append(&freq_title);
    freq_control_box.append(&lpf_value_scale);
    freq_control_box.append(&lpf_value_label);
    
    lpf_page.append(&freq_control_box);

    // HPF page with frequency control
    let hpf_page = Box::new(Orientation::Vertical, 0);
    let hpf_row = adw::ActionRow::builder()
        .title("HPF Activated")
        .subtitle("Enable this effect")
        .build();
    let hpf_toggle = Switch::new();
    hpf_toggle.set_active(settings.hpf.active);
    hpf_toggle.set_valign(gtk4::Align::Center);
    hpf_row.add_suffix(&hpf_toggle);
    hpf_page.append(&hpf_row);
 
    let hpf_logical_val = settings.hpf.value.max(HPF_MIN);
    let hpf_ui_val = hpf_to_ui(hpf_logical_val);
    let hpf_adj = gtk4::Adjustment::new(hpf_ui_val as f64, HPF_MIN as f64, HPF_MAX as f64, 50.0, 500.0, 0.0);
    let hpf_value_scale = gtk4::Scale::new(Orientation::Horizontal, Some(&hpf_adj));
    hpf_value_scale.set_draw_value(false);
    hpf_value_scale.add_css_class("hpf-scale");
    hpf_value_scale.set_inverted(true);
    hpf_value_scale.set_hexpand(true);
    hpf_value_scale.set_margin_start(24);
    hpf_value_scale.set_margin_end(24);
    hpf_value_scale.set_margin_top(12);

    let hpf_value_label = gtk4::Label::builder()
        .label(&format!("{:.0} Hz", settings.hpf.value))
        .halign(gtk4::Align::Center)
        .build();
    hpf_value_label.add_css_class("freq-value");

    let hpf_freq_title = gtk4::Label::builder()
        .label("Cutoff Frequency")
        .halign(gtk4::Align::Center)
        .build();
    hpf_freq_title.set_margin_top(16);

    let hpf_freq_control_box = Box::new(Orientation::Vertical, 0);
    hpf_freq_control_box.set_margin_bottom(20);
    hpf_freq_control_box.append(&hpf_freq_title);
    hpf_freq_control_box.append(&hpf_value_scale);
    hpf_freq_control_box.append(&hpf_value_label);
    
    hpf_page.append(&hpf_freq_control_box);
    
    effects_stack.add_named(&eq_page, Some("EQ"));
    effects_stack.add_named(&lpf_page, Some("LPF"));
    effects_stack.add_named(&hpf_page, Some("HPF"));

    let effects_group = adw::PreferencesGroup::builder()
        .title("Effects")
        .build();

    effects_group.add(&btn_box);
    
    let preferences_page = adw::PreferencesPage::new();
    preferences_page.add(&settings_group);
    preferences_page.add(&effects_group);

    let page_container = Box::new(Orientation::Vertical, 0);
    page_container.append(&preferences_page);
    page_container.append(&effects_stack);

    let scrolled_window = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&page_container)
        .build();
    
    page.set_child(Some(&scrolled_window));
    page.add_overlay(&actions_box);

    DevicePageWidgets {
        page,
        sidebar_btn: btn,
        sidebar_btn_label: btn_name_lbl,
        sidebar_dirty_label,
        name_entry,
        volume_scale,
        toggle,
        effects_btns: buttons,
        effects_stack,
        eq_toggle,
        lpf_toggle,
        lpf_value_scale,
        lpf_value_label,
        hpf_toggle,
        hpf_value_scale,
        hpf_value_label,
        eq_bands,
        eq_band_labels,
        eq_flat_btn: flat_btn,
        order_entry,
        actions_box,
        save_btn,
        cancel_btn,
    }
}

fn debounce_spawn<F>(timer: &Rc<RefCell<Option<tokio::task::JoinHandle<()>>>>, f: F) 
where F: std::future::Future<Output = ()> + Send + 'static 
{
    if let Some(handle) = timer.borrow_mut().take() {
        handle.abort();
    }
    let handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        f.await;
    });
    *timer.borrow_mut() = Some(handle);
}

fn connect_device_page_signals(
    state: &Arc<AppState>,
    device_name: &str,
    widgets: Arc<DevicePageWidgets>,
) {
    connect_routing_and_name_signals(state, device_name, &widgets);
    connect_eq_signals(state, device_name, &widgets);
    connect_effect_toggles(state, device_name, &widgets);
    connect_value_scales(state, device_name, &widgets);
    connect_save_cancel_signals(state, device_name, &widgets);
}

fn connect_eq_signals(state: &Arc<AppState>, device_name: &str, widgets: &Arc<DevicePageWidgets>) {
    let state_clone = Arc::clone(state);
    let dev_clone = device_name.to_string();
    let bands_clone = widgets.eq_bands.clone();
    let labels_clone = widgets.eq_band_labels.clone();
    
    // Flat button logic
    let s_flat = Arc::clone(&state_clone);
    let d_flat = dev_clone.clone();
    let b_flat = bands_clone.clone();
    let l_flat = labels_clone.clone();
    widgets.eq_flat_btn.connect_clicked(move |_| {
        let zeros = vec![0.0; 16];
        s_flat.set_eq_values(&d_flat, zeros);
        for (i, scale) in b_flat.iter().enumerate() {
            scale.set_value(0.0);
            l_flat[i].set_text("0.0");
        }
    });

    // We need a way to collect all current slider values
    let eq_timer = Rc::new(RefCell::new(None::<tokio::task::JoinHandle<()>>));
    
    for (i, scale) in widgets.eq_bands.iter().enumerate() {
        let scale = scale.clone();
        let s_inner = Arc::clone(&state_clone);
        let d_inner = dev_clone.clone();
        let b_inner = bands_clone.clone();
        let l_inner = labels_clone.clone();
        let t_inner = Rc::clone(&eq_timer);
        let idx = i;
        
        let scale_for_closure = scale.clone();
        scale.connect_value_changed(move |_| {
            let val = scale_for_closure.value() as f32;
            l_inner[idx].set_text(&format!("{:.1}", val));

            let current_vals: Vec<f32> = b_inner.iter().map(|s| s.value() as f32).collect();
            let s = Arc::clone(&s_inner);
            let d = d_inner.clone();
            let v = current_vals.clone();
            
            debounce_spawn(&t_inner, async move {
                s.set_eq_values(&d, v);
            });
        });
    }
}

fn connect_routing_and_name_signals(state: &Arc<AppState>, device_name: &str, widgets: &Arc<DevicePageWidgets>) {
    let dev_clone = device_name.to_string();
    let state_clone = Arc::clone(state);

    widgets.toggle.connect_state_set(move |_, s| {
        state_clone.set_device_routing(&dev_clone, s);
        glib::Propagation::Proceed
    });

    let state_vol_clone = Arc::clone(state);
    let dev_vol_clone = device_name.to_string();
    widgets.volume_scale.connect_value_changed(move |scale| {
        state_vol_clone.set_device_volume(&dev_vol_clone, scale.value() as f32);
    });

    let state_order_clone = Arc::clone(state);
    let dev_order_clone = device_name.to_string();
    widgets.order_entry.connect_changed(move |entry| {
        state_order_clone.set_effect_order(&dev_order_clone, entry.text().to_string());
    });

    let state_name_clone = Arc::clone(state);
    let dev_name_clone = device_name.to_string();
    let btn_lbl_clone = widgets.sidebar_btn_label.clone();
    widgets.name_entry.connect_changed(move |entry| {
        let new_name = entry.text().to_string();
        state_name_clone.update_device_name(&dev_name_clone, new_name.clone());
        btn_lbl_clone.set_text(&new_name);
    });
}

fn connect_effect_toggles(state: &Arc<AppState>, device_name: &str, widgets: &Arc<DevicePageWidgets>) {
    let stack_clone = widgets.effects_stack.clone();
    let btns_clone = widgets.effects_btns.clone();
    
    for (i, btn) in widgets.effects_btns.iter().enumerate() {
        let stack_inner = stack_clone.clone();
        let btns_inner = btns_clone.clone();
        let effect_name = match i {
            0 => "EQ", 1 => "LPF", _ => "HPF",
        };

        btn.connect_clicked(move |_| {
            stack_inner.set_visible_child_name(effect_name);
            for (j, b) in btns_inner.iter().enumerate() {
                if i == j { b.add_css_class("suggested-action"); } 
                else { b.remove_css_class("suggested-action"); }
            }
        });
    }

    let effects = [
        ("EQ", widgets.eq_toggle.clone()),
        ("LPF", widgets.lpf_toggle.clone()),
        ("HPF", widgets.hpf_toggle.clone()),
    ];

    for (name, sw) in effects {
        let state_eff_clone = Arc::clone(state);
        let dev_eff_clone = device_name.to_string();
        let name_eff_clone = name.to_string();
        sw.connect_state_set(move |_, s| {
            if let Some(eff_type) = crate::effects::EffectType::from_str(&name_eff_clone) {
                state_eff_clone.set_effect_active(&dev_eff_clone, eff_type, s);
            }
            glib::Propagation::Proceed
        });
    }
}

fn connect_value_scales(state: &Arc<AppState>, device_name: &str, widgets: &Arc<DevicePageWidgets>) {
    let state_val_clone = Arc::clone(state);
    let dev_val_clone = device_name.to_string();
    let lbl_val_clone = widgets.lpf_value_label.clone();
    let lpf_timer = Rc::new(RefCell::new(None::<tokio::task::JoinHandle<()>>));

    widgets.lpf_value_scale.connect_value_changed(move |scale| {
        let val = scale.value() as f32;
        lbl_val_clone.set_text(&format!("{:.0} Hz", val));
        let s_clone = Arc::clone(&state_val_clone);
        let d_clone = dev_val_clone.clone();
        debounce_spawn(&lpf_timer, async move {
            s_clone.set_effect_value(&d_clone, crate::effects::EffectType::LPF, val);
        });
    });

    let state_hpf_clone = Arc::clone(state);
    let dev_hpf_clone = device_name.to_string();
    let lbl_hpf_clone = widgets.hpf_value_label.clone();
    let hpf_timer = Rc::new(RefCell::new(None::<tokio::task::JoinHandle<()>>));

    widgets.hpf_value_scale.connect_value_changed(move |scale| {
        let val = scale.value() as f32;
        let actual_val = ui_to_hpf(val);
        lbl_hpf_clone.set_text(&format!("{:.0} Hz", actual_val));
        let s_clone = Arc::clone(&state_hpf_clone);
        let d_clone = dev_hpf_clone.clone();
        debounce_spawn(&hpf_timer, async move {
            s_clone.set_effect_value(&d_clone, crate::effects::EffectType::HPF, actual_val);
        });
    });
}

fn connect_save_cancel_signals(state: &Arc<AppState>, device_name: &str, widgets: &Arc<DevicePageWidgets>) {
    let state_save_clone = Arc::clone(state);
    let dev_save_clone = device_name.to_string();
    widgets.save_btn.connect_clicked(move |_| {
        state_save_clone.save_device(&dev_save_clone);
    });

    let state_cancel_clone = Arc::clone(state);
    let dev_cancel_clone = device_name.to_string();
    let widgets_cancel_clone = Arc::clone(&widgets);
    widgets.cancel_btn.connect_clicked(move |_| {
        let (reverted_s, is_active, name) = state_cancel_clone.revert_device(&dev_cancel_clone);
        
        // Block individual widget signals from triggering backend tasks
        state_cancel_clone.is_syncing.store(true, Ordering::SeqCst);
        sync_widgets_to_settings(&widgets_cancel_clone, &reverted_s);
        state_cancel_clone.is_syncing.store(false, Ordering::SeqCst);

        widgets_cancel_clone.sidebar_btn_label.set_text(&name);

        // Trigger a single consolidated backend update
        let state_final = Arc::clone(&state_cancel_clone);
        let dev_final = dev_cancel_clone.clone();
        tokio::spawn(async move {
            state_final.apply_backend_state(&dev_final, is_active).await;
        });
    });
}
