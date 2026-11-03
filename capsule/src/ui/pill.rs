//! 收起态药丸胶囊：未固定到任务栏时的横排药丸（276×32）与单元格绘制。

use super::*;

impl CapsuleView {
    /// 未固定时的药丸胶囊：276×32 圆角底，两行 × 三列
    /// （行1 CPU/硬盘读/网络上行，行2 内存/硬盘写/网络下行）。
    pub(super) fn render_pill(&self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        let Some(font) = font else { return };
        fill_round(px, 0.0, 0.0, PILL_W as f32, PILL_H as f32, PILL_R, BG);

        if !frame.has_data {
            let (text, c) = status_text(frame.conn, frame.dark);
            let tw = font.measure(FONT_SIZE, &text);
            let bl = PILL_H as f32 / 2.0 + font.ascent(FONT_SIZE) * 0.36;
            font.draw(px, (PILL_W as f32 - tw) / 2.0, bl, FONT_SIZE, &text, c);
            return;
        }

        let s = frame.snap;
        let cpu = s.cpu.as_ref().and_then(|c| c.usage).map(|v| v as f32);
        let mem = s.memory.as_ref().and_then(|m| m.usage).map(|v| v as f32);
        let (dr, dw) = disk_io_sum(s);
        let (nr, nw) = net_io_sum(s);
        let (cy0, cy1) = (PILL_ROW_CY[0], PILL_ROW_CY[1]);
        let (x0, x1, x2) = (PILL_COL_X[0], PILL_COL_X[1], PILL_COL_X[2]);

        // 列1：CPU（行1）/ 内存（行2），数值即百分比，无英文简写
        draw_pill_cell(
            px,
            font,
            self.icons.cpu.as_ref(),
            x0,
            cy0,
            None,
            &pill_pct(cpu),
            FG,
        );
        draw_pill_cell(
            px,
            font,
            self.icons.ram.as_ref(),
            x0,
            cy1,
            None,
            &pill_pct(mem),
            FG,
        );
        // 列2：硬盘读取（行1）/ 写入（行2），R 蓝、W 绿
        draw_pill_cell(
            px,
            font,
            self.icons.disk.as_ref(),
            x1,
            cy0,
            Some(("R", READ_BLUE)),
            &fmt::rate(dr),
            FG,
        );
        draw_pill_cell(
            px,
            font,
            self.icons.disk.as_ref(),
            x1,
            cy1,
            Some(("W", LEVEL_GREEN)),
            &fmt::rate(dw),
            FG,
        );
        // 列3：网络上行（行1）/ 下行（行2），UL 橙、DL 紫
        draw_pill_cell(
            px,
            font,
            self.icons.net.as_ref(),
            x2,
            cy0,
            Some(("UL", TX_LINE)),
            &fmt::rate(nw),
            FG,
        );
        draw_pill_cell(
            px,
            font,
            self.icons.net.as_ref(),
            x2,
            cy1,
            Some(("DL", DL_PURPLE)),
            &fmt::rate(nr),
            FG,
        );
    }
}

/// 药丸胶囊单元格：图标（可选，随行中线垂直居中）+ 彩色前缀（可选，其后留一个字符间距）
/// + 前景色数值。返回结束 x。
#[allow(clippy::too_many_arguments)]
fn draw_pill_cell(
    px: &mut Pixmap,
    font: &Font,
    icon: Option<&Pixmap>,
    x: f32,
    cy: f32,
    prefix: Option<(&str, Rgba)>,
    value: &str,
    vc: Rgba,
) -> f32 {
    let mut tx = x;
    if let Some(ic) = icon {
        let isz = ic.width() as f32;
        px.draw_pixmap(
            x as i32,
            (cy - isz / 2.0).round() as i32,
            ic.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
        tx = x + isz + 4.0;
    }
    let bl = cy + font.ascent(FONT_SIZE_SMALL) * 0.36;
    if let Some((p, pc)) = prefix {
        tx = font.draw(
            px,
            tx,
            bl,
            FONT_SIZE_SMALL,
            p,
            color(pc.0, pc.1, pc.2, pc.3),
        );
        // 前缀与数值之间保留一个字符间距
        tx += font.measure(FONT_SIZE_SMALL, "R");
    }
    font.draw(
        px,
        tx,
        bl,
        FONT_SIZE_SMALL,
        value,
        color(vc.0, vc.1, vc.2, vc.3),
    )
}

/// 药丸胶囊里的百分比文本（无数据为 `--%`）。
fn pill_pct(v: Option<f32>) -> String {
    v.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()))
}
