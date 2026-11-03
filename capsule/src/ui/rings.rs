//! 收起态三环：任务栏内横排三环 / 竖排纵向堆叠三环。

use super::*;

impl CapsuleView {
    /// 竖排收起态：三环纵向堆叠（CPU / 内存 / 网络），与任务栏三环同一套画法，
    /// 只是排列方向与尺寸不同。(ox, oy) 为左上角偏移——展开态要画在窗口侧边。
    pub(super) fn render_rings_v(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        ox: f32,
        oy: f32,
    ) {
        // 协商速率读不到时（WiFi 网卡没有 sysfs speed）网络环改用会话峰值做满环基准
        let (rx, tx) = net_io_sum(frame.snap);
        let decay = |p: u64, v: u64| (p * 99 / 100).max(v);
        self.net_peak = (decay(self.net_peak.0, rx), decay(self.net_peak.1, tx));
        fill_round(px, ox, oy, PILL_V_W as f32, PILL_V_H as f32, PILL_R, BG);
        draw_rings_v(px, frame, font, ox, oy, self.net_peak);
    }

    pub(super) fn render_rings(&mut self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        // 协商速率读不到时（WiFi 网卡没有 sysfs speed）网络环改用会话峰值做满环基准
        let (rx, tx) = net_io_sum(frame.snap);
        let decay = |p: u64, v: u64| (p * 99 / 100).max(v);
        self.net_peak = (decay(self.net_peak.0, rx), decay(self.net_peak.1, tx));
        draw_rings(px, frame, font, self.net_peak);
    }
}

/// 网络环两个百分比（上行、下行）：优先以网卡**协商速率**为分母；
/// 协商速率读不到时（WiFi 网卡无 sysfs speed、网卡 down）退化为以会话峰值为基准的相对刻度。
fn net_ring_pct(s: &Snapshot, peak: (u64, u64)) -> (Option<f32>, Option<f32>) {
    let (rx, tx) = net_io_sum(s);
    let speed = s
        .network
        .as_deref()
        .and_then(|v| v.iter().find_map(|n| n.speed_mbps.filter(|v| *v > 0)));
    let Some(speed) = speed else {
        // 未知协商速率：相对峰值（峰值恒 ≥ 当前值，结果落在 0~100）
        let rel =
            |v: u64, p: u64| Some(((v as f64 / p.max(1) as f64) * 100.0).clamp(0.0, 100.0) as f32);
        return (rel(tx, peak.1), rel(rx, peak.0));
    };
    // 协商速率是 Mbps（10^6 bit/s），换算成字节/秒作为满环基准
    let cap_bytes = speed as f64 * 1_000_000.0 / 8.0;
    let pct = |v: u64| Some(((v as f64 / cap_bytes) * 100.0).clamp(0.0, 100.0) as f32);
    (pct(tx), pct(rx))
}

