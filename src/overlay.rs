use raylib::prelude::*;

pub struct CommandOverlay {
    visible: bool,
    command: String,
    explanation: String,
}

impl CommandOverlay {
    pub fn new() -> Self {
        CommandOverlay {
            visible: false,
            command: String::new(),
            explanation: String::new(),
        }
    }

    pub fn show(&mut self, command: &str, explanation: &str) {
        self.visible = true;
        self.command = command.to_string();
        self.explanation = explanation.to_string();
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.command.clear();
        self.explanation.clear();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn render(&self, font: &raylib::ffi::Font, screen_w: i32, screen_h: i32, font_size: i32, d: &mut RaylibDrawHandle) {
        if !self.visible {
            return;
        }

        let status_bar_h = font_size + 8;
        let box_h = font_size * 5;
        let box_y = screen_h - box_h - status_bar_h - 10;
        let box_x = 20;
        let box_w = screen_w - 40;

        d.draw_rectangle(box_x - 2, box_y - 2, box_w + 4, box_h + 4, Color::new(80, 60, 10, 200));
        d.draw_rectangle(box_x, box_y, box_w, box_h, Color::new(25, 25, 35, 240));
        d.draw_rectangle_lines(box_x, box_y, box_w, box_h, Color::new(120, 100, 50, 255));

        let cmd_text = format!("  run_command: {}", self.command);
        let cmd_c = std::ffi::CString::new(cmd_text.as_str()).unwrap_or_default();
        unsafe {
            raylib::ffi::DrawTextEx(
                *font,
                cmd_c.as_ptr(),
                raylib::ffi::Vector2 { x: (box_x + 10) as f32, y: (box_y + 8) as f32 },
                font_size as f32,
                0.0,
                raylib::ffi::Color { r: 0, g: 200, b: 200, a: 255 },
            );
        }

        if !self.explanation.is_empty() {
            let exp_text = format!("  ({})", self.explanation);
            let exp_c = std::ffi::CString::new(exp_text.as_str()).unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    *font,
                    exp_c.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (box_x + 10) as f32,
                        y: (box_y + 8 + font_size) as f32,
                    },
                    font_size as f32,
                    0.0,
                    raylib::ffi::Color { r: 180, g: 180, b: 180, a: 255 },
                );
            }
        }

        let hint = "  [Enter] Run  |  [Esc] Cancel  |  [e] Edit";
        let hint_c = std::ffi::CString::new(hint).unwrap_or_default();
        unsafe {
            raylib::ffi::DrawTextEx(
                *font,
                hint_c.as_ptr(),
                raylib::ffi::Vector2 {
                    x: (box_x + 10) as f32,
                    y: (box_y + box_h - font_size - 8) as f32,
                },
                font_size as f32,
                0.0,
                raylib::ffi::Color { r: 200, g: 200, b: 0, a: 255 },
            );
        }
    }
}
