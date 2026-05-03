use gtk4::prelude::*;
use gtk4::{
    Box, Orientation, Stack, CssProvider, PolicyType
};
use libadwaita as adw;
use adw::prelude::*;

mod backend;
mod session;
mod state;
mod style;
mod ui;
mod paths;
mod effects;

use glib::{ControlFlow, timeout_add_local};
use backend::PipeWireBackend;
use effects::{EffectType, types::FilterConfigValue};
use state::AppState;
use style::APP_CSS;
use ui::add_device_to_ui;
use log::{error,info};
use std::collections::{HashSet, HashMap};
use std::sync::{Arc, Once, OnceLock};
use std::rc::Rc;
use std::cell::RefCell;
use tokio::sync::mpsc; // Added for fallback channel
use std::sync::Mutex;   // Added for receiver sharing
                        //
thread_local! {
    static HOLD_GUARD: RefCell<Option<gio::ApplicationHoldGuard>> = RefCell::new(None);
}

static APP_STATE: OnceLock<Arc<AppState>> = OnceLock::new();
static BACKEND_INITIALIZED: Once = Once::new();
static APP_EVENT_SENDER: OnceLock<mpsc::UnboundedSender<AppEvent>> = OnceLock::new();
static APP_EVENT_RECEIVER: OnceLock<Arc<Mutex<mpsc::UnboundedReceiver<AppEvent>>>> = OnceLock::new();

enum AppEvent {
    DeviceAdded(String),
    DeviceRemoved(String),
}

