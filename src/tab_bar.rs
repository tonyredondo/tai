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

    pub fn handle_click(&self, local_mx: i32, _local_my: i32, bar_width: i32, tab_count: usize) -> TabBarAction {
        let tab_w = self.tab_width(tab_count, bar_width);
        let new_btn_x = (tab_count as i32) * tab_w;

        if local_mx >= new_btn_x && local_mx < new_btn_x + self.height {
            return TabBarAction::New;
        }

        let tab_idx = (local_mx / tab_w) as usize;
        if tab_idx >= tab_count {
            return TabBarAction::None;
        }

        let close_x = (tab_idx as i32 + 1) * tab_w - self.height;
        if local_mx >= close_x {
            return TabBarAction::Close(tab_idx);
        }

        TabBarAction::SwitchTo(tab_idx)
    }

    fn tab_width(&self, tab_count: usize, bar_width: i32) -> i32 {
        let max_tab_w = 280;
        let btn_zone = self.height + 8;
        let available = bar_width - btn_zone;
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
        offset_x: i32,
        offset_y: i32,
        bar_width: i32,
        d: &mut RaylibDrawHandle,
    ) {
        let h = self.height;
        let accent = Color::new(58, 62, 78, 255);
        let border_line = Color::new(58, 62, 78, 255);

        d.draw_rectangle(offset_x, offset_y, bar_width, h, Color::new(22, 22, 28, 255));

        let tab_count = titles.len();
        let left_pad = 4;
        let tab_w = self.tab_width(tab_count, bar_width - left_pad);
        let label_size = (font_size - 2).max(8);

        let top_pad = 2;
        let tab_y = offset_y + top_pad;
        let tab_h = h - top_pad;
        let tabs_x = offset_x + left_pad;
        let active_x = tabs_x + active as i32 * tab_w;

        for (i, title) in titles.iter().enumerate() {
            let x = tabs_x + i as i32 * tab_w;
            let is_active = i == active;

            if is_active {
                let radius = 8;
                let tab_bg = Color::new(26, 27, 30, 255);
                let extended_h = tab_h + radius * 2;
                let roundness = (radius as f32 * 2.0) / (extended_h as f32).min(tab_w as f32);
                let rect = raylib::ffi::Rectangle {
                    x: x as f32,
                    y: tab_y as f32,
                    width: tab_w as f32,
                    height: extended_h as f32,
                };
                unsafe {
                    raylib::ffi::BeginScissorMode(x, tab_y, tab_w, tab_h);
                    raylib::ffi::DrawRectangleRounded(rect, roundness, 12, accent.into());
                    let inner_rect = raylib::ffi::Rectangle {
                        x: (x + 1) as f32,
                        y: (tab_y + 1) as f32,
                        width: (tab_w - 2) as f32,
                        height: (extended_h - 1) as f32,
                    };
                    raylib::ffi::DrawRectangleRounded(inner_rect, roundness, 12, tab_bg.into());
                    raylib::ffi::EndScissorMode();
                }
            } else {
                d.draw_rectangle(x + tab_w - 1, tab_y + 4, 1, tab_h - 8, Color::new(45, 45, 55, 150));
            }

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
                        y: (tab_y + (tab_h - label_size) / 2) as f32,
                    },
                    label_size as f32,
                    0.0,
                    text_color,
                );
            }

            let close_x = x + tab_w - self.height;
            let close_y = tab_y + (tab_h - label_size) / 2;
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

        // Bottom border line with gap for active tab
        let bottom_y = offset_y + h - 1;
        if active_x > offset_x {
            d.draw_rectangle(offset_x, bottom_y, active_x - offset_x, 1, border_line);
        }
        let active_end = active_x + tab_w;
        let bar_end = offset_x + bar_width;
        if active_end < bar_end {
            d.draw_rectangle(active_end, bottom_y, bar_end - active_end, 1, border_line);
        }

        let plus_x = tabs_x + tab_count as i32 * tab_w;
        let c_plus = std::ffi::CString::new("+").unwrap_or_default();
        unsafe {
            raylib::ffi::DrawTextEx(
                *font,
                c_plus.as_ptr(),
                raylib::ffi::Vector2 {
                    x: (plus_x + 10) as f32,
                    y: (tab_y + (tab_h - label_size) / 2) as f32,
                },
                label_size as f32,
                0.0,
                raylib::ffi::Color { r: 120, g: 130, b: 160, a: 200 },
            );
        }
    }
}
