use super::*;
use crate::data::{Cpu, Disk, Gpu, Memory, NetIf, Snapshot, Volume};
use crate::fmt;
use crate::icons::Icons;

const VW: u32 = 1080;
const VH: u32 = 1920;
const VFPS: u32 = 30;
const SC: [f32; 6] = [3.0, 4.0, 4.0, 6.0, 9.0, 4.0];
fn fc(i: usize) -> u32 {
    (SC[i] * VFPS as f32).round() as u32
}

// ---- 深色主题配色（视频专用，产品本体仍是浅色磨砂主题）----
type C4 = (u8, u8, u8, u8);
const BG_TOP: C4 = (12, 15, 21, 255);
const BG_BOT: C4 = (28, 32, 43, 255);
const BARC: C4 = (17, 20, 27, 255);
const PILLC: C4 = (34, 39, 49, 248);
const PANELC: C4 = (27, 30, 38, 252);
const CARDC: C4 = (39, 45, 57, 255);
const TRACKC: C4 = (64, 72, 88, 255);
const FGC: C4 = (235, 239, 246, 255);
const DIMC: C4 = (148, 156, 170, 255);
const BLUEC: C4 = (45, 150, 255, 255);
const GREENC: C4 = (52, 199, 123, 255);
const ORANGEC: C4 = (240, 140, 50, 255);
const REDC: C4 = (235, 84, 72, 255);
const PURPLEC: C4 = (168, 85, 247, 255);
const RBLUEC: C4 = (72, 168, 255, 255);

// ---- 缓动 ----
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}
fn ease_in_out(t: f32) -> f32 {
    let t = clamp01(t);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}
fn ease_out_quad(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}
fn ease_out_back(t: f32) -> f32 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

fn mul_a(c: C4, a: u8) -> C4 {
    (c.0, c.1, c.2, ((c.3 as u16 * a as u16) / 255) as u8)
}

// ---- 模拟数据（随帧变化，便于动画呈现）----
fn mock_snap(frame: u32) -> Snapshot {
    let t = frame as f64 / 30.0;
    Snapshot {
        timestamp: 1700000000 + frame as i64 * 1000,
        hostname: Some("飞牛NAS".into()),
        cpu: Some(Cpu {
            usage: Some(35.0 + (t * 2.3).sin() * 15.0),
            temp: Some(48.0 + (t * 1.7).cos() * 8.0),
            vendor: None,
            load1: None,
            load5: None,
            load15: None,
            load30: None,
        }),
        memory: Some(Memory {
            usage: Some(62.0 + (t * 1.1).sin() * 8.0),
            used: Some(20 * 1024 * 1024 * 1024),
            total: Some(32 * 1024 * 1024 * 1024),
            cache: None,
            free: None,
        }),
        disks: Some(vec![
            Disk {
                device: "sda".into(),
                read_bytes_per_sec: Some((100_000_000.0 + (t * 3.0).sin() * 50_000_000.0) as u64),
                write_bytes_per_sec: Some((80_000_000.0 + (t * 2.5).cos() * 40_000_000.0) as u64),
                used_bytes: Some(800 * 1024 * 1024 * 1024),
                total_bytes: Some(1000 * 1024 * 1024 * 1024),
            },
            Disk {
                device: "sdb".into(),
                read_bytes_per_sec: Some((50_000_000.0 + (t * 2.0).cos() * 20_000_000.0) as u64),
                write_bytes_per_sec: Some((120_000_000.0 + (t * 3.5).sin() * 60_000_000.0) as u64),
                used_bytes: Some(1500 * 1024 * 1024 * 1024),
                total_bytes: Some(2000 * 1024 * 1024 * 1024),
            },
        ]),
        volumes: Some(vec![
            Volume {
                name: "/vol1".into(),
                used_bytes: Some(1800 * 1024 * 1024 * 1024),
                total_bytes: Some(3000 * 1024 * 1024 * 1024),
            },
            Volume {
                name: "/vol2".into(),
                used_bytes: Some(500 * 1024 * 1024 * 1024),
                total_bytes: Some(1000 * 1024 * 1024 * 1024),
            },
        ]),
        gpus: Some(vec![Gpu {
            index: 0,
            name: "Intel UHD 770".into(),
            vendor: Some("intel".into()),
            gpu_type: Some("igpu".into()),
            utilization: Some((28.0 + (t * 1.8).sin() * 12.0) as i32),
            temp: Some((42.0 + (t * 1.3).cos() * 6.0) as i64),
            memory_used: Some(2 * 1024 * 1024 * 1024),
            memory_total: Some(8 * 1024 * 1024 * 1024),
        }]),
        network: Some(vec![NetIf {
            name: "eth0".into(),
            rx_bytes_per_sec: Some((50_000_000.0 + (t * 2.8).sin() * 30_000_000.0) as u64),
            tx_bytes_per_sec: Some((30_000_000.0 + (t * 2.2).cos() * 20_000_000.0) as u64),
            speed_mbps: Some(1000),
        }]),
        traffic: None,
    }
}

// ---- 背景：深色渐变 + 柔和光晕 ----
fn vfill_rect(px: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: C4) {
    if let Some(rc) = Rect::from_xywh(x, y, w, h) {
        let mut pa = Paint::default();
        pa.set_color_rgba8(c.0, c.1, c.2, c.3);
        px.fill_rect(rc, &pa, Transform::identity(), None);
    }
}

fn wallpaper(px: &mut Pixmap) {
    for y in 0..VH {
        let t = y as f32 / VH as f32;
        let r = lerp(BG_TOP.0 as f32, BG_BOT.0 as f32, t) as u8;
        let g = lerp(BG_TOP.1 as f32, BG_BOT.1 as f32, t) as u8;
        let b = lerp(BG_TOP.2 as f32, BG_BOT.2 as f32, t) as u8;
        vfill_rect(px, 0.0, y as f32, VW as f32, 1.0, (r, g, b, 255));
    }
    glow(px, 880.0, 250.0, 780.0, (40, 110, 220), 30);
    glow(px, 110.0, 1680.0, 640.0, (96, 64, 180), 22);
}

