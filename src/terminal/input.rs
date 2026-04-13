use crate::bindings::*;
use crate::terminal::engine::Terminal;
use crate::terminal::pty::Pty;
use raylib::prelude::*;

fn raylib_key_to_ghostty(rl_key: KeyboardKey) -> u32 {
    use KeyboardKey::*;
    let k = rl_key as i32;

    if k >= KEY_A as i32 && k <= KEY_Z as i32 {
        return GhosttyKey_GHOSTTY_KEY_A + (k - KEY_A as i32) as u32;
    }
    if k >= KEY_ZERO as i32 && k <= KEY_NINE as i32 {
        return GhosttyKey_GHOSTTY_KEY_DIGIT_0 + (k - KEY_ZERO as i32) as u32;
    }
    if k >= KEY_F1 as i32 && k <= KEY_F12 as i32 {
        return GhosttyKey_GHOSTTY_KEY_F1 + (k - KEY_F1 as i32) as u32;
    }

    match rl_key {
        KEY_SPACE => GhosttyKey_GHOSTTY_KEY_SPACE,
        KEY_ENTER => GhosttyKey_GHOSTTY_KEY_ENTER,
        KEY_TAB => GhosttyKey_GHOSTTY_KEY_TAB,
        KEY_BACKSPACE => GhosttyKey_GHOSTTY_KEY_BACKSPACE,
        KEY_DELETE => GhosttyKey_GHOSTTY_KEY_DELETE,
        KEY_ESCAPE => GhosttyKey_GHOSTTY_KEY_ESCAPE,
        KEY_UP => GhosttyKey_GHOSTTY_KEY_ARROW_UP,
        KEY_DOWN => GhosttyKey_GHOSTTY_KEY_ARROW_DOWN,
        KEY_LEFT => GhosttyKey_GHOSTTY_KEY_ARROW_LEFT,
        KEY_RIGHT => GhosttyKey_GHOSTTY_KEY_ARROW_RIGHT,
        KEY_HOME => GhosttyKey_GHOSTTY_KEY_HOME,
        KEY_END => GhosttyKey_GHOSTTY_KEY_END,
        KEY_PAGE_UP => GhosttyKey_GHOSTTY_KEY_PAGE_UP,
        KEY_PAGE_DOWN => GhosttyKey_GHOSTTY_KEY_PAGE_DOWN,
        KEY_INSERT => GhosttyKey_GHOSTTY_KEY_INSERT,
        KEY_MINUS => GhosttyKey_GHOSTTY_KEY_MINUS,
        KEY_EQUAL => GhosttyKey_GHOSTTY_KEY_EQUAL,
        KEY_LEFT_BRACKET => GhosttyKey_GHOSTTY_KEY_BRACKET_LEFT,
        KEY_RIGHT_BRACKET => GhosttyKey_GHOSTTY_KEY_BRACKET_RIGHT,
        KEY_BACKSLASH => GhosttyKey_GHOSTTY_KEY_BACKSLASH,
        KEY_SEMICOLON => GhosttyKey_GHOSTTY_KEY_SEMICOLON,
        KEY_APOSTROPHE => GhosttyKey_GHOSTTY_KEY_QUOTE,
        KEY_COMMA => GhosttyKey_GHOSTTY_KEY_COMMA,
        KEY_PERIOD => GhosttyKey_GHOSTTY_KEY_PERIOD,
        KEY_SLASH => GhosttyKey_GHOSTTY_KEY_SLASH,
        KEY_GRAVE => GhosttyKey_GHOSTTY_KEY_BACKQUOTE,
        _ => GhosttyKey_GHOSTTY_KEY_UNIDENTIFIED,
    }
}

fn get_ghostty_mods(rl: &RaylibHandle) -> u16 {
    let mut mods: u16 = 0;
    if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT) {
        mods |= GHOSTTY_MODS_SHIFT as u16;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL) || rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL) {
        mods |= GHOSTTY_MODS_CTRL as u16;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_ALT) || rl.is_key_down(KeyboardKey::KEY_RIGHT_ALT) {
        mods |= GHOSTTY_MODS_ALT as u16;
    }
    if rl.is_key_down(KeyboardKey::KEY_LEFT_SUPER) || rl.is_key_down(KeyboardKey::KEY_RIGHT_SUPER) {
        mods |= GHOSTTY_MODS_SUPER as u16;
    }
    mods
}

