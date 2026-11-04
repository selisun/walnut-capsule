//! 旧版四主题的版式复刻：顶栏 + 圆环卡片（dark/light 单列；bignum-* 双列）。
//!
//! 与 `pipeline` 的通用网格不同，这里按旧 `screen.ps1` 的排版重画，
//! 主题通过 `render = "legacy-single" | "legacy-dual"` 选用。

use screen_core::{Color, ComponentKind, FrameBuffer, ScreenProfile, ThemeConfig};

use crate::canvas::Canvas;
use crate::error::RenderError;
use crate::text::TextRenderer;

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
}

/// 旧版版式入口。`dual` 由主题 `render` 字段决定。
pub fn render_legacy(
    profile: &ScreenProfile,
    theme: &ThemeConfig,
    data: &LegacyData,
    visible: &[ComponentKind],
    mut text: Option<&mut dyn TextRenderer>,
) -> Result<FrameBuffer, RenderError> {
    let mut canvas = Canvas::new(profile.width, profile.height)?;
    canvas.clear(theme.background.fallback_color);
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
    draw(canvas, text, host, 10.0, 2.0, 150.0, 12.0, color_text)?;
    if let Some(addr) = data.addr.as_deref() {
        draw(canvas, text, addr, w - 200.0, 5.0, 190.0, 9.0, color_dim)?;
    }
    // 连接状态：圆点 + 文字
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
    let y = if one_line { 5.0 } else { 26.0 };
    draw(canvas, text, label, w - 62.0, y, 52.0, 10.0, color_dim)?;
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
    let (top, bottom, side, gap) = (40.0, 14.0, 10.0, 6.0);
    let ch = ((h - top - bottom - (n as f32 - 1.0) * gap) / n as f32).floor();
    for (i, kind) in visible.iter().enumerate() {
        let x = side;
        let y = top + i as f32 * (ch + gap);
        let cw = w - side * 2.0;
        canvas.fill_rect(screen_core::Rect::new(x, y, cw, ch), theme.palette.card)?;
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
        canvas.fill_rect(screen_core::Rect::new(x, y, cw, ch), theme.palette.card)?;
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
    let (title, pct, temp) = card_values(data, kind);
    draw(
        canvas,
        text,
        &title,
        x + 10.0,
        y + 6.0,
        w - 62.0,
        11.0,
        theme.palette.dim,
    )?;
    if let Some(t) = temp {
        draw(
            canvas,
            text,
            &format!("{t:.0}°"),
            x + 60.0,
            y + 6.0,
            w - 70.0,
            11.0,
            temp_color(theme, t),
        )?;
    }
    let r = 30.0;
    let cx = x + w / 2.0;
    let cy = y + h / 2.0 + 10.0;
    draw_ring_value(canvas, theme, text, cx, cy, r, pct)?;
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
        canvas.fill_rect(screen_core::Rect::new(left, y, tw, th), theme.palette.card)?;
        draw_dual_card(canvas, theme, text, *kind, data, left, y, tw, th, false)?;
        // 右列：电脑
        canvas.fill_rect(screen_core::Rect::new(rx, y, rw, th), theme.palette.card)?;
        draw_dual_card(canvas, theme, text, *kind, data, rx, y, rw, th, true)?;
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
            canvas.fill_rect(screen_core::Rect::new(x, ry, cw, rh), theme.palette.card)?;
            draw_dual_card(canvas, theme, text, *kind, data, x, ry, cw, rh, row == 1)?;
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
        let (up, down) = if local {
            (None, None)
        } else {
            (data.up, data.down)
        };
        draw(
            canvas,
            text,
            &name,
            x + 8.0,
            y + 8.0,
            w - 16.0,
            10.0,
            theme.palette.dim,
        )?;
        let line1 = format!("收 {}", rate(up));
        let line2 = format!("发 {}", rate(down));
        let size = if h >= 90.0 { 12.0 } else { 10.0 };
        draw(
            canvas,
            text,
            &line1,
            x + 8.0,
            y + h * 0.36,
            w - 16.0,
            size,
            theme.palette.accent,
        )?;
        draw(
            canvas,
            text,
            &line2,
            x + 8.0,
            y + h * 0.60,
            w - 16.0,
            size,
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
        w - 40.0,
        10.0,
        theme.palette.dim,
    )?;
    if let Some(t) = temp {
        draw(
            canvas,
            text,
            &format!("{t:.0}°"),
            x + w - 38.0,
            y + 4.0,
            34.0,
            12.0,
            temp_color(theme, t),
        )?;
    }
    let value = match pct {
        Some(v) => format!("{:.0}%", v.clamp(0.0, 100.0)),
        None => "--".to_string(),
    };
    let size = if h >= 90.0 { 30.0 } else { 22.0 };
    draw(
        canvas,
        text,
        &value,
        x + 4.0,
        y + h * 0.42,
        w - 8.0,
        size,
        pct.map_or(theme.palette.dim, |v| usage_color(theme, v)),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 卡片内容
// ---------------------------------------------------------------------------

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
    draw(
        canvas,
        text,
        &title,
        x + 14.0,
        y + 10.0,
        w - 170.0,
        13.0,
        theme.palette.dim,
    )?;
    let r2 = 30.0;
    let cy = y + h / 2.0;
    let cx2 = x + w - 16.0 - r2;
    let cx1 = cx2 - 2.0 * r2 - 14.0;
    draw_ring_value(canvas, theme, text, cx1, cy, r2, pct)?;
    // 温度环：无数据时只画轨道
    let temp_pct = temp.unwrap_or(0.0);
    draw_ring_track(canvas, theme, cx2, cy, r2)?;
    if temp.is_some() {
        canvas.draw_ring(
            cx2,
            cy,
            r2,
            7.0,
            0.0,
            temp_pct.clamp(0.0, 100.0) * 3.6,
            temp_color(theme, temp_pct),
        )?;
        draw(
            canvas,
            text,
            &format!("{temp_pct:.0}°"),
            cx2 - r2,
            cy - 10.0,
            r2 * 2.0,
            12.0,
            theme.palette.text,
        )?;
    }
    draw(
        canvas,
        text,
        "温度",
        cx2 - r2,
        cy + 4.0,
        r2 * 2.0,
        8.0,
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
    draw(
        canvas,
        text,
        "内存",
        x + 14.0,
        y + 12.0,
        w - 170.0,
        13.0,
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
        x + 14.0,
        y + 34.0,
        w - 170.0,
        10.0,
        theme.palette.dim,
    )?;
    let r2 = 30.0;
    let cy = y + h / 2.0;
    let cx = x + w - 16.0 - r2;
    draw_ring_value(canvas, theme, text, cx, cy, r2, data.mem)?;
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
        ("磁盘", data.read, data.write, "读", "写")
    };
    draw(
        canvas,
        text,
        title,
        x + 12.0,
        y + 10.0,
        w - 24.0,
        13.0,
        theme.palette.dim,
    )?;
    draw(
        canvas,
        text,
        &format!("{label_a} {}", rate(a)),
        x + 12.0,
        y + h * 0.42,
        w - 24.0,
        13.0,
        theme.palette.accent,
    )?;
    draw(
        canvas,
        text,
        &format!("{label_b} {}", rate(b)),
        x + 12.0,
        y + h * 0.66,
        w - 24.0,
        13.0,
        theme.palette.warn,
    )?;
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
    draw(
        canvas,
        text,
        title,
        x + 12.0,
        y + 10.0,
        w - 24.0,
        13.0,
        theme.palette.dim,
    )?;
    draw(
        canvas,
        text,
        &value,
        x + 12.0,
        y + h * 0.45,
        w - 24.0,
        18.0,
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
    pct: Option<f32>,
) -> Result<(), RenderError> {
    draw_ring_track(canvas, theme, cx, cy, r)?;
    if let Some(v) = pct {
        canvas.draw_ring(
            cx,
            cy,
            r,
            7.0,
            0.0,
            v.clamp(0.0, 100.0) * 3.6,
            usage_color(theme, v),
        )?;
    }
    let label = match pct {
        Some(v) => format!("{:.0}%", v.clamp(0.0, 100.0)),
        None => "--".to_string(),
    };
    draw(
        canvas,
        text,
        &label,
        cx - r,
        cy - 8.0,
        r * 2.0,
        12.0,
        theme.palette.text,
    )?;
    draw(
        canvas,
        text,
        "使用率",
        cx - r,
        cy + 5.0,
        r * 2.0,
        8.0,
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
) -> Result<(), RenderError> {
    // 轨道用比卡片略亮的颜色，浅色主题下用 dim 的低透明度近似
    let track = Color::new(
        theme.palette.dim.r,
        theme.palette.dim.g,
        theme.palette.dim.b,
        70,
    );
    canvas.draw_ring(cx, cy, r, 7.0, 0.0, 360.0, track)
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
    format!("{gb:.0}G")
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
}
