//! CapsuleView：悬浮胶囊。
//! 收起：两行实时数据（行1 CPU/GPU/内存，行2 硬盘/网络）；
//! 展开：顶栏仅显 NAS 主机名，下方为更宽的单列面板（顶栏右对齐）——
//! CPU/内存/GPU 环形仪表、存储状态列表卡、硬盘读写列表卡、网卡心电图卡。
//! 展开/收起带高度缓动 + 面板淡入动效。

use super::View;
use crate::data::{ConnStatus, Disk, Frame, Snapshot, WM_APP_PANEL};
use crate::fmt;
use crate::font::Font;
use std::collections::{HashMap, VecDeque};
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tiny_skia::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// 未固定时的药丸胶囊宽度（两行 × 三列）与高度（与窗口标题栏相当，约 32）。
const PILL_W: u32 = 276;
const PILL_H: u32 = 32;
/// 药丸底圆角（随高度收窄而收敛，避免变成整颗胶囊形）
const PILL_R: f32 = 10.0;
const CAPSULE_H: u32 = 52;
/// 展开面板宽于胶囊，展开时顶栏拉到与卡片同宽（右缘固定，向左生长）
pub(crate) const PANEL_W: u32 = 320;
const PANEL_PAD: f32 = 14.0;
const HEADER_H: f32 = 24.0;
const GAP: u32 = 8;
/// 展开态顶栏右侧设置齿轮的边长（像素）
const GEAR_SIZE: f32 = 16.0;

/// 收起态三圆环布局：半径、描边、中心 y、三个圆心 x。
/// 环高与任务栏等高（48），三环加大后数值与标注都收进环内。
const RING_R: f32 = 21.0;
const RING_STROKE: f32 = 5.0;
const RING_CY: f32 = 24.0;
const RING_CX: [f32; 3] = [24.0, 95.0, 166.0];
const RING_LABEL_FS: f32 = 6.5;
const RING_VALUE_FS: f32 = 11.5;
const RING_NET_FS: f32 = 7.0;

/// 收起态药丸胶囊（未固定）布局：图标边长、两行中线、三列左缘。
const PILL_ICON: f32 = 11.0;
const PILL_ROW_CY: [f32; 2] = [8.0, 24.0];
const PILL_COL_X: [f32; 3] = [14.0, 96.0, 180.0];

/// 固定到任务栏且任务栏为深色时的文字配色（浅色任务栏复用白色主题配色）
const PIN_FG: (u8, u8, u8, u8) = (240, 243, 247, 255);
const PIN_FG_DIM: (u8, u8, u8, u8) = (168, 176, 188, 255);

const CARD_W: f32 = PANEL_W as f32 - PANEL_PAD * 2.0;
const CARD_GAP: f32 = 10.0;
const CARD_PAD: f32 = 14.0;

// 展开/收起动效时长（毫秒）。对齐 Windows 11 弹出面板的节奏：进场约 300ms，退场略快。
const EXPAND_MS: f32 = 300.0;
const COLLAPSE_MS: f32 = 220.0;

// 卡片高度
const GAUGE_H: f32 = 92.0; // 环形仪表卡
const DUAL_GAUGE_H: f32 = 116.0; // 内存单大环卡
const CPU_CARD_H: f32 = 132.0; // CPU 双环仪表卡（更高，环更大）
const BAR_BASE: f32 = 40.0; // 横条卡标题区+底部留白
const BAR_ROW_H: f32 = 48.0; // 每块盘：信息行 + 用量横条
const CHART_H: f32 = 100.0; // 心电图卡

// 卷卡（卷名 + 已用/总量 + 用量进度条）
const VOL_ROW_H: f32 = 42.0;

// 网卡心电图：保留样本数
const NET_HIST: usize = 60;

const FONT_SIZE: f32 = 14.0;
const FONT_SIZE_SMALL: f32 = 12.5;
const FONT_SIZE_TITLE: f32 = 16.5; // 卡片标题字号（CPU/内存/网络/硬盘读写/存储状态）
const FONT_SIZE_HEADER: f32 = 17.0; // 展开态顶栏 NAS 名称字号（加粗加大）

fn color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

/// RGBA 字节颜色（本模块调色板常量统一使用该元组类型）。
type Rgba = (u8, u8, u8, u8);

// 白色主题
const BG: (u8, u8, u8, u8) = (246, 248, 251, 242);
const CARD_BG: (u8, u8, u8, u8) = (255, 255, 255, 250);
const CARD_LINE: (u8, u8, u8, u8) = (224, 228, 235, 255);
const FG: (u8, u8, u8, u8) = (30, 34, 42, 255);
const FG_DIM: (u8, u8, u8, u8) = (128, 135, 148, 255);
const ACCENT: (u8, u8, u8, u8) = (30, 144, 255, 255);
/// 硬盘读取标记 R 的正蓝色（比 ACCENT 更饱和，小字号下辨识度高）。
const READ_BLUE: (u8, u8, u8, u8) = (0, 102, 255, 255);
const WARN: (u8, u8, u8, u8) = (232, 120, 30, 255);
const LEVEL_GREEN: (u8, u8, u8, u8) = (38, 168, 92, 255);
const LEVEL_RED: (u8, u8, u8, u8) = (224, 72, 60, 255);
/// 网络下行标记 DL 的紫色（上行 UL 复用 TX_LINE 橙）。
const DL_PURPLE: (u8, u8, u8, u8) = (147, 51, 234, 255);
const TRACK: (u8, u8, u8, u8) = (226, 230, 237, 255);
const TX_LINE: (u8, u8, u8, u8) = (240, 130, 60, 255);

/// 磨砂模式下背景更透，露出系统模糊层。
fn frost_bg(frost: bool) -> (u8, u8, u8, u8) {
    if frost { (250, 251, 253, 95) } else { BG }
}
fn frost_card_bg(frost: bool) -> (u8, u8, u8, u8) {
    if frost { (255, 255, 255, 115) } else { CARD_BG }
}

/// 派发给后台线程的面板构建请求：只携带 Arc 快照指针与少量小字段，
/// UI 线程发送时不阻塞、不复制快照。
struct BuildReq {
    seq: u64,
    conn: ConnStatus,
    ph: u32,
    frost: bool,
    has_data: bool,
    /// 标题行放面板底部（固定到任务栏且任务栏在下方，标题行贴任务栏一侧）
    header_bottom: bool,
    snap: Arc<Snapshot>,
}

/// 后台线程任务：展开时构建整层面板；收起时只推进心电图历史（展开即有满图）。
enum PanelJob {
    Build(BuildReq),
    Advance { seq: u64, has_data: bool, snap: Arc<Snapshot> },
    /// UI 归还的旧面板像素缓冲，供下一轮构建复用（稳态零大内存分配）
    Recycle(Pixmap),
}

/// 后台线程构建完成的展开面板整层（大底色 + 卡片底/描边 + 标题状态 + 全部正文）。
struct BuiltPanel {
    seq: u64,
    conn: ConnStatus,
    ph: u32,
    frost: bool,
    header_bottom: bool,
    pm: Pixmap,
}

pub struct CapsuleView {
    /// 展开目标态
    open: bool,
    /// 当前展开动画进度 0（收起）..1（展开）
    anim: f32,
    anim_from: f32,
    anim_start: Instant,
    /// 每行（像素 y）的附加不透明度 0..255，展开时面板区域淡入
    row_fade: Vec<u8>,
    /// 面板构建请求发送端：随视图 drop 关闭，后台线程 recv 失败即退出
    req_tx: mpsc::Sender<PanelJob>,
    /// 面板构建结果接收端（仅 UI 线程在渲染时 drain）
    panel_rx: mpsc::Receiver<BuiltPanel>,
    /// 最新已构建面板（收起后释放约 1MB 像素缓冲）
    current: Option<BuiltPanel>,
    /// 顶栏圆角条缓存 (frost, 320×52)：宽度动画期间整层 blit，
    /// 超出可视宽的部分不会被上屏，省掉每帧抗锯齿 fill_path
    bar: Option<(bool, Pixmap)>,
    /// 已派发请求的键 (seq, conn, frost, ph)：动画帧键不变，避免重复派发
    sent_key: Option<(u64, ConnStatus, bool, u32, bool)>,
    /// 收起态已推进心电图历史的数据版本（与构建线程 last_seq 各自去重）
    last_adv_seq: u64,
    /// 展开态顶栏右侧的设置齿轮图标（渲染失败为 None，自动隐藏按钮）
    gear: Option<Pixmap>,
    /// 未固定时药丸胶囊用的四个线性图标
    icons: crate::icons::Icons,
    /// 标题行设置齿轮在窗口内的位置（左上角）；供命中测试复用
    gear_pos: (f32, f32),
    /// 任务栏高度（用于垂直居中对齐，0 表示未知）
    taskbar_h: i32,
    /// 鼠标悬停态：圆环描边加亮
    hovered: bool,
}

/// 卡片正文。
enum Body {
    /// 环形仪表：标题、百分比、明细
    Gauge { title: String, pct: Option<f32>, detail: String },
    /// 双环仪表：名称（可带小字明细）在左，右侧两个数据环
    DualGauge { title: String, subtitle: String, rings: [RingSpec; 2] },
    /// 单大圆环仪表：名称（可带小字明细）在左，右侧一个大圆环
    BigGauge { title: String, subtitle: String, pct: Option<f32> },
    /// 横向用量条：标题、右上注、每盘一条
    Bars { title: String, note: String, rows: Vec<BarRow> },
    /// 卷树：标题、右上注、卷行（可点开看底层盘）
    Volumes { title: String, note: String, rows: Vec<VolRow> },
    /// 心电图：标题、样本、当前速率、两条曲线颜色
    Chart { title: String, pts: Vec<(f32, f32, f32)>, detail1: String, detail2: String, c1: (u8, u8, u8, u8), c2: (u8, u8, u8, u8) },
}

