use raylib::prelude::*;

pub enum TabBarAction {
    None,
    SwitchTo(usize),
    Close(usize),
    New,
}

pub struct TabBar {
    pub height: i32,
}

impl TabBar {
    pub fn new(cell_height: i32) -> Self {
        TabBar {
            height: cell_height + 14,
        }
    }

    pub fn update_height(&mut self, cell_height: i32) {
        self.height = cell_height + 14;
    }

    pub fn handle_click(&self, mx: i32, _my: i32, screen_w: i32, tab_count: usize) -> TabBarAction {
        let tab_w = self.tab_width(tab_count, screen_w);
        let new_btn_x = (tab_count as i32) * tab_w;

        if mx >= new_btn_x && mx < new_btn_x + self.height {
            return TabBarAction::New;
        }

        let tab_idx = (mx / tab_w) as usize;
        if tab_idx >= tab_count {
            return TabBarAction::None;
        }

        let close_x = (tab_idx as i32 + 1) * tab_w - self.height;
        if mx >= close_x {
            return TabBarAction::Close(tab_idx);
        }

        TabBarAction::SwitchTo(tab_idx)
    }

    fn tab_width(&self, tab_count: usize, screen_w: i32) -> i32 {
        let max_tab_w = 280;
        let btn_zone = self.height + 8;
        let available = screen_w - btn_zone;
        let per_tab = if tab_count > 0 {
            available / tab_count as i32
        } else {
            available
        };
        per_tab.min(max_tab_w).max(80)
    }

    pub fn render(
        &self,
        titles: &[String],
        active: usize,
        font: &raylib::ffi::Font,
        font_size: i32,
        screen_w: i32,
        d: &mut RaylibDrawHandle,
    ) {
        let h = self.height;
        d.draw_rectangle(0, 0, screen_w, h, Color::new(22, 22, 28, 255));
        d.draw_rectangle(0, h - 1, screen_w, 1, Color::new(45, 45, 55, 255));

        let tab_count = titles.len();
        let tab_w = self.tab_width(tab_count, screen_w);
        let label_size = (font_size - 2).max(8);

        for (i, title) in titles.iter().enumerate() {
            let x = i as i32 * tab_w;
            let is_active = i == active;

            let bg = if is_active {
                Color::new(38, 38, 48, 255)
            } else {
                Color::new(26, 26, 32, 255)
            };
            d.draw_rectangle(x, 0, tab_w, h, bg);

            if is_active {
                d.draw_rectangle(x, h - 2, tab_w, 2, Color::new(80, 140, 220, 255));
            }

            d.draw_rectangle(x + tab_w - 1, 2, 1, h - 4, Color::new(45, 45, 55, 200));

            let max_chars = ((tab_w - self.height - 14) / (label_size / 2 + 1)).max(1) as usize;
            let label: String = if title.len() > max_chars {
                format!("{}...", &title[..max_chars.saturating_sub(3)])
            } else {
                title.clone()
            };

            let text_color = if is_active {
                raylib::ffi::Color { r: 220, g: 225, b: 240, a: 255 }
            } else {
                raylib::ffi::Color { r: 130, g: 135, b: 150, a: 255 }
            };

            let c_label = std::ffi::CString::new(label.as_str()).unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    *font,
                    c_label.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (x + 14) as f32,
                        y: ((h - label_size) / 2) as f32,
                    },
                    label_size as f32,
                    0.0,
                    text_color,
                );
            }

            let close_x = x + tab_w - self.height;
            let close_y = (h - label_size) / 2;
            let close_color = if is_active {
                raylib::ffi::Color { r: 160, g: 160, b: 170, a: 180 }
            } else {
                raylib::ffi::Color { r: 100, g: 100, b: 110, a: 120 }
            };
            let c_close = std::ffi::CString::new("x").unwrap_or_default();
            unsafe {
                raylib::ffi::DrawTextEx(
                    *font,
                    c_close.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (close_x + 4) as f32,
                        y: close_y as f32,
                    },
                    label_size as f32,
                    0.0,
                    close_color,
                );
            }
        }

        let plus_x = tab_count as i32 * tab_w;
        let c_plus = std::ffi::CString::new("+").unwrap_or_default();
        unsafe {
            raylib::ffi::DrawTextEx(
                *font,
                c_plus.as_ptr(),
                raylib::ffi::Vector2 {
                    x: (plus_x + 10) as f32,
                    y: ((h - label_size) / 2) as f32,
                },
                label_size as f32,
                0.0,
                raylib::ffi::Color { r: 120, g: 130, b: 160, a: 200 },
            );
        }
    }
}
