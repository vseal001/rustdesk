use std::{
    collections::HashMap,
    iter::FromIterator,
    sync::{Arc, Mutex},
};

use sciter::Value;

use hbb_common::{
    allow_err,
    config::{LocalConfig, PeerConfig},
    log,
};

#[cfg(not(any(feature = "flutter", feature = "cli")))]
use crate::ui_session_interface::Session;
use crate::{common::get_app_name, ipc, ui_interface::*};

mod cm;
#[cfg(feature = "inline")]
pub mod inline;
pub mod remote;

#[allow(dead_code)]
type Status = (i32, bool, i64, String);

lazy_static::lazy_static! {
    // stupid workaround for https://sciter.com/forums/topic/crash-on-latest-tis-mac-sdk-sometimes/
    static ref STUPID_VALUES: Mutex<Vec<Arc<Vec<Value>>>> = Default::default();
}

#[cfg(not(any(feature = "flutter", feature = "cli")))]
lazy_static::lazy_static! {
    pub static ref CUR_SESSION: Arc<Mutex<Option<Session<remote::SciterHandler>>>> = Default::default();
}

struct UIHostHandler;

pub fn start(args: &mut [String]) {
    #[cfg(target_os = "macos")]
    crate::platform::delegate::show_dock();
    #[cfg(all(target_os = "linux", feature = "inline"))]
    {
        let app_dir = std::env::var("APPDIR").unwrap_or("".to_string());
        let mut so_path = "/usr/share/rustdesk/libsciter-gtk.so".to_owned();
        for (prefix, dir) in [
            ("", "/usr"),
            ("", "/app"),
            (&app_dir, "/usr"),
            (&app_dir, "/app"),
        ]
        .iter()
        {
            let path = format!("{prefix}{dir}/share/rustdesk/libsciter-gtk.so");
            if std::path::Path::new(&path).exists() {
                so_path = path;
                break;
            }
        }
        sciter::set_library(&so_path).ok();
    }
    #[cfg(windows)]
    // Check if there is a sciter.dll nearby.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sciter_dll_path = parent.join("sciter.dll");
            if sciter_dll_path.exists() {
                // Try to set the sciter dll.
                let p = sciter_dll_path.to_string_lossy().to_string();
                log::debug!("Found dll:{}, \n {:?}", p, sciter::set_library(&p));
            }
        }
    }
    // https://github.com/c-smile/sciter-sdk/blob/master/include/sciter-x-types.h
    // https://github.com/rustdesk/rustdesk/issues/132#issuecomment-886069737
    #[cfg(windows)]
    allow_err!(sciter::set_options(sciter::RuntimeOptions::GfxLayer(
        sciter::GFX_LAYER::WARP
    )));
    use sciter::SCRIPT_RUNTIME_FEATURES::*;
    allow_err!(sciter::set_options(sciter::RuntimeOptions::ScriptFeatures(
        ALLOW_FILE_IO as u8 | ALLOW_SOCKET_IO as u8 | ALLOW_EVAL as u8 | ALLOW_SYSINFO as u8
    )));
    let mut frame = sciter::WindowBuilder::main_window().create();
    #[cfg(windows)]
    allow_err!(sciter::set_options(sciter::RuntimeOptions::UxTheming(true)));
    frame.set_title(&crate::get_app_name());
    #[cfg(target_os = "macos")]
    crate::platform::delegate::make_menubar(frame.get_host(), args.is_empty());
    #[cfg(windows)]
    crate::platform::try_set_window_foreground(frame.get_hwnd() as _);
    let page;
    if args.len() > 1 && args[0] == "--play" {
        args[0] = "--connect".to_owned();
        let path: std::path::PathBuf = (&args[1]).into();
        let id = path
            .file_stem()
            .map(|p| p.to_str().unwrap_or(""))
            .unwrap_or("")
            .to_owned();
        args[1] = id;
    }
    if args.is_empty() {
        std::thread::spawn(move || check_zombie());
        crate::common::check_software_update();
        frame.event_handler(UI {});
        frame.sciter_handler(UIHostHandler {});
        page = "index.html";
        // Start pulse audio local server.
        #[cfg(target_os = "linux")]
        std::thread::spawn(crate::ipc::start_pa);
    } else if args[0] == "--install" {
        frame.event_handler(UI {});
        frame.sciter_handler(UIHostHandler {});
        page = "install.html";
    } else if args[0] == "--cm" {
        frame.register_behavior("connection-manager", move || {
            Box::new(cm::SciterConnectionManager::new())
        });
        page = "cm.html";
        *cm::HIDE_CM.lock().unwrap() = crate::ipc::get_config("hide_cm")
            .ok()
            .flatten()
            .unwrap_or_default()
            == "true";
    } else if (args[0] == "--connect"
        || args[0] == "--file-transfer"
        || args[0] == "--port-forward"
        || args[0] == "--rdp")
        && args.len() > 1
    {
        #[cfg(windows)]
        {
            let hw = frame.get_host().get_hwnd();
            crate::platform::windows::enable_lowlevel_keyboard(hw as _);
        }
        let mut iter = args.iter();
        let Some(cmd) = iter.next() else {
            log::error!("Failed to get cmd arg");
            return;
        };
        let cmd = cmd.to_owned();
        let Some(id) = iter.next() else {
            log::error!("Failed to get id arg");
            return;
        };
        let id = id.to_owned();
        let pass = iter.next().unwrap_or(&"".to_owned()).clone();
        let args: Vec<String> = iter.map(|x| x.clone()).collect();
        frame.set_title(&id);
        frame.register_behavior("native-remote", move || {
            let handler =
                remote::SciterSession::new(cmd.clone(), id.clone(), pass.clone(), args.clone());
            #[cfg(not(any(feature = "flutter", feature = "cli")))]
            {
                *CUR_SESSION.lock().unwrap() = Some(handler.inner());
            }
            Box::new(handler)
        });
        page = "remote.html";
    } else {
        log::error!("Wrong command: {:?}", args);
        return;
    }
    #[cfg(feature = "inline")]
    {
        let html = if page == "index.html" {
            inline::get_index()
        } else if page == "cm.html" {
            inline::get_cm()
        } else if page == "install.html" {
            inline::get_install()
        } else {
            inline::get_remote()
        };
        frame.load_html(html.as_bytes(), Some(page));
    }
    #[cfg(not(feature = "inline"))]
    frame.load_file(&format!(
        "file://{}/src/ui/{}",
        std::env::current_dir()
            .map(|c| c.display().to_string())
            .unwrap_or("".to_owned()),
        page
    ));
    let hide_cm = *cm::HIDE_CM.lock().unwrap();
    if !args.is_empty() && args[0] == "--cm" && hide_cm {
        // run_app calls expand(show) + run_loop, we use collapse(hide) + run_loop instead to create a hidden window
        frame.collapse(true);
        frame.run_loop();
        return;
    }
    frame.run_app();
}