/// 卷行：卷名、容量文本、用量比例（0..100）。
struct VolRow {
    name: String,
    /// "5.8G / 25G" 形式的已用 / 总量
    cap: String,
    /// 用量百分比（None 表示无数据，不画进度条）
    ratio: Option<f32>,
}

/// 横条行：盘名、用量比例（0..100）、容量明细。
struct BarRow {
    name: String,
    ratio: Option<f32>,
    cap: String,
}

/// 双环卡中的单个环：环心文字、进度（0..100）、颜色、底部标注。
struct RingSpec {
    center: String,
    ratio: Option<f32>,
    color: (u8, u8, u8, u8),
    cap: String,
}

impl RingSpec {
    fn new(center: impl Into<String>, ratio: Option<f32>, color: (u8, u8, u8, u8), cap: impl Into<String>) -> Self {
        Self { center: center.into(), ratio, color, cap: cap.into() }
    }
}

struct Card {
    body: Body,
    h: f32,
}

impl Card {
    fn gauge(title: impl Into<String>, pct: Option<f32>, detail: String) -> Self {
        Self { body: Body::Gauge { title: title.into(), pct, detail }, h: GAUGE_H }
    }
    fn dual_gauge(title: impl Into<String>, subtitle: impl Into<String>, rings: [RingSpec; 2]) -> Self {
        Self { body: Body::DualGauge { title: title.into(), subtitle: subtitle.into(), rings }, h: CPU_CARD_H }
    }
    fn big_gauge(title: impl Into<String>, subtitle: impl Into<String>, pct: Option<f32>) -> Self {
        Self { body: Body::BigGauge { title: title.into(), subtitle: subtitle.into(), pct }, h: DUAL_GAUGE_H }
    }
    fn bars(title: impl Into<String>, note: String, rows: Vec<BarRow>) -> Self {
        let h = BAR_BASE + rows.len() as f32 * BAR_ROW_H;
        Self { body: Body::Bars { title: title.into(), note, rows }, h }
    }
    /// 卷卡高度：标题/留白 + 每卷一行。
    fn volumes(title: impl Into<String>, note: String, rows: Vec<VolRow>) -> Self {
        let h = BAR_BASE + rows.len() as f32 * VOL_ROW_H;
        Self { body: Body::Volumes { title: title.into(), note, rows }, h }
    }
    fn chart(
        title: impl Into<String>,
        pts: Vec<(f32, f32, f32)>,
        detail1: String,
        detail2: String,
        c1: (u8, u8, u8, u8),
        c2: (u8, u8, u8, u8),
    ) -> Self {
        Self { body: Body::Chart { title: title.into(), pts, detail1, detail2, c1, c2 }, h: CHART_H }
    }
}

impl CapsuleView {
    pub fn new(hwnd: Arc<AtomicIsize>) -> Self {
        let (req_tx, panel_rx) = PanelBuilder::spawn(hwnd);
        Self {
            open: false,
            anim: 0.0,
            anim_from: 0.0,
            anim_start: Instant::now(),
            row_fade: Vec::new(),
            req_tx,
            panel_rx,
            current: None,
            bar: None,
            sent_key: None,
            last_adv_seq: 0,
            gear: crate::icons::gear(GEAR_SIZE as u32, FG_DIM),
            icons: crate::icons::Icons::load(PILL_ICON as u32, FG_DIM),
            gear_pos: (0.0, 0.0),
            taskbar_h: 0,
            hovered: false,
        }
    }

    /// 收起态窗口宽度：固定到任务栏用任务栏内三环窗口宽度，否则为药丸胶囊宽度。
    fn collapsed_width_for(&self, pinned: bool) -> u32 {
        if pinned { RINGS_W } else { PILL_W }
    }

    /// 收起态窗口高度：固定到任务栏用三环窗口高度，否则为药丸胶囊高度。
    fn collapsed_h(&self, pinned: bool) -> u32 {
        if pinned { RINGS_H } else { PILL_H }
    }

    /// 收起态分发：固定到任务栏画透明三圆环，未固定画药丸胶囊。
    fn render_collapsed(&self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        if frame.pinned {
            self.render_rings(px, frame, font);
        } else {
            self.render_pill(px, frame, font);
        }
    }

    /// 未固定时的药丸胶囊：276×32 圆角底，两行 × 三列
    /// （行1 CPU/硬盘读/网络上行，行2 内存/硬盘写/网络下行）。
    fn render_pill(&self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        let Some(font) = font else { return };
        fill_round(px, 0.0, 0.0, PILL_W as f32, PILL_H as f32, PILL_R, frost_bg(frame.frost));

        if !frame.has_data {
            let (text, c) = status_text(frame.conn);
            let tw = font.measure(FONT_SIZE, &text);
            let bl = PILL_H as f32 / 2.0 + font.ascent(FONT_SIZE) * 0.36;
            font.draw(px, (PILL_W as f32 - tw) / 2.0, bl, FONT_SIZE, &text, c);
            return;
        }

        let s = frame.snap;
        let cpu = s.cpu.as_ref().and_then(|c| c.usage).map(|v| v as f32);
        let mem = s.memory.as_ref().and_then(|m| m.usage).map(|v| v as f32);
        let (dr, dw) = disk_io_sum(s);
        let (nr, nw) = net_io_sum(s);
        let (cy0, cy1) = (PILL_ROW_CY[0], PILL_ROW_CY[1]);
        let (x0, x1, x2) = (PILL_COL_X[0], PILL_COL_X[1], PILL_COL_X[2]);

        // 列1：CPU（行1）/ 内存（行2），数值即百分比，无英文简写
        draw_pill_cell(px, font, self.icons.cpu.as_ref(), x0, cy0, None, &pill_pct(cpu), FG);
        draw_pill_cell(px, font, self.icons.ram.as_ref(), x0, cy1, None, &pill_pct(mem), FG);
        // 列2：硬盘读取（行1）/ 写入（行2），R 蓝、W 绿
        draw_pill_cell(px, font, self.icons.disk.as_ref(), x1, cy0, Some(("R", READ_BLUE)), &fmt::rate(dr), FG);
        draw_pill_cell(px, font, self.icons.disk.as_ref(), x1, cy1, Some(("W", LEVEL_GREEN)), &fmt::rate(dw), FG);
        // 列3：网络上行（行1）/ 下行（行2），UL 橙、DL 紫
        draw_pill_cell(px, font, self.icons.net.as_ref(), x2, cy0, Some(("UL", TX_LINE)), &fmt::rate(nw), FG);
        draw_pill_cell(px, font, self.icons.net.as_ref(), x2, cy1, Some(("DL", DL_PURPLE)), &fmt::rate(nr), FG);
    }

    fn render_rings(&self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        draw_rings(px, frame, font);
    }

