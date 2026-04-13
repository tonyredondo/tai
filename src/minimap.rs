use raylib::prelude::*;

struct LineInfo {
    r: u8,
    g: u8,
    b: u8,
    density: f32,
}

struct PixelInfo {
    density: f32,
    alpha: u8,
}

#[derive(PartialEq)]
enum ParserState {
    Normal,
    EscSeen,
    CsiParam,
}

pub struct Minimap {
    lines: Vec<LineInfo>,
    pixel_cache: Vec<PixelInfo>,
    pixel_cache_dirty: bool,
    last_cache_line_count: usize,
    last_cache_height: i32,

    current_fg: (u8, u8, u8),
    line_fg: (u8, u8, u8),
    line_has_color: bool,
    current_line_chars: u32,
    max_line_chars: u32,
    state: ParserState,
    escape_buf: Vec<u8>,
    in_alt_screen: bool,
    cols: u16,

    pub width: i32,
    pub dragging: bool,
    drag_offset: f64,
}

const DEFAULT_FG: (u8, u8, u8) = (200, 200, 200);

const PALETTE_16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 49, 49),
    (13, 188, 121),
    (229, 229, 16),
    (36, 114, 200),
    (188, 63, 188),
    (17, 168, 205),
    (229, 229, 229),
    (102, 102, 102),
    (241, 76, 76),
    (35, 209, 139),
    (245, 245, 67),
    (59, 142, 234),
    (214, 112, 214),
    (41, 184, 219),
    (255, 255, 255),
];

fn palette_256(idx: u8) -> (u8, u8, u8) {
    if idx < 16 {
        return PALETTE_16[idx as usize];
    }
    if idx < 232 {
        let i = idx - 16;
        let r = (i / 36) % 6;
        let g = (i / 6) % 6;
        let b = i % 6;
        let to_val = |v: u8| if v == 0 { 0u8 } else { 55 + 40 * v };
        return (to_val(r), to_val(g), to_val(b));
    }
    let gray = 8 + 10 * (idx - 232);
    (gray, gray, gray)
}

impl Minimap {
    pub fn new(cols: u16) -> Self {
        Minimap {
            lines: Vec::with_capacity(4096),
            pixel_cache: Vec::new(),
            pixel_cache_dirty: true,
            last_cache_line_count: 0,
            last_cache_height: 0,
            current_fg: DEFAULT_FG,
            line_fg: DEFAULT_FG,
            line_has_color: false,
            current_line_chars: 0,
            max_line_chars: 0,
            state: ParserState::Normal,
            escape_buf: Vec::with_capacity(32),
            in_alt_screen: false,
            cols,
            width: 40,
            dragging: false,
            drag_offset: 0.0,
        }
    }

    pub fn set_cols(&mut self, cols: u16) {
        self.cols = cols;
    }