struct UI {}

impl UI {
    fn recent_sessions_updated(&self) -> bool {
        recent_sessions_updated()
    }

    fn get_id(&self) -> String {
        ipc::get_id()
    }

    fn temporary_password(&mut self) -> String {
        temporary_password()
    }

    fn update_temporary_password(&self) {
        update_temporary_password()
    }

    fn permanent_password(&self) -> String {
        permanent_password()
    }

    fn set_permanent_password(&self, password: String) {
        set_permanent_password(password);
    }

    fn get_remote_id(&mut self) -> String {
        LocalConfig::get_remote_id()
    }

    fn set_remote_id(&mut self, id: String) {
        LocalConfig::set_remote_id(&id);
    }

    fn goto_install(&mut self) {
        goto_install();
    }

    fn install_me(&mut self, _options: String, _path: String) {
        install_me(_options, _path, false, false);
    }

    fn update_me(&self, _path: String) {
        update_me(_path);
    }

    fn run_without_install(&self) {
        run_without_install();
    }

    fn show_run_without_install(&self) -> bool {
        show_run_without_install()
    }

    fn get_license(&self) -> String {
        get_license()
    }

    fn get_option(&self, key: String) -> String {
        get_option(key)
    }

    fn get_local_option(&self, key: String) -> String {
        get_local_option(key)
    }

    fn set_local_option(&self, key: String, value: String) {
        set_local_option(key, value);
    }

    fn peer_has_password(&self, id: String) -> bool {
        peer_has_password(id)
    }

    fn forget_password(&self, id: String) {
        forget_password(id)
    }

    fn get_peer_option(&self, id: String, name: String) -> String {
        get_peer_option(id, name)
    }

    fn set_peer_option(&self, id: String, name: String, value: String) {
        set_peer_option(id, name, value)
    }

    fn using_public_server(&self) -> bool {
        crate::using_public_server()
    }

    fn get_options(&self) -> Value {
        let hashmap: HashMap<String, String> =
            serde_json::from_str(&get_options()).unwrap_or_default();
        let mut m = Value::map();
        for (k, v) in hashmap {
            m.set_item(k, v);
        }
        m
    }

    fn test_if_valid_server(&self, host: String, test_with_proxy: bool) -> String {
        test_if_valid_server(host, test_with_proxy)
    }

    fn get_sound_inputs(&self) -> Value {
        Value::from_iter(get_sound_inputs())
    }

    fn set_options(&self, v: Value) {
        let mut m = HashMap::new();
        for (k, v) in v.items() {
            if let Some(k) = k.as_string() {
                if let Some(v) = v.as_string() {
                    if !v.is_empty() {
                        m.insert(k, v);
                    }
                }
            }
        }
        set_options(m);
    }

    fn set_option(&self, key: String, value: String) {
        set_option(key, value);
    }

    fn install_path(&mut self) -> String {
        install_path()
    }

    fn install_options(&self) -> String {
        install_options()
    }

    fn get_socks(&self) -> Value {
        Value::from_iter(get_socks())
    }

    fn set_socks(&self, proxy: String, username: String, password: String) {
        set_socks(proxy, username, password)
    }

    fn is_installed(&self) -> bool {
        is_installed()
    }

    fn is_root(&self) -> bool {
        is_root()
    }

    fn is_release(&self) -> bool {
        #[cfg(not(debug_assertions))]
        return true;
        #[cfg(debug_assertions)]
        return false;
    }

    fn is_share_rdp(&self) -> bool {
        is_share_rdp()
    }

    fn set_share_rdp(&self, _enable: bool) {
        set_share_rdp(_enable);
    }

    fn is_installed_lower_version(&self) -> bool {
        is_installed_lower_version()
    }

    fn closing(&mut self, x: i32, y: i32, w: i32, h: i32) {
        crate::server::input_service::fix_key_down_timeout_at_exit();
        LocalConfig::set_size(x, y, w, h);
    }

    fn get_size(&mut self) -> Value {
        let s = LocalConfig::get_size();
        let mut v = Vec::new();
        v.push(s.0);
        v.push(s.1);
        v.push(s.2);
        v.push(s.3);
        Value::from_iter(v)
    }

    fn get_mouse_time(&self) -> f64 {
        get_mouse_time()
    }

    fn check_mouse_time(&self) {
        check_mouse_time()
    }

    fn get_connect_status(&mut self) -> Value {
        let mut v = Value::array(0);
        let x = get_connect_status();
        v.push(x.status_num);
        v.push(x.key_confirmed);
        v.push(x.id);
        v
    }

    #[inline]
    fn get_peer_value(id: String, p: PeerConfig) -> Value {
        let values = vec![
            id,
            p.info.username.clone(),
            p.info.hostname.clone(),
            p.info.platform.clone(),
            p.options.get("alias").unwrap_or(&"".to_owned()).to_owned(),
        ];
        Value::from_iter(values)
    }

    fn get_peer(&self, id: String) -> Value {
        let c = get_peer(id.clone());
        Self::get_peer_value(id, c)
    }

    fn get_fav(&self) -> Value {
        Value::from_iter(get_fav())
    }

    fn store_fav(&self, fav: Value) {
        let mut tmp = vec![];
        fav.values().for_each(|v| {
            if let Some(v) = v.as_string() {
                if !v.is_empty() {
                    tmp.push(v);
                }
            }
        });
        store_fav(tmp);
    }

    fn get_recent_sessions(&mut self) -> Value {
        // to-do: limit number of recent sessions, and remove old peer file
        let peers: Vec<Value> = PeerConfig::peers(None)
            .drain(..)
            .map(|p| Self::get_peer_value(p.0, p.2))
            .collect();
        Value::from_iter(peers)
    }

    fn get_icon(&mut self) -> String {
        get_icon()
    }

    fn remove_peer(&mut self, id: String) {
        PeerConfig::remove(&id);
    }

    fn remove_discovered(&mut self, id: String) {
        remove_discovered(id);
    }

    fn send_wol(&mut self, id: String) {
        crate::lan::send_wol(id)
    }

    fn new_remote(&mut self, id: String, remote_type: String, force_relay: bool) {
        new_remote(id, remote_type, force_relay)
    }

    fn is_process_trusted(&mut self, _prompt: bool) -> bool {
        is_process_trusted(_prompt)
    }

    fn is_can_screen_recording(&mut self, _prompt: bool) -> bool {
        is_can_screen_recording(_prompt)
    }

    fn is_installed_daemon(&mut self, _prompt: bool) -> bool {
        is_installed_daemon(_prompt)
    }

    fn get_error(&mut self) -> String {
        get_error()
    }

