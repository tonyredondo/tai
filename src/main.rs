#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code)]

mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

mod ai;
mod config;
mod minimap;
mod overlay;
mod router;
mod selection;
mod session;
mod split;
mod status_bar;
mod tab;
mod tab_bar;
mod terminal;
mod workspace;

use config::TaiConfig;
use router::InputMode;
use selection::TextSelection;
use split::{SplitDirection, SplitNode, PanelRect, alloc_panel_id, create_panel, panel_term_size};
use status_bar::StatusBar;
use tab::TabSession;
use tab_bar::TabBarAction;
use workspace::{Workspace, WorkspaceManager, SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH, ROW_HEIGHT, SIDEBAR_BUTTON_HEIGHT};
use raylib::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static SIGNAL_EXIT: AtomicBool = AtomicBool::new(false);

const FONT_DATA: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

struct FontMetrics {
    font: raylib::ffi::Font,
    cell_width: i32,
    cell_height: i32,
    font_size_px: i32,
}

fn load_font(font_size: i32, dpi_scale: f32, codepoints: &mut Vec<i32>) -> FontMetrics {
    let font_size_px = (font_size as f32 * dpi_scale) as i32;
    let font = unsafe {
        let f = raylib::ffi::LoadFontFromMemory(
            b".ttf\0".as_ptr() as *const i8,
            FONT_DATA.as_ptr() as *const u8,
            FONT_DATA.len() as i32,
            font_size_px,
            codepoints.as_mut_ptr(),
            codepoints.len() as i32,
        );
        raylib::ffi::SetTextureFilter(
            f.texture,
            raylib::ffi::TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
        );
        f
    };

    let glyph_size = unsafe {
        raylib::ffi::MeasureTextEx(
            font,
            b"M\0".as_ptr() as *const i8,
            font_size_px as f32,
            0.0,
        )
    };
    let cell_width = (glyph_size.x / dpi_scale) as i32;
    let cell_height = (glyph_size.y / dpi_scale) as i32;

    FontMetrics {
        font,
        cell_width: cell_width.max(1),
        cell_height: cell_height.max(1),
        font_size_px,
    }
}

fn build_terminal_codepoints() -> Vec<i32> {
    let mut cps: Vec<i32> = Vec::with_capacity(3000);
    cps.extend(32..=126);
    cps.extend(160..=255);
    cps.extend(0x100..=0x17F);
    cps.extend(0x180..=0x24F);
    cps.extend(0x2000..=0x206F);
    cps.extend(0x2070..=0x209F);
    cps.extend(0x20A0..=0x20CF);
    cps.extend(0x2100..=0x214F);
    cps.extend(0x2150..=0x218F);
    cps.extend(0x2190..=0x21FF);
    cps.extend(0x2200..=0x22FF);
    cps.extend(0x2300..=0x23FF);
    cps.extend(0x2400..=0x243F);
    cps.extend(0x2460..=0x24FF);
    cps.extend(0x2500..=0x257F);
    cps.extend(0x2580..=0x259F);
    cps.extend(0x25A0..=0x25FF);
    cps.extend(0x2600..=0x26FF);
    cps.extend(0x2700..=0x27BF);
    cps.extend(0x27C0..=0x27FF);
    cps.extend(0x2800..=0x28FF);
    cps.extend(0x2900..=0x297F);
    cps.extend(0xE0A0..=0xE0D4);
    cps.extend(0xE200..=0xE2FF);
    cps.extend(0xF000..=0xF0FF);
    cps
}

fn extract_selected_text(sel: &TextSelection, rows: &[String]) -> String {
    let (s, e) = match (sel.start, sel.end) {
        (Some(s), Some(e)) => {
            if s.row < e.row || (s.row == e.row && s.col <= e.col) {
                (s, e)
            } else {
                (e, s)
            }
        }
        _ => return String::new(),
    };

    let mut result = String::new();
    for row in s.row..=e.row {
        if row < 0 || row as usize >= rows.len() {
            continue;
        }
        let line = &rows[row as usize];
        let chars: Vec<char> = line.chars().collect();

        let col_start = if row == s.row { s.col as usize } else { 0 };
        let col_end = if row == e.row {
            (e.col as usize).min(chars.len().saturating_sub(1))
        } else {
            chars.len().saturating_sub(1)
        };

        if col_start <= col_end && col_start < chars.len() {
            let end = (col_end + 1).min(chars.len());
            let slice: String = chars[col_start..end].iter().collect();
            result.push_str(slice.trim_end());
        }
        if row < e.row {
            result.push('\n');
        }
    }
    result
}

fn relayout_and_resize(
    root: &mut SplitNode,
    scr_w: i32,
    scr_h: i32,
    status_bar_height: i32,
    sidebar_w: i32,
    pad: i32,
    minimap_width: i32,
    cell_width: i32,
    cell_height: i32,
) {
    root.layout(PanelRect { x: sidebar_w, y: 0, w: scr_w - sidebar_w, h: scr_h - status_bar_height });
    root.for_each_panel_mut(&mut |panel| {
        let (cols, rows) = panel_term_size(&panel.rect, pad, minimap_width, panel.tab_bar.height, cell_width, cell_height);
        for tab in &mut panel.tabs {
            tab.resize(cols, rows, cell_width as u32, cell_height as u32);
        }
    });
}

fn create_fresh_panel(
    config: &TaiConfig,
    scr_w: i32,
    scr_h: i32,
    status_bar_height: i32,
    sidebar_w: i32,
    pad: i32,
    minimap_width: i32,
    cell_width: i32,
    cell_height: i32,
) -> (SplitNode, u32, u32) {
    let mut next_panel_id: u32 = 0;
    let initial_id = alloc_panel_id(&mut next_panel_id);
    let initial_rect = PanelRect { x: sidebar_w, y: 0, w: scr_w - sidebar_w, h: scr_h - status_bar_height };
    let tab_bar_height = cell_height + 14;
    let (term_cols, term_rows) = panel_term_size(&initial_rect, pad, minimap_width, tab_bar_height, cell_width, cell_height);

    match create_panel(initial_id, config, term_cols, term_rows, cell_width, cell_height) {
        Ok(panel) => {
            let mut node = SplitNode::Leaf(panel);
            node.layout(initial_rect);
            (node, initial_id, next_panel_id)
        }
        Err(e) => {
            eprintln!("Failed to create initial panel: {e}");
            std::process::exit(1);
        }
    }
}

extern "C" fn signal_handler(_sig: nix::libc::c_int) {
    SIGNAL_EXIT.store(true, Ordering::SeqCst);
}

fn ssh_cwd_display(backend: &terminal::backend::Backend, osc_title: &str) -> String {
    if let Some(cwd) = backend.get_cwd() {
        return cwd.display().to_string();
    }
    // Fallback: try to extract path from terminal title (e.g. "user@host: /path" or "user@host:~/dir")
    if !osc_title.is_empty() {
        if let Some(colon_pos) = osc_title.find(':') {
            let after = osc_title[colon_pos + 1..].trim();
            if !after.is_empty() && (after.starts_with('/') || after.starts_with('~')) {
                return after.to_string();
            }
        }
    }
    "~".to_string()
}

fn find_resource(name: &str) -> Option<std::path::PathBuf> {
    // 1. Check relative to CWD (development: cargo run)
    let cwd = std::env::current_dir().ok()?;
    let p = cwd.join(name);
    if p.exists() {
        return Some(p);
    }
    // 2. Check relative to the executable (inside .app bundle: Contents/MacOS/../Resources/)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos_dir) = exe.parent() {
            let bundle_res = macos_dir.join("../Resources").join(name);
            if bundle_res.exists() {
                return Some(bundle_res);
            }
        }
    }
    None
}

fn set_app_icon() {
    let icon_path = match find_resource("assets/icon.png")
        .or_else(|| find_resource("icon.png"))
    {
        Some(p) => p,
        None => return,
    };

    // Set window icon via raylib (for non-macOS platforms)
    unsafe {
        let path_c = std::ffi::CString::new(icon_path.to_string_lossy().as_bytes()).ok();
        if let Some(ref p) = path_c {
            let mut icon = raylib::ffi::LoadImage(p.as_ptr());
            if !icon.data.is_null() {
                raylib::ffi::ImageFormat(
                    &mut icon,
                    raylib::ffi::PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8 as i32,
                );
                raylib::ffi::SetWindowIcon(icon);
                raylib::ffi::UnloadImage(icon);
            }
        }
    }

    // On macOS, SetWindowIcon doesn't affect the dock icon.
    // Use NSApplication.setApplicationIconImage via the ObjC runtime.
    #[cfg(target_os = "macos")]
    unsafe {
        type Id = *mut std::ffi::c_void;
        type Sel = *mut std::ffi::c_void;
        type MsgSend0 = unsafe extern "C" fn(Id, Sel) -> Id;
        type MsgSend1 = unsafe extern "C" fn(Id, Sel, Id) -> Id;
        type MsgSendStr = unsafe extern "C" fn(Id, Sel, *const i8) -> Id;

        unsafe extern "C" {
            fn objc_getClass(name: *const i8) -> Id;
            fn sel_registerName(name: *const i8) -> Sel;
            fn objc_msgSend();
        }

        let m0: MsgSend0 = std::mem::transmute(objc_msgSend as *const ());
        let m1: MsgSend1 = std::mem::transmute(objc_msgSend as *const ());
        let ms: MsgSendStr = std::mem::transmute(objc_msgSend as *const ());

        let app = m0(
            objc_getClass(c"NSApplication".as_ptr()),
            sel_registerName(c"sharedApplication".as_ptr()),
        );
        if app.is_null() {
            return;
        }

        let path_c = match std::ffi::CString::new(icon_path.to_string_lossy().as_bytes()) {
            Ok(c) => c,
            Err(_) => return,
        };
        let ns_path = ms(
            objc_getClass(c"NSString".as_ptr()),
            sel_registerName(c"stringWithUTF8String:".as_ptr()),
            path_c.as_ptr(),
        );

        let image = m0(
            objc_getClass(c"NSImage".as_ptr()),
            sel_registerName(c"alloc".as_ptr()),
        );
        let image = m1(
            image,
            sel_registerName(c"initWithContentsOfFile:".as_ptr()),
            ns_path,
        );
        if image.is_null() {
            return;
        }

        m1(
            app,
            sel_registerName(c"setApplicationIconImage:".as_ptr()),
            image,
        );
    }
}

fn inherit_shell_env() {
    if std::env::var("OPENAI_API_KEY").is_ok() {
        return;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let output = std::process::Command::new(&shell)
        .args(["-i", "-l", "-c", "env"])
        .stderr(std::process::Stdio::null())
        .output();
    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some((key, val)) = line.split_once('=') {
                if key == "OPENAI_API_KEY"
                    || key == "ANTHROPIC_API_KEY"
                    || (key.starts_with("TAI_") && std::env::var(key).is_err())
                {
                    unsafe { std::env::set_var(key, val); }
                }
            }
        }
    }
}