fn raylib_key_unshifted_codepoint(rl_key: KeyboardKey) -> u32 {
    let k = rl_key as i32;
    if k >= KeyboardKey::KEY_A as i32 && k <= KeyboardKey::KEY_Z as i32 {
        return b'a' as u32 + (k - KeyboardKey::KEY_A as i32) as u32;
    }
    if k >= KeyboardKey::KEY_ZERO as i32 && k <= KeyboardKey::KEY_NINE as i32 {
        return b'0' as u32 + (k - KeyboardKey::KEY_ZERO as i32) as u32;
    }
    match rl_key {
        KeyboardKey::KEY_SPACE => b' ' as u32,
        KeyboardKey::KEY_MINUS => b'-' as u32,
        KeyboardKey::KEY_EQUAL => b'=' as u32,
        KeyboardKey::KEY_LEFT_BRACKET => b'[' as u32,
        KeyboardKey::KEY_RIGHT_BRACKET => b']' as u32,
        KeyboardKey::KEY_BACKSLASH => b'\\' as u32,
        KeyboardKey::KEY_SEMICOLON => b';' as u32,
        KeyboardKey::KEY_APOSTROPHE => b'\'' as u32,
        KeyboardKey::KEY_COMMA => b',' as u32,
        KeyboardKey::KEY_PERIOD => b'.' as u32,
        KeyboardKey::KEY_SLASH => b'/' as u32,
        KeyboardKey::KEY_GRAVE => b'`' as u32,
        _ => 0,
    }
}

fn utf8_encode(cp: u32) -> Vec<u8> {
    let mut buf = [0u8; 4];
    let c = char::from_u32(cp).unwrap_or('\u{FFFD}');
    let s = c.encode_utf8(&mut buf);
    s.as_bytes().to_vec()
}

fn raylib_mouse_to_ghostty(btn: MouseButton) -> u32 {
    match btn {
        MouseButton::MOUSE_BUTTON_LEFT => GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_LEFT,
        MouseButton::MOUSE_BUTTON_RIGHT => GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_RIGHT,
        MouseButton::MOUSE_BUTTON_MIDDLE => GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_MIDDLE,
        MouseButton::MOUSE_BUTTON_SIDE => GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FOUR,
        MouseButton::MOUSE_BUTTON_EXTRA => GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FIVE,
        MouseButton::MOUSE_BUTTON_FORWARD => GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SIX,
        MouseButton::MOUSE_BUTTON_BACK => GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SEVEN,
    }
}