fn glow(px: &mut Pixmap, cx: f32, cy: f32, rad: f32, col: (u8, u8, u8), a0: u8) {
    let n = 26;
    for i in (1..=n).rev() {
        let f = i as f32 / n as f32;
        let rr = rad * f;
        let a = (a0 as f32 / 14.0 * f * f) as u8;
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, rr);
        if let Some(p) = pb.finish() {
            let mut pa = Paint::default();
            pa.set_color_rgba8(col.0, col.1, col.2, a.max(1));
            pa.anti_alias = true;
            px.fill_path(&p, &pa, FillRule::Winding, Transform::identity(), None);
        }
    }
}

fn taskbar(px: &mut Pixmap) {
    vfill_rect(px, 0.0, 1808.0, VW as f32, 112.0, BARC);
    vfill_rect(px, 0.0, 1808.0, VW as f32, 1.5, (255, 255, 255, 16));
}

// ---- 基础图元 ----
fn rfill(px: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: C4) {
    fill_round(px, x, y, w, h, r, c);
}

fn rstroke(px: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, wd: f32, c: C4) {
    let p = rounded_rect(x, y, w, h, r);
    let mut pa = Paint::default();
    pa.set_color_rgba8(c.0, c.1, c.2, c.3);
    pa.anti_alias = true;
    px.stroke_path(
        &p,
        &pa,
        &Stroke {
            width: wd,
            ..Default::default()
        },
        Transform::identity(),
        None,
    );
}

fn dot(px: &mut Pixmap, cx: f32, cy: f32, r: f32, c: C4) {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(p) = pb.finish() {
        let mut pa = Paint::default();
        pa.set_color_rgba8(c.0, c.1, c.2, c.3);
        pa.anti_alias = true;
        px.fill_path(&p, &pa, FillRule::Winding, Transform::identity(), None);
    }
}

