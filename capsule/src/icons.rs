//! 内嵌 SVG 线性图标（Lucide 风格，ISC 许可），启动时一次性渲染成位图缓存。

use tiny_skia::Pixmap;

/// 图标统一外壳：24×24 线性描边，`{C}` 为颜色、`{P}` 为路径内容。
const TPL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="{C}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">{P}</svg>"#;

/// Lucide「settings」齿轮：展开态顶栏的设置按钮。
const SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="{C}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>"#;

/// Lucide「cpu」：CPU 使用率。
const CPU: &str = r#"<rect width="16" height="16" x="4" y="4" rx="2"/><rect width="6" height="6" x="9" y="9" rx="1"/><path d="M15 2v2M15 20v2M2 15h2M2 9h2M20 15h2M20 9h2M9 2v2M9 20v2"/>"#;

/// Lucide「memory-stick」：内存使用率。
const RAM: &str = r#"<path d="M6 19v-3M10 19v-3M14 19v-3M18 19v-3M8 11V9M16 11V9M12 11V9"/><path d="M2 15h20v3a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z"/><path d="M4 15V5a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v10"/>"#;

/// Lucide「hard-drive」：硬盘读写速率。
const DISK: &str = r#"<line x1="22" x2="2" y1="12" y2="12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><line x1="6" x2="6.01" y1="16" y2="16"/><line x1="10" x2="10.01" y1="16" y2="16"/>"#;

/// Lucide「arrow-up-down」：网络上下行速率。
const NET: &str = r#"<path d="m21 16-4 4-4-4"/><path d="M17 20V4"/><path d="m3 8 4-4 4 4"/><path d="M7 4v16"/>"#;

/// 收起态药丸胶囊用的四个图标（按统一颜色一次性渲染）。
pub struct Icons {
    pub cpu: Option<Pixmap>,
    pub ram: Option<Pixmap>,
    pub disk: Option<Pixmap>,
    pub net: Option<Pixmap>,
}

impl Icons {
    pub fn load(size: u32, color: (u8, u8, u8, u8)) -> Self {
        Self {
            cpu: icon(CPU, size, color),
            ram: icon(RAM, size, color),
            disk: icon(DISK, size, color),
            net: icon(NET, size, color),
        }
    }
}

/// 渲染单个自定义图标（透明底，尺寸为像素边长）。
pub fn gear(size: u32, color: (u8, u8, u8, u8)) -> Option<Pixmap> {
    render(with_color(SETTINGS, color), size)
}

fn icon(paths: &str, size: u32, color: (u8, u8, u8, u8)) -> Option<Pixmap> {
    render(with_color(&TPL.replace("{P}", paths), color), size)
}

/// 把 `{C}` 占位替换成 `#RRGGBB`。
fn with_color(svg: &str, color: (u8, u8, u8, u8)) -> String {
    let hex = format!("#{:.2X}{:.2X}{:.2X}", color.0, color.1, color.2);
    svg.replace("{C}", &hex)
}

fn render(svg: String, size: u32) -> Option<Pixmap> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(&svg, &opt).ok()?;
    let mut pm = resvg::tiny_skia::Pixmap::new(size, size)?;
    pm.fill(resvg::tiny_skia::Color::TRANSPARENT);
    let s = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    let mut out = Pixmap::new(size, size)?;
    out.data_mut().copy_from_slice(pm.data());
    Some(out)
}