    fn render_expanded(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        p: f32,
        vis_w: u32,
    ) -> bool {
        let ph = panel_height(frame.has_data, frame.snap);
        // 固定到任务栏且任务栏在下方时，标题行放面板底部（贴任务栏一侧）
        let header_bottom = frame.pinned && frame.expand_up;
        // 数据版本/连接状态/磨砂/面板高度/标题行位置变化时，把整层构建派发到后台线程；
        // 动画帧键不变，不重复派发。面板光栅化（2~4ms）完全离开 UI 关键路径。
        let key = (frame.seq, frame.conn.clone(), frame.frost, ph, header_bottom);
        let up_to_date = self.current.as_ref().is_some_and(|c| {
            c.seq == frame.seq
                && c.conn == *frame.conn
                && c.frost == frame.frost
                && c.ph == ph
                && c.header_bottom == header_bottom
        });
        if !up_to_date && self.sent_key.as_ref() != Some(&key) {
            let _ = self.req_tx.send(PanelJob::Build(BuildReq {
                seq: frame.seq,
                conn: frame.conn.clone(),
                ph,
                frost: frame.frost,
                has_data: frame.has_data,
                header_bottom,
                snap: frame.arc.clone(),
            }));
            self.sent_key = Some(key);
            // 稳态数据帧：本帧不再重画旧面板（约 3ms 后 WM_APP_PANEL 会带着
            // 新面板强制上屏），省掉一次纯浪费的 1MB blit + ULW 提交。
            // 动画进行中不能跳，每一帧都要随进度上屏。
            if p >= 0.999 {
                return false;
            }
        }

        // 面板/顶栏合成走逐行 memcpy 快路径（见 blit_rows），不用 tiny-skia 混合管线

        // 取回新面板放在帧尾（见函数末）：避免在构建线程刚写完 1MB 像素、
        // 缓存尚热在其核心时本帧立即冷读（实测会造成 3~5ms blit 尖峰）。

        // 展开方向：固定到任务栏时窗口本身就是面板（底边贴任务栏，标题行贴任务栏一侧）；
        // 未固定时顶栏贴窗口上沿、面板在其下方。
        let up = frame.expand_up;
        let pinned = frame.pinned;
        let vis_h = self.row_fade.len() as u32;
        let bar_y = 0.0f32;
        let y0 = if pinned { 0i32 } else { (CAPSULE_H + GAP) as i32 };
        // 面板 blit 的纵向锚点：固定态窗口已是全尺寸，面板只画“当前动画高度”那一段
        // （向上展开时贴窗口底、源取面板最下面一段），其余留透明，靠内容生长成动画。
        let (panel_dy, panel_sy, panel_h) = if pinned {
            let h = if p >= 0.999 {
                ph
            } else {
                let r = ease_motion(p);
                ((RINGS_H as f32) + (ph as f32 - RINGS_H as f32) * r).round().max(1.0) as u32
            };
            let h = h.min(ph);
            if up { (vis_h as i32 - h as i32, ph - h, h) } else { (0, 0, h) }
        } else {
            (y0, 0, ph)
        };

        // 面板区域随展开进度淡入（固定态整窗就是面板；未固定时顶栏保持不透明）
        if p < 0.999 {
            // 内容在动画前段就基本不透明（约 45% 处到位），后段靠位移收尾，
            // 避免整段都在半透明里"发灰"。
            let a = ease_out_quad((p / 0.45).clamp(0.0, 1.0));
            let av = ((a * 255.0).round() as u8).max(if p > 0.001 { 1 } else { 0 });
            if pinned || up {
                for v in self.row_fade.iter_mut().take(ph as usize) {
                    *v = av;
                }
            } else {
                for v in self.row_fade.iter_mut().skip(CAPSULE_H as usize) {
                    *v = av;
                }
            }
        }

        // 顶栏：展开时与面板同宽。圆角条按 frost 缓存为 320×52 像素层，
        // 动画中整层 blit（超出可视宽的右段不会被 StretchDIBits 上屏）。
        // 固定在任务栏时顶栏不画底（透明融入任务栏）。
        let bar_w = vis_w;
        if !frame.pinned {
            if !matches!(&self.bar, Some((f, _)) if *f == frame.frost) {
                let mut bpm = Pixmap::new(PANEL_W, CAPSULE_H).unwrap();
                fill_round(
                    &mut bpm,
                    0.0,
                    0.0,
                    PANEL_W as f32,
                    CAPSULE_H as f32,
                    14.0,
                    frost_bg(frame.frost),
                );
                self.bar = Some((frame.frost, bpm));
            }
            blit_rows(px, self.bar.as_ref().unwrap().1.as_ref(), 0, bar_y as i32, 0, vis_w, CAPSULE_H);
        }

        // 展开动画中窗口向左加宽（右缘不动）：面板内容右缘锚定，整体左移宽度差
        let off = bar_w as i32 - PANEL_W as i32; // 动画中为负

        if let Some(font) = font {
            // 固定到深色任务栏时顶栏文字改用浅色
            let light_on_dark = frame.pinned && !frame.taskbar_light;
            let (fg, dim) = if light_on_dark {
                (
                    color(PIN_FG.0, PIN_FG.1, PIN_FG.2, PIN_FG.3),
                    color(PIN_FG_DIM.0, PIN_FG_DIM.1, PIN_FG_DIM.2, PIN_FG_DIM.3),
                )
            } else {
                (color(FG.0, FG.1, FG.2, FG.3), color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3))
            };

            // 未固定：名称/IP 在顶栏（顶栏在面板区域之外，不会被面板 blit 覆盖）；
            // 固定到任务栏时改画在面板标题行（见帧尾），状态灯/状态/齿轮两者都在帧尾画
            if !frame.pinned {
                let name = frame.snap.hostname.as_deref().map(str::trim).filter(|h| !h.is_empty());
                let hbaseline = bar_y + CAPSULE_H as f32 / 2.0 + font.ascent(FONT_SIZE_HEADER) * 0.36;
                let avail = (bar_w as f32 - PANEL_PAD * 2.0).max(20.0);
                draw_title(
                    font,
                    px,
                    14.0,
                    hbaseline,
                    avail,
                    (FONT_SIZE_HEADER, FONT_SIZE),
                    (fg, dim),
                    name,
                    frame.nas_host.trim(),
                );
            }
        }

        // 每帧只 blit 一次已就位的面板。展开首帧/数据刚到达的瞬间新面板可能尚未
        // 构建完，此时该区域逐行 alpha≈0（淡入刚开始），无上屏可见空窗。
        if let Some(c) = &self.current {
            blit_rows(px, c.pm.as_ref(), off, panel_dy, panel_sy, vis_w, panel_h);
        }

        // 帧尾才取回新面板（本帧主体 blit 的是上一帧已就位的面板）：
        // 构建线程 post 完消息即阻塞在 recv，此刻已不占访存带宽；
        // 若拿到新面板，就在同位置补一次 blit 即时上屏（与冷写并发已消除），
        // 数据显示不额外延迟一个采样周期。旧缓冲归还复用，稳态零大内存分配。
        let mut swapped = false;
        while let Ok(panel) = self.panel_rx.try_recv() {
            if let Some(old) = self.current.replace(panel) {
                let _ = self.req_tx.send(PanelJob::Recycle(old.pm));
            }
            swapped = true;
        }
        if swapped {
            if let Some(c) = &self.current {
                blit_rows(px, c.pm.as_ref(), off, panel_dy, panel_sy, vis_w, panel_h);
            }
        }

        // 标题行：状态灯 + 连接状态 + 设置齿轮（固定到任务栏时左侧还有 NAS 名称/IP）。
        // 固定态标题行贴任务栏一侧（任务栏在下方时贴面板底）；未固定时在面板顶部。
        // 必须在面板像素 blit 之后再画——面板底含这一行，先画会被盖掉。
        if let Some(font) = font {
            let baseline = if pinned {
                if up {
                    vis_h as f32 - PANEL_PAD - font.ascent(FONT_SIZE_SMALL)
                } else {
                    PANEL_PAD + font.ascent(FONT_SIZE_SMALL)
                }
            } else {
                y0 as f32 + PANEL_PAD + font.ascent(FONT_SIZE_SMALL)
            };
            let cy = baseline - font.ascent(FONT_SIZE_SMALL) * 0.34;
            // 齿轮贴右内边距；状态文字右对齐到齿轮左侧
            let gx = PANEL_W as f32 - PANEL_PAD - GEAR_SIZE;
            let gy = cy - GEAR_SIZE / 2.0;
            self.gear_pos = (gx, gy);
            let (text, sc, dc) = conn_badge(frame.conn);
            let tw = font.measure(FONT_SIZE_SMALL, &text);
            let status_right = gx - 8.0;
            font.draw(px, status_right - tw, baseline, FONT_SIZE_SMALL, &text, sc);
            // 状态灯：连接状态色实心圆点，排在状态文字之前
            let dot_r = 3.0;
            let dot_cx = status_right - tw - 9.0 - dot_r;
            fill_dot(px, dot_cx, cy, dot_r, dc);
            if frame.pinned {
                // 固定到任务栏：名称/IP 排在状态灯之前（面板浅色底，用主题深色文字）
                let name = frame.snap.hostname.as_deref().map(str::trim).filter(|h| !h.is_empty());
                let avail = (dot_cx - dot_r - 10.0 - PANEL_PAD).max(60.0);
                draw_title(
                    font,
                    px,
                    PANEL_PAD,
                    baseline,
                    avail,
                    (FONT_SIZE_SMALL, FONT_SIZE_SMALL),
                    (color(FG.0, FG.1, FG.2, FG.3), color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3)),
                    name,
                    frame.nas_host.trim(),
                );
            }
            if let Some(g) = self.gear.as_ref() {
                px.draw_pixmap(
                    gx as i32,
                    gy.round() as i32,
                    g.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
        true
    }
}

/// 无变换、整体不透明的 Pixmap 合成快路径：源/目的同为预乘 RGBA，
/// 与 Source 模式 draw_pixmap（opacity=1）逐位等价，但只做逐行 memcpy，
/// 省掉 tiny-skia 逐像素 u16 混合——1MB 面板在低频小核上也只需数百微秒。
/// `dx` 为负（展开动画中面板相对窗口左移）时源从 `-dx` 列起取，右缘锚定；
/// 源纵向起点由 `sy` 指定（固定态面板只画动画高度那一段，取面板最下面一段）。
fn blit_rows(dst: &mut Pixmap, src: PixmapRef<'_>, dx: i32, dy: i32, sy: u32, vis_w: u32, h: u32) {
    use std::ptr;
    let sx = dx.saturating_neg().max(0) as u32;
    let dx = dx.max(0) as u32;
    let dy = dy.max(0) as u32;
    let w = vis_w
        .min(src.width().saturating_sub(sx))
        .min(dst.width().saturating_sub(dx));
    let h = h
        .min(src.height().saturating_sub(sy))
        .min(dst.height().saturating_sub(dy));
    if w == 0 || h == 0 {
        return;
    }
    let sstride = src.width() as usize * 4;
    let dstride = dst.width() as usize * 4;
    let n = w as usize * 4;
    unsafe {
        let sp = src.data().as_ptr();
        let dp = dst.data_mut().as_mut_ptr();
        for i in 0..h as usize {
            ptr::copy_nonoverlapping(
                sp.add((sy as usize + i) * sstride + sx as usize * 4),
                dp.add((dy as usize + i) * dstride + dx as usize * 4),
                n,
            );
        }
    }
}

/// 组装设备卡片：CPU/内存/GPU 仪表、存储状态、硬盘读写、网卡心电图。
/// 心电图历史由后台构建线程持有（UI 线程不再触碰）。
fn build_cards(
    s: &Snapshot,
    net_hist: &HashMap<String, VecDeque<(u64, u64)>>,
    disk_hist: &VecDeque<(u64, u64)>,
) -> Vec<Card> {
    let mut cards = Vec::new();

    if let Some(c) = &s.cpu {
        let usage = c.usage.map(|v| v as f32);
        let r_usage = RingSpec::new(
            usage.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round())),
            usage,
            usage.map_or(ACCENT, usage_color),
            "使用率",
        );
        let r_temp = match c.temp.map(|v| v as f32) {
            Some(t) => RingSpec::new(
                format!("{t:.0}°"),
                Some(((t - 30.0) / 60.0).clamp(0.0, 1.0) * 100.0),
                temp_color(t),
                "温度",
            ),
            None => RingSpec::new("--°", None, ACCENT, "温度"),
        };
        cards.push(Card::dual_gauge("CPU", "", [r_usage, r_temp]));
    }
    if let Some(m) = &s.memory {
        let usage = m.usage.map(|v| v as f32);
        let subtitle = match (m.used, m.total) {
            (Some(u), Some(t)) => format!("{} / {}", fmt::bytes(u), fmt::bytes(t)),
            _ => String::new(),
        };
        cards.push(Card::big_gauge("内存", subtitle, usage));
    }
    for g in s.gpus.as_deref().unwrap_or(&[]) {
        let detail = match (g.temp, g.memory_used, g.memory_total) {
            (t, Some(mu), Some(mt)) => {
                let temp = t.map_or_else(|| "--".into(), |v| format!("{v}°C"));
                format!("{} · 显存 {}/{}", temp, fmt::bytes(mu), fmt::bytes(mt))
            }
            (Some(t), _, _) => format!("{t}°C"),
            _ => String::new(),
        };
        cards.push(Card::gauge(format!("GPU{} · {}", g.index, g.name), g.utilization.map(|v| v as f32), detail));
    }

    let disks = s.disks.as_deref().unwrap_or(&[]);
    let volumes = s.volumes.as_deref().unwrap_or(&[]);
    let has_storage = !disks.is_empty() || !volumes.is_empty();

    // 网卡心电图：每个接口一张图（蓝收/橙发）
    for n in s.network.as_deref().unwrap_or(&[]) {
        let pts = net_hist.get(&n.name).map_or_else(Vec::new, hist_points);
        let d1 = format!("收 {}", n.rx_bytes_per_sec.map_or_else(|| "--".into(), fmt::rate));
        let d2 = format!("发 {}", n.tx_bytes_per_sec.map_or_else(|| "--".into(), fmt::rate));
        cards.push(Card::chart("网络".to_string(), pts, d1, d2, ACCENT, TX_LINE));
    }

    if has_storage {
        // 硬盘读写：所有盘汇总的读（蓝）/写（绿）双曲线心电图
        let (dr, dw) = disk_io_sum(s);
        let pts = hist_points(disk_hist);
        let d1 = format!("读 {}", fmt::rate(dr));
        let d2 = format!("写 {}", fmt::rate(dw));
        cards.push(Card::chart("硬盘读写".to_string(), pts, d1, d2, READ_BLUE, LEVEL_GREEN));

        if !volumes.is_empty() {
            // 方案一：卷为主，默认只显示卷行；点开的卷列出底层盘（standby 灰显）
            let rows = volumes
                .iter()
                .map(|vol| {
                    let cap = match (vol.used_bytes, vol.total_bytes) {
                        (Some(u), Some(t)) if t > 0 => {
                            format!("{} / {}", fmt::bytes(u), fmt::bytes(t))
                        }
                        (_, Some(t)) if t > 0 => format!("总量 {}", fmt::bytes(t)),
                        _ => "容量未知".to_string(),
                    };
                    let ratio = match (vol.used_bytes, vol.total_bytes) {
                        (Some(u), Some(t)) if t > 0 => Some(u as f32 / t as f32 * 100.0),
                        _ => None,
                    };
                    VolRow { name: vol.name.clone(), cap, ratio }
                })
                .collect();
            cards.push(Card::volumes("存储状态", format!("{} 卷", volumes.len()), rows));
        } else {
            // 兜底：旧服务端/无卷数据时，盘直接平铺，保证物理盘可见
            let rows = disks
                .iter()
                .map(|d| {
                    let name = d.device.trim_start_matches("/dev/").to_string();
                    let (ratio, cap) = disk_capacity_text(d);
                    BarRow { name, ratio, cap }
                })
                .collect();
            cards.push(Card::bars("存储状态", format!("{} 块盘", disks.len()), rows));
        }
    }

    cards
}

