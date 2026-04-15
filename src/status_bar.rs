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
        sidebar_w: i32,
        mode: InputMode,
        cwd: &str,
        _ai_input: &str,
        auto_execute: bool,
        panel_info: Option<(usize, usize, usize, usize)>,
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

        d.draw_rectangle(sidebar_w, bar_y, screen_w - sidebar_w, bar_h, bg_color);

        let tab_prefix = match panel_info {
            Some((panel_idx, panel_count, tab_idx, tab_count)) => {
                if panel_count > 1 {
                    format!("Panel {}/{} | Tab {}/{} | ", panel_idx, panel_count, tab_idx, tab_count)
                } else if tab_count > 1 {
                    format!("Tab {}/{} | ", tab_idx, tab_count)
                } else {
                    String::new()
                }
            }
            None => String::new(),
        };

        let text = match mode {
            InputMode::Shell => {
                let yolo = if auto_execute { " | YOLO" } else { "" };
                if self.ai_available {
                    format!(
                        " {}shell | model: {} | cwd: {} | Ctrl+/ AI | F1 help{}",
                        tab_prefix, self.model, cwd, yolo
                    )
                } else {
                    format!(
                        " {}shell | cwd: {} | Set OPENAI_API_KEY to enable AI",
                        tab_prefix, cwd
                    )
                }
            }
            InputMode::AiPrompt => {
                format!(" {}AI mode | Enter to submit | Ctrl+J newline | Esc cancel", tab_prefix)
            }
            InputMode::AiStreaming => {
                format!(" {}AI responding...  (Esc to cancel)", tab_prefix)
            }
            InputMode::CommandConfirm => {
                format!(" {}Command confirmation  [Enter] Run | [Esc] Cancel | [e] Edit", tab_prefix)
            }
        };

        let c_text = std::ffi::CString::new(text.as_str()).unwrap_or_default();
        unsafe {
            raylib::ffi::DrawTextEx(
                *font,
                c_text.as_ptr(),
                raylib::ffi::Vector2 {
                    x: sidebar_w as f32,
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
            d.draw_rectangle(sidebar_w, accent_y, screen_w - sidebar_w, 2, Color::new(80, 160, 255, 255));
        }
    }
}