fn main() {
    // Initialize logger
    env_logger::init();
    info!("Starting Car DSP...");

    // Ensure systemd service is present and points to current binary
    match paths::ensure_systemd_service() {
        Ok(updated) => {
            if updated {
                if let Err(e) = paths::restart_systemd_service() {
                    error!("Failed to restart systemd service after update: {}", e);
                }
            }
        }
        Err(e) => error!("Failed to manage systemd service: {}", e),
    }

    // Ensure XDG config directories exist
    if let Err(e) = paths::ensure_config_dirs() {
        error!("Failed to ensure config directories: {}", e);
    }

    // Initialize tokio runtime for the backend
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let app = adw::Application::builder()
        .application_id("com.riku.car-dsp")
        .build();

    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &adw::Application) {
    info!("Building user interface...");
    
    // 1. Initialize Global State and Backend Workers (once per process lifetime)
    let state = APP_STATE.get_or_init(|| {
        let session_path = paths::get_session_path();
        let (backend_tx, backend_rx) = mpsc::unbounded_channel();
        let s = Arc::new(AppState::new(PipeWireBackend::new(), &session_path, backend_tx));
        
        // Start the sequential backend worker
        let s_worker = Arc::clone(&s);
        tokio::spawn(async move {
            s_worker.backend_worker(backend_rx).await;
        });
        
        s
    });

    BACKEND_INITIALIZED.call_once(|| {
        // Background Initialization
        let state_init = Arc::clone(state);
        let (sender, receiver) = mpsc::unbounded_channel::<AppEvent>();
        
        // Store the receiver in a way that build_ui can access it. 
        // Since build_ui is called multiple times, we need a global for the receiver.
        // However, we can use a simpler approach: move the receiver into a global OnceLock as well.
        APP_EVENT_RECEIVER.set(Arc::new(Mutex::new(receiver))).ok();

        // let sender_init = sender.clone();
        tokio::spawn(async move {
            if let Err(e) = state_init.backend.start_virtual_sink().await {
                error!("Error starting virtual sink: {}", e);
            }

            let session_data = state_init.state.lock().unwrap().current.clone();
            
            if let Ok(available_devices) = state_init.backend.get_output_devices().await {
                info!("Available output devices for init: {:?}", available_devices);
                for (dev, settings) in &session_data.devices {
                    if available_devices.contains(dev) && settings.active {
                        let mut active_effects = Vec::new();
                        if settings.eq.active {
                            if let Err(e) = state_init.backend.spawn_filter(dev, EffectType::EQ, FilterConfigValue::Bands(settings.eq.bands.clone())).await {
                                error!("Error spawning initial EQ for {}: {}", dev, e);
                            }
                            active_effects.push("EQ");
                        }
                        if settings.lpf.active { 
                            if let Err(e) = state_init.backend.spawn_filter(dev, EffectType::LPF, FilterConfigValue::Single(settings.lpf.value)).await {
                                error!("Error spawning initial LPF for {}: {}", dev, e);
                            }
                            active_effects.push("LPF"); 
                        }
                        if settings.hpf.active { 
                            if let Err(e) = state_init.backend.spawn_filter(dev, EffectType::HPF, FilterConfigValue::Single(settings.hpf.value)).await {
                               error!("Error spawning initial HPF for {}: {}", dev, e);
                            }
                            active_effects.push("HPF"); 
                        }

                        if let Err(e) = state_init.backend.set_routing(dev, true, &active_effects, &settings.effect_order).await {
                            error!("Error applying initial routing for {}: {}", dev, e);
                        }
                    }
                }
            } else {
                error!("Failed to get output devices during initialization");
            }
        });

        // Background scanning for new devices
        let state_scan = Arc::clone(state);
        APP_EVENT_SENDER.set(sender).unwrap(); // Use global sender
        
        tokio::spawn(async move {
            let mut known_devices = HashSet::<String>::new();
            if let Ok(devs) = state_scan.backend.get_output_devices().await {
                known_devices.extend(devs);
            }

           loop {
                tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
                match state_scan.backend.get_output_devices().await {
                    Ok(current_devs) => {
                        let current_set: HashSet<_> = current_devs.into_iter().collect();

                        for dev in &known_devices {
                            if !current_set.contains(dev) {
                                info!("Device lost: {}", dev);
                                let _ = APP_EVENT_SENDER.get().unwrap().send(AppEvent::DeviceRemoved(dev.clone()));
                            }
                        }

                        for dev in &current_set {
                            if !known_devices.contains(dev) {
                                info!("New device discovered: {}", dev);
                                let is_active = {
                                    let state_lock = state_scan.state.lock().unwrap();
                                    state_lock.current.devices.get(dev).map(|s| s.active).unwrap_or(false)
                                };
                                if is_active {
                                    info!("Device {} is marked active, applying routing...", dev);
                                    state_scan.set_device_routing(dev, true);
                                }
                                let _ = APP_EVENT_SENDER.get().unwrap().send(AppEvent::DeviceAdded(dev.clone()));
                            }
                        }
                        known_devices = current_set;
                    }
                    Err(e) => {
                        error!("Error scanning for output devices: {}", e);
                    }
                }
            }
        });
    });

     // Access global event channels
    let _sender = APP_EVENT_SENDER.get().expect("Sender not initialized");
    let receiver = APP_EVENT_RECEIVER.get().expect("Receiver not initialized");

    let backend_cleanup = Arc::clone(state);
    app.connect_shutdown(move |_| {
        info!("Shutting down Car DSP...");
        if let Err(e) = backend_cleanup.backend.cleanup_sync() {
            error!("Error during synchronous cleanup: {}", e);
        }
    });

    let provider = CssProvider::new();
    provider.load_from_data(APP_CSS);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Car DSP")
        .default_width(600)
        .default_height(400)
        .build();

    let header_bar = adw::HeaderBar::new();
    let main_box = Box::new(Orientation::Horizontal, 0);
    let sidebar = Box::new(Orientation::Vertical, 0);
    let device_list_box = Box::new(Orientation::Vertical, 0);
    
    let scroll_window = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vexpand(true)
        .child(&device_list_box)
        .build();

    let content_stack = Stack::new();

    sidebar.add_css_class("sidebar");
    sidebar.set_margin_start(12);
    sidebar.set_margin_end(12);
    sidebar.set_margin_top(12);
    sidebar.set_margin_bottom(12);
    sidebar.set_width_request(220);

    // Global Save All button
    let save_all_btn = gtk4::Button::builder()
        .label("Save All Changes")
        .visible(false)
        .build();
    save_all_btn.add_css_class("suggested-action");
    save_all_btn.set_size_request(-1, 40); 

    let state_save_all = Arc::clone(&state);
    save_all_btn.connect_clicked(move |_| {
        state_save_all.save_all();
        info!("All session changes saved to disk");
    });

    // Container to reserve space for the save button to prevent layout jumping
    let footer_box = gtk4::Box::new(Orientation::Vertical, 0);
    footer_box.set_size_request(-1, 100); 
    footer_box.set_vexpand(false);
    footer_box.set_valign(gtk4::Align::Center);

    let footer_spacer = gtk4::Box::new(Orientation::Vertical, 0);
    footer_spacer.set_vexpand(true);
    footer_box.append(&footer_spacer);

    footer_box.append(&save_all_btn);

    // Sidebar layout: [Scrollable Device List] -> [Footer]
    sidebar.append(&scroll_window);
    sidebar.append(&footer_box);

    let state_ui_clone = Arc::clone(&state);
    let device_list_box_clone = device_list_box.clone();
    let stack_clone = content_stack.clone();
    let save_all_btn_clone = save_all_btn.clone();
    
    // Wrap widgets in Rc<RefCell> so both the event handler and the timer can access them
    let device_widgets = Rc::new(RefCell::new(HashMap::<String, Arc<ui::DevicePageWidgets>>::new()));

    // Track timers so they can be stopped when the window is closed
    let active_timers = Rc::new(RefCell::new(Vec::new()));

    // 1. Handle events via polling
    let widgets_for_receiver = Rc::clone(&device_widgets);
    let state_for_receiver = Arc::clone(&state_ui_clone);
    let list_for_receiver = device_list_box_clone.clone();
    let stack_for_receiver = stack_clone.clone();
    let receiver_for_poll = Arc::clone(&receiver);

    // Populate UI with currently available devices
    let state_for_init = Arc::clone(&state_ui_clone);
    let list_for_init = device_list_box_clone.clone();
    let stack_for_init = stack_clone.clone();
    let widgets_for_init = Rc::clone(&device_widgets);
    glib::MainContext::default().spawn_local(async move {
        if let Ok(devices) = state_for_init.backend.get_output_devices().await {
            glib::idle_add_local(move || {
                let mut widgets = widgets_for_init.borrow_mut();
                for (i, device_name) in devices.iter().enumerate() {
                    let w = add_device_to_ui(&state_for_init, device_name, &list_for_init, &stack_for_init);
                    if i == 0 {
                        stack_for_init.set_visible_child_name(device_name);
                        w.sidebar_btn.add_css_class("suggested-action");
                    }
                    widgets.insert(device_name.clone(), w);
                }
                ControlFlow::Break
            });
        }
    });

    let event_timer_id = timeout_add_local(std::time::Duration::from_millis(50), move || {
        let mut rx = receiver_for_poll.lock().unwrap();
        while let Ok(event) = rx.try_recv() {
            let mut widgets = widgets_for_receiver.borrow_mut();
            match event {
                AppEvent::DeviceAdded(dev_name) => {
                    let w = add_device_to_ui(&state_for_receiver, &dev_name, &list_for_receiver, &stack_for_receiver);
                    widgets.insert(dev_name, w);
                }
                AppEvent::DeviceRemoved(dev_name) => {
                    ui::remove_device_from_ui(&dev_name, &list_for_receiver, &stack_for_receiver);
                    widgets.remove(&dev_name);
                }
            }
        }
        ControlFlow::Continue
    });
    active_timers.borrow_mut().push(event_timer_id);

    // 2. Separate timer for UI state polling (dirty indicators)
    let widgets_for_timer = Rc::clone(&device_widgets);
    let state_for_timer = Arc::clone(&state_ui_clone);
    let btn_for_timer = save_all_btn_clone.clone();

    let state_timer_id = timeout_add_local(std::time::Duration::from_millis(200), move || {
        let is_dirty = state_for_timer.is_any_dirty();
        btn_for_timer.set_visible(is_dirty);

        let widgets = widgets_for_timer.borrow();
        for (dev_name, w) in widgets.iter() {
            ui::update_device_ui_state(&state_for_timer, dev_name, w);
        }
        ControlFlow::Continue
    });
    active_timers.borrow_mut().push(state_timer_id);

    // Handle window close request
    let app_clone = app.clone();
    let timers_clone = Rc::clone(&active_timers);
    window.connect_close_request(move |win| {
        let app_inner = app_clone.clone();
        let win_clone = win.clone();
        let timers_inner = timers_clone.clone();

        let dialog = adw::AlertDialog::builder()
            .title("Exit Car DSP")
            .body("Do you want to close the application completely or keep it running in the background?")
            .build();

        dialog.add_response("exit", "Exit Completely");
        dialog.add_response("background", "Run in Background");
        dialog.set_response_appearance("exit", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("exit"));
        dialog.set_close_response("exit");

        dialog.choose(Some(win), None::<&gio::Cancellable>, move |response| {
            if response == "background" {
                for id in timers_inner.borrow_mut().drain(..) {
                    id.remove();
                }
                let guard = app_inner.hold();
                HOLD_GUARD.with(|hg| {
                    *hg.borrow_mut() = Some(guard);
                });
                win_clone.destroy();
            } else {
                app_inner.quit();
            }
        });

       glib::Propagation::Stop
    });

    main_box.append(&sidebar);
    main_box.append(&content_stack);
    main_box.set_hexpand(true);
    content_stack.set_hexpand(true);
    content_stack.set_vexpand(true);

    let content_box = Box::new(Orientation::Vertical, 0);

    content_box.append(&header_bar);
    content_box.append(&main_box);

    window.set_content(Some(&content_box));
    window.present();
}

