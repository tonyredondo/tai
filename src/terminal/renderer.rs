use crate::bindings::*;
use raylib::prelude::*;

fn resolve_color(
    color: GhosttyStyleColor,
    colors: &GhosttyRenderStateColors,
    fallback: GhosttyColorRgb,
) -> GhosttyColorRgb {
    unsafe {
        match color.tag {
            GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_RGB => color.value.rgb,
            GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE => {
                colors.palette[color.value.palette as usize]
            }
            _ => fallback,
        }
    }
}

fn has_explicit_color(color: &GhosttyStyleColor) -> bool {
    color.tag != GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE
}

fn draw_braille(d: &mut RaylibDrawHandle, cp: u32, x: i32, y: i32, cw: i32, ch: i32, color: Color) {
    let pattern = cp - 0x2800;
    // Braille dot positions: 2 columns x 4 rows
    // Bit mapping: 0→(0,0) 1→(1,0) 2→(2,0) 3→(0,1) 4→(1,1) 5→(2,1) 6→(3,0) 7→(3,1)
    let dots: [(u32, i32, i32); 8] = [
        (1,   0, 0), (2,   1, 0), (4,   2, 0),
        (8,   0, 1), (16,  1, 1), (32,  2, 1),
        (64,  3, 0), (128, 3, 1),
    ];
    let dot_w = (cw as f32 * 0.25).max(1.5);
    let col_spacing = cw as f32 / 2.0;
    let row_spacing = ch as f32 / 4.0;
    let x_off = cw as f32 * 0.18;
    let y_off = row_spacing * 0.3;

    for &(bit, row, col) in &dots {
        if pattern & bit != 0 {
            let cx = x as f32 + x_off + col as f32 * col_spacing;
            let cy = y as f32 + y_off + row as f32 * row_spacing;
            d.draw_circle(cx as i32, cy as i32, dot_w, color);
        }
    }
}