    fn is_login_wayland(&mut self) -> bool {
        is_login_wayland()
    }

    fn current_is_wayland(&mut self) -> bool {
        current_is_wayland()
    }

    fn get_software_update_url(&self) -> String {
        crate::SOFTWARE_UPDATE_URL.lock().unwrap().clone()
    }

    fn get_new_version(&self) -> String {
        get_new_version()
    }

    fn get_version(&self) -> String {
        get_version()
    }

    fn get_fingerprint(&self) -> String {
        get_fingerprint()
    }

    fn get_app_name(&self) -> String {
        get_app_name()
    }

    fn get_software_ext(&self) -> String {
        #[cfg(windows)]
        let p = "exe";
        #[cfg(target_os = "macos")]
        let p = "dmg";
        #[cfg(target_os = "linux")]
        let p = "deb";
        p.to_owned()
    }

    fn get_software_store_path(&self) -> String {
        let mut p = std::env::temp_dir();
        let name = crate::SOFTWARE_UPDATE_URL
            .lock()
            .unwrap()
            .split("/")
            .last()
            .map(|x| x.to_owned())
            .unwrap_or(crate::get_app_name());
        p.push(name);
        format!("{}.{}", p.to_string_lossy(), self.get_software_ext())
    }

    fn create_shortcut(&self, _id: String) {
        #[cfg(windows)]
        create_shortcut(_id)
    }

    fn discover(&self) {
        std::thread::spawn(move || {
            allow_err!(crate::lan::discover());
        });
    }

    fn get_lan_peers(&self) -> String {
        // let peers = get_lan_peers()
        //     .into_iter()
        //     .map(|mut peer| {
        //         (
        //             peer.remove("id").unwrap_or_default(),
        //             peer.remove("username").unwrap_or_default(),
        //             peer.remove("hostname").unwrap_or_default(),
        //             peer.remove("platform").unwrap_or_default(),
        //         )
        //     })
        //     .collect::<Vec<(String, String, String, String)>>();
        serde_json::to_string(&get_lan_peers()).unwrap_or_default()
    }

    fn get_uuid(&self) -> String {
        get_uuid()
    }

    fn open_url(&self, url: String) {
        #[cfg(windows)]
        let p = "explorer";
        #[cfg(target_os = "macos")]
        let p = "open";
        #[cfg(target_os = "linux")]
        let p = if std::path::Path::new("/usr/bin/firefox").exists() {
            "firefox"
        } else {
            "xdg-open"
        };
        allow_err!(std::process::Command::new(p).arg(url).spawn());
    }

    fn change_id(&self, id: String) {
        reset_async_job_status();
        let old_id = self.get_id();
        change_id_shared(id, old_id);
    }

    fn http_request(&self, url: String, method: String, body: Option<String>, header: String) {
        http_request(url, method, body, header)
    }

    fn post_request(&self, url: String, body: String, header: String) {
        post_request(url, body, header)
    }

    fn is_ok_change_id(&self) -> bool {
        hbb_common::machine_uid::get().is_ok()
    }

    fn get_async_job_status(&self) -> String {
        get_async_job_status()
    }

    fn get_http_status(&self, url: String) -> Option<String> {
        get_async_http_status(url)
    }

    fn t(&self, name: String) -> String {
        crate::client::translate(name)
    }

    fn is_xfce(&self) -> bool {
        crate::platform::is_xfce()
    }

    fn get_api_server(&self) -> String {
        get_api_server()
    }

    fn has_hwcodec(&self) -> bool {
        has_hwcodec()
    }

    fn has_vram(&self) -> bool {
        has_vram()
    }

    fn get_langs(&self) -> String {
        get_langs()
    }

    fn video_save_directory(&self, root: bool) -> String {
        video_save_directory(root)
    }

    fn handle_relay_id(&self, id: String) -> String {
        handle_relay_id(&id).to_owned()
    }

    fn get_login_device_info(&self) -> String {
        get_login_device_info_json()
    }

    fn support_remove_wallpaper(&self) -> bool {
        support_remove_wallpaper()
    }

    fn has_valid_2fa(&self) -> bool {
        has_valid_2fa()
    }

    fn generate2fa(&self) -> String {
        generate2fa()
    }

    pub fn verify2fa(&self, code: String) -> bool {
        verify2fa(code)
    }

    fn verify_login(&self, raw: String, id: String) -> bool {
        crate::verify_login(&raw, &id)
    }

    fn generate_2fa_img_src(&self, data: String) -> String {
        let v = qrcode_generator::to_png_to_vec(data, qrcode_generator::QrCodeEcc::Low, 128)
            .unwrap_or_default();
        let s = hbb_common::sodiumoxide::base64::encode(
            v,
            hbb_common::sodiumoxide::base64::Variant::Original,
        );
        format!("data:image/png;base64,{s}")
    }

    pub fn check_hwcodec(&self) {
        check_hwcodec()
    }
}

impl sciter::EventHandler for UI {
    sciter::dispatch_script_call! {
        fn t(String);
        fn get_api_server();
        fn is_xfce();
        fn using_public_server();
        fn get_id();
        fn temporary_password();
        fn update_temporary_password();
        fn permanent_password();
        fn set_permanent_password(String);
        fn get_remote_id();
        fn set_remote_id(String);
        fn closing(i32, i32, i32, i32);
        fn get_size();
        fn new_remote(String, String, bool);
        fn send_wol(String);
        fn remove_peer(String);
        fn remove_discovered(String);
        fn get_connect_status();
        fn get_mouse_time();
        fn check_mouse_time();
        fn get_recent_sessions();
        fn get_peer(String);
        fn get_fav();
        fn store_fav(Value);
        fn recent_sessions_updated();
        fn get_icon();
        fn install_me(String, String);
        fn is_installed();
        fn is_root();
        fn is_release();
        fn set_socks(String, String, String);
        fn get_socks();
        fn is_share_rdp();
        fn set_share_rdp(bool);
        fn is_installed_lower_version();
        fn install_path();
        fn install_options();
        fn goto_install();
        fn is_process_trusted(bool);
        fn is_can_screen_recording(bool);
        fn is_installed_daemon(bool);
        fn get_error();
        fn is_login_wayland();
        fn current_is_wayland();
        fn get_options();
        fn get_option(String);
        fn get_local_option(String);
        fn set_local_option(String, String);
        fn get_peer_option(String, String);
        fn peer_has_password(String);
        fn forget_password(String);
        fn set_peer_option(String, String, String);
        fn get_license();
        fn test_if_valid_server(String, bool);
        fn get_sound_inputs();
        fn set_options(Value);
        fn set_option(String, String);
        fn get_software_update_url();
        fn get_new_version();
        fn get_version();
        fn get_fingerprint();
        fn update_me(String);
        fn show_run_without_install();
        fn run_without_install();
        fn get_app_name();
        fn get_software_store_path();
        fn get_software_ext();
        fn open_url(String);
        fn change_id(String);
        fn get_async_job_status();
        fn post_request(String, String, String);
        fn is_ok_change_id();
        fn create_shortcut(String);
        fn discover();
        fn get_lan_peers();
        fn get_uuid();
        fn has_hwcodec();
        fn has_vram();
        fn get_langs();
        fn video_save_directory(bool);
        fn handle_relay_id(String);
        fn get_login_device_info();
        fn support_remove_wallpaper();
        fn has_valid_2fa();
        fn generate2fa();
        fn generate_2fa_img_src(String);
        fn verify2fa(String);
        fn check_hwcodec();
        fn verify_login(String, String);
    }
}