pub fn handle_input(rl: &RaylibHandle, terminal: &mut Terminal, pty: &Pty) -> Vec<char> {
    unsafe {
        ghostty_key_encoder_setopt_from_terminal(terminal.key_encoder(), terminal.handle());
    }

    let mut char_utf8: Vec<u8> = Vec::new();
    let mut typed_chars: Vec<char> = Vec::new();
    loop {
        let ch = unsafe { raylib::ffi::GetCharPressed() };
        if ch == 0 {
            break;
        }
        if let Some(c) = char::from_u32(ch as u32) {
            typed_chars.push(c);
        }
        char_utf8.extend(utf8_encode(ch as u32));
    }

    let special_keys: &[KeyboardKey] = &[
        KeyboardKey::KEY_SPACE,
        KeyboardKey::KEY_ENTER,
        KeyboardKey::KEY_TAB,
        KeyboardKey::KEY_BACKSPACE,
        KeyboardKey::KEY_DELETE,
        KeyboardKey::KEY_ESCAPE,
        KeyboardKey::KEY_UP,
        KeyboardKey::KEY_DOWN,
        KeyboardKey::KEY_LEFT,
        KeyboardKey::KEY_RIGHT,
        KeyboardKey::KEY_HOME,
        KeyboardKey::KEY_END,
        KeyboardKey::KEY_PAGE_UP,
        KeyboardKey::KEY_PAGE_DOWN,
        KeyboardKey::KEY_INSERT,
        KeyboardKey::KEY_MINUS,
        KeyboardKey::KEY_EQUAL,
        KeyboardKey::KEY_LEFT_BRACKET,
        KeyboardKey::KEY_RIGHT_BRACKET,
        KeyboardKey::KEY_BACKSLASH,
        KeyboardKey::KEY_SEMICOLON,
        KeyboardKey::KEY_APOSTROPHE,
        KeyboardKey::KEY_COMMA,
        KeyboardKey::KEY_PERIOD,
        KeyboardKey::KEY_SLASH,
        KeyboardKey::KEY_GRAVE,
        KeyboardKey::KEY_F1,
        KeyboardKey::KEY_F2,
        KeyboardKey::KEY_F3,
        KeyboardKey::KEY_F4,
        KeyboardKey::KEY_F5,
        KeyboardKey::KEY_F6,
        KeyboardKey::KEY_F7,
        KeyboardKey::KEY_F8,
        KeyboardKey::KEY_F9,
        KeyboardKey::KEY_F10,
        KeyboardKey::KEY_F11,
        KeyboardKey::KEY_F12,
    ];

    let mut keys_to_check = Vec::with_capacity(72);
    for k in (KeyboardKey::KEY_A as i32)..=(KeyboardKey::KEY_Z as i32) {
        keys_to_check.push(unsafe { std::mem::transmute(k) });
    }
    for k in (KeyboardKey::KEY_ZERO as i32)..=(KeyboardKey::KEY_NINE as i32) {
        keys_to_check.push(unsafe { std::mem::transmute(k) });
    }
    keys_to_check.extend_from_slice(special_keys);

    let mods = get_ghostty_mods(rl);

    for &rl_key in &keys_to_check {
        let pressed = rl.is_key_pressed(rl_key);
        let repeated = unsafe { raylib::ffi::IsKeyPressedRepeat(rl_key as i32) };
        let released = rl.is_key_released(rl_key);
        if !pressed && !repeated && !released {
            continue;
        }

        let gkey = raylib_key_to_ghostty(rl_key);
        if gkey == GhosttyKey_GHOSTTY_KEY_UNIDENTIFIED {
            continue;
        }

        let action = if released {
            GhosttyKeyAction_GHOSTTY_KEY_ACTION_RELEASE
        } else if pressed {
            GhosttyKeyAction_GHOSTTY_KEY_ACTION_PRESS
        } else {
            GhosttyKeyAction_GHOSTTY_KEY_ACTION_REPEAT
        };

        unsafe {
            ghostty_key_event_set_key(terminal.key_event(), gkey);
            ghostty_key_event_set_action(terminal.key_event(), action);
            ghostty_key_event_set_mods(terminal.key_event(), mods);

            let ucp = raylib_key_unshifted_codepoint(rl_key);
            ghostty_key_event_set_unshifted_codepoint(terminal.key_event(), ucp);

            let mut consumed: u16 = 0;
            if ucp != 0 && (mods & GHOSTTY_MODS_SHIFT as u16) != 0 {
                consumed |= GHOSTTY_MODS_SHIFT as u16;
            }
            ghostty_key_event_set_consumed_mods(terminal.key_event(), consumed);

            if !char_utf8.is_empty() && !released {
                ghostty_key_event_set_utf8(
                    terminal.key_event(),
                    char_utf8.as_ptr() as *const i8,
                    char_utf8.len(),
                );
                char_utf8.clear();
            } else {
                ghostty_key_event_set_utf8(terminal.key_event(), std::ptr::null(), 0);
            }

            let mut buf = [0u8; 128];
            let mut written: usize = 0;
            let res = ghostty_key_encoder_encode(
                terminal.key_encoder(),
                terminal.key_event(),
                buf.as_mut_ptr() as *mut i8,
                buf.len(),
                &mut written,
            );
            if res == GhosttyResult_GHOSTTY_SUCCESS && written > 0 {
                pty.write(&buf[..written]);
                char_utf8.clear();
            }
        }
    }

    if !char_utf8.is_empty() {
        pty.write(&char_utf8);
    }

    typed_chars
}

