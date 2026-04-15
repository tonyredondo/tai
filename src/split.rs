use raylib::prelude::*;

use crate::config::TaiConfig;
use crate::tab::TabSession;
use crate::tab_bar::TabBar;

#[derive(Clone, Copy, Debug)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug)]
pub struct PanelRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PanelRect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

pub struct Panel {
    pub id: u32,
    pub tabs: Vec<TabSession>,
    pub active_tab: usize,
    pub tab_bar: TabBar,
    pub rect: PanelRect,
}

impl Panel {
    pub fn new(id: u32, initial_tab: TabSession, cell_height: i32) -> Self {
        Panel {
            id,
            tabs: vec![initial_tab],
            active_tab: 0,
            tab_bar: TabBar::new(cell_height),
            rect: PanelRect { x: 0, y: 0, w: 0, h: 0 },
        }
    }

    pub fn active_tab(&self) -> &TabSession {
        &self.tabs[self.active_tab]
    }

    pub fn active_tab_mut(&mut self) -> &mut TabSession {
        &mut self.tabs[self.active_tab]
    }
}

pub const SEPARATOR_PX: i32 = 2;
const SEPARATOR_HIT_PX: i32 = 6;
const MIN_PANEL_W: i32 = 80;
const MIN_PANEL_H: i32 = 60;

#[derive(Clone, Copy, Debug)]
pub struct SeparatorHit {
    pub direction: SplitDirection,
    pub origin: i32,
    pub total: i32,
    pub node_ptr: usize,
}

pub enum SplitNode {
    Leaf(Panel),
    Split {
        direction: SplitDirection,
        ratio: f32,
        left: Box<SplitNode>,
        right: Box<SplitNode>,
    },
}

impl SplitNode {
    pub fn layout(&mut self, rect: PanelRect) {
        match self {
            SplitNode::Leaf(panel) => {
                panel.rect = rect;
            }
            SplitNode::Split { direction, ratio, left, right } => {
                match direction {
                    SplitDirection::Horizontal => {
                        let left_w = ((rect.w - SEPARATOR_PX) as f32 * *ratio) as i32;
                        let left_w = left_w.max(MIN_PANEL_W).min(rect.w - SEPARATOR_PX - MIN_PANEL_W);
                        let right_w = rect.w - SEPARATOR_PX - left_w;
                        left.layout(PanelRect {
                            x: rect.x,
                            y: rect.y,
                            w: left_w,
                            h: rect.h,
                        });
                        right.layout(PanelRect {
                            x: rect.x + left_w + SEPARATOR_PX,
                            y: rect.y,
                            w: right_w,
                            h: rect.h,
                        });
                    }
                    SplitDirection::Vertical => {
                        let top_h = ((rect.h - SEPARATOR_PX) as f32 * *ratio) as i32;
                        let top_h = top_h.max(MIN_PANEL_H).min(rect.h - SEPARATOR_PX - MIN_PANEL_H);
                        let bottom_h = rect.h - SEPARATOR_PX - top_h;
                        left.layout(PanelRect {
                            x: rect.x,
                            y: rect.y,
                            w: rect.w,
                            h: top_h,
                        });
                        right.layout(PanelRect {
                            x: rect.x,
                            y: rect.y + top_h + SEPARATOR_PX,
                            w: rect.w,
                            h: bottom_h,
                        });
                    }
                }
            }
        }
    }

    pub fn for_each_panel<F: FnMut(&Panel)>(&self, f: &mut F) {
        match self {
            SplitNode::Leaf(panel) => f(panel),
            SplitNode::Split { left, right, .. } => {
                left.for_each_panel(f);
                right.for_each_panel(f);
            }
        }
    }

    pub fn for_each_panel_mut<F: FnMut(&mut Panel)>(&mut self, f: &mut F) {
        match self {
            SplitNode::Leaf(panel) => f(panel),
            SplitNode::Split { left, right, .. } => {
                left.for_each_panel_mut(f);
                right.for_each_panel_mut(f);
            }
        }
    }

    pub fn find_panel_at(&mut self, x: i32, y: i32) -> Option<&mut Panel> {
        match self {
            SplitNode::Leaf(panel) => {
                if panel.rect.contains(x, y) {
                    Some(panel)
                } else {
                    None
                }
            }
            SplitNode::Split { left, right, .. } => {
                left.find_panel_at(x, y).or_else(|| right.find_panel_at(x, y))
            }
        }
    }

