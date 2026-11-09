//! 旧版四主题的版式复刻：顶栏 + 圆环卡片（dark/light 单列；bignum-* 双列）。
//!
//! 与 `pipeline` 的通用网格不同，这里按旧 `screen.ps1` 的排版重画，
//! 主题通过 `render = "legacy-single" | "legacy-dual"` 选用。

use screen_core::{
    Color, ComponentKind, FrameBuffer, OverlayMetric, RateUnit, ScreenProfile, SlotAlign,
    ThemeConfig,
};
use tiny_skia::PathBuilder;

use crate::canvas::Canvas;
use crate::error::RenderError;
use crate::text::TextRenderer;

/// 卡片圆角半径（与旧 `screen.ps1` 的 `radius = 10` 对齐）。
const CARD_RADIUS: f32 = 10.0;

/// 旧版卡片：圆角 + 1px 描边（描边色由卡片色向文字色混 10% 得到）。
fn draw_card(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Result<(), RenderError> {
    if let Some(path) = round_rect_path(x, y, w, h, CARD_RADIUS) {
        canvas.fill_path(&path, card_border(theme))?;
    }
    let inset = 1.0;
    if let Some(path) = round_rect_path(
        x + inset,
        y + inset,
        (w - inset * 2.0).max(1.0),
        (h - inset * 2.0).max(1.0),
        (CARD_RADIUS - inset).max(1.0),
    ) {
        canvas.fill_path(&path, theme.palette.card)?;
    }
    Ok(())
}

fn card_border(theme: &ThemeConfig) -> Color {
    let mix = |a: u8, b: u8| ((a as u32 * 90 + b as u32 * 10) / 100) as u8;
    Color::new(
        mix(theme.palette.card.r, theme.palette.text.r),
        mix(theme.palette.card.g, theme.palette.text.g),
        mix(theme.palette.card.b, theme.palette.text.b),
        255,
    )
}

fn round_rect_path(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    if !(w > 0.0) || !(h > 0.0) {
        return None;
    }
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x + w - r, y);
    b.quad_to(x + w, y, x + w, y + r);
    b.line_to(x + w, y + h - r);
    b.quad_to(x + w, y + h, x + w - r, y + h);
    b.line_to(x + r, y + h);
    b.quad_to(x, y + h, x, y + h - r);
    b.line_to(x, y + r);
    b.quad_to(x, y, x + r, y);
    b.close();
    b.finish()
}

/// 一帧里旧版版式需要的全部数据；缺失即按 `--` 绘制。
#[derive(Debug, Clone, Default)]
pub struct LegacyData {
    pub host: Option<String>,
    pub addr: Option<String>,
    pub online: bool,
    pub cpu: Option<f32>,
    pub cpu_temp: Option<f32>,
    pub mem: Option<f32>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    pub gpu: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub up: Option<u64>,
    pub down: Option<u64>,
    pub read: Option<u64>,
    pub write: Option<u64>,
    /// 电脑本机（双显右列）
    pub pc_cpu: Option<f32>,
    pub pc_mem: Option<f32>,
    pub pc_gpu: Option<f32>,
    pub pc_gpu_temp: Option<f32>,
    /// 电脑本机网络：收 / 发速率（字节每秒）
    pub pc_rx: Option<u64>,
    pub pc_tx: Option<u64>,
}

/// 旧版版式入口。`dual` 由主题 `render` 字段决定。
pub fn render_legacy(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    data: &LegacyData,
    visible: &[ComponentKind],
    text: Option<&mut dyn TextRenderer>,
) -> Result<FrameBuffer, RenderError> {
    render_legacy_bg(profile, theme, data, visible, text, None)
}

/// 带背景图的旧版版式。卡片是半透明的，因此背景图会从卡片下方透出。
pub fn render_legacy_bg(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    data: &LegacyData,
    visible: &[ComponentKind],
    mut text: Option<&mut dyn TextRenderer>,
    background: Option<&crate::image::BackgroundImage>,
) -> Result<FrameBuffer, RenderError> {
    let mut canvas = crate::pipeline::paint_background(profile, theme, background)?;
    if theme.overlay.is_some() || theme.render.as_deref() == Some("legacy-overlay") {
        draw_overlay(&mut canvas, theme, data, &mut text)?;
        return canvas.into_frame();
    }
    let dual = theme.render.as_deref() == Some("legacy-dual");
    if dual {
        draw_top_bar(&mut canvas, theme, data, &mut text, true)?;
        if profile.orientation == screen_core::Orientation::Portrait {
            draw_dual_portrait(&mut canvas, theme, data, visible, &mut text)?;
        } else {
            draw_dual_landscape(&mut canvas, theme, data, visible, &mut text)?;
        }
    } else {
        draw_top_bar(&mut canvas, theme, data, &mut text, false)?;
        if profile.orientation == screen_core::Orientation::Portrait {
            draw_single_portrait(&mut canvas, theme, data, visible, &mut text)?;
        } else {
            draw_single_landscape(&mut canvas, theme, data, visible, &mut text)?;
        }
    }
    canvas.into_frame()
}

// ---------------------------------------------------------------------------
// 官方皮肤：整屏底图 + 只填数值
// ---------------------------------------------------------------------------

/// `render = "legacy-overlay"` 或主题带 `overlay.slots`：底图已经把圆环、卡片、图标都画好了，
/// 这里只按槽位声明贴数值，不绘制任何几何图形。
fn draw_overlay(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let Some(overlay) = theme.overlay.as_ref() else {
        return draw_overlay_preset(canvas, theme, data, text);
    };
    // 横竖两套版式的数值位置不同：优先用按方向声明的槽位。
    let orientation = if canvas.width() >= canvas.height() {
        "landscape"
    } else {
        "portrait"
    };
    let slots = overlay.variants.get(orientation).unwrap_or(&overlay.slots);
    for slot in slots {
        let Some(mut value) = overlay_value(slot.metric, data, slot.decimals, slot.rate_unit)
        else {
            continue;
        };
        if let Some(suffix) = slot.suffix.as_deref() {
            value.push_str(suffix);
        }
        let color = slot.color.unwrap_or(theme.palette.text);
        let box_w = if slot.w > 0.0 { slot.w } else { 240.0 };
        let tw = estimate_text_width(&value, slot.size);
        let x = match slot.align {
            SlotAlign::Left => slot.x,
            SlotAlign::Right => slot.x + (box_w - tw).max(0.0),
            SlotAlign::Center => slot.x + ((box_w - tw) / 2.0).max(0.0),
        };
        draw(canvas, text, &value, x, slot.y, box_w, slot.size, color)?;
    }
    Ok(())
}

/// 旧预设：主题没写 `overlay.slots` 时，按 320×480 官方三环底图的固定环心贴数值。
fn draw_overlay_preset(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let portrait = h > w;

    // (数值, 环心 x, 环心 y, 数值字号, 标签字号)
    let rings: [(Option<f32>, f32, f32, f32, f32); 3] = if portrait {
        [
            (data.cpu, 103.0, 262.0, 32.0, 10.0),
            (data.mem, 243.0, 148.0, 30.0, 10.0),
            (data.gpu, 206.0, 348.0, 32.0, 10.0),
        ]
    } else {
        [
            (data.cpu, 80.0, 150.0, 30.0, 10.0),
            (data.mem, 240.0, 150.0, 30.0, 10.0),
            (data.gpu, 400.0, 150.0, 30.0, 10.0),
        ]
    };
    for (value, cx, cy, vsize, lsize) in rings {
        let Some(v) = value else { continue };
        let s = format!("{v:.0}%");
        let x = cx - estimate_text_width(&s, vsize) / 2.0;
        draw(
            canvas,
            text,
            &s,
            x,
            cy - vsize,
            200.0,
            vsize,
            theme.palette.text,
        )?;
        let lx = cx - estimate_text_width("使用率", lsize) / 2.0;
        draw(
            canvas,
            text,
            "使用率",
            lx,
            cy + 8.0,
            200.0,
            lsize,
            theme.palette.dim,
        )?;
    }

    // 底部一行：网络 / 硬盘速率
    let y = h - if portrait { 36.0 } else { 22.0 };
    let line = format!(
        "网 {} / {}    盘 {} / {}",
        rate(data.down),
        rate(data.up),
        rate(data.read),
        rate(data.write)
    );
    draw(
        canvas,
        text,
        &line,
        10.0,
        y,
        w - 20.0,
        12.0,
        theme.palette.dim,
    )?;
    Ok(())
}

