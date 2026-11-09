use super::*;
use crate::data::{Cpu, Disk, Gpu, Memory, NetIf, Snapshot, Volume};
use crate::fmt;
use crate::icons::Icons;

const VW: u32 = 1920;
const VH: u32 = 1080;
const VFPS: u32 = 30;
const SC: [f32; 6] = [3.0, 4.0, 4.0, 6.0, 9.0, 4.0];
fn fc(i: usize) -> u32 {
    (SC[i] * VFPS as f32).round() as u32
}

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

fn vfill_rect(px: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: C4) {
    if let Some(rc) = Rect::from_xywh(x, y, w, h) {
        let mut pa = Paint::default();
        pa.set_color_rgba8(c.0, c.1, c.2, c.3);
        px.fill_rect(rc, &pa, Transform::identity(), None);
    }
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

fn wallpaper(px: &mut Pixmap) {
    for y in 0..VH {
        let t = y as f32 / VH as f32;
        let r = lerp(BG_TOP.0 as f32, BG_BOT.0 as f32, t) as u8;
        let g = lerp(BG_TOP.1 as f32, BG_BOT.1 as f32, t) as u8;
        let b = lerp(BG_TOP.2 as f32, BG_BOT.2 as f32, t) as u8;
        vfill_rect(px, 0.0, y as f32, VW as f32, 1.0, (r, g, b, 255));
    }
    glow(px, 1520.0, 140.0, 720.0, (40, 110, 220), 30);
    glow(px, 200.0, 940.0, 600.0, (96, 64, 180), 22);
}

/// 横屏底部任务栏
fn taskbar(px: &mut Pixmap) {
    vfill_rect(px, 0.0, 948.0, VW as f32, 132.0, BARC);
    vfill_rect(px, 0.0, 948.0, VW as f32, 1.5, (255, 255, 255, 16));
}

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
fn temp_col(v: f32) -> C4 {
    if v < 60.0 {
        GREENC
    } else if v < 80.0 {
        ORANGEC
    } else {
        REDC
    }
}

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

/// 未固定展开态的顶栏内容（状态灯 + NAS 名 + IP + 齿轮），x/y 为 320×52 顶栏左上角
fn draw_bar_head(px: &mut Pixmap, font: &Font, x: f32, y: f32, s: f32, a: u8) {
    let cyy = y + 26.0 * s;
    dot(px, x + 22.0 * s, cyy, 4.0 * s, mul_a(GREENC, a));
    let bl = cyy + font.ascent(17.0 * s) * 0.36;
    let nx = x + 34.0 * s;
    let endx = font.draw_bold(
        px,
        nx,
        bl,
        17.0 * s,
        "飞牛NAS",
        color(FGC.0, FGC.1, FGC.2, a),
    );
    font.draw(
        px,
        endx + 8.0 * s,
        bl,
        12.0 * s,
        "192.168.1.100",
        color(DIMC.0, DIMC.1, DIMC.2, a),
    );
    let gx = x + (320.0 - 22.0) * s;
    circle_stroke(px, gx, cyy, 6.5 * s, 1.6 * s, mul_a(DIMC, a));
    dot(px, gx, cyy, 1.8 * s, mul_a(DIMC, a));
}

fn draw_rings_at(px: &mut Pixmap, font: &Font, ox: f32, oy: f32, s: f32, frame: u32, a: u8) {
    let snap = mock_snap(frame);
    let p = |v: f32| v * s;
    let cy = oy + p(24.0);
    let cpu_v = snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) as f32;
    let mem_v = snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) as f32;
    let cc = |c: C4| (c.0, c.1, c.2, (c.3 as u16 * a as u16 / 255) as u8);

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