/// 三圆环收起态：透明背景 + 横向三环（CPU / 内存 / 网络），数值与标注都收在环内。
/// 由任务栏内的子窗口直接调用，不依赖 View 实例。
pub(crate) fn draw_rings(
    px: &mut Pixmap,
    frame: &Frame,
    font: Option<&Font>,
    net_peak: (u64, u64),
) {
    let Some(font) = font else { return };
    // 三环直接浮在任务栏上，文字对比度取决于任务栏底色（不是应用主题）：
    // 深色任务栏用浅色标注 + 提亮后的分级色，亮色任务栏沿用主题配色。
    let dark_taskbar = frame.pinned && !frame.taskbar_light;
    let base = if dark_taskbar { PIN_FG_DIM } else { FG_DIM };
    let d = if dark_taskbar {
        base
    } else {
        text_c(frame.dark, base)
    };
    let dim = color(d.0, d.1, d.2, d.3);
    let tint = |c: (u8, u8, u8, u8)| if dark_taskbar { ring_text_color(c) } else { c };

    if !frame.has_data {
        // 无数据时只画一个居中的状态环（灰轨道）+ 状态文字。
        // 深色任务栏没有 apply_dark 兜底，这里必须直接取浅色文案。
        let (text, c) = status_text(frame.conn, frame.dark && !dark_taskbar);
        let cx = RINGS_W as f32 / 2.0;
        draw_ring(px, cx, RING_CY, RING_R, RING_STROKE, None, TRACK);
        draw_ring_inner(
            px,
            font,
            cx,
            RING_CY,
            &text,
            c,
            "",
            dim,
            RING_VALUE_FS,
            RING_LABEL_FS,
            false,
        );
        return;
    }

    let s = frame.snap;
    let cpu_pct = s.cpu.as_ref().and_then(|c| c.usage).map(|v| v as f32);
    let mem_pct = s.memory.as_ref().and_then(|m| m.usage).map(|v| v as f32);
    let (nr, nw) = net_io_sum(s);

    // CPU 环：数值（分色）+ 环内小字标注
    let cpu_color = cpu_pct.map_or(ACCENT, usage_color);
    let cpu_text_color = tint(cpu_color);
    draw_ring(
        px,
        RING_CX[0],
        RING_CY,
        RING_R,
        RING_STROKE,
        cpu_pct,
        cpu_color,
    );
    let cpu_text = cpu_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    draw_ring_inner(
        px,
        font,
        RING_CX[0],
        RING_CY,
        &cpu_text,
        color(
            cpu_text_color.0,
            cpu_text_color.1,
            cpu_text_color.2,
            cpu_text_color.3,
        ),
        "CPU",
        dim,
        RING_VALUE_FS,
        RING_LABEL_FS,
        false,
    );

    // 内存环
    let mem_color = mem_pct.map_or(ACCENT, usage_color);
    let mem_text_color = tint(mem_color);
    draw_ring(
        px,
        RING_CX[1],
        RING_CY,
        RING_R,
        RING_STROKE,
        mem_pct,
        mem_color,
    );
    let mem_text = mem_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    draw_ring_inner(
        px,
        font,
        RING_CX[1],
        RING_CY,
        &mem_text,
        color(
            mem_text_color.0,
            mem_text_color.1,
            mem_text_color.2,
            mem_text_color.3,
        ),
        "内存",
        dim,
        RING_VALUE_FS,
        RING_LABEL_FS,
        false,
    );

    // 网络环：上半＝上行（橙）、下半＝下行（紫）。分母优先取网卡协商速率，
    // 读不到时退化为会话峰值相对刻度。环内两行数值不再带 UL/DL 前缀——
    // 方向由颜色（上橙下紫）与所在半环表达，腾出的宽度用于放大字号
    let (ul_pct, dl_pct) = net_ring_pct(s, net_peak);
    draw_ring_split(
        px,
        RING_CX[2],
        RING_CY,
        RING_R,
        RING_STROKE,
        ul_pct,
        TX_LINE,
        dl_pct,
        DL_PURPLE,
    );
    draw_net_inner(
        px,
        font,
        RING_CX[2],
        RING_CY,
        &fmt::rate_compact(nw),
        &fmt::rate_compact(nr),
        RING_NET_FS,
        false,
        false,
    );
}

/// 竖排收起态：三环纵向堆叠（CPU / 内存 / 网络），环更大、字号相应放大。
/// 与任务栏三环共用 draw_ring / draw_ring_split / 环内文字的画法。
pub(crate) fn draw_rings_v(
    px: &mut Pixmap,
    frame: &Frame,
    font: Option<&Font>,
    ox: f32,
    oy: f32,
    net_peak: (u64, u64),
) {
    let Some(font) = font else { return };
    // 暗色主题下环内标注统一提白（同 draw_rings）
    let d = text_c(frame.dark, FG_DIM);
    let dim = color(d.0, d.1, d.2, d.3);
    let cx = ox + RING_V_CX;

    if !frame.has_data {
        // 无数据：中环画灰轨道，环内放短状态文案（竖条窄，长句放不下）
        let (text, c) = status_text_short(frame.conn, frame.dark);
        let cy = oy + RING_V_CY[1];
        draw_ring(px, cx, cy, RING_V_R, RING_V_STROKE, None, TRACK);
        let tw = font.measure_bold(RING_V_NET_FS, &text);
        font.draw_bold(
            px,
            cx - tw / 2.0,
            cy + RING_V_NET_FS * 0.36,
            RING_V_NET_FS,
            &text,
            c,
        );
        return;
    }

    let s = frame.snap;
    let cpu_pct = s.cpu.as_ref().and_then(|c| c.usage).map(|v| v as f32);
    let mem_pct = s.memory.as_ref().and_then(|m| m.usage).map(|v| v as f32);
    let (nr, nw) = net_io_sum(s);

    // CPU 环
    let cpu_color = cpu_pct.map_or(ACCENT, usage_color);
    let cy0 = oy + RING_V_CY[0];
    draw_ring(px, cx, cy0, RING_V_R, RING_V_STROKE, cpu_pct, cpu_color);
    let cpu_text = cpu_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    let c0 = text_c(frame.dark, cpu_color);
    draw_ring_inner(
        px,
        font,
        cx,
        cy0,
        &cpu_text,
        color(c0.0, c0.1, c0.2, c0.3),
        "CPU",
        dim,
        RING_V_VALUE_FS,
        RING_V_LABEL_FS,
        true,
    );

    // 内存环
    let mem_color = mem_pct.map_or(ACCENT, usage_color);
    let cy1 = oy + RING_V_CY[1];
    draw_ring(px, cx, cy1, RING_V_R, RING_V_STROKE, mem_pct, mem_color);
    let mem_text = mem_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    let m1 = text_c(frame.dark, mem_color);
    draw_ring_inner(
        px,
        font,
        cx,
        cy1,
        &mem_text,
        color(m1.0, m1.1, m1.2, m1.3),
        "内存",
        dim,
        RING_V_VALUE_FS,
        RING_V_LABEL_FS,
        true,
    );

    // 网络环：上半橙＝上行、下半紫＝下行
    let (ul_pct, dl_pct) = net_ring_pct(s, net_peak);
    let cy2 = oy + RING_V_CY[2];
    draw_ring_split(
        px,
        cx,
        cy2,
        RING_V_R,
        RING_V_STROKE,
        ul_pct,
        TX_LINE,
        dl_pct,
        DL_PURPLE,
    );
    draw_net_inner(
        px,
        font,
        cx,
        cy2,
        &fmt::rate_compact(nw),
        &fmt::rate_compact(nr),
        RING_V_NET_FS,
        true,
        frame.dark,
    );
}