/// 追加网卡样本并清理已消失的接口（后台线程持有心电图历史）。
fn update_net_hist(hist: &mut HashMap<String, VecDeque<(u64, u64)>>, s: &Snapshot) {
    let Some(nets) = &s.network else { return };
    for n in nets {
        let e = hist.entry(n.name.clone()).or_default();
        e.push_back((n.rx_bytes_per_sec.unwrap_or(0), n.tx_bytes_per_sec.unwrap_or(0)));
        while e.len() > NET_HIST {
            e.pop_front();
        }
    }
    hist.retain(|k, _| nets.iter().any(|n| &n.name == k));
}

/// 追加硬盘总读/总写样本（所有盘汇总），用于读写心电图。
fn update_disk_hist(hist: &mut VecDeque<(u64, u64)>, s: &Snapshot) {
    let (dr, dw) = disk_io_sum(s);
    hist.push_back((dr, dw));
    while hist.len() > NET_HIST {
        hist.pop_front();
    }
}

/// 展开面板总高度（仅由设备数量决定）。
fn panel_height(has_data: bool, snap: &Snapshot) -> u32 {
    if !has_data {
        return (PANEL_PAD * 2.0 + HEADER_H) as u32;
    }
    let hs = card_heights(snap);
    let sum: f32 = hs.iter().sum();
    (PANEL_PAD * 2.0 + HEADER_H + sum + CARD_GAP * (hs.len() as f32 - 1.0)) as u32
}

/// 展开面板高度：固定到任务栏时窗口本身就是面板（底边贴任务栏，无顶栏），
/// 未固定时 = 顶栏 + 间隙 + 面板。
fn expanded_h(frame: &Frame) -> u32 {
    let ph = panel_height(frame.has_data, frame.snap);
    if frame.pinned { ph } else { CAPSULE_H + GAP + ph }
}

/// 卡片区起始 y：标题行在底部时卡片从顶部内边距开始，否则让出顶部标题行。
fn card_top_y(header_bottom: bool) -> f32 {
    if header_bottom { PANEL_PAD } else { PANEL_PAD + HEADER_H }
}

/// 后台面板构建线程：把每秒一次的整层光栅化（2~4ms）移出 UI 帧关键路径。
/// 持有独立的字体实例、心电图历史与可复用 clip mask；
/// 请求通道关闭（UI 退出）时 recv 出错，线程自然结束。
struct PanelBuilder {
    font: Option<Font>,
    net_hist: HashMap<String, VecDeque<(u64, u64)>>,
    disk_hist: VecDeque<(u64, u64)>,
    last_seq: u64,
    clip: Option<Mask>,
    clip_h: u32,
    /// UI 归还的面板像素缓冲（尺寸匹配时复用，稳态零大分配）
    spare: Vec<Pixmap>,
    rx: mpsc::Receiver<PanelJob>,
    tx: mpsc::Sender<BuiltPanel>,
    hwnd: Arc<AtomicIsize>,
}

impl PanelBuilder {
    fn spawn(hwnd: Arc<AtomicIsize>) -> (mpsc::Sender<PanelJob>, mpsc::Receiver<BuiltPanel>) {
        let (req_tx, req_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        thread::Builder::new()
            .name("panel".into())
            .spawn(move || {
                let mut b = PanelBuilder {
                    font: Font::load(),
                    net_hist: HashMap::new(),
                    disk_hist: VecDeque::new(),
                    last_seq: 0,
                    clip: None,
                    clip_h: 0,
                    spare: Vec::new(),
                    rx: req_rx,
                    tx: out_tx,
                    hwnd,
                };
                // 低于 UI 线程优先级：面板构建是 1Hz 离屏活，晚几毫秒无妨，
                // 但不能抢占动画/数据帧造成 6ms 以上的调度尖峰
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                }
                // 启动空闲期预热字形冷光栅化，展开首帧不卡
                if let Some(f) = &b.font {
                    f.preheat();
                }
                b.run();
            })
            .expect("spawn panel thread");
        (req_tx, out_rx)
    }