    pub fn panel_by_id(&self, id: u32) -> Option<&Panel> {
        match self {
            SplitNode::Leaf(panel) => {
                if panel.id == id { Some(panel) } else { None }
            }
            SplitNode::Split { left, right, .. } => {
                left.panel_by_id(id).or_else(|| right.panel_by_id(id))
            }
        }
    }

    pub fn panel_by_id_mut(&mut self, id: u32) -> Option<&mut Panel> {
        match self {
            SplitNode::Leaf(panel) => {
                if panel.id == id { Some(panel) } else { None }
            }
            SplitNode::Split { left, right, .. } => {
                left.panel_by_id_mut(id).or_else(|| right.panel_by_id_mut(id))
            }
        }
    }

    pub fn collect_leaves(&self) -> Vec<u32> {
        let mut ids = Vec::new();
        self.collect_leaves_inner(&mut ids);
        ids
    }

    fn collect_leaves_inner(&self, ids: &mut Vec<u32>) {
        match self {
            SplitNode::Leaf(panel) => ids.push(panel.id),
            SplitNode::Split { left, right, .. } => {
                left.collect_leaves_inner(ids);
                right.collect_leaves_inner(ids);
            }
        }
    }

    pub fn panel_count(&self) -> usize {
        match self {
            SplitNode::Leaf(_) => 1,
            SplitNode::Split { left, right, .. } => left.panel_count() + right.panel_count(),
        }
    }

    fn find_node_containing(&mut self, panel_id: u32) -> Option<&mut SplitNode> {
        match self {
            SplitNode::Leaf(p) => {
                if p.id == panel_id { Some(self) } else { None }
            }
            SplitNode::Split { left, right, .. } => {
                if left.find_node_containing(panel_id).is_some() {
                    left.find_node_containing(panel_id)
                } else {
                    right.find_node_containing(panel_id)
                }
            }
        }
    }

    pub fn split_panel(&mut self, panel_id: u32, direction: SplitDirection, new_panel: Panel) -> bool {
        let target = match self.find_node_containing(panel_id) {
            Some(node) => node as *mut SplitNode,
            None => return false,
        };
        let target = unsafe { &mut *target };
        let placeholder = SplitNode::Leaf(Panel {
            id: 0, tabs: Vec::new(), active_tab: 0,
            tab_bar: TabBar::new(1), rect: PanelRect { x: 0, y: 0, w: 0, h: 0 },
        });
        let old = std::mem::replace(target, placeholder);
        *target = SplitNode::Split {
            direction,
            ratio: 0.5,
            left: Box::new(old),
            right: Box::new(SplitNode::Leaf(new_panel)),
        };
        true
    }

    pub fn close_panel(&mut self, panel_id: u32) -> bool {
        match self {
            SplitNode::Leaf(_) => false,
            SplitNode::Split { left, right, .. } => {
                if let SplitNode::Leaf(p) = left.as_ref() {
                    if p.id == panel_id {
                        let sibling = std::mem::replace(right.as_mut(), SplitNode::Leaf(Panel {
                            id: 0, tabs: Vec::new(), active_tab: 0,
                            tab_bar: TabBar::new(1), rect: PanelRect { x: 0, y: 0, w: 0, h: 0 },
                        }));
                        *self = sibling;
                        return true;
                    }
                }
                if let SplitNode::Leaf(p) = right.as_ref() {
                    if p.id == panel_id {
                        let sibling = std::mem::replace(left.as_mut(), SplitNode::Leaf(Panel {
                            id: 0, tabs: Vec::new(), active_tab: 0,
                            tab_bar: TabBar::new(1), rect: PanelRect { x: 0, y: 0, w: 0, h: 0 },
                        }));
                        *self = sibling;
                        return true;
                    }
                }
                left.close_panel(panel_id) || right.close_panel(panel_id)
            }
        }
    }

    pub fn separator_at(&self, mx: i32, my: i32) -> Option<SeparatorHit> {
        match self {
            SplitNode::Leaf(_) => None,
            SplitNode::Split { direction, left, right, .. } => {
                match direction {
                    SplitDirection::Horizontal => {
                        let sep_x = get_right_edge(left);
                        let (sy, sh) = get_vertical_span(self);
                        if mx >= sep_x - SEPARATOR_HIT_PX && mx <= sep_x + SEPARATOR_PX + SEPARATOR_HIT_PX
                            && my >= sy && my < sy + sh
                        {
                            let (sx, sw) = get_horizontal_span(self);
                            return Some(SeparatorHit {
                                direction: *direction,
                                origin: sx,
                                total: sw,
                                node_ptr: self as *const SplitNode as usize,
                            });
                        }
                    }
                    SplitDirection::Vertical => {
                        let sep_y = get_bottom_edge(left);
                        let (sx, sw) = get_horizontal_span(self);
                        if my >= sep_y - SEPARATOR_HIT_PX && my <= sep_y + SEPARATOR_PX + SEPARATOR_HIT_PX
                            && mx >= sx && mx < sx + sw
                        {
                            let (sy, sh) = get_vertical_span(self);
                            return Some(SeparatorHit {
                                direction: *direction,
                                origin: sy,
                                total: sh,
                                node_ptr: self as *const SplitNode as usize,
                            });
                        }
                    }
                }
                left.separator_at(mx, my).or_else(|| right.separator_at(mx, my))
            }
        }
    }