fn main() {
    inherit_shell_env();

    unsafe {
        nix::libc::signal(nix::libc::SIGTERM, signal_handler as *const () as nix::libc::sighandler_t);
        nix::libc::signal(nix::libc::SIGINT, signal_handler as *const () as nix::libc::sighandler_t);
    }

    let config = TaiConfig::load();
    let default_font_size = config.terminal.font_size;

    // --- CLI flags (BEFORE raylib init, headless operations exit early) ---
    let args: Vec<String> = std::env::args().collect();
    let mut cli_export: Option<String> = None;
    let mut cli_import: Option<String> = None;
    let mut cli_reset = false;
    let mut cli_list = false;

    {
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--export" => {
                    if let Some(name) = args.get(i + 1).cloned() {
                        cli_export = Some(name);
                    } else {
                        eprintln!("Error: --export requires a session name.");
                        return;
                    }
                    i += 2;
                }
                "--import" => {
                    if let Some(name) = args.get(i + 1).cloned() {
                        cli_import = Some(name);
                    } else {
                        eprintln!("Error: --import requires a session name.");
                        return;
                    }
                    i += 2;
                }
                "--reset" => {
                    cli_reset = true;
                    i += 1;
                }
                "--list-sessions" => {
                    cli_list = true;
                    i += 1;
                }
                _ => { i += 1; }
            }
        }
    }

    if cli_list {
        let names = session::list_sessions();
        if names.is_empty() {
            println!("No saved sessions.");
        } else {
            for name in &names {
                println!("{name}");
            }
        }
        return;
    }

    if let Some(name) = cli_export {
        match session::export_session(&name) {
            Ok(()) => println!("Session exported as '{name}'."),
            Err(e) => eprintln!("Export failed: {e}"),
        }
        return;
    }

    if cli_reset {
        if let Err(e) = session::reset_session() {
            eprintln!("Reset failed: {e}");
        }
    }

    // --- Load session state (for font size and later restore) ---
    let loaded_session = if cli_reset {
        None
    } else if let Some(ref name) = cli_import {
        match session::import_session(name) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Import failed: {e}");
                None
            }
        }
    } else {
        match session::load() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[TAI] Failed to load session: {e}");
                None
            }
        }
    };

    let mut font_size = loaded_session
        .as_ref()
        .map(|s| s.font_size)
        .unwrap_or(default_font_size);

    let (saved_win_w, saved_win_h, saved_win_x, saved_win_y) = loaded_session
        .as_ref()
        .map(|s| (s.window_width, s.window_height, s.window_x, s.window_y))
        .unwrap_or((0, 0, 0, 0));

    let init_w = if saved_win_w > 0 { saved_win_w } else { 1200 };
    let init_h = if saved_win_h > 0 { saved_win_h } else { 750 };

    unsafe {
        raylib::ffi::SetConfigFlags(
            raylib::consts::ConfigFlags::FLAG_WINDOW_HIGHDPI as u32,
        );
    }

    let (mut rl, thread) = raylib::init()
        .size(init_w, init_h)
        .title("Terminal AI")
        .resizable()
        .build();

    if saved_win_w > 0 && saved_win_h > 0 {
        unsafe { raylib::ffi::SetWindowPosition(saved_win_x, saved_win_y); }
    }

    set_app_icon();

    rl.set_target_fps(60);

    // Derive DPI scale from render_w / screen_w. This updates reliably on macOS
    // whenever the window moves to a display with a different pixel density,
    // unlike GetWindowScaleDPI() which can report stale values.
    let compute_dpi = |rl: &RaylibHandle| -> f32 {
        let screen_w = rl.get_screen_width() as f32;
        let render_w = unsafe { raylib::ffi::GetRenderWidth() as f32 };
        let raw = unsafe { raylib::ffi::GetWindowScaleDPI().y };
        let ratio = if screen_w > 0.0 { render_w / screen_w } else { raw };
        // Prefer the render/screen ratio but fall back to GetWindowScaleDPI
        // when the ratio is suspicious (e.g. during window init).
        if ratio >= 0.5 && ratio <= 4.0 { ratio } else { raw.max(1.0) }
    };
    let mut dpi_scale = compute_dpi(&rl);
    eprintln!("[TAI] Startup DPI scale: {}", dpi_scale);

    let mut codepoints = build_terminal_codepoints();
    let mut metrics = load_font(font_size, dpi_scale, &mut codepoints);
    let mut mono_font = metrics.font;
    let mut cell_width = metrics.cell_width;
    let mut cell_height = metrics.cell_height;

    let pad = 4;
    let minimap_width = 40;
    let mut status_bar_height = font_size + 8;

    let scr_w = rl.get_screen_width();
    let scr_h = rl.get_screen_height();

    let ai_available = config.api_key().is_some();
    let status_bar = StatusBar::new(&config.ai.model, ai_available);

    // --- Restore session or create fresh ---
    let mut wm = if let Some(state) = loaded_session {
        let initial_sidebar_w = if state.sidebar_visible { state.sidebar_width_px } else { 0 };
        match session::restore(
            state,
            &config,
            scr_w, scr_h,
            status_bar_height,
            initial_sidebar_w,
            pad, minimap_width,
            cell_width, cell_height,
        ) {
            Ok((restored_wm, _restored_font_size)) => restored_wm,
            Err(e) => {
                eprintln!("[TAI] Session restore failed, starting fresh: {e}");
                let (root, fid, nid) = create_fresh_panel(&config, scr_w, scr_h, status_bar_height, 0, pad, minimap_width, cell_width, cell_height);
                WorkspaceManager::new(Workspace {
                    name: "default".to_string(), root,
                    focused_panel_id: fid, next_panel_id: nid,
                    ssh_info: None, ssh_password: String::new(),
                })
            }
        }
    } else {
        let (root, fid, nid) = create_fresh_panel(&config, scr_w, scr_h, status_bar_height, 0, pad, minimap_width, cell_width, cell_height);
        WorkspaceManager::new(Workspace {
            name: "default".to_string(), root,
            focused_panel_id: fid, next_panel_id: nid,
            ssh_info: None, ssh_password: String::new(),
        })
    };

    let mut prev_width = scr_w;
    let mut prev_height = scr_h;
    let mut title_frame: u32 = 0;
    let mut last_title = String::new();

    unsafe { raylib::ffi::SetExitKey(0); }

    let mut app_exit = false;
    let mut show_help = false;
    let mut help_scroll: i32 = 0;
    let mut show_session_mgr = false;
    let mut sm_names: Vec<String> = Vec::new();
    let mut sm_selected: usize = 0;
    let mut sm_scroll: usize = 0;
    let mut sm_input: Option<String> = None;
    let mut sm_status: Option<(String, Instant)> = None;
    let mut last_autosave = Instant::now();

    let mut sidebar_last_click: Option<(usize, Instant)> = None;
    let mut separator_drag: Option<split::SeparatorHit> = None;
    let mut sidebar_drag: bool = false;

    let mut show_ssh_connect = false;
    let mut ssh_host = String::new();
    let mut ssh_port = String::from("22");
    let mut ssh_user = String::new();
    let mut ssh_password = String::new();
    let mut ssh_focus: usize = 0; // 0=host, 1=port, 2=user, 3=password
    let mut ssh_error: Option<String> = None;
    let mut ssh_manager = crate::terminal::ssh::SshConnectionManager::new();

    // Reconnect SSH for all workspaces that have SSH info
    for ws_idx in 0..wm.workspaces.len() {
        let ssh_info = wm.workspaces[ws_idx].ssh_info.clone();
        let ssh_password = wm.workspaces[ws_idx].ssh_password.clone();
        if let Some(ref info) = ssh_info {
            match ssh_manager.get_or_connect(&info.host, info.port, &info.user, &ssh_password) {
                Ok(session) => {
                    wm.workspaces[ws_idx].root.for_each_panel_mut(&mut |panel| {
                        for tab in &mut panel.tabs {
                            if tab.ssh_info.is_some() && tab.child_exited {
                                let (cols, rows) = panel_term_size(
                                    &panel.rect, pad, minimap_width, panel.tab_bar.height, cell_width, cell_height,
                                );
                                if let Ok(ssh_backend) = ssh_manager.open_channel(&session, cols, rows, info.clone()) {
                                    tab.revive_ssh(ssh_backend, cell_width, cell_height);
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("[TAI] Failed to reconnect SSH for workspace '{}': {e}", info.host);
                }
            }
        }
    }

    while !rl.window_should_close() && !app_exit && !SIGNAL_EXIT.load(Ordering::Relaxed) {
        if last_autosave.elapsed().as_secs() >= 5 {
            let win_pos = unsafe { raylib::ffi::GetWindowPosition() };
            if let Err(e) = session::save(
                &wm, font_size,
                rl.get_screen_width(), rl.get_screen_height(),
                win_pos.x as i32, win_pos.y as i32,
            ) {
                eprintln!("[TAI] Autosave failed: {e}");
            }
            last_autosave = Instant::now();
        }
        // Handle resize and DPI changes (e.g. moving to an external monitor).
        let current_dpi = compute_dpi(&rl);
        let dpi_changed = (current_dpi - dpi_scale).abs() > 0.05;
        let w = rl.get_screen_width();
        let h = rl.get_screen_height();
        let size_changed = w != prev_width || h != prev_height;
        if rl.is_window_resized() || dpi_changed || size_changed {
            if dpi_changed {
                eprintln!(
                    "[TAI] DPI scale changed: {:.3} -> {:.3}, reloading font at size {} ({} panels)",
                    dpi_scale, current_dpi, font_size,
                    wm.workspaces.iter().map(|w| w.panel_count()).sum::<usize>(),
                );
                let old_font = mono_font;
                dpi_scale = current_dpi;
                metrics = load_font(font_size, dpi_scale, &mut codepoints);
                mono_font = metrics.font;
                cell_width = metrics.cell_width;
                cell_height = metrics.cell_height;
                status_bar_height = font_size + 8;
                // Propagate new cell height to every panel's tab bar.
                for ws in &mut wm.workspaces {
                    ws.root.for_each_panel_mut(&mut |panel| {
                        panel.tab_bar.update_height(cell_height);
                    });
                }
                unsafe { raylib::ffi::UnloadFont(old_font); }
            }
            let sidebar_w = wm.sidebar_width();
            for ws in &mut wm.workspaces {
                relayout_and_resize(
                    &mut ws.root, w, h, status_bar_height, sidebar_w,
                    pad, minimap_width, cell_width, cell_height,
                );
            }
            prev_width = w;
            prev_height = h;
        }

        // Read PTY and poll AI for ALL workspaces/panels/tabs
        for ws in &mut wm.workspaces {
            ws.root.for_each_panel_mut(&mut |panel| {
                for tab in &mut panel.tabs {
                    tab.read_pty();
                    tab.poll_ai();
                }
            });
        }

        // Update window title from focused panel's active tab
        title_frame += 1;
        if title_frame % 30 == 0 {
            let ws = wm.active();
            if let Some(panel) = ws.root.panel_by_id(ws.focused_panel_id) {
                let tab_title = panel.active_tab().title();
                let new_title = if wm.workspaces.len() > 1 {
                    if tab_title == "shell" {
                        format!("Terminal AI - [{}]", ws.name)
                    } else {
                        format!("Terminal AI - [{}] {}", ws.name, tab_title)
                    }
                } else if tab_title == "shell" {
                    "Terminal AI".to_string()
                } else {
                    format!("Terminal AI - {}", tab_title)
                };
                if new_title != last_title {
                    let c_title = std::ffi::CString::new(new_title.as_str()).unwrap_or_default();
                    unsafe { raylib::ffi::SetWindowTitle(c_title.as_ptr()); }
                    last_title = new_title;
                }
            }
        }

        // Key modifiers
        let cmd_held = rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER);
        let shift_held = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);
        let ctrl_held = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);
        let alt_held = rl.is_key_down(KeyboardKey::KEY_LEFT_ALT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT);

        // Help panel toggle (F1)
        if rl.is_key_pressed(KeyboardKey::KEY_F1) {
            show_help = !show_help;
            if show_help {
                show_session_mgr = false;
                help_scroll = 0;
            }
        } else if show_help && rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            show_help = false;
        }

        if show_help {
            loop {
                if unsafe { raylib::ffi::GetCharPressed() } == 0 { break; }
            }
            let wheel = unsafe { raylib::ffi::GetMouseWheelMove() } as i32;
            if wheel != 0 {
                help_scroll = (help_scroll - wheel * 24).max(0);
            }
        }

        // Session manager toggle (F2)
        if rl.is_key_pressed(KeyboardKey::KEY_F2) {
            show_session_mgr = !show_session_mgr;
            if show_session_mgr {
                show_help = false;
                sm_names = session::list_sessions();
                sm_selected = 0;
                sm_scroll = 0;
                sm_input = None;
                sm_status = None;
            }
        }

        // Session manager status message expiry
        if sm_status.as_ref().map_or(false, |(_, ts)| ts.elapsed().as_secs() >= 3) {
            sm_status = None;
        }

        // Session manager input handling
        if show_session_mgr {
            const SM_MAX_VISIBLE: usize = 15;

            if let Some(ref mut input) = sm_input {
                // Name-input mode: read chars into buffer
                loop {
                    let ch = unsafe { raylib::ffi::GetCharPressed() };
                    if ch == 0 { break; }
                    if let Some(c) = char::from_u32(ch as u32) {
                        if c as u32 > 31 && c != '/' && c != '\0' && input.len() < 64 {
                            input.push(c);
                        }
                    }
                }
                if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
                    || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_BACKSPACE as i32) }
                {
                    input.pop();
                }
                if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                    let name = input.clone();
                    if !name.is_empty() {
                        let win_pos = unsafe { raylib::ffi::GetWindowPosition() };
                        match session::save(
                            &wm, font_size,
                            rl.get_screen_width(), rl.get_screen_height(),
                            win_pos.x as i32, win_pos.y as i32,
                        ) {
                            Ok(()) => {
                                match session::export_session(&name) {
                                    Ok(()) => {
                                        sm_names = session::list_sessions();
                                        sm_selected = sm_names.iter().position(|n| n == &name).unwrap_or(0);
                                        if sm_selected >= sm_scroll + SM_MAX_VISIBLE {
                                            sm_scroll = sm_selected.saturating_sub(SM_MAX_VISIBLE - 1);
                                        } else if sm_selected < sm_scroll {
                                            sm_scroll = sm_selected;
                                        }
                                        sm_status = Some((format!("Session '{}' exported", name), Instant::now()));
                                    }
                                    Err(e) => {
                                        sm_status = Some((format!("Export failed: {e}"), Instant::now()));
                                    }
                                }
                            }
                            Err(e) => {
                                sm_status = Some((format!("Save failed: {e}"), Instant::now()));
                            }
                        }
                        sm_input = None;
                    }
                }
                if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                    sm_input = None;
                }
            } else {
                // List mode: swallow all chars to prevent leaking to terminal
                loop {
                    if unsafe { raylib::ffi::GetCharPressed() } == 0 { break; }
                }

                if rl.is_key_pressed(KeyboardKey::KEY_UP)
                    || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_UP as i32) }
                {
                    sm_selected = sm_selected.saturating_sub(1);
                    if sm_selected < sm_scroll {
                        sm_scroll = sm_selected;
                    }
                }
                if rl.is_key_pressed(KeyboardKey::KEY_DOWN)
                    || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_DOWN as i32) }
                {
                    sm_selected = (sm_selected + 1).min(sm_names.len().saturating_sub(1));
                    if sm_selected >= sm_scroll + SM_MAX_VISIBLE {
                        sm_scroll = sm_selected.saturating_sub(SM_MAX_VISIBLE - 1);
                    }
                }
                if rl.is_key_pressed(KeyboardKey::KEY_S) {
                    sm_input = Some(String::new());
                }
                if rl.is_key_pressed(KeyboardKey::KEY_D) && !sm_names.is_empty() {
                    let name = sm_names[sm_selected].clone();
                    match session::delete_named_session(&name) {
                        Ok(()) => {
                            sm_status = Some((format!("Session '{}' deleted", name), Instant::now()));
                        }
                        Err(e) => {
                            sm_status = Some((format!("{e}"), Instant::now()));
                        }
                    }
                    sm_names = session::list_sessions();
                    sm_selected = sm_selected.min(sm_names.len().saturating_sub(1));
                    if sm_selected < sm_scroll {
                        sm_scroll = sm_selected;
                    }
                    if !sm_names.is_empty() && sm_scroll > sm_names.len().saturating_sub(SM_MAX_VISIBLE) {
                        sm_scroll = sm_names.len().saturating_sub(SM_MAX_VISIBLE);
                    }
                }
                if rl.is_key_pressed(KeyboardKey::KEY_ENTER) && !sm_names.is_empty() {
                    // Load selected session (in-app)
                    let win_pos = unsafe { raylib::ffi::GetWindowPosition() };
                    let _ = session::save(
                        &wm, font_size,
                        rl.get_screen_width(), rl.get_screen_height(),
                        win_pos.x as i32, win_pos.y as i32,
                    );

                    let name = sm_names[sm_selected].clone();
                    match session::import_session(&name) {
                        Ok(Some(state)) => {
                            let new_font_size = state.font_size;
                            let font_changed = new_font_size != font_size;
                            let old_font_size = font_size;
                            let old_mono_font = mono_font;
                            let old_cell_width = cell_width;
                            let old_cell_height = cell_height;
                            let old_status_bar_height = status_bar_height;

                            if font_changed {
                                font_size = new_font_size;
                                metrics = load_font(font_size, dpi_scale, &mut codepoints);
                                mono_font = metrics.font;
                                cell_width = metrics.cell_width;
                                cell_height = metrics.cell_height;
                                status_bar_height = font_size + 8;
                            }

                            let load_sidebar_w = if state.sidebar_visible { state.sidebar_width_px } else { 0 };

                            let scr_w = rl.get_screen_width();
                            let scr_h = rl.get_screen_height();
                            match session::restore(
                                state, &config, scr_w, scr_h,
                                status_bar_height, load_sidebar_w, pad, minimap_width, cell_width, cell_height,
                            ) {
                                Ok((new_wm, _)) => {
                                    if font_changed {
                                        unsafe { raylib::ffi::UnloadFont(old_mono_font); }
                                    }
                                    ssh_manager.clear();
                                    wm = new_wm;

                                    // Reconnect SSH for all workspaces
                                    for ws_idx in 0..wm.workspaces.len() {
                                        let ssh_info = wm.workspaces[ws_idx].ssh_info.clone();
                                        let ssh_password = wm.workspaces[ws_idx].ssh_password.clone();
                                        if let Some(ref info) = ssh_info {
                                            if let Ok(sess) = ssh_manager.get_or_connect(&info.host, info.port, &info.user, &ssh_password) {
                                                wm.workspaces[ws_idx].root.for_each_panel_mut(&mut |panel| {
                                                    for tab in &mut panel.tabs {
                                                        if tab.ssh_info.is_some() && tab.child_exited {
                                                            let (cols, rows) = panel_term_size(
                                                                &panel.rect, pad, minimap_width, panel.tab_bar.height, cell_width, cell_height,
                                                            );
                                                            if let Ok(backend) = ssh_manager.open_channel(&sess, cols, rows, info.clone()) {
                                                                tab.revive_ssh(backend, cell_width, cell_height);
                                                            }
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                    }

                                    last_autosave = Instant::now();
                                    last_title.clear();
                                    show_session_mgr = false;
                                }
                                Err(e) => {
                                    if font_changed {
                                        unsafe { raylib::ffi::UnloadFont(mono_font); }
                                        mono_font = old_mono_font;
                                        font_size = old_font_size;
                                        cell_width = old_cell_width;
                                        cell_height = old_cell_height;
                                        status_bar_height = old_status_bar_height;
                                    }
                                    sm_status = Some((format!("Restore failed: {e}"), Instant::now()));
                                }
                            }
                        }
                        Ok(None) => {
                            sm_status = Some(("Session corrupted or incompatible".into(), Instant::now()));
                        }
                        Err(e) => {
                            sm_status = Some((format!("Import failed: {e}"), Instant::now()));
                        }
                    }
                }
                if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                    show_session_mgr = false;
                }
            }
        }

        // SSH connect overlay toggle (Cmd+Shift+S)
        if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_S) && !show_help && !show_session_mgr {
            show_ssh_connect = !show_ssh_connect;
            if show_ssh_connect {
                ssh_host.clear();
                ssh_port = "22".to_string();
                ssh_user.clear();
                ssh_password.clear();
                ssh_focus = 0;
                ssh_error = None;
            }
        }

        // SSH connect overlay input handling
        if show_ssh_connect {
            if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                show_ssh_connect = false;
            } else if rl.is_key_pressed(KeyboardKey::KEY_TAB) {
                ssh_focus = (ssh_focus + 1) % 4;
            } else if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                let port: u16 = ssh_port.parse().unwrap_or(22);
                let info = crate::terminal::ssh::SshTabInfo {
                    host: ssh_host.clone(),
                    port,
                    user: ssh_user.clone(),
                };

                let (cols, rows) = {
                    let ws = wm.active();
                    if let Some(fp) = ws.root.panel_by_id(ws.focused_panel_id) {
                        let tbh = fp.tab_bar.height;
                        panel_term_size(&fp.rect, pad, minimap_width, tbh, cell_width, cell_height)
                    } else {
                        (80, 24)
                    }
                };

                let try_connect = |mgr: &mut crate::terminal::ssh::SshConnectionManager| -> Result<crate::terminal::ssh::SshBackend, String> {
                    let session = mgr.get_or_connect(&info.host, info.port, &info.user, &ssh_password)?;
                    mgr.open_channel(&session, cols, rows, info.clone())
                };

                let result = try_connect(&mut ssh_manager);
                let result = match result {
                    Ok(backend) => Ok(backend),
                    Err(_) => {
                        ssh_manager.remove(&info.host, info.port, &info.user);
                        try_connect(&mut ssh_manager)
                    }
                };

                match result {
                    Ok(ssh_backend) => {
                        match crate::tab::TabSession::new_ssh(&config, ssh_backend, cols, rows, cell_width, cell_height) {
                            Ok(tab) => {
                                let panel = split::Panel::new(0, tab, cell_height);
                                let ws_name = format!("{}@{}", info.user, info.host);
                                wm.add(Workspace {
                                    name: ws_name,
                                    root: SplitNode::Leaf(panel),
                                    focused_panel_id: 0,
                                    next_panel_id: 1,
                                    ssh_info: Some(info),
                                    ssh_password: ssh_password.clone(),
                                });
                                let sidebar_w = wm.sidebar_width();
                                let scr_w = rl.get_screen_width();
                                let scr_h = rl.get_screen_height();
                                relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                                show_ssh_connect = false;
                                last_title.clear();
                            }
                            Err(e) => {
                                ssh_error = Some(format!("Tab creation failed: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        ssh_error = Some(e);
                    }
                }
            } else {
                let field = match ssh_focus {
                    0 => &mut ssh_host,
                    1 => &mut ssh_port,
                    2 => &mut ssh_user,
                    _ => &mut ssh_password,
                };
                if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE) || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_BACKSPACE as i32) } {
                    field.pop();
                } else {
                    loop {
                        let ch = unsafe { raylib::ffi::GetCharPressed() };
                        if ch == 0 { break; }
                        if let Some(c) = char::from_u32(ch as u32) {
                            field.push(c);
                        }
                    }
                }
            }
        }

        // Sidebar rename input handling
        if wm.renaming.is_some() {
            if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                wm.renaming = None;
                wm.rename_buf.clear();
            } else if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                if let Some(idx) = wm.renaming {
                    let trimmed = wm.rename_buf.trim().to_string();
                    if !trimmed.is_empty() && idx < wm.workspaces.len() {
                        wm.workspaces[idx].name = trimmed;
                    }
                }
                wm.renaming = None;
                wm.rename_buf.clear();
            } else {
                if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE) || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_BACKSPACE as i32) } {
                    wm.rename_buf.pop();
                } else {
                    loop {
                        let ch = unsafe { raylib::ffi::GetCharPressed() };
                        if ch == 0 { break; }
                        if let Some(c) = char::from_u32(ch as u32) {
                            if c as u32 > 31 && c != '\0' && wm.rename_buf.len() < 32 {
                                wm.rename_buf.push(c);
                            }
                        }
                    }
                }
            }
        }

        // Split keybindings (Cmd+D, Cmd+Shift+D, Cmd+Shift+W, Cmd+Option+Arrows)
        let mut did_split_action = false;
        if show_help || show_session_mgr || show_ssh_connect || wm.renaming.is_some() {
            // all input blocked while overlay is open
        } else if cmd_held && !shift_held && rl.is_key_pressed(KeyboardKey::KEY_D) {
            let sidebar_w = wm.sidebar_width();
            let ws = wm.active_mut();
            let new_id = alloc_panel_id(&mut ws.next_panel_id);
            if let Some(fp) = ws.root.panel_by_id(ws.focused_panel_id) {
                let tbh = fp.tab_bar.height;
                let r = fp.rect;
                let (cols, rows) = panel_term_size(&PanelRect { x: 0, y: 0, w: r.w / 2, h: r.h }, pad, minimap_width, tbh, cell_width, cell_height);
                let tab_result = if let Some(ref info) = ws.ssh_info {
                    ssh_manager.get_or_connect(&info.host, info.port, &info.user, &ws.ssh_password)
                        .and_then(|s| ssh_manager.open_channel(&s, cols, rows, info.clone()))
                        .and_then(|backend| TabSession::new_ssh(&config, backend, cols, rows, cell_width, cell_height))
                } else {
                    TabSession::new(&config, cols, rows, cell_width, cell_height)
                };
                if let Ok(tab) = tab_result {
                    let new_panel = split::Panel::new(new_id, tab, cell_height);
                    ws.root.split_panel(ws.focused_panel_id, SplitDirection::Horizontal, new_panel);
                    let scr_w = rl.get_screen_width();
                    let scr_h = rl.get_screen_height();
                    relayout_and_resize(&mut ws.root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                    ws.focused_panel_id = new_id;
                    last_title.clear();
                }
            }
            did_split_action = true;
        } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_D) {
            let sidebar_w = wm.sidebar_width();
            let ws = wm.active_mut();
            let new_id = alloc_panel_id(&mut ws.next_panel_id);
            if let Some(fp) = ws.root.panel_by_id(ws.focused_panel_id) {
                let tbh = fp.tab_bar.height;
                let r = fp.rect;
                let (cols, rows) = panel_term_size(&PanelRect { x: 0, y: 0, w: r.w, h: r.h / 2 }, pad, minimap_width, tbh, cell_width, cell_height);
                let tab_result = if let Some(ref info) = ws.ssh_info {
                    ssh_manager.get_or_connect(&info.host, info.port, &info.user, &ws.ssh_password)
                        .and_then(|s| ssh_manager.open_channel(&s, cols, rows, info.clone()))
                        .and_then(|backend| TabSession::new_ssh(&config, backend, cols, rows, cell_width, cell_height))
                } else {
                    TabSession::new(&config, cols, rows, cell_width, cell_height)
                };
                if let Ok(tab) = tab_result {
                    let new_panel = split::Panel::new(new_id, tab, cell_height);
                    ws.root.split_panel(ws.focused_panel_id, SplitDirection::Vertical, new_panel);
                    let scr_w = rl.get_screen_width();
                    let scr_h = rl.get_screen_height();
                    relayout_and_resize(&mut ws.root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                    ws.focused_panel_id = new_id;
                    last_title.clear();
                }
            }
            did_split_action = true;
        } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_W) {
            let panel_count = wm.active().root.panel_count();
            if panel_count > 1 {
                let ws = wm.active_mut();
                let fid = ws.focused_panel_id;
                let leaves = ws.root.collect_leaves();
                let idx = leaves.iter().position(|&id| id == fid).unwrap_or(0);
                ws.root.close_panel(fid);
                let new_leaves = ws.root.collect_leaves();
                ws.focused_panel_id = new_leaves[idx.min(new_leaves.len() - 1)];
                let sidebar_w = wm.sidebar_width();
                let scr_w = rl.get_screen_width();
                let scr_h = rl.get_screen_height();
                relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                last_title.clear();
            } else if wm.workspaces.len() > 1 {
                let idx = wm.active;
                wm.remove(idx);
                let sidebar_w = wm.sidebar_width();
                let scr_w = rl.get_screen_width();
                let scr_h = rl.get_screen_height();
                relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_RIGHT) {
            let ws = wm.active_mut();
            let leaves = ws.root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == ws.focused_panel_id) {
                ws.focused_panel_id = leaves[(idx + 1) % leaves.len()];
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_LEFT) {
            let ws = wm.active_mut();
            let leaves = ws.root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == ws.focused_panel_id) {
                ws.focused_panel_id = leaves[if idx == 0 { leaves.len() - 1 } else { idx - 1 }];
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_DOWN) {
            let ws = wm.active_mut();
            let leaves = ws.root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == ws.focused_panel_id) {
                ws.focused_panel_id = leaves[(idx + 1) % leaves.len()];
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_UP) {
            let ws = wm.active_mut();
            let leaves = ws.root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == ws.focused_panel_id) {
                ws.focused_panel_id = leaves[if idx == 0 { leaves.len() - 1 } else { idx - 1 }];
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && !shift_held && !ctrl_held && rl.is_key_pressed(KeyboardKey::KEY_N) {
            // New local workspace
            let name = wm.next_name();
            let future_sidebar_w = wm.sidebar_width();
            let scr_w = rl.get_screen_width();
            let scr_h = rl.get_screen_height();
            let (root, fid, nid) = create_fresh_panel(&config, scr_w, scr_h, status_bar_height, future_sidebar_w, pad, minimap_width, cell_width, cell_height);
            wm.add(Workspace {
                name, root, focused_panel_id: fid, next_panel_id: nid,
                ssh_info: None, ssh_password: String::new(),
            });
            let sidebar_w = wm.sidebar_width();
            relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
            last_title.clear();
            did_split_action = true;
        } else if ctrl_held && cmd_held && rl.is_key_pressed(KeyboardKey::KEY_RIGHT_BRACKET) {
            wm.next();
            let sidebar_w = wm.sidebar_width();
            let scr_w = rl.get_screen_width();
            let scr_h = rl.get_screen_height();
            relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
            last_title.clear();
            did_split_action = true;
        } else if ctrl_held && cmd_held && rl.is_key_pressed(KeyboardKey::KEY_LEFT_BRACKET) {
            wm.prev();
            let sidebar_w = wm.sidebar_width();
            let scr_w = rl.get_screen_width();
            let scr_h = rl.get_screen_height();
            relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
            last_title.clear();
            did_split_action = true;
        } else if ctrl_held && cmd_held {
            let key_nums = [
                KeyboardKey::KEY_ONE, KeyboardKey::KEY_TWO, KeyboardKey::KEY_THREE,
                KeyboardKey::KEY_FOUR, KeyboardKey::KEY_FIVE, KeyboardKey::KEY_SIX,
                KeyboardKey::KEY_SEVEN, KeyboardKey::KEY_EIGHT, KeyboardKey::KEY_NINE,
            ];
            for (i, &key) in key_nums.iter().enumerate() {
                if rl.is_key_pressed(key) && i < wm.workspaces.len() {
                    wm.switch_to(i);
                    let sidebar_w = wm.sidebar_width();
                    let scr_w = rl.get_screen_width();
                    let scr_h = rl.get_screen_height();
                    relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                    last_title.clear();
                    did_split_action = true;
                    break;
                }
            }
        } else if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_BACKSLASH) {
            wm.sidebar_visible = !wm.sidebar_visible;
            let sidebar_w = wm.sidebar_width();
            let scr_w = rl.get_screen_width();
            let scr_h = rl.get_screen_height();
            relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
            did_split_action = true;
        }

        if show_session_mgr || show_ssh_connect || wm.renaming.is_some() {
            // input handled by overlay above
        } else if did_split_action {
            // skip other keybindings this frame
        } else {
            // Tab management keybindings (scoped to focused panel)
            let mut tab_action_done = false;
            if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_T) {
                let ws = wm.active_mut();
                if let Some(panel) = ws.root.panel_by_id_mut(ws.focused_panel_id) {
                    let (cols, rows) = panel_term_size(&panel.rect, pad, minimap_width, panel.tab_bar.height, cell_width, cell_height);
                    let tab_result = if let Some(ref info) = ws.ssh_info {
                        let sess = ssh_manager.get_or_connect(&info.host, info.port, &info.user, &ws.ssh_password);
                        sess.and_then(|s| ssh_manager.open_channel(&s, cols, rows, info.clone()))
                            .and_then(|backend| TabSession::new_ssh(&config, backend, cols, rows, cell_width, cell_height))
                    } else {
                        TabSession::new(&config, cols, rows, cell_width, cell_height)
                    };
                    if let Ok(t) = tab_result {
                        panel.tabs.push(t);
                        panel.active_tab = panel.tabs.len() - 1;
                        last_title.clear();
                    }
                }
                tab_action_done = true;
            } else if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_W) {
                // Phase 1: remove the tab
                let mut tab_removed_empty = false;
                {
                    let ws = wm.active_mut();
                    if let Some(panel) = ws.root.panel_by_id_mut(ws.focused_panel_id) {
                        panel.tabs.remove(panel.active_tab);
                        if panel.tabs.is_empty() {
                            tab_removed_empty = true;
                        } else {
                            panel.active_tab = panel.active_tab.min(panel.tabs.len() - 1);
                            last_title.clear();
                        }
                    }
                }
                // Phase 2: handle empty panel
                if tab_removed_empty {
                    let panel_count = wm.active().root.panel_count();
                    if panel_count <= 1 {
                        if wm.workspaces.len() <= 1 {
                            app_exit = true;
                        } else {
                            let idx = wm.active;
                            wm.remove(idx);
                            let sidebar_w = wm.sidebar_width();
                            let scr_w = rl.get_screen_width();
                            let scr_h = rl.get_screen_height();
                            relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                            last_title.clear();
                        }
                    } else {
                        let sidebar_w = wm.sidebar_width();
                        let ws = wm.active_mut();
                        let fid = ws.focused_panel_id;
                        let leaves = ws.root.collect_leaves();
                        let idx = leaves.iter().position(|&id| id == fid).unwrap_or(0);
                        ws.root.close_panel(fid);
                        let new_leaves = ws.root.collect_leaves();
                        ws.focused_panel_id = new_leaves[idx.min(new_leaves.len() - 1)];
                        let scr_w = rl.get_screen_width();
                        let scr_h = rl.get_screen_height();
                        relayout_and_resize(&mut ws.root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                        last_title.clear();
                    }
                }
                tab_action_done = true;
            } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_RIGHT_BRACKET) {
                let ws = wm.active_mut();
                if let Some(panel) = ws.root.panel_by_id_mut(ws.focused_panel_id) {
                    panel.active_tab = (panel.active_tab + 1) % panel.tabs.len();
                    last_title.clear();
                }
                tab_action_done = true;
            } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_LEFT_BRACKET) {
                let ws = wm.active_mut();
                if let Some(panel) = ws.root.panel_by_id_mut(ws.focused_panel_id) {
                    panel.active_tab = if panel.active_tab == 0 { panel.tabs.len() - 1 } else { panel.active_tab - 1 };
                    last_title.clear();
                }
                tab_action_done = true;
            } else if cmd_held {
                let key_nums = [
                    KeyboardKey::KEY_ONE, KeyboardKey::KEY_TWO, KeyboardKey::KEY_THREE,
                    KeyboardKey::KEY_FOUR, KeyboardKey::KEY_FIVE, KeyboardKey::KEY_SIX,
                    KeyboardKey::KEY_SEVEN, KeyboardKey::KEY_EIGHT, KeyboardKey::KEY_NINE,
                ];
                let ws = wm.active_mut();
                for (i, &key) in key_nums.iter().enumerate() {
                    if rl.is_key_pressed(key) {
                        if let Some(panel) = ws.root.panel_by_id_mut(ws.focused_panel_id) {
                            if i < panel.tabs.len() {
                                panel.active_tab = i;
                                last_title.clear();
                                tab_action_done = true;
                                break;
                            }
                        }
                    }
                }
            }

            if app_exit {
                // pass through to exit
            } else if !tab_action_done {
                // Font size change
                let font_size_change = if cmd_held && (rl.is_key_pressed(KeyboardKey::KEY_EQUAL) || rl.is_key_pressed(KeyboardKey::KEY_KP_ADD)) {
                    Some((font_size + 2).min(72))
                } else if cmd_held && (rl.is_key_pressed(KeyboardKey::KEY_MINUS) || rl.is_key_pressed(KeyboardKey::KEY_KP_SUBTRACT)) {
                    Some((font_size - 2).max(8))
                } else if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_ZERO) {
                    Some(default_font_size)
                } else {
                    None
                };

                if let Some(new_size) = font_size_change {
                    font_size = new_size;
                    let old_font = mono_font;
                    metrics = load_font(font_size, dpi_scale, &mut codepoints);
                    mono_font = metrics.font;
                    cell_width = metrics.cell_width;
                    cell_height = metrics.cell_height;
                    status_bar_height = font_size + 8;
                    for ws in &mut wm.workspaces {
                        ws.root.for_each_panel_mut(&mut |panel| {
                            panel.tab_bar.update_height(cell_height);
                        });
                    }
                    unsafe { raylib::ffi::UnloadFont(old_font); }
                    let w = rl.get_screen_width();
                    let h = rl.get_screen_height();
                    let sidebar_w = wm.sidebar_width();
                    relayout_and_resize(&mut wm.active_mut().root, w, h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                }

                // Copy/Paste (focused panel's active tab)
                let ws = wm.active_mut();
                if let Some(panel) = ws.root.panel_by_id_mut(ws.focused_panel_id) {
                    let tab = panel.active_tab_mut();
                    if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_C) {
                        if tab.selection.has_selection() {
                            let rows = tab.term.get_viewport_rows();
                            let selected = extract_selected_text(&tab.selection, &rows);
                            if !selected.is_empty() {
                                selection::copy_to_clipboard(&selected);
                            }
                            tab.selection.clear();
                        }
                    }
                    if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_V) {
                        if let Some(text) = selection::paste_from_clipboard() {
                            if !text.is_empty() {
                                tab.backend.write(text.as_bytes());
                            }
                        }
                    }
                }

                // Mouse handling -- route to hovered panel
                {
                    let mx = rl.get_mouse_x();
                    let my = rl.get_mouse_y();
                    let sidebar_w = wm.sidebar_width();

                    // Sidebar resize drag handling (checked first so edge grabs aren't swallowed)
                    let sidebar_edge_hit = 4;
                    if sidebar_drag {
                        if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                            let new_w = mx.max(SIDEBAR_MIN_WIDTH).min(SIDEBAR_MAX_WIDTH);
                            if new_w != wm.sidebar_width_px {
                                wm.sidebar_width_px = new_w;
                                let sw2 = wm.sidebar_width();
                                let scr_w = rl.get_screen_width();
                                let scr_h = rl.get_screen_height();
                                relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sw2, pad, minimap_width, cell_width, cell_height);
                            }
                        } else {
                            sidebar_drag = false;
                        }
                    } else if wm.sidebar_visible && rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                        let sw = wm.sidebar_width_px;
                        if mx >= sw - sidebar_edge_hit && mx <= sw + sidebar_edge_hit {
                            sidebar_drag = true;
                        }
                    }

                    // Sidebar mouse handling (skip when dragging the sidebar edge)
                    if !sidebar_drag && sidebar_w > 0 && mx < sidebar_w {
                        let wheel = unsafe { raylib::ffi::GetMouseWheelMove() } as i32;
                        if wheel != 0 {
                            let scr_h = rl.get_screen_height();
                            let max_scroll = (wm.workspaces.len() as i32 * ROW_HEIGHT - scr_h + SIDEBAR_BUTTON_HEIGHT).max(0);
                            wm.sidebar_scroll = (wm.sidebar_scroll - wheel * ROW_HEIGHT).max(0).min(max_scroll);
                        }

                        // Dismiss context menu on any click outside it
                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                            if wm.context_menu.is_some() {
                                let dismiss = if let Some((_, cmx, cmy)) = wm.context_menu {
                                    let cm_w = 140;
                                    let cm_h = 64;
                                    !(mx >= cmx && mx < cmx + cm_w && my >= cmy && my < cmy + cm_h)
                                } else { true };
                                if dismiss { wm.context_menu = None; }
                            }
                        }

                        // Context menu click handling
                        if let Some((ctx_idx, cmx, cmy)) = wm.context_menu {
                            let cm_w = 140;
                            let item_h = 28;
                            if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT)
                                && mx >= cmx && mx < cmx + cm_w && my >= cmy && my < cmy + item_h * 2
                            {
                                let item = (my - cmy) / item_h;
                                if item == 0 {
                                    wm.renaming = Some(ctx_idx);
                                    wm.rename_buf = wm.workspaces[ctx_idx].name.clone();
                                    wm.context_menu = None;
                                } else if item == 1 && wm.workspaces.len() > 1 {
                                    wm.remove(ctx_idx);
                                    wm.context_menu = None;
                                    let sidebar_w2 = wm.sidebar_width();
                                    let scr_w2 = rl.get_screen_width();
                                    let scr_h2 = rl.get_screen_height();
                                    relayout_and_resize(&mut wm.active_mut().root, scr_w2, scr_h2, status_bar_height, sidebar_w2, pad, minimap_width, cell_width, cell_height);
                                    last_title.clear();
                                } else {
                                    wm.context_menu = None;
                                }
                            }
                        } else if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                            let scroll = wm.sidebar_scroll;
                            let scr_h = rl.get_screen_height();
                            let plus_btn_y = scr_h - SIDEBAR_BUTTON_HEIGHT;

                            if my >= plus_btn_y {
                                // '+' button clicked — create new workspace
                                let name = wm.next_name();
                                let future_sidebar_w = wm.sidebar_width();
                                let scr_w = rl.get_screen_width();
                                let (root, fid, nid) = create_fresh_panel(&config, scr_w, scr_h, status_bar_height, future_sidebar_w, pad, minimap_width, cell_width, cell_height);
                                wm.add(Workspace {
                                    name, root, focused_panel_id: fid, next_panel_id: nid,
                                    ssh_info: None, ssh_password: String::new(),
                                });
                                let sidebar_w2 = wm.sidebar_width();
                                relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w2, pad, minimap_width, cell_width, cell_height);
                                last_title.clear();
                            } else {
                                let clicked_idx = ((my + scroll) / ROW_HEIGHT) as usize;
                                if clicked_idx < wm.workspaces.len() {
                                    let now = Instant::now();
                                    let is_double = if let Some((prev_idx, prev_time)) = sidebar_last_click {
                                        prev_idx == clicked_idx && now.duration_since(prev_time).as_millis() < 400
                                    } else { false };

                                    if is_double {
                                        wm.renaming = Some(clicked_idx);
                                        wm.rename_buf = wm.workspaces[clicked_idx].name.clone();
                                        sidebar_last_click = None;
                                    } else {
                                        sidebar_last_click = Some((clicked_idx, now));
                                        if clicked_idx != wm.active {
                                            wm.switch_to(clicked_idx);
                                            let sidebar_w2 = wm.sidebar_width();
                                            let scr_w = rl.get_screen_width();
                                            let scr_h2 = rl.get_screen_height();
                                            relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h2, status_bar_height, sidebar_w2, pad, minimap_width, cell_width, cell_height);
                                            last_title.clear();
                                        }
                                    }
                                }
                            }
                        }

                        // Right-click on workspace item -> context menu
                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT) {
                            let scroll = wm.sidebar_scroll;
                            let clicked_idx = ((my + scroll) / ROW_HEIGHT) as usize;
                            if clicked_idx < wm.workspaces.len() {
                                wm.context_menu = Some((clicked_idx, mx, my));
                            }
                        }
                    } else {
                        // Click outside sidebar dismisses context menu and rename
                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) || rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_RIGHT) {
                            wm.context_menu = None;
                            if let Some(idx) = wm.renaming {
                                let trimmed = wm.rename_buf.trim().to_string();
                                if !trimmed.is_empty() && idx < wm.workspaces.len() {
                                    wm.workspaces[idx].name = trimmed;
                                }
                                wm.renaming = None;
                                wm.rename_buf.clear();
                            }
                        }

                    // Separator drag handling (resize splits)
                    if let Some(hit) = separator_drag {
                        if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                            let pos = match hit.direction {
                                split::SplitDirection::Horizontal => mx,
                                split::SplitDirection::Vertical => my,
                            };
                            let available = hit.total - split::SEPARATOR_PX;
                            if available > 0 {
                                let new_ratio = (pos - hit.origin) as f32 / available as f32;
                                wm.active_mut().root.update_ratio_by_ptr(hit.node_ptr, new_ratio);
                                let sidebar_w = wm.sidebar_width();
                                let scr_w = rl.get_screen_width();
                                let scr_h = rl.get_screen_height();
                                relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                            }
                        } else {
                            separator_drag = None;
                        }
                    } else if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                        if let Some(hit) = wm.active().root.separator_at(mx, my) {
                            separator_drag = Some(hit);
                        }
                    }

                    // Set cursor for sidebar edge / separator hover/drag
                    if sidebar_drag || (wm.sidebar_visible && !separator_drag.is_some() && {
                        let sw = wm.sidebar_width_px;
                        mx >= sw - sidebar_edge_hit && mx <= sw + sidebar_edge_hit
                    }) {
                        unsafe { raylib::ffi::SetMouseCursor(raylib::ffi::MouseCursor::MOUSE_CURSOR_RESIZE_EW as i32); }
                    } else if let Some(hit) = separator_drag {
                        match hit.direction {
                            split::SplitDirection::Horizontal => unsafe { raylib::ffi::SetMouseCursor(raylib::ffi::MouseCursor::MOUSE_CURSOR_RESIZE_EW as i32); },
                            split::SplitDirection::Vertical => unsafe { raylib::ffi::SetMouseCursor(raylib::ffi::MouseCursor::MOUSE_CURSOR_RESIZE_NS as i32); },
                        }
                    } else if let Some(hit) = wm.active().root.separator_at(mx, my) {
                        match hit.direction {
                            split::SplitDirection::Horizontal => unsafe { raylib::ffi::SetMouseCursor(raylib::ffi::MouseCursor::MOUSE_CURSOR_RESIZE_EW as i32); },
                            split::SplitDirection::Vertical => unsafe { raylib::ffi::SetMouseCursor(raylib::ffi::MouseCursor::MOUSE_CURSOR_RESIZE_NS as i32); },
                        }
                    } else {
                        unsafe { raylib::ffi::SetMouseCursor(raylib::ffi::MouseCursor::MOUSE_CURSOR_DEFAULT as i32); }
                    }

                    // Check if any panel's minimap is being dragged
                    let mut dragging_panel_id: Option<u32> = None;
                    wm.active().root.for_each_panel(&mut |panel| {
                        let tab = &panel.tabs[panel.active_tab];
                        if tab.minimap.dragging {
                            dragging_panel_id = Some(panel.id);
                        }
                    });

                    let hover_panel_id = if sidebar_drag || separator_drag.is_some() {
                        None
                    } else if let Some(drag_id) = dragging_panel_id {
                        Some(drag_id)
                    } else {
                        wm.active_mut().root.find_panel_at(mx, my).map(|p| p.id)
                    };

                    if let Some(hpid) = hover_panel_id {
                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                            wm.active_mut().focused_panel_id = hpid;
                        }

                        let ws = wm.active_mut();
                        if let Some(panel) = ws.root.panel_by_id_mut(hpid) {
                            let r = panel.rect;
                            let local_mx = mx - r.x;
                            let local_my = my - r.y;
                            let tab_bar_h = panel.tab_bar.height;
                            let tab_count = panel.tabs.len();

                            if local_my < tab_bar_h {
                                // Tab bar area
                                if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                                    match panel.tab_bar.handle_click(local_mx, local_my, r.w, tab_count) {
                                        TabBarAction::SwitchTo(idx) => {
                                            panel.active_tab = idx;
                                            last_title.clear();
                                        }
                                        TabBarAction::Close(idx) => {
                                            panel.tabs.remove(idx);
                                            if panel.tabs.is_empty() {
                                                // handled after this block
                                            } else {
                                                panel.active_tab = panel.active_tab.min(panel.tabs.len() - 1);
                                                last_title.clear();
                                            }
                                        }
                                        TabBarAction::New => {
                                            let (cols, rows) = panel_term_size(&r, pad, minimap_width, tab_bar_h, cell_width, cell_height);
                                            let tab_result = if let Some(ref info) = ws.ssh_info {
                                                ssh_manager.get_or_connect(&info.host, info.port, &info.user, &ws.ssh_password)
                                                    .and_then(|s| ssh_manager.open_channel(&s, cols, rows, info.clone()))
                                                    .and_then(|backend| TabSession::new_ssh(&config, backend, cols, rows, cell_width, cell_height))
                                            } else {
                                                TabSession::new(&config, cols, rows, cell_width, cell_height)
                                            };
                                            if let Ok(t) = tab_result {
                                                panel.tabs.push(t);
                                                panel.active_tab = panel.tabs.len() - 1;
                                                last_title.clear();
                                            }
                                        }
                                        TabBarAction::None => {}
                                    }
                                }
                            } else {
                                let tab = &mut panel.tabs[panel.active_tab];
                                let mut mouse_tracking = false;
                                unsafe {
                                    bindings::ghostty_terminal_get(
                                        tab.term.handle(),
                                        bindings::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
                                        &mut mouse_tracking as *mut bool as *mut std::ffi::c_void,
                                    );
                                }
                                let in_minimap = local_mx >= r.w - tab.minimap.width;
                                let content_y = r.y + tab_bar_h;
                                let content_h = r.h - tab_bar_h;

                                if in_minimap || tab.minimap.dragging {
                                    if let Some((total, offset, len)) = tab.term.get_scrollbar() {
                                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                                            let delta = tab.minimap.handle_mouse_press(my, content_y, content_h, total, offset, len);
                                            if delta != 0 { tab.term.scroll_viewport(delta); }
                                        } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) && tab.minimap.dragging {
                                            let delta = tab.minimap.handle_mouse_drag(my, content_y, content_h, total, offset, len);
                                            if delta != 0 { tab.term.scroll_viewport(delta); }
                                        }
                                    }
                                    if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
                                        tab.minimap.handle_mouse_release();
                                    }
                                } else {
                                    // Terminal area: always call handle_mouse for mouse tracking + scroll wheel
                                    terminal::input::handle_mouse(
                                        &rl, &mut tab.term, &mut tab.backend,
                                        cell_width, cell_height,
                                        pad, pad + tab_bar_h,
                                        pad + minimap_width,
                                        r.w, r.h,
                                        r.x, r.y,
                                    );
                                    if !mouse_tracking {
                                        let (col, row) = selection::mouse_to_cell(local_mx, local_my - tab_bar_h, cell_width, cell_height, pad, pad);
                                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                                            tab.selection.begin(col, row);
                                        } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) && tab.selection.active {
                                            tab.selection.update(col, row);
                                        } else if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) && tab.selection.active {
                                            tab.selection.update(col, row);
                                            tab.selection.finish();
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Close empty panels from tab bar close button
                    let empty_panel = wm.active().root.panel_by_id(wm.active().focused_panel_id)
                        .map(|p| p.tabs.is_empty())
                        .unwrap_or(false);
                    if empty_panel {
                        if wm.active().root.panel_count() <= 1 {
                            if wm.workspaces.len() <= 1 {
                                app_exit = true;
                            } else {
                                let idx = wm.active;
                                wm.remove(idx);
                                let sidebar_w = wm.sidebar_width();
                                let scr_w = rl.get_screen_width();
                                let scr_h = rl.get_screen_height();
                                relayout_and_resize(&mut wm.active_mut().root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                                last_title.clear();
                            }
                        } else {
                            let sidebar_w = wm.sidebar_width();
                            let ws = wm.active_mut();
                            let fid = ws.focused_panel_id;
                            let leaves = ws.root.collect_leaves();
                            let idx = leaves.iter().position(|&id| id == fid).unwrap_or(0);
                            ws.root.close_panel(fid);
                            let new_leaves = ws.root.collect_leaves();
                            ws.focused_panel_id = new_leaves[idx.min(new_leaves.len() - 1)];
                            let scr_w = rl.get_screen_width();
                            let scr_h = rl.get_screen_height();
                            relayout_and_resize(&mut ws.root, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cell_width, cell_height);
                            last_title.clear();
                        }
                    }
                } // end else (not sidebar)
                } // end mouse handling block

                // Keyboard input dispatch -- focused panel only
                let ws = wm.active_mut();
                if let Some(panel) = ws.root.panel_by_id_mut(ws.focused_panel_id) {
                    if !panel.tabs.is_empty() {
                        let tab = &mut panel.tabs[panel.active_tab];
                        let mode = tab.router.mode();

                        if ctrl_held && rl.is_key_pressed(KeyboardKey::KEY_SLASH) {
                            tab.router.toggle_ai_mode();
                        } else if ctrl_held && rl.is_key_pressed(KeyboardKey::KEY_Y) {
                            tab.router.toggle_auto_execute();
                        } else {
                            match mode {
                                InputMode::Shell => {
                                    if !tab.child_exited {
                                        let chars = terminal::input::handle_input(&rl, &mut tab.term, &mut tab.backend);
                                        for c in chars {
                                            tab.router.track_shell_char(c);
                                        }
                                    }
                                }
                                InputMode::AiPrompt => {
                                    let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
                                        || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);
                                    if ctrl && rl.is_key_pressed(KeyboardKey::KEY_J) {
                                        tab.router.handle_ai_prompt_char('\n');
                                    } else {
                                        loop {
                                            let ch = unsafe { raylib::ffi::GetCharPressed() };
                                            if ch == 0 { break; }
                                            if ch == 0x0A { continue; }
                                            if let Some(c) = char::from_u32(ch as u32) {
                                                tab.router.handle_ai_prompt_char(c);
                                            }
                                        }
                                    }
                                    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
                                        || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_BACKSPACE as i32) }
                                    {
                                        tab.router.handle_ai_prompt_backspace();
                                    }
                                    if rl.is_key_pressed(KeyboardKey::KEY_UP)
                                        || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_UP as i32) }
                                    {
                                        tab.router.handle_ai_prompt_history_up();
                                    }
                                    if rl.is_key_pressed(KeyboardKey::KEY_DOWN)
                                        || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_DOWN as i32) }
                                    {
                                        tab.router.handle_ai_prompt_history_down();
                                    }
                                    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) && !ctrl {
                                        tab.router.handle_ai_prompt_submit(&mut tab.term, &mut tab.backend);
                                    }
                                    if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                                        tab.router.handle_ai_prompt_cancel();
                                    }
                                }
                                InputMode::AiStreaming => {
                                    if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                                        // TODO: cancel
                                    }
                                }
                                InputMode::CommandConfirm => {
                                    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                                        tab.router.handle_command_confirm_enter(&mut tab.term, &mut tab.backend, &mut tab.overlay);
                                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                                        tab.router.handle_command_confirm_cancel(&mut tab.backend, &mut tab.overlay);
                                    } else if rl.is_key_pressed(KeyboardKey::KEY_E) {
                                        tab.router.handle_command_confirm_edit(&mut tab.backend, &mut tab.overlay);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if app_exit {
            break;
        }

        // Collect focused panel info for status bar and AI prompt
        let focused_mode;
        let focused_ai_input;
        let focused_auto_exec;
        let focused_cwd;
        let focused_panel_rect;
        let panel_count = wm.active().root.panel_count();

        let panel_info;
        {
            let ws = wm.active();
            let leaves = ws.root.collect_leaves();
            let panel_idx = leaves.iter().position(|&id| id == ws.focused_panel_id).unwrap_or(0) + 1;
            if let Some(fp) = ws.root.panel_by_id(ws.focused_panel_id) {
                let tab = fp.active_tab();
                focused_mode = tab.router.mode();
                focused_ai_input = tab.router.ai_input_buffer().to_string();
                focused_auto_exec = tab.router.auto_execute();
                focused_cwd = if let Some(ref info) = tab.ssh_info {
                    let remote_path = ssh_cwd_display(&tab.backend, &tab.term.last_osc_title);
                    format!("ssh://{}@{}:{} {}", info.user, info.host, info.port, remote_path)
                } else if let Some(ref info) = ws.ssh_info {
                    let remote_path = ssh_cwd_display(&tab.backend, &tab.term.last_osc_title);
                    format!("ssh://{}@{}:{} {}", info.user, info.host, info.port, remote_path)
                } else {
                    tab.backend.get_cwd()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "~".to_string())
                };
                focused_panel_rect = fp.rect;
                panel_info = Some((panel_idx, panel_count, fp.active_tab + 1, fp.tabs.len()));
            } else {
                focused_mode = InputMode::Shell;
                focused_ai_input = String::new();
                focused_auto_exec = false;
                focused_cwd = "~".to_string();
                focused_panel_rect = PanelRect { x: 0, y: 0, w: 0, h: 0 };
                panel_info = None;
            }
        }

        // --- RENDERING ---
        let mut d = rl.begin_drawing(&thread);

        // Clear with dark background first
        d.clear_background(Color::new(26, 27, 30, 255));

        // Render all panels (active workspace only)
        let ws_focused_panel_id = wm.active().focused_panel_id;
        wm.active_mut().root.for_each_panel_mut(&mut |panel| {
            let r = panel.rect;
            let is_focused = panel.id == ws_focused_panel_id;

            let tab_titles: Vec<String> = panel.tabs.iter().map(|t| t.title()).collect();
            let active_idx = panel.active_tab;

            if panel.tabs.is_empty() {
                return;
            }

            let tab = &mut panel.tabs[active_idx];
            tab.term.update_render_state();

            let bg_color = Color::new(26, 27, 30, 255);

            let tab_bar_h = panel.tab_bar.height;

            // Scissor clip to panel bounds
            unsafe { raylib::ffi::BeginScissorMode(r.x, r.y, r.w, r.h); }

            d.draw_rectangle(r.x, r.y, r.w, r.h, bg_color);

            // Tab bar
            panel.tab_bar.render(&tab_titles, active_idx, &mono_font, font_size, r.x, r.y, r.w, &mut d);

            // Terminal grid
            let grid_pad_x = r.x + pad;
            let grid_pad_y = r.y + tab_bar_h + pad;
            terminal::renderer::render_terminal(
                tab.term.render_state(),
                tab.term.row_iter(),
                tab.term.row_cells(),
                &mono_font,
                cell_width,
                cell_height,
                font_size,
                grid_pad_x,
                grid_pad_y,
                tab.term.handle(),
                &mut d,
            );

            // Minimap (content area below tab bar)
            if let Some((total, offset, len)) = tab.term.get_scrollbar() {
                let content_y = r.y + tab_bar_h;
                let content_h = r.h - tab_bar_h;
                tab.minimap.render(total, offset, len, r.x, content_y, r.w, content_h, &mut d);
            }

            // Selection highlight
            if tab.selection.has_selection() {
                let term_cols = (r.w - 2 * pad - minimap_width) / cell_width;
                let term_rows = ((r.h - tab_bar_h) - 2 * pad) / cell_height;
                tab.selection.render(cell_width, cell_height, grid_pad_x, grid_pad_y, term_cols, term_rows, &mut d);
            }

            // Overlay (focused only)
            if is_focused {
                tab.overlay.render(&mono_font, r.x, r.y, r.w, r.h, font_size, &mut d);
            }

            // [process exited] banner
            if tab.child_exited {
                let msg = "[process exited]";
                let msg_c = std::ffi::CString::new(msg).unwrap();
                let banner_h = font_size + 8;
                d.draw_rectangle(r.x, r.y + r.h - banner_h, r.w, banner_h, Color::new(0, 0, 0, 180));
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font,
                        msg_c.as_ptr(),
                        raylib::ffi::Vector2 {
                            x: (r.x + 10) as f32,
                            y: (r.y + r.h - banner_h + 4) as f32,
                        },
                        font_size as f32,
                        0.0,
                        raylib::ffi::Color { r: 255, g: 255, b: 255, a: 255 },
                    );
                }
            }

            unsafe { raylib::ffi::EndScissorMode(); }

            // Focus border (outside scissor)
            if is_focused && panel_count > 1 {
                let focus_color = Color::new(58, 62, 78, 255);
                d.draw_rectangle(r.x, r.y, r.w, 2, focus_color);
                d.draw_rectangle(r.x, r.y + r.h - 2, r.w, 2, focus_color);
                d.draw_rectangle(r.x, r.y, 2, r.h, focus_color);
                d.draw_rectangle(r.x + r.w - 2, r.y, 2, r.h, focus_color);
            }
        });

        // Separator lines between panels
        wm.active().root.draw_separators(&mut d);

        // Floating AI prompt panel (focused panel only)
        if focused_mode == InputMode::AiPrompt {
            let pr = focused_panel_rect;
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();

            let ai_panel_w = (pr.w * 3 / 4).min(700).max(300);
            let ai_panel_x = pr.x + (pr.w - ai_panel_w) / 2;
            let inner_pad = 12;
            let label_h = cell_height + 4;

            let text_area_w = ai_panel_w - inner_pad * 2;
            let chars_per_line = (text_area_w / cell_width).max(1) as usize;

            let mut wrapped_lines: Vec<String> = Vec::new();
            for logical_line in focused_ai_input.split('\n') {
                if logical_line.is_empty() {
                    wrapped_lines.push(String::new());
                } else {
                    let chars: Vec<char> = logical_line.chars().collect();
                    for chunk in chars.chunks(chars_per_line) {
                        wrapped_lines.push(chunk.iter().collect());
                    }
                }
            }

            let line_count = wrapped_lines.len().max(1) as i32;
            let text_h = line_count * cell_height;
            let ai_panel_h = label_h + text_h + inner_pad * 2 + 4;
            let ai_panel_y = pr.y + pr.h - ai_panel_h - 8;

            d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 100));

            d.draw_rectangle(ai_panel_x, ai_panel_y, ai_panel_w, ai_panel_h, Color::new(30, 30, 38, 245));
            d.draw_rectangle(ai_panel_x, ai_panel_y, ai_panel_w, 1, Color::new(70, 80, 110, 200));
            d.draw_rectangle(ai_panel_x, ai_panel_y + ai_panel_h - 1, ai_panel_w, 1, Color::new(70, 80, 110, 200));
            d.draw_rectangle(ai_panel_x, ai_panel_y, 1, ai_panel_h, Color::new(70, 80, 110, 200));
            d.draw_rectangle(ai_panel_x + ai_panel_w - 1, ai_panel_y, 1, ai_panel_h, Color::new(70, 80, 110, 200));

            let label = std::ffi::CString::new("Ask AI").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font,
                    label.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (ai_panel_x + inner_pad) as f32,
                        y: (ai_panel_y + inner_pad / 2) as f32,
                    },
                    (font_size - 2) as f32,
                    0.0,
                    raylib::ffi::Color { r: 100, g: 110, b: 140, a: 200 },
                );
            }

            d.draw_rectangle(
                ai_panel_x + inner_pad,
                ai_panel_y + label_h,
                text_area_w,
                1,
                Color::new(55, 60, 75, 180),
            );

            let text_y = ai_panel_y + label_h + 6;
            let text_x = ai_panel_x + inner_pad;

            let blink_on = (d.get_time() * 2.0) as i32 % 2 == 0;
            for (i, wline) in wrapped_lines.iter().enumerate() {
                let cursor_str = if i == wrapped_lines.len() - 1 && blink_on { "|" } else { "" };
                let display = format!("{}{}", wline, cursor_str);
                let c_text = std::ffi::CString::new(display.as_str()).unwrap_or_default();
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font,
                        c_text.as_ptr(),
                        raylib::ffi::Vector2 {
                            x: text_x as f32,
                            y: (text_y + i as i32 * cell_height) as f32,
                        },
                        font_size as f32,
                        0.0,
                        raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 },
                    );
                }
            }
        }

        // Help panel overlay (two-column layout)
        if show_help {
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();

            d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 160));

            let content_x = wm.sidebar_width();
            let content_w = screen_w - content_x;
            let line_h = cell_height + 2;
            let inner_pad = 20;
            let section_gap = 10;
            let label_size = (font_size - 2).max(8);
            let key_col_w = 170;

            // Left column: General, Workspaces, Tabs, Splits
            // Right column: Font, AI Prompt, Command Confirm, Session
            let left_sections: &[(&str, &[(&str, &str)])] = &[
                ("General", &[
                    ("F1", "Toggle this help panel"),
                    ("F2", "Session manager"),
                    ("Cmd+Shift+S", "New SSH workspace"),
                    ("Ctrl+/", "Toggle AI prompt"),
                    ("Ctrl+Y", "Toggle YOLO (auto-execute)"),
                    ("Cmd+C", "Copy selection"),
                    ("Cmd+V", "Paste from clipboard"),
                ]),
                ("Workspaces", &[
                    ("Cmd+N", "New local workspace"),
                    ("Ctrl+Cmd+]  /  [", "Next / prev workspace"),
                    ("Ctrl+Cmd+1..9", "Jump to workspace N"),
                    ("Cmd+\\", "Toggle sidebar"),
                    ("Double-click", "Rename workspace"),
                    ("Right-click", "Context menu"),
                ]),
                ("Tabs", &[
                    ("Cmd+T", "New tab"),
                    ("Cmd+W", "Close tab"),
                    ("Cmd+Shift+]  /  [", "Next / prev tab"),
                    ("Cmd+1..9", "Jump to tab N"),
                ]),
            ];

            let right_sections: &[(&str, &[(&str, &str)])] = &[
                ("Splits", &[
                    ("Cmd+D", "Split horizontal"),
                    ("Cmd+Shift+D", "Split vertical"),
                    ("Cmd+Shift+W", "Close panel/workspace"),
                    ("Cmd+Opt+Arrow", "Focus next/prev panel"),
                ]),
                ("Font", &[
                    ("Cmd++  /  -", "Increase / decrease"),
                    ("Cmd+0", "Reset font size"),
                ]),
                ("AI Prompt", &[
                    ("Enter", "Submit prompt"),
                    ("Ctrl+J", "Newline in prompt"),
                    ("Up/Down", "History navigation"),
                    ("Esc", "Cancel prompt"),
                ]),
                ("Command Confirm", &[
                    ("Enter", "Run command"),
                    ("e", "Edit command"),
                    ("Esc", "Cancel"),
                ]),
                ("Session (CLI flags)", &[
                    ("--export <name>", "Export session"),
                    ("--import <name>", "Import session"),
                    ("--reset", "Reset session"),
                    ("--list-sessions", "List saved sessions"),
                ]),
            ];

            let col_height = |sections: &[(&str, &[(&str, &str)])]| -> i32 {
                let mut h = 0i32;
                for (si, (_, entries)) in sections.iter().enumerate() {
                    if si > 0 { h += section_gap; }
                    h += line_h; // section header
                    h += entries.len() as i32 * line_h;
                }
                h
            };

            let left_h = col_height(left_sections);
            let right_h = col_height(right_sections);
            let body_content_h = left_h.max(right_h) + inner_pad;

            let title_h = cell_height + 12;
            let footer_h = cell_height + 8;
            let col_w = 420;
            let divider_gap = 24;
            let help_w = (inner_pad + col_w + divider_gap + col_w + inner_pad).min(content_w - 40);
            let help_h = (title_h + body_content_h + footer_h).min(screen_h - 40);
            let help_x = content_x + (content_w - help_w) / 2;
            let help_y = (screen_h - help_h) / 2;
            let body_h = help_h - title_h - footer_h;

            let max_help_scroll = (body_content_h - body_h).max(0);
            help_scroll = help_scroll.min(max_help_scroll);

            // Background + border
            d.draw_rectangle(help_x, help_y, help_w, help_h, Color::new(22, 22, 30, 250));
            d.draw_rectangle_lines(help_x, help_y, help_w, help_h, Color::new(60, 70, 100, 200));

            // Title
            let title = std::ffi::CString::new("Keyboard Shortcuts").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, title.as_ptr(),
                    raylib::ffi::Vector2 { x: (help_x + inner_pad) as f32, y: (help_y + (title_h - font_size) / 2) as f32 },
                    font_size as f32, 0.0,
                    raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 },
                );
            }
            d.draw_rectangle(help_x + inner_pad, help_y + title_h - 1, help_w - inner_pad * 2, 1, Color::new(55, 60, 75, 180));

            let body_y = help_y + title_h;

            unsafe { raylib::ffi::BeginScissorMode(help_x, body_y, help_w, body_h); }

            let draw_column = |sections: &[(&str, &[(&str, &str)])], col_x: i32, start_y: i32, scroll: i32| {
                let mut cy = start_y - scroll;
                for (si, (section_name, entries)) in sections.iter().enumerate() {
                    if si > 0 { cy += section_gap; }
                    let section_c = std::ffi::CString::new(*section_name).unwrap_or_default();
                    unsafe {
                        raylib::ffi::DrawTextEx(
                            mono_font, section_c.as_ptr(),
                            raylib::ffi::Vector2 { x: col_x as f32, y: cy as f32 },
                            label_size as f32, 0.0,
                            raylib::ffi::Color { r: 80, g: 140, b: 220, a: 255 },
                        );
                    }
                    cy += line_h;

                    for (key, desc) in *entries {
                        let key_c = std::ffi::CString::new(*key).unwrap_or_default();
                        let desc_c = std::ffi::CString::new(*desc).unwrap_or_default();
                        unsafe {
                            raylib::ffi::DrawTextEx(
                                mono_font, key_c.as_ptr(),
                                raylib::ffi::Vector2 { x: (col_x + 8) as f32, y: cy as f32 },
                                label_size as f32, 0.0,
                                raylib::ffi::Color { r: 200, g: 200, b: 140, a: 255 },
                            );
                            raylib::ffi::DrawTextEx(
                                mono_font, desc_c.as_ptr(),
                                raylib::ffi::Vector2 { x: (col_x + key_col_w) as f32, y: cy as f32 },
                                label_size as f32, 0.0,
                                raylib::ffi::Color { r: 180, g: 180, b: 190, a: 255 },
                            );
                        }
                        cy += line_h;
                    }
                }
            };

            let left_x = help_x + inner_pad;
            let right_x = help_x + inner_pad + col_w + divider_gap;
            let col_start_y = body_y + inner_pad / 2;

            draw_column(left_sections, left_x, col_start_y, help_scroll);
            draw_column(right_sections, right_x, col_start_y, help_scroll);

            // Vertical divider between columns
            let div_x = help_x + inner_pad + col_w + divider_gap / 2;
            d.draw_rectangle(div_x, body_y + 4 - help_scroll, 1, body_content_h - 8, Color::new(45, 50, 65, 150));

            unsafe { raylib::ffi::EndScissorMode(); }

            // Footer
            let footer_y = help_y + help_h - footer_h;
            d.draw_rectangle(help_x, footer_y, help_w, footer_h, Color::new(22, 22, 30, 250));
            d.draw_rectangle(help_x + inner_pad, footer_y, help_w - inner_pad * 2, 1, Color::new(55, 60, 75, 180));
            let hint_text = if max_help_scroll > 0 { "F1 or Esc to close  \u{2022}  Scroll for more" } else { "F1 or Esc to close" };
            let hint = std::ffi::CString::new(hint_text).unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, hint.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (help_x + inner_pad) as f32,
                        y: (footer_y + (footer_h - label_size) / 2) as f32,
                    },
                    label_size as f32, 0.0,
                    raylib::ffi::Color { r: 100, g: 100, b: 110, a: 180 },
                );
            }
        }

        // Session manager overlay
        if show_session_mgr {
            const SM_MAX_VISIBLE: usize = 15;

            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();

            d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 160));

            let inner_pad = 16;
            let line_h = cell_height + 4;
            let label_size = (font_size - 2).max(8);
            let title_h = cell_height + 8;
            let visible_count = sm_names.len().min(SM_MAX_VISIBLE).max(1);
            let list_h = visible_count as i32 * line_h;
            let footer_h = line_h + 4;
            let status_h = if sm_status.is_some() { line_h } else { 0 };
            let input_h = if sm_input.is_some() { line_h + 4 } else { 0 };

            let content_x = wm.sidebar_width();
            let content_w = screen_w - content_x;
            let sm_w = 480.min(content_w - 40);
            let sm_h = title_h + inner_pad * 2 + list_h + footer_h + status_h + input_h;
            let sm_h = sm_h.min(screen_h - 40);
            let sm_x = content_x + (content_w - sm_w) / 2;
            let sm_y = (screen_h - sm_h) / 2;

            d.draw_rectangle(sm_x, sm_y, sm_w, sm_h, Color::new(25, 25, 32, 250));
            d.draw_rectangle(sm_x, sm_y, sm_w, 1, Color::new(70, 80, 110, 220));
            d.draw_rectangle(sm_x, sm_y + sm_h - 1, sm_w, 1, Color::new(70, 80, 110, 220));
            d.draw_rectangle(sm_x, sm_y, 1, sm_h, Color::new(70, 80, 110, 220));
            d.draw_rectangle(sm_x + sm_w - 1, sm_y, 1, sm_h, Color::new(70, 80, 110, 220));

            let title_text = std::ffi::CString::new("Session Manager").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, title_text.as_ptr(),
                    raylib::ffi::Vector2 { x: (sm_x + inner_pad) as f32, y: (sm_y + inner_pad / 2 + 2) as f32 },
                    font_size as f32, 0.0,
                    raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 },
                );
            }
            d.draw_rectangle(sm_x + inner_pad, sm_y + title_h, sm_w - inner_pad * 2, 1, Color::new(55, 60, 75, 180));

            let list_y = sm_y + title_h + inner_pad;

            if sm_names.is_empty() {
                let empty_msg = std::ffi::CString::new("No saved sessions").unwrap_or_default();
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, empty_msg.as_ptr(),
                        raylib::ffi::Vector2 { x: (sm_x + inner_pad + 10) as f32, y: list_y as f32 },
                        label_size as f32, 0.0,
                        raylib::ffi::Color { r: 120, g: 120, b: 130, a: 180 },
                    );
                }
            } else {
                let end = (sm_scroll + SM_MAX_VISIBLE).min(sm_names.len());
                for (i, name) in sm_names[sm_scroll..end].iter().enumerate() {
                    let abs_idx = sm_scroll + i;
                    let iy = list_y + i as i32 * line_h;

                    if abs_idx == sm_selected {
                        d.draw_rectangle(sm_x + inner_pad - 2, iy - 1, sm_w - inner_pad * 2 + 4, line_h, Color::new(50, 60, 90, 200));
                    }

                    let name_c = std::ffi::CString::new(name.as_str()).unwrap_or_default();
                    let text_color = if abs_idx == sm_selected {
                        raylib::ffi::Color { r: 255, g: 255, b: 255, a: 255 }
                    } else {
                        raylib::ffi::Color { r: 180, g: 180, b: 190, a: 255 }
                    };
                    unsafe {
                        raylib::ffi::DrawTextEx(
                            mono_font, name_c.as_ptr(),
                            raylib::ffi::Vector2 { x: (sm_x + inner_pad + 10) as f32, y: iy as f32 },
                            label_size as f32, 0.0,
                            text_color,
                        );
                    }
                }

                if sm_scroll > 0 {
                    let arrow = std::ffi::CString::new("▲").unwrap_or_default();
                    unsafe {
                        raylib::ffi::DrawTextEx(
                            mono_font, arrow.as_ptr(),
                            raylib::ffi::Vector2 { x: (sm_x + sm_w - inner_pad - 16) as f32, y: list_y as f32 },
                            label_size as f32, 0.0,
                            raylib::ffi::Color { r: 150, g: 150, b: 160, a: 200 },
                        );
                    }
                }
                if end < sm_names.len() {
                    let arrow = std::ffi::CString::new("▼").unwrap_or_default();
                    unsafe {
                        raylib::ffi::DrawTextEx(
                            mono_font, arrow.as_ptr(),
                            raylib::ffi::Vector2 {
                                x: (sm_x + sm_w - inner_pad - 16) as f32,
                                y: (list_y + (visible_count as i32 - 1) * line_h) as f32,
                            },
                            label_size as f32, 0.0,
                            raylib::ffi::Color { r: 150, g: 150, b: 160, a: 200 },
                        );
                    }
                }
            }

            let mut bottom_y = list_y + list_h;

            if let Some((ref msg, _)) = sm_status {
                let msg_c = std::ffi::CString::new(msg.as_str()).unwrap_or_default();
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, msg_c.as_ptr(),
                        raylib::ffi::Vector2 { x: (sm_x + inner_pad + 10) as f32, y: bottom_y as f32 },
                        label_size as f32, 0.0,
                        raylib::ffi::Color { r: 240, g: 220, b: 100, a: 255 },
                    );
                }
                bottom_y += line_h;
            }

            if let Some(ref input) = sm_input {
                let blink_on = (d.get_time() * 2.0) as i32 % 2 == 0;
                let cursor = if blink_on { "_" } else { " " };
                let display = format!("Save as: {}{}", input, cursor);
                let input_c = std::ffi::CString::new(display.as_str()).unwrap_or_default();
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, input_c.as_ptr(),
                        raylib::ffi::Vector2 { x: (sm_x + inner_pad + 10) as f32, y: (bottom_y + 2) as f32 },
                        label_size as f32, 0.0,
                        raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 },
                    );
                }
                bottom_y += line_h + 4;
            }

            d.draw_rectangle(sm_x + inner_pad, bottom_y, sm_w - inner_pad * 2, 1, Color::new(55, 60, 75, 180));

            let footer_text = if sm_input.is_some() {
                "[Enter] Confirm  [Esc] Cancel"
            } else {
                "[s] Save  [Enter] Load  [d] Delete  [Esc] Close"
            };
            let footer_c = std::ffi::CString::new(footer_text).unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, footer_c.as_ptr(),
                    raylib::ffi::Vector2 { x: (sm_x + inner_pad) as f32, y: (bottom_y + 4) as f32 },
                    label_size as f32, 0.0,
                    raylib::ffi::Color { r: 100, g: 100, b: 110, a: 180 },
                );
            }
        }

        // SSH connect overlay
        if show_ssh_connect {
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();
            d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 160));

            let inner_pad = 16;
            let line_h = cell_height + 6;
            let label_size = (font_size - 2).max(8);
            let title_h = cell_height + 8;
            let fields_h = line_h * 4 + inner_pad;
            let error_h = if ssh_error.is_some() { line_h + 4 } else { 0 };
            let footer_h = line_h;

            let content_x = wm.sidebar_width();
            let content_w = screen_w - content_x;
            let box_w = 420.min(content_w - 40);
            let box_h = title_h + inner_pad * 2 + fields_h + error_h + footer_h;
            let box_x = content_x + (content_w - box_w) / 2;
            let box_y = (screen_h - box_h) / 2;

            d.draw_rectangle(box_x, box_y, box_w, box_h, Color::new(25, 25, 32, 250));
            d.draw_rectangle(box_x, box_y, box_w, 1, Color::new(70, 80, 110, 220));
            d.draw_rectangle(box_x, box_y + box_h - 1, box_w, 1, Color::new(70, 80, 110, 220));
            d.draw_rectangle(box_x, box_y, 1, box_h, Color::new(70, 80, 110, 220));
            d.draw_rectangle(box_x + box_w - 1, box_y, 1, box_h, Color::new(70, 80, 110, 220));

            let title_text = std::ffi::CString::new("SSH Connect").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, title_text.as_ptr(),
                    raylib::ffi::Vector2 { x: (box_x + inner_pad) as f32, y: (box_y + inner_pad / 2 + 2) as f32 },
                    font_size as f32, 0.0,
                    raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 },
                );
            }
            d.draw_rectangle(box_x + inner_pad, box_y + title_h, box_w - inner_pad * 2, 1, Color::new(55, 60, 75, 180));

            let fields_y = box_y + title_h + inner_pad;
            let labels = ["Host:", "Port:", "User:", "Pass:"];
            let values = [&ssh_host, &ssh_port, &ssh_user, &ssh_password];

            for (i, (label, value)) in labels.iter().zip(values.iter()).enumerate() {
                let fy = fields_y + i as i32 * line_h;
                let is_focused = i == ssh_focus;

                if is_focused {
                    d.draw_rectangle(box_x + inner_pad - 2, fy - 1, box_w - inner_pad * 2 + 4, line_h, Color::new(40, 45, 70, 200));
                }

                let label_c = std::ffi::CString::new(*label).unwrap_or_default();
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, label_c.as_ptr(),
                        raylib::ffi::Vector2 { x: (box_x + inner_pad + 4) as f32, y: fy as f32 },
                        label_size as f32, 0.0,
                        raylib::ffi::Color { r: 140, g: 140, b: 150, a: 255 },
                    );
                }

                let display_val = if i == 3 {
                    "\u{2022}".repeat(value.len())
                } else {
                    (*value).clone()
                };
                let cursor = if is_focused { "_" } else { "" };
                let val_text = format!("{}{}", display_val, cursor);
                let val_c = std::ffi::CString::new(val_text).unwrap_or_default();
                let val_color = if is_focused {
                    raylib::ffi::Color { r: 255, g: 255, b: 255, a: 255 }
                } else {
                    raylib::ffi::Color { r: 180, g: 180, b: 190, a: 255 }
                };
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, val_c.as_ptr(),
                        raylib::ffi::Vector2 { x: (box_x + inner_pad + 60) as f32, y: fy as f32 },
                        label_size as f32, 0.0,
                        val_color,
                    );
                }
            }

            if let Some(ref err) = ssh_error {
                let ey = fields_y + 4 * line_h + 4;
                let err_c = std::ffi::CString::new(err.as_str()).unwrap_or_default();
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, err_c.as_ptr(),
                        raylib::ffi::Vector2 { x: (box_x + inner_pad + 4) as f32, y: ey as f32 },
                        label_size as f32, 0.0,
                        raylib::ffi::Color { r: 255, g: 80, b: 80, a: 255 },
                    );
                }
            }

            let footer_y = box_y + box_h - footer_h - inner_pad / 2;
            let footer_c = std::ffi::CString::new("Enter: Connect  Tab: Next field  Esc: Cancel").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, footer_c.as_ptr(),
                    raylib::ffi::Vector2 { x: (box_x + inner_pad) as f32, y: footer_y as f32 },
                    label_size as f32, 0.0,
                    raylib::ffi::Color { r: 100, g: 100, b: 110, a: 180 },
                );
            }
        }

        // Status bar (global, at bottom)
        status_bar.render(
            &mono_font,
            d.get_screen_width(),
            d.get_screen_height(),
            font_size,
            wm.sidebar_width(),
            focused_mode,
            &focused_cwd,
            &focused_ai_input,
            focused_auto_exec,
            panel_info,
            &mut d,
        );

        // Sidebar (draws on top of left edge)
        let sidebar_w = wm.sidebar_width();
        if sidebar_w > 0 {
            let scr_h = d.get_screen_height();
            let name_size = (font_size - 2).max(8);
            let info_size = (font_size - 4).max(7);
            let plus_btn_y = scr_h - SIDEBAR_BUTTON_HEIGHT;
            let list_area_h = plus_btn_y;

            let max_scroll = (wm.workspaces.len() as i32 * ROW_HEIGHT - list_area_h).max(0);
            let scroll = wm.sidebar_scroll.min(max_scroll).max(0);

            d.draw_rectangle(0, 0, sidebar_w, scr_h, Color::new(15, 15, 20, 255));

            let mx = d.get_mouse_x();
            let my = d.get_mouse_y();
            let mouse_in_sidebar = mx < sidebar_w;

            unsafe { raylib::ffi::BeginScissorMode(0, 0, sidebar_w, list_area_h); }

            for (i, ws) in wm.workspaces.iter().enumerate() {
                let row_y = i as i32 * ROW_HEIGHT - scroll;
                if row_y + ROW_HEIGHT < 0 || row_y > list_area_h { continue; }

                let is_active = i == wm.active;
                let is_hover = mouse_in_sidebar && my >= row_y && my < row_y + ROW_HEIGHT && my < plus_btn_y;

                if is_active {
                    d.draw_rectangle(0, row_y, sidebar_w - 1, ROW_HEIGHT, Color::new(40, 45, 65, 220));
                } else if is_hover {
                    d.draw_rectangle(0, row_y, sidebar_w - 1, ROW_HEIGHT, Color::new(30, 35, 50, 180));
                }

                let is_renaming = wm.renaming == Some(i);
                let content_h = name_size + 4 + info_size;
                let top_pad = (ROW_HEIGHT - content_h) / 2;
                let name_y = row_y + top_pad;
                let info_y = name_y + name_size + 4;

                if is_renaming {
                    d.draw_rectangle(6, name_y - 1, sidebar_w - 14, name_size + 4, Color::new(30, 35, 55, 255));
                    d.draw_rectangle_lines(6, name_y - 1, sidebar_w - 14, name_size + 4, Color::new(80, 130, 220, 200));

                    let cursor_text = format!("{}_", &wm.rename_buf);
                    let rename_c = std::ffi::CString::new(cursor_text.as_str()).unwrap_or_default();
                    unsafe {
                        raylib::ffi::DrawTextEx(
                            mono_font, rename_c.as_ptr(),
                            raylib::ffi::Vector2 { x: 10.0, y: name_y as f32 },
                            name_size as f32, 0.0,
                            raylib::ffi::Color { r: 255, g: 255, b: 255, a: 255 },
                        );
                    }
                } else {
                    let name_truncated: String = ws.name.chars().take(22).collect();
                    let name_c = std::ffi::CString::new(name_truncated.as_str()).unwrap_or_default();
                    unsafe {
                        raylib::ffi::DrawTextEx(
                            mono_font, name_c.as_ptr(),
                            raylib::ffi::Vector2 { x: 10.0, y: name_y as f32 },
                            name_size as f32, 0.0,
                            if is_active {
                                raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 }
                            } else {
                                raylib::ffi::Color { r: 150, g: 155, b: 170, a: 255 }
                            },
                        );
                    }
                }

                let panels = ws.panel_count();
                let tabs = ws.total_tab_count();
                let info_text = if ws.ssh_info.is_some() {
                    format!("SSH \u{00b7} {} panel{} \u{00b7} {} tab{}", panels, if panels != 1 {"s"} else {""}, tabs, if tabs != 1 {"s"} else {""})
                } else {
                    format!("{} panel{} \u{00b7} {} tab{}", panels, if panels != 1 {"s"} else {""}, tabs, if tabs != 1 {"s"} else {""})
                };
                let info_c = std::ffi::CString::new(info_text.as_str()).unwrap_or_default();
                let info_color = if ws.ssh_info.is_some() {
                    raylib::ffi::Color { r: 70, g: 145, b: 200, a: 200 }
                } else {
                    raylib::ffi::Color { r: 100, g: 105, b: 120, a: 200 }
                };
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, info_c.as_ptr(),
                        raylib::ffi::Vector2 { x: 10.0, y: info_y as f32 },
                        info_size as f32, 0.0,
                        info_color,
                    );
                }

                d.draw_rectangle(4, row_y + ROW_HEIGHT - 1, sidebar_w - 9, 1, Color::new(35, 40, 55, 150));
            }

            unsafe { raylib::ffi::EndScissorMode(); }

            // '+' button at bottom
            let plus_hover = mouse_in_sidebar && my >= plus_btn_y;
            d.draw_rectangle(0, plus_btn_y, sidebar_w - 1, SIDEBAR_BUTTON_HEIGHT, Color::new(20, 22, 30, 255));
            d.draw_rectangle(0, plus_btn_y, sidebar_w - 1, 1, Color::new(45, 50, 65, 180));
            if plus_hover {
                d.draw_rectangle(4, plus_btn_y + 4, sidebar_w - 9, SIDEBAR_BUTTON_HEIGHT - 8, Color::new(40, 50, 70, 200));
            }
            let plus_c = std::ffi::CString::new("+  New workspace").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, plus_c.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: 10.0,
                        y: (plus_btn_y + (SIDEBAR_BUTTON_HEIGHT - name_size) / 2) as f32,
                    },
                    name_size as f32, 0.0,
                    if plus_hover {
                        raylib::ffi::Color { r: 200, g: 210, b: 230, a: 255 }
                    } else {
                        raylib::ffi::Color { r: 120, g: 130, b: 150, a: 220 }
                    },
                );
            }

            d.draw_rectangle(sidebar_w - 1, 0, 1, scr_h, Color::new(55, 60, 75, 180));

            // Context menu
            if let Some((ctx_idx, cmx, cmy)) = wm.context_menu {
                if ctx_idx < wm.workspaces.len() {
                    let cm_w = 140;
                    let item_h = 28;
                    let cm_h = item_h * 2;
                    let cm_y = cmy.min(scr_h - cm_h);
                    let cm_x = cmx.min(sidebar_w - cm_w);

                    d.draw_rectangle(cm_x, cm_y, cm_w, cm_h, Color::new(30, 32, 42, 245));
                    d.draw_rectangle_lines(cm_x, cm_y, cm_w, cm_h, Color::new(60, 65, 80, 200));

                    let items: [(&str, bool); 2] = [
                        ("Rename", true),
                        ("Close", wm.workspaces.len() > 1),
                    ];

                    for (j, (label, enabled)) in items.iter().enumerate() {
                        let iy = cm_y + j as i32 * item_h;
                        let item_hover = mx >= cm_x && mx < cm_x + cm_w && my >= iy && my < iy + item_h;
                        if item_hover && *enabled {
                            d.draw_rectangle(cm_x + 2, iy + 2, cm_w - 4, item_h - 4, Color::new(50, 55, 75, 200));
                        }
                        let label_c = std::ffi::CString::new(*label).unwrap_or_default();
                        let color = if *enabled {
                            if item_hover { raylib::ffi::Color { r: 230, g: 235, b: 245, a: 255 } }
                            else { raylib::ffi::Color { r: 180, g: 185, b: 200, a: 255 } }
                        } else {
                            raylib::ffi::Color { r: 80, g: 85, b: 100, a: 150 }
                        };
                        unsafe {
                            raylib::ffi::DrawTextEx(
                                mono_font, label_c.as_ptr(),
                                raylib::ffi::Vector2 { x: (cm_x + 12) as f32, y: (iy + (item_h - info_size) / 2) as f32 },
                                info_size as f32, 0.0,
                                color,
                            );
                        }
                    }
                }
            }
        }
    }

    // Save session on exit (all vars still alive, Drop not yet fired)
    let win_pos = unsafe { raylib::ffi::GetWindowPosition() };
    if let Err(e) = session::save(
        &wm, font_size,
        rl.get_screen_width(), rl.get_screen_height(),
        win_pos.x as i32, win_pos.y as i32,
    ) {
        eprintln!("[TAI] Failed to save session on exit: {e}");
    }
}