    fn run(&mut self) {
        while let Ok(first) = self.rx.recv() {
            // 排空积压：Advance 按序推进历史；多个 Build 只构建最新一份。
            // 1Hz 推送下通常没有积压，循环只在动画/切换瞬间生效。
            let mut pending: Option<BuildReq> = None;
            self.handle(first, &mut pending);
            while let Ok(job) = self.rx.try_recv() {
                self.handle(job, &mut pending);
            }
            let Some(req) = pending else { continue };
            let panel = self.build(&req);
            if self.tx.send(panel).is_err() {
                break; // UI 端已释放接收端
            }
            // 构建可能晚于触发帧，主动唤醒 UI 取结果重绘
            let h = self.hwnd.load(Ordering::Acquire);
            if h != 0 {
                unsafe {
                    let _ = PostMessageW(HWND(h as *mut _), WM_APP_PANEL, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    fn handle(&mut self, job: PanelJob, pending: &mut Option<BuildReq>) {
        match job {
            PanelJob::Advance { seq, has_data, snap } => {
                if has_data && seq != self.last_seq {
                    self.last_seq = seq;
                    update_net_hist(&mut self.net_hist, &snap);
                    update_disk_hist(&mut self.disk_hist, &snap);
                }
            }
            PanelJob::Build(req) => *pending = Some(req),
            PanelJob::Recycle(pm) => self.spare.push(pm),
        }
    }

    fn build(&mut self, req: &BuildReq) -> BuiltPanel {
        // 心电图历史只按数据版本推进一次
        if req.has_data && req.seq != self.last_seq {
            self.last_seq = req.seq;
            update_net_hist(&mut self.net_hist, &req.snap);
            update_disk_hist(&mut self.disk_hist, &req.snap);
        }

        let ph = req.ph;
        // 优先复用 UI 归还的同尺寸缓冲；复用前整幅清零（圆角外域必须保持透明）
        let mut pm = match self.spare.iter().position(|p| p.width() == PANEL_W && p.height() == ph) {
            Some(i) => {
                let mut pm = self.spare.swap_remove(i);
                pm.data_mut().fill(0);
                pm
            }
            None => Pixmap::new(PANEL_W, ph).unwrap(),
        };
        if self.clip.as_ref().is_none() || self.clip_h != ph {
            self.clip = Mask::new(PANEL_W, ph);
            self.clip_h = ph;
        }
        let heights = card_heights(&req.snap);

        // 背景与卡片底/描边：clip mask 借用限制在本块内
        {
            let clip = self.clip.as_mut().unwrap();
            fill_round_opaque(&mut pm, clip, 0.0, 0.0, PANEL_W as f32, ph as f32, 14.0, frost_bg(req.frost));

            let mut cy = card_top_y(req.header_bottom);
            for &h in &heights {
                fill_round_opaque(&mut pm, clip, PANEL_PAD, cy, CARD_W, h, 10.0, frost_card_bg(req.frost));
                if !req.frost {
                    let line = rounded_rect(PANEL_PAD + 0.5, cy + 0.5, CARD_W - 1.0, h - 1.0, 10.0);
                    let mut paint = Paint::default();
                    paint.set_color_rgba8(CARD_LINE.0, CARD_LINE.1, CARD_LINE.2, CARD_LINE.3);
                    paint.anti_alias = true;
                    let stroke = Stroke { width: 1.0, ..Default::default() };
                    pm.stroke_path(&line, &paint, &stroke, Transform::identity(), None);
                }
                cy += h + CARD_GAP;
            }
        }

        if let Some(font) = &self.font {
            // 标题行的连接状态灯/文字与设置齿轮由 render_expanded 在面板 blit 之后绘制
            if req.has_data {
                let cards = build_cards(&req.snap, &self.net_hist, &self.disk_hist);
                let mut cy = card_top_y(req.header_bottom);
                for card in &cards {
                    draw_card(&mut pm, font, PANEL_PAD, cy, CARD_W, card);
                    cy += card.h + CARD_GAP;
                }
            }
        }

        BuiltPanel {
            seq: req.seq,
            conn: req.conn.clone(),
            ph,
            frost: req.frost,
            header_bottom: req.header_bottom,
            pm,
        }
    }
}

/// 速率样本队列 -> 归一化折线点（x: 0..1，y1/y2: 1=底 0=顶，两线各自独立归一化）。
fn hist_points(q: &VecDeque<(u64, u64)>) -> Vec<(f32, f32, f32)> {
    let n_pts = q.len();
    if n_pts < 2 {
        return Vec::new();
    }
    // 两条线共用两通道中的最大值归一化。副通道（tx/write）远小于主通道时
    // 自然贴底，只有真有大流量时才起来，不会显得"带宽跑满"。
    let max = q
        .iter()
        .fold(1.0f64, |m, e| m.max(e.0 as f64).max(e.1 as f64))
        * 1.2; // 20% 头部余量
    q.iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i as f32 / (n_pts - 1) as f32,
                (1.0 - e.0 as f64 / max).clamp(0.0, 1.0) as f32,
                (1.0 - e.1 as f64 / max).clamp(0.0, 1.0) as f32,
            )
        })
        .collect()
}

/// 绘制一张卡片正文（背景与描边由 panel_bg 缓存统一提供），按正文类型分发。
fn draw_card(px: &mut Pixmap, font: &Font, x: f32, y: f32, w: f32, card: &Card) {
    let h = card.h;

    let fg = color(FG.0, FG.1, FG.2, FG.3);
    let dim = color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3);

    match &card.body {
        Body::DualGauge { title, subtitle, rings } => {
            // 左：名称（有小字明细时名称在上、明细在下，整体垂直居中）
            let title = fit_text_bold(font, FONT_SIZE_TITLE, title, 124.0);
            if subtitle.is_empty() {
                font.draw_bold(
                    px,
                    x + CARD_PAD,
                    y + h / 2.0 + font.ascent(FONT_SIZE_TITLE) / 2.0,
                    FONT_SIZE_TITLE,
                    &title,
                    fg,
                );
            } else {
                let sub = fit_text(font, FONT_SIZE_SMALL, subtitle, 124.0);
                font.draw_bold(px, x + CARD_PAD, y + 40.0 + font.ascent(FONT_SIZE_TITLE), FONT_SIZE_TITLE, &title, fg);
                font.draw(px, x + CARD_PAD, y + 74.0, FONT_SIZE_SMALL, &sub, dim);
            }
            // 右：两个加大的数据环（标注在环内：上数值、下小字）
            const RING_R: f32 = 33.0;
            const RING_FS: f32 = 17.0;
            const CAP_FS: f32 = 10.0;
            let cy = y + h / 2.0;
            let cx_t = x + w - CARD_PAD - RING_R - 1.0;
            let cx_u = cx_t - 82.0;

            let mut one = |cx: f32, spec: &RingSpec| {
                draw_ring(px, cx, cy, RING_R, 9.0, spec.ratio, spec.color);
                let vc = if spec.ratio.is_some() {
                    color(spec.color.0, spec.color.1, spec.color.2, spec.color.3)
                } else {
                    fg
                };
                // 数值居环心偏上
                let tw = font.measure(RING_FS, &spec.center);
                font.draw(px, cx - tw / 2.0, cy - 1.0, RING_FS, &spec.center, vc);
                // 标注在数值下方，环内居中
                let cw = font.measure(CAP_FS, &spec.cap);
                font.draw(px, cx - cw / 2.0, cy + 13.0, CAP_FS, &spec.cap, dim);
            };

            one(cx_u, &rings[0]);
            one(cx_t, &rings[1]);
        }
        Body::BigGauge { title, subtitle, pct } => {
            // 左：名称 + 小字明细，垂直居中
            let title = fit_text_bold(font, FONT_SIZE_TITLE, title, 150.0);
            if subtitle.is_empty() {
                font.draw_bold(
                    px,
                    x + CARD_PAD,
                    y + h / 2.0 + font.ascent(FONT_SIZE_TITLE) / 2.0,
                    FONT_SIZE_TITLE,
                    &title,
                    fg,
                );
            } else {
                let sub = fit_text(font, FONT_SIZE_SMALL, subtitle, 150.0);
                font.draw_bold(px, x + CARD_PAD, y + 40.0 + font.ascent(FONT_SIZE_TITLE), FONT_SIZE_TITLE, &title, fg);
                font.draw(px, x + CARD_PAD, y + 74.0, FONT_SIZE_SMALL, &sub, dim);
            }
            // 右：单个大圆环
            const RING_R: f32 = 36.0;
            const RING_FS: f32 = 19.0;
            let cy = y + h / 2.0;
            let cx = x + w - CARD_PAD - RING_R - 2.0;
            let rc = pct.map_or(ACCENT, usage_color);
            draw_ring(px, cx, cy, RING_R, 8.0, *pct, rc);
            let ctext = pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
            let tw = font.measure(RING_FS, &ctext);
            let vc = if pct.is_some() { color(rc.0, rc.1, rc.2, rc.3) } else { fg };
            font.draw(px, cx - tw / 2.0, cy + font.ascent(RING_FS) / 2.0, RING_FS, &ctext, vc);
        }
        Body::Gauge { title, pct, detail } => {
            // 左：环形仪表（按使用率分级变色）
            let cx = x + CARD_PAD + 20.0;
            let cy = y + h / 2.0;
            let ring_c = pct.map_or(ACCENT, usage_color);
            draw_ring(px, cx, cy, 20.0, 5.0, *pct, ring_c);
            let label = pct.map_or_else(|| "—".into(), |v| format!("{:.0}%", v.round()));
            let tw = font.measure(FONT_SIZE_SMALL, &label);
            let lc = if pct.is_some() { color(ring_c.0, ring_c.1, ring_c.2, ring_c.3) } else { fg };
            font.draw(px, cx - tw / 2.0, cy + font.ascent(FONT_SIZE_SMALL) / 2.0, FONT_SIZE_SMALL, &label, lc);
            // 右：标题 + 明细
            let tx = x + CARD_PAD + 56.0;
            let avail = w - (tx - x) - CARD_PAD;
            let title = fit_text_bold(font, FONT_SIZE, title, avail);
            font.draw_bold(px, tx, y + 30.0 + font.ascent(FONT_SIZE), FONT_SIZE, &title, fg);
            if !detail.is_empty() {
                font.draw(px, tx, y + 58.0, FONT_SIZE_SMALL, detail, dim);
            }
        }
        Body::Bars { title, note, rows } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            if !note.is_empty() {
                let nw = font.measure(FONT_SIZE_SMALL, note);
                font.draw(px, x + w - CARD_PAD - nw, y + 24.0, FONT_SIZE_SMALL, note, dim);
            }
            for (i, row) in rows.iter().enumerate() {
                let ry = y + 28.0 + i as f32 * BAR_ROW_H;
                // 信息行：仅盘名（不显示温度与电源状态）
                let ib = ry + 17.0;
                font.draw(px, x + CARD_PAD, ib, FONT_SIZE_SMALL, &row.name, fg);
                // 用量横条
                let bx = x + CARD_PAD;
                let by = ry + 24.0;
                const BH: f32 = 10.0;
                let bw = w - CARD_PAD * 2.0;
                fill_round(px, bx, by, bw, BH, BH / 2.0, TRACK);
                if let Some(p) = row.ratio {
                    let pc = usage_color(p);
                    let fw = (bw * (p / 100.0).clamp(0.0, 1.0)).max(BH);
                    fill_round(px, bx, by, fw, BH, BH / 2.0, pc);
                }
                // 容量明细
                font.draw(px, x + CARD_PAD, ry + 48.0, FONT_SIZE_SMALL, &row.cap, dim);
            }
        }
        Body::Volumes { title, note, rows } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            if !note.is_empty() {
                let nw = font.measure(FONT_SIZE_SMALL, note);
                font.draw(px, x + w - CARD_PAD - nw, y + 24.0, FONT_SIZE_SMALL, note, dim);
            }
            let mut ry = y + 28.0;
            for row in rows {
                // 卷行：上半 卷名(左) + 容量文本(右)；下半 用量进度条
                let text_baseline = ry + 16.0;
                let right = x + w - CARD_PAD;
                let cap_w = font.measure(FONT_SIZE_SMALL, &row.cap);
                font.draw(px, right - cap_w, text_baseline, FONT_SIZE_SMALL, &row.cap, dim);
                // 卷名超长时按剩余宽度截断，避免与容量文本重叠
                let name_max = right - cap_w - 8.0 - (x + CARD_PAD);
                let vname = fit_text(font, FONT_SIZE_SMALL, &row.name, name_max.max(20.0));
                font.draw(px, x + CARD_PAD, text_baseline, FONT_SIZE_SMALL, &vname, fg);
                // 用量进度条
                const PB_H: f32 = 6.0;
                let bx = x + CARD_PAD;
                let by = ry + 24.0;
                let bw = w - CARD_PAD * 2.0;
                fill_round(px, bx, by, bw, PB_H, PB_H / 2.0, TRACK);
                if let Some(p) = row.ratio {
                    let pc = usage_color(p);
                    let fw = (bw * (p / 100.0).clamp(0.0, 1.0)).max(PB_H);
                    fill_round(px, bx, by, fw, PB_H, PB_H / 2.0, pc);
                }
                ry += VOL_ROW_H;
            }
        }
        Body::Chart { title, pts, detail1, detail2, c1, c2 } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            if !pts.is_empty() {
                draw_chart(px, x + CARD_PAD, y + 32.0, w - CARD_PAD * 2.0, 42.0, pts, *c1, *c2);
            }
            // 底部明细：左段用 c1 色，右段用 c2 色（与线条颜色一致）
            let by = y + h - 8.0;
            let c1c = color(c1.0, c1.1, c1.2, c1.3);
            let c2c = color(c2.0, c2.1, c2.2, c2.3);
            if !detail1.is_empty() {
                font.draw(px, x + CARD_PAD, by, FONT_SIZE_SMALL, detail1, c1c);
            }
            if !detail2.is_empty() {
                let w2 = font.measure(FONT_SIZE_SMALL, detail2);
                font.draw(px, x + w - CARD_PAD - w2, by, FONT_SIZE_SMALL, detail2, c2c);
            }
        }
    }
}