    pub fn update_ratio_by_ptr(&mut self, node_ptr: usize, new_ratio: f32) -> bool {
        if std::ptr::eq(self, node_ptr as *const SplitNode) {
            if let SplitNode::Split { ratio, .. } = self {
                *ratio = new_ratio.clamp(0.1, 0.9);
                return true;
            }
        }
        if let SplitNode::Split { left, right, .. } = self {
            if left.update_ratio_by_ptr(node_ptr, new_ratio) { return true; }
            if right.update_ratio_by_ptr(node_ptr, new_ratio) { return true; }
        }
        false
    }

    pub fn draw_separators(&self, d: &mut raylib::prelude::RaylibDrawHandle) {
        if let SplitNode::Split { direction, left, right, .. } = self {
            let sep_color = raylib::prelude::Color::new(55, 55, 65, 255);
            match direction {
                SplitDirection::Horizontal => {
                    let sep_x = get_right_edge(left) ;
                    let (sy, sh) = get_vertical_span(self);
                    d.draw_rectangle(sep_x, sy, SEPARATOR_PX, sh, sep_color);
                }
                SplitDirection::Vertical => {
                    let sep_y = get_bottom_edge(left);
                    let (sx, sw) = get_horizontal_span(self);
                    d.draw_rectangle(sx, sep_y, sw, SEPARATOR_PX, sep_color);
                }
            }
            left.draw_separators(d);
            right.draw_separators(d);
        }
    }
}

fn get_right_edge(node: &SplitNode) -> i32 {
    match node {
        SplitNode::Leaf(p) => p.rect.x + p.rect.w,
        SplitNode::Split { right, .. } => get_right_edge(right),
    }
}

fn get_bottom_edge(node: &SplitNode) -> i32 {
    match node {
        SplitNode::Leaf(p) => p.rect.y + p.rect.h,
        SplitNode::Split { right, .. } => get_bottom_edge(right),
    }
}

fn get_vertical_span(node: &SplitNode) -> (i32, i32) {
    match node {
        SplitNode::Leaf(p) => (p.rect.y, p.rect.h),
        SplitNode::Split { left, right, .. } => {
            let (ly, lh) = get_vertical_span(left);
            let (ry, rh) = get_vertical_span(right);
            let min_y = ly.min(ry);
            let max_end = (ly + lh).max(ry + rh);
            (min_y, max_end - min_y)
        }
    }
}

fn get_horizontal_span(node: &SplitNode) -> (i32, i32) {
    match node {
        SplitNode::Leaf(p) => (p.rect.x, p.rect.w),
        SplitNode::Split { left, right, .. } => {
            let (lx, lw) = get_horizontal_span(left);
            let (rx, rw) = get_horizontal_span(right);
            let min_x = lx.min(rx);
            let max_end = (lx + lw).max(rx + rw);
            (min_x, max_end - min_x)
        }
    }
}

pub fn panel_term_size(
    rect: &PanelRect,
    pad: i32,
    minimap_width: i32,
    tab_bar_height: i32,
    cw: i32,
    ch: i32,
) -> (u16, u16) {
    let cols = ((rect.w - 2 * pad - minimap_width) / cw).max(1) as u16;
    let rows = ((rect.h - tab_bar_height - 2 * pad) / ch).max(1) as u16;
    (cols, rows)
}

pub fn alloc_panel_id(counter: &mut u32) -> u32 {
    let id = *counter;
    *counter += 1;
    id
}

pub fn create_panel(
    id: u32,
    config: &TaiConfig,
    cols: u16,
    rows: u16,
    cw: i32,
    ch: i32,
) -> Result<Panel, String> {
    let tab = TabSession::new(config, cols, rows, cw, ch)?;
    Ok(Panel::new(id, tab, ch))
}
