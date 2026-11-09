//! 生成小屏主题预览图（192×288）：先用真实 GDI 字体渲染 320×480，再双线性缩放。
//!
//! 用法：cargo run -p screen-render --example make_preview -- <主题.json> <输出.png>

use screen_core::{ComponentKind, Orientation, ScreenProfile};
use screen_render::{create_text_renderer, render_legacy, FontSpec, LegacyData};

const SRC_W: u32 = 320;
const SRC_H: u32 = 480;
const OUT_W: u32 = 192;
const OUT_H: u32 = 288;

/// 第三个参数可选：逗号分隔的显示项（如 `cpu,memory,gpu`），用于预览「项目减少」的版式。
fn parse_kind(s: &str) -> Option<ComponentKind> {
    match s.trim().to_ascii_lowercase().as_str() {
        "cpu" => Some(ComponentKind::Cpu),
        "mem" | "memory" => Some(ComponentKind::Memory),
        "gpu" => Some(ComponentKind::Gpu),
        "net" | "network" => Some(ComponentKind::Network),
        "disk" => Some(ComponentKind::Disk),
        "temp" => Some(ComponentKind::Temp),
        "clock" => Some(ComponentKind::Clock),
        "storage" => Some(ComponentKind::Storage),
        _ => None,
    }
}

fn sample(src: &[u8], x: f32, y: f32) -> [f32; 4] {
    let x = x.clamp(0.0, (SRC_W - 1) as f32);
    let y = y.clamp(0.0, (SRC_H - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min((SRC_W - 1) as usize);
    let y1 = (y0 + 1).min((SRC_H - 1) as usize);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let at = |px: usize, py: usize, c: usize| src[(py * SRC_W as usize + px) * 4 + c] as f32;
    let mut out = [0.0f32; 4];
    for (c, slot) in out.iter_mut().enumerate() {
        let a = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let b = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *slot = a * (1.0 - fy) + b * fy;
    }
    out
}

fn preview_data() -> LegacyData {
    LegacyData {
        host: Some("NEW-nas".into()),
        addr: Some("192.168.66.126".into()),
        online: true,
        cpu: Some(3.0),
        cpu_temp: Some(41.0),
        mem: Some(53.0),
        mem_used: Some(8 * 1024 * 1024 * 1024),
        mem_total: Some(16 * 1024 * 1024 * 1024),
        gpu: Some(0.0),
        gpu_temp: Some(42.0),
        // NAS 网络：收 1.7 MB/s，发 127 KB/s
        up: Some(1_782_579),
        down: Some(130_048),
        read: Some(0),
        write: Some(0),
        pc_cpu: Some(30.0),
        pc_mem: Some(73.0),
        pc_gpu: Some(5.0),
        pc_gpu_temp: Some(34.0),
        pc_rx: Some(40_346),
        pc_tx: Some(6_554),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let theme_path = args.next().unwrap_or_else(|| "theme.json".into());
    let out_path = args.next().unwrap_or_else(|| "preview.png".into());

    let json = std::fs::read_to_string(&theme_path).expect("读取主题文件失败");
    let theme = screen_theme::load_str(&json).expect("解析主题失败");

    let profile = ScreenProfile::new("p", SRC_W, SRC_H, Orientation::Portrait, true)
        .expect("固定 320×480 竖屏 Profile 必须有效");
    let data = preview_data();
    let mut text = create_text_renderer(&FontSpec {
        family: theme.typography.family.clone(),
        // 与产品路径一致：按 32px 取字形轮廓再缩放
        size_px: 32.0,
        weight: theme
            .typography
            .weights
            .get("value")
            .copied()
            .unwrap_or(400),
    })
    .expect("GDI 文字渲染器不可用");

    let visible = match args.next() {
        Some(list) => list.split(',').filter_map(parse_kind).collect::<Vec<_>>(),
        None => theme.components.visible.clone(),
    };
    let frame =
        render_legacy(&profile, &theme, &data, &visible, Some(text.as_mut())).expect("渲染失败");

    let src = frame.pixels();
    let mut pixmap = tiny_skia::Pixmap::new(OUT_W, OUT_H).expect("pixmap");
    let dst = pixmap.data_mut();
    let sx = SRC_W as f32 / OUT_W as f32;
    let sy = SRC_H as f32 / OUT_H as f32;
    for y in 0..OUT_H as usize {
        for x in 0..OUT_W as usize {
            // 像素中心对齐采样，避免缩略图整体偏半像素
            let fx = (x as f32 + 0.5) * sx - 0.5;
            let fy = (y as f32 + 0.5) * sy - 0.5;
            let px = sample(src, fx, fy);
            let i = (y * OUT_W as usize + x) * 4;
            for (c, v) in px.iter().enumerate() {
                dst[i + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    pixmap.save_png(&out_path).expect("保存 PNG 失败");
    println!("OK: {out_path} ({SRC_W}x{SRC_H} -> {OUT_W}x{OUT_H})");
}
