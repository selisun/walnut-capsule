use super::*;
use crate::data::{Cpu, Disk, Gpu, Memory, NetIf, Snapshot, Volume};

const W: u32 = 1080;
const H: u32 = 1920;
const FPS: u32 = 30;

// 6 场景时长（秒）
const SCENES: [f32; 6] = [3.0, 4.0, 4.0, 6.0, 9.0, 4.0];

fn frame_count(scene: usize) -> u32 {
    (SCENES[scene] * FPS as f32).round() as u32
}

fn dark_bg() -> Rgba {
    (18, 20, 24, 255)
}

fn white() -> Rgba {
    (255, 255, 255, 255)
}

fn accent() -> Rgba {
    ACCENT
}

// 格式化速率
fn format_rate(bytes_per_sec: u64) -> String {
    let rate = bytes_per_sec as f64;
    if rate >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1}GB/s", rate / (1024.0 * 1024.0 * 1024.0))
    } else if rate >= 1024.0 * 1024.0 {
        format!("{:.1}MB/s", rate / (1024.0 * 1024.0))
    } else if rate >= 1024.0 {
        format!("{:.1}KB/s", rate / 1024.0)
    } else {
        format!("{}B/s", rate as u64)
    }
}

fn mock_snap(frame: u32) -> Snapshot {
    let t = frame as f64 / 30.0;
    Snapshot {
        timestamp: (1700000000 + frame as i64 * 1000),
        hostname: Some("飞牛NAS".into()),
        cpu: Some(Cpu {
            usage: Some(35.0 + (t * 2.3).sin() * 15.0),
            temp: Some(48.0 + (t * 1.7).cos() * 8.0),
            load1: None,
            load5: None,
            load15: None,
        }),
        memory: Some(Memory {
            usage: Some(62.0 + (t * 1.1).sin() * 8.0),
            used: Some(20 * 1024 * 1024 * 1024),
            total: Some(32 * 1024 * 1024 * 1024),
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

fn render_scene1(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(W, H).unwrap();
    px.fill(color(dark_bg().0, dark_bg().1, dark_bg().2, dark_bg().3));

    let t = frame as f32 / frame_count(0) as f32;
    let alpha = if t < 0.1 {
        (t / 0.1 * 255.0) as u8
    } else if t > 0.9 {
        ((1.0 - (t - 0.9) / 0.1) * 255.0) as u8
    } else {
        255
    };

    let y1 = 700.0;
    let y2 = 850.0;
    font.draw_bold(
        &mut px,
        150.0,
        y1,
        62.0,
        "还在开浏览器",
        color(white().0, white().1, white().2, alpha),
    );
    font.draw_bold(
        &mut px,
        150.0,
        y2,
        62.0,
        "看 NAS 状态？",
        color(white().0, white().1, white().2, alpha),
    );

    px
}

fn render_scene2(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(W, H).unwrap();
    px.fill(color(dark_bg().0, dark_bg().1, dark_bg().2, dark_bg().3));

    let snap = mock_snap(frame);
    let icons = crate::icons::Icons::load(PILL_ICON as u32, white());

    // 镜头特写：从远处快速推进放大
    let t = (frame as f32 / frame_count(1) as f32).clamp(0.0, 1.0);
    let eased = ease_motion(t);
    let scale = 1.0 + eased * 2.5; // 从1.0快速推进到3.5倍特写
    let alpha = (eased * 255.0) as u8;

    let pill_w = PILL_W as f32 * scale;
    let pill_h = PILL_H as f32 * scale;
    let cx = W as f32 / 2.0;
    let cy = 800.0;
    let x = cx - pill_w / 2.0;
    let y = cy - pill_h / 2.0;

    // 绘制胶囊背景
    fill_round(&mut px, x, y, pill_w, pill_h, PILL_R * scale, BG);

    // 绘制6格数据
    let cell_w = pill_w / 3.0;
    let cell_h = pill_h / 2.0;
    let txt_size = 28.0 * scale / 3.0;

    // CPU
    let cpu_val = snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0);
    let cpu_text = format!("{}%", cpu_val.round() as i32);
    font.draw(
        &mut px,
        x + cell_w * 0.5 - 30.0 * scale / 3.0,
        y + cell_h * 0.5 + 10.0 * scale / 3.0,
        txt_size,
        "CPU",
        color(white().0, white().1, white().2, 180),
    );
    font.draw_bold(
        &mut px,
        x + cell_w * 0.5 - 25.0 * scale / 3.0,
        y + cell_h * 0.5 + 40.0 * scale / 3.0,
        txt_size * 1.2,
        &cpu_text,
        color(accent().0, accent().1, accent().2, alpha),
    );

    // 内存
    let mem_val = snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0);
    let mem_text = format!("{}%", mem_val.round() as i32);
    font.draw(
        &mut px,
        x + cell_w * 1.5 - 30.0 * scale / 3.0,
        y + cell_h * 0.5 + 10.0 * scale / 3.0,
        txt_size,
        "MEM",
        color(white().0, white().1, white().2, 180),
    );
    font.draw_bold(
        &mut px,
        x + cell_w * 1.5 - 25.0 * scale / 3.0,
        y + cell_h * 0.5 + 40.0 * scale / 3.0,
        txt_size * 1.2,
        &mem_text,
        color(accent().0, accent().1, accent().2, alpha),
    );

    // GPU
    let gpu_val = snap
        .gpus
        .as_ref()
        .and_then(|g| g.first())
        .and_then(|g| g.utilization)
        .unwrap_or(0);
    let gpu_text = format!("{}%", gpu_val);
    font.draw(
        &mut px,
        x + cell_w * 2.5 - 30.0 * scale / 3.0,
        y + cell_h * 0.5 + 10.0 * scale / 3.0,
        txt_size,
        "GPU",
        color(white().0, white().1, white().2, 180),
    );
    font.draw_bold(
        &mut px,
        x + cell_w * 2.5 - 25.0 * scale / 3.0,
        y + cell_h * 0.5 + 40.0 * scale / 3.0,
        txt_size * 1.2,
        &gpu_text,
        color(accent().0, accent().1, accent().2, alpha),
    );

    // 硬盘读写
    let (dr, dw) = disk_io_sum(&snap);
    let disk_text = format!("↑{}", format_rate(dw));
    font.draw(
        &mut px,
        x + cell_w * 0.5 - 35.0 * scale / 3.0,
        y + cell_h * 1.5 + 10.0 * scale / 3.0,
        txt_size,
        "DISK",
        color(white().0, white().1, white().2, 180),
    );
    font.draw_bold(
        &mut px,
        x + cell_w * 0.5 - 30.0 * scale / 3.0,
        y + cell_h * 1.5 + 40.0 * scale / 3.0,
        txt_size * 1.0,
        &disk_text,
        color(white().0, white().1, white().2, alpha),
    );

    // 网络
    let (nr, nw) = net_io_sum(&snap);
    let net_text = format!("↑{}", format_rate(nw));
    font.draw(
        &mut px,
        x + cell_w * 1.5 - 30.0 * scale / 3.0,
        y + cell_h * 1.5 + 10.0 * scale / 3.0,
        txt_size,
        "NET",
        color(white().0, white().1, white().2, 180),
    );
    font.draw_bold(
        &mut px,
        x + cell_w * 1.5 - 25.0 * scale / 3.0,
        y + cell_h * 1.5 + 40.0 * scale / 3.0,
        txt_size * 1.0,
        &net_text,
        color(white().0, white().1, white().2, alpha),
    );

    // 存储
    let vol = snap.volumes.as_ref().and_then(|v| v.first());
    let stor_pct = if let Some(v) = vol {
        if let (Some(used), Some(total)) = (v.used_bytes, v.total_bytes) {
            (used as f64 / total as f64 * 100.0) as i32
        } else {
            0
        }
    } else {
        0
    };
    let stor_text = format!("{}%", stor_pct);
    font.draw(
        &mut px,
        x + cell_w * 2.5 - 35.0 * scale / 3.0,
        y + cell_h * 1.5 + 10.0 * scale / 3.0,
        txt_size,
        "STOR",
        color(white().0, white().1, white().2, 180),
    );
    font.draw_bold(
        &mut px,
        x + cell_w * 2.5 - 25.0 * scale / 3.0,
        y + cell_h * 1.5 + 40.0 * scale / 3.0,
        txt_size * 1.2,
        &stor_text,
        color(accent().0, accent().1, accent().2, alpha),
    );

    // 标题和描述
    let text_alpha = (t * 255.0).min(255.0) as u8;
    font.draw_bold(
        &mut px,
        150.0,
        cy - 200.0,
        52.0,
        "悬浮胶囊",
        color(white().0, white().1, white().2, text_alpha),
    );
    font.draw(
        &mut px,
        150.0,
        cy + 280.0,
        38.0,
        "六项关键指标一目了然",
        color(
            white().0,
            white().1,
            white().2,
            (text_alpha as f32 * 0.8) as u8,
        ),
    );

    px
}

fn render_scene3(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(W, H).unwrap();
    px.fill(color(dark_bg().0, dark_bg().1, dark_bg().2, dark_bg().3));

    let snap = mock_snap(frame);
    let t = (frame as f32 / frame_count(2) as f32).clamp(0.0, 1.0);

    // 从胶囊形态变形为三环形态
    let morph = ease_motion(t);
    let scale = 3.0 + morph * 0.5;

    let cx = W as f32 / 2.0;
    let cy = 1700.0; // 任务栏位置

    // 绘制模拟任务栏背景
    let taskbar_w = W as f32;
    let taskbar_h = 80.0;
    let taskbar_y = H as f32 - taskbar_h;
    fill_round(
        &mut px,
        0.0,
        taskbar_y,
        taskbar_w,
        taskbar_h,
        0.0,
        (25, 27, 31, 255),
    );

    // 绘制三环
    let ring_r = 20.0 * scale;
    let ring_gap = 15.0 * scale;
    let total_w = ring_r * 6.0 + ring_gap * 2.0;
    let ring_x = cx - total_w / 2.0;
    let ring_y = cy;

    // CPU环
    let cpu_pct = (snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) / 100.0) as f32;
    draw_ring(
        &mut px,
        ring_x + ring_r,
        ring_y,
        ring_r,
        ring_r - 6.0 * scale,
        cpu_pct,
        accent(),
    );

    // 内存环
    let mem_pct = (snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) / 100.0) as f32;
    draw_ring(
        &mut px,
        ring_x + ring_r * 3.0 + ring_gap,
        ring_y,
        ring_r,
        ring_r - 6.0 * scale,
        mem_pct,
        accent(),
    );

    // GPU环
    let gpu_pct = snap
        .gpus
        .as_ref()
        .and_then(|g| g.first())
        .and_then(|g| g.utilization)
        .unwrap_or(0) as f32
        / 100.0;
    draw_ring(
        &mut px,
        ring_x + ring_r * 5.0 + ring_gap * 2.0,
        ring_y,
        ring_r,
        ring_r - 6.0 * scale,
        gpu_pct,
        accent(),
    );

    // 标题和描述
    let text_alpha = (t * 255.0) as u8;
    font.draw_bold(
        &mut px,
        150.0,
        600.0,
        52.0,
        "融入任务栏",
        color(white().0, white().1, white().2, text_alpha),
    );
    font.draw(
        &mut px,
        150.0,
        750.0,
        38.0,
        "三环设计不遮挡内容",
        color(
            white().0,
            white().1,
            white().2,
            (text_alpha as f32 * 0.8) as u8,
        ),
    );

    px
}