/// 环内文字：上行数值（分色）+ 下行灰色小字标注，两行都收在环内。
/// 字号由调用方给定（横排三环小、竖排三环大），行位按字号比例排布，大小环都居中；
/// `bold` 供竖排三环用粗体（环小、字要顶得住）。
#[allow(clippy::too_many_arguments)]
fn draw_ring_inner(
    px: &mut Pixmap,
    font: &Font,
    cx: f32,
    cy: f32,
    value: &str,
    vc: Color,
    label: &str,
    dim: Color,
    value_fs: f32,
    label_fs: f32,
    bold: bool,
) {
    let measure = |size: f32, t: &str| {
        if bold {
            font.measure_bold(size, t)
        } else {
            font.measure(size, t)
        }
    };
    let put = |px: &mut Pixmap, x: f32, baseline: f32, size: f32, t: &str, c: Color| {
        if bold {
            font.draw_bold(px, x, baseline, size, t, c);
        } else {
            font.draw(px, x, baseline, size, t, c);
        }
    };
    let vw = measure(value_fs, value);
    put(px, cx - vw / 2.0, cy - value_fs * 0.17, value_fs, value, vc);
    if !label.is_empty() {
        let lw = measure(label_fs, label);
        put(
            px,
            cx - lw / 2.0,
            cy + label_fs * 1.45,
            label_fs,
            label,
            dim,
        );
    }
}

/// 网络环内文字：上行（橙）/ 下行（紫）两行速率数值，行位置与半环颜色一一对应。
/// `dark`（仅竖排环用）：暗色下文字统一走中性色，随后被 apply_dark 提白。
fn draw_net_inner(
    px: &mut Pixmap,
    font: &Font,
    cx: f32,
    cy: f32,
    ul: &str,
    dl: &str,
    fs: f32,
    bold: bool,
    dark: bool,
) {
    let cu = text_c(dark, TX_LINE);
    let cd = text_c(dark, DL_PURPLE);
    let measure = |t: &str| {
        if bold {
            font.measure_bold(fs, t)
        } else {
            font.measure(fs, t)
        }
    };
    let put = |px: &mut Pixmap, x: f32, baseline: f32, t: &str, c: Color| {
        if bold {
            font.draw_bold(px, x, baseline, fs, t, c);
        } else {
            font.draw(px, x, baseline, fs, t, c);
        }
    };
    let uw = measure(ul);
    let dw = measure(dl);
    put(
        px,
        cx - uw / 2.0,
        cy - fs * 0.17,
        ul,
        color(cu.0, cu.1, cu.2, cu.3),
    );
    put(
        px,
        cx - dw / 2.0,
        cy + fs * 0.87,
        dl,
        color(cd.0, cd.1, cd.2, cd.3),
    );
}