/// 自适应 MB/s：小数位按量级收放，字符串长度稳定在 5 字符左右。
///
/// `<10` 三位小数、`10–99` 两位、`100–999` 一位、`≥1000` 整数；
/// 有流量但不足 0.001 MB/s 时显示 `0.001`，完全空闲显示 `0.0`。
fn format_mb_fit(bytes: u64) -> String {
    // 完全空闲固定显示 0.0，避免和「有流量但极小」的 0.001 混淆。
    if bytes == 0 {
        return "0.0".to_string();
    }
    let mb = bytes as f64 / 1024.0 / 1024.0;
    let decimals = if mb < 10.0 {
        3
    } else if mb < 100.0 {
        2
    } else if mb < 1000.0 {
        1
    } else {
        0
    };
    let shown = if mb < 0.001 { 0.001 } else { mb };
    format!("{shown:.decimals$}")
}

/// 数值槽取值。没有数据的指标返回 `None`，整槽留空（不填 0、不写 `--`）。
fn overlay_value(
    metric: OverlayMetric,
    data: &LegacyData,
    decimals: u8,
    rate_unit: RateUnit,
) -> Option<String> {
    let d = decimals as usize;
    let num = |v: f32| format!("{:.*}", d, v);
    let gb = |b: u64| format!("{:.*}", d, b as f64 / 1024.0 / 1024.0 / 1024.0);
    // 速率的单位口径由主题决定：底图印了 MB/s 时只贴数字。
    let rate = |b: u64| match rate_unit {
        RateUnit::Mb => format!("{:.*}", d, b as f64 / 1024.0 / 1024.0),
        RateUnit::MbFit => format_mb_fit(b),
        RateUnit::Auto => crate::metrics::format_rate(b),
    };
    match metric {
        OverlayMetric::Cpu => data.cpu.map(num),
        OverlayMetric::CpuTemp => data.cpu_temp.map(num),
        OverlayMetric::Gpu => data.gpu.map(num),
        OverlayMetric::GpuTemp => data.gpu_temp.map(num),
        OverlayMetric::Memory => data.mem.map(num),
        OverlayMetric::MemoryUsed => data.mem_used.map(gb),
        OverlayMetric::MemoryTotal => data.mem_total.map(gb),
        OverlayMetric::NetworkDown => data.down.map(rate),
        OverlayMetric::NetworkUp => data.up.map(rate),
        OverlayMetric::DiskRead => data.read.map(rate),
        OverlayMetric::DiskWrite => data.write.map(rate),
        OverlayMetric::Host => data.host.clone(),
    }
}

// ---------------------------------------------------------------------------
// 顶栏
// ---------------------------------------------------------------------------