    pub fn rebuild_from_text(&mut self, text: &str) {
        self.lines.clear();
        self.pixel_cache.clear();
        self.pixel_cache_dirty = true;
        self.last_cache_line_count = 0;
        self.current_line_chars = 0;
        self.max_line_chars = 0;
        self.current_fg = DEFAULT_FG;
        self.line_fg = DEFAULT_FG;
        self.line_has_color = false;
        self.state = ParserState::Normal;
        self.escape_buf.clear();

        let cols = self.cols as u32;
        if cols == 0 {
            return;
        }

        for line in text.split('\n') {
            let char_count = line.chars().count() as u32;
            if char_count == 0 {
                self.lines.push(LineInfo {
                    r: DEFAULT_FG.0,
                    g: DEFAULT_FG.1,
                    b: DEFAULT_FG.2,
                    density: 0.0,
                });
                continue;
            }

            if char_count > cols {
                let full_rows = char_count / cols;
                let remainder = char_count % cols;
                for _ in 0..full_rows {
                    self.lines.push(LineInfo {
                        r: DEFAULT_FG.0,
                        g: DEFAULT_FG.1,
                        b: DEFAULT_FG.2,
                        density: 1.0,
                    });
                }
                if remainder > 0 {
                    self.lines.push(LineInfo {
                        r: DEFAULT_FG.0,
                        g: DEFAULT_FG.1,
                        b: DEFAULT_FG.2,
                        density: remainder as f32 / cols as f32,
                    });
                }
            } else {
                self.lines.push(LineInfo {
                    r: DEFAULT_FG.0,
                    g: DEFAULT_FG.1,
                    b: DEFAULT_FG.2,
                    density: (char_count as f32 / cols as f32).min(1.0),
                });
            }
        }

        self.pixel_cache_dirty = true;
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.state {
                ParserState::Normal => {
                    if b == 0x1b {
                        self.state = ParserState::EscSeen;
                    } else if b == b'\n' {
                        self.commit_line();
                    } else if b == b'\r' {
                        if self.current_line_chars > self.max_line_chars {
                            self.max_line_chars = self.current_line_chars;
                        }
                        self.current_line_chars = 0;
                    } else if b >= 0x20 && b != 0x7F {
                        self.current_line_chars += 1;
                    }
                }
                ParserState::EscSeen => {
                    if b == b'[' {
                        self.escape_buf.clear();
                        self.state = ParserState::CsiParam;
                    } else {
                        self.state = ParserState::Normal;
                    }
                }
                ParserState::CsiParam => {
                    if b >= 0x40 && b <= 0x7E {
                        if b == b'm' {
                            self.parse_sgr();
                        } else if b == b'h' || b == b'l' {
                            self.parse_mode_set(b);
                        }
                        self.state = ParserState::Normal;
                    } else {
                        self.escape_buf.push(b);
                    }
                }
            }
        }
    }

    fn commit_line(&mut self) {
        if self.in_alt_screen {
            self.current_line_chars = 0;
            self.max_line_chars = 0;
            self.line_has_color = false;
            return;
        }
        let chars = self.current_line_chars.max(self.max_line_chars);
        let cols = self.cols as u32;
        let fg = if self.line_has_color {
            self.line_fg
        } else {
            self.current_fg
        };

        if cols > 0 && chars > cols {
            let full_rows = chars / cols;
            let remainder = chars % cols;
            for _ in 0..full_rows {
                self.lines.push(LineInfo {
                    r: fg.0,
                    g: fg.1,
                    b: fg.2,
                    density: 1.0,
                });
            }
            if remainder > 0 {
                self.lines.push(LineInfo {
                    r: fg.0,
                    g: fg.1,
                    b: fg.2,
                    density: remainder as f32 / cols as f32,
                });
            }
        } else {
            let density = if cols > 0 {
                (chars as f32 / cols as f32).min(1.0)
            } else {
                0.0
            };
            self.lines.push(LineInfo {
                r: fg.0,
                g: fg.1,
                b: fg.2,
                density,
            });
        }

        self.current_line_chars = 0;
        self.max_line_chars = 0;
        self.line_has_color = false;
        self.pixel_cache_dirty = true;
    }

    fn set_fg(&mut self, color: (u8, u8, u8)) {
        self.current_fg = color;
        if color != DEFAULT_FG {
            self.line_fg = color;
            self.line_has_color = true;
        }
    }

    fn parse_sgr(&mut self) {
        let buf_copy = self.escape_buf.clone();
        let buf = std::str::from_utf8(&buf_copy).unwrap_or("");
        if buf.is_empty() {
            self.current_fg = DEFAULT_FG;
            return;
        }
        let params: Vec<&str> = buf.split(';').collect();
        let mut i = 0;
        while i < params.len() {
            let n: u16 = params[i].parse().unwrap_or(0);
            match n {
                0 => self.current_fg = DEFAULT_FG,
                30..=37 => {
                    self.set_fg(PALETTE_16[(n - 30) as usize]);
                }
                39 => self.current_fg = DEFAULT_FG,
                90..=97 => {
                    self.set_fg(PALETTE_16[(n - 90 + 8) as usize]);
                }
                38 => {
                    if i + 1 < params.len() {
                        let mode: u16 = params[i + 1].parse().unwrap_or(0);
                        if mode == 5 && i + 2 < params.len() {
                            let idx: u8 = params[i + 2].parse().unwrap_or(0);
                            self.set_fg(palette_256(idx));
                            i += 2;
                        } else if mode == 2 && i + 4 < params.len() {
                            let r: u8 = params[i + 2].parse().unwrap_or(0);
                            let g: u8 = params[i + 3].parse().unwrap_or(0);
                            let b: u8 = params[i + 4].parse().unwrap_or(0);
                            self.set_fg((r, g, b));
                            i += 4;
                        } else {
                            i += 1;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn parse_mode_set(&mut self, terminator: u8) {
        let buf = std::str::from_utf8(&self.escape_buf).unwrap_or("");
        if buf.starts_with('?') {
            let num_str = &buf[1..];
            let nums: Vec<u16> = num_str
                .split(';')
                .filter_map(|s| s.parse().ok())
                .collect();
            if nums.contains(&1049) {
                self.in_alt_screen = terminator == b'h';
            }
        }
    }

    fn rebuild_pixel_cache(&mut self, minimap_height: i32) {
        let row_h = 2;
        let rows = (minimap_height.max(1) / row_h) as usize;
        self.pixel_cache.clear();
        self.pixel_cache.reserve(rows);

        let line_count = self.lines.len();
        if line_count == 0 {
            for _ in 0..rows {
                self.pixel_cache.push(PixelInfo {
                    density: 0.0,
                    alpha: 0,
                });
            }
            self.pixel_cache_dirty = false;
            self.last_cache_line_count = 0;
            self.last_cache_height = minimap_height;
            return;
        }

        for py in 0..rows {
            let start = (py * line_count) / rows;
            let end = ((py + 1) * line_count) / rows;
            let end = end.max(start + 1).min(line_count);

            let mut max_density: f32 = 0.0;
            for li in &self.lines[start..end] {
                if li.density > max_density {
                    max_density = li.density;
                }
            }

            if max_density < 0.02 {
                self.pixel_cache.push(PixelInfo {
                    density: 0.0,
                    alpha: 0,
                });
            } else {
                let alpha = 60 + (max_density * 195.0).min(195.0) as u8;
                self.pixel_cache.push(PixelInfo {
                    density: max_density,
                    alpha,
                });
            }
        }

        self.pixel_cache_dirty = false;
        self.last_cache_line_count = line_count;
        self.last_cache_height = minimap_height;
    }

    pub fn render(
        &mut self,
        scrollbar_total: u64,
        scrollbar_offset: u64,
        scrollbar_len: u64,
        panel_x: i32,
        panel_y: i32,
        panel_w: i32,
        panel_h: i32,
        d: &mut RaylibDrawHandle,
    ) {
        let minimap_x = panel_x + panel_w - self.width;
        let minimap_h = panel_h;
        if minimap_h <= 0 {
            return;
        }

        let row_h = 2;
        let margin = 4;
        let bar_area = self.width - margin * 2;

        d.draw_rectangle(minimap_x, panel_y, self.width, minimap_h, Color::new(18, 18, 22, 255));
        d.draw_line(minimap_x, panel_y, minimap_x, panel_y + minimap_h, Color::new(45, 45, 50, 255));

        if self.pixel_cache_dirty
            || self.lines.len() != self.last_cache_line_count
            || minimap_h != self.last_cache_height
        {
            self.rebuild_pixel_cache(minimap_h);
        }

        let (indicator_y, indicator_h) = self.indicator_rect(scrollbar_total, scrollbar_offset, scrollbar_len, minimap_h);

        for (i, pixel) in self.pixel_cache.iter().enumerate() {
            if pixel.alpha == 0 {
                continue;
            }
            let y = panel_y + i as i32 * row_h;
            let in_viewport = indicator_h > 0 && (y - panel_y) + row_h > indicator_y && (y - panel_y) < indicator_y + indicator_h;

            let bar_w = ((pixel.density * bar_area as f32).max(2.0)) as i32;
            let base_alpha = if in_viewport { pixel.alpha } else { pixel.alpha / 2 };
            d.draw_rectangle(
                minimap_x + margin,
                y,
                bar_w.min(bar_area),
                row_h.min(panel_y + minimap_h - y),
                Color::new(140, 150, 170, base_alpha),
            );
        }

        if indicator_h > 0 {
            d.draw_rectangle(
                minimap_x + 1,
                panel_y + indicator_y,
                self.width - 2,
                indicator_h,
                Color::new(200, 210, 230, 18),
            );

            let line_color = if self.dragging {
                Color::new(130, 160, 200, 200)
            } else {
                Color::new(100, 110, 135, 160)
            };
            d.draw_rectangle(minimap_x + 1, panel_y + indicator_y, self.width - 2, 1, line_color);
            d.draw_rectangle(minimap_x + 1, panel_y + indicator_y + indicator_h - 1, self.width - 2, 1, line_color);
        }
    }

    fn indicator_rect(&self, scrollbar_total: u64, scrollbar_offset: u64, scrollbar_len: u64, minimap_h: i32) -> (i32, i32) {
        if scrollbar_total <= scrollbar_len || scrollbar_total == 0 || minimap_h <= 0 {
            return (0, 0);
        }
        let scrollable = scrollbar_total - scrollbar_len;
        let h = ((scrollbar_len as f64 / scrollbar_total as f64) * minimap_h as f64)
            .max(12.0) as i32;
        let y = if scrollable > 0 {
            ((scrollbar_offset as f64 / scrollable as f64) * (minimap_h - h) as f64) as i32
        } else {
            0
        };
        (y, h)
    }

    pub fn handle_mouse_press(
        &mut self,
        mouse_y: i32,
        panel_y: i32,
        panel_h: i32,
        scrollbar_total: u64,
        scrollbar_offset: u64,
        scrollbar_len: u64,
    ) -> i32 {
        let minimap_h = panel_h;
        if minimap_h <= 0 || scrollbar_total <= scrollbar_len {
            return 0;
        }

        let local_y = mouse_y - panel_y;
        let (ind_y, ind_h) = self.indicator_rect(scrollbar_total, scrollbar_offset, scrollbar_len, minimap_h);

        if local_y >= ind_y && local_y <= ind_y + ind_h {
            self.dragging = true;
            self.drag_offset = local_y as f64 - ind_y as f64;
            return 0;
        }

        let frac = (local_y as f64 / minimap_h as f64).clamp(0.0, 1.0);
        let scrollable = scrollbar_total - scrollbar_len;
        let target = (frac * scrollable as f64) as i64;
        (target - scrollbar_offset as i64) as i32
    }

    pub fn handle_mouse_drag(
        &self,
        mouse_y: i32,
        panel_y: i32,
        panel_h: i32,
        scrollbar_total: u64,
        scrollbar_offset: u64,
        scrollbar_len: u64,
    ) -> i32 {
        if !self.dragging {
            return 0;
        }
        let minimap_h = panel_h;
        if minimap_h <= 0 || scrollbar_total <= scrollbar_len {
            return 0;
        }

        let local_y = mouse_y - panel_y;
        let scrollable = scrollbar_total - scrollbar_len;
        let (_, ind_h) = self.indicator_rect(scrollbar_total, scrollbar_offset, scrollbar_len, minimap_h);
        let track_h = (minimap_h - ind_h) as f64;
        if track_h <= 0.0 {
            return 0;
        }

        let top_y = (local_y as f64 - self.drag_offset).clamp(0.0, track_h);
        let frac = top_y / track_h;
        let target = (frac * scrollable as f64) as i64;
        (target - scrollbar_offset as i64) as i32
    }

    pub fn handle_mouse_release(&mut self) {
        self.dragging = false;
    }
}