pub fn handle_mouse(
    rl: &RaylibHandle,
    terminal: &mut Terminal,
    pty: &Pty,
    cell_width: i32,
    cell_height: i32,
    pad_x: i32,
    pad_y: i32,
    padding_right: i32,
    panel_w: i32,
    panel_h: i32,
    panel_offset_x: i32,
    panel_offset_y: i32,
) {
    unsafe {
        ghostty_mouse_encoder_setopt_from_terminal(terminal.mouse_encoder(), terminal.handle());

        let mut enc_size: GhosttyMouseEncoderSize = std::mem::zeroed();
        enc_size.size = std::mem::size_of::<GhosttyMouseEncoderSize>();
        enc_size.screen_width = panel_w as u32;
        enc_size.screen_height = panel_h as u32;
        enc_size.cell_width = cell_width as u32;
        enc_size.cell_height = cell_height as u32;
        enc_size.padding_top = pad_y as u32;
        enc_size.padding_bottom = pad_x as u32;
        enc_size.padding_left = pad_x as u32;
        enc_size.padding_right = padding_right as u32;
        ghostty_mouse_encoder_setopt(
            terminal.mouse_encoder(),
            GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
            &enc_size as *const _ as *const std::ffi::c_void,
        );

        let any_pressed = rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT)
            || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT)
            || rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE);
        ghostty_mouse_encoder_setopt(
            terminal.mouse_encoder(),
            GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
            &any_pressed as *const bool as *const std::ffi::c_void,
        );

        let track_cell = true;
        ghostty_mouse_encoder_setopt(
            terminal.mouse_encoder(),
            GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL,
            &track_cell as *const bool as *const std::ffi::c_void,
        );

        let mods = get_ghostty_mods(rl);
        let pos = rl.get_mouse_position();
        ghostty_mouse_event_set_mods(terminal.mouse_event(), mods);
        ghostty_mouse_event_set_position(
            terminal.mouse_event(),
            GhosttyMousePosition {
                x: pos.x - panel_offset_x as f32,
                y: pos.y - panel_offset_y as f32,
            },
        );

        let buttons = [
            MouseButton::MOUSE_BUTTON_LEFT,
            MouseButton::MOUSE_BUTTON_RIGHT,
            MouseButton::MOUSE_BUTTON_MIDDLE,
            MouseButton::MOUSE_BUTTON_SIDE,
            MouseButton::MOUSE_BUTTON_EXTRA,
            MouseButton::MOUSE_BUTTON_FORWARD,
            MouseButton::MOUSE_BUTTON_BACK,
        ];

        for &btn in &buttons {
            let gbtn = raylib_mouse_to_ghostty(btn);
            if gbtn == GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_UNKNOWN {
                continue;
            }

            if rl.is_mouse_button_pressed(btn) {
                ghostty_mouse_event_set_action(
                    terminal.mouse_event(),
                    GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_PRESS,
                );
                ghostty_mouse_event_set_button(terminal.mouse_event(), gbtn);
                mouse_encode_and_write(pty, terminal);
            } else if rl.is_mouse_button_released(btn) {
                ghostty_mouse_event_set_action(
                    terminal.mouse_event(),
                    GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_RELEASE,
                );
                ghostty_mouse_event_set_button(terminal.mouse_event(), gbtn);
                mouse_encode_and_write(pty, terminal);
            }
        }

        let delta = rl.get_mouse_delta();
        if delta.x != 0.0 || delta.y != 0.0 {
            ghostty_mouse_event_set_action(
                terminal.mouse_event(),
                GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_MOTION,
            );
            if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_LEFT) {
                ghostty_mouse_event_set_button(
                    terminal.mouse_event(),
                    GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_LEFT,
                );
            } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_RIGHT) {
                ghostty_mouse_event_set_button(
                    terminal.mouse_event(),
                    GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_RIGHT,
                );
            } else if rl.is_mouse_button_down(MouseButton::MOUSE_BUTTON_MIDDLE) {
                ghostty_mouse_event_set_button(
                    terminal.mouse_event(),
                    GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_MIDDLE,
                );
            } else {
                ghostty_mouse_event_clear_button(terminal.mouse_event());
            }
            mouse_encode_and_write(pty, terminal);
        }

        let wheel = rl.get_mouse_wheel_move();
        if wheel != 0.0 {
            let mut mouse_tracking = false;
            ghostty_terminal_get(
                terminal.handle(),
                GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
                &mut mouse_tracking as *mut bool as *mut std::ffi::c_void,
            );

            if mouse_tracking {
                let scroll_btn = if wheel > 0.0 {
                    GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FOUR
                } else {
                    GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FIVE
                };
                ghostty_mouse_event_set_button(terminal.mouse_event(), scroll_btn);
                ghostty_mouse_event_set_action(
                    terminal.mouse_event(),
                    GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_PRESS,
                );
                mouse_encode_and_write(pty, terminal);
                ghostty_mouse_event_set_action(
                    terminal.mouse_event(),
                    GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_RELEASE,
                );
                mouse_encode_and_write(pty, terminal);
            } else {
                let delta = if wheel > 0.0 { -3 } else { 3 };
                terminal.scroll_viewport(delta);
            }
        }
    }
}

fn mouse_encode_and_write(pty: &Pty, terminal: &Terminal) {
    unsafe {
        let mut buf = [0u8; 128];
        let mut written: usize = 0;
        let res = ghostty_mouse_encoder_encode(
            terminal.mouse_encoder(),
            terminal.mouse_event(),
            buf.as_mut_ptr() as *mut i8,
            buf.len(),
            &mut written,
        );
        if res == GhosttyResult_GHOSTTY_SUCCESS && written > 0 {
            pty.write(&buf[..written]);
        }
    }
}
