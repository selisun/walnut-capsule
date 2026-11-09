//! 生成 480×320 小屏底图（NAS 指标版）：卡片 + 小图标 + 标签 + 单位，数值留给主题槽位。
//!
//! 用法：cargo run -p screen-render --example make_panel -- <out.png>

use screen_core::Color;
use screen_render::{create_text_renderer, Canvas, FontSpec, TextRenderer};
use tiny_skia::{Path, PathBuilder};

const W: u32 = 480;
const H: u32 = 320;

const C_BG: (u8, u8, u8) = (0x0B, 0x0D, 0x10);
const C_CARD: (u8, u8, u8) = (0x15, 0x18, 0x1D);
const C_BORDER: (u8, u8, u8) = (0x23, 0x2A, 0x33);
const C_TEXT: (u8, u8, u8) = (0xE8, 0xED, 0xF2);
const C_DIM: (u8, u8, u8) = (0x8A, 0x94, 0xA6);
const C_BLUE: (u8, u8, u8) = (0x4E, 0xA1, 0xFF);
const C_ORANGE: (u8, u8, u8) = (0xFF, 0xB0, 0x20);
const C_GREEN: (u8, u8, u8) = (0x37, 0xD6, 0x7A);
const C_PURPLE: (u8, u8, u8) = (0xA7, 0x8B, 0xFA);

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::new(c.0, c.1, c.2, 255)
}

fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let mut b = PathBuilder::new();
    let rr = r.min(w / 2.0).min(h / 2.0);
    b.move_to(x + rr, y);
    b.line_to(x + w - rr, y);
    b.quad_to(x + w, y, x + w, y + rr);
    b.line_to(x + w, y + h - rr);
    b.quad_to(x + w, y + h, x + w - rr, y + h);
    b.line_to(x + rr, y + h);
    b.quad_to(x, y + h, x, y + h - rr);
    b.line_to(x, y + rr);
    b.quad_to(x, y, x + rr, y);
    b.close();
    b.finish().expect("path")
}

/// 圆（用 32 段折线近似，避免依赖额外几何 API）。
fn circle(cx: f32, cy: f32, r: f32) -> Path {
    let mut b = PathBuilder::new();
    let n = 32;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 {
            b.move_to(x, y);
        } else {
            b.line_to(x, y);
        }
    }
    b.close();
    b.finish().expect("circle")
}

/// 圆环扇段（角度制，y 向下，0° 指向 +x，顺时针）。
fn arc_band(cx: f32, cy: f32, r: f32, thickness: f32, a0: f32, a1: f32) -> Path {
    let mut b = PathBuilder::new();
    let steps = 14;
    let inner = (r - thickness).max(0.5);
    for i in 0..=steps {
        let t = (a0 + (a1 - a0) * i as f32 / steps as f32).to_radians();
        let (x, y) = (cx + r * t.cos(), cy + r * t.sin());
        if i == 0 {
            b.move_to(x, y);
        } else {
            b.line_to(x, y);
        }
    }
    for i in (0..=steps).rev() {
        let t = (a0 + (a1 - a0) * i as f32 / steps as f32).to_radians();
        b.line_to(cx + inner * t.cos(), cy + inner * t.sin());
    }
    b.close();
    b.finish().expect("arc")
}

struct Panel {
    canvas: Canvas,
    text: Box<dyn TextRenderer>,
}

impl Panel {
    fn fill(&mut self, path: &Path, c: (u8, u8, u8)) {
        self.canvas.fill_path(path, rgb(c)).expect("fill");
    }

    fn card(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.fill(&round_rect(x, y, w, h, 10.0), C_BORDER);
        self.fill(&round_rect(x + 1.0, y + 1.0, w - 2.0, h - 2.0, 9.0), C_CARD);
    }

    fn stripe(&mut self, x: f32, y: f32, h: f32, c: (u8, u8, u8)) {
        self.fill(&round_rect(x + 1.0, y, 3.0, h, 1.5), c);
    }

    fn text(&mut self, s: &str, x: f32, y: f32, size: f32, c: (u8, u8, u8)) {
        let mut t: Option<&mut dyn TextRenderer> = Some(&mut *self.text);
        if let Some(r) = t.as_mut() {
            let _ = r.draw_text(&mut self.canvas, s, x, y, 200.0, size, rgb(c));
        }
    }

    fn text_right(&mut self, s: &str, right: f32, y: f32, size: f32, c: (u8, u8, u8)) {
        let w = self
            .text
            .measure_text(s, size)
            .unwrap_or_else(|| s.chars().count() as f32 * size * 0.55);
        self.text(s, right - w, y, size, c);
    }

    fn hline(&mut self, x: f32, y: f32, w: f32) {
        self.canvas
            .fill_rect(screen_core::Rect::new(x, y, w, 1.0), rgb(C_BORDER))
            .expect("hline");
    }