// 竖版三环（竖排胶囊收起态）：56×178，三环纵向堆叠
fn draw_rings_v_at(px: &mut Pixmap, font: &Font, ox: f32, oy: f32, s: f32, frame: u32, a: u8) {
    let snap = mock_snap(frame);
    let p = |v: f32| v * s;
    let cc = |c: C4| (c.0, c.1, c.2, (c.3 as u16 * a as u16 / 255) as u8);

    // 胶囊底
    fill_round(
        px,
        ox,
        oy,
        p(PILL_V_W as f32),
        p(PILL_V_H as f32),
        p(PILL_R),
        (BG.0, BG.1, BG.2, a),
    );

    let cx = ox + p(RING_V_CX);
    let cpu_v = snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) as f32;
    let mem_v = snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) as f32;

    // CPU 环（上）
    let cy0 = oy + p(RING_V_CY[0]);
    ring(
        px,
        cx,
        cy0,
        p(RING_V_R),
        p(RING_V_STROKE),
        Some(cpu_v),
        cc(usage_col(cpu_v)),
        a,
    );
    let vt = format!("{:.0}%", cpu_v.round());
    let vw = font.measure(p(RING_V_VALUE_FS), &vt);
    font.draw_bold(
        px,
        cx - vw / 2.0,
        cy0 - p(RING_V_VALUE_FS * 0.36),
        p(RING_V_VALUE_FS),
        &vt,
        color(FGC.0, FGC.1, FGC.2, a),
    );
    let lw = font.measure(p(RING_V_LABEL_FS), "CPU");
    font.draw(
        px,
        cx - lw / 2.0,
        cy0 + p(RING_V_R - 4.0),
        p(RING_V_LABEL_FS),
        "CPU",
        color(DIMC.0, DIMC.1, DIMC.2, a),
    );

    // 内存环（中）
    let cy1 = oy + p(RING_V_CY[1]);
    ring(
        px,
        cx,
        cy1,
        p(RING_V_R),
        p(RING_V_STROKE),
        Some(mem_v),
        cc(usage_col(mem_v)),
        a,
    );
    let vt = format!("{:.0}%", mem_v.round());
    let vw = font.measure(p(RING_V_VALUE_FS), &vt);
    font.draw_bold(
        px,
        cx - vw / 2.0,
        cy1 - p(RING_V_VALUE_FS * 0.36),
        p(RING_V_VALUE_FS),
        &vt,
        color(FGC.0, FGC.1, FGC.2, a),
    );
    let lw = font.measure(p(RING_V_LABEL_FS), "内存");
    font.draw(
        px,
        cx - lw / 2.0,
        cy1 + p(RING_V_R - 4.0),
        p(RING_V_LABEL_FS),
        "内存",
        color(DIMC.0, DIMC.1, DIMC.2, a),
    );

    // 网络环（下，上下半环）
    let (nr, nw) = net_io_sum(&snap);
    let cap = 125_000_000u64;
    let ul = (nw as f32 / cap as f32 * 100.0).min(100.0);
    let dl = (nr as f32 / cap as f32 * 100.0).min(100.0);
    let cy2 = oy + p(RING_V_CY[2]);
    ring_split(px, cx, cy2, p(RING_V_R), p(RING_V_STROKE), ul, dl, a);
    let us = fmt::rate_compact(nw);
    let ds = fmt::rate_compact(nr);
    let uw = font.measure(p(RING_V_NET_FS), &us);
    let dw = font.measure(p(RING_V_NET_FS), &ds);
    font.draw_bold(
        px,
        cx - uw / 2.0,
        cy2 - p(RING_V_NET_FS * 0.85),
        p(RING_V_NET_FS),
        &us,
        color(ORANGEC.0, ORANGEC.1, ORANGEC.2, a),
    );
    font.draw_bold(
        px,
        cx - dw / 2.0,
        cy2 + p(RING_V_NET_FS * 0.15),
        p(RING_V_NET_FS),
        &ds,
        color(PURPLEC.0, PURPLEC.1, PURPLEC.2, a),
    );
}

const PPW: f32 = 320.0;
const PPH: f32 = 854.0;
const CARD_Y: [f32; 6] = [38.0, 180.0, 306.0, 432.0, 574.0, 716.0];
const CARD_H: [f32; 6] = [132.0, 116.0, 116.0, 132.0, 132.0, 124.0];

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

