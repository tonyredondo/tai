use crate::bindings::*;
use std::ptr;

pub struct Terminal {
    handle: GhosttyTerminal,
    render_state: GhosttyRenderState,
    row_iter: GhosttyRenderStateRowIterator,
    row_cells: GhosttyRenderStateRowCells,
    key_encoder: GhosttyKeyEncoder,
    key_event: GhosttyKeyEvent,
    mouse_encoder: GhosttyMouseEncoder,
    mouse_event: GhosttyMouseEvent,
    placement_iter: GhosttyKittyGraphicsPlacementIterator,
    formatter: Option<GhosttyFormatter>,
    cols: u16,
    rows: u16,
    pty_fd: i32,
    cell_width: i32,
    cell_height: i32,
    vt_mirror: Vec<u8>,
    effects_ctx: *mut EffectsContext,
    pub last_osc_title: String,
}

pub enum PtyReadResult {
    Ok,
    Eof,
    Error,
}

struct EffectsContext {
    pty_fd: i32,
    cell_width: i32,
    cell_height: i32,
    cols: u16,
    rows: u16,
    title_ptr: *mut String,
}

unsafe extern "C" fn effect_write_pty(
    _terminal: GhosttyTerminal,
    userdata: *mut std::ffi::c_void,
    data: *const u8,
    len: usize,
) {
    unsafe {
        let ctx = &*(userdata as *const EffectsContext);
        let buf = std::slice::from_raw_parts(data, len);
        crate::terminal::pty::pty_write_raw(ctx.pty_fd, buf);
    }
}

unsafe extern "C" fn effect_size(
    _terminal: GhosttyTerminal,
    userdata: *mut std::ffi::c_void,
    out_size: *mut GhosttySizeReportSize,
) -> bool {
    unsafe {
        let ctx = &*(userdata as *const EffectsContext);
        (*out_size).rows = ctx.rows;
        (*out_size).columns = ctx.cols;
        (*out_size).cell_width = ctx.cell_width as u32;
        (*out_size).cell_height = ctx.cell_height as u32;
    }
    true
}

unsafe extern "C" fn effect_device_attributes(
    _terminal: GhosttyTerminal,
    _userdata: *mut std::ffi::c_void,
    out_attrs: *mut GhosttyDeviceAttributes,
) -> bool {
    unsafe {
        (*out_attrs).primary.conformance_level = GHOSTTY_DA_CONFORMANCE_VT220 as u16;
        (*out_attrs).primary.features[0] = GHOSTTY_DA_FEATURE_COLUMNS_132 as u16;
        (*out_attrs).primary.features[1] = GHOSTTY_DA_FEATURE_SELECTIVE_ERASE as u16;
        (*out_attrs).primary.features[2] = GHOSTTY_DA_FEATURE_ANSI_COLOR as u16;
        (*out_attrs).primary.num_features = 3;
        (*out_attrs).secondary.device_type = GHOSTTY_DA_DEVICE_TYPE_VT220 as u16;
        (*out_attrs).secondary.firmware_version = 1;
        (*out_attrs).secondary.rom_cartridge = 0;
        (*out_attrs).tertiary.unit_id = 0;
    }
    true
}

unsafe extern "C" fn effect_xtversion(
    _terminal: GhosttyTerminal,
    _userdata: *mut std::ffi::c_void,
) -> GhosttyString {
    GhosttyString {
        ptr: b"tai\0".as_ptr(),
        len: 3,
    }
}

unsafe extern "C" fn effect_title_changed(
    terminal: GhosttyTerminal,
    userdata: *mut std::ffi::c_void,
) {
    unsafe {
        let ctx = &*(userdata as *const EffectsContext);
        let mut title = GhosttyString {
            ptr: ptr::null(),
            len: 0,
        };
        if ghostty_terminal_get(
            terminal,
            GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_TITLE,
            &mut title as *mut _ as *mut std::ffi::c_void,
        ) == GhosttyResult_GHOSTTY_SUCCESS
        {
            if !title.ptr.is_null() && title.len > 0 {
                let slice = std::slice::from_raw_parts(title.ptr, title.len);
                if let Ok(s) = std::str::from_utf8(slice) {
                    if !ctx.title_ptr.is_null() {
                        *ctx.title_ptr = s.to_string();
                    }
                }
            }
        }
    }
}

unsafe extern "C" fn effect_color_scheme(
    _terminal: GhosttyTerminal,
    _userdata: *mut std::ffi::c_void,
    _out_scheme: *mut GhosttyColorScheme,
) -> bool {
    false
}

