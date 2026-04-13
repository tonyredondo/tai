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
mod status_bar;
mod tab;
mod tab_bar;
mod terminal;

use config::TaiConfig;
use router::InputMode;
use selection::TextSelection;
use status_bar::StatusBar;
use tab::TabSession;
use tab_bar::{TabBar, TabBarAction};
use raylib::prelude::*;

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

fn calc_term_size(w: i32, h: i32, pad: i32, minimap_width: i32, tab_bar_height: i32, status_bar_height: i32, cell_width: i32, cell_height: i32) -> (u16, u16) {
    let cols = ((w - 2 * pad - minimap_width) / cell_width).max(1) as u16;
    let rows = (((h - status_bar_height - tab_bar_height) - 2 * pad) / cell_height).max(1) as u16;
    (cols, rows)
}

fn main() {
    let config = TaiConfig::load();
    let default_font_size = config.terminal.font_size;
    let mut font_size = default_font_size;

    unsafe {
        raylib::ffi::SetConfigFlags(
            raylib::consts::ConfigFlags::FLAG_WINDOW_HIGHDPI as u32,
        );
    }

    let (mut rl, thread) = raylib::init()
        .size(800, 600)
        .title("Terminal AI")
        .resizable()
        .build();

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
    let mut tab_bar = TabBar::new(cell_height);
    let tab_bar_height = tab_bar.height;

    let scr_w = rl.get_screen_width();
    let scr_h = rl.get_screen_height();
    let (term_cols, term_rows) = calc_term_size(scr_w, scr_h, pad, minimap_width, tab_bar_height, status_bar_height, cell_width, cell_height);

    let ai_available = config.api_key().is_some();
    let status_bar = StatusBar::new(&config.ai.model, ai_available);

    let mut tabs: Vec<TabSession> = Vec::new();
    match TabSession::new(&config, term_cols, term_rows, cell_width, cell_height) {
        Ok(t) => tabs.push(t),
        Err(e) => {
            eprintln!("Failed to create tab: {e}");
            return;
        }
    }
    let mut active_tab: usize = 0;

    let mut prev_width = scr_w;
    let mut prev_height = scr_h;
    let mut title_frame: u32 = 0;
    let mut last_title = String::new();

    unsafe { raylib::ffi::SetExitKey(0); }

    while !rl.window_should_close() {
        if tabs.is_empty() {
            break;
        }

        // Handle resize
        if rl.is_window_resized() {
            let w = rl.get_screen_width();
            let h = rl.get_screen_height();
            if w != prev_width || h != prev_height {
                let (cols, rows) = calc_term_size(w, h, pad, minimap_width, tab_bar.height, status_bar_height, cell_width, cell_height);
                for tab in &mut tabs {
                    tab.resize(cols, rows, cell_width as u32, cell_height as u32);
                }
                prev_width = w;
                prev_height = h;
            }
        }

        // Read PTY for ALL tabs
        for tab in &mut tabs {
            tab.read_pty();
        }

        // Poll AI for ALL tabs
        for tab in &mut tabs {
            tab.poll_ai();
        }

        // Update window title from active tab (~every 30 frames)
        title_frame += 1;
        if title_frame % 30 == 0 {
            let tab = &tabs[active_tab];
            let tab_title = tab.title();
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

        // Key modifiers
        let cmd_held = rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER);
        let shift_held = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);
        let ctrl_held = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);

        // Tab management keybindings
        if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_T) {
            let (cols, rows) = calc_term_size(
                rl.get_screen_width(), rl.get_screen_height(),
                pad, minimap_width, tab_bar.height, status_bar_height,
                cell_width, cell_height,
            );
            if let Ok(t) = TabSession::new(&config, cols, rows, cell_width, cell_height) {
                tabs.push(t);
                active_tab = tabs.len() - 1;
            }
        } else if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_W) {
            tabs.remove(active_tab);
            if tabs.is_empty() {
                break;
            }
            active_tab = active_tab.min(tabs.len() - 1);
            last_title.clear();
        } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_RIGHT_BRACKET) {
            active_tab = (active_tab + 1) % tabs.len();
            last_title.clear();
        } else if cmd_held && shift_held && rl.is_key_pressed(KeyboardKey::KEY_LEFT_BRACKET) {
            active_tab = if active_tab == 0 { tabs.len() - 1 } else { active_tab - 1 };
            last_title.clear();
        } else {
            // Cmd+1..9 jump to tab
            let mut tab_jump = false;
            if cmd_held {
                let key_nums = [
                    KeyboardKey::KEY_ONE, KeyboardKey::KEY_TWO, KeyboardKey::KEY_THREE,
                    KeyboardKey::KEY_FOUR, KeyboardKey::KEY_FIVE, KeyboardKey::KEY_SIX,
                    KeyboardKey::KEY_SEVEN, KeyboardKey::KEY_EIGHT, KeyboardKey::KEY_NINE,
                ];
                for (i, &key) in key_nums.iter().enumerate() {
                    if rl.is_key_pressed(key) && i < tabs.len() {
                        active_tab = i;
                        last_title.clear();
                        tab_jump = true;
                        break;
                    }
                }
            }

            if !tab_jump {
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
                    tab_bar.update_height(cell_height);
                    unsafe { raylib::ffi::UnloadFont(old_font); }
                    let w = rl.get_screen_width();
                    let h = rl.get_screen_height();
                    let (cols, rows) = calc_term_size(w, h, pad, minimap_width, tab_bar.height, status_bar_height, cell_width, cell_height);
                    for tab in &mut tabs {
                        tab.resize(cols, rows, cell_width as u32, cell_height as u32);
                    }
                }

                // Copy/Paste (active tab)
                let tab = &mut tabs[active_tab];
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

                // Mouse handling
                {
                    let mx = rl.get_mouse_x();
                    let my = rl.get_mouse_y();
                    let scr_w_now = rl.get_screen_width();
                    let scr_h_now = rl.get_screen_height();

                    if my < tab_bar.height {
                        // Tab bar area
                        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                            match tab_bar.handle_click(mx, my, scr_w_now, tabs.len()) {
                                TabBarAction::SwitchTo(idx) => {
                                    active_tab = idx;
                                    last_title.clear();
                                }
                                TabBarAction::Close(idx) => {
                                    tabs.remove(idx);
                                    if tabs.is_empty() {
                                        break;
                                    }
                                    active_tab = active_tab.min(tabs.len() - 1);
                                    last_title.clear();
                                }
                                TabBarAction::New => {
                                    let (cols, rows) = calc_term_size(
                                        scr_w_now, scr_h_now, pad, minimap_width,
                                        tab_bar.height, status_bar_height, cell_width, cell_height,
                                    );
                                    if let Ok(t) = TabSession::new(&config, cols, rows, cell_width, cell_height) {
                                        tabs.push(t);
                                        active_tab = tabs.len() - 1;
                                        last_title.clear();
                                    }
                                }
                                TabBarAction::None => {}
                            }
                        }
                    } else {
                        let tab = &mut tabs[active_tab];
                        let mut mouse_tracking = false;
                        unsafe {
                            bindings::ghostty_terminal_get(
                                tab.term.handle(),
                                bindings::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
                                &mut mouse_tracking as *mut bool as *mut std::ffi::c_void,
                            );
                        }
                        let in_minimap = mx >= scr_w_now - tab.minimap.width;

                        if in_minimap || tab.minimap.dragging {
                            if let Some((total, offset, len)) = tab.term.get_scrollbar() {
                                if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                                    let delta = tab.minimap.handle_mouse_press(my, scr_h_now, status_bar_height + tab_bar.height, total, offset, len);
                                    if delta != 0 { tab.term.scroll_viewport(delta); }
                                } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) && tab.minimap.dragging {
                                    let delta = tab.minimap.handle_mouse_drag(my, scr_h_now, status_bar_height + tab_bar.height, total, offset, len);
                                    if delta != 0 { tab.term.scroll_viewport(delta); }
                                }
                            }
                            if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
                                tab.minimap.handle_mouse_release();
                            }
                        } else if !mouse_tracking {
                            let (col, row) = selection::mouse_to_cell(mx, my - tab_bar.height, cell_width, cell_height, pad);
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

                // Input handling based on mode (active tab)
                let tab = &mut tabs[active_tab];
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
                                terminal::input::handle_mouse(&rl, &mut tab.term, &tab.pty, cell_width, cell_height, pad + tab_bar.height);
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

        // Collect info before mutable borrow for rendering
        let tab_titles: Vec<String> = tabs.iter().map(|t| t.title()).collect();
        let tab_count = tabs.len();

        // Render active tab
        let tab = &mut tabs[active_tab];
        tab.term.update_render_state();

        let bg_color = unsafe {
            let mut colors: bindings::GhosttyRenderStateColors = std::mem::zeroed();
            colors.size = std::mem::size_of::<bindings::GhosttyRenderStateColors>();
            bindings::ghostty_render_state_colors_get(tab.term.render_state(), &mut colors);
            Color::new(colors.background.r, colors.background.g, colors.background.b, 255)
        };

        let cwd_str = tab.pty
            .get_cwd()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~".to_string());

        let active_mode = tab.router.mode();
        let ai_input = tab.router.ai_input_buffer().to_string();
        let auto_exec = tab.router.auto_execute();
        let child_exited = tab.child_exited;

        let mut d = rl.begin_drawing(&thread);
        d.clear_background(bg_color);

        // Tab bar
        tab_bar.render(&tab_titles, active_tab, &mono_font, font_size, d.get_screen_width(), &mut d);

        // Terminal grid (offset by tab bar height)
        terminal::renderer::render_terminal(
            tab.term.render_state(),
            tab.term.row_iter(),
            tab.term.row_cells(),
            &mono_font,
            cell_width,
            cell_height,
            font_size,
            pad + tab_bar.height,
            tab.term.handle(),
            &mut d,
        );

        // Minimap
        if let Some((total, offset, len)) = tab.term.get_scrollbar() {
            let scr_w = d.get_screen_width();
            let scr_h = d.get_screen_height();
            tab.minimap.render(total, offset, len, scr_w, scr_h, status_bar_height + tab_bar.height, pad, &mut d);
        }

        // Text selection highlight
        if tab.selection.has_selection() {
            let scr_w = d.get_screen_width();
            let term_cols = (scr_w - 2 * pad) / cell_width;
            let scr_h = d.get_screen_height();
            let term_rows = ((scr_h - status_bar_height - tab_bar.height) - 2 * pad) / cell_height;
            tab.selection.render(cell_width, cell_height, pad + tab_bar.height, term_cols, term_rows, &mut d);
        }

        // Floating AI prompt panel
        if active_mode == InputMode::AiPrompt {
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();

            let panel_w = (screen_w * 3 / 4).min(700).max(300);
            let panel_x = (screen_w - panel_w) / 2;
            let inner_pad = 12;
            let label_h = cell_height + 4;

            let lines: Vec<&str> = ai_input.split('\n').collect();
            let line_count = lines.len().max(1) as i32;
            let text_h = line_count * cell_height;
            let panel_h = label_h + text_h + inner_pad * 2 + 4;
            let panel_y = screen_h - status_bar_height - panel_h - 8;

            d.draw_rectangle(0, 0, screen_w, screen_h, Color::new(0, 0, 0, 100));

            d.draw_rectangle(panel_x, panel_y, panel_w, panel_h, Color::new(30, 30, 38, 245));
            d.draw_rectangle(panel_x, panel_y, panel_w, 1, Color::new(70, 80, 110, 200));
            d.draw_rectangle(panel_x, panel_y + panel_h - 1, panel_w, 1, Color::new(70, 80, 110, 200));
            d.draw_rectangle(panel_x, panel_y, 1, panel_h, Color::new(70, 80, 110, 200));
            d.draw_rectangle(panel_x + panel_w - 1, panel_y, 1, panel_h, Color::new(70, 80, 110, 200));

            let label = std::ffi::CString::new("Ask AI").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font,
                    label.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (panel_x + inner_pad) as f32,
                        y: (panel_y + inner_pad / 2) as f32,
                    },
                    (font_size - 2) as f32,
                    0.0,
                    raylib::ffi::Color { r: 100, g: 110, b: 140, a: 200 },
                );
            }

            d.draw_rectangle(
                panel_x + inner_pad,
                panel_y + label_h,
                panel_w - inner_pad * 2,
                1,
                Color::new(55, 60, 75, 180),
            );

            let text_y = panel_y + label_h + 6;
            let text_x = panel_x + inner_pad;

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

        tab.overlay.render(
            &mono_font,
            d.get_screen_width(),
            d.get_screen_height(),
            font_size,
            &mut d,
        );

        status_bar.render(
            &mono_font,
            d.get_screen_width(),
            d.get_screen_height(),
            font_size,
            active_mode,
            &cwd_str,
            &ai_input,
            auto_exec,
            if tab_count > 1 { Some((active_tab + 1, tab_count)) } else { None },
            &mut d,
        );

        if child_exited {
            let msg = "[process exited]";
            let msg_c = std::ffi::CString::new(msg).unwrap();
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();
            let banner_h = font_size + 8;
            d.draw_rectangle(0, screen_h - banner_h - status_bar_height, screen_w, banner_h, Color::new(0, 0, 0, 180));
            unsafe {
                raylib::ffi::DrawTextEx(
                    mono_font,
                    msg_c.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: 10.0,
                        y: (screen_h - banner_h - status_bar_height + 4) as f32,
                    },
                    font_size as f32,
                    0.0,
                    raylib::ffi::Color { r: 255, g: 255, b: 255, a: 255 },
                );
            }
        }
    }
}