    // ---------------- 图标 ----------------

    /// 芯片（CPU / GPU 标题）：外框 + 内芯 + 四边引脚。
    fn icon_chip(&mut self, cx: f32, cy: f32, c: (u8, u8, u8)) {
        let s = 7.0;
        self.fill(&round_rect(cx - s, cy - s, s * 2.0, s * 2.0, 2.0), c);
        self.fill(
            &round_rect(
                cx - s + 2.2,
                cy - s + 2.2,
                s * 2.0 - 4.4,
                s * 2.0 - 4.4,
                1.0,
            ),
            C_CARD,
        );
        self.fill(
            &round_rect(
                cx - s + 4.0,
                cy - s + 4.0,
                s * 2.0 - 8.0,
                s * 2.0 - 8.0,
                0.5,
            ),
            c,
        );
        for i in 0..3 {
            let off = -4.0 + i as f32 * 4.0;
            self.canvas
                .fill_rect(
                    screen_core::Rect::new(cx + off - 0.75, cy - s - 2.0, 1.5, 2.0),
                    rgb(c),
                )
                .expect("pin");
            self.canvas
                .fill_rect(
                    screen_core::Rect::new(cx + off - 0.75, cy + s, 1.5, 2.0),
                    rgb(c),
                )
                .expect("pin");
        }
    }

    /// 使用率：小圆环。
    fn icon_ring(&mut self, cx: f32, cy: f32, c: (u8, u8, u8)) {
        self.fill(&circle(cx, cy, 6.0), c);
        self.fill(&circle(cx, cy, 3.0), C_CARD);
    }

    /// 温度：温度计（球泡 + 管身）。
    fn icon_thermo(&mut self, cx: f32, cy: f32, c: (u8, u8, u8)) {
        self.fill(&round_rect(cx - 1.5, cy - 7.0, 3.0, 10.0, 1.5), c);
        self.fill(&circle(cx, cy + 5.0, 3.6), c);
        self.fill(&circle(cx, cy + 5.0, 1.6), C_CARD);
    }

    /// 内存：内存条（外框 + 底部引脚）。
    fn icon_mem(&mut self, cx: f32, cy: f32, c: (u8, u8, u8)) {
        self.fill(&round_rect(cx - 8.0, cy - 5.0, 16.0, 10.0, 1.5), c);
        self.fill(&round_rect(cx - 6.4, cy - 3.4, 12.8, 6.0, 0.8), C_CARD);
        for i in 0..4 {
            let x = cx - 6.0 + i as f32 * 4.0;
            self.canvas
                .fill_rect(screen_core::Rect::new(x - 0.6, cy + 5.0, 1.2, 1.8), rgb(c))
                .expect("pin");
        }
    }

    /// 网络：三段 WiFi 弧 + 圆点。
    fn icon_wifi(&mut self, cx: f32, cy: f32, c: (u8, u8, u8)) {
        let base = cy + 5.0;
        for (i, r) in [11.0f32, 7.5, 4.0].iter().enumerate() {
            let _ = i;
            self.fill(&arc_band(cx, base, *r, 1.8, 205.0, 335.0), c);
        }
        self.fill(&circle(cx, base, 1.6), c);
    }

    /// 硬盘：盘体 + 主轴圆。
    fn icon_disk(&mut self, cx: f32, cy: f32, c: (u8, u8, u8)) {
        self.fill(&round_rect(cx - 8.0, cy - 6.0, 16.0, 12.0, 2.0), c);
        self.fill(&round_rect(cx - 6.4, cy - 4.4, 12.8, 8.8, 1.2), C_CARD);
        self.fill(&circle(cx - 2.0, cy, 2.4), c);
        self.fill(&circle(cx - 2.0, cy, 1.0), C_CARD);
    }

    /// 上下箭头（网络上下行）。
    fn icon_arrow(&mut self, cx: f32, cy: f32, up: bool, c: (u8, u8, u8)) {
        let mut b = PathBuilder::new();
        if up {
            b.move_to(cx, cy - 7.0);
            b.line_to(cx + 4.5, cy - 1.0);
            b.line_to(cx + 1.6, cy - 1.0);
            b.line_to(cx + 1.6, cy + 7.0);
            b.line_to(cx - 1.6, cy + 7.0);
            b.line_to(cx - 1.6, cy - 1.0);
            b.line_to(cx - 4.5, cy - 1.0);
        } else {
            b.move_to(cx, cy + 7.0);
            b.line_to(cx + 4.5, cy + 1.0);
            b.line_to(cx + 1.6, cy + 1.0);
            b.line_to(cx + 1.6, cy - 7.0);
            b.line_to(cx - 1.6, cy - 7.0);
            b.line_to(cx - 1.6, cy + 1.0);
            b.line_to(cx - 4.5, cy + 1.0);
        }
        b.close();
        if let Some(p) = b.finish() {
            self.fill(&p, c);
        }
    }