impl Terminal {
    pub fn new(cols: u16, rows: u16, scrollback: u32) -> Result<Self, String> {
        unsafe {
            let mut handle: GhosttyTerminal = ptr::null_mut();
            let opts = GhosttyTerminalOptions {
                cols,
                rows,
                max_scrollback: scrollback as usize,
            };
            let res = ghostty_terminal_new(ptr::null(), &mut handle, opts);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                return Err(format!("ghostty_terminal_new failed: {res}"));
            }

            let mut render_state: GhosttyRenderState = ptr::null_mut();
            let res = ghostty_render_state_new(ptr::null(), &mut render_state);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_terminal_free(handle);
                return Err(format!("ghostty_render_state_new failed: {res}"));
            }

            let mut row_iter: GhosttyRenderStateRowIterator = ptr::null_mut();
            let res = ghostty_render_state_row_iterator_new(ptr::null(), &mut row_iter);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_render_state_free(render_state);
                ghostty_terminal_free(handle);
                return Err(format!("row_iterator_new failed: {res}"));
            }

            let mut row_cells: GhosttyRenderStateRowCells = ptr::null_mut();
            let res = ghostty_render_state_row_cells_new(ptr::null(), &mut row_cells);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_render_state_row_iterator_free(row_iter);
                ghostty_render_state_free(render_state);
                ghostty_terminal_free(handle);
                return Err(format!("row_cells_new failed: {res}"));
            }

            let mut key_encoder: GhosttyKeyEncoder = ptr::null_mut();
            let res = ghostty_key_encoder_new(ptr::null(), &mut key_encoder);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_render_state_row_cells_free(row_cells);
                ghostty_render_state_row_iterator_free(row_iter);
                ghostty_render_state_free(render_state);
                ghostty_terminal_free(handle);
                return Err(format!("key_encoder_new failed: {res}"));
            }

            let mut key_event: GhosttyKeyEvent = ptr::null_mut();
            let res = ghostty_key_event_new(ptr::null(), &mut key_event);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_key_encoder_free(key_encoder);
                ghostty_render_state_row_cells_free(row_cells);
                ghostty_render_state_row_iterator_free(row_iter);
                ghostty_render_state_free(render_state);
                ghostty_terminal_free(handle);
                return Err(format!("key_event_new failed: {res}"));
            }

            let mut mouse_encoder: GhosttyMouseEncoder = ptr::null_mut();
            let res = ghostty_mouse_encoder_new(ptr::null(), &mut mouse_encoder);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_key_event_free(key_event);
                ghostty_key_encoder_free(key_encoder);
                ghostty_render_state_row_cells_free(row_cells);
                ghostty_render_state_row_iterator_free(row_iter);
                ghostty_render_state_free(render_state);
                ghostty_terminal_free(handle);
                return Err(format!("mouse_encoder_new failed: {res}"));
            }

            let mut mouse_event: GhosttyMouseEvent = ptr::null_mut();
            let res = ghostty_mouse_event_new(ptr::null(), &mut mouse_event);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_mouse_encoder_free(mouse_encoder);
                ghostty_key_event_free(key_event);
                ghostty_key_encoder_free(key_encoder);
                ghostty_render_state_row_cells_free(row_cells);
                ghostty_render_state_row_iterator_free(row_iter);
                ghostty_render_state_free(render_state);
                ghostty_terminal_free(handle);
                return Err(format!("mouse_event_new failed: {res}"));
            }

            let mut placement_iter: GhosttyKittyGraphicsPlacementIterator = ptr::null_mut();
            let res = ghostty_kitty_graphics_placement_iterator_new(ptr::null(), &mut placement_iter);
            if res != GhosttyResult_GHOSTTY_SUCCESS {
                ghostty_mouse_event_free(mouse_event);
                ghostty_mouse_encoder_free(mouse_encoder);
                ghostty_key_event_free(key_event);
                ghostty_key_encoder_free(key_encoder);
                ghostty_render_state_row_cells_free(row_cells);
                ghostty_render_state_row_iterator_free(row_iter);
                ghostty_render_state_free(render_state);
                ghostty_terminal_free(handle);
                return Err(format!("placement_iterator_new failed: {res}"));
            }

            Ok(Terminal {
                handle,
                render_state,
                row_iter,
                row_cells,
                key_encoder,
                key_event,
                mouse_encoder,
                mouse_event,
                placement_iter,
                formatter: None,
                cols,
                rows,
                pty_fd: -1,
                cell_width: 0,
                cell_height: 0,
                vt_mirror: Vec::with_capacity(4096),
                effects_ctx: ptr::null_mut(),
                last_osc_title: String::new(),
            })
        }
    }

    pub fn setup_effects(&mut self, pty_fd: i32, cell_width: i32, cell_height: i32) {
        self.pty_fd = pty_fd;
        self.cell_width = cell_width;
        self.cell_height = cell_height;

        if !self.effects_ctx.is_null() {
            unsafe { drop(Box::from_raw(self.effects_ctx)); }
        }

        unsafe {
            let ctx = Box::new(EffectsContext {
                pty_fd,
                cell_width,
                cell_height,
                cols: self.cols,
                rows: self.rows,
                title_ptr: &mut self.last_osc_title as *mut String,
            });
            let ctx_ptr = Box::into_raw(ctx);
            self.effects_ctx = ctx_ptr;

            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_USERDATA,
                ctx_ptr as *const std::ffi::c_void,
            );
            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                effect_write_pty as *const std::ffi::c_void,
            );
            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_SIZE,
                effect_size as *const std::ffi::c_void,
            );
            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_DEVICE_ATTRIBUTES,
                effect_device_attributes as *const std::ffi::c_void,
            );
            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_XTVERSION,
                effect_xtversion as *const std::ffi::c_void,
            );
            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_TITLE_CHANGED,
                effect_title_changed as *const std::ffi::c_void,
            );
            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_COLOR_SCHEME,
                effect_color_scheme as *const std::ffi::c_void,
            );

            let kitty_limit: u64 = 64 * 1024 * 1024;
            ghostty_terminal_set(
                self.handle,
                GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT,
                &kitty_limit as *const u64 as *const std::ffi::c_void,
            );
        }
    }

    pub fn vt_write(&mut self, data: &[u8]) {
        unsafe {
            ghostty_terminal_vt_write(self.handle, data.as_ptr(), data.len());
        }
        self.vt_mirror.extend_from_slice(data);
    }

    pub fn drain_vt_mirror(&mut self) -> Option<Vec<u8>> {
        if self.vt_mirror.is_empty() {
            None
        } else {
            let data = std::mem::take(&mut self.vt_mirror);
            Some(data)
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cw: u32, ch: u32) {
        self.cols = cols;
        self.rows = rows;
        if !self.effects_ctx.is_null() {
            unsafe {
                (*self.effects_ctx).cols = cols;
                (*self.effects_ctx).rows = rows;
                (*self.effects_ctx).cell_width = cw as i32;
                (*self.effects_ctx).cell_height = ch as i32;
            }
        }
        unsafe {
            ghostty_terminal_resize(self.handle, cols, rows, cw, ch);
        }
    }

    pub fn scroll_viewport(&mut self, delta: i32) {
        unsafe {
            let sv = GhosttyTerminalScrollViewport {
                tag: GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_DELTA,
                value: GhosttyTerminalScrollViewportValue { delta: delta as isize },
            };
            ghostty_terminal_scroll_viewport(self.handle, sv);
        }
    }

    pub fn update_render_state(&mut self) {
        unsafe {
            ghostty_render_state_update(self.render_state, self.handle);
        }
    }

    pub fn get_buffer_text(&self, _lines: usize) -> String {
        unsafe {
            let mut opts: GhosttyFormatterTerminalOptions = std::mem::zeroed();
            opts.size = std::mem::size_of::<GhosttyFormatterTerminalOptions>();
            opts.emit = GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_PLAIN;
            opts.trim = true;

            let mut formatter: GhosttyFormatter = ptr::null_mut();
            let res = ghostty_formatter_terminal_new(
                ptr::null(),
                &mut formatter,
                self.handle,
                opts,
            );
            if res != GhosttyResult_GHOSTTY_SUCCESS || formatter.is_null() {
                return String::new();
            }

            let mut out_ptr: *mut u8 = ptr::null_mut();
            let mut out_len: usize = 0;
            let res = ghostty_formatter_format_alloc(
                formatter,
                ptr::null(),
                &mut out_ptr,
                &mut out_len,
            );

            let result = if res == GhosttyResult_GHOSTTY_SUCCESS && !out_ptr.is_null() && out_len > 0
            {
                let slice = std::slice::from_raw_parts(out_ptr, out_len);
                let s = String::from_utf8_lossy(slice).to_string();
                ghostty_free(ptr::null(), out_ptr, out_len);
                s
            } else {
                String::new()
            };

            ghostty_formatter_free(formatter);
            result
        }
    }

    pub fn get_viewport_rows(&self) -> Vec<String> {
        unsafe {
            let mut rows = Vec::new();
            if ghostty_render_state_get(
                self.render_state,
                GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                &self.row_iter as *const _ as *mut std::ffi::c_void,
            ) != GhosttyResult_GHOSTTY_SUCCESS
            {
                return rows;
            }

            while ghostty_render_state_row_iterator_next(self.row_iter) {
                if ghostty_render_state_row_get(
                    self.row_iter,
                    GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    &self.row_cells as *const _ as *mut std::ffi::c_void,
                ) != GhosttyResult_GHOSTTY_SUCCESS
                {
                    rows.push(String::new());
                    continue;
                }

                let mut line = String::new();
                while ghostty_render_state_row_cells_next(self.row_cells) {
                    let mut grapheme_len: u32 = 0;
                    ghostty_render_state_row_cells_get(
                        self.row_cells,
                        GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                        &mut grapheme_len as *mut u32 as *mut std::ffi::c_void,
                    );
                    if grapheme_len == 0 {
                        line.push(' ');
                        continue;
                    }
                    let len = grapheme_len.min(16) as usize;
                    let mut codepoints = [0u32; 16];
                    ghostty_render_state_row_cells_get(
                        self.row_cells,
                        GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                        codepoints.as_mut_ptr() as *mut std::ffi::c_void,
                    );
                    for &cp in &codepoints[..len] {
                        if let Some(c) = char::from_u32(cp) {
                            line.push(c);
                        }
                    }
                }
                rows.push(line.trim_end().to_string());
            }
            rows
        }
    }

    pub fn cursor_viewport_position(&self) -> Option<(u16, u16)> {
        unsafe {
            let mut visible = false;
            ghostty_render_state_get(
                self.render_state,
                GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE,
                &mut visible as *mut bool as *mut std::ffi::c_void,
            );
            let mut in_viewport = false;
            ghostty_render_state_get(
                self.render_state,
                GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE,
                &mut in_viewport as *mut bool as *mut std::ffi::c_void,
            );
            if !in_viewport {
                return None;
            }
            let mut cx: u16 = 0;
            let mut cy: u16 = 0;
            ghostty_render_state_get(
                self.render_state,
                GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X,
                &mut cx as *mut u16 as *mut std::ffi::c_void,
            );
            ghostty_render_state_get(
                self.render_state,
                GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y,
                &mut cy as *mut u16 as *mut std::ffi::c_void,
            );
            Some((cx, cy))
        }
    }

    pub fn handle(&self) -> GhosttyTerminal {
        self.handle
    }

    pub fn render_state(&self) -> GhosttyRenderState {
        self.render_state
    }

    pub fn row_iter(&self) -> GhosttyRenderStateRowIterator {
        self.row_iter
    }

    pub fn row_cells(&self) -> GhosttyRenderStateRowCells {
        self.row_cells
    }

    pub fn key_encoder(&self) -> GhosttyKeyEncoder {
        self.key_encoder
    }

    pub fn key_event(&self) -> GhosttyKeyEvent {
        self.key_event
    }

    pub fn mouse_encoder(&self) -> GhosttyMouseEncoder {
        self.mouse_encoder
    }

    pub fn mouse_event(&self) -> GhosttyMouseEvent {
        self.mouse_event
    }

    pub fn placement_iter(&self) -> GhosttyKittyGraphicsPlacementIterator {
        self.placement_iter
    }

    pub fn get_scrollbar(&self) -> Option<(u64, u64, u64)> {
        unsafe {
            let mut sb: GhosttyTerminalScrollbar = std::mem::zeroed();
            if ghostty_terminal_get(
                self.handle,
                GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                &mut sb as *mut _ as *mut std::ffi::c_void,
            ) == GhosttyResult_GHOSTTY_SUCCESS
            {
                Some((sb.total, sb.offset, sb.len))
            } else {
                None
            }
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe {
            if !self.effects_ctx.is_null() {
                drop(Box::from_raw(self.effects_ctx));
                self.effects_ctx = ptr::null_mut();
            }
            if let Some(f) = self.formatter {
                ghostty_formatter_free(f);
            }
            ghostty_kitty_graphics_placement_iterator_free(self.placement_iter);
            ghostty_mouse_event_free(self.mouse_event);
            ghostty_mouse_encoder_free(self.mouse_encoder);
            ghostty_key_event_free(self.key_event);
            ghostty_key_encoder_free(self.key_encoder);
            ghostty_render_state_row_cells_free(self.row_cells);
            ghostty_render_state_row_iterator_free(self.row_iter);
            ghostty_render_state_free(self.render_state);
            ghostty_terminal_free(self.handle);
        }
    }
}