pub fn render_terminal(
    render_state: GhosttyRenderState,
    row_iter: GhosttyRenderStateRowIterator,
    row_cells: GhosttyRenderStateRowCells,
    font: &raylib::ffi::Font,
    cell_width: i32,
    cell_height: i32,
    font_size: i32,
    pad: i32,
    _terminal: GhosttyTerminal,
    d: &mut RaylibDrawHandle,
) {
    unsafe {
        let mut colors: GhosttyRenderStateColors = std::mem::zeroed();
        colors.size = std::mem::size_of::<GhosttyRenderStateColors>();
        if ghostty_render_state_colors_get(render_state, &mut colors) != GhosttyResult_GHOSTTY_SUCCESS
        {
            return;
        }

        if ghostty_render_state_get(
            render_state,
            GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
            &row_iter as *const _ as *mut std::ffi::c_void,
        ) != GhosttyResult_GHOSTTY_SUCCESS
        {
            return;
        }

        let mut y = pad;

        while ghostty_render_state_row_iterator_next(row_iter) {
            if ghostty_render_state_row_get(
                row_iter,
                GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                &row_cells as *const _ as *mut std::ffi::c_void,
            ) != GhosttyResult_GHOSTTY_SUCCESS
            {
                y += cell_height;
                continue;
            }

            let mut x = pad;

            while ghostty_render_state_row_cells_next(row_cells) {
                let mut grapheme_len: u32 = 0;
                ghostty_render_state_row_cells_get(
                    row_cells,
                    GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                    &mut grapheme_len as *mut u32 as *mut std::ffi::c_void,
                );

                let mut style: GhosttyStyle = std::mem::zeroed();
                style.size = std::mem::size_of::<GhosttyStyle>();
                ghostty_render_state_row_cells_get(
                    row_cells,
                    GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                    &mut style as *mut _ as *mut std::ffi::c_void,
                );

                if grapheme_len == 0 {
                    let bg = resolve_color(style.bg_color, &colors, colors.background);
                    if has_explicit_color(&style.bg_color) {
                        d.draw_rectangle(x, y, cell_width, cell_height, Color::new(bg.r, bg.g, bg.b, 255));
                    }
                    x += cell_width;
                    continue;
                }

                let len = grapheme_len.min(16) as usize;
                let mut codepoints = [0u32; 16];
                ghostty_render_state_row_cells_get(
                    row_cells,
                    GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                    codepoints.as_mut_ptr() as *mut std::ffi::c_void,
                );

                let mut text = String::with_capacity(len * 4);
                for &cp in &codepoints[..len] {
                    if let Some(c) = char::from_u32(cp) {
                        text.push(c);
                    }
                }

                let mut draw_fg = resolve_color(style.fg_color, &colors, colors.foreground);
                let mut draw_bg = resolve_color(style.bg_color, &colors, colors.background);
                let mut do_bg = has_explicit_color(&style.bg_color);

                if style.inverse {
                    std::mem::swap(&mut draw_fg, &mut draw_bg);
                    do_bg = true;
                }

                if do_bg {
                    d.draw_rectangle(
                        x, y, cell_width, cell_height,
                        Color::new(draw_bg.r, draw_bg.g, draw_bg.b, 255),
                    );
                }

                if style.invisible {
                    x += cell_width;
                    continue;
                }

                let alpha = if style.faint { 128u8 } else { 255u8 };
                let italic_offset = if style.italic { font_size / 6 } else { 0 };
                let ray_fg = Color::new(draw_fg.r, draw_fg.g, draw_fg.b, alpha);

                if len == 1 && codepoints[0] >= 0x2800 && codepoints[0] <= 0x28FF {
                    draw_braille(d, codepoints[0], x, y, cell_width, cell_height, ray_fg);
                    x += cell_width;
                    continue;
                }

                let c_text = std::ffi::CString::new(text.as_str()).unwrap_or_default();

                raylib::ffi::DrawTextEx(
                    *font,
                    c_text.as_ptr(),
                    raylib::ffi::Vector2 {
                        x: (x + italic_offset) as f32,
                        y: y as f32,
                    },
                    font_size as f32,
                    0.0,
                    raylib::ffi::Color {
                        r: ray_fg.r,
                        g: ray_fg.g,
                        b: ray_fg.b,
                        a: ray_fg.a,
                    },
                );

                if style.bold {
                    raylib::ffi::DrawTextEx(
                        *font,
                        c_text.as_ptr(),
                        raylib::ffi::Vector2 {
                            x: (x + italic_offset + 1) as f32,
                            y: y as f32,
                        },
                        font_size as f32,
                        0.0,
                        raylib::ffi::Color {
                            r: ray_fg.r,
                            g: ray_fg.g,
                            b: ray_fg.b,
                            a: ray_fg.a,
                        },
                    );
                }

                if style.underline != 0 {
                    let uy = y + cell_height - 2;
                    d.draw_line(x, uy, x + cell_width, uy, Color::new(ray_fg.r, ray_fg.g, ray_fg.b, ray_fg.a));
                }

                if style.strikethrough {
                    let sy = y + cell_height / 2;
                    d.draw_line(x, sy, x + cell_width, sy, Color::new(ray_fg.r, ray_fg.g, ray_fg.b, ray_fg.a));
                }

                x += cell_width;
            }

            let clean = false;
            ghostty_render_state_row_set(
                row_iter,
                GhosttyRenderStateRowOption_GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY,
                &clean as *const bool as *const std::ffi::c_void,
            );

            y += cell_height;
        }

        // Draw cursor
        let mut cursor_visible = false;
        ghostty_render_state_get(
            render_state,
            GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE,
            &mut cursor_visible as *mut bool as *mut std::ffi::c_void,
        );
        let mut cursor_in_viewport = false;
        ghostty_render_state_get(
            render_state,
            GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE,
            &mut cursor_in_viewport as *mut bool as *mut std::ffi::c_void,
        );

        if cursor_visible && cursor_in_viewport {
            let mut cx: u16 = 0;
            let mut cy: u16 = 0;
            ghostty_render_state_get(
                render_state,
                GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X,
                &mut cx as *mut u16 as *mut std::ffi::c_void,
            );
            ghostty_render_state_get(
                render_state,
                GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y,
                &mut cy as *mut u16 as *mut std::ffi::c_void,
            );

            let mut cur_rgb = colors.foreground;
            if colors.cursor_has_value {
                cur_rgb = colors.cursor;
            }
            let cur_x = pad + cx as i32 * cell_width;
            let cur_y = pad + cy as i32 * cell_height;
            d.draw_rectangle(
                cur_x,
                cur_y,
                cell_width,
                cell_height,
                Color::new(cur_rgb.r, cur_rgb.g, cur_rgb.b, 128),
            );
        }

        // Reset dirty state
        let clean_state = GhosttyRenderStateDirty_GHOSTTY_RENDER_STATE_DIRTY_FALSE;
        ghostty_render_state_set(
            render_state,
            GhosttyRenderStateOption_GHOSTTY_RENDER_STATE_OPTION_DIRTY,
            &clean_state as *const _ as *const std::ffi::c_void,
        );
    }
}