impl sciter::host::HostHandler for UIHostHandler {
    fn on_graphics_critical_failure(&mut self) {
        log::error!("Critical rendering error: e.g. DirectX gfx driver error. Most probably bad gfx drivers.");
    }
}

#[cfg(not(target_os = "linux"))]
fn get_sound_inputs() -> Vec<String> {
    let mut out = Vec::new();
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    if let Ok(devices) = host.devices() {
        for device in devices {
            if device.default_input_config().is_err() {
                continue;
            }
            if let Ok(name) = device.name() {
                out.push(name);
            }
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn get_sound_inputs() -> Vec<String> {
    crate::platform::linux::get_pa_sources()
        .drain(..)
        .map(|x| x.1)
        .collect()
}

// sacrifice some memory
pub fn value_crash_workaround(values: &[Value]) -> Arc<Vec<Value>> {
    let persist = Arc::new(values.to_vec());
    STUPID_VALUES.lock().unwrap().push(persist.clone());
    persist
}

pub fn get_icon() -> String {
    // 128x128
    #[cfg(target_os = "macos")]
    // 128x128 on 160x160 canvas, then shrink to 128, mac looks better with padding
    {
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAIAAAACACAYAAADDPmHLAAABhGlDQ1BJQ0MgUHJvZmlsZQAAeJx9kT1Iw0AYht+mSkUqHewg4pChOlkQFXHUVihChVArtOpgcukfNGlIUlwcBdeCgz+LVQcXZ10dXAVB8AfE1cVJ0UVK/C4ptIjxjuMe3vvel7vvAKFZZZrVMwFoum1mUgkxl18VQ68QEEKYZkRmljEvSWn4jq97BPh+F+dZ/nV/jgG1YDEgIBLPMcO0iTeIZzZtg/M+cZSVZZX4nHjcpAsSP3Jd8fiNc8llgWdGzWwmSRwlFktdrHQxK5sa8TRxTNV0yhdyHquctzhr1Tpr35O/MFzQV5a5TmsEKSxiCRJEKKijgipsxGnXSbGQofOEj3/Y9UvkUshVASPHAmrQILt+8D/43VurODXpJYUTQO+L43yMAqFdoNVwnO9jx2mdAMFn4Erv+GtNYPaT9EZHix0BkW3g4rqjKXvA5Q4w9GTIpuxKQVpCsQi8n9E35YHBW6B/zetb+xynD0CWepW+AQ4OgbESZa/7vLuvu2//1rT79wPpl3Jw73bJbgAAEjFJREFUeJztnXt0HNV9x793dnZnd7VrSZZkYUfCMgvW2jWYIBU7x8etamiUUNuxUcsfbU6NA6VuOGBCcIzbnB4fTltxjlqCoQlN0iakJ03NIUEQTMPLj9MqB5m4ASzhbJEFklfGL71W+96Zubd/aFdeSavH3NnZ3XHm84935bkzd+/3e3/3MXfuABYWFhYWFhYWFhYWFhYWFhYWFhYWFhYW1zKk0Bdsampa1dLSsmXjxo23+/3+NQ0NDSs9Hk8VABcAodD5KRIUQDwSiYwMDQ0Fe3t7P+zu7n63q6vr+MmTJz8udubyjs/nq+7o6HgsGAx+wBijzGIuaDAYPN3R0fENn89XU2zddOPz+Wo7OzufkWU5VuySNSGxzs7OZ30+X22xddSMJElCe3v7XlmWQ8UuRbMjy3Kovb19ryRJ5mgeGxsbl/f09BwvdsFda/T09BxvbGxcXmx956W1tbUpHA6fL3ZhXauEw+Hzra2tTcXWOSdtbW2brZBvPLIsh9ra2jYXW+9ptLa2NlniFw5ZlkMlEwkaGxuXW2G/8ITD4fP56BPomgiSJEk4derU0XXr1rXozUghUBnFZapgjCqYYBQpRgEAdiLAQwgqBRG1gh12Yo4Od29v74nm5uY7kskk5T2HqCcDBw8efKgUxWeM4ayaxDtyHO/JcfQoCZxVkzivJKBMHgEwOvkvWDrR5HcChlpBxGrRibWiC7fay7DB4cXN9jLYSswY69atazl48OBDBw4cOMR7Du4I4PP5agOBwEeiKC7hPUc+CVMVr6eieDUZxlupGC5SBVeFBgCa9ZlhPhNMHcPUqc+VRMAfOCuw1VmFbc4qVNscBfldC6EoyoTf71/d399/iSc9twE6Ozuf2bFjx0O86fMBZRRvyUk8nwjjlcQE4lNCqpguZH5MkDneBobPO5fi3rIV2OmqLnqT8fLLLz+7c+fOh3nSchnA5/NVBwKBc6IounjS6yXKKH6QiOLp2Bg+phSTwmZELIwJMp+XC3b8lacOD3rqsLRIUUFRlLjf77++v79/WGtaLuvu2bPnK8UQP8EonoqHsWr0Ah6OjqfFBwABIASTfk57mtiufgYBpmqpkPWZZP1fdlphRlrbnMdfoDL+duITrLzwS/zN+EcYp3L+f/gCiKLo2rNnz26etFwRIBgMflBXV3cLT1oeGGP4aSqOfbEIBlUZuWsnUMxIkDl+qWDD35XfiL/01EEoYNMwNDR0ur6+fr3WdJpzuGHDhlV1dXU3a03Hy6Cq4IvhcdwTCWOQ0hw125Z1dPEiQeb4Uariq2MB7BrpBWMZ8xhPXV3dzRs2bFilNZ1mA7S0tGxBARaSMDA8n0jilokQ3lBUzC9qaZkAIPhx7CLeTGhukvVA0tpoQrMBmpubb9eaRitRyvDn4Rh2xyKYyFSiWQVd+iZ4IzHCWwRcNDc3/67WNJongvx+/1qtabQwqFJsn4jgtKpOFiShmCxYNvk9u90mtqw2nmR9ByaFpVeb8JzHZ84npC+RuRZyXEvIavOzrzX38eI0UxoPjzaaI0BDQ8P1WtMslvdSCjaMhXFaSYvIALDs2on8R4Kpz/mPBH/krNJaBLpoaGhYqTWNZgOkF3Dmna6UgpZQFJcoAxiZ1vk21ATTRM2fCR701OP3nUt5ioIbHm00d+YYYyryvHr3aErBtrEo4oxNr5SEzais2cM25BiGzRzyaRgiThvy6Rsi3l92Hb5bcWNBh4FpKCHa2h0eA+R1bNOdUnDHaBSxTG5MboJJ8X3FEB8AQAjRpGlRDXBWodg4EsVIZkYvI7xJTXC/u7qo4gPaDVC0nE5Qhm0jMYyo6TY/A8Nkm51VxmboEzzgrim6+DwUJbeMMeweTSAg07lFNpEJHiirxXMVN5hOfEDnghBenovKeCmupEVOq0qQjgTpcD9lgnn+PzPWX2CewA6GZtGBJtGBNTYbVgo21Ag2uAkBARBjFCNUxSBV8H9KEu/JcZyU44hPhfe55wkecNfgufLrTSk+UIQ+QJ9Msf5ibEaPf4E2n6NP4ALFToeEP5FcuFOU4BG0/dQkY/ifVAQ/S4bxQnwMY4xhZp9gj7sK317Cd9OHMYYjiWEcS4xCBLDdVYPNeRg2lnQnkDGGLZcSOJHK6nTl2QTLCMOjTjf+QpKwVMjPTFycUbwQH8e3YyM4oyZxk82BR9xV2OVaCo3lDQBIMoo/HfkQL8UvT/4hHWm+5r0eT1X6deW1pA3w72EZu0aSk18EIJ8mkAiw3y1hn+TSXNsLSZJRtI18iNfiw8g1mnipej12uvkfByxZA0Qow+rzcVxQsxaw5skEG+02/MhThtViYefetZJgFHePnMEvEqOYa0h5g+jEmeWbIHH2KUp2GHgoJOOCMmPIR4EpJafKYYHe/4zRwdddTvx3ucdE4o9hviHlx0oC/xIZKli+ChIBJijDynNxjFM2vWZn4IgEAgG+53XiPpekNTsFJ8Eodo4E8HoyhOkTU7knl64TRHy8fBNcgvZBWklGgO+FFIxn+n3ZNTuDxkggAPhPr8sU4scZxY7RPryenEj/JXtOInckuEgV/DD6aUHyZ7gBVMrw7Lgyt6gZNJjg+14X7nGVxrr8+Ygziu2jfXgjGYLWW8lPR4ZAGZ11znxjuAFei1Ocm1b79ZlgX5mEr7hLX/wEo9g2ehZvJ8PIPcM4vwn6lDjeTo4bnk/DDfDDkJKj88Zngk12G/7BW/phHwCeiFzE0VQ0Z0dvsSZ4PnrB8Hwa2gkcVxlq+xNIZa40axiXo6OXYUbH0CkAPbVluFEs/SlXxhiqL/dilGaFPo4l525CcGnFJngE+6KvXVKdwNciFCk2ozZzRoK/9jhMIT4AJMAwOlVP5h7yLRQJYozh9cSYoXk12ABqWmB9JrjOJuDrntJv9zO4iIA1ohP5WF72qsEriw0zAGUUb8Uy4Q66TPCY1w63rXSnd3PxsLsK+Vhj+FZizNAHTAwzQCAJDM94iovHBG4BuL9s8W1gqfBlZzm8RIBeE1ygCs4qMcPyaZgBuuOZ2k90maDNbUe5yWo/AHgEEXc7M1sn6DPBO6mwYfk0zAC/TrC55/M1mOAeV2nP8c/H3dISXBWV3wTvyVHD8miYAc5kDKDDBE4G3GFiA2xxuCGSLIE5TdCrxA3Lo2EG+CjJkLNd12CC2yUbXCV8b38hPIKI9VOjAX4T9CkJw/JoiAEUxnBhqgPIb4Imk2yPOx+ftaf30dBhgqCagmrQfQFDSviyAtCMoDpMsMZhfgM0ihJy1+zFm4CC4ApVDMmfISU8qgJTP0KHCVaK5g3/GeoFEfl47mDUoK1nDDFARGVXRQe4TVBj4vY/Q3VmUYdOE4TN1ASkskXVYQK3+VsAOAFcFZLfBCkzGWBKZECXCcxf/wE23xNLGkxg1GSwIQZwEMwWlcMEMeMXxBhOhFHk4zE0h0G7jRhiAG/m9+g0wbBq3E2QQnGFprdT0GkCD8cDKIvBEANUipi7ZmswQVA2vwEG1Ow9i/hNUKlhUYgWDDHAMnv6p+g0QSBlfgOcUbOHb3wmICCoNZMBREJQ71igjV+ECf43bn4DvKvI0Pto+gqbI31PIf8YNtDyOYAFn/FfwATdcYYUNa8JgqqS3s+Y6DLBTaLTsDwaZoCbnWRySZcOE0Qp8MuYeQ3wmpzCVSH5TfA7ZjTAre70D9Fpgp+Gs/f3MRcvJOPIx04lt4rGLYU3zACfc2d90WGCwxMUCRM2A32qghNKugOo0wSfc5QZlk/DDNDoIqjNvpnDaYJRGXghZL4ZoacTcUwWbw5RNZhgKRGwxsAXURhmAEKAO5fg6tw+wG2C74yZywBDKsUPkknMvqmj3QR3Sl5D9x8y9HbLtvKM2PpM0JMwVxPwzXgciVmdOT4TbJU8hubVUAN8sRxwEug2wQ12Y8bARnAiJeNHyVT6mz4T2AFsNbD9T1/NOJaIBNsrs2sznwkeqTLHfeFxxnBvNL2GPw/7GN4leVGZp42u5sLwkr23GrPu8k2xCBPsqbLhvsrSNwBlFPdOxDCoZG9+qc8Eu5xeg3NdAAN8vpzgBolwmeCrVQK+s0Lg2oqt0OyPJPFKUsbM2UxeE6wQbNjmyB5LG4PhBrARYG8tZof3BUzwYLWAf/6MOcR/IpLAP8aS89zX0G6Ch11LDJv/z0bzFXg2iYqqDKveB67IwOwNHrNOJwBgDA8tIzhUV/riU0axL5zEU7EUpt3+nNKWTdN5sbuclxOCgcrrUMHR/pfU/gAZymwE+zMvOl8gEuxdJuBQna3kxR+jDF8aS+CpaLrHv6g7nDOKe45I8KjLyyU+DwWJAAAQVxnWfAAMTo2QZkeCR2oJnqoTUOLa4+2EgvvG4ziXmaKe+VsWFQmyijErEtQSgr7KZfByFkJJRgAAcNkIOq7HnL39ry3TJz5lFBMqg4GP0mNQpviz4Tj+cDiOcwrjurk1+ffs272YFgn+3u3hFp+Hgo6v/riKYP+KrD+kw/83VxD8Uz2f+AnKcOCCgqqAgvLfyPD1pfCtEQVjeVxPeCZF8cBIAjddiuEnsXm2vNNpgk2iHbulwr6SuWBNQDYfRBneDE1e/AsVwDo3n+NjKsP2QRVHIzNeOQMGpw3Y7hVwt0fAnV4BVRr2GGAMOCtT/FdcxeGYgu5k+pa0MHnumdeaHt75mgMnGH5dXok1Nn2vcCjZzaLzTUxl2Dag4liELVzQBFgrAZ91EvglAXU2gioRcKUrZYQBV1SGAYXiNzLDuymKTxU6XdQMBpngW243HnHqr/2/FQaIKQxbByiOR6jGgp5jCLrYtBnybIKtdjte8brzctfvmjfAlPhh3h54aZnAJwr4VbkXlUJ+umMlOwrIBzGFYesnFMdDenrgc8xDLDZtBp6t7mccX0EEvOr15E18HkxjgKjKsPUsxfEQoHe1cSmYwAnglSVlWFPk9xzwGKDgy3OiKsO2PobjkawcmNgEDhD8rLwMv+fI+0vbNGvDYwDjdizKgcIYvtTHcHwCswvehCZwAuiscOMuyZAnfTRro9kAkUjE2L1LZ/CTYeBoKP0lWyTAdCaoAMGblWVGic+ljWYDDAwMnNOaRg9vhzC3SIBpTHCTIOCd6jJslox7V+fAwMCg1jSaDRAIBM5oTaOHqZXlJjbBVknEyWVl8NuN7fDxaKPZAKdOnXpXaxo9bKvAwiIBJWkCJyF4ulzCz6tdqCzAfkfd3d2atdFsgBMnThzD1Z9tODuWEuyqgelMsMkh4L3r3Ni7xFGotQ2sq6vrmNZEXDkLBoMf1NXV3cKTlgfGgP8YZth/Dvg087j9IlYW6Zufn+P8C6SttRG0V9qxy2Mr6Aulh4aGTtfX16/Xmo4rh4cPH/4xTzpeCAG+XEPQtx5or8/sQFJakaBCIHiiwo6zn3Fht9de8LeJ82rCFQF8Pl91IBA4J4piYW9epwmrDN+/BDxzGRhMFjcS1NsJHioX8cASsWjb2iuKEvP7/Sv7+/uHtablsml/f//wkSNH/pUnbT7w2ggeXUHQfwvw89UEOysJJAEFiwQOADvKBLy8XMLHK53YV1ncdxocOXLk33jEBzgjAAD4fL7aQCDwkSiKSxY+2ngmFIZXQ8AvxhjejABXlPxGghoR2FJGsM1jw10eAZUl8hILRVEm/H7/6v7+/ks86XX9ivb29r2PP/7403rOYQSMAYEEQ3cUeD/G0JNk6E8CQyk2bZieywREAOrswI0OgrVOgtucBBtcBGskFLxdXwxPPvnkIwcOHDjEm16XASRJEk6dOnV03bp1LXrOUygUxnBZBsYoEFYnt7QlYLCTyWal0gYsE1GQBzLyQW9v74nm5uY7kslk8Z6fb2xsXB4Oh88zi4ISDofPNzY2Ll9YoQLQ2tp6myzLoWIXym8LsiyHWltbm4qt+zTa2to2WyYwHlmWQ21tbZuLrXdOWltbb7OaA+MIh8PnS67mz6SxsXF5T0/PsWIX1rVGT0/P8ZJp8xdCkiShvb39YatJ0I8sy6H29va9kmTCN2j5fL7aF1988ZAsy9FiF6TZkGU51tnZ+YzP56stto668fl81R0dHY8Fg8H3GWO02IVbwtBgMHi6o6PjGz6fr6YQ2hR8xqOpqamhpaVly8aNG2/3+/1rGxoaVno8nioALphombpOKIB4JBIZGRgYOBcIBD7s7u7+VVdX17GTJ09+UuzMWVhYWFhYWFhYWFhYWFhYWFhYWFhYWFhcO/w/ziodTPNubD8AAAAASUVORK5CYII=".into()
    }
    #[cfg(not(target_os = "macos"))] // 128x128 no padding
    {
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAIAAAACACAYAAADDPmHLAAAACXBIWXMAAEiuAABIrgHwmhA7AAATN0lEQVR4nO2de5QcVZ3HP7/bPT1JJjMJSWaSYfI0wgoj8ggPeScoQQi7nvUFuwKi67orenRdFLMoLxVQEB+4Z3fxHPGw4llI0BNNkCzrYQkiCUIieJiBFQiJJoRkJpOEJGQm031/+0f1o6q6uufVVd2Zru8/1XVv9e/eW79P/e6t1y1RVcabZNvz00jI8aDHI7oQa9sQbQVmALOAFtQCubbnlwdR7UVkJ2gvSi+a2Ywx3WQGunXOkter0Z4wJUc6ANLz6mQYPAebOR/V08F2AjOd3Gzb1OJZ96QXQQAakAagmb3Ai8CzYNeRnPyEzjq1p8JNilRHHADShaH1pTNRloFdDHIaaNLJzTnPuv5RcQjc6wp0gT6OTf83B+yjevzSw2NtY5Q6YgCQN17uxMiHQa8EfRvgcgYUOy8yCFzb272IWY0dXEnPxrW6aPngyFoZvWoaAHnjpVYSDf8A8neone+klnKGL696EOTSehH5Kdh/1dnvfWWotlZLNQmA7Hr1RJBrEHMF2En5jJLOq1kIskb1MZS7mbtnjfKRmtrhNQWA9Gw9DzE3gy4pOAk8TjwyIchlPA/2m8zZ+2CtgFATAEjfjuNRvRnNfLiQqoxTCEDtC6hdrvPe9zBVVlUBkD1752Pf+hrIR0GNd4fBOIcA1K7Fppfrgkufp0qqCgCCJujr+Txqvw5M8u7g4UIwHGf48moRArBo5nYOXHyjdnoqF4kiB0D29LwT1R8Bp3sdW9cQgGZu1XkXf5WIFRkA0kcD9H4F+BcglXdCDEGu3AyZw5264NL/I0KZKAqR3p6jYc9jwE1AylO0uKsghaUY17o7LSfjSnf9z72URAnbgrfpue1N8bb59IAyhiw3qP7+/+TLTZBMXUHECh0A6dt7Pia5ETgHEr7cGIJClgHlXUSs0AAQVojs3vMlVH+NMquQE0NQBgJ3nxWJQgFAukiya+l9qNyB4tyo8Qw1YgiC7ejviVgVB0B20Ejb3gcRrnScLoHjnhiCIggGyRz+CRGrogDItp7JJPauQflAPjGGwFduCQjUXqfzl20hYlXsNFB63mrHDv4KOAnJ2vS1FTSw6wN/11crp4gB5+vhnCLeqbMXX0cVVBEAZNf+GdjMOkSOd1nOLt0bQgwBXghEvqMd511LlTTmLkB6e5qx9hFwOR9As971+ejI6w58p2qBdkbZHYjcVU3nwxgBEDTFYONDIKcCPocSQ1C+3Lu047wvUmWNGgDpwvD6/hXA0rxTIYZgOBCo3qkdZ1fd+TCWCDDlzRuB93udGEMwDAju0tnnVGXAF6RRDQJl+74LwTwCmghqa96b4v9jnQ8MRe7S9tNr4sjPacQAyLa98yCxEWG6kxLgvBgCV3quveYubT+1ppwPIwRADjCBvW+uBznJScjl1DwEb2IzLyGyFZGtqPYCBxA7iM3aFnMUaqeDHg3MRTOdwLRi26OBwN6hR5/xZWpQyRFtvefgLR7na3aJODvGnaZk+z11bZeVigOBOz23vd8O4IwJ3BAYwDr9bX7Hu/4kZgtq14KuQ+1GWp9+ZTQPYcruLR2k+09GdTEi7wFORBA0W36+TFx18e0Ek7hDZ51Wk86HEUQA+dNbpyH2KSBZdGTXQiQQsxmbvh8xD+j0WS8Oq1EjlPS+3E5m8EPA5ag9E2zx6NYTCfR2bV90fRh1AZCt/3MyRj4K2om1/RjzDJK4T2dfsH3YNoYDgHSRouXAsygneJwN1YZAUbsG1e8wY926KB+1lt4/Hku6/7PA1UCzD4LdqH5Gjz7lwVDK7n40RbP5HsinA+A7BPYLOvd99wzL1rAA2HLgFgw3FhLyJXrXo4NAgQfR9NfCOtqHK+nramGQD6HpU1DdD/oCA6//TOcv6w+lvO5HUzQnHgL5y8Bz6MK46GM6d+l/DmlvKABk8/5jSZgXUG0IullWBQh+g6av1emtz5St+DiUvLK2kQkTHgK5tHiwWwTBmwymj9GFl+wqZ3PoC0EmcRtKAyKBz1HmvVNUj4CLPGO7WPQWluXsOGpxXTq/+9EUE5tXOM4Hii6AFV2kaiHVcCNDqGwEkM2HzkDseiRrXXBG6dFHgt8AV+qMqVvLN2d8SrY8PIGGqT9HMxc7KUHXOQIjwSCGTp299OVStstHAGNvA8RjO/JIoD/ETH1P3Tq/+9EUDdNWAheP4hnDBqzcVtZ+qQggr711CcrD+SPSbTuaSDCI8EltnTrkQGa8SrY8PIHUtFWoXuSk+K9UDisSKJmBs3T+sg1BZZSOABbnLRV1eSa6SDCA5fK6dv4raxtpbHsI5SJXanYxokggJCd+pWQ5QRFANh86A9UNHjvRRYJDWL1E26c+XqrS412y7fmJmP5fABcWLj8XjY5HEgksJnmcdiz+o7+s4Ahg7ReKj9ZIIsEgVj5c187fsXESZuCXIBdmU5x9XHx+PJJIYLCDnwsszx8B5LVDc8jwKmiDx3b4kUCBq7S95f6gitaDZMfGSahdDVwQeJSMLRIcRPvn6NxL97jLLI4AGT4DNJQ8gsOLBN+ta+f30QCyCjEXlH+oZNSRoAnT9Pf+cj0ASBcGxfWCYkQQqK6jr6Vm75hFooHf3w1cWHBqGBDox/3FeiNAsv9coMM7Ig8dgn2QuVI7SfsrVy+S7ZuOBT5V5PBKQ6D6Dtn+xEnusv1dwGVBF5RChUC5Vjum/Zl6VsJ8CjG5Bx/xLCsOQfoyd9F5AKSLJPDBwFG622BlIXiM2Y/cS71LEic4y9E8aDpCCJTLhBX5jEIESAwsAdp8N2DChMCSMdfWynRpVZXqQP53+BAsYNus03MpBQCUpa7fhA6B8hOd1/QcsQB9wuOosCEgc0luzbjSLyg+xQwNggwZ/Qaxsmq8D7Q/OgjM4vwvAOliKsqJQMB1hlAg+LkuaKnZ+XOjls56Rw8kH3DWIoHg3bJj4yTIRYDE4fOBxMjf4BklBGK/Qyyv1P6gcm8gDQlBCnvwTMiVmNHFRc4LCwKkW+e0BN6arGdp+wmbEOmKDgJdDIXSTnZqkatNqBDU7eXeoSUPVPZdxDIQqJyOq6TOks6rNASJxH8RK1gZu7LgvNAh6HRSug/MxDa8kfdYkR1fev530d28rMraeVEXNHknkojlkezs3gLMI3/3r1IzlRQdyTCwb6ohnejMH9kBNisaCYS1xCovSfxv9kf4kaDxqOMMualdooAgI+uIVV42va7YeSFBIHQakHm+UbrrN5WFwMgmYpVXYsJzzo8IIFBdaFDanJVcoutPlYVgly6YWN93/Yajnle7gezXxkKGQG2bIX8DiHAhsPoSsYaU891BebWQEiIEYloNIm3Oe/m5GriXFYRAZEtQg2MFSMz2aOYsklaDitMFhA2BsrW4pbGCJc43isOHoNWANucdFSYEIn3FDY1VQj0F54UKwTSDZq2EDoEeKGpmrGCJyc4tEDoExuv8MCGwcpBYw5O6nw0IFwKDZmcwDBsC45m5MVY5Gf/9+/AgMIApcl4YEGSYQKxhSiZHNaOpQbPP44cNgchEfzNjlZDayQWnQogQqAHZXdJ5lYTAapu/nbFKyWT3VegQ7DHZWTNLO69SEBg6iDU8qZ0b0QTXvQboLRScW4YAgTKHWMOTmLnOj9Ah6DWI9BYP2ggDgk5iDSnZ8ccZiHF9ZzFECJRdhozu8jjVycguKwrB0fL8/hnEKq9EapHjPPdZc2gQ9BqMeTk7Sg8fgsbUyUUNjuWVyGnZH+FDYBKvGTLWmWo1CIKcKgaB8yhyrDKSxAWulZAhSHQZEskuz3OHQU6FSkHwXmKVlOzc2gRylucoDRWCZLfRU9gN7IoGAlkkLx1o9Tc8VlbJ1IWIaXRWQoegR2e9oydnqRsgAggSZBo+SKxS+ihA+UkechozBN0uK1J4VStsCITLiVUk2fPnKSDLip0XEgTwTMGC2sc9BYULwbnSPbCQWF7Z1FVA9n5JBBCI84i+8+9Jyd8CgxFBYFA+S6y8BE0Any9yNIQFQYbGpifz/9RODoBuKiooPAg+IZtpJpajvt0fQBILRzfxU04jgEDkOZ06f6/7X2BlXcGxoUPQwqH+eDAIucm5bnbWDJFAYO3j/n+AMasBooMgcSqxYNaeTwKuF2ajgMA84t8aJvIUyjYgKgiCrjnWlWRf71GI3Bz4reTwINjJzL943L8l2okFVhY7NiQIhPg9wYx8F3SmsxIVBPIzRTL+rRypcb5zFzoE2kNLwwPUsaRnzyWo+Zg3DkYAgYjnW4ZeAO7id5B9hSssCIwoyD/rHA5Rp5KevtkY+XHgfgsXgtfZtfBJAnKdeqxAsfKjvMFQIJCb9YRU3c4TJGgKSazE0gYl9ltoECTuy3b1RTkFpbgHpT8cCPQb+s7k14rKrBMJK4Sd+/4D1XcDhesi0UCQJtH4b/46FQGgp9AD+tPgu3m+gmEEEOi39F2pG/zl1ZV2XvQthI979w1EBMFDOq19m79KJd7WMXd7Kueu2OgguEVPTC0PLqs+JDv3XQ98aeSv1lcIArU/CKpXIAB6Bn9AecxTOXfFRgbBTXpS8uagcupBwgqRHfvuBG7NJ0YPwbPa9rangupX5n09uaXE3TxfBctAYLlBT67jPn8Hjbzxvh8jfLHoslekEGhJH5T/dvAGXQ1c6r2SqF7b+fJddgygcqMuMl8vaXycS7b2zSaVXAm8O3C/5TcMSM9tX7SPATJ4ZSn/NTF5Umd0nFuqnuXf2LWyHMiMOBKoXF/Xzt++//00JDeheEf7+d8uhR0JbKbsx7jKAqBn0YXqfZ5KDAWByFd1kbm9nN3xKtmx7yjZvv8eYBWK8+xjuf2WU3gQrNK2uYF9f36rcl0AgDzDbNL6EkJTwS7B3YHhOl1k7ixrcBxKukgx5c1Pg9wATPd0i0X3YrLp7rS8oYp2B4OInqTT2rvL1n0oAABkPVeD3otkiw9qjOFLeqr59pDGRinpOtyJyicQPRZIoXY9yr16woQ/hVXm0HUiRfPByzF6A6pvdxJzmbmtqgSBpr+uMzpuHLINwwEAQNZzBej3EaZ5KiH0ofYaPSPxYOl/j17SRYrMwPcR+UdfuWRHOatAf0DnL5+I6gNU8qeDM8F+DORzQEfRQLjqENBN355T9O3HDPgzitoyXAAA5FmmkOavUNuJkETMcxxilS4mlAmg5GmamDS4BliMltmJzvpWYAXGrCDTsMl/zXvMdXl1/wyS5iKUv0VYimqywl9N9xU4aggs2HN0Wtv6YTRrZABEKXmaJiYOPgycXzidHRICR0Z2o/o48ATocwz0/0FPmrp3ROVv7l8Augg4DbFLgEWA8ZQ79g9mhwCB3K3Tpn9+iOa5iqhBALLO/xXKeUUNHS4E+bz8Tnwd2Ab6BkZ2YkmT0DdRBJGpoE0YmYG1c4H5SPYRbb8df7m1BIHyBwb6z9Sj299imKo5AKSLSRxOr0FYUtT4sUPgyyt1JJUot7Yh2I+mT9fprSOak7mmpm7zOB+Kz49zy3KPnAdcEIvsa2jFLQq2E1Tu2K4TKBm5eqTOhxoCQLqYxEB6DbDE29AjEILh2KksBN/U1qk/ZxSqiS5AupjMQOZXCOcGhncoDoNHQncwHDtj7Q6Uh2mb8n73g54jUdUjgDxNE4cyq4Fz80dS0BEZR4Li+otsQPddNlrnQ5UjgDxNE4mMc54P3iMnjgTlIwH6Aklzvk5vGdMs7FUDwHG+rgFd7ME8hiC4/l4br2EaztbWSTsYo6rSBUgXkzGZtYU5g1x7J6wXUsdPd7AFuLASzocqACAfQTio96NyDuBtfE4xBMH1hxcYGDxbZ05xfVNobIq8C5DfcSno6rJhMKe4O3CnPY2VZdrRvJsKKvouwNorAke5cSTw5XnquQY9eEGlnQ9VGQPIcUDwqU4MgS9PFNU76Gv565Fc3x+JkmEYHUKDzs6gsFNQvGm4VnB2mnFtnwvv+W1xdroE2Ana3m8Hd57PDkOUS4n65+x4bGftWHW1p6SdvQhXa8eUX4Q5z3oVugDdCJQb5caRAH0OtadqR/MvCFnRA6DmHnKTQ8QQ+O1YkO+jLWfpnMqN9MspcgD0bDahcntJZxSludJhPEPwAtizdc7kf4ry1fnq3Av4Hl9F5ToU55m1+oZgALiJ/ZMX6ZyWwoSdEam69wI2MA+1tyLyN/gft8rtpfF7nUARXUVGr9eFzVX7sHZt3A5+ik6MfgtYVh8Q8BRkluuC5t9QZdUEADnJBi4C/TK5J4LGGwTC7zB6k85vWkuNqKYAyEnWcyIJew0qVyFMOMIhGARdhegPdWHTr4PaW03VJAA5yW9pp8FeA3IF6HwnMZeZ26pmIdgOei9p/l2Pq8yduzBU0wC4Jc/SScZeieEqoN1JzOXWCARKL4ZHwK5kcNIj2pn9KmsN64gBICdZQYL56fPALMXoEpBFQLJKEKQR/T0q60jqag5PeLLSbySFrSMOAL/kGZqRzHmoLEH0ZKATmBkSBD0IXahuICFPkGp8Ut/G/sq3Kjod8QAESTYxHU2/E8xxYBdgpA10BkororMQaSls7IGgD0Mvqr0ouxF2IfIyRl6kIdmtx1Dx27HV1v8D00hh5A6BMbUAAAAASUVORK5CYII=".into()
    }
}