fn draw_top_bar(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    text: &mut Option<&mut dyn TextRenderer>,
    one_line: bool,
) -> Result<(), RenderError> {
    let w = canvas.width() as f32;
    let color_text = theme.palette.text;
    let color_dim = theme.palette.dim;
    let host = data.host.as_deref().unwrap_or("--");
    let label = if data.online {
        "已连接"
    } else {
        "未连接"
    };
    let color = if data.online {
        theme.palette.accent
    } else {
        theme.palette.danger
    };

    // 横屏：主机名 / IP / 状态 同一行、同为 19px；状态最右，IP 紧挨其左
    if !one_line && w >= 400.0 {
        let size = 19.0;
        let y = 5.0;
        // 「已连接 / 未连接」都是 3 个汉字，19px 约 57px 宽
        let label_left = w - 10.0 - 57.0;
        let ip_right = label_left - 19.0;
        let ip_max = 220.0;
        let host_max = (ip_right - ip_max - 10.0).max(40.0);
        draw(canvas, text, host, 10.0, y, host_max, size, color_text)?;
        if let Some(addr) = data.addr.as_deref() {
            draw_right(canvas, text, addr, ip_right, y, ip_max, size, color_dim)?;
        }
        draw_right(canvas, text, label, w - 10.0, y, 80.0, size, color_dim)?;
        canvas.fill_rect(
            screen_core::Rect::new(label_left - 11.0, y + 6.5, 6.0, 6.0),
            color,
        )?;
        return Ok(());
    }

    draw(canvas, text, host, 10.0, 1.0, 150.0, 17.0, color_text)?;
    // 双显主题顶栏只有一行（主机名 + 连接状态），地址会与「已连接」相撞，按旧版不画
    if !one_line {
        if let Some(addr) = data.addr.as_deref() {
            let addr_x = if w >= 400.0 { w - 320.0 } else { w - 190.0 };
            draw(canvas, text, addr, addr_x, 4.0, 180.0, 12.0, color_dim)?;
        }
    }
    // 连接状态：圆点 + 文字
    let y = if one_line { 4.0 } else { 24.0 };
    draw(canvas, text, label, w - 62.0, y, 52.0, 11.0, color_dim)?;
    canvas.fill_rect(screen_core::Rect::new(w - 72.0, y + 3.0, 6.0, 6.0), color)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 单列（dark / light）
// ---------------------------------------------------------------------------

fn draw_single_portrait(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    visible: &[ComponentKind],
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let n = visible.len();
    if n == 0 {
        return Ok(());
    }
    // 少于 3 项时仍按 3 项的行高布局：卡片不会被拉得过大，内容也在此封顶。
    let rows = n.max(3);
    let (top, bottom, side, gap) = (40.0, 14.0, 10.0, 6.0);
    let ch = ((h - top - bottom - (rows as f32 - 1.0) * gap) / rows as f32).floor();
    for (i, kind) in visible.iter().enumerate() {
        let x = side;
        let y = top + i as f32 * (ch + gap);
        let cw = w - side * 2.0;
        draw_card(canvas, theme, x, y, cw, ch)?;
        match kind {
            ComponentKind::Cpu | ComponentKind::Gpu => {
                draw_ring_pair(canvas, theme, data, *kind, x, y, cw, ch, text)?
            }
            ComponentKind::Memory => draw_memory_card(canvas, theme, data, x, y, cw, ch, text)?,
            ComponentKind::Network | ComponentKind::Disk => {
                draw_flow_card(canvas, theme, data, *kind, x, y, cw, ch, text)?
            }
            ComponentKind::Temp | ComponentKind::Clock | ComponentKind::Storage => {
                draw_simple_card(canvas, theme, data, *kind, x, y, cw, ch, text)?
            }
        }
    }
    Ok(())
}

fn draw_single_landscape(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    visible: &[ComponentKind],
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let n = visible.len();
    if n == 0 {
        return Ok(());
    }
    let cols = 3.min(n);
    let rows = n.div_ceil(cols);
    let (top, bottom, side, gap) = (40.0, 8.0, 10.0, 6.0);
    let ch = ((h - top - bottom - (rows as f32 - 1.0) * gap) / rows as f32).floor();
    for (i, kind) in visible.iter().enumerate() {
        let r = i / cols;
        let c = i % cols;
        let in_row = cols.min(n - r * cols);
        let cw = ((w - side * 2.0 - (in_row as f32 - 1.0) * gap) / in_row as f32).floor();
        let x = side + c as f32 * (cw + gap);
        let y = top + r as f32 * (ch + gap);
        draw_card(canvas, theme, x, y, cw, ch)?;
        draw_landscape_card(canvas, theme, data, *kind, x, y, cw, ch, text)?;
    }
    Ok(())
}

/// 横屏卡片：标题左上、温度右上、圆环居中。
fn draw_landscape_card(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    kind: ComponentKind,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    // 横屏的「网络 / 硬盘」卡沿用旧版文字速率，不画圆环
    if matches!(kind, ComponentKind::Network | ComponentKind::Disk) {
        let (title, a, b, label_a, label_b) = if kind == ComponentKind::Network {
            ("网络", data.down, data.up, "收", "发")
        } else {
            ("硬盘", data.read, data.write, "读", "写")
        };
        // 标题与数值统一贴卡片左侧；字号加大到 24px
        let text_x = x + 10.0;
        draw(
            canvas,
            text,
            title,
            text_x,
            y + 6.0,
            w - 20.0,
            15.0,
            theme.palette.dim,
        )?;
        draw(
            canvas,
            text,
            &format!("{label_a} {}", rate(a)),
            text_x,
            y + 40.0,
            w - 20.0,
            24.0,
            theme.palette.accent,
        )?;
        draw(
            canvas,
            text,
            &format!("{label_b} {}", rate(b)),
            text_x,
            y + 74.0,
            w - 20.0,
            24.0,
            theme.palette.warn,
        )?;
        return Ok(());
    }
    let (title, pct, temp) = card_values(data, kind);
    // 右上角有内容（温度 / 内存用量）时缩窄标题可用宽度
    let title_w = if temp.is_some() || kind == ComponentKind::Memory {
        w - 80.0
    } else {
        w - 20.0
    };
    draw(
        canvas,
        text,
        &title,
        x + 10.0,
        y + 6.0,
        title_w,
        15.0,
        theme.palette.dim,
    )?;
    if kind == ComponentKind::Memory {
        // 已用/总量移到右上角，与温度同位；紧凑横排
        let line = match (data.mem_used, data.mem_total) {
            (Some(u), Some(t)) => format!("{}/{}", gb_short(u), gb_short(t)),
            (None, Some(t)) => format!("--/{}", gb_short(t)),
            _ => "--".to_string(),
        };
        draw_right(
            canvas,
            text,
            &line,
            x + w - 10.0,
            y + 3.0,
            90.0,
            15.0,
            theme.palette.dim,
        )?;
    }
    if let Some(t) = temp {
        // 温度固定在卡片右上角，右对齐；13px → 17px
        draw_right(
            canvas,
            text,
            &format!("{t:.0}°"),
            x + w - 10.0,
            y + 3.0,
            60.0,
            17.0,
            temp_color(theme, t),
        )?;
    }
    // 三类卡圆环统一加大：半径 h*0.36（原 0.30 / 0.26），环内数值 26px
    // 标签下移到 9px，避免与 26px 数字底沿贴在一起
    let r = (h * 0.36).clamp(34.0, 50.0);
    let cy = y + h / 2.0 + 6.0;
    draw_ring_value(
        canvas,
        theme,
        text,
        x + w / 2.0,
        cy,
        r,
        8.0,
        26.0,
        13.0,
        9.0,
        pct,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 双列（bignum-*）
// ---------------------------------------------------------------------------

fn dual_kinds(visible: &[ComponentKind]) -> Vec<ComponentKind> {
    visible
        .iter()
        .copied()
        .filter(|k| {
            matches!(
                k,
                ComponentKind::Cpu
                    | ComponentKind::Memory
                    | ComponentKind::Gpu
                    | ComponentKind::Network
            )
        })
        .collect()
}

fn draw_dual_portrait(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    visible: &[ComponentKind],
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let kinds = dual_kinds(visible);
    if kinds.is_empty() {
        return Ok(());
    }
    let (left, gap, tw, top) = (10.0, 8.0, 146.0, 44.0);
    let rows = kinds.len();
    let th = ((h - top - 8.0 - (rows as f32 - 1.0) * gap) / rows as f32).floor();
    let rx = left + tw + gap;
    let rw = w - rx - left;
    for (i, kind) in kinds.iter().enumerate() {
        let y = top + i as f32 * (th + gap);
        // 左列：NAS
        draw_card(canvas, theme, left, y, tw, th)?;
        draw_dual_card(
            canvas, theme, text, *kind, data, left, y, tw, th, false, false,
        )?;
        // 右列：电脑
        draw_card(canvas, theme, rx, y, rw, th)?;
        draw_dual_card(canvas, theme, text, *kind, data, rx, y, rw, th, true, false)?;
    }
    Ok(())
}

fn draw_dual_landscape(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    visible: &[ComponentKind],
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let kinds = dual_kinds(visible);
    if kinds.is_empty() {
        return Ok(());
    }
    let (pad, gap, top) = (10.0, 8.0, 36.0);
    let n = kinds.len() as f32;
    let cw = ((w - pad * 2.0 - (n - 1.0) * gap) / n).floor();
    let rh = ((h - top - 8.0 - gap) / 2.0).floor();
    for row in 0..2 {
        let ry = top + row as f32 * (rh + gap);
        for (i, kind) in kinds.iter().enumerate() {
            let x = pad + i as f32 * (cw + gap);
            draw_card(canvas, theme, x, ry, cw, rh)?;
            draw_dual_card(
                canvas,
                theme,
                text,
                *kind,
                data,
                x,
                ry,
                cw,
                rh,
                row == 1,
                true,
            )?;
        }
    }
    Ok(())
}

/// 双显卡片：`local=false` 画 NAS（左/上排），`local=true` 画电脑（右/下排）。
#[allow(clippy::too_many_arguments)]
fn draw_dual_card(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    text: &mut Option<&mut dyn TextRenderer>,
    kind: ComponentKind,
    data: &LegacyData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    local: bool,
    landscape: bool,
) -> Result<(), RenderError> {
    let (base, pct, temp) = match kind {
        ComponentKind::Cpu => ("CPU", data.cpu, data.cpu_temp),
        ComponentKind::Memory => ("RAM", data.mem, None),
        ComponentKind::Gpu => ("GPU", data.gpu, data.gpu_temp),
        _ => ("网络", None, None),
    };
    let name = if local {
        format!("电脑 {base}")
    } else {
        base.to_string()
    };
    if kind == ComponentKind::Network {
        // 双显右列「电脑 网络」用本机网卡速率；NAS 列仍用 NAS 上报值
        let (up, down) = if local {
            (data.pc_rx, data.pc_tx)
        } else {
            (data.up, data.down)
        };
        // 收发行字号：横屏 18px、竖屏 18px（原 16/15），极窄卡片才回退 14px。
        let (name_size, rate_size, name_y, line1_y, line2_y, inset) = if landscape {
            (12.0, 18.0, 8.0, 30.0, 62.0, 4.0)
        } else if w >= 130.0 {
            (13.0, 18.0, 8.0, 32.0, 60.0, 8.0)
        } else {
            (12.0, 14.0, 8.0, 32.0, 58.0, 6.0)
        };
        draw(
            canvas,
            text,
            &name,
            x + inset,
            y + name_y,
            w - inset * 2.0,
            name_size,
            theme.palette.dim,
        )?;
        let line1 = format!("收 {}", rate(up));
        let line2 = format!("发 {}", rate(down));
        draw(
            canvas,
            text,
            &line1,
            x + inset,
            y + line1_y,
            w - inset * 2.0,
            rate_size,
            theme.palette.accent,
        )?;
        draw(
            canvas,
            text,
            &line2,
            x + inset,
            y + line2_y,
            w - inset * 2.0,
            rate_size,
            theme.palette.warn,
        )?;
        return Ok(());
    }
    let (pct, temp) = if local {
        match kind {
            ComponentKind::Cpu => (data.pc_cpu, None),
            ComponentKind::Memory => (data.pc_mem, None),
            ComponentKind::Gpu => (data.pc_gpu, data.pc_gpu_temp),
            _ => (None, None),
        }
    } else {
        (pct, temp)
    };
    draw(
        canvas,
        text,
        &name,
        x + 8.0,
        y + 8.0,
        w - 44.0,
        if w >= 130.0 { 13.0 } else { 12.0 },
        theme.palette.dim,
    )?;
    if let Some(t) = temp {
        draw(
            canvas,
            text,
            &format!("{t:.0}°"),
            x + w - 40.0,
            y + 4.0,
            36.0,
            17.0,
            temp_color(theme, t),
        )?;
    }
    // 数值加大（竖屏 48→56、横屏 32→48）；百分号保持原绝对字号不动。
    let (size, pct_size) = if landscape {
        (48.0, 16.0)
    } else {
        (56.0, 24.0)
    };
    match pct {
        Some(v) => {
            // 数字用大字号，`%` 按基线对齐，避免百分号抢视觉。
            let color = usage_color(theme, v);
            let num = format!("{:.0}", v.clamp(0.0, 100.0));
            let num_w = text_width(text, &num, size);
            let pct_w = text_width(text, "%", pct_size);
            let left = x + (w - (num_w + pct_w)) / 2.0;
            let top = y + h * 0.30;
            draw(canvas, text, &num, left, top, w - 8.0, size, color)?;
            draw(
                canvas,
                text,
                "%",
                left + num_w,
                top + (size - pct_size),
                pct_w.max(1.0),
                pct_size,
                color,
            )?;
        }
        None => {
            draw_centered(
                canvas,
                text,
                "--",
                x + w / 2.0,
                y + h * 0.30,
                w - 8.0,
                size,
                theme.palette.dim,
            )?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 卡片内容
// ---------------------------------------------------------------------------

/// 竖屏单列内容缩放：5 项时卡片高约 80px，项目减少后按行高等比放大（上限 1.55 倍）。
fn single_scale(h: f32) -> f32 {
    (h / 80.0).clamp(1.0, 1.55)
}

/// CPU / GPU：名称 + 使用率环 + 温度环。
#[allow(clippy::too_many_arguments)]
fn draw_ring_pair(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    kind: ComponentKind,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (title, pct, temp) = card_values(data, kind);
    let s = single_scale(h);
    // 名称只跟着放大一点点（上限 1.3 倍）：3 项时行高变大，标题不必和环内数字一样夸张
    let title_size = 16.0 * s.min(1.3);
    // 环随行高放大，但上限压低：环左缘必须留在标题文字右侧，避免大字号时叠字。
    let r2 = ((h - 12.0) / 2.0).min(34.0 + 12.0 * (s - 1.0));
    let ring_w = 8.0 * s;
    let cy = y + h / 2.0;
    let cx2 = x + w - 16.0 * s - r2;
    let cx1 = cx2 - 2.0 * r2 - 14.0 * s;
    // 环内文字放大要收敛：上限 1.25 倍，并受环内径约束，避免「使用率 / 数值」压到环壁上。
    let inner = (r2 - ring_w).max(1.0);
    let value_size = (16.0 * s.min(1.25)).min((inner * 0.85).max(16.0));
    let label_size = (11.0 * s.min(1.25)).min((inner * 0.45).max(11.0));
    // 标题宽度止于左环左缘，大字时不会压到环上
    let title_x = x + 14.0 * s;
    let title_w = (cx1 - r2 - title_x - 6.0).max(48.0);
    draw(
        canvas,
        text,
        &title,
        title_x,
        y + 10.0 * s,
        title_w,
        title_size,
        theme.palette.dim,
    )?;
    draw_ring_value(
        canvas,
        theme,
        text,
        cx1,
        cy,
        r2,
        ring_w,
        value_size,
        label_size,
        label_size * 0.20,
        pct,
    )?;
    // 温度环：无数据时只画轨道
    let temp_pct = temp.unwrap_or(0.0);
    draw_ring_track(canvas, theme, cx2, cy, r2, ring_w)?;
    if temp.is_some() {
        canvas.draw_ring(
            cx2,
            cy,
            r2,
            ring_w,
            0.0,
            temp_pct.clamp(0.0, 100.0) * 3.6,
            temp_color(theme, temp_pct),
        )?;
        draw_centered(
            canvas,
            text,
            &format!("{temp_pct:.0}°"),
            cx2,
            cy - value_size * 0.80,
            r2 * 2.0,
            value_size,
            theme.palette.text,
        )?;
    }
    draw_centered(
        canvas,
        text,
        "温度",
        cx2,
        cy + label_size * 0.20,
        r2 * 2.0,
        label_size,
        theme.palette.dim,
    )?;
    Ok(())
}

fn draw_memory_card(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let s = single_scale(h);
    // 与双环卡一致：名称只轻微放大
    let title_size = 16.0 * s.min(1.3);
    let r2 = ((h - 12.0) / 2.0).min(34.0 + 12.0 * (s - 1.0));
    let ring_w = 8.0 * s;
    let cy = y + h / 2.0;
    let cx = x + w - 16.0 * s - r2;
    let title_x = x + 14.0 * s;
    let title_w = (cx - r2 - title_x - 6.0).max(80.0);
    draw(
        canvas,
        text,
        "内存",
        title_x,
        y + 12.0 * s,
        title_w,
        title_size,
        theme.palette.dim,
    )?;
    let line = match (data.mem_used, data.mem_total) {
        (Some(u), Some(t)) => format!("{} / {}", gb_short(u), gb_short(t)),
        (None, Some(t)) => format!("-- / {}", gb_short(t)),
        _ => "--".to_string(),
    };
    draw(
        canvas,
        text,
        &line,
        title_x,
        y + 38.0 * s,
        title_w,
        12.0 * s,
        theme.palette.dim,
    )?;
    // 与双环卡一致：环内文字最多放大 1.25 倍，并受内径约束
    let inner = (r2 - ring_w).max(1.0);
    let value_size = (16.0 * s.min(1.25)).min((inner * 0.85).max(16.0));
    let label_size = (11.0 * s.min(1.25)).min((inner * 0.45).max(11.0));
    draw_ring_value(
        canvas,
        theme,
        text,
        cx,
        cy,
        r2,
        ring_w,
        value_size,
        label_size,
        label_size * 0.20,
        data.mem,
    )?;
    Ok(())
}

fn draw_flow_card(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    kind: ComponentKind,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (title, a, b, label_a, label_b) = if kind == ComponentKind::Network {
        ("网络", data.down, data.up, "收", "发")
    } else {
        ("硬盘", data.read, data.write, "读", "写")
    };
    let wide = w >= 200.0;
    // 项目减少、卡片变高时字号跟着放大；速率行受列宽限制，封顶 23px 防截断。
    let s = single_scale(h);
    let (title_size, value_size) = if wide {
        ((16.0 * s).min(19.0), (20.0 * s).min(23.0))
    } else {
        ((13.0 * s).min(16.0), (17.0 * s).min(20.0))
    };
    draw(
        canvas,
        text,
        title,
        x + 12.0,
        y + if wide { 4.0 * s } else { 6.0 * s },
        w - 24.0,
        title_size,
        theme.palette.dim,
    )?;
    let v1 = format!("{label_a} {}", rate(a));
    let v2 = format!("{label_b} {}", rate(b));
    if wide {
        // 宽卡：收/发、读/写 同一行（左侧一份、右半区一份）
        draw(
            canvas,
            text,
            &v1,
            x + 12.0,
            y + 39.0 * s,
            (w - 24.0) / 2.0,
            value_size,
            theme.palette.accent,
        )?;
        draw_right(
            canvas,
            text,
            &v2,
            x + w - 12.0,
            y + 39.0 * s,
            w / 2.0 - 12.0,
            value_size,
            theme.palette.warn,
        )?;
    } else {
        draw(
            canvas,
            text,
            &v1,
            x + 10.0,
            y + 45.0 * s,
            w - 20.0,
            value_size,
            theme.palette.accent,
        )?;
        draw(
            canvas,
            text,
            &v2,
            x + 10.0,
            y + 70.0 * s,
            w - 20.0,
            value_size,
            theme.palette.warn,
        )?;
    }
    Ok(())
}

fn draw_simple_card(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    data: &LegacyData,
    kind: ComponentKind,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: &mut Option<&mut dyn TextRenderer>,
) -> Result<(), RenderError> {
    let (title, value) = match kind {
        ComponentKind::Temp => {
            let v = data.cpu_temp.or(data.gpu_temp);
            ("温度", v.map_or("--".to_string(), |t| format!("{t:.0}°C")))
        }
        ComponentKind::Clock => (("时钟"), "--".to_string()),
        _ => ("存储", "--".to_string()),
    };
    let s = single_scale(h);
    draw(
        canvas,
        text,
        title,
        x + 12.0,
        y + 10.0 * s,
        w - 24.0,
        (13.0 * s).min(18.0),
        theme.palette.dim,
    )?;
    draw(
        canvas,
        text,
        &value,
        x + 12.0,
        y + h * 0.45,
        w - 24.0,
        (18.0 * s).min(30.0),
        theme.palette.text,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

fn card_values(data: &LegacyData, kind: ComponentKind) -> (String, Option<f32>, Option<f32>) {
    match kind {
        ComponentKind::Cpu => ("CPU".to_string(), data.cpu, data.cpu_temp),
        ComponentKind::Gpu => ("GPU".to_string(), data.gpu, data.gpu_temp),
        ComponentKind::Memory => ("内存".to_string(), data.mem, None),
        ComponentKind::Network => ("网络".to_string(), None, None),
        ComponentKind::Disk => ("磁盘".to_string(), None, None),
        ComponentKind::Storage => ("存储".to_string(), None, None),
        ComponentKind::Temp => ("温度".to_string(), None, None),
        ComponentKind::Clock => ("时钟".to_string(), None, None),
    }
}

fn draw_ring_value(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    text: &mut Option<&mut dyn TextRenderer>,
    cx: f32,
    cy: f32,
    r: f32,
    thickness: f32,
    value_size: f32,
    label_size: f32,
    label_dy: f32,
    pct: Option<f32>,
) -> Result<(), RenderError> {
    draw_ring_track(canvas, theme, cx, cy, r, thickness)?;
    if let Some(v) = pct {
        canvas.draw_ring(
            cx,
            cy,
            r,
            thickness,
            0.0,
            v.clamp(0.0, 100.0) * 3.6,
            usage_color(theme, v),
        )?;
    }
    let label = match pct {
        Some(v) => format!("{:.0}%", v.clamp(0.0, 100.0)),
        None => "--".to_string(),
    };
    draw_centered(
        canvas,
        text,
        &label,
        cx,
        cy - value_size * 0.80,
        r * 2.0,
        value_size,
        theme.palette.text,
    )?;
    draw_centered(
        canvas,
        text,
        "使用率",
        cx,
        cy + label_dy,
        r * 2.0,
        label_size,
        theme.palette.dim,
    )?;
    Ok(())
}

fn draw_ring_track(
    canvas: &mut Canvas,
    theme: &ThemeConfig,
    cx: f32,
    cy: f32,
    r: f32,
    thickness: f32,
) -> Result<(), RenderError> {
    // 轨道用比卡片略亮的颜色，浅色主题下用 dim 的低透明度近似
    let track = Color::new(
        theme.palette.dim.r,
        theme.palette.dim.g,
        theme.palette.dim.b,
        70,
    );
    canvas.draw_ring(cx, cy, r, thickness, 0.0, 360.0, track)
}

fn usage_color(theme: &ThemeConfig, pct: f32) -> Color {
    if pct >= 90.0 {
        theme.palette.danger
    } else if pct >= 70.0 {
        theme.palette.warn
    } else {
        theme.palette.accent
    }
}

fn temp_color(theme: &ThemeConfig, t: f32) -> Color {
    if t >= 80.0 {
        theme.palette.danger
    } else if t >= 65.0 {
        theme.palette.warn
    } else {
        theme.palette.accent
    }
}

fn rate(value: Option<u64>) -> String {
    match value {
        Some(v) => crate::metrics::format_rate(v),
        None => "--".to_string(),
    }
}

fn gb_short(bytes: u64) -> String {
    let gb = bytes as f64 / 1024.0 / 1024.0 / 1024.0;
    format!("{gb:.1}G")
}

#[allow(clippy::too_many_arguments)]
fn draw(
    canvas: &mut Canvas,
    text: &mut Option<&mut dyn TextRenderer>,
    s: &str,
    x: f32,
    y: f32,
    w: f32,
    size: f32,
    color: Color,
) -> Result<(), RenderError> {
    if let Some(renderer) = text.as_deref_mut() {
        renderer.draw_text(canvas, s, x, y, w.max(0.0), size, color)?;
    }
    Ok(())
}

/// 文本实测宽度；无渲染器时退回近似估算。
fn text_width(text: &mut Option<&mut dyn TextRenderer>, s: &str, size: f32) -> f32 {
    text.as_deref_mut()
        .and_then(|renderer| renderer.measure_text(s, size))
        .unwrap_or_else(|| estimate_text_width(s, size))
}

/// 以 `center_x` 为中心画文字（文本渲染器没有对齐参数，这里按实测宽度回推起点）。
#[allow(clippy::too_many_arguments)]
fn draw_centered(
    canvas: &mut Canvas,
    text: &mut Option<&mut dyn TextRenderer>,
    s: &str,
    center_x: f32,
    y: f32,
    max_w: f32,
    size: f32,
    color: Color,
) -> Result<(), RenderError> {
    if text.is_some() {
        let w = text_width(text, s, size);
        if let Some(renderer) = text.as_deref_mut() {
            renderer.draw_text(canvas, s, center_x - w / 2.0, y, max_w, size, color)?;
        }
    }
    Ok(())
}

/// 右对齐到 `right_x`。
#[allow(clippy::too_many_arguments)]
fn draw_right(
    canvas: &mut Canvas,
    text: &mut Option<&mut dyn TextRenderer>,
    s: &str,
    right_x: f32,
    y: f32,
    max_w: f32,
    size: f32,
    color: Color,
) -> Result<(), RenderError> {
    if let Some(renderer) = text.as_deref_mut() {
        let w = renderer
            .measure_text(s, size)
            .unwrap_or_else(|| estimate_text_width(s, size));
        renderer.draw_text(canvas, s, right_x - w, y, max_w, size, color)?;
    }
    Ok(())
}

/// 无渲染器时的宽度近似：中日韩按 1em，数字/字母 0.55em，`%` 0.85em。
fn estimate_text_width(s: &str, size: f32) -> f32 {
    s.chars()
        .map(|c| match c {
            ' ' => 0.30,
            '%' => 0.85,
            '°' => 0.50,
            c if (c as u32) < 0x2E80 => 0.55,
            _ => 1.0,
        })
        .sum::<f32>()
        * size
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::{Orientation, Palette};

    fn theme(render: &str) -> ThemeConfig {
        ThemeConfig {
            schema: ThemeConfig::SCHEMA_V1.to_string(),
            id: "t".into(),
            name: "测试".into(),
            author: None,
            version: "1.0.0".into(),
            palette: Palette {
                bg: Color::rgb(16, 20, 24),
                card: Color::rgb(27, 32, 40),
                text: Color::rgb(232, 237, 242),
                dim: Color::rgb(138, 148, 166),
                accent: Color::rgb(55, 214, 122),
                warn: Color::rgb(255, 176, 32),
                danger: Color::rgb(255, 90, 95),
            },
            typography: Default::default(),
            background: Default::default(),
            components: Default::default(),
            layout: Default::default(),
            render: Some(render.to_string()),
            overlay: None,
            resources: Vec::new(),
        }
    }

    fn profile(portrait: bool) -> ScreenProfile {
        if portrait {
            ScreenProfile::new("p", 320, 480, Orientation::Portrait, true).expect("profile")
        } else {
            ScreenProfile::new("l", 480, 320, Orientation::Landscape, true).expect("profile")
        }
    }

    fn data() -> LegacyData {
        LegacyData {
            host: Some("nas".into()),
            addr: Some("192.168.1.2:8765".into()),
            online: true,
            cpu: Some(42.0),
            cpu_temp: Some(55.0),
            mem: Some(63.0),
            mem_used: Some(6 * 1024 * 1024 * 1024),
            mem_total: Some(16 * 1024 * 1024 * 1024),
            gpu: Some(30.0),
            gpu_temp: Some(60.0),
            up: Some(1024),
            down: Some(2048),
            read: Some(4096),
            write: Some(2048),
            pc_cpu: Some(20.0),
            pc_mem: Some(50.0),
            pc_gpu: Some(35.0),
            pc_gpu_temp: Some(52.0),
            pc_rx: Some(3072),
            pc_tx: Some(1536),
        }
    }

    fn kinds() -> Vec<ComponentKind> {
        vec![
            ComponentKind::Cpu,
            ComponentKind::Memory,
            ComponentKind::Gpu,
            ComponentKind::Network,
            ComponentKind::Disk,
        ]
    }

    #[test]
    fn legacy_single_renders_both_orientations() {
        for portrait in [true, false] {
            let frame = render_legacy(
                &profile(portrait),
                &theme("legacy-single"),
                &data(),
                &kinds(),
                None,
            )
            .expect("render");
            assert_eq!(frame.width(), if portrait { 320 } else { 480 });
            assert_eq!(frame.height(), if portrait { 480 } else { 320 });
        }
    }

    #[test]
    fn legacy_dual_renders_both_orientations() {
        for portrait in [true, false] {
            let frame = render_legacy(
                &profile(portrait),
                &theme("legacy-dual"),
                &data(),
                &kinds(),
                None,
            )
            .expect("render");
            assert!(frame.pixels().iter().any(|v| *v != 0));
        }
    }

    #[test]
    fn legacy_dual_ignores_non_dual_kinds() {
        let only_disk = vec![ComponentKind::Disk];
        let frame = render_legacy(
            &profile(true),
            &theme("legacy-dual"),
            &data(),
            &only_disk,
            None,
        )
        .expect("render");
        // 只有背景与顶栏，不应 panic，尺寸仍正确
        assert_eq!(frame.width(), 320);
    }

    #[test]
    fn legacy_single_handles_missing_values() {
        let frame = render_legacy(
            &profile(true),
            &theme("legacy-single"),
            &LegacyData::default(),
            &kinds(),
            None,
        )
        .expect("render");
        assert_eq!(frame.height(), 480);
    }

    /// 记录所有绘制文本的测试渲染器。
    struct Recorder(Vec<String>);

    impl TextRenderer for Recorder {
        fn draw_text(
            &mut self,
            _canvas: &mut Canvas,
            text: &str,
            _x: f32,
            _y: f32,
            _max_width: f32,
            _size: f32,
            _color: Color,
        ) -> Result<(), RenderError> {
            self.0.push(text.to_string());
            Ok(())
        }
    }

    #[test]
    fn dual_local_network_shows_pc_rates() {
        let mut rec = Recorder(Vec::new());
        let data = LegacyData {
            pc_rx: Some(12 * 1024 * 1024),
            pc_tx: Some(3 * 1024 * 1024),
            ..data()
        };
        render_legacy(
            &profile(true),
            &theme("legacy-dual"),
            &data,
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let joined = rec.0.join("|");
        assert!(
            joined.contains("电脑 网络"),
            "双显右列应有电脑网络卡：{joined}"
        );
        // 换算成人类可读速率后不应是占位符
        assert!(joined.contains("12.0 MB/s"), "收速率：{joined}");
        assert!(joined.contains("3.0 MB/s"), "发速率：{joined}");
    }

    #[test]
    fn dual_local_network_falls_back_to_dash() {
        let mut rec = Recorder(Vec::new());
        let data = LegacyData {
            pc_rx: None,
            pc_tx: None,
            ..data()
        };
        render_legacy(
            &profile(true),
            &theme("legacy-dual"),
            &data,
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let joined = rec.0.join("|");
        assert!(joined.contains("电脑 网络"), "{joined}");
        assert!(joined.contains("收 --"), "{joined}");
        assert!(joined.contains("发 --"), "{joined}");
    }

    /// 记录文本、字号与 y 坐标，用于版式断言。
    struct SizedRecorder(Vec<(String, f32, f32)>);

    impl TextRenderer for SizedRecorder {
        fn draw_text(
            &mut self,
            _canvas: &mut Canvas,
            text: &str,
            _x: f32,
            y: f32,
            _max_width: f32,
            size: f32,
            _color: Color,
        ) -> Result<(), RenderError> {
            self.0.push((text.to_string(), size, y));
            Ok(())
        }
    }

    #[test]
    fn single_card_corner_is_rounded() {
        let mut th = theme("legacy-single");
        th.palette.bg = Color::rgb(0, 0, 0);
        th.palette.card = Color::rgb(255, 255, 255);
        th.background.fallback_color = Color::rgb(0, 0, 0);
        let frame = render_legacy(&profile(true), &th, &data(), &kinds(), None).expect("render");
        let px = |x: usize, y: usize| {
            let i = (y * 320 + x) * 4;
            (
                frame.pixels()[i],
                frame.pixels()[i + 1],
                frame.pixels()[i + 2],
            )
        };
        // 卡片左上角 (10,40)：圆角外仍是背景色
        assert_eq!(px(10, 40), (0, 0, 0), "圆角处应露出背景");
        // 圆角内部应是卡片/描边色
        assert_ne!(px(18, 42), (0, 0, 0), "圆角内部应是卡片");
    }

    #[test]
    fn dual_value_uses_large_font() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(true),
            &theme("legacy-dual"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let (num_size, pct_size) = {
            let num = rec
                .0
                .iter()
                .find(|(t, _, _)| t == "42")
                .expect("应画 CPU 使用率数字");
            let pct = rec.0.iter().find(|(t, _, _)| t == "%").expect("应画百分号");
            (num.1, pct.1)
        };
        assert_eq!(num_size, 56.0, "竖屏双显数值应加大到 56px");
        assert_eq!(pct_size, 24.0, "百分号字号不应跟着变大");
    }

    #[test]
    fn dual_landscape_network_rates_use_larger_font() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-dual"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let size_of = |prefix: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t.starts_with(prefix))
                .map(|(_, size, _)| *size)
                .unwrap_or_else(|| panic!("缺少文本：{prefix}"))
        };
        assert_eq!(size_of("收 "), 18.0, "横屏收发行应加大到 18px");
        assert_eq!(size_of("发 "), 18.0, "横屏发发行应加大到 18px");
    }

    #[test]
    fn dual_portrait_network_rates_also_grow() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(true),
            &theme("legacy-dual"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let size = rec
            .0
            .iter()
            .find(|(t, _, _)| t.starts_with("收 "))
            .map(|(_, size, _)| *size)
            .expect("缺少收速率");
        assert_eq!(size, 18.0, "竖屏收发行也应放大到 18px");
    }

    #[test]
    fn dual_landscape_value_grows_but_percent_stays() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-dual"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let num_size = rec
            .0
            .iter()
            .find(|(t, _, _)| t == "42")
            .map(|(_, size, _)| *size)
            .expect("应画 CPU 使用率数字");
        let pct_size = rec
            .0
            .iter()
            .find(|(t, _, _)| t == "%")
            .map(|(_, size, _)| *size)
            .expect("应画百分号");
        assert_eq!(num_size, 48.0, "横屏数值应加大到 48px");
        assert_eq!(pct_size, 16.0, "百分号字号不应跟着变大");
    }

    #[test]
    fn single_fewer_items_enlarge_ring_values() {
        let value_size = |kinds: &[ComponentKind]| {
            let mut rec = SizedRecorder(Vec::new());
            render_legacy(
                &profile(true),
                &theme("legacy-single"),
                &data(),
                kinds,
                Some(&mut rec),
            )
            .expect("render");
            rec.0
                .iter()
                .find(|(t, _, _)| t == "42%")
                .map(|(_, size, _)| *size)
                .unwrap_or_else(|| panic!("缺少 CPU 使用率"))
        };
        let five = value_size(&kinds());
        let three = value_size(&[
            ComponentKind::Cpu,
            ComponentKind::Memory,
            ComponentKind::Gpu,
        ]);
        assert!(three > five, "项目减少后数值应变大：{three} vs {five}");
    }

    #[test]
    fn single_below_three_items_keep_three_item_scale() {
        let value_size = |kinds: &[ComponentKind]| {
            let mut rec = SizedRecorder(Vec::new());
            render_legacy(
                &profile(true),
                &theme("legacy-single"),
                &data(),
                kinds,
                Some(&mut rec),
            )
            .expect("render");
            rec.0
                .iter()
                .find(|(t, _, _)| t == "42%")
                .map(|(_, size, _)| *size)
                .unwrap_or_else(|| panic!("缺少 CPU 使用率"))
        };
        let three = value_size(&[
            ComponentKind::Cpu,
            ComponentKind::Memory,
            ComponentKind::Gpu,
        ]);
        let two = value_size(&[ComponentKind::Cpu, ComponentKind::Memory]);
        assert_eq!(two, three, "少于 3 项时应按 3 项的行高封顶，不再继续放大");
    }

    #[test]
    fn wide_flow_card_puts_rates_on_one_row() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(true),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let y_of = |prefix: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t.starts_with(prefix))
                .map(|(_, _, y)| *y)
                .unwrap_or_else(|| panic!("缺少文本：{prefix}"))
        };
        assert_eq!(y_of("收 "), y_of("发 "), "收/发应同一行");
        assert_eq!(y_of("读 "), y_of("写 "), "读/写应同一行");
    }

    #[test]
    fn portrait_flow_rates_use_larger_font() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(true),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let size_of = |prefix: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t.starts_with(prefix))
                .map(|(_, size, _)| *size)
                .unwrap_or_else(|| panic!("缺少文本：{prefix}"))
        };
        for prefix in ["收 ", "发 ", "读 ", "写 "] {
            assert!(size_of(prefix) >= 20.0, "竖屏速率字号偏小：{prefix}");
        }
    }

    #[test]
    fn landscape_top_bar_is_one_row_with_equal_size() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let find = |t: &str| {
            rec.0
                .iter()
                .find(|(s, _, _)| s == t)
                .map(|(_, size, y)| (*size, *y))
                .unwrap_or_else(|| panic!("缺少文本：{t}"))
        };
        let (host_size, host_y) = find("nas");
        let (addr_size, addr_y) = find("192.168.1.2:8765");
        let (label_size, label_y) = find("已连接");
        assert_eq!(host_y, addr_y, "主机名与 IP 应同一行");
        assert_eq!(addr_y, label_y, "IP 与状态应同一行");
        assert_eq!(host_size, addr_size, "主机名与 IP 字号应相同");
        assert_eq!(addr_size, label_size, "IP 与状态字号应相同");
        assert!(host_size >= 19.0, "顶栏字号偏小：{host_size}");
    }

    #[test]
    fn landscape_top_bar_puts_ip_before_status() {
        let mut rec = XRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let x_of = |t: &str| {
            rec.0
                .iter()
                .find(|(s, _, _)| s == t)
                .map(|(_, x, _)| *x)
                .unwrap_or_else(|| panic!("缺少文本：{t}"))
        };
        assert!(x_of("nas") < x_of("192.168.1.2:8765"), "主机名应在 IP 左侧");
        assert!(
            x_of("192.168.1.2:8765") < x_of("已连接"),
            "IP 应位于「已连接」左侧"
        );
    }

    #[test]
    fn landscape_flow_rates_use_larger_font() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let size_of = |prefix: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t.starts_with(prefix))
                .map(|(_, size, _)| *size)
                .unwrap_or_else(|| panic!("缺少文本：{prefix}"))
        };
        for prefix in ["收 ", "发 ", "读 ", "写 "] {
            assert!(size_of(prefix) >= 24.0, "横屏速率字号偏小：{prefix}");
        }
    }

    #[test]
    fn landscape_flow_text_aligns_with_title() {
        let mut rec = XRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let x_of = |title: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t == title)
                .map(|(_, x, _)| *x)
                .unwrap_or_else(|| panic!("缺少文本：{title}"))
        };
        let x_of_value = |prefix: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t.starts_with(prefix))
                .map(|(_, x, _)| *x)
                .unwrap_or_else(|| panic!("缺少文本：{prefix}"))
        };
        // 标题与数值共用卡片左边界（x+10）
        for (title, prefix) in [("网络", "收 "), ("硬盘", "读 ")] {
            let (tx, vx) = (x_of(title), x_of_value(prefix));
            assert!(
                (vx - tx).abs() < 0.5,
                "数值应与标题左对齐：{title} tx={tx} vx={vx}"
            );
        }
    }

    #[test]
    fn landscape_flow_titles_sit_top_left() {
        let mut rec = XRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let y_of = |title: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t == title)
                .map(|(_, _, y)| *y)
                .unwrap_or_else(|| panic!("缺少文本：{title}"))
        };
        let y_of_value = |prefix: &str| {
            rec.0
                .iter()
                .find(|(t, _, _)| t.starts_with(prefix))
                .map(|(_, _, y)| *y)
                .unwrap_or_else(|| panic!("缺少文本：{prefix}"))
        };
        // 标题在卡片顶部（数值上方）
        for (title, value_prefix) in [("网络", "收 "), ("硬盘", "读 ")] {
            let (ty, vy) = (y_of(title), y_of_value(value_prefix));
            assert!(ty < vy, "标题应在数值上方：{title} ty={ty} vy={vy}");
        }
        assert!(
            !rec.0.iter().any(|(t, _, _)| t == "硬盘读写"),
            "不应再出现「硬盘读写」"
        );
    }

    #[test]
    fn landscape_temp_sits_top_right_and_is_larger() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let (text, size, y) = rec
            .0
            .iter()
            .find(|(t, _, _)| t.ends_with('°'))
            .expect("应有温度文本");
        assert!(*size >= 17.0, "温度字号偏小：{text} {size}");
        assert!(*y < 50.0, "温度应贴近卡片顶部：y={y}");

        let mut xrec = XRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut xrec),
        )
        .expect("render");
        let (_, x, _) = xrec
            .0
            .iter()
            .find(|(t, _, _)| t.ends_with('°'))
            .expect("应有温度文本");
        assert!(*x > 100.0, "温度应位于卡片右侧：x={x}");
    }

    #[test]
    fn landscape_memory_line_is_compact_and_readable() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let (text, size, _) = rec
            .0
            .iter()
            .find(|(t, _, _)| t.contains("/16.0G"))
            .expect("应有内存用量");
        assert_eq!(text, "6.0G/16.0G", "内存用量应为紧凑横排");
        assert!(*size >= 14.0, "内存字号偏小：{size}");
    }

    #[test]
    fn landscape_memory_line_sits_top_right() {
        let mut rec = XRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let (_, x, y) = rec
            .0
            .iter()
            .find(|(t, _, _)| t == "6.0G/16.0G")
            .expect("应有内存用量");
        assert!(*x > 200.0, "内存用量应在卡片右侧：x={x}");
        assert!(*y < 50.0, "内存用量应贴卡片顶部：y={y}");
    }

    #[test]
    fn landscape_ring_value_uses_larger_font() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let (_, size, _) = rec
            .0
            .iter()
            .find(|(t, _, _)| t.contains('%'))
            .expect("应有环内使用率");
        assert!(*size >= 26.0, "环内数值字号偏小：{size}");
    }

    #[test]
    fn landscape_ring_label_sits_clear_of_value() {
        let mut rec = SizedRecorder(Vec::new());
        render_legacy(
            &profile(false),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let mut pairs = 0;
        let mut value: Option<(f32, f32)> = None; // (字号, y)
        for (text, size, y) in &rec.0 {
            if text.ends_with('%') {
                value = Some((*size, *y));
            } else if text == "使用率" {
                let (vsize, vy) = value.expect("使用率之前应有百分比数值");
                // 标签文字框顶部应低于数值基线至少 2px
                let gap = *y - (vy + vsize);
                assert!(gap >= 2.0, "使用率离百分比太近：gap={gap}");
                pairs += 1;
            }
        }
        assert_eq!(pairs, 3, "横屏应有三处环内使用率，实际 {pairs}");
    }

    /// 记录文本、x、y，用于对齐断言。
    struct XRecorder(Vec<(String, f32, f32)>);

    impl TextRenderer for XRecorder {
        fn draw_text(
            &mut self,
            _canvas: &mut Canvas,
            text: &str,
            x: f32,
            y: f32,
            _max_width: f32,
            _size: f32,
            _color: Color,
        ) -> Result<(), RenderError> {
            self.0.push((text.to_string(), x, y));
            Ok(())
        }
    }

    #[test]
    fn ring_value_and_label_share_center() {
        let mut rec = XRecorder(Vec::new());
        render_legacy(
            &profile(true),
            &theme("legacy-single"),
            &data(),
            &kinds(),
            Some(&mut rec),
        )
        .expect("render");
        let at = |t: &str| {
            rec.0
                .iter()
                .find(|(s, _, _)| s == t)
                .map(|(_, x, y)| (*x, *y))
                .unwrap_or_else(|| panic!("缺少文本：{t}"))
        };
        let (vx, vy) = at("42%");
        let (lx, ly) = at("使用率");
        let vc = vx + estimate_text_width("42%", 16.0) / 2.0;
        let lc = lx + estimate_text_width("使用率", 11.0) / 2.0;
        assert!((vc - lc).abs() < 0.6, "数值与标签应共用环心：{vc} vs {lc}");
        assert!(ly > vy, "使用率应在数值下方");
    }

    #[test]
    fn overlay_mode_draws_values_on_background_without_cards() {
        let p = profile(true);
        let bg = solid_bg(320, 480, (10, 200, 10));
        let mut th = theme("legacy-overlay");
        th.background.dim = 0.0;
        let mut rec = XRecorder(Vec::new());
        let frame = render_legacy_bg(&p, &th, &data(), &kinds(), Some(&mut rec), Some(&bg))
            .expect("render");
        for want in ["42%", "63%", "30%"] {
            assert!(
                rec.0.iter().any(|(t, _, _)| t == want),
                "官方皮肤应只贴数值，缺少：{want}"
            );
        }
        // 卡片不再绘制，底图原样保留
        assert_eq!(&frame.pixels()[0..4], &[10, 200, 10, 255]);
    }

    #[test]
    fn overlay_slots_place_values_at_declared_pixels() {
        let p = profile(false);
        let bg = solid_bg(480, 320, (0, 0, 0));
        let mut th = theme("legacy-overlay");
        th.background.dim = 0.0;
        th.overlay = Some(screen_core::Overlay {
            slots: vec![screen_core::OverlaySlot {
                metric: OverlayMetric::Cpu,
                x: 100.0,
                y: 40.0,
                w: 60.0,
                size: 20.0,
                align: SlotAlign::Right,
                color: Some(Color::rgb(255, 255, 255)),
                suffix: Some("%".into()),
                decimals: 0,
                rate_unit: RateUnit::Auto,
            }],
            variants: Default::default(),
        });
        let mut rec = XRecorder(Vec::new());
        render_legacy_bg(&p, &th, &data(), &kinds(), Some(&mut rec), Some(&bg)).expect("render");
        let (text, x, y) = rec
            .0
            .iter()
            .find(|(t, _, _)| t == "42%")
            .expect("槽位应贴出 CPU 数值");
        assert_eq!(*y, 40.0, "数值应落在声明的 y");
        let right = x + estimate_text_width("42%", 20.0);
        assert!(
            (right - 160.0).abs() < 1.0,
            "右对齐应贴到 x+w：right={right}"
        );
    }

    #[test]
    fn overlay_missing_metric_leaves_slot_empty() {
        let p = profile(false);
        let mut th = theme("legacy-overlay");
        th.overlay = Some(screen_core::Overlay {
            slots: vec![screen_core::OverlaySlot {
                metric: OverlayMetric::GpuTemp,
                x: 0.0,
                y: 0.0,
                w: 100.0,
                size: 16.0,
                align: SlotAlign::Left,
                color: None,
                suffix: None,
                decimals: 0,
                rate_unit: RateUnit::Auto,
            }],
            variants: Default::default(),
        });
        let mut data = data();
        data.gpu_temp = None;
        let mut rec = XRecorder(Vec::new());
        render_legacy(&p, &th, &data, &kinds(), Some(&mut rec)).expect("render");
        assert!(
            rec.0.is_empty(),
            "缺数据的槽位应整项留空，实际写了：{:?}",
            rec.0
        );
    }

    #[test]
    fn overlay_rate_unit_mb_prints_plain_number() {
        let p = profile(false);
        let mut th = theme("legacy-overlay");
        th.overlay = Some(screen_core::Overlay {
            slots: vec![screen_core::OverlaySlot {
                metric: OverlayMetric::DiskRead,
                x: 0.0,
                y: 0.0,
                w: 100.0,
                size: 16.0,
                align: SlotAlign::Left,
                color: None,
                suffix: None,
                decimals: 1,
                rate_unit: RateUnit::Mb,
            }],
            variants: Default::default(),
        });
        let mut data = data();
        data.read = Some(12 * 1024 * 1024);
        let mut rec = XRecorder(Vec::new());
        render_legacy(&p, &th, &data, &kinds(), Some(&mut rec)).expect("render");
        assert_eq!(
            rec.0.first().map(|t| t.0.as_str()),
            Some("12.0"),
            "底图固定印 MB/s 时只贴数字"
        );
    }

    #[test]
    fn overlay_rate_unit_auto_keeps_human_readable_unit() {
        let p = profile(false);
        let mut th = theme("legacy-overlay");
        th.overlay = Some(screen_core::Overlay {
            slots: vec![screen_core::OverlaySlot {
                metric: OverlayMetric::DiskRead,
                x: 0.0,
                y: 0.0,
                w: 100.0,
                size: 16.0,
                align: SlotAlign::Left,
                color: None,
                suffix: None,
                decimals: 1,
                rate_unit: RateUnit::Auto,
            }],
            variants: Default::default(),
        });
        let mut data = data();
        data.read = Some(1536);
        let mut rec = XRecorder(Vec::new());
        render_legacy(&p, &th, &data, &kinds(), Some(&mut rec)).expect("render");
        assert_eq!(
            rec.0.first().map(|t| t.0.as_str()),
            Some("1.5 KB/s"),
            "默认仍按人类可读单位"
        );
    }

    #[test]
    fn overlay_variants_pick_slots_by_orientation() {
        let slot = |y: f32| screen_core::OverlaySlot {
            metric: OverlayMetric::Cpu,
            x: 100.0,
            y,
            w: 60.0,
            size: 20.0,
            align: SlotAlign::Left,
            color: None,
            suffix: None,
            decimals: 0,
            rate_unit: RateUnit::Auto,
        };
        let mut variants = std::collections::BTreeMap::new();
        variants.insert("landscape".to_string(), vec![slot(200.0)]);
        variants.insert("portrait".to_string(), vec![slot(300.0)]);

        let mut th = theme("legacy-overlay");
        th.overlay = Some(screen_core::Overlay {
            slots: Vec::new(),
            variants,
        });

        for (portrait, want) in [(false, 200.0), (true, 300.0)] {
            let mut rec = XRecorder(Vec::new());
            render_legacy(&profile(portrait), &th, &data(), &kinds(), Some(&mut rec))
                .expect("render");
            let y = rec.0.first().map(|t| t.2).expect("应贴出数值");
            assert_eq!(y, want, "portrait={portrait} 应使用对应方向的槽位");
        }
    }

    #[test]
    fn mb_fit_keeps_width_stable_across_magnitudes() {
        // 0.0 / 0.001 / 5.500 / 10.00 / 128.0 / 1024：长度都在 3–5 字符
        assert_eq!(format_mb_fit(0), "0.0", "完全空闲固定 0.0");
        assert_eq!(format_mb_fit(1), "0.001", "有流量但极小时抬到 0.001");
        assert_eq!(format_mb_fit(512), "0.001");
        assert_eq!(format_mb_fit(5 * 1024 * 1024), "5.000");
        assert_eq!(format_mb_fit(5 * 1024 * 1024 + 512 * 1024), "5.500");
        assert_eq!(format_mb_fit(10 * 1024 * 1024), "10.00");
        assert_eq!(format_mb_fit(1024 * 1024 * 1024 / 8), "128.0");
        assert_eq!(format_mb_fit(1024 * 1024 * 1024), "1024");
        assert_eq!(format_mb_fit(12 * 1024 * 1024 * 1024), "12288");
    }

    #[test]
    fn overlay_rate_unit_mb_fit_uses_adaptive_decimals() {
        let p = profile(false);
        let mut th = theme("legacy-overlay");
        th.overlay = Some(screen_core::Overlay {
            slots: vec![screen_core::OverlaySlot {
                metric: OverlayMetric::NetworkDown,
                x: 0.0,
                y: 0.0,
                w: 100.0,
                size: 16.0,
                align: SlotAlign::Left,
                color: None,
                suffix: None,
                decimals: 1,
                rate_unit: RateUnit::MbFit,
            }],
            variants: Default::default(),
        });
        let mut data = data();
        data.down = Some(10 * 1024 * 1024);
        let mut rec = XRecorder(Vec::new());
        render_legacy(&p, &th, &data, &kinds(), Some(&mut rec)).expect("render");
        assert_eq!(
            rec.0.first().map(|t| t.0.as_str()),
            Some("10.00"),
            "10 MB/s 应显示两位小数"
        );
    }

    fn solid_bg(w: u32, h: u32, rgb: (u8, u8, u8)) -> crate::image::BackgroundImage {
        let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("pixmap");
        pixmap.fill(tiny_skia::Color::from_rgba8(rgb.0, rgb.1, rgb.2, 255));
        let png = pixmap.encode_png().expect("encode");
        crate::image::BackgroundImage::from_png(&png).expect("decode")
    }

    #[test]
    fn legacy_draws_background_image_under_cards() {
        let p = profile(false);
        let bg = solid_bg(480, 320, (200, 10, 10));
        let mut th = theme("legacy-dual");
        th.background.dim = 0.0;
        // 卡片全透明，背景图应完整透出
        th.palette.card = Color::new(0, 0, 0, 0);
        let frame = render_legacy_bg(&p, &th, &data(), &kinds(), None, Some(&bg))
            .expect("render with background");
        // 左上角与卡片内部都应是背景图的红色
        for (x, y) in [(2usize, 2usize), (240, 160)] {
            let i = (y * p.width as usize + x) * 4;
            assert!(
                frame.pixels()[i] > 150 && frame.pixels()[i + 1] < 80,
                "({x},{y}) 应透出背景图：{:?}",
                &frame.pixels()[i..i + 3]
            );
        }
    }

    #[test]
    fn legacy_without_background_keeps_theme_fallback_color() {
        let p = profile(false);
        let mut th = theme("legacy-dual");
        th.background.fallback_color = Color::rgb(9, 9, 9);
        th.palette.card = Color::new(0, 0, 0, 0);
        let frame = render_legacy(&p, &th, &data(), &kinds(), None).expect("render");
        assert_eq!(&frame.pixels()[0..4], &[9, 9, 9, 255]);
    }
}