    /// 读写：小方块（避免和网络箭头混淆）。
    fn icon_dot(&mut self, cx: f32, cy: f32, c: (u8, u8, u8)) {
        self.fill(&round_rect(cx - 3.0, cy - 3.0, 6.0, 6.0, 1.5), c);
    }
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "panel.png".into());

    let mut canvas = Canvas::new(W, H).expect("canvas");
    canvas.clear(rgb(C_BG));
    let text = create_text_renderer(&FontSpec {
        family: "Microsoft YaHei".to_string(),
        size_px: 22.0,
        weight: 400,
    })
    .expect("GDI 文字渲染器不可用");
    let mut p = Panel { canvas, text };

    // ---- CPU 卡：10,10 225×98 ----
    p.card(10.0, 10.0, 225.0, 98.0);
    p.stripe(10.0, 22.0, 74.0, C_BLUE);
    p.icon_chip(30.0, 26.0, C_BLUE);
    p.text("CPU", 44.0, 18.0, 15.0, C_TEXT);
    p.icon_ring(28.0, 52.0, C_BLUE);
    p.text("使用率", 42.0, 46.0, 12.0, C_DIM);
    p.text_right("%", 222.0, 44.0, 14.0, C_DIM);
    p.icon_thermo(28.0, 78.0, C_ORANGE);
    p.text("温度", 42.0, 76.0, 12.0, C_DIM);
    p.text_right("°C", 222.0, 74.0, 14.0, C_DIM);

    // ---- GPU 卡：10,116 225×98 ----
    p.card(10.0, 116.0, 225.0, 98.0);
    p.stripe(10.0, 128.0, 74.0, C_ORANGE);
    p.icon_chip(30.0, 132.0, C_ORANGE);
    p.text("GPU", 44.0, 124.0, 15.0, C_TEXT);
    p.icon_ring(28.0, 158.0, C_ORANGE);
    p.text("使用率", 42.0, 152.0, 12.0, C_DIM);
    p.text_right("%", 222.0, 150.0, 14.0, C_DIM);
    p.icon_thermo(28.0, 184.0, C_ORANGE);
    p.text("温度", 42.0, 182.0, 12.0, C_DIM);
    p.text_right("°C", 222.0, 180.0, 14.0, C_DIM);

    // ---- 内存卡：10,222 225×88 ----
    p.card(10.0, 222.0, 225.0, 88.0);
    p.stripe(10.0, 234.0, 64.0, C_BLUE);
    p.icon_mem(30.0, 238.0, C_BLUE);
    p.text("内存", 44.0, 230.0, 15.0, C_TEXT);
    p.icon_ring(28.0, 266.0, C_BLUE);
    p.text("使用率", 42.0, 262.0, 12.0, C_DIM);
    p.text_right("%", 222.0, 260.0, 14.0, C_DIM);

    // ---- 网络卡：243,10 227×146 ----
    p.card(243.0, 10.0, 227.0, 146.0);
    p.stripe(243.0, 22.0, 122.0, C_GREEN);
    p.icon_wifi(263.0, 24.0, C_GREEN);
    p.text("网络", 280.0, 18.0, 15.0, C_TEXT);
    p.icon_arrow(262.0, 60.0, true, C_GREEN);
    p.text("上行", 278.0, 56.0, 12.0, C_DIM);
    p.hline(255.0, 80.0, 200.0);
    p.icon_arrow(262.0, 102.0, false, C_GREEN);
    p.text("下行", 278.0, 98.0, 12.0, C_DIM);

    // ---- 硬盘卡：243,164 227×146 ----
    p.card(243.0, 164.0, 227.0, 146.0);
    p.stripe(243.0, 176.0, 122.0, C_PURPLE);
    p.icon_disk(263.0, 178.0, C_PURPLE);
    p.text("硬盘", 280.0, 172.0, 15.0, C_TEXT);
    p.icon_dot(261.0, 214.0, C_PURPLE);
    p.text("读", 276.0, 210.0, 12.0, C_DIM);
    p.hline(255.0, 234.0, 200.0);
    p.icon_dot(261.0, 256.0, C_PURPLE);
    p.text("写", 276.0, 252.0, 12.0, C_DIM);

    let frame = p.canvas.into_frame().expect("frame");
    let mut pixmap = tiny_skia::Pixmap::new(frame.width(), frame.height()).expect("pixmap");
    pixmap.data_mut().copy_from_slice(frame.pixels());
    let png = pixmap.encode_png().expect("encode");
    std::fs::write(&out, png).expect("write png");
    println!("saved {out}");
}
