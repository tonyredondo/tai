use crate::router::InputMode;
use raylib::prelude::*;

pub struct StatusBar {
    pub model: String,
    pub ai_available: bool,
}

impl StatusBar {
    pub fn new(model: &str, ai_available: bool) -> Self {
        StatusBar {
            model: model.to_string(),
            ai_available,
        }
    }

    pub fn render(
        &self,
        font: &raylib::ffi::Font,
        screen_w: i32,
        screen_h: i32,
        font_size: i32,
        mode: InputMode,
        cwd: &str,
        ai_input: &str,
        auto_execute: bool,
        d: &mut RaylibDrawHandle,
    ) {
        let bar_h = font_size + 8;
        let bar_y = screen_h - bar_h;

        let (bg_color, text_color) = match mode {
            InputMode::Shell => (
                Color::new(30, 30, 50, 255),
                Color::new(180, 180, 180, 255),
            ),
            InputMode::AiPrompt => (
                Color::new(20, 50, 100, 255),
                Color::new(100, 220, 255, 255),
            ),
            InputMode::AiStreaming => (
                Color::new(50, 30, 80, 255),
                Color::new(200, 150, 255, 255),
            ),
            InputMode::CommandConfirm => (
                Color::new(80, 60, 10, 255),
                Color::new(255, 220, 100, 255),
            ),
        };

        d.draw_rectangle(0, bar_y, screen_w, bar_h, bg_color);

        let text = match mode {
            InputMode::Shell => {
                let yolo = if auto_execute { " | YOLO" } else { "" };
                if self.ai_available {
                    format!(
                        " [TAI] shell | model: {} | cwd: {} | Ctrl+/ for AI | Ctrl+Y YOLO{}",
                        self.model, cwd, yolo
                    )
                } else {
                    format!(
                        " [TAI] shell | cwd: {} | Set OPENAI_API_KEY to enable AI",
                        cwd
                    )
                }
            }
            InputMode::AiPrompt => {
                " [TAI] AI mode | Enter to submit | Ctrl+J for newline | Esc to cancel".to_string()
            }
            InputMode::AiStreaming => {
                " [TAI] AI responding...  (Esc to cancel)".to_string()
            }
            InputMode::CommandConfirm => {
                " [TAI] Command confirmation  [Enter] Run | [Esc] Cancel | [e] Edit".to_string()
            }
        };

        let c_text = std::ffi::CString::new(text.as_str()).unwrap_or_default();
        unsafe {
            raylib::ffi::DrawTextEx(
                *font,
                c_text.as_ptr(),
                raylib::ffi::Vector2 {
                    x: 0.0,
                    y: bar_y as f32 + 4.0,
                },
                font_size as f32,
                0.0,
                raylib::ffi::Color {
                    r: text_color.r,
                    g: text_color.g,
                    b: text_color.b,
                    a: text_color.a,
                },
            );
        }

        if mode == InputMode::AiPrompt {
            let accent_y = bar_y;
            d.draw_rectangle(0, accent_y, screen_w, 2, Color::new(80, 160, 255, 255));
        }
    }
}