fn blit_cam(px: &mut Pixmap, src: &Pixmap, z: f32, cx: f32, cy: f32) {
    let tx = VW as f32 / 2.0 - z * cx;
    let ty = VH as f32 / 2.0 - z * cy;
    blit(px, src, z, tx, ty, 1.0, None);
}

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
        430.0 - rise,
        88.0,
        "还在开浏览器",
        mul_a(FGC, a),
        true,
    );
    center_text(
        &mut px,
        font,
        552.0 - rise,
        88.0,
        "看 NAS 状态？",
        mul_a(FGC, a),
        true,
    );
    let uw = ease_out_cubic(clamp01((t - 0.28) / 0.3)) * 320.0;
    if uw > 1.0 {
        let line_c = (
            BLUEC.0,
            BLUEC.1,
            BLUEC.2,
            (BLUEC.3 as u16 * a as u16 / 255) as u8,
        );
        rfill(
            &mut px,
            (VW as f32 - 320.0) / 2.0,
            604.0 - rise,
            uw,
            6.0,
            3.0,
            line_c,
        );
    }
    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 2：悬浮胶囊（真实形态：贴屏幕右上角）============
fn scene2(frame: u32, font: &Font, icons: &Icons) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    let total = fc(1);
    let t = frame as f32 / total as f32;

    // ---------- A 段（t<0.45）：横向药丸胶囊，六项指标 ----------
    let a_out = 1.0 - clamp01((t - 0.40) / 0.05); // 0.40 起淡出，0.45 消失
    if a_out > 0.0 {
        const PS2: f32 = 2.4;
        const RIGHT_M: f32 = 170.0;
        const TOP_M: f32 = 150.0;
        let pcx = VW as f32 - RIGHT_M - 138.0 * PS2;
        let pcy = TOP_M + 16.0 * PS2;
        let e = ease_out_back(clamp01(t / 0.30));
        let sc = 0.45 + 0.55 * e;
        let cx = pcx + (1.0 - e) * 60.0;
        let cy = pcy + (1.0 - e) * 14.0;
        let pa = (clamp01(t / 0.18) * 255.0 * a_out) as u8;
        draw_pill(&mut px, font, icons, cx, cy, PS2 * sc, frame, pa);
    }

    // ---------- B 段（t>=0.45）：竖排胶囊，竖版三环纵向堆叠 ----------
    let v_in = clamp01((t - 0.45) / 0.10); // 0.45 起淡入，0.55 到位
    if v_in > 0.0 {
        const VS: f32 = 2.2; // 竖排胶囊放大倍数（56×178 → 123×392）
        let vw = PILL_V_W as f32 * VS;
        let vh = PILL_V_H as f32 * VS;
        // 放在画面右侧中部，与横向胶囊同列
        let vx = VW as f32 - 200.0 - vw;
        let vy = (VH as f32 - vh) / 2.0;
        let va = (v_in * 255.0) as u8;
        draw_rings_v_at(&mut px, font, vx, vy, VS, frame, va);
    }

    // 标题（全程，A→B 时不淡出）
    let a1 = (ease_out_cubic(clamp01((t - 0.36) / 0.18)) * 255.0) as u8;
    font.draw_bold(
        &mut px,
        150.0,
        430.0,
        80.0,
        "悬浮胶囊",
        color(FGC.0, FGC.1, FGC.2, a1),
    );
    font.draw(
        &mut px,
        152.0,
        512.0,
        42.0,
        "横竖两种形态 · 一目了然",
        color(DIMC.0, DIMC.1, DIMC.2, a1),
    );

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 3：融入任务栏（横屏任务栏在底部）============
fn scene3(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    taskbar(&mut px);
    let total = fc(2);
    let t = frame as f32 / total as f32;

    let slide = ease_out_cubic(clamp01(t / 0.32));
    let center_x = lerp(VW as f32 + 300.0, VW as f32 / 2.0, slide);
    let push = ease_in_out(clamp01((t - 0.42) / 0.42));
    let s = 2.15 * (1.0 + 0.12 * push);
    let ox = center_x - 95.0 * s;
    // 锚定任务栏中线（1014），同时保证三环整体在画面内
    let oy = (1014.0 - 24.0 * s).min(1072.0 - 48.0 * s);
    let ra = (clamp01((t - 0.18) / 0.15) * 255.0) as u8;
    draw_rings_at(&mut px, font, ox, oy, s, frame, ra);

    let a1 = (ease_out_cubic(clamp01((t - 0.14) / 0.18))
        * (1.0 - ease_in_out(clamp01((t - 0.82) / 0.12)))
        * 255.0) as u8;
    font.draw_bold(
        &mut px,
        150.0,
        386.0,
        80.0,
        "融入任务栏",
        color(FGC.0, FGC.1, FGC.2, a1),
    );
    font.draw(
        &mut px,
        152.0,
        466.0,
        42.0,
        "三环设计 · 不遮挡内容",
        color(DIMC.0, DIMC.1, DIMC.2, a1),
    );

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 4：两种形态点击展开 ============
// A 段（t<0.5）：屏幕右上角的悬浮胶囊 → 点击，顶栏拉宽、面板向“下”展开（真实方向）
// B 段（t≥0.5）：底部任务栏三环 → 点击，面板向“上”生长
fn scene4(frame: u32, font: &Font, icons: &Icons) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    taskbar(&mut px);
    let total = fc(3);
    let t = frame as f32 / total as f32;

    // ---------------- A 段：悬浮胶囊（右上角，1:1 真实比例）----------------
    let ga = clamp01((0.50 - t) / 0.06); // 0.44 起淡出，0.50 消失
    if ga > 0.0 {
        const SA: f32 = 0.92;
        const RE_A: f32 = 1800.0; // 窗口右缘（真实产品右缘贴屏幕右上角）
        const BY_A: f32 = 70.0; // 窗口顶
        let pill_cx = RE_A - 138.0 * SA;
        let pill_cy = BY_A + 16.0 * SA;

        // 顶栏：276×32 药丸 → 320×52 顶栏（右缘不动，向左加宽）
        let grow = ease_out_cubic(clamp01((t - 0.24) / 0.18));
        let bw = (276.0 + (320.0 - 276.0) * grow) * SA;
        let bh = (32.0 + (52.0 - 32.0) * grow) * SA;
        let bx = RE_A - bw;
        rfill(
            &mut px,
            bx,
            BY_A,
            bw,
            bh,
            (10.0 + 4.0 * grow) * SA,
            mul_a(PILLC, (ga * 255.0) as u8),
        );

        // 收起态六项指标（随拉宽淡出）
        let ma = ((1.0 - clamp01((t - 0.22) / 0.14)) * ga * 255.0) as u8;
        draw_pill(&mut px, font, icons, pill_cx, pill_cy, SA, frame, ma);

        // 顶栏标题内容（拉宽后淡入）
        let ha = (ease_out_quad(clamp01((t - 0.30) / 0.16)) * ga * 255.0) as u8;
        draw_bar_head(&mut px, font, RE_A - 320.0 * SA, BY_A, SA, ha);

        // 面板向下生长：遮罩从顶栏底部向下扩张；render_panel 自带的 38px 标题行
        // 与顶栏内容重复，上移 38px 只取卡片区（0.46s 前必须全部展开完毕）
        let rv = ease_out_cubic(clamp01((t - 0.28) / 0.18));
        if rv > 0.001 {
            let panel = render_panel(frame, font, SA);
            let pwp = 320.0 * SA;
            let px0 = RE_A - pwp;
            let py = BY_A + 52.0 * SA + 8.0 * SA;
            let rh = (PPH - 38.0) * SA * rv;
            let mut mask = Mask::new(VW, VH).unwrap();
            let path = rounded_rect(px0, py, pwp, rh, 14.0 * SA);
            mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
            let op = ease_out_quad(clamp01((t - 0.30) / 0.18)) * ga;
            blit(&mut px, &panel, 1.0, px0, py - 38.0 * SA, op, Some(&mask));
        }

        // 点击波纹：“点一下”配音锚点 12.32s = t 0.22
        if t >= 0.22 && t < 0.46 {
            let k = ease_out_cubic(clamp01((t - 0.22) / 0.24));
            let a = ((1.0 - k) * 200.0 * ga) as u8;
            circle_stroke(
                &mut px,
                pill_cx,
                pill_cy,
                8.0 + k * 130.0,
                4.0,
                (BLUEC.0, BLUEC.1, BLUEC.2, a),
            );
        }

        // 鼠标：左下移入 → 点击 → 向右上移出
        let (mx, my, ms, mc) = if t < 0.22 {
            let k = ease_in_out(t / 0.22);
            (
                lerp(1360.0, pill_cx, k),
                lerp(430.0, pill_cy, k),
                1.5,
                255.0,
            )
        } else if t < 0.34 {
            (pill_cx, pill_cy, 1.3, 255.0)
        } else if t < 0.44 {
            let k = (t - 0.34) / 0.10;
            (
                lerp(pill_cx, 1560.0, k),
                lerp(pill_cy, 210.0, k),
                1.3,
                (1.0 - k) * 255.0,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let mc = (mc * ga) as u8;
        if mc > 1 {
            cursor(&mut px, mx, my, ms, mc);
        }
    }

    // ---------------- B 段：任务栏三环点击向上展开 ----------------
    let gb = clamp01((t - 0.48) / 0.08) * (1.0 - clamp01((t - 0.985) / 0.015));
    if gb > 0.0 {
        const SB: f32 = 0.98; // 面板缩放
        const RS: f32 = 2.15; // 三环缩放（与场景3一致）
        let ring_cx = VW as f32 / 2.0;
        let ring_cy = 1014.0;
        let ox = ring_cx - 95.0 * RS;
        let oy = ring_cy - 24.0 * RS;
        draw_rings_at(&mut px, font, ox, oy, RS, frame, (gb * 255.0) as u8);

        // 面板自三环向上生长
        let rv = ease_out_cubic(clamp01((t - 0.70) / 0.26));
        if rv > 0.001 {
            let panel = render_panel(frame, font, SB);
            let pwp = 320.0 * SB;
            let php = PPH * SB;
            let px0 = (VW as f32 - pwp) / 2.0;
            let pbot = 948.0 - 10.0;
            let rh = php * rv;
            let mut mask = Mask::new(VW, VH).unwrap();
            let path = rounded_rect(px0, pbot - rh, pwp, rh, 14.0 * SB);
            mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
            let op = ease_out_quad(clamp01((t - 0.70) / 0.16)) * gb;
            blit(&mut px, &panel, 1.0, px0, pbot - php, op, Some(&mask));
        }

        // 三环点击波纹
        if t >= 0.68 && t < 0.92 {
            let k = ease_out_cubic(clamp01((t - 0.68) / 0.24));
            let a = ((1.0 - k) * 200.0 * gb) as u8;
            circle_stroke(
                &mut px,
                ring_cx,
                ring_cy,
                8.0 + k * 130.0,
                4.0,
                (BLUEC.0, BLUEC.1, BLUEC.2, a),
            );
        }

        // 鼠标：右上方向移入任务栏 → 点击 → 原路淡出
        let (mx, my, ms, mc) = if t < 0.56 {
            (0.0, 0.0, 0.0, 0.0)
        } else if t < 0.68 {
            let k = ease_in_out((t - 0.56) / 0.12);
            (
                lerp(1460.0, ring_cx, k),
                lerp(770.0, ring_cy, k),
                1.5,
                255.0,
            )
        } else if t < 0.80 {
            (ring_cx, ring_cy, 1.3, 255.0)
        } else if t < 0.90 {
            let k = (t - 0.80) / 0.10;
            (
                lerp(ring_cx, 1380.0, k),
                lerp(ring_cy, 780.0, k),
                1.3,
                (1.0 - k) * 255.0,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let mc = (mc * gb) as u8;
        if mc > 1 {
            cursor(&mut px, mx, my, ms, mc);
        }
    }

    // 左侧标题（两段演示全程保留）
    let a1 = (clamp01(t / 0.10) * (1.0 - clamp01((t - 0.93) / 0.07)) * 255.0) as u8;
    let a2 = (a1 as u16 * 80 / 100) as u8;
    font.draw_bold(
        &mut px,
        150.0,
        430.0,
        88.0,
        "点一下",
        color(FGC.0, FGC.1, FGC.2, a1),
    );
    font.draw(
        &mut px,
        152.0,
        512.0,
        42.0,
        "完整面板即刻展开",
        color(DIMC.0, DIMC.1, DIMC.2, a2),
    );

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 5：镜头逐卡推进（每张卡有明确停留段）============
/// (时间点, 缩放, 离屏中心 y)；离屏 PS=2.2，各卡中心：229/524/801/1096/1408/1712
const CAM_KF: [(f32, f32, f32); 16] = [
    (0.00, 0.80, 940.0),
    (0.10, 0.80, 940.0),
    (0.22, 2.60, 229.0),
    (0.30, 2.60, 229.0),
    (0.36, 2.60, 524.0),
    (0.43, 2.60, 524.0),
    (0.49, 2.60, 801.0),
    (0.55, 2.60, 801.0),
    (0.63, 2.60, 1096.0),
    (0.70, 2.60, 1096.0),
    (0.76, 2.60, 1408.0),
    (0.81, 2.60, 1408.0),
    (0.85, 2.60, 1712.0),
    (0.90, 2.60, 1712.0),
    (0.98, 0.80, 940.0),
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
    let fin = 0.06;
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
    let fs = 36.0;
    let tw = font.measure(fs, text);
    let bw = tw + 72.0;
    let bh = 76.0;
    let bx = (VW as f32 - bw) / 2.0;
    let by = 980.0;
    rfill(
        px,
        bx,
        by,
        bw,
        bh,
        bh / 2.0,
        (10, 13, 18, (205.0 * a) as u8),
    );
    center_text(
        px,
        font,
        by + 50.0,
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

    // 开场标题：推镜一开始即快速淡出，避免叠到面板标题行
    let a0 = (ease_out_cubic(clamp01(t / 0.12))
        * (1.0 - ease_in_out(clamp01((t - 0.08) / 0.10)))
        * 255.0) as u8;
    center_text(
        &mut px,
        font,
        132.0,
        60.0,
        "完整功能面板",
        mul_a(FGC, a0),
        true,
    );

    caption(&mut px, font, t, 0.20, 0.30, "CPU 使用率 · 温度双环");
    caption(&mut px, font, t, 0.36, 0.43, "内存实时占用");
    caption(&mut px, font, t, 0.49, 0.55, "显卡负载 · 温度");
    caption(&mut px, font, t, 0.63, 0.70, "网络速率 心电图");
    caption(&mut px, font, t, 0.76, 0.81, "硬盘读写速率");
    caption(&mut px, font, t, 0.85, 0.90, "存储空间用量");

    fade_edges(&mut px, frame, total);
    px
}

// ============ 场景 6：收尾品牌 ============
fn scene6(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(VW, VH).unwrap();
    wallpaper(&mut px);
    let total = fc(5);
    let t = frame as f32 / total as f32;

    let snap = mock_snap(frame);
    let draw = ease_out_cubic(clamp01(t / 0.32));
    let cpu_v = snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) as f32;
    let mem_v = snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) as f32;
    let cyy = 356.0;
    let r = 52.0;
    ring(
        &mut px,
        960.0 - 140.0,
        cyy,
        r,
        10.0,
        Some(cpu_v * draw),
        usage_col(cpu_v),
        255,
    );
    ring(
        &mut px,
        960.0,
        cyy,
        r,
        10.0,
        Some(mem_v * draw),
        usage_col(mem_v),
        255,
    );
    let (nr, nw) = net_io_sum(&snap);
    let cap = 125_000_000u64;
    ring_split(
        &mut px,
        960.0 + 140.0,
        cyy,
        r,
        10.0,
        nw as f32 / cap as f32 * 100.0 * draw,
        nr as f32 / cap as f32 * 100.0 * draw,
        255,
    );
    center_text(
        &mut px,
        font,
        cyy + 104.0,
        22.0,
        "CPU  内存  网络",
        mul_a(DIMC, (clamp01((t - 0.15) / 0.2) * 255.0) as u8),
        false,
    );

    let rise = |start: f32| -> (u8, f32) {
        let k = ease_out_cubic(clamp01((t - start) / 0.22));
        ((k * 255.0) as u8, (1.0 - k) * 22.0)
    };
    let (a1, r1) = rise(0.22);
    center_text(
        &mut px,
        font,
        650.0 - r1,
        104.0,
        "核桃胶囊",
        mul_a(BLUEC, a1),
        true,
    );
    let (a2, r2) = rise(0.38);
    center_text(
        &mut px,
        font,
        812.0 - r2,
        50.0,
        "飞牛应用中心 · 正式上架",
        mul_a(FGC, a2),
        true,
    );

    fade_edges(&mut px, frame, total);
    px
}

#[test]
fn export_h_video() {
    let font = Font::load().unwrap();
    let icons = Icons::load(56, FGC);
    let out_dir = std::path::Path::new("d:\\nas胶囊\\视频帧-横屏");
    std::fs::create_dir_all(out_dir).unwrap();

    let total_frames: u32 = (0..6).map(|i| fc(i)).sum();
    println!(
        "Total frames: {}, duration: {:.1}s",
        total_frames,
        total_frames as f32 / VFPS as f32
    );

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