/// 无卷数据兜底场景的物理盘容量文本（仅用盘自身统计）。
fn disk_capacity_text(d: &Disk) -> (Option<f32>, String) {
    let total = d.total_bytes.filter(|t| *t > 0);
    let used = d.used_bytes;
    match (used, total) {
        (Some(u), Some(t)) => (
            Some(u as f32 / t as f32 * 100.0),
            format!("已用 {} / {}", fmt::bytes(u), fmt::bytes(t)),
        ),
        (None, Some(t)) => (None, format!("总量 {}", fmt::bytes(t))),
        _ => (None, "容量未知".to_string()),
    }
}

/// 环形仪表：灰轨道 + 分级色弧。
fn draw_ring(px: &mut Pixmap, cx: f32, cy: f32, r: f32, width: f32, pct: Option<f32>, c: (u8, u8, u8, u8)) {
    let mut stroke = |path: &Path, col: (u8, u8, u8, u8), width: f32| {
        let mut paint = Paint::default();
        paint.set_color_rgba8(col.0, col.1, col.2, col.3);
        paint.anti_alias = true;
        let s = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        px.stroke_path(path, &paint, &s, Transform::identity(), None);
    };
    if let Some(p) = arc_path(cx, cy, r, -TAU / 4.0, -TAU / 4.0 + TAU) {
        stroke(&p, TRACK, width);
    }
    if let Some(v) = pct {
        let v = (v / 100.0).clamp(0.0, 1.0);
        if v > 0.005 {
            if let Some(p) = arc_path(cx, cy, r, -TAU / 4.0, -TAU / 4.0 + TAU * v) {
                stroke(&p, c, width);
            }
        }
    }
}

/// 使用率分级颜色：<60% 绿、60~85% 橙、≥85% 红。
fn usage_color(v: f32) -> (u8, u8, u8, u8) {
    if v < 60.0 {
        LEVEL_GREEN
    } else if v < 85.0 {
        WARN
    } else {
        LEVEL_RED
    }
}

/// 温度分级颜色：<60°C 绿、60~80°C 橙、≥80°C 红。
fn temp_color(t: f32) -> (u8, u8, u8, u8) {
    if t < 60.0 {
        LEVEL_GREEN
    } else if t < 80.0 {
        WARN
    } else {
        LEVEL_RED
    }
}

/// 圆弧折线路径（角度制采样，a0/a1 为弧度）。
fn arc_path(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Option<Path> {
    let steps = (((a1 - a0) / TAU) * 72.0).ceil().max(2.0) as usize;
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let a = a0 + (a1 - a0) * i as f32 / steps as f32;
        let (sx, sy) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 {
            pb.move_to(sx, sy);
        } else {
            pb.line_to(sx, sy);
        }
    }
    pb.finish()
}

/// 折线/面积图：rx 蓝线、tx 橙线（或读/写绿紫），底部基线，下方半透明填充。
#[allow(clippy::too_many_arguments)]
fn draw_chart(
    px: &mut Pixmap,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    pts: &[(f32, f32, f32)],
    c1: Rgba,
    c2: Rgba,
) {
    fill_round(px, cx, cy + ch, cw, 1.0, 0.0, TRACK);
    for (comp, c, width) in [(1usize, c1, 1.4f32), (2, c2, 1.0)] {
        // 面积图：下方填充半透明底色，上方描折线
        let mut pb_fill = PathBuilder::new();
        let mut pb_line = PathBuilder::new();
        for (i, p) in pts.iter().enumerate() {
            let vx = cx + p.0 * cw;
            let v = if comp == 1 { p.1 } else { p.2 };
            let vy = cy + v.clamp(0.0, 1.0) * ch;
            if i == 0 {
                pb_fill.move_to(vx, cy + ch);
                pb_fill.line_to(vx, vy);
                pb_line.move_to(vx, vy);
            } else {
                pb_fill.line_to(vx, vy);
                pb_line.line_to(vx, vy);
            }
        }
        // 面积填充闭合
        if !pts.is_empty() {
            let last_x = cx + pts[pts.len() - 1].0 * cw;
            pb_fill.line_to(last_x, cy + ch);
            pb_fill.close();
        }
        // 填充面积
        if let Some(path) = pb_fill.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(c.0, c.1, c.2, (c.3 / 6).max(8));
            paint.anti_alias = true;
            px.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        // 描折线
        if let Some(path) = pb_line.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(c.0, c.1, c.2, c.3);
            paint.anti_alias = true;
            let stroke = Stroke { width, ..Default::default() };
            px.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// 超出 max_w 的文本截断加省略号。
fn fit_text(font: &Font, size: f32, text: &str, max_w: f32) -> String {
    if font.measure(size, text) <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let candidate = format!("{out}{ch}");
        if font.measure(size, &format!("{candidate}…")) > max_w {
            break;
        }
        out = candidate;
    }
    format!("{out}…")
}

/// fit_text 的粗体版：截断宽度按粗体测量，避免粗体字超宽后仍按常规字宽截断。
fn fit_text_bold(font: &Font, size: f32, text: &str, max_w: f32) -> String {
    if font.measure_bold(size, text) <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let candidate = format!("{out}{ch}");
        if font.measure_bold(size, &format!("{candidate}…")) > max_w {
            break;
        }
        out = candidate;
    }
    format!("{out}…")
}

/// 标题行连接状态：短标签 + 文字颜色 + 状态灯颜色（在线为绿灯）。
fn conn_badge(conn: &ConnStatus) -> (String, Color, (u8, u8, u8, u8)) {
    let (_, text_color) = status_text(conn);
    match conn {
        ConnStatus::Online => ("已连接".to_string(), text_color, LEVEL_GREEN),
        ConnStatus::Connecting => ("连接中…".to_string(), text_color, ACCENT),
        ConnStatus::Reconnect(n) => (format!("{n}s 后重连"), text_color, WARN),
        ConnStatus::AuthFailed => ("令牌错误".to_string(), text_color, LEVEL_RED),
    }
}

/// 状态灯：实心圆点。
fn fill_dot(px: &mut Pixmap, cx: f32, cy: f32, r: f32, c: (u8, u8, u8, u8)) {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.0, c.1, c.2, c.3);
        paint.anti_alias = true;
        px.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// 画「NAS 名称（粗体）+ IP」标题：名称超宽按 avail 截断；缺名称只显 IP，都没有则 “NAS” 兜底。
