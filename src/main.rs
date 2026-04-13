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

use config::TaiConfig;
use router::InputMode;
use selection::TextSelection;
use split::{SplitDirection, SplitNode, PanelRect, alloc_panel_id, create_panel, panel_term_size};
use status_bar::StatusBar;
use tab::TabSession;
use tab_bar::TabBarAction;
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
    pad: i32,
    minimap_width: i32,
    cell_width: i32,
    cell_height: i32,
) {
    root.layout(PanelRect { x: 0, y: 0, w: scr_w, h: scr_h - status_bar_height });
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
    pad: i32,
    minimap_width: i32,
    cell_width: i32,
    cell_height: i32,
) -> (SplitNode, u32, u32) {
    let mut next_panel_id: u32 = 0;
    let initial_id = alloc_panel_id(&mut next_panel_id);
    let initial_rect = PanelRect { x: 0, y: 0, w: scr_w, h: scr_h - status_bar_height };
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

fn main() {
    unsafe {
        nix::libc::signal(nix::libc::SIGTERM, signal_handler as nix::libc::sighandler_t);
        nix::libc::signal(nix::libc::SIGINT, signal_handler as nix::libc::sighandler_t);
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

    let init_w = if saved_win_w > 0 { saved_win_w } else { 800 };
    let init_h = if saved_win_h > 0 { saved_win_h } else { 600 };

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

    rl.set_target_fps(60);

    let dpi_scale = unsafe {
        let s = raylib::ffi::GetWindowScaleDPI();
        s.y
    };

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
    let (mut root, mut focused_panel_id, mut next_panel_id) = if let Some(state) = loaded_session {
        match session::restore(
            state,
            &config,
            scr_w, scr_h,
            status_bar_height,
            pad, minimap_width,
            cell_width, cell_height,
        ) {
            Ok((tree, focused, next_id, _restored_font_size)) => {
                (tree, focused, next_id)
            }
            Err(e) => {
                eprintln!("[TAI] Session restore failed, starting fresh: {e}");
                create_fresh_panel(&config, scr_w, scr_h, status_bar_height, pad, minimap_width, cell_width, cell_height)
            }
        }
    } else {
        create_fresh_panel(&config, scr_w, scr_h, status_bar_height, pad, minimap_width, cell_width, cell_height)
    };

    let mut prev_width = scr_w;
    let mut prev_height = scr_h;
    let mut title_frame: u32 = 0;
    let mut last_title = String::new();

    unsafe { raylib::ffi::SetExitKey(0); }

    let mut app_exit = false;
    let mut show_help = false;
    let mut show_session_mgr = false;
    let mut sm_names: Vec<String> = Vec::new();
    let mut sm_selected: usize = 0;
    let mut sm_scroll: usize = 0;
    let mut sm_input: Option<String> = None;
    let mut sm_status: Option<(String, Instant)> = None;
    let mut last_autosave = Instant::now();

    while !rl.window_should_close() && !app_exit && !SIGNAL_EXIT.load(Ordering::Relaxed) {
        if last_autosave.elapsed().as_secs() >= 5 {
            let win_pos = unsafe { raylib::ffi::GetWindowPosition() };
            if let Err(e) = session::save(
                &root, focused_panel_id, next_panel_id, font_size,
                rl.get_screen_width(), rl.get_screen_height(),
                win_pos.x as i32, win_pos.y as i32,
            ) {
                eprintln!("[TAI] Autosave failed: {e}");
            }
            last_autosave = Instant::now();
        }
        // Handle resize
        if rl.is_window_resized() {
            let w = rl.get_screen_width();
            let h = rl.get_screen_height();
            if w != prev_width || h != prev_height {
                relayout_and_resize(&mut root, w, h, status_bar_height, pad, minimap_width, cell_width, cell_height);
                prev_width = w;
                prev_height = h;
            }
        }

        // Read PTY and poll AI for ALL panels/tabs
        root.for_each_panel_mut(&mut |panel| {
            for tab in &mut panel.tabs {
                tab.read_pty();
                tab.poll_ai();
            }
        });

        // Update window title from focused panel's active tab
        title_frame += 1;
        if title_frame % 30 == 0 {
            if let Some(panel) = root.panel_by_id(focused_panel_id) {
                let tab_title = panel.active_tab().title();
                let new_title = if tab_title == "shell" {
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
            }
        } else if show_help && rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            show_help = false;
        }

        if show_help {
            loop {
                if unsafe { raylib::ffi::GetCharPressed() } == 0 { break; }
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
                            &root, focused_panel_id, next_panel_id, font_size,
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
                        &root, focused_panel_id, next_panel_id, font_size,
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

                            let scr_w = rl.get_screen_width();
                            let scr_h = rl.get_screen_height();
                            match session::restore(
                                state, &config, scr_w, scr_h,
                                status_bar_height, pad, minimap_width, cell_width, cell_height,
                            ) {
                                Ok((new_root, focused, next_id, _)) => {
                                    if font_changed {
                                        unsafe { raylib::ffi::UnloadFont(old_mono_font); }
                                    }
                                    root = new_root;
                                    focused_panel_id = focused;
                                    next_panel_id = next_id;
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

        // Split keybindings (Cmd+D, Cmd+Shift+D, Cmd+Shift+W, Cmd+Option+Arrows)
        let mut did_split_action = false;
        if show_help || show_session_mgr {
            // all input blocked while overlay is open
        } else if cmd_held && !shift_held && rl.is_key_pressed(KeyboardKey::KEY_D) {
            let new_id = alloc_panel_id(&mut next_panel_id);
            if let Some(fp) = root.panel_by_id(focused_panel_id) {
                let tbh = fp.tab_bar.height;
                let r = fp.rect;
                let (cols, rows) = panel_term_size(&PanelRect { x: 0, y: 0, w: r.w / 2, h: r.h }, pad, minimap_width, tbh, cell_width, cell_height);
                if let Ok(new_panel) = create_panel(new_id, &config, cols, rows, cell_width, cell_height) {
                    root.split_panel(focused_panel_id, SplitDirection::Horizontal, new_panel);
                    let scr_w = rl.get_screen_width();
                    let scr_h = rl.get_screen_height();
                    relayout_and_resize(&mut root, scr_w, scr_h, status_bar_height, pad, minimap_width, cell_width, cell_height);
                    focused_panel_id = new_id;
                    last_title.clear();
                }
            }
            did_split_action = true;
        } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_D) {
            let new_id = alloc_panel_id(&mut next_panel_id);
            if let Some(fp) = root.panel_by_id(focused_panel_id) {
                let tbh = fp.tab_bar.height;
                let r = fp.rect;
                let (cols, rows) = panel_term_size(&PanelRect { x: 0, y: 0, w: r.w, h: r.h / 2 }, pad, minimap_width, tbh, cell_width, cell_height);
                if let Ok(new_panel) = create_panel(new_id, &config, cols, rows, cell_width, cell_height) {
                    root.split_panel(focused_panel_id, SplitDirection::Vertical, new_panel);
                    let scr_w = rl.get_screen_width();
                    let scr_h = rl.get_screen_height();
                    relayout_and_resize(&mut root, scr_w, scr_h, status_bar_height, pad, minimap_width, cell_width, cell_height);
                    focused_panel_id = new_id;
                    last_title.clear();
                }
            }
            did_split_action = true;
        } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_W) {
            if root.panel_count() > 1 {
                let leaves = root.collect_leaves();
                let idx = leaves.iter().position(|&id| id == focused_panel_id).unwrap_or(0);
                root.close_panel(focused_panel_id);
                let new_leaves = root.collect_leaves();
                focused_panel_id = new_leaves[idx.min(new_leaves.len() - 1)];
                let scr_w = rl.get_screen_width();
                let scr_h = rl.get_screen_height();
                relayout_and_resize(&mut root, scr_w, scr_h, status_bar_height, pad, minimap_width, cell_width, cell_height);
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_RIGHT) {
            let leaves = root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == focused_panel_id) {
                focused_panel_id = leaves[(idx + 1) % leaves.len()];
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_LEFT) {
            let leaves = root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == focused_panel_id) {
                focused_panel_id = leaves[if idx == 0 { leaves.len() - 1 } else { idx - 1 }];
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_DOWN) {
            let leaves = root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == focused_panel_id) {
                focused_panel_id = leaves[(idx + 1) % leaves.len()];
                last_title.clear();
            }
            did_split_action = true;
        } else if cmd_held && alt_held && rl.is_key_pressed(KeyboardKey::KEY_UP) {
            let leaves = root.collect_leaves();
            if let Some(idx) = leaves.iter().position(|&id| id == focused_panel_id) {
                focused_panel_id = leaves[if idx == 0 { leaves.len() - 1 } else { idx - 1 }];
                last_title.clear();
            }
            did_split_action = true;
        }

        if show_session_mgr {
            // input handled by session manager above
        } else if did_split_action {
            // skip other keybindings this frame
        } else {
            // Tab management keybindings (scoped to focused panel)
            let mut tab_action_done = false;
            if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_T) {
                if let Some(panel) = root.panel_by_id_mut(focused_panel_id) {
                    let (cols, rows) = panel_term_size(&panel.rect, pad, minimap_width, panel.tab_bar.height, cell_width, cell_height);
                    if let Ok(t) = TabSession::new(&config, cols, rows, cell_width, cell_height) {
                        panel.tabs.push(t);
                        panel.active_tab = panel.tabs.len() - 1;
                        last_title.clear();
                    }
                }
                tab_action_done = true;
            } else if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_W) {
                if let Some(panel) = root.panel_by_id_mut(focused_panel_id) {
                    panel.tabs.remove(panel.active_tab);
                    if panel.tabs.is_empty() {
                        if root.panel_count() <= 1 {
                            app_exit = true;
                        }
                        // will close panel below
                    } else {
                        panel.active_tab = panel.active_tab.min(panel.tabs.len() - 1);
                        last_title.clear();
                    }
                }
                // If the panel became empty and there are other panels, close it
                let should_close = root.panel_by_id(focused_panel_id)
                    .map(|p| p.tabs.is_empty())
                    .unwrap_or(false);
                if should_close && !app_exit && root.panel_count() > 1 {
                    let leaves = root.collect_leaves();
                    let idx = leaves.iter().position(|&id| id == focused_panel_id).unwrap_or(0);
                    root.close_panel(focused_panel_id);
                    let new_leaves = root.collect_leaves();
                    focused_panel_id = new_leaves[idx.min(new_leaves.len() - 1)];
                    let scr_w = rl.get_screen_width();
                    let scr_h = rl.get_screen_height();
                    relayout_and_resize(&mut root, scr_w, scr_h, status_bar_height, pad, minimap_width, cell_width, cell_height);
                    last_title.clear();
                }
                tab_action_done = true;
            } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_RIGHT_BRACKET) {
                if let Some(panel) = root.panel_by_id_mut(focused_panel_id) {
                    panel.active_tab = (panel.active_tab + 1) % panel.tabs.len();
                    last_title.clear();
                }
                tab_action_done = true;
            } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_LEFT_BRACKET) {
                if let Some(panel) = root.panel_by_id_mut(focused_panel_id) {
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
                for (i, &key) in key_nums.iter().enumerate() {
                    if rl.is_key_pressed(key) {
                        if let Some(panel) = root.panel_by_id_mut(focused_panel_id) {
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
                    root.for_each_panel_mut(&mut |panel| {
                        panel.tab_bar.update_height(cell_height);
                    });
                    unsafe { raylib::ffi::UnloadFont(old_font); }
                    let w = rl.get_screen_width();
                    let h = rl.get_screen_height();
                    relayout_and_resize(&mut root, w, h, status_bar_height, pad, minimap_width, cell_width, cell_height);
                }

                // Copy/Paste (focused panel's active tab)
                if let Some(panel) = root.panel_by_id_mut(focused_panel_id) {
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
                                tab.pty.write(text.as_bytes());
                            }
                        }
                    }
                }

                // Mouse handling -- route to hovered panel
                {
                    let mx = rl.get_mouse_x();
                    let my = rl.get_mouse_y();

                    // Check if any panel's minimap is being dragged
                    let mut dragging_panel_id: Option<u32> = None;
                    root.for_each_panel(&mut |panel| {
                        let tab = &panel.tabs[panel.active_tab];
                        if tab.minimap.dragging {
                            dragging_panel_id = Some(panel.id);
                        }
                    });

                    let hover_panel_id = if let Some(drag_id) = dragging_panel_id {
                        Some(drag_id)
                    } else {
                        root.find_panel_at(mx, my).map(|p| p.id)
                    };

                    if let Some(hpid) = hover_panel_id {
                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                            focused_panel_id = hpid;
                        }

                        if let Some(panel) = root.panel_by_id_mut(hpid) {
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
                                            if let Ok(t) = TabSession::new(&config, cols, rows, cell_width, cell_height) {
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
                                        &rl, &mut tab.term, &tab.pty,
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
                    let empty_panel = root.panel_by_id(focused_panel_id)
                        .map(|p| p.tabs.is_empty())
                        .unwrap_or(false);
                    if empty_panel {
                        if root.panel_count() <= 1 {
                            app_exit = true;
                        } else {
                            let leaves = root.collect_leaves();
                            let idx = leaves.iter().position(|&id| id == focused_panel_id).unwrap_or(0);
                            root.close_panel(focused_panel_id);
                            let new_leaves = root.collect_leaves();
                            focused_panel_id = new_leaves[idx.min(new_leaves.len() - 1)];
                            let scr_w = rl.get_screen_width();
                            let scr_h = rl.get_screen_height();
                            relayout_and_resize(&mut root, scr_w, scr_h, status_bar_height, pad, minimap_width, cell_width, cell_height);
                            last_title.clear();
                        }
                    }
                }

                // Keyboard input dispatch -- focused panel only
                if let Some(panel) = root.panel_by_id_mut(focused_panel_id) {
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
                                        let chars = terminal::input::handle_input(&rl, &mut tab.term, &tab.pty);
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
                                        tab.router.handle_ai_prompt_submit(&mut tab.term, &tab.pty);
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
                                        tab.router.handle_command_confirm_enter(&mut tab.term, &tab.pty, &mut tab.overlay);
                                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                                        tab.router.handle_command_confirm_cancel(&tab.pty, &mut tab.overlay);
                                    } else if rl.is_key_pressed(KeyboardKey::KEY_E) {
                                        tab.router.handle_command_confirm_edit(&tab.pty, &mut tab.overlay);
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
        let panel_count = root.panel_count();

        let panel_info;
        {
            let leaves = root.collect_leaves();
            let panel_idx = leaves.iter().position(|&id| id == focused_panel_id).unwrap_or(0) + 1;
            if let Some(fp) = root.panel_by_id(focused_panel_id) {
                let tab = fp.active_tab();
                focused_mode = tab.router.mode();
                focused_ai_input = tab.router.ai_input_buffer().to_string();
                focused_auto_exec = tab.router.auto_execute();
                focused_cwd = tab.pty.get_cwd()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "~".to_string());
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
        d.clear_background(Color::new(20, 20, 25, 255));

        // Render all panels
        root.for_each_panel_mut(&mut |panel| {
            let r = panel.rect;
            let is_focused = panel.id == focused_panel_id;

            let tab_titles: Vec<String> = panel.tabs.iter().map(|t| t.title()).collect();
            let active_idx = panel.active_tab;

            if panel.tabs.is_empty() {
                return;
            }

            let tab = &mut panel.tabs[active_idx];
            tab.term.update_render_state();

            let bg_color = unsafe {
                let mut colors: bindings::GhosttyRenderStateColors = std::mem::zeroed();
                colors.size = std::mem::size_of::<bindings::GhosttyRenderStateColors>();
                bindings::ghostty_render_state_colors_get(tab.term.render_state(), &mut colors);
                Color::new(colors.background.r, colors.background.g, colors.background.b, 255)
            };

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
                d.draw_rectangle(r.x, r.y, r.w, 2, Color::new(80, 140, 220, 200));
                d.draw_rectangle(r.x, r.y + r.h - 2, r.w, 2, Color::new(80, 140, 220, 200));
                d.draw_rectangle(r.x, r.y, 2, r.h, Color::new(80, 140, 220, 200));
                d.draw_rectangle(r.x + r.w - 2, r.y, 2, r.h, Color::new(80, 140, 220, 200));
            }
        });

        // Separator lines between panels
        root.draw_separators(&mut d);

        // Floating AI prompt panel (focused panel only)
        if focused_mode == InputMode::AiPrompt {
            let pr = focused_panel_rect;
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();

            let ai_panel_w = (pr.w * 3 / 4).min(700).max(300);
            let ai_panel_x = pr.x + (pr.w - ai_panel_w) / 2;
            let inner_pad = 12;
            let label_h = cell_height + 4;

            let lines: Vec<&str> = focused_ai_input.split('\n').collect();
            let line_count = lines.len().max(1) as i32;
            let text_h = line_count * cell_height;
            let ai_panel_h = label_h + text_h + inner_pad * 2 + 4;
            let ai_panel_y = pr.y + pr.h - ai_panel_h - 8;

            // Dim overlay over full window
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
                ai_panel_w - inner_pad * 2,
                1,
                Color::new(55, 60, 75, 180),
            );

            let text_y = ai_panel_y + label_h + 6;
            let text_x = ai_panel_x + inner_pad;

            let blink_on = (d.get_time() * 2.0) as i32 % 2 == 0;
            for (i, line) in lines.iter().enumerate() {
                let cursor_str = if i == lines.len() - 1 && blink_on { "|" } else { "" };
                let display = format!("{}{}", line, cursor_str);
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

        // Help panel overlay
        if show_help {
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();

            d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 160));

            let help_w = 520.min(screen_w - 40);
            let help_x = (screen_w - help_w) / 2;
            let line_h = cell_height + 2;
            let inner_pad = 16;
            let section_gap = 6;

            let sections: &[(&str, &[(&str, &str)])] = &[
                ("General", &[
                    ("F1", "Toggle this help panel"),
                    ("F2", "Session manager"),
                    ("Ctrl+/", "Toggle AI prompt"),
                    ("Ctrl+Y", "Toggle YOLO (auto-execute)"),
                    ("Cmd+C", "Copy selection"),
                    ("Cmd+V", "Paste from clipboard"),
                ]),
                ("Tabs", &[
                    ("Cmd+T", "New tab"),
                    ("Cmd+W", "Close tab"),
                    ("Cmd+Shift+]", "Next tab"),
                    ("Cmd+Shift+[", "Previous tab"),
                    ("Cmd+1..9", "Jump to tab N"),
                ]),
                ("Splits", &[
                    ("Cmd+D", "Split horizontal"),
                    ("Cmd+Shift+D", "Split vertical"),
                    ("Cmd+Shift+W", "Close panel"),
                    ("Cmd+Opt+Right/Down", "Focus next panel"),
                    ("Cmd+Opt+Left/Up", "Focus previous panel"),
                ]),
                ("Font", &[
                    ("Cmd++", "Increase font size"),
                    ("Cmd+-", "Decrease font size"),
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

            let title_h = cell_height + 8;
            let mut total_lines = 0;
            for (_, entries) in sections {
                total_lines += 1 + entries.len();
            }
            total_lines += sections.len().saturating_sub(1);

            let help_h = title_h + inner_pad * 2 + total_lines as i32 * line_h + (sections.len() as i32 - 1) * section_gap;
            let help_h = help_h.min(screen_h - 40);
            let help_y = (screen_h - help_h) / 2;

            d.draw_rectangle(help_x, help_y, help_w, help_h, Color::new(25, 25, 32, 250));
            d.draw_rectangle(help_x, help_y, help_w, 1, Color::new(70, 80, 110, 220));
            d.draw_rectangle(help_x, help_y + help_h - 1, help_w, 1, Color::new(70, 80, 110, 220));
            d.draw_rectangle(help_x, help_y, 1, help_h, Color::new(70, 80, 110, 220));
            d.draw_rectangle(help_x + help_w - 1, help_y, 1, help_h, Color::new(70, 80, 110, 220));

            let title = std::ffi::CString::new("Keyboard Shortcuts").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, title.as_ptr(),
                    raylib::ffi::Vector2 { x: (help_x + inner_pad) as f32, y: (help_y + inner_pad / 2 + 2) as f32 },
                    font_size as f32, 0.0,
                    raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 },
                );
            }

            d.draw_rectangle(help_x + inner_pad, help_y + title_h, help_w - inner_pad * 2, 1, Color::new(55, 60, 75, 180));

            let label_size = (font_size - 2).max(8);
            let key_col_w = 200;
            let mut cy = help_y + title_h + inner_pad;

            for (si, (section_name, entries)) in sections.iter().enumerate() {
                if si > 0 {
                    cy += section_gap;
                }

                let section_c = std::ffi::CString::new(*section_name).unwrap_or_default();
                unsafe {
                    raylib::ffi::DrawTextEx(
                        mono_font, section_c.as_ptr(),
                        raylib::ffi::Vector2 { x: (help_x + inner_pad) as f32, y: cy as f32 },
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
                            raylib::ffi::Vector2 { x: (help_x + inner_pad + 10) as f32, y: cy as f32 },
                            label_size as f32, 0.0,
                            raylib::ffi::Color { r: 200, g: 200, b: 140, a: 255 },
                        );
                        raylib::ffi::DrawTextEx(
                            mono_font, desc_c.as_ptr(),
                            raylib::ffi::Vector2 { x: (help_x + inner_pad + key_col_w) as f32, y: cy as f32 },
                            label_size as f32, 0.0,
                            raylib::ffi::Color { r: 180, g: 180, b: 190, a: 255 },
                        );
                    }
                    cy += line_h;
                }
            }

            let hint = std::ffi::CString::new("Press F1 or Esc to close").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font, hint.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (help_x + inner_pad) as f32,
                        y: (help_y + help_h - cell_height - inner_pad / 2) as f32,
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

            let sm_w = 480.min(screen_w - 40);
            let sm_h = title_h + inner_pad * 2 + list_h + footer_h + status_h + input_h;
            let sm_h = sm_h.min(screen_h - 40);
            let sm_x = (screen_w - sm_w) / 2;
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

        // Status bar (global, at bottom)
        status_bar.render(
            &mono_font,
            d.get_screen_width(),
            d.get_screen_height(),
            font_size,
            focused_mode,
            &focused_cwd,
            &focused_ai_input,
            focused_auto_exec,
            panel_info,
            &mut d,
        );
    }

    // Save session on exit (all vars still alive, Drop not yet fired)
    let win_pos = unsafe { raylib::ffi::GetWindowPosition() };
    if let Err(e) = session::save(
        &root, focused_panel_id, next_panel_id, font_size,
        rl.get_screen_width(), rl.get_screen_height(),
        win_pos.x as i32, win_pos.y as i32,
    ) {
        eprintln!("[TAI] Failed to save session on exit: {e}");
    }
}