fn circle_stroke(px: &mut Pixmap, cx: f32, cy: f32, r: f32, wd: f32, c: C4) {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(p) = pb.finish() {
        let mut pa = Paint::default();
        pa.set_color_rgba8(c.0, c.1, c.2, c.3);
        pa.anti_alias = true;
        px.stroke_path(
            &p,
            &pa,
            &Stroke {
                width: wd,
                line_cap: LineCap::Round,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

/// 使用率环（灰轨道 + 分级色弧）
fn ring(px: &mut Pixmap, cx: f32, cy: f32, r: f32, wd: f32, pct: Option<f32>, c: C4, track_a: u8) {
    let do_stroke = |px: &mut Pixmap, path: &Path, col: C4| {
        let mut pa = Paint::default();
        pa.set_color_rgba8(col.0, col.1, col.2, col.3);
        pa.anti_alias = true;
        px.stroke_path(
            path,
            &pa,
            &Stroke {
                width: wd,
                line_cap: LineCap::Round,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    };
    if let Some(p) = arc_path(cx, cy, r, -TAU / 4.0, -TAU / 4.0 + TAU) {
        do_stroke(px, &p, mul_a(TRACKC, track_a));
    }
    if let Some(v) = pct {
        let v = clamp01(v / 100.0);
        if v > 0.005 {
            if let Some(p) = arc_path(cx, cy, r, -TAU / 4.0, -TAU / 4.0 + TAU * v) {
                do_stroke(px, &p, c);
            }
        }
    }
}

/// 上下半环（上橙下紫，网络用）
fn ring_split(px: &mut Pixmap, cx: f32, cy: f32, r: f32, wd: f32, up: f32, down: f32, track_a: u8) {
    let stroke = |px: &mut Pixmap, a0: f32, a1: f32, col: C4| {
        if let Some(p) = arc_path(cx, cy, r, a0, a1) {
            let mut pa = Paint::default();
            pa.set_color_rgba8(col.0, col.1, col.2, col.3);
            pa.anti_alias = true;
            px.stroke_path(
                &p,
                &pa,
                &Stroke {
                    width: wd,
                    line_cap: LineCap::Round,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
    };
    stroke(px, -PI / 2.0, -PI / 2.0 + TAU, mul_a(TRACKC, track_a));
    let u = clamp01(up / 100.0);
    let d = clamp01(down / 100.0);
    if u > 0.005 {
        stroke(px, PI, PI + PI * u, ORANGEC);
    }
    if d > 0.005 {
        stroke(px, PI - PI * d, PI, PURPLEC);
    }
}

fn usage_col(v: f32) -> C4 {
    if v < 60.0 {
        GREENC
    } else if v < 85.0 {
        ORANGEC
    } else {
        REDC
    }
}

// ---- 收起态胶囊（复刻 render_pill）----
fn vicon(px: &mut Pixmap, ic: Option<&Pixmap>, x: f32, cy: f32, s: f32, a: u8) {
    let Some(ic) = ic else { return };
    let target = 11.0 * s;
    let k = target / ic.width() as f32;
    let mut pp = PixmapPaint::default();
    pp.opacity = a as f32 / 255.0;
    let tr = Transform::from_row(k, 0.0, 0.0, k, x, cy - target / 2.0);
    px.draw_pixmap(0, 0, ic.as_ref(), &pp, tr, None);
}

#[allow(clippy::too_many_arguments)]
fn pill_cell(
    px: &mut Pixmap,
    font: &Font,
    x: f32,
    cy: f32,
    fs: f32,
    prefix: Option<(&str, C4)>,
    val: &str,
    a: u8,
) {
    let bl = cy + font.ascent(fs) * 0.36;
    let mut tx = x;
    if let Some((p, c)) = prefix {
        tx = font.draw(px, tx, bl, fs, p, color(c.0, c.1, c.2, a));
        tx += font.measure(fs, "R");
    }
    font.draw(px, tx, bl, fs, val, color(FGC.0, FGC.1, FGC.2, a));
}

fn draw_pill(
    px: &mut Pixmap,
    font: &Font,
    icons: &Icons,
    pcx: f32,
    pcy: f32,
    s: f32,
    frame: u32,
    a: u8,
) {
    let snap = mock_snap(frame);
    let w = 276.0 * s;
    let h = 32.0 * s;
    let x = pcx - w / 2.0;
    let y = pcy - h / 2.0;
    rfill(px, x, y, w, h, 10.0 * s, mul_a(PILLC, a));
    rstroke(
        px,
        x + 0.5,
        y + 0.5,
        w - 1.0,
        h - 1.0,
        10.0 * s,
        s.max(1.0),
        (255, 255, 255, (16.0 * a as f32 / 255.0) as u8),
    );

    let fs = 12.5 * s;
    let after_icon = (11.0 + 4.0) * s;
    let col = |cx0: f32| x + cx0 * s;
    let row = |r0: f32| y + r0 * s;

    // 列1：CPU / 内存
    vicon(px, icons.cpu.as_ref(), col(14.0), row(8.0), s, a);
    pill_cell(
        px,
        font,
        col(14.0) + after_icon,
        row(8.0),
        fs,
        None,
        &format!(
            "{:.0}%",
            snap.cpu
                .as_ref()
                .and_then(|c| c.usage)
                .unwrap_or(0.0)
                .round()
        ),
        a,
    );
    vicon(px, icons.ram.as_ref(), col(14.0), row(24.0), s, a);
    pill_cell(
        px,
        font,
        col(14.0) + after_icon,
        row(24.0),
        fs,
        None,
        &format!(
            "{:.0}%",
            snap.memory
                .as_ref()
                .and_then(|m| m.usage)
                .unwrap_or(0.0)
                .round()
        ),
        a,
    );
    // 列2：R / W
    let (dr, dw) = disk_io_sum(&snap);
    vicon(px, icons.disk.as_ref(), col(96.0), row(8.0), s, a);
    pill_cell(
        px,
        font,
        col(96.0) + after_icon,
        row(8.0),
        fs,
        Some(("R", RBLUEC)),
        &fmt::rate(dr),
        a,
    );
    vicon(px, icons.disk.as_ref(), col(96.0), row(24.0), s, a);
    pill_cell(
        px,
        font,
        col(96.0) + after_icon,
        row(24.0),
        fs,
        Some(("W", GREENC)),
        &fmt::rate(dw),
        a,
    );
    // 列3：UL / DL
    let (nr, nw) = net_io_sum(&snap);
    vicon(px, icons.net.as_ref(), col(180.0), row(8.0), s, a);
    pill_cell(
        px,
        font,
        col(180.0) + after_icon,
        row(8.0),
        fs,
        Some(("UL", ORANGEC)),
        &fmt::rate(nw),
        a,
    );
    vicon(px, icons.net.as_ref(), col(180.0), row(24.0), s, a);
    pill_cell(
        px,
        font,
        col(180.0) + after_icon,
        row(24.0),
        fs,
        Some(("DL", PURPLEC)),
        &fmt::rate(nr),
        a,
    );
}

// ---- 任务栏三环（复刻 draw_rings）----
fn draw_rings_at(px: &mut Pixmap, font: &Font, ox: f32, oy: f32, s: f32, frame: u32, a: u8) {
    let snap = mock_snap(frame);
    let p = |v: f32| v * s;
    let cy = oy + p(24.0);
    let cpu_v = snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) as f32;
    let mem_v = snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) as f32;
    let cc = |c: C4| (c.0, c.1, c.2, (c.3 as u16 * a as u16 / 255) as u8);

    // CPU
    let cx0 = ox + p(24.0);
    ring(
        px,
        cx0,
        cy,
        p(21.0),
        p(5.0),
        Some(cpu_v),
        cc(usage_col(cpu_v)),
        a,
    );
    let vt = format!("{:.0}%", cpu_v.round());
    let vw = font.measure(p(13.5), &vt);
    font.draw(
        px,
        cx0 - vw / 2.0,
        cy - p(2.0),
        p(13.5),
        &vt,
        color(FGC.0, FGC.1, FGC.2, a),
    );
    let lw = font.measure(p(7.5), "CPU");
    font.draw(
        px,
        cx0 - lw / 2.0,
        cy + p(11.0),
        p(7.5),
        "CPU",
        color(DIMC.0, DIMC.1, DIMC.2, a),
    );

    // 内存
    let cx1 = ox + p(95.0);
    ring(
        px,
        cx1,
        cy,
        p(21.0),
        p(5.0),
        Some(mem_v),
        cc(usage_col(mem_v)),
        a,
    );
    let vt = format!("{:.0}%", mem_v.round());
    let vw = font.measure(p(13.5), &vt);
    font.draw(
        px,
        cx1 - vw / 2.0,
        cy - p(2.0),
        p(13.5),
        &vt,
        color(FGC.0, FGC.1, FGC.2, a),
    );
    let lw = font.measure(p(7.5), "内存");
    font.draw(
        px,
        cx1 - lw / 2.0,
        cy + p(11.0),
        p(7.5),
        "内存",
        color(DIMC.0, DIMC.1, DIMC.2, a),
    );

    // 网络（上橙下紫，1Gbps 满环）
    let (nr, nw) = net_io_sum(&snap);
    let cap = 125_000_000u64;
    let ul = (nw as f32 / cap as f32 * 100.0).min(100.0);
    let dl = (nr as f32 / cap as f32 * 100.0).min(100.0);
    let cx2 = ox + p(166.0);
    ring_split(px, cx2, cy, p(21.0), p(5.0), ul, dl, a);
    let us = fmt::rate_compact(nw);
    let ds = fmt::rate_compact(nr);
    let uw = font.measure(p(11.5), &us);
    let dw2 = font.measure(p(11.5), &ds);
    font.draw(
        px,
        cx2 - uw / 2.0,
        cy - p(2.0),
        p(11.5),
        &us,
        color(ORANGEC.0, ORANGEC.1, ORANGEC.2, a),
    );
    font.draw(
        px,
        cx2 - dw2 / 2.0,
        cy + p(10.0),
        p(11.5),
        &ds,
        color(PURPLEC.0, PURPLEC.1, PURPLEC.2, a),
    );
}

// ---- 展开面板（离屏渲染，复刻 build/draw_card 的深色版）----
const PPW: f32 = 320.0;
const PPH: f32 = 854.0; // mock 数据固定高度
const CARD_Y: [f32; 6] = [38.0, 180.0, 306.0, 432.0, 574.0, 716.0];
const CARD_H: [f32; 6] = [132.0, 116.0, 116.0, 132.0, 132.0, 124.0];

/// 合成心电图样本（两条归一化曲线）
fn synth(frame: u32, k: f32) -> Vec<(f32, f32, f32)> {
    let f = frame as f32;
    (0..60)
        .map(|i| {
            let p = i as f32;
            let v1 = clamp01(
                0.60 + 0.25 * (p * 0.45 + f * 0.18 + k).sin()
                    + 0.15 * (p * 0.19 - f * 0.11 + k * 2.7).sin(),
            );
            let v2 = clamp01(
                0.36 + 0.20 * (p * 0.34 - f * 0.14 + k * 1.7).sin()
                    + 0.10 * (p * 0.50 + f * 0.08).sin(),
            );
            (p / 59.0, 1.0 - v1, 1.0 - v2)
        })
        .collect()
}

fn panel_chart(
    px: &mut Pixmap,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    pts: &[(f32, f32, f32)],
    c1: C4,
    c2: C4,
    a: u8,
) {
    rfill(px, cx, cy + ch, cw, 2.0, 0.0, mul_a(TRACKC, a));
    for (comp, c, wd) in [(1usize, c1, 1.6f32), (2, c2, 1.3)] {
        let mut pb_fill = PathBuilder::new();
        let mut pb_line = PathBuilder::new();
        for (i, pt) in pts.iter().enumerate() {
            let vx = cx + pt.0 * cw;
            let v = if comp == 1 { pt.1 } else { pt.2 };
            let vy = cy + v * ch;
            if i == 0 {
                pb_fill.move_to(vx, cy + ch);
                pb_fill.line_to(vx, vy);
                pb_line.move_to(vx, vy);
            } else {
                pb_fill.line_to(vx, vy);
                pb_line.line_to(vx, vy);
            }
        }
        if let Some(last) = pts.last() {
            pb_fill.line_to(cx + last.0 * cw, cy + ch);
            pb_fill.close();
        }
        if let Some(p) = pb_fill.finish() {
            let mut pa = Paint::default();
            pa.set_color_rgba8(
                c.0,
                c.1,
                c.2,
                ((c.3 / 6).max(8) as u16 * a as u16 / 255) as u8,
            );
            pa.anti_alias = true;
            px.fill_path(&p, &pa, FillRule::Winding, Transform::identity(), None);
        }
        if let Some(p) = pb_line.finish() {
            let mut pa = Paint::default();
            pa.set_color_rgba8(c.0, c.1, c.2, (c.3 as u16 * a as u16 / 255) as u8);
            pa.anti_alias = true;
            px.stroke_path(
                &p,
                &pa,
                &Stroke {
                    width: wd,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
    }
}

fn render_panel(frame: u32, font: &Font, s: f32) -> Pixmap {
    let snap = mock_snap(frame);
    let mut pm = Pixmap::new((PPW * s) as u32, (PPH * s) as u32).unwrap();
    // 面板底 + 细边
    rfill(&mut pm, 0.0, 0.0, PPW * s, PPH * s, 14.0 * s, PANELC);
    rstroke(
        &mut pm,
        0.75,
        0.75,
        PPW * s - 1.5,
        PPH * s - 1.5,
        14.0 * s,
        1.0,
        (255, 255, 255, 16),
    );
    // 卡片底
    for i in 0..6 {
        rfill(
            &mut pm,
            14.0 * s,
            CARD_Y[i] * s,
            292.0 * s,
            CARD_H[i] * s,
            10.0 * s,
            CARDC,
        );
    }
    // ---- 标题行：状态灯 + NAS 名 + IP + 齿轮 ----
    let hcy = 24.0 * s;
    dot(&mut pm, 22.0 * s, hcy, 4.0 * s, GREENC);
    let bl = hcy + font.ascent(17.0 * s) * 0.36;
    let nx = 34.0 * s;
    let endx = font.draw_bold(
        &mut pm,
        nx,
        bl,
        17.0 * s,
        "飞牛NAS",
        color(FGC.0, FGC.1, FGC.2, 255),
    );
    font.draw(
        &mut pm,
        endx + 8.0 * s,
        bl,
        12.0 * s,
        "192.168.1.100",
        color(DIMC.0, DIMC.1, DIMC.2, 255),
    );
    let gx = (320.0 - 22.0) * s;
    circle_stroke(&mut pm, gx, hcy, 6.5 * s, 1.6 * s, mul_a(DIMC, 230));
    dot(&mut pm, gx, hcy, 1.8 * s, mul_a(DIMC, 230));

    let cpu_v = snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) as f32;
    let cpu_t = snap.cpu.as_ref().and_then(|c| c.temp).unwrap_or(0.0) as f32;
    let mem_v = snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) as f32;
    let gpu = snap.gpus.as_ref().and_then(|g| g.first());
    let gpu_v = gpu.and_then(|g| g.utilization).unwrap_or(0) as f32;
    let gpu_t = gpu.and_then(|g| g.temp).unwrap_or(0) as f32;

    // ---- 卡1：CPU 双环（使用率 + 温度）----
    let y = CARD_Y[0] * s;
    let h = CARD_H[0] * s;
    let cy = y + h / 2.0;
    let cx_t = (14.0 + 292.0 - CARD_PAD - 33.0 - 1.0) * s;
    let cx_u = cx_t - 82.0 * s;
    font.draw_bold(
        &mut pm,
        14.0 * s,
        cy + font.ascent(16.5 * s) / 2.0,
        16.5 * s,
        "CPU",
        color(FGC.0, FGC.1, FGC.2, 255),
    );
    for (cx, pct, temp) in [(cx_u, Some(cpu_v), false), (cx_t, Some(cpu_t), true)] {
        let v = pct.unwrap();
        let col = if temp { temp_col(v) } else { usage_col(v) };
        let ratio = if temp {
            ((v - 30.0) / 60.0 * 100.0).clamp(0.0, 100.0)
        } else {
            v
        };
        ring(&mut pm, cx, cy, 33.0 * s, 9.0 * s, Some(ratio), col, 255);
        let ct = if temp {
            format!("{:.0}°", v)
        } else {
            format!("{:.0}%", v.round())
        };
        let cw2 = font.measure(17.0 * s, &ct);
        font.draw(
            &mut pm,
            cx - cw2 / 2.0,
            cy - 2.0 * s,
            17.0 * s,
            &ct,
            color(col.0, col.1, col.2, 255),
        );
        let cap = if temp { "温度" } else { "使用率" };
        let capw = font.measure(10.0 * s, cap);
        font.draw(
            &mut pm,
            cx - capw / 2.0,
            cy + 13.0 * s,
            10.0 * s,
            cap,
            color(DIMC.0, DIMC.1, DIMC.2, 255),
        );
    }
    let _ = cx_t;

    // ---- 卡2：内存 单大环 ----
    let y = CARD_Y[1] * s;
    let h = CARD_H[1] * s;
    let cy = y + h / 2.0;
    let mc = usage_col(mem_v);
    let mr = 36.0 * s;
    let mcx = (306.0 - 36.0 - 2.0) * s;
    font.draw_bold(
        &mut pm,
        14.0 * s,
        y + 40.0 * s + font.ascent(16.5 * s),
        16.5 * s,
        "内存",
        color(FGC.0, FGC.1, FGC.2, 255),
    );
    let sub = format!(
        "{} / {}",
        fmt::bytes(snap.memory.as_ref().and_then(|m| m.used).unwrap_or(0)),
        fmt::bytes(snap.memory.as_ref().and_then(|m| m.total).unwrap_or(0))
    );
    font.draw(
        &mut pm,
        14.0 * s,
        y + 74.0 * s,
        12.5 * s,
        &sub,
        color(DIMC.0, DIMC.1, DIMC.2, 255),
    );
    ring(&mut pm, mcx, cy, mr, 8.0 * s, Some(mem_v), mc, 255);
    let mt = format!("{:.0}%", mem_v.round());
    let mw = font.measure(19.0 * s, &mt);
    font.draw(
        &mut pm,
        mcx - mw / 2.0,
        cy + font.ascent(19.0 * s) * 0.35,
        19.0 * s,
        &mt,
        color(mc.0, mc.1, mc.2, 255),
    );

    // ---- 卡3：GPU 双环 ----
    let y = CARD_Y[2] * s;
    let h = CARD_H[2] * s;
    let cy = y + h / 2.0;
    let cx_t = (306.0 - 33.0 - 1.0) * s;
    let cx_u = cx_t - 82.0 * s;
    let avail = cx_u - 33.0 * s - 4.5 * s - 14.0 * s;
    let gname = fit_text_bold(font, 16.5 * s, "GPU0 · Intel UHD 770", avail);
    font.draw_bold(
        &mut pm,
        14.0 * s,
        y + 40.0 * s + font.ascent(16.5 * s),
        16.5 * s,
        &gname,
        color(FGC.0, FGC.1, FGC.2, 255),
    );
    font.draw(
        &mut pm,
        14.0 * s,
        y + 74.0 * s,
        12.5 * s,
        "显存 2.0G / 8.0G",
        color(DIMC.0, DIMC.1, DIMC.2, 255),
    );
    for (cx, v, is_temp) in [(cx_u, gpu_v, false), (cx_t, gpu_t, true)] {
        let col = if is_temp { temp_col(v) } else { usage_col(v) };
        let ratio = if is_temp {
            ((v - 30.0) / 60.0 * 100.0).clamp(0.0, 100.0)
        } else {
            v
        };
        ring(&mut pm, cx, cy, 33.0 * s, 9.0 * s, Some(ratio), col, 255);
        let ct = if is_temp {
            format!("{:.0}°", v)
        } else {
            format!("{:.0}%", v.round())
        };
        let cw2 = font.measure(17.0 * s, &ct);
        font.draw(
            &mut pm,
            cx - cw2 / 2.0,
            cy - 2.0 * s,
            17.0 * s,
            &ct,
            color(col.0, col.1, col.2, 255),
        );
        let cap = if is_temp { "温度" } else { "使用率" };
        let capw = font.measure(10.0 * s, cap);
        font.draw(
            &mut pm,
            cx - capw / 2.0,
            cy + 13.0 * s,
            10.0 * s,
            cap,
            color(DIMC.0, DIMC.1, DIMC.2, 255),
        );
    }

    // ---- 卡4：网络心电图 ----
    let y = CARD_Y[3] * s;
    let h = CARD_H[3] * s;
    font.draw_bold(
        &mut pm,
        14.0 * s,
        y + 24.0 * s,
        16.5 * s,
        "网络 · 1 Gbps",
        color(FGC.0, FGC.1, FGC.2, 255),
    );
    let pts = synth(frame, 1.3);
    panel_chart(
        &mut pm,
        14.0 * s,
        y + 32.0 * s,
        292.0 * s,
        h - 62.0 * s,
        &pts,
        BLUEC,
        ORANGEC,
        255,
    );
    let (nr, nw) = net_io_sum(&snap);
    let d1 = format!("收 {}", fmt::rate(nr));
    let d2 = format!("发 {}", fmt::rate(nw));
    font.draw(
        &mut pm,
        14.0 * s,
        y + h - 8.0 * s,
        12.5 * s,
        &d1,
        color(BLUEC.0, BLUEC.1, BLUEC.2, 255),
    );
    let w2 = font.measure(12.5 * s, &d2);
    font.draw(
        &mut pm,
        306.0 * s - w2,
        y + h - 8.0 * s,
        12.5 * s,
        &d2,
        color(ORANGEC.0, ORANGEC.1, ORANGEC.2, 255),
    );

    // ---- 卡5：硬盘读写心电图 ----
    let y = CARD_Y[4] * s;
    let h = CARD_H[4] * s;
    font.draw_bold(
        &mut pm,
        14.0 * s,
        y + 24.0 * s,
        16.5 * s,
        "硬盘读写",
        color(FGC.0, FGC.1, FGC.2, 255),
    );
    let pts = synth(frame + 20, 4.1);
    panel_chart(
        &mut pm,
        14.0 * s,
        y + 32.0 * s,
        292.0 * s,
        h - 62.0 * s,
        &pts,
        RBLUEC,
        GREENC,
        255,
    );
    let (dr, dw) = disk_io_sum(&snap);
    let d1 = format!("读 {}", fmt::rate(dr));
    let d2 = format!("写 {}", fmt::rate(dw));
    font.draw(
        &mut pm,
        14.0 * s,
        y + h - 8.0 * s,
        12.5 * s,
        &d1,
        color(RBLUEC.0, RBLUEC.1, RBLUEC.2, 255),
    );
    let w2 = font.measure(12.5 * s, &d2);
    font.draw(
        &mut pm,
        306.0 * s - w2,
        y + h - 8.0 * s,
        12.5 * s,
        &d2,
        color(GREENC.0, GREENC.1, GREENC.2, 255),
    );

    // ---- 卡6：存储状态（卷行）----
    let y = CARD_Y[5] * s;
    font.draw_bold(
        &mut pm,
        14.0 * s,
        y + 24.0 * s,
        16.5 * s,
        "存储状态",
        color(FGC.0, FGC.1, FGC.2, 255),
    );
    let note = "2 卷";
    let nw2 = font.measure(12.5 * s, note);
    font.draw(
        &mut pm,
        306.0 * s - nw2,
        y + 24.0 * s,
        12.5 * s,
        note,
        color(DIMC.0, DIMC.1, DIMC.2, 255),
    );
    for (i, vol) in snap
        .volumes
        .as_ref()
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        let ry = y + 28.0 * s + i as f32 * 42.0 * s;
        let cap = format!(
            "{} / {}",
            fmt::bytes(vol.used_bytes.unwrap_or(0)),
            fmt::bytes(vol.total_bytes.unwrap_or(0))
        );
        let ratio = (vol.used_bytes.unwrap_or(0) as f32 / vol.total_bytes.unwrap_or(1) as f32
            * 100.0)
            .clamp(0.0, 100.0);
        let capw = font.measure(12.5 * s, &cap);
        font.draw(
            &mut pm,
            306.0 * s - capw,
            ry + 16.0 * s,
            12.5 * s,
            &cap,
            color(DIMC.0, DIMC.1, DIMC.2, 255),
        );
        font.draw(
            &mut pm,
            14.0 * s,
            ry + 16.0 * s,
            12.5 * s,
            &vol.name,
            color(FGC.0, FGC.1, FGC.2, 255),
        );
        let bw = 292.0 * s;
        let by = ry + 24.0 * s;
        rfill(&mut pm, 14.0 * s, by, bw, 6.0 * s, 3.0 * s, TRACKC);
        rfill(
            &mut pm,
            14.0 * s,
            by,
            (bw * ratio / 100.0).max(6.0 * s),
            6.0 * s,
            3.0 * s,
            usage_col(ratio),
        );
    }

    pm
}

fn temp_col(v: f32) -> C4 {
    if v < 60.0 {
        GREENC
    } else if v < 80.0 {
        ORANGEC
    } else {
        REDC
    }
}

/// 离屏面板 -> 屏幕（tx/ty 为面板左上角屏幕坐标，z 整体缩放）
fn blit(
    px: &mut Pixmap,
    src: &Pixmap,
    z: f32,
    tx: f32,
    ty: f32,
    opacity: f32,
    mask: Option<&Mask>,
) {
    let tr = Transform::from_row(z, 0.0, 0.0, z, tx, ty);
    let mut pp = PixmapPaint::default();
    pp.opacity = opacity.clamp(0.0, 1.0);
    px.draw_pixmap(0, 0, src.as_ref(), &pp, tr, mask);
}

/// 摄像机式 blit：屏幕中心对齐离屏坐标 (cx, cy)
fn blit_cam(px: &mut Pixmap, src: &Pixmap, z: f32, cx: f32, cy: f32) {
    let tx = VW as f32 / 2.0 - z * cx;
    let ty = VH as f32 / 2.0 - z * cy;
    blit(px, src, z, tx, ty, 1.0, None);
}

// ---- 鼠标指针 ----
fn cursor(px: &mut Pixmap, x: f32, y: f32, s: f32, a: u8) {
    let pts: [(f32, f32); 7] = [
        (0.0, 0.0),
        (0.0, 36.0),
        (9.0, 27.0),
        (14.5, 40.0),
        (20.0, 38.0),
        (15.0, 25.0),
        (26.0, 25.0),
    ];
    let build = |shx: f32, shy: f32| -> Option<Path> {
        let mut pb = PathBuilder::new();
        for (i, (px0, py0)) in pts.iter().enumerate() {
            let (vx, vy) = (x + shx + px0 * s, y + shy + py0 * s);
            if i == 0 {
                pb.move_to(vx, vy);
            } else {
                pb.line_to(vx, vy);
            }
        }
        pb.close();
        pb.finish()
    };
    // 阴影
    if let Some(p) = build(2.5, 3.5) {
        let mut pa = Paint::default();
        pa.set_color_rgba8(0, 0, 0, (a as u16 * 150 / 255) as u8);
        pa.anti_alias = true;
        px.fill_path(&p, &pa, FillRule::Winding, Transform::identity(), None);
    }
    if let Some(p) = build(0.0, 0.0) {
        let mut pa = Paint::default();
        pa.set_color_rgba8(255, 255, 255, a);
        pa.anti_alias = true;
        px.fill_path(&p, &pa, FillRule::Winding, Transform::identity(), None);
        let mut sp = Paint::default();
        sp.set_color_rgba8(20, 22, 28, a);
        sp.anti_alias = true;
        px.stroke_path(
            &p,
            &sp,
            &Stroke {
                width: 1.6,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

// ---- 场景首尾黑场过渡 ----
fn fade_edges(px: &mut Pixmap, frame: u32, total: u32) {
    let n = 8u32;
    let a = if frame < n {
        255.0 * (1.0 - frame as f32 / n as f32)
    } else if frame >= total - n {
        255.0 * ((frame - (total - n)) as f32 / n as f32)
    } else {
        0.0
    };
    if a > 0.0 {
        vfill_rect(px, 0.0, 0.0, VW as f32, VH as f32, (0, 0, 0, a as u8));
    }
}

fn center_text(px: &mut Pixmap, font: &Font, cy: f32, fs: f32, text: &str, c: C4, bold: bool) {
    let w = if bold {
        font.measure_bold(fs, text)
    } else {
        font.measure(fs, text)
    };
    let x = (VW as f32 - w) / 2.0;
    if bold {
        font.draw_bold(px, x, cy, fs, text, color(c.0, c.1, c.2, c.3));
    } else {
        font.draw(px, x, cy, fs, text, color(c.0, c.1, c.2, c.3));
    }
}

// ============ 场景 1：钩子 ============
fn scene1(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    let total = fc(0);
    let t = frame as f32 / total as f32;
    let a_in = ease_out_cubic(clamp01(t / 0.25));
    let a_out = 1.0 - ease_in_out(clamp01((t - 0.78) / 0.18));
    let a = (a_in * a_out * 255.0) as u8;
    let rise = (1.0 - ease_out_cubic(clamp01(t / 0.3))) * 18.0;
    center_text(
        &mut px,
        font,
        780.0 - rise,
        72.0,
        "还在开浏览器",
        mul_a(FGC, a),
        true,
    );
    center_text(
        &mut px,
        font,
        892.0 - rise,
        72.0,
        "看 NAS 状态？",
        mul_a(FGC, a),
        true,
    );
    // 蓝色强调下划线
    let uw = ease_out_cubic(clamp01((t - 0.28) / 0.3)) * 260.0;
    if uw > 1.0 {
        let line_c = (
            BLUEC.0,
            BLUEC.1,
            BLUEC.2,
            (BLUEC.3 as u16 * a as u16 / 255) as u8,
        );
        rfill(
            &mut px,
            (VW as f32 - 260.0) / 2.0,
            940.0 - rise,
            uw,
            6.0,
            3.0,
            line_c,
        );
    }
    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 2：悬浮胶囊特写 ============
fn scene2(frame: u32, font: &Font, icons: &Icons) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    let total = fc(1);
    let t = frame as f32 / total as f32;

    let e = ease_out_back(clamp01(t / 0.30));
    let sc = 0.45 + 0.55 * e;
    let pa = (clamp01(t / 0.18) * 255.0) as u8;
    draw_pill(&mut px, font, icons, 540.0, 800.0, 2.6 * sc, frame, pa);

    let a1 = (ease_out_cubic(clamp01((t - 0.36) / 0.18)) * 255.0) as u8;
    center_text(&mut px, font, 560.0, 66.0, "悬浮胶囊", mul_a(FGC, a1), true);
    center_text(
        &mut px,
        font,
        1040.0,
        36.0,
        "六项关键指标 一目了然",
        mul_a(DIMC, a1),
        false,
    );

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 3：融入任务栏 ============
fn scene3(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    taskbar(&mut px);
    let total = fc(2);
    let t = frame as f32 / total as f32;

    // 三环从右侧滑入
    let slide = ease_out_cubic(clamp01(t / 0.32));
    let center_x = lerp(VW as f32 + 280.0, VW as f32 / 2.0, slide);
    // 入场后缓慢镜头推进（特写三环）
    let push = ease_in_out(clamp01((t - 0.42) / 0.42));
    let s = 2.35 * (1.0 + 0.15 * push);
    let ox = center_x - 95.0 * s;
    // 以任务栏中线为锚点推进，并保证三环（含描边/标注）不被屏幕底边裁切
    let oy = (1864.0 - 24.0 * s).min(1902.0 - 48.0 * s);
    let ra = (clamp01((t - 0.18) / 0.15) * 255.0) as u8;
    draw_rings_at(&mut px, font, ox, oy, s, frame, ra);

    // 标题
    let a1 = (ease_out_cubic(clamp01((t - 0.14) / 0.18))
        * (1.0 - ease_in_out(clamp01((t - 0.82) / 0.12)))
        * 255.0) as u8;
    font.draw_bold(
        &mut px,
        92.0,
        720.0,
        64.0,
        "融入任务栏",
        color(FGC.0, FGC.1, FGC.2, a1),
    );
    font.draw(
        &mut px,
        94.0,
        800.0,
        34.0,
        "三环设计 · 不遮挡内容",
        color(DIMC.0, DIMC.1, DIMC.2, a1),
    );

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 4：点击展开交互 ============
fn scene4(frame: u32, font: &Font, icons: &Icons) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    taskbar(&mut px);
    let total = fc(3);
    let t = frame as f32 / total as f32;

    const PS: f32 = 1.6;
    let panel = render_panel(frame, font, PS);
    let pill_cy = 270.0;

    // 面板展开（裁剪遮罩控制生长高度）
    let rv = ease_out_cubic(clamp01((t - 0.30) / 0.45));
    if rv > 0.001 {
        let pwp = PPW * PS;
        let php = PPH * PS;
        let px0 = (VW as f32 - pwp) / 2.0;
        let py0 = pill_cy + 16.0 * PS + 10.0;
        let rh = php * rv;
        let mut mask = Mask::new(VW, VH).unwrap();
        let path = rounded_rect(px0, py0, pwp, rh, 22.0);
        mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
        let op = ease_out_quad(clamp01((t - 0.30) / 0.18));
        blit(&mut px, &panel, 1.0, px0, py0, op, Some(&mask));
    }

    // 胶囊
    draw_pill(&mut px, font, icons, 540.0, pill_cy, PS, frame, 255);

    // 点击波纹
    if t >= 0.22 && t < 0.46 {
        let k = ease_out_cubic(clamp01((t - 0.22) / 0.24));
        let r = 8.0 + k * 110.0;
        let a = ((1.0 - k) * 200.0) as u8;
        circle_stroke(
            &mut px,
            540.0,
            pill_cy,
            r,
            4.0,
            (BLUEC.0, BLUEC.1, BLUEC.2, a),
        );
    }

    // 鼠标
    let (mx, my, ms, ma) = if t < 0.22 {
        let k = ease_in_out(t / 0.22);
        (lerp(950.0, 540.0, k), lerp(1480.0, pill_cy, k), 1.5, 255.0)
    } else if t < 0.32 {
        (540.0, pill_cy, 1.32, 255.0)
    } else if t < 0.46 {
        let k = (t - 0.32) / 0.14;
        (
            lerp(540.0, 700.0, k),
            lerp(pill_cy, 190.0, k),
            1.32,
            (1.0 - k) * 255.0,
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    if ma > 1.0 {
        cursor(&mut px, mx, my, ms, ma as u8);
    }

    // 标题（面板展开后淡出）
    let a1 = (1.0 - ease_in_out(clamp01((t - 0.50) / 0.14)) * 255.0 / 255.0 * 1.0) * 255.0;
    let a1 = a1.clamp(0.0, 255.0) as u8;
    let a2 = (a1 as u16 * 80 / 100) as u8;
    font.draw_bold(
        &mut px,
        92.0,
        130.0,
        60.0,
        "点一下",
        color(FGC.0, FGC.1, FGC.2, a1),
    );
    font.draw(
        &mut px,
        94.0,
        196.0,
        34.0,
        "完整面板即刻展开",
        color(DIMC.0, DIMC.1, DIMC.2, a2),
    );

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 5：镜头推进功能特写 ============
/// 摄像机关键帧 (时间点, 缩放, 离屏中心y)
const CAM_KF: [(f32, f32, f32); 8] = [
    (0.00, 0.80, 940.0),
    (0.15, 0.80, 940.0),
    (0.31, 1.45, 229.0),
    (0.47, 1.45, 801.0),
    (0.63, 1.45, 1096.0),
    (0.79, 1.45, 1620.0),
    (0.93, 0.80, 940.0),
    (1.01, 0.80, 940.0),
];

fn cam_at(t: f32) -> (f32, f32) {
    for i in 0..CAM_KF.len() - 1 {
        let (t0, z0, y0) = CAM_KF[i];
        let (t1, z1, y1) = CAM_KF[i + 1];
        if t >= t0 && t <= t1 {
            let k = ease_in_out((t - t0) / (t1 - t0));
            return (lerp(z0, z1, k), lerp(y0, y1, k));
        }
    }
    (0.80, 940.0)
}

fn caption(px: &mut Pixmap, font: &Font, t: f32, start: f32, end: f32, text: &str) {
    let fin = 0.07;
    let a = if t < start || t > end {
        0.0
    } else if t < start + fin {
        ease_out_cubic((t - start) / fin)
    } else if t > end - fin {
        1.0 - ease_in_out((t - (end - fin)) / fin)
    } else {
        1.0
    };
    if a <= 0.01 {
        return;
    }
    let fs = 32.0;
    let tw = font.measure(fs, text);
    let bw = tw + 64.0;
    let bh = 66.0;
    let bx = (VW as f32 - bw) / 2.0;
    let by = 1700.0;
    rfill(
        px,
        bx,
        by,
        bw,
        bh,
        bh / 2.0,
        (10, 13, 18, (200.0 * a) as u8),
    );
    center_text(
        px,
        font,
        by + 44.0,
        fs,
        text,
        (FGC.0, FGC.1, FGC.2, (255.0 * a) as u8),
        false,
    );
}

fn scene5(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    let total = fc(4);
    let t = frame as f32 / total as f32;

    const PS: f32 = 2.2;
    let panel = render_panel(frame, font, PS);
    let (z, cy) = cam_at(t);
    blit_cam(&mut px, &panel, z, PPW * PS / 2.0, cy);

    // 开场标题
    let a0 = (ease_out_cubic(clamp01(t / 0.12))
        * (1.0 - ease_in_out(clamp01((t - 0.22) / 0.12)))
        * 255.0) as u8;
    center_text(
        &mut px,
        font,
        200.0,
        48.0,
        "完整功能面板",
        mul_a(FGC, a0),
        true,
    );

    // 各特写段的功能标注
    caption(&mut px, font, t, 0.21, 0.40, "CPU 使用率 · 温度双环");
    caption(&mut px, font, t, 0.37, 0.56, "内存 · 显卡 实时占用");
    caption(&mut px, font, t, 0.53, 0.72, "网络速率 心电图");
    caption(&mut px, font, t, 0.69, 0.90, "硬盘读写 · 存储用量");

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 6：收尾品牌 ============
fn scene6(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    let total = fc(5);
    let t = frame as f32 / total as f32;

    // 品牌三环标识（描边生长动画）
    let snap = mock_snap(frame);
    let draw = ease_out_cubic(clamp01(t / 0.32));
    let cpu_v = snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) as f32;
    let mem_v = snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) as f32;
    let cyy = 560.0;
    let r = 46.0;
    ring(
        &mut px,
        540.0 - 132.0,
        cyy,
        r,
        9.0,
        Some(cpu_v * draw),
        usage_col(cpu_v),
        255,
    );
    ring(
        &mut px,
        540.0,
        cyy,
        r,
        9.0,
        Some(mem_v * draw),
        usage_col(mem_v),
        255,
    );
    let (nr, nw) = net_io_sum(&snap);
    let cap = 125_000_000u64;
    ring_split(
        &mut px,
        540.0 + 132.0,
        cyy,
        r,
        9.0,
        nw as f32 / cap as f32 * 100.0 * draw,
        nr as f32 / cap as f32 * 100.0 * draw,
        255,
    );
    center_text(
        &mut px,
        font,
        cyy + 88.0,
        20.0,
        "CPU  内存  网络",
        mul_a(DIMC, (clamp01((t - 0.15) / 0.2) * 255.0) as u8),
        false,
    );

    // 文字依次浮入
    let rise = |start: f32| -> (u8, f32) {
        let k = ease_out_cubic(clamp01((t - start) / 0.22));
        ((k * 255.0) as u8, (1.0 - k) * 22.0)
    };
    let (a1, r1) = rise(0.22);
    center_text(
        &mut px,
        font,
        830.0 - r1,
        80.0,
        "核桃胶囊",
        mul_a(BLUEC, a1),
        true,
    );
    let (a2, r2) = rise(0.38);
    center_text(
        &mut px,
        font,
        956.0 - r2,
        40.0,
        "飞牛应用中心 · 内测上线",
        mul_a(FGC, a2),
        true,
    );
    let (a3, r3) = rise(0.52);
    center_text(
        &mut px,
        font,
        1040.0 - r3,
        32.0,
        "全功能免费",
        mul_a(DIMC, a3),
        false,
    );

    fade_edges(&mut px, frame, total);
    px
}

#[test]
fn export_full_video_v2() {
    let font = Font::load().unwrap();
    let icons = Icons::load(56, FGC);
    let out_dir = std::path::Path::new("d:\\nas胶囊\\视频帧-全功能");
    std::fs::create_dir_all(out_dir).unwrap();

    let total_frames: u32 = (0..6).map(|i| fc(i)).sum();
    println!(
        "Total frames: {}, duration: {:.1}s",
        total_frames,
        total_frames as f32 / VFPS as f32
    );

    // 可选：环境变量 VSCENES="5" 只重渲指定场景（帧号仍按全局序号输出，便于增量覆盖）
    let only: Option<Vec<usize>> = std::env::var("VSCENES").ok().map(|v| {
        v.split(',')
            .filter_map(|x| x.trim().parse::<usize>().ok())
            .collect()
    });

    let mut gf = 0u32;
    for scene in 0..6 {
        let count = fc(scene);
        let skip = only.as_ref().is_some_and(|v| !v.contains(&scene));
        println!(
            "Scene {}: {} frames ({:.1}s){}",
            scene + 1,
            count,
            SC[scene],
            if skip { " [skipped]" } else { "" }
        );
        for local in 0..count {
            if !skip {
                let px = match scene {
                    0 => scene1(local, &font),
                    1 => scene2(local, &font, &icons),
                    2 => scene3(local, &font),
                    3 => scene4(local, &font, &icons),
                    4 => scene5(local, &font),
                    5 => scene6(local, &font),
                    _ => unreachable!(),
                };
                px.save_png(&out_dir.join(format!("frame_{:05}.png", gf)))
                    .unwrap();
            }
            gf += 1;
        }
    }
    println!(
        "All {} frames exported to: {}",
        total_frames,
        out_dir.display()
    );
}