/// 未固定时用在顶栏（大字号），固定到任务栏时用在面板标题行（小字号，排在“已连接”之前）。
#[allow(clippy::too_many_arguments)]
fn draw_title(
    font: &Font,
    px: &mut Pixmap,
    x: f32,
    baseline: f32,
    avail: f32,
    sizes: (f32, f32),
    colors: (Color, Color),
    name: Option<&str>,
    ip: &str,
) {
    let (name_size, ip_size) = sizes;
    let (fg, dim) = colors;
    let gap = 8.0;
    match (name, ip.is_empty()) {
        (Some(n), false) => {
            let ip_w = font.measure(ip_size, ip);
            let name_max = (avail - ip_w - gap).max(30.0);
            let nlabel = fit_text_bold(font, name_size, n, name_max);
            let n_end = font.draw_bold(px, x, baseline, name_size, &nlabel, fg);
            font.draw(px, n_end + gap, baseline, ip_size, ip, dim);
        }
        (Some(n), true) => {
            let nlabel = fit_text_bold(font, name_size, n, avail);
            font.draw_bold(px, x, baseline, name_size, &nlabel, fg);
        }
        (None, false) => {
            let ilabel = fit_text(font, ip_size, ip, avail);
            font.draw(px, x, baseline, ip_size, &ilabel, dim);
        }
        (None, true) => {
            font.draw(px, x, baseline, ip_size, "NAS", dim);
        }
    }
}

impl View for CapsuleView {
    fn render(&mut self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>, vis: (u32, u32)) -> bool {
        // 重建每行不透明度表（默认全不透明），长度只覆盖可视行
        self.row_fade.clear();
        self.row_fade.resize(vis.1 as usize, 255);

        if self.anim > 0.0005 || (frame.pinned && self.open) {
            self.render_expanded(px, frame, font, self.anim, vis.0)
        } else {
            // 收起后释放后台面板像素缓冲与派发记录：旧缓冲归还构建线程复用，
            // 在途结果同样回收，避免下次展开闪一帧过期面板（历史样本仍在线程累积）
            self.sent_key = None;
            if let Some(old) = self.current.take() {
                let _ = self.req_tx.send(PanelJob::Recycle(old.pm));
            }
            while let Ok(panel) = self.panel_rx.try_recv() {
                let _ = self.req_tx.send(PanelJob::Recycle(panel.pm));
            }
            // 收起态也按数据版本把快照发给构建线程推进心电图，展开即有满图；
            // 只做两次 VecDeque push，线程不做任何光栅化。
            if frame.has_data && frame.seq != self.last_adv_seq {
                self.last_adv_seq = frame.seq;
                let _ = self.req_tx.send(PanelJob::Advance {
                    seq: frame.seq,
                    has_data: frame.has_data,
                    snap: frame.arc.clone(),
                });
            }
            self.render_collapsed(px, frame, font);
            true
        }
    }

    /// 是否处于展开视觉态（含动画过程）。
    fn expanded(&self) -> bool {
        self.anim > 0.0005
    }

    fn set_expanded(&mut self, open: bool) {
        if self.open == open {
            return;
        }
        self.open = open;
        // 从当前进度开始，支持动画中途反向
        self.anim_from = self.anim;
        self.anim_start = Instant::now();
    }

    /// 立即跳到目标展开态，不做尺寸动画（固定在任务栏时的面板形态）。
    fn jump_anim(&mut self) {
        self.anim = if self.open { 1.0 } else { 0.0 };
        self.anim_from = self.anim;
    }

    fn tick(&mut self) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        if (self.anim - target).abs() < f32::EPSILON {
            return false;
        }
        let dur = if self.open { EXPAND_MS } else { COLLAPSE_MS };
        let t = (self.anim_start.elapsed().as_secs_f32() * 1000.0 / dur).clamp(0.0, 1.0);
        self.anim = self.anim_from + (target - self.anim_from) * t;
        if t >= 1.0 {
            self.anim = target;
            false
        } else {
            true
        }
    }

    /// 展开动画过程中的“可见面板高度”。固定态窗口一次到位、面板内容由贴任务栏一侧
    /// 生长，亚克力背景层按此高度跟随，始终与可见面板同框（避免露出整块灰底）。
    fn visible_panel_h(&self, frame: &Frame) -> u32 {
        let ph = panel_height(frame.has_data, frame.snap);
        if !frame.pinned || self.anim >= 0.999 {
            return ph;
        }
        if self.anim <= 0.0005 {
            return RINGS_H;
        }
        let r = ease_motion(self.anim);
        ((RINGS_H as f32) + (ph as f32 - RINGS_H as f32) * r)
            .round()
            .clamp(1.0, ph as f32) as u32
    }

    fn frame_size(&self, frame: &Frame) -> (u32, u32) {
        // 固定到任务栏：窗口尺寸一次到位（展开动画只动内容，不再每帧改窗口几何，
        // 否则 SetWindowPos 的合成开销会让动画抖动）
        if frame.pinned {
            return (PANEL_W, expanded_h(frame));
        }
        let cw = self.collapsed_width_for(frame.pinned);
        let ch = self.collapsed_h(frame.pinned);
        if self.anim <= 0.0005 {
            (cw, ch)
        } else {
            let full = expanded_h(frame);
            let r = ease_motion(self.anim);
            let w = cw as f32 + (PANEL_W as f32 - cw as f32) * r;
            let h = ch as f32 + (full as f32 - ch as f32) * r;
            (w.round() as u32, h.round() as u32)
        }
    }

    fn buffer_size(&self, frame: &Frame) -> (u32, u32) {
        if self.anim <= 0.0005 && !self.open {
            (self.collapsed_width_for(frame.pinned), self.collapsed_h(frame.pinned))
        } else {
            // 展开目标态（含展开/收起动画过程）：一次分配到全展开尺寸，
            // 动画期间只改变可视窗口与逐行淡入，不再重建缓冲与 DIB。
            (PANEL_W, expanded_h(frame))
        }
    }

    fn row_fade(&self) -> &[u8] {
        &self.row_fade
    }

    /// 设置齿轮命中测试（仅展开动画完全到位后生效）。齿轮在面板标题行右侧。
    fn hit_settings(&self, x: i32, y: i32) -> bool {
        if !(self.open && self.anim > 0.999) {
            return false;
        }
        let (gx, gy) = self.gear_pos;
        let pad = 6.0; // 命中区比图标略大，便于点按
        let xf = x as f32;
        let yf = y as f32;
        xf >= gx - pad && xf <= gx + GEAR_SIZE + pad && yf >= gy - pad && yf <= gy + GEAR_SIZE + pad
    }

    /// 按任务栏高度调整垂直居中（三圆环布局不再需要药丸底内缩）。
    /// 返回布局是否变化。
    fn set_taskbar_style(&mut self, taskbar_h: i32, _mini: bool) -> bool {
        if taskbar_h == self.taskbar_h {
            return false;
        }
        self.taskbar_h = taskbar_h;
        true
    }

    /// 鼠标悬停态：圆环描边加亮。返回是否发生变化。
    fn set_hovered(&mut self, on: bool) -> bool {
        if self.hovered == on {
            return false;
        }
        self.hovered = on;
        true
    }
}

/// 缓动：三次贝塞尔，等价 Fluent/Material 的标准曲线 `cubic-bezier(0.4, 0, 0.2, 1)`。
/// 进场（p 递增）＝快出慢收，出场（p 递减）＝慢起快收，同一曲线两个方向都顺。
/// 贝塞尔需先按横坐标反解参数 u（牛顿迭代 4 次，每帧仅调用两次，开销可忽略）。
fn ease_motion(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    const X1: f32 = 0.4;
    const Y1: f32 = 0.0;
    const X2: f32 = 0.2;
    const Y2: f32 = 1.0;
    /// 贝塞尔分量：3(1-u)²u·p1 + 3(1-u)u²·p2 + u³
    fn bez(u: f32, p1: f32, p2: f32) -> f32 {
        let m = 1.0 - u;
        3.0 * m * m * u * p1 + 3.0 * m * u * u * p2 + u * u * u
    }
    /// 上式的导数
    fn dbez(u: f32, p1: f32, p2: f32) -> f32 {
        let m = 1.0 - u;
        3.0 * m * m * p1 + 6.0 * m * u * (p2 - p1) + 3.0 * u * u * (1.0 - p2)
    }
    let mut u = t;
    for _ in 0..4 {
        let d = dbez(u, X1, X2);
        if d.abs() < 1e-6 {
            break;
        }
        u = (u - (bez(u, X1, X2) - t) / d).clamp(0.0, 1.0);
    }
    bez(u, Y1, Y2)
}

/// 缓动：二次 ease-out，用于内容淡入（更快到位）。
fn ease_out_quad(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

impl CapsuleView {
    /// 未固定时的药丸胶囊收起宽度。
    pub const fn pill_width() -> u32 {
        PILL_W
    }
}

/// 各卡片高度（与运行时数据无关，仅由设备数量决定）。
/// 顺序：CPU → 内存 → GPU → 网卡图 → 硬盘读写图 → 存储状态
fn card_heights(s: &Snapshot) -> Vec<f32> {
    let mut v = Vec::new();
    if s.cpu.is_some() {
        v.push(CPU_CARD_H);
    }
    if s.memory.is_some() {
        v.push(DUAL_GAUGE_H);
    }
    let ng = s.gpus.as_deref().map_or(0, |g| g.len());
    v.extend(std::iter::repeat_n(GAUGE_H, ng));
    // 网卡心电图：每接口一张
    let nn = s.network.as_deref().map_or(0, |n| n.len());
    v.extend(std::iter::repeat_n(CHART_H, nn));

    let nd = s.disks.as_deref().map_or(0, |d| d.len());
    let nv = s.volumes.as_deref().map_or(0, |v| v.len());
    if nd + nv > 0 {
        // 硬盘读写汇总图
        v.push(CHART_H);
        // 存储状态卡（卷行或盘平铺）
        if nv > 0 {
            v.push(BAR_BASE + nv as f32 * VOL_ROW_H);
        } else {
            v.push(BAR_BASE + nd as f32 * BAR_ROW_H);
        }
    }
    v
}

fn status_text(conn: &ConnStatus) -> (String, Color) {
    match conn {
        ConnStatus::Online => ("等待数据…".into(), color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3)),
        ConnStatus::Connecting => ("正在连接 NAS…".into(), color(ACCENT.0, ACCENT.1, ACCENT.2, ACCENT.3)),
        ConnStatus::Reconnect(n) => (format!("已断开，{n}s 后重连"), color(WARN.0, WARN.1, WARN.2, WARN.3)),
        ConnStatus::AuthFailed => ("配对令牌错误".into(), color(LEVEL_RED.0, LEVEL_RED.1, LEVEL_RED.2, LEVEL_RED.3)),
    }
}

