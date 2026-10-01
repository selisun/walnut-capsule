//! CapsuleView：悬浮胶囊。
//! 收起：一行 5 项实时数据；展开：胶囊下方挂全量明细面板。

use super::View;
use crate::data::{ConnStatus, SharedState};
use crate::fmt;
use crate::font::Font;
use tiny_skia::*;

const CAPSULE_W: u32 = 520;
const CAPSULE_H: u32 = 40;
const PANEL_W: u32 = CAPSULE_W;
const PANEL_LINE_H: f32 = 26.0;
const PANEL_PAD: f32 = 14.0;
const GAP: u32 = 8;

const FONT_SIZE: f32 = 14.0;
const FONT_SIZE_SMALL: f32 = 12.5;

fn color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

const BG: (u8, u8, u8, u8) = (18, 20, 28, 232);
const FG: (u8, u8, u8, u8) = (235, 238, 245, 255);
const FG_DIM: (u8, u8, u8, u8) = (150, 158, 172, 255);
const ACCENT: (u8, u8, u8, u8) = (96, 200, 255, 255);
const WARN: (u8, u8, u8, u8) = (255, 176, 96, 255);

pub struct CapsuleView {
    expanded: bool,
}

impl CapsuleView {
    pub fn new() -> Self {
        Self { expanded: false }
    }

    fn panel_height(state: &SharedState) -> u32 {
        // 行数：标题 + CPU + 内存 + GPU×n + 盘×n + 网卡×n
        let n = 3
            + state.snap.gpus.as_ref().map_or(0, |v| v.len())
            + state.snap.disks.as_ref().map_or(0, |v| v.len())
            + state.snap.network.as_ref().map_or(0, |v| v.len());
        (PANEL_PAD * 2.0 + n as f32 * PANEL_LINE_H) as u32
    }

    fn fill_round(px: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: (u8, u8, u8, u8)) {
        let path = rounded_rect(x, y, w, h, r);
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.0, c.1, c.2, c.3);
        paint.anti_alias = true;
        px.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    fn render_collapsed(&self, px: &mut Pixmap, state: &SharedState, font: Option<&Font>) {
        Self::fill_round(px, 0.0, 0.0, CAPSULE_W as f32, CAPSULE_H as f32, 20.0, BG);
        let Some(font) = font else { return };
        let baseline = (CAPSULE_H as f32 - FONT_SIZE) / 2.0 + font.ascent(FONT_SIZE);
        let mut x = 18.0;

        if !state.has_data {
            let (text, c) = status_text(&state.conn);
            font.draw(px, x, baseline, FONT_SIZE, &text, c);
            return;
        }

        let s = &state.snap;
        let cpu = s.cpu.as_ref().and_then(|c| c.usage);
        let gpu = first_gpu_util(s);
        let mem = s.memory.as_ref().and_then(|m| m.usage);
        let (dr, dw) = disk_io_sum(s);
        let (nr, nw) = net_io_sum(s);

        x = seg(px, font, x, baseline, "CPU ", cpu.map(fmt::pct));
        x = sep(px, font, x, baseline);
        x = seg(px, font, x, baseline, "GPU ", gpu.map(|v| format!("{v}%")));
        x = sep(px, font, x, baseline);
        x = seg(px, font, x, baseline, "内存 ", mem.map(fmt::pct));
        x = sep(px, font, x, baseline);
        x = seg(px, font, x, baseline, "硬盘 ", Some(io_text(dr, dw)));
        x = sep(px, font, x, baseline);
        let _ = seg(px, font, x, baseline, "网络 ", Some(io_text(nr, nw)));
    }