// 绘制圆环进度
fn draw_ring(
    px: &mut Pixmap,
    cx: f32,
    cy: f32,
    outer_r: f32,
    inner_r: f32,
    progress: f32,
    c: Rgba,
) {
    use tiny_skia::*;

    // 绘制背景圆环
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, outer_r);
    if let Some(outer_path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(40, 42, 46, 255);
        paint.anti_alias = true;
        px.fill_path(
            &outer_path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // 挖空内圆
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, inner_r);
    if let Some(inner_path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(18, 20, 24, 255);
        paint.blend_mode = BlendMode::Source;
        paint.anti_alias = true;
        px.fill_path(
            &inner_path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // 绘制进度弧（使用stroke绘制）
    if progress > 0.0 {
        let stroke_width = outer_r - inner_r;
        let stroke_r = (outer_r + inner_r) / 2.0;
        let angle_deg = progress * 360.0;

        // 简化：绘制多段直线来模拟圆弧
        let segments = (angle_deg / 5.0).ceil() as i32;
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.0, c.1, c.2, c.3);
        paint.anti_alias = true;

        for i in 0..segments {
            let a1 = (i as f32 / segments as f32) * angle_deg;
            let a2 = ((i + 1) as f32 / segments as f32) * angle_deg;

            let x1 = cx + (a1 - 90.0).to_radians().cos() * stroke_r;
            let y1 = cy + (a1 - 90.0).to_radians().sin() * stroke_r;
            let x2 = cx + (a2 - 90.0).to_radians().cos() * stroke_r;
            let y2 = cy + (a2 - 90.0).to_radians().sin() * stroke_r;

            let mut pb = PathBuilder::new();
            pb.move_to(x1, y1);
            pb.line_to(x2, y2);

            if let Some(path) = pb.finish() {
                let stroke = Stroke {
                    width: stroke_width,
                    line_cap: LineCap::Round,
                    ..Default::default()
                };
                px.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
    }
}

fn render_scene4(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(W, H).unwrap();
    px.fill(color(dark_bg().0, dark_bg().1, dark_bg().2, dark_bg().3));

    let total = frame_count(3);
    let progress = (frame as f32 / total as f32).clamp(0.0, 1.0);

    // 镜头推进：在鼠标交互时逐渐放大聚焦
    let camera_zoom = if progress < 0.3 {
        1.0 + progress * 0.5 // 鼠标移入时推进: 1.0 -> 1.15
    } else {
        1.15 + (progress - 0.3) * 0.3 // 点击和展开时继续推进: 1.15 -> 1.36
    };

    // 分段动画：0-0.3 鼠标移入，0.3-0.4 点击，0.4-1.0 面板展开
    let _snap = mock_snap(frame);

    // 胶囊初始位置（屏幕上方），随镜头推进放大
    let pill_scale = 2.5 * camera_zoom;
    let pill_w = PILL_W as f32 * pill_scale;
    let pill_h = PILL_H as f32 * pill_scale;
    let pill_cx = W as f32 / 2.0;
    let pill_cy = 400.0;
    let pill_x = pill_cx - pill_w / 2.0;
    let pill_y = pill_cy - pill_h / 2.0;

    // 绘制胶囊
    fill_round(
        &mut px,
        pill_x,
        pill_y,
        pill_w,
        pill_h,
        PILL_R * pill_scale,
        BG,
    );

    // 鼠标动画
    let mouse_x;
    let mouse_y;
    let mouse_scale;

    if progress < 0.3 {
        // 鼠标从右下移向胶囊中心
        let t = progress / 0.3;
        let ease_t = ease_motion(t);
        mouse_x = W as f32 * 0.8 - ease_t * (W as f32 * 0.8 - pill_cx);
        mouse_y = H as f32 * 0.8 - ease_t * (H as f32 * 0.8 - pill_cy);
        mouse_scale = 1.0;
    } else if progress < 0.4 {
        // 点击动画（鼠标缩小）
        let t = (progress - 0.3) / 0.1;
        mouse_x = pill_cx;
        mouse_y = pill_cy;
        mouse_scale = 1.0 - t * 0.2;
    } else {
        // 点击完成，鼠标淡出
        let t = (progress - 0.4) / 0.1;
        mouse_x = pill_cx;
        mouse_y = pill_cy;
        mouse_scale = 0.8 - t * 0.8;
    }

    // 绘制鼠标指针（简化版）
    if mouse_scale > 0.0 {
        let alpha = (mouse_scale * 255.0) as u8;
        draw_cursor(&mut px, mouse_x, mouse_y, mouse_scale * 50.0, alpha);
    }

    // 面板展开动画（同时受镜头推进影响）
    if progress > 0.4 {
        let expand_t = ((progress - 0.4) / 0.6).clamp(0.0, 1.0);
        let eased = ease_motion(expand_t);

        // 面板从胶囊位置向下展开，随镜头放大
        let panel_scale = 1.5 * camera_zoom;
        let panel_w = PANEL_W as f32 * panel_scale;
        let panel_h = 1200.0 * camera_zoom * eased; // 逐渐展开高度
        let panel_x = pill_cx - panel_w / 2.0;
        let panel_y = pill_cy + pill_h / 2.0;

        fill_round(&mut px, panel_x, panel_y, panel_w, panel_h, 16.0, BG);

        // 面板内容逐渐显示
        if eased > 0.3 {
            let content_alpha = ((eased - 0.3) / 0.7 * 255.0) as u8;
            font.draw_bold(
                &mut px,
                panel_x + 40.0 * camera_zoom,
                panel_y + 60.0 * camera_zoom,
                32.0 * camera_zoom,
                "飞牛NAS",
                color(white().0, white().1, white().2, content_alpha),
            );
        }
    }

    // 标题
    let text_alpha = if progress < 0.5 {
        255
    } else {
        ((1.0 - (progress - 0.5) / 0.5) * 255.0) as u8
    };
    font.draw_bold(
        &mut px,
        150.0,
        250.0,
        48.0,
        "点一下",
        color(white().0, white().1, white().2, text_alpha),
    );
    font.draw(
        &mut px,
        150.0,
        320.0,
        36.0,
        "完整面板即刻展开",
        color(
            white().0,
            white().1,
            white().2,
            (text_alpha as f32 * 0.8) as u8,
        ),
    );

    px
}

// 绘制鼠标指针
fn draw_cursor(px: &mut Pixmap, x: f32, y: f32, size: f32, alpha: u8) {
    use tiny_skia::*;

    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x, y + size);
    pb.line_to(x + size * 0.35, y + size * 0.7);
    pb.line_to(x + size * 0.5, y + size * 0.5);
    pb.close();

    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(255, 255, 255, alpha);
        paint.anti_alias = true;
        px.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        // 黑色边框
        paint.set_color_rgba8(0, 0, 0, alpha);
        px.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: 2.0,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

fn render_scene5(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(W, H).unwrap();
    px.fill(color(dark_bg().0, dark_bg().1, dark_bg().2, dark_bg().3));

    let snap = mock_snap(frame);
    let total = frame_count(4);
    let progress = (frame as f32 / total as f32).clamp(0.0, 1.0);

    // 强化镜头推进：从1.0快速推进到2.0实现特写效果
    let eased_progress = ease_motion(progress);
    let zoom = 1.0 + eased_progress * 1.0; // 1.0 -> 2.0
    let base_scale = 1.8 * zoom;

    let cx = W as f32 / 2.0;
    let cy = H as f32 / 2.0;

    // 面板尺寸和位置
    let panel_w = PANEL_W as f32 * base_scale;
    let panel_h = 1400.0 * base_scale;
    let panel_x = cx - panel_w / 2.0;
    let panel_y = cy - panel_h / 2.0;

    // 绘制面板背景
    fill_round(&mut px, panel_x, panel_y, panel_w, panel_h, 20.0 * zoom, BG);

    // 标题
    let title_y = panel_y + 50.0 * zoom;
    font.draw_bold(
        &mut px,
        panel_x + 40.0 * zoom,
        title_y,
        36.0 * zoom,
        "飞牛NAS",
        color(white().0, white().1, white().2, 255),
    );

    // CPU、内存、GPU 环形仪表区域（分段展示）
    let rings_y = title_y + 100.0 * zoom;
    let ring_size = 80.0 * zoom;
    let ring_gap = 40.0 * zoom;

    if progress > 0.1 {
        let alpha = (((progress - 0.1) / 0.1).min(1.0) * 255.0) as u8;

        // CPU环
        let cpu_x = panel_x + panel_w * 0.2;
        let cpu_pct = (snap.cpu.as_ref().and_then(|c| c.usage).unwrap_or(0.0) / 100.0) as f32;
        draw_ring(
            &mut px,
            cpu_x,
            rings_y,
            ring_size,
            ring_size - 12.0 * zoom,
            cpu_pct,
            accent(),
        );
        font.draw(
            &mut px,
            cpu_x - 25.0 * zoom,
            rings_y + ring_size + 30.0 * zoom,
            24.0 * zoom,
            "CPU",
            color(white().0, white().1, white().2, alpha),
        );
        font.draw_bold(
            &mut px,
            cpu_x - 30.0 * zoom,
            rings_y + ring_size + 65.0 * zoom,
            28.0 * zoom,
            &format!("{}%", (cpu_pct * 100.0) as i32),
            color(accent().0, accent().1, accent().2, alpha),
        );
    }

    if progress > 0.25 {
        let alpha = (((progress - 0.25) / 0.1).min(1.0) * 255.0) as u8;

        // 内存环
        let mem_x = panel_x + panel_w * 0.5;
        let mem_pct = (snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) / 100.0) as f32;
        draw_ring(
            &mut px,
            mem_x,
            rings_y,
            ring_size,
            ring_size - 12.0 * zoom,
            mem_pct,
            accent(),
        );
        font.draw(
            &mut px,
            mem_x - 30.0 * zoom,
            rings_y + ring_size + 30.0 * zoom,
            24.0 * zoom,
            "MEM",
            color(white().0, white().1, white().2, alpha),
        );
        font.draw_bold(
            &mut px,
            mem_x - 30.0 * zoom,
            rings_y + ring_size + 65.0 * zoom,
            28.0 * zoom,
            &format!("{}%", (mem_pct * 100.0) as i32),
            color(accent().0, accent().1, accent().2, alpha),
        );
    }

    if progress > 0.4 {
        let alpha = (((progress - 0.4) / 0.1).min(1.0) * 255.0) as u8;

        // GPU环
        let gpu_x = panel_x + panel_w * 0.8;
        let gpu_pct = snap
            .gpus
            .as_ref()
            .and_then(|g| g.first())
            .and_then(|g| g.utilization)
            .unwrap_or(0) as f32
            / 100.0;
        draw_ring(
            &mut px,
            gpu_x,
            rings_y,
            ring_size,
            ring_size - 12.0 * zoom,
            gpu_pct,
            accent(),
        );
        font.draw(
            &mut px,
            gpu_x - 28.0 * zoom,
            rings_y + ring_size + 30.0 * zoom,
            24.0 * zoom,
            "GPU",
            color(white().0, white().1, white().2, alpha),
        );
        font.draw_bold(
            &mut px,
            gpu_x - 30.0 * zoom,
            rings_y + ring_size + 65.0 * zoom,
            28.0 * zoom,
            &format!("{}%", (gpu_pct * 100.0) as i32),
            color(accent().0, accent().1, accent().2, alpha),
        );
    }

    // 网络曲线区域
    if progress > 0.55 {
        let alpha = (((progress - 0.55) / 0.1).min(1.0) * 255.0) as u8;
        let net_y = rings_y + ring_size * 2.0 + 100.0 * zoom;
        let net_h = 150.0 * zoom;
        let net_w = panel_w - 80.0 * zoom;

        font.draw(
            &mut px,
            panel_x + 40.0 * zoom,
            net_y - 30.0 * zoom,
            24.0 * zoom,
            "网络流量",
            color(white().0, white().1, white().2, alpha),
        );

        // 绘制简化的网络曲线
        let (nr, nw) = net_io_sum(&snap);
        font.draw(
            &mut px,
            panel_x + 40.0 * zoom,
            net_y + net_h + 35.0 * zoom,
            22.0 * zoom,
            &format!("↑ {} ↓ {}", format_rate(nw), format_rate(nr)),
            color(accent().0, accent().1, accent().2, alpha),
        );
    }

    // 硬盘和存储信息
    if progress > 0.7 {
        let alpha = (((progress - 0.7) / 0.1).min(1.0) * 255.0) as u8;
        let disk_y = panel_y + panel_h - 200.0 * zoom;

        let (dr, dw) = disk_io_sum(&snap);
        font.draw(
            &mut px,
            panel_x + 40.0 * zoom,
            disk_y,
            24.0 * zoom,
            "硬盘读写",
            color(white().0, white().1, white().2, alpha),
        );
        font.draw(
            &mut px,
            panel_x + 40.0 * zoom,
            disk_y + 40.0 * zoom,
            22.0 * zoom,
            &format!("读 {} 写 {}", format_rate(dr), format_rate(dw)),
            color(white().0, white().1, white().2, alpha),
        );

        if let Some(vol) = snap.volumes.as_ref().and_then(|v| v.first()) {
            if let (Some(used), Some(total)) = (vol.used_bytes, vol.total_bytes) {
                let stor_pct = (used as f64 / total as f64 * 100.0) as i32;
                font.draw(
                    &mut px,
                    panel_x + 40.0 * zoom,
                    disk_y + 90.0 * zoom,
                    24.0 * zoom,
                    "存储空间",
                    color(white().0, white().1, white().2, alpha),
                );
                font.draw(
                    &mut px,
                    panel_x + 40.0 * zoom,
                    disk_y + 130.0 * zoom,
                    22.0 * zoom,
                    &format!("已用 {}%", stor_pct),
                    color(accent().0, accent().1, accent().2, alpha),
                );
            }
        }
    }

    // 场景描述文字（随镜头推进逐渐淡出）
    let desc_alpha = if progress < 0.2 {
        255
    } else {
        ((1.0 - (progress - 0.2) / 0.2).max(0.0) * 255.0) as u8
    };
    if desc_alpha > 0 {
        // 文字位置随镜头zoom轻微调整
        let text_offset = (1.0 - zoom / 2.0) * 50.0;
        font.draw_bold(
            &mut px,
            150.0 + text_offset,
            200.0,
            42.0 * (2.0 - zoom / 2.0),
            "完整功能面板",
            color(white().0, white().1, white().2, desc_alpha),
        );
        font.draw(
            &mut px,
            150.0 + text_offset,
            270.0,
            32.0 * (2.0 - zoom / 2.0),
            "所有指标一屏掌握",
            color(
                white().0,
                white().1,
                white().2,
                (desc_alpha as f32 * 0.8) as u8,
            ),
        );
    }

    px
}

fn render_scene6(frame: u32, font: &Font) -> Pixmap {
    let mut px = Pixmap::new(W, H).unwrap();
    px.fill(color(dark_bg().0, dark_bg().1, dark_bg().2, dark_bg().3));

    let t = frame as f32 / frame_count(5) as f32;
    let alpha = if t < 0.15 {
        (t / 0.15 * 255.0) as u8
    } else {
        255
    };

    let cy = 900.0;
    font.draw_bold(
        &mut px,
        180.0,
        cy - 100.0,
        72.0,
        "核桃胶囊",
        color(accent().0, accent().1, accent().2, alpha),
    );
    font.draw_bold(
        &mut px,
        180.0,
        cy + 50.0,
        42.0,
        "飞牛应用中心",
        color(white().0, white().1, white().2, alpha),
    );
    font.draw_bold(
        &mut px,
        180.0,
        cy + 140.0,
        42.0,
        "内测上线",
        color(white().0, white().1, white().2, alpha),
    );
    font.draw(
        &mut px,
        180.0,
        cy + 240.0,
        36.0,
        "全功能免费",
        color(white().0, white().1, white().2, (alpha as f32 * 0.8) as u8),
    );

    px
}

#[test]
fn export_full_video() {
    let font = Font::load().unwrap();
    let out_dir = std::path::Path::new("d:\\nas胶囊\\视频帧-全功能");
    std::fs::create_dir_all(out_dir).unwrap();

    let total_frames: u32 = (0..6).map(|i| frame_count(i)).sum();
    println!(
        "Total frames: {}, duration: {:.1}s",
        total_frames,
        total_frames as f32 / FPS as f32
    );

    let mut global_frame = 0u32;
    for scene in 0..6 {
        let count = frame_count(scene);
        println!(
            "Scene {}: {} frames ({:.1}s)",
            scene + 1,
            count,
            SCENES[scene]
        );

        for local_frame in 0..count {
            let px = match scene {
                0 => render_scene1(local_frame, &font),
                1 => render_scene2(local_frame, &font),
                2 => render_scene3(local_frame, &font),
                3 => render_scene4(local_frame, &font),
                4 => render_scene5(local_frame, &font),
                5 => render_scene6(local_frame, &font),
                _ => unreachable!(),
            };

            let path = out_dir.join(format!("frame_{:05}.png", global_frame));
            px.save_png(&path).unwrap();
            global_frame += 1;

            if global_frame % 90 == 0 {
                println!("  Rendered {}/{}", global_frame, total_frames);
            }
        }
    }

    println!("All frames exported to: {}", out_dir.display());
    println!("\nTo create video, run on NAS:");
    println!("ffmpeg -framerate 30 -i frame_%05d.png -c:v libx264 -preset medium -crf 23 -pix_fmt yuv420p output.mp4");
}
