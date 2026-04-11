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
mod terminal;

use ai::bridge::AiBridge;
use config::TaiConfig;
use minimap::Minimap;
use overlay::CommandOverlay;
use router::{InputMode, InputRouter};
use selection::TextSelection;
use status_bar::StatusBar;
use terminal::engine::{PtyReadResult, Terminal};
use terminal::pty::Pty;
use raylib::prelude::*;
use nix::libc;

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
    cps.extend(32..=126);          // Basic Latin (ASCII)
    cps.extend(160..=255);         // Latin-1 Supplement
    cps.extend(0x100..=0x17F);     // Latin Extended-A
    cps.extend(0x180..=0x24F);     // Latin Extended-B
    cps.extend(0x2000..=0x206F);   // General Punctuation
    cps.extend(0x2070..=0x209F);   // Superscripts/Subscripts
    cps.extend(0x20A0..=0x20CF);   // Currency Symbols
    cps.extend(0x2100..=0x214F);   // Letterlike Symbols
    cps.extend(0x2150..=0x218F);   // Number Forms
    cps.extend(0x2190..=0x21FF);   // Arrows
    cps.extend(0x2200..=0x22FF);   // Mathematical Operators
    cps.extend(0x2300..=0x23FF);   // Miscellaneous Technical
    cps.extend(0x2400..=0x243F);   // Control Pictures
    cps.extend(0x2460..=0x24FF);   // Enclosed Alphanumerics
    cps.extend(0x2500..=0x257F);   // Box Drawing
    cps.extend(0x2580..=0x259F);   // Block Elements
    cps.extend(0x25A0..=0x25FF);   // Geometric Shapes
    cps.extend(0x2600..=0x26FF);   // Miscellaneous Symbols
    cps.extend(0x2700..=0x27BF);   // Dingbats
    cps.extend(0x27C0..=0x27FF);   // Supplemental Arrows-A
    cps.extend(0x2800..=0x28FF);   // Braille Patterns
    cps.extend(0x2900..=0x297F);   // Supplemental Arrows-B
    cps.extend(0xE0A0..=0xE0D4);   // Powerline Symbols
    cps.extend(0xE200..=0xE2FF);   // Extra terminal symbols
    cps.extend(0xF000..=0xF0FF);   // Private Use Area (common)
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
        .title("TAI - Terminal AI")
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

    let scr_w = rl.get_screen_width();
    let scr_h = rl.get_screen_height();
    let term_cols = ((scr_w - 2 * pad - minimap_width) / cell_width).max(1) as u16;
    let term_rows = (((scr_h - status_bar_height) - 2 * pad) / cell_height).max(1) as u16;

    let mut term = match Terminal::new(term_cols, term_rows, config.terminal.scrollback) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to create terminal: {e}");
            return;
        }
    };

    let pty = match Pty::spawn(term_cols, term_rows, cell_width, cell_height) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to spawn PTY: {e}");
            return;
        }
    };

    term.setup_effects(pty.master_fd(), cell_width, cell_height);
    term.resize(term_cols, term_rows, cell_width as u32, cell_height as u32);

    let ai_bridge = config.api_key().map(|key| AiBridge::new(&config.ai, &key));
    let ai_available = ai_bridge.is_some();

    let mut router = InputRouter::new(&config, ai_bridge);
    let mut overlay = CommandOverlay::new();
    let status_bar = StatusBar::new(&config.ai.model, ai_available);
    let mut text_selection = TextSelection::new();

    let mut minimap = Minimap::new(term_cols);
    let mut pty_mirror: Vec<u8> = Vec::with_capacity(8192);

    let mut prev_width = scr_w;
    let mut prev_height = scr_h;
    let mut child_exited = false;
    let mut title_frame: u32 = 0;
    let mut last_title = String::new();

    // Disable Raylib's default Escape-to-close so Esc can be used for AI prompt cancel
    unsafe { raylib::ffi::SetExitKey(0); }

    while !rl.window_should_close() {
        // Handle resize
        if rl.is_window_resized() {
            let w = rl.get_screen_width();
            let h = rl.get_screen_height();
            if w != prev_width || h != prev_height {
                let cols = ((w - 2 * pad - minimap_width) / cell_width).max(1) as u16;
                let rows = (((h - status_bar_height) - 2 * pad) / cell_height).max(1) as u16;
                term.resize(cols, rows, cell_width as u32, cell_height as u32);
                pty.resize(cols, rows, cell_width, cell_height);
                minimap.set_cols(cols);
                prev_width = w;
                prev_height = h;
            }
        }

        // Read PTY
        if !child_exited {
            let capture = router.capture_buffer();
            match pty.read_nonblocking(&mut term, capture, Some(&mut pty_mirror)) {
                PtyReadResult::Ok => {}
                PtyReadResult::Eof | PtyReadResult::Error => {
                    child_exited = true;
                }
            }
            if !pty_mirror.is_empty() {
                minimap.feed(&pty_mirror);
                pty_mirror.clear();
            }
        }

        // Update window title with foreground process name (~every 30 frames)
        title_frame += 1;
        if title_frame % 30 == 0 {
            let new_title = match pty.get_foreground_process_name() {
                Some(name) => format!("TAI - {}", name),
                None => "TAI - Terminal AI".to_string(),
            };
            if new_title != last_title {
                let c_title = std::ffi::CString::new(new_title.as_str()).unwrap_or_default();
                unsafe { raylib::ffi::SetWindowTitle(c_title.as_ptr()); }
                last_title = new_title;
            }
        }

        // Cmd+Plus / Cmd+Minus / Cmd+0 for font size
        let cmd_held = rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER);

        if cmd_held && (rl.is_key_pressed(KeyboardKey::KEY_EQUAL) || rl.is_key_pressed(KeyboardKey::KEY_KP_ADD)) {
            font_size = (font_size + 2).min(72);
            let old_font = mono_font;
            metrics = load_font(font_size, dpi_scale, &mut codepoints);
            mono_font = metrics.font;
            cell_width = metrics.cell_width;
            cell_height = metrics.cell_height;
            status_bar_height = font_size + 8;
            unsafe { raylib::ffi::UnloadFont(old_font); }
            let w = rl.get_screen_width();
            let h = rl.get_screen_height();
            let cols = ((w - 2 * pad - minimap_width) / cell_width).max(1) as u16;
            let rows = (((h - status_bar_height) - 2 * pad) / cell_height).max(1) as u16;
            term.resize(cols, rows, cell_width as u32, cell_height as u32);
            pty.resize(cols, rows, cell_width, cell_height);
            minimap.set_cols(cols);
        }
        if cmd_held && (rl.is_key_pressed(KeyboardKey::KEY_MINUS) || rl.is_key_pressed(KeyboardKey::KEY_KP_SUBTRACT)) {
            font_size = (font_size - 2).max(8);
            let old_font = mono_font;
            metrics = load_font(font_size, dpi_scale, &mut codepoints);
            mono_font = metrics.font;
            cell_width = metrics.cell_width;
            cell_height = metrics.cell_height;
            status_bar_height = font_size + 8;
            unsafe { raylib::ffi::UnloadFont(old_font); }
            let w = rl.get_screen_width();
            let h = rl.get_screen_height();
            let cols = ((w - 2 * pad - minimap_width) / cell_width).max(1) as u16;
            let rows = (((h - status_bar_height) - 2 * pad) / cell_height).max(1) as u16;
            term.resize(cols, rows, cell_width as u32, cell_height as u32);
            pty.resize(cols, rows, cell_width, cell_height);
            minimap.set_cols(cols);
        }
        if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_ZERO) {
            font_size = default_font_size;
            let old_font = mono_font;
            metrics = load_font(font_size, dpi_scale, &mut codepoints);
            mono_font = metrics.font;
            cell_width = metrics.cell_width;
            cell_height = metrics.cell_height;
            status_bar_height = font_size + 8;
            unsafe { raylib::ffi::UnloadFont(old_font); }
            let w = rl.get_screen_width();
            let h = rl.get_screen_height();
            let cols = ((w - 2 * pad - minimap_width) / cell_width).max(1) as u16;
            let rows = (((h - status_bar_height) - 2 * pad) / cell_height).max(1) as u16;
            term.resize(cols, rows, cell_width as u32, cell_height as u32);
            pty.resize(cols, rows, cell_width, cell_height);
            minimap.set_cols(cols);
        }

        // Cmd+C = copy selection, Cmd+V = paste
        if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_C) {
            if text_selection.has_selection() {
                let rows = term.get_viewport_rows();
                let selected = extract_selected_text(&text_selection, &rows);
                if !selected.is_empty() {
                    selection::copy_to_clipboard(&selected);
                }
                text_selection.clear();
            }
        }
        if cmd_held && rl.is_key_pressed(KeyboardKey::KEY_V) {
            if let Some(text) = selection::paste_from_clipboard() {
                if !text.is_empty() {
                    pty.write(text.as_bytes());
                }
            }
        }

        // Mouse: minimap click-to-scroll or text selection
        {
            let mut mouse_tracking = false;
            unsafe {
                bindings::ghostty_terminal_get(
                    term.handle(),
                    bindings::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
                    &mut mouse_tracking as *mut bool as *mut std::ffi::c_void,
                );
            }
            let mx = rl.get_mouse_x();
            let my = rl.get_mouse_y();
            let scr_w_now = rl.get_screen_width();
            let in_minimap = mx >= scr_w_now - minimap.width;

            if in_minimap || minimap.dragging {
                if let Some((total, offset, len)) = term.get_scrollbar() {
                    let scr_h_now = rl.get_screen_height();
                    if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                        let delta = minimap.handle_mouse_press(my, scr_h_now, status_bar_height, total, offset, len);
                        if delta != 0 {
                            term.scroll_viewport(delta);
                        }
                    } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) && minimap.dragging {
                        let delta = minimap.handle_mouse_drag(my, scr_h_now, status_bar_height, total, offset, len);
                        if delta != 0 {
                            term.scroll_viewport(delta);
                        }
                    }
                }
                if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
                    minimap.handle_mouse_release();
                }
            } else if !mouse_tracking {
                let (col, row) = selection::mouse_to_cell(mx, my, cell_width, cell_height, pad);

                if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
                    text_selection.begin(col, row);
                } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) && text_selection.active {
                    text_selection.update(col, row);
                } else if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) && text_selection.active {
                    text_selection.update(col, row);
                    text_selection.finish();
                }
            }
        }

        // Input handling based on mode
        let mode = router.mode();

        let ctrl_held = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
            || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);

        // Ctrl+/ = AI prompt toggle
        if ctrl_held && rl.is_key_pressed(KeyboardKey::KEY_SLASH) {
            router.toggle_ai_mode();
        // Ctrl+Y = YOLO mode toggle (auto-execute)
        } else if ctrl_held && rl.is_key_pressed(KeyboardKey::KEY_Y) {
            router.toggle_auto_execute();
        } else {
            match mode {
                InputMode::Shell => {
                    if !child_exited {
                        let chars = terminal::input::handle_input(&rl, &mut term, &pty);
                        for c in chars {
                            router.track_shell_char(c);
                        }
                        terminal::input::handle_mouse(&rl, &mut term, &pty, cell_width, cell_height, pad);
                    }
                }
                InputMode::AiPrompt => {
                    let ctrl = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
                        || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);

                    // Ctrl+J inserts a newline
                    if ctrl && rl.is_key_pressed(KeyboardKey::KEY_J) {
                        router.handle_ai_prompt_char('\n');
                    } else {
                        loop {
                            let ch = unsafe { raylib::ffi::GetCharPressed() };
                            if ch == 0 { break; }
                            if ch == 0x0A { continue; } // skip raw \n from Ctrl+J
                            if let Some(c) = char::from_u32(ch as u32) {
                                router.handle_ai_prompt_char(c);
                            }
                        }
                    }
                    if rl.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
                        || unsafe { raylib::ffi::IsKeyPressedRepeat(KeyboardKey::KEY_BACKSPACE as i32) }
                    {
                        router.handle_ai_prompt_backspace();
                    }
                    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) && !ctrl {
                        router.handle_ai_prompt_submit(&mut term, &pty);
                    }
                    if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                        router.handle_ai_prompt_cancel();
                    }
                }
                InputMode::AiStreaming => {
                    if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                        // TODO: send cancel request
                    }
                }
                InputMode::CommandConfirm => {
                    if rl.is_key_pressed(KeyboardKey::KEY_ENTER) {
                        router.handle_command_confirm_enter(&mut term, &pty, &mut overlay);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
                        router.handle_command_confirm_cancel(&pty, &mut overlay);
                    } else if rl.is_key_pressed(KeyboardKey::KEY_E) {
                        router.handle_command_confirm_edit(&pty, &mut overlay);
                    }
                }
            }
        }

        // Poll AI responses
        router.poll_ai_responses(&mut term, &pty, &mut overlay);

        if let Some(vt_data) = term.drain_vt_mirror() {
            minimap.feed(&vt_data);
        }

        // Update render state
        term.update_render_state();

        // Get background color
        let bg_color = unsafe {
            let mut colors: bindings::GhosttyRenderStateColors = std::mem::zeroed();
            colors.size = std::mem::size_of::<bindings::GhosttyRenderStateColors>();
            bindings::ghostty_render_state_colors_get(term.render_state(), &mut colors);
            Color::new(colors.background.r, colors.background.g, colors.background.b, 255)
        };

        let cwd_str = pty
            .get_cwd()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~".to_string());

        // Render
        let mut d = rl.begin_drawing(&thread);
        d.clear_background(bg_color);

        terminal::renderer::render_terminal(
            term.render_state(),
            term.row_iter(),
            term.row_cells(),
            &mono_font,
            cell_width,
            cell_height,
            font_size,
            pad,
            term.handle(),
            &mut d,
        );

        // Draw minimap
        if let Some((total, offset, len)) = term.get_scrollbar() {
            let scr_w = d.get_screen_width();
            let scr_h = d.get_screen_height();
            minimap.render(total, offset, len, scr_w, scr_h, status_bar_height, pad, &mut d);
        }

        // Draw text selection highlight
        if text_selection.has_selection() {
            let scr_w = d.get_screen_width();
            let term_cols = (scr_w - 2 * pad) / cell_width;
            let scr_h = d.get_screen_height();
            let term_rows = ((scr_h - status_bar_height) - 2 * pad) / cell_height;
            text_selection.render(cell_width, cell_height, pad, term_cols, term_rows, &mut d);
        }

        // Draw floating AI prompt panel
        if router.mode() == InputMode::AiPrompt {
            let input = router.ai_input_buffer();
            let screen_w = d.get_screen_width();
            let screen_h = d.get_screen_height();

            let panel_w = (screen_w * 3 / 4).min(700).max(300);
            let panel_x = (screen_w - panel_w) / 2;
            let inner_pad = 12;
            let label_h = cell_height + 4;

            let lines: Vec<&str> = input.split('\n').collect();
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

        overlay.render(
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
            router.mode(),
            &cwd_str,
            router.ai_input_buffer(),
            router.auto_execute(),
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

    // Cleanup: send SIGHUP to child
    unsafe {
        if !child_exited {
            libc::kill(pty.child_pid().as_raw(), libc::SIGHUP);
        }
        libc::waitpid(pty.child_pid().as_raw(), std::ptr::null_mut(), 0);
    }
}