    fn render_expanded(&self, px: &mut Pixmap, state: &SharedState, font: Option<&Font>) {
        self.render_collapsed(px, state, font);
        let Some(font) = font else { return };

        let y0 = CAPSULE_H as f32 + GAP as f32;
        let ph = Self::panel_height(state) as f32;
        Self::fill_round(px, 0.0, y0, PANEL_W as f32, ph, 14.0, BG);

        let mut baseline = y0 + PANEL_PAD + font.ascent(FONT_SIZE);
        let x = PANEL_PAD + 4.0;

        // 标题 + 连接状态
        let (_, c) = status_text(&state.conn);
        let st = match &state.conn {
            ConnStatus::Online => "已连接".to_string(),
            ConnStatus::Connecting => "连接中…".to_string(),
            ConnStatus::Reconnect(n) => format!("{n}s 后重连"),
        };
        font.draw(px, x, baseline, FONT_SIZE_SMALL, &format!("NAS 状态：{st}"), c);
        baseline += PANEL_LINE_H;

        if !state.has_data {
            return;
        }
        let s = &state.snap;
        let dim = color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3);
        let fg = color(FG.0, FG.1, FG.2, FG.3);

        // CPU
        if let Some(cpu) = &s.cpu {
            let usage = cpu.usage.map(fmt::pct).unwrap_or("--".into());
            let temp = cpu.temp.map(|t| format!("{:.0}°C", t)).unwrap_or("--".into());
            font.draw(px, x, baseline, FONT_SIZE_SMALL, &format!("CPU  {usage}  ·  {temp}"), fg);
        } else {
            font.draw(px, x, baseline, FONT_SIZE_SMALL, "CPU  --", dim);
        }
        baseline += PANEL_LINE_H;

        // 内存
        if let Some(m) = &s.memory {
            let usage = m.usage.map(fmt::pct).unwrap_or("--".into());
            let used = m.used.map(fmt::bytes).unwrap_or("--".into());
            let total = m.total.map(fmt::bytes).unwrap_or("--".into());
            font.draw(px, x, baseline, FONT_SIZE_SMALL, &format!("内存  {usage}  ·  {used} / {total}"), fg);
        } else {
            font.draw(px, x, baseline, FONT_SIZE_SMALL, "内存  --", dim);
        }
        baseline += PANEL_LINE_H;

        // GPU
        if let Some(gpus) = &s.gpus {
            for g in gpus {
                let util = g.utilization.map(|v| format!("{v}%")).unwrap_or("--".into());
                let temp = g.temp.map(|v| format!("{v}°C")).unwrap_or("--".into());
                let mu = g.memory_used.map(fmt::bytes).unwrap_or("--".into());
                let mt = g.memory_total.map(fmt::bytes).unwrap_or("--".into());
                font.draw(
                    px, x, baseline, FONT_SIZE_SMALL,
                    &format!("GPU{}  {}  {util}  ·  {temp}  ·  显存 {mu}/{mt}", g.index, g.name),
                    fg,
                );
                baseline += PANEL_LINE_H;
            }
        }

        // 硬盘
        if let Some(disks) = &s.disks {
            for d in disks {
                let name = d.device.trim_start_matches("/dev/");
                let line = match d.status.as_deref() {
                    Some("standby") => format!("{name}  休眠中"),
                    Some(st) => {
                        let temp = d.temp.map(|v| format!("{v}°C")).unwrap_or("--".into());
                        let r = d.read_bytes_per_sec.map(fmt::rate).unwrap_or("--".into());
                        let w = d.write_bytes_per_sec.map(fmt::rate).unwrap_or("--".into());
                        format!("{name}  {st}  ·  {temp}  ·  ↓{r}/s  ↑{w}/s")
                    }
                    None => format!("{name}  --"),
                };
                font.draw(px, x, baseline, FONT_SIZE_SMALL, &line, fg);
                baseline += PANEL_LINE_H;
            }
        }

        // 网络
        if let Some(nets) = &s.network {
            for n in nets {
                let r = n.rx_bytes_per_sec.map(fmt::rate).unwrap_or("--".into());
                let t = n.tx_bytes_per_sec.map(fmt::rate).unwrap_or("--".into());
                font.draw(px, x, baseline, FONT_SIZE_SMALL, &format!("{}  ↓{r}/s  ↑{t}/s", n.name), fg);
                baseline += PANEL_LINE_H;
            }
        }
    }
}

