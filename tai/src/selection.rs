use raylib::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub struct CellPos {
    pub col: i32,
    pub row: i32,
}

pub struct TextSelection {
    pub start: Option<CellPos>,
    pub end: Option<CellPos>,
    pub active: bool, // mouse is being dragged
}

impl TextSelection {
    pub fn new() -> Self {
        TextSelection {
            start: None,
            end: None,
            active: false,
        }
    }

    pub fn begin(&mut self, col: i32, row: i32) {
        self.start = Some(CellPos { col, row });
        self.end = Some(CellPos { col, row });
        self.active = true;
    }

    pub fn update(&mut self, col: i32, row: i32) {
        if self.active {
            self.end = Some(CellPos { col, row });
        }
    }

    pub fn finish(&mut self) {
        self.active = false;
    }

    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
        self.active = false;
    }

    pub fn has_selection(&self) -> bool {
        if let (Some(s), Some(e)) = (self.start, self.end) {
            s.col != e.col || s.row != e.row
        } else {
            false
        }
    }

    fn ordered(&self) -> Option<(CellPos, CellPos)> {
        let (s, e) = (self.start?, self.end?);
        if s.row < e.row || (s.row == e.row && s.col <= e.col) {
            Some((s, e))
        } else {
            Some((e, s))
        }
    }

    pub fn is_cell_selected(&self, col: i32, row: i32) -> bool {
        let (s, e) = match self.ordered() {
            Some(pair) => pair,
            None => return false,
        };

        if row < s.row || row > e.row {
            return false;
        }
        if s.row == e.row {
            return col >= s.col && col <= e.col;
        }
        if row == s.row {
            return col >= s.col;
        }
        if row == e.row {
            return col <= e.col;
        }
        true
    }

    pub fn render(
        &self,
        cell_width: i32,
        cell_height: i32,
        pad: i32,
        term_cols: i32,
        term_rows: i32,
        d: &mut RaylibDrawHandle,
    ) {
        let (s, e) = match self.ordered() {
            Some(pair) => pair,
            None => return,
        };

        let highlight = Color::new(80, 140, 220, 100);

        for row in s.row..=e.row {
            if row < 0 || row >= term_rows {
                continue;
            }

            let col_start = if row == s.row { s.col } else { 0 };
            let col_end = if row == e.row { e.col } else { term_cols - 1 };

            let x = pad + col_start * cell_width;
            let y = pad + row * cell_height;
            let w = (col_end - col_start + 1) * cell_width;

            d.draw_rectangle(x, y, w, cell_height, highlight);
        }
    }
}

pub fn mouse_to_cell(mouse_x: i32, mouse_y: i32, cell_width: i32, cell_height: i32, pad: i32) -> (i32, i32) {
    let col = (mouse_x - pad) / cell_width;
    let row = (mouse_y - pad) / cell_height;
    (col.max(0), row.max(0))
}

pub fn copy_to_clipboard(text: &str) {
    use std::process::{Command, Stdio};
    use std::io::Write;
    if let Ok(mut child) = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(ref mut stdin) = child.stdin {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn paste_from_clipboard() -> Option<String> {
    use std::process::Command;
    Command::new("pbpaste")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
}