fn disk_io_sum(s: &Snapshot) -> (u64, u64) {
    s.disks
        .as_ref()
        .map(|v| {
            v.iter().fold((0u64, 0u64), |(r, w), d| {
                (r + d.read_bytes_per_sec.unwrap_or(0), w + d.write_bytes_per_sec.unwrap_or(0))
            })
        })
        .unwrap_or((0, 0))
}

fn net_io_sum(s: &Snapshot) -> (u64, u64) {
    s.network
        .as_ref()
        .map(|v| {
            v.iter().fold((0u64, 0u64), |(r, w), n| {
                (r + n.rx_bytes_per_sec.unwrap_or(0), w + n.tx_bytes_per_sec.unwrap_or(0))
            })
        })
        .unwrap_or((0, 0))
}

/// 药丸胶囊单元格：图标（可选，随行中线垂直居中）+ 彩色前缀（可选，其后留一个字符间距）
/// + 前景色数值。返回结束 x。
#[allow(clippy::too_many_arguments)]
fn draw_pill_cell(
    px: &mut Pixmap,
    font: &Font,
    icon: Option<&Pixmap>,
    x: f32,
    cy: f32,
    prefix: Option<(&str, Rgba)>,
    value: &str,
    vc: Rgba,
) -> f32 {
    let mut tx = x;
    if let Some(ic) = icon {
        let isz = ic.width() as f32;
        px.draw_pixmap(
            x as i32,
            (cy - isz / 2.0).round() as i32,
            ic.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
        tx = x + isz + 4.0;
    }
    let bl = cy + font.ascent(FONT_SIZE_SMALL) * 0.36;
    if let Some((p, pc)) = prefix {
        tx = font.draw(px, tx, bl, FONT_SIZE_SMALL, p, color(pc.0, pc.1, pc.2, pc.3));
        // 前缀与数值之间保留一个字符间距
        tx += font.measure(FONT_SIZE_SMALL, "R");
    }
    font.draw(px, tx, bl, FONT_SIZE_SMALL, value, color(vc.0, vc.1, vc.2, vc.3))
}

/// 药丸胶囊里的百分比文本（无数据为 `--%`）。
fn pill_pct(v: Option<f32>) -> String {
    v.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()))
}

/// 任务栏内三环子窗口的尺寸：宽 = 三环排布宽度，高 = 与任务栏等高（环与文字都收在条内）。
pub(crate) const RINGS_W: u32 = 190;
pub(crate) const RINGS_H: u32 = 48;

/// 三圆环收起态：透明背景 + 横向三环（CPU / 内存 / 网络），数值与标注都收在环内。
/// 由任务栏内的子窗口直接调用，不依赖 View 实例。
pub(crate) fn draw_rings(px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
    let Some(font) = font else { return };
    // 环内数值各自用分级色，标注用灰色（深色任务栏换浅灰）
    let dim = if frame.pinned && !frame.taskbar_light {
        color(PIN_FG_DIM.0, PIN_FG_DIM.1, PIN_FG_DIM.2, PIN_FG_DIM.3)
    } else {
        color(FG_DIM.0, FG_DIM.1, FG_DIM.2, FG_DIM.3)
    };

    if !frame.has_data {
        // 无数据时只画一个居中的状态环（灰轨道）+ 状态文字
        let (text, c) = status_text(frame.conn);
        let cx = RINGS_W as f32 / 2.0;
        draw_ring(px, cx, RING_CY, RING_R, RING_STROKE, None, TRACK);
        draw_ring_inner(px, font, cx, RING_CY, &text, c, "", dim);
        return;
    }

    let s = frame.snap;
    let cpu_pct = s.cpu.as_ref().and_then(|c| c.usage).map(|v| v as f32);
    let mem_pct = s.memory.as_ref().and_then(|m| m.usage).map(|v| v as f32);
    let (nr, nw) = net_io_sum(s);

    // CPU 环：数值（分色）+ 环内小字标注
    let cpu_color = cpu_pct.map_or(ACCENT, usage_color);
    draw_ring(px, RING_CX[0], RING_CY, RING_R, RING_STROKE, cpu_pct, cpu_color);
    let cpu_text = cpu_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    draw_ring_inner(px, font, RING_CX[0], RING_CY, &cpu_text, color(cpu_color.0, cpu_color.1, cpu_color.2, cpu_color.3), "CPU", dim);

    // 内存环
    let mem_color = mem_pct.map_or(ACCENT, usage_color);
    draw_ring(px, RING_CX[1], RING_CY, RING_R, RING_STROKE, mem_pct, mem_color);
    let mem_text = mem_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    draw_ring_inner(px, font, RING_CX[1], RING_CY, &mem_text, color(mem_color.0, mem_color.1, mem_color.2, mem_color.3), "内存", dim);

    // 网络环：环内上行（橙）/ 下行（紫）两行速率
    draw_ring(px, RING_CX[2], RING_CY, RING_R, RING_STROKE, None, ACCENT);
    draw_net_inner(px, font, RING_CX[2], RING_CY, &format!("UL {}", fmt::rate(nw)), &format!("DL {}", fmt::rate(nr)));
}

/// 环内文字：上行数值（分色）+ 下行灰色小字标注，两行都收在环内。
#[allow(clippy::too_many_arguments)]
fn draw_ring_inner(px: &mut Pixmap, font: &Font, cx: f32, cy: f32, value: &str, vc: Color, label: &str, dim: Color) {
    let vw = font.measure(RING_VALUE_FS, value);
    font.draw(px, cx - vw / 2.0, cy - 1.0, RING_VALUE_FS, value, vc);
    if !label.is_empty() {
        let lw = font.measure(RING_LABEL_FS, label);
        font.draw(px, cx - lw / 2.0, cy + 12.0, RING_LABEL_FS, label, dim);
    }
}

/// 网络环内文字：上行（橙）/ 下行（紫）两行小字速率。
fn draw_net_inner(px: &mut Pixmap, font: &Font, cx: f32, cy: f32, ul: &str, dl: &str) {
    let uw = font.measure(RING_NET_FS, ul);
    let dw = font.measure(RING_NET_FS, dl);
    font.draw(px, cx - uw / 2.0, cy - 2.0, RING_NET_FS, ul, color(TX_LINE.0, TX_LINE.1, TX_LINE.2, TX_LINE.3));
    font.draw(px, cx - dw / 2.0, cy + 10.0, RING_NET_FS, dl, color(DL_PURPLE.0, DL_PURPLE.1, DL_PURPLE.2, DL_PURPLE.3));
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

/// 抗锯齿圆角矩形填充（半透明/磨砂路径用）。
fn fill_round(px: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: (u8, u8, u8, u8)) {
    let path = rounded_rect(x, y, w, h, r);
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.0, c.1, c.2, c.3);
    paint.anti_alias = true;
    px.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// 不透明圆角矩形快速填充。
/// tiny-skia 的抗锯齿 fill_path 会对矩形内全部像素（约 20 万）走一遍
/// 覆盖/混合管线；而圆角需要抗锯齿的只有四个角。这里先用 Source 直写
/// 十字内域（无混合），再把同一条圆角路径限制在四个 r×r 角块 clip 内填充，
/// 抗锯齿像素量降到 4r² 量级。半透明色（磨砂模式）回退原路径。
#[allow(clippy::too_many_arguments)]
fn fill_round_opaque(
    px: &mut Pixmap,
    clip: &mut Mask,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    c: (u8, u8, u8, u8),
) {
    if c.3 != 255 {
        fill_round(px, x, y, w, h, r, c);
        return;
    }
    let mut p = Paint::default();
    p.set_color_rgba8(c.0, c.1, c.2, 255);
    p.blend_mode = BlendMode::Source;
    p.anti_alias = false;
    px.fill_rect(
        Rect::from_xywh(x + r, y, (w - 2.0 * r).max(0.0), h).unwrap(),
        &p,
        Transform::identity(),
        None,
    );
    px.fill_rect(
        Rect::from_xywh(x, y + r, w, (h - 2.0 * r).max(0.0)).unwrap(),
        &p,
        Transform::identity(),
        None,
    );

    // 四个角块位置写入 clip mask（mask 每次复用，先清零）
    clip.data_mut().fill(0);
    let mut pb = PathBuilder::new();
    pb.push_rect(Rect::from_xywh(x, y, r, r).unwrap());
    pb.push_rect(Rect::from_xywh(x + w - r, y, r, r).unwrap());
    pb.push_rect(Rect::from_xywh(x, y + h - r, r, r).unwrap());
    pb.push_rect(Rect::from_xywh(x + w - r, y + h - r, r, r).unwrap());
    let blocks = pb.finish().unwrap();
    clip.fill_path(&blocks, FillRule::Winding, false, Transform::identity());

    let mut pa = Paint::default();
    pa.set_color_rgba8(c.0, c.1, c.2, 255);
    pa.blend_mode = BlendMode::Source;
    pa.anti_alias = true;
    let rr = rounded_rect(x, y, w, h, r);
    px.fill_path(&rr, &pa, FillRule::Winding, Transform::identity(), Some(clip));
}