impl View for CapsuleView {
    fn render(&mut self, px: &mut Pixmap, state: &SharedState, font: Option<&Font>) {
        if self.expanded {
            self.render_expanded(px, state, font);
        } else {
            self.render_collapsed(px, state, font);
        }
    }

    fn expanded(&self) -> bool {
        self.expanded
    }

    fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }
}

impl CapsuleView {
    /// 展开时窗口总高。
    pub fn total_height(state: &SharedState, expanded: bool) -> u32 {
        if expanded {
            CAPSULE_H + GAP + Self::panel_height(state)
        } else {
            CAPSULE_H
        }
    }
    pub const fn width() -> u32 {
        CAPSULE_W
    }
}

fn status_text(conn: &ConnStatus) -> (String, Color) {
    match conn {
        ConnStatus::Online => ("等待数据…".into(), color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3)),
        ConnStatus::Connecting => ("正在连接 NAS…".into(), color(ACCENT.0, ACCENT.1, ACCENT.2, ACCENT.3)),
        ConnStatus::Reconnect(n) => (format!("已断开，{n}s 后重连"), color(WARN.0, WARN.1, WARN.2, WARN.3)),
    }
}

fn first_gpu_util(s: &crate::data::Snapshot) -> Option<i32> {
    s.gpus.as_ref()?.first()?.utilization
}

fn disk_io_sum(s: &crate::data::Snapshot) -> (u64, u64) {
    s.disks
        .as_ref()
        .map(|v| {
            v.iter().fold((0u64, 0u64), |(r, w), d| {
                (r + d.read_bytes_per_sec.unwrap_or(0), w + d.write_bytes_per_sec.unwrap_or(0))
            })
        })
        .unwrap_or((0, 0))
}

fn net_io_sum(s: &crate::data::Snapshot) -> (u64, u64) {
    s.network
        .as_ref()
        .map(|v| {
            v.iter().fold((0u64, 0u64), |(r, w), n| {
                (r + n.rx_bytes_per_sec.unwrap_or(0), w + n.tx_bytes_per_sec.unwrap_or(0))
            })
        })
        .unwrap_or((0, 0))
}

fn io_text(down: u64, up: u64) -> String {
    format!("↓{} ↑{}", fmt::rate(down), fmt::rate(up))
}

/// 绘制一段 "标签 值"：标签暗色、值亮色。返回末端 x。
fn seg(px: &mut Pixmap, font: &Font, x: f32, baseline: f32, label: &str, value: Option<String>) -> f32 {
    let dim = color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3);
    let fg = color(FG.0, FG.1, FG.2, FG.3);
    let x = font.draw(px, x, baseline, FONT_SIZE, label, dim);
    font.draw(px, x, baseline, FONT_SIZE, &value.unwrap_or("--".into()), fg)
}

fn sep(px: &mut Pixmap, font: &Font, x: f32, baseline: f32) -> f32 {
    let dim = color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3);
    font.draw(px, x + 6.0, baseline, FONT_SIZE, "·", dim) + 6.0
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let mut pb = PathBuilder::new();
    let (x2, y2) = (x + w, y + h);
    pb.move_to(x + r, y);
    pb.line_to(x2 - r, y);
    pb.cubic_to(x2 - r / 2.0, y, x2, y + r / 2.0, x2, y + r);
    pb.line_to(x2, y2 - r);
    pb.cubic_to(x2, y2 - r / 2.0, x2 - r / 2.0, y2, x2 - r, y2);
    pb.line_to(x + r, y2);
    pb.cubic_to(x + r / 2.0, y2, x, y2 - r / 2.0, x, y2 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r / 2.0, x + r / 2.0, y, x + r, y);
    pb.close();
    pb.finish().unwrap_or_else(|| PathBuilder::from_rect(Rect::from_xywh(x, y, w, h).unwrap()))
}
