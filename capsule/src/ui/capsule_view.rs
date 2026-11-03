//! CapsuleView：悬浮胶囊。
//! 收起：两行实时数据（行1 CPU/GPU/内存，行2 硬盘/网络）；
//! 展开：顶栏仅显 NAS 主机名，下方为更宽的单列面板（顶栏右对齐）——
//! CPU/内存/GPU 环形仪表、存储状态列表卡、硬盘读写列表卡、网卡心电图卡。
//! 展开/收起带高度缓动 + 面板淡入动效。

use super::View;
use crate::config::CardVis;
use crate::data::{ConnStatus, Disk, Frame, Snapshot, WM_APP_PANEL};
use crate::fmt;
use crate::font::Font;
use std::collections::{HashMap, VecDeque};
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tiny_skia::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

#[path = "panel.rs"]
mod panel;
use panel::*;

#[path = "big.rs"]
mod big;
pub(crate) use big::big_panel_height;
use big::*;

#[path = "pill.rs"]
mod pill;
#[path = "rings.rs"]
mod rings;
pub(crate) use rings::draw_rings;

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
/// 固定到任务栏时标题行（状态灯/名称/齿轮）在面板内的纵向中心：
/// PANEL_PAD(14) + FONT_SIZE_SMALL(12.5) / 2 取整。设置窗按它对齐齿轮，减少鼠标移动。
pub(crate) const HEADER_ROW_CY: i32 = 20;
const GAP: u32 = 8;
/// 展开态顶栏右侧设置齿轮的边长（像素）
const GEAR_SIZE: f32 = 16.0;

/// 收起态三圆环布局：半径、描边、中心 y、三个圆心 x。
/// 环高与任务栏等高（48），三环加大后数值与标注都收进环内。
const RING_R: f32 = 21.0;
const RING_STROKE: f32 = 5.0;
const RING_CY: f32 = 24.0;
const RING_CX: [f32; 3] = [24.0, 95.0, 166.0];
const RING_LABEL_FS: f32 = 7.5;
const RING_VALUE_FS: f32 = 12.5;
const RING_NET_FS: f32 = 11.5;

/// 超大卡片（贴屏幕侧边滑出）：宽 660，四块全宽图表，高度由工作区高度决定（上下各留 12），
/// 由 app 通过 `View::set_big_geometry` 下发。
pub(crate) const BIG_W: u32 = 660;
const BIG_PAD: f32 = 24.0;
const BIG_R: f32 = 12.0;
/// 大卡内每张设备卡的固定高度（勾选卡片数变化时不再拉伸卡片，面板高度随之变矮/变高）
pub(crate) const BIG_CARD_H: f32 = 152.0;
/// 大卡标题行高度（状态灯 + 名称/IP + 齿轮）
const BIG_HEADER_H: f32 = 40.0;
/// 大卡内容宽（660 − 左右 24）
const BIG_INNER_W: f32 = BIG_W as f32 - BIG_PAD * 2.0;
/// 四块图表卡：高度按面板剩余空间均分（上下 24 边距 + 40 标题行 + 3 个 14 间距之后平分）
const BIG_BLOCK_GAP: f32 = 14.0;
/// 超大卡片底面：不透明（小面板的 BG 带 242 的透明感，大面积看得出背后窗口的文字）
const BIG_BG: (u8, u8, u8, u8) = (246, 248, 251, 255);
/// 大卡内每块图表各自一张卡片底（不透明）
const BIG_CARD_BG: (u8, u8, u8, u8) = (255, 255, 255, 255);
/// 大卡右栏统计卡的底色：比卡片底稍灰一层，用来分组（与网络卡右栏一致）
const BIG_SUB_BG: (u8, u8, u8, u8) = (244, 247, 251, 255);
/// 大卡内卡片的内边距
const BIG_CARD_PAD: f32 = 14.0;

/// 收起态药丸胶囊（未固定）布局：图标边长、两行中线、三列左缘。
const PILL_ICON: f32 = 11.0;
const PILL_ROW_CY: [f32; 2] = [8.0, 24.0];
const PILL_COL_X: [f32; 3] = [14.0, 96.0, 180.0];
/// 竖排（细长条）胶囊：三环纵向堆叠，卡片贴着环收紧（左右各 3.75，环间与上下各 8）。
/// 环外径 48.5（2×22 + 4.5），三环总高 145.5。
pub(crate) const PILL_V_W: u32 = 56;
pub(crate) const PILL_V_H: u32 = 178;
/// 任务栏内三环子窗口的尺寸：宽 = 三环排布宽度，高 = 与任务栏等高（环与文字都收在条内）。
pub(crate) const RINGS_W: u32 = 190;
pub(crate) const RINGS_H: u32 = 48;
const RING_V_R: f32 = 22.0;
const RING_V_STROKE: f32 = 4.5;
const RING_V_CX: f32 = PILL_V_W as f32 / 2.0;
const RING_V_CY: [f32; 3] = [32.25, 88.75, 145.25];
/// 竖排环内文字比横排三环大且更粗（横排三环：数值 12.5 / 标注 7.5 / 网络 11.5，常规字重）
const RING_V_VALUE_FS: f32 = 12.5;
const RING_V_LABEL_FS: f32 = 8.5;
const RING_V_NET_FS: f32 = 11.5;

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
const DUAL_GAUGE_H: f32 = 116.0; // 单大环卡（内存 / 显卡）
const CPU_CARD_H: f32 = 132.0; // CPU 双环仪表卡（更高，环更大）
const BAR_BASE: f32 = 40.0; // 横条卡标题区+底部留白
const BAR_ROW_H: f32 = 48.0; // 每块盘：信息行 + 用量横条
const CHART_H: f32 = 132.0; // 心电图卡

// 卷卡（卷名 + 已用/总量 + 用量进度条）
const VOL_ROW_H: f32 = 42.0;

// 网卡心电图：保留样本数
const NET_HIST: usize = 60;

// 字号：GDI 只支持整数字号（select() 里按 round 收敛），且网格对齐会把笔画填满像素，
// 同样字号比轮廓光栅化"胖"一档——所以整体比轮廓时代的标称值低 3px 校准。
const FONT_SIZE: f32 = 11.0;
const FONT_SIZE_SMALL: f32 = 10.5;
const FONT_SIZE_TITLE: f32 = 13.5; // 卡片标题字号（CPU/内存/网络/硬盘读写/存储状态）
const FONT_SIZE_HEADER: f32 = 13.0; // NAS 名字号：未固定顶栏与固定/竖排的面板标题行统一用这一档
/// 仪表卡环心数值字号：CPU/显卡的双环与内存的大环**统一**用这一档，避免同屏两种环数值大小不一。
const GAUGE_VALUE_FS: f32 = 16.0;
/// 仪表卡环内标注字号（"使用率"/"温度"）
const GAUGE_CAP_FS: f32 = 10.0;
/// 卡片右上角小字相对标题基线的抬升量：小字与标题**共用同一条基线**时视觉中心偏低
/// （汉字在基线上方占的字面高度明显大于拉丁字母），看着标题"高了一截"。
/// 抬 2px 让两者视觉同心。
const NOTE_LIFT: f32 = 2.0;

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

/// 暗色主题下文字统一走中性色（随后由 apply_dark 提成纯白），亮色主题下保留语义色。
/// 环内/卡片里的数值本来靠颜色表达分级，但产品口径是"暗色下所有文字都是白的"，
/// 所以这里在**画字时**做替换——像素映射做不到（分不清白色文字和白色卡片）。
fn text_c(dark: bool, semantic: (u8, u8, u8, u8)) -> (u8, u8, u8, u8) {
    if dark {
        FG
    } else {
        semantic
    }
}

/// 派发给后台线程的面板构建请求：只携带 Arc 快照指针与少量小字段，
/// UI 线程发送时不阻塞、不复制快照。
struct BuildReq {
    seq: u64,
    conn: ConnStatus,
    ph: u32,
    /// 超大卡片：整层按 w×ph 构建，两列排版
    big: bool,
    w: u32,
    has_data: bool,
    /// 暗色主题：面板里的文字随主题取色（图形仍按亮色彩色画）
    dark: bool,
    /// 标题行放面板底部（固定到任务栏且任务栏在下方，标题行贴任务栏一侧）
    header_bottom: bool,
    /// 卡片显示开关：与 UI 线程算高度用的是同一份，避免高度与内容错位
    cards: CardVis,
    snap: Arc<Snapshot>,
}

/// 后台线程任务：展开时构建整层面板；收起时只推进心电图历史（展开即有满图）。
enum PanelJob {
    Build(BuildReq),
    Advance {
        seq: u64,
        has_data: bool,
        snap: Arc<Snapshot>,
    },
    /// UI 归还的旧面板像素缓冲，供下一轮构建复用（稳态零大内存分配）
    Recycle(Pixmap),
}

/// "10-18" → "10月18日"（NAS 只发月-日，最近 30 天不需要年份）。
fn fmt_date_cn(s: &str) -> String {
    match s.split_once('-') {
        Some((m, d)) => format!(
            "{}月{}日",
            m.trim_start_matches('0'),
            d.trim_start_matches('0')
        ),
        None => s.to_string(),
    }
}

/// 大卡网络卡右栏柱状图的几何（构建线程算好，UI 线程做悬停命中与信息栏绘制）。
#[derive(Clone, Copy)]
struct BarsGeom {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    /// 信息栏文字的基线 y
    line_y: f32,
    count: usize,
}

/// 后台线程构建完成的展开面板整层（大底色 + 卡片底/描边 + 标题状态 + 全部正文）。
/// 一律按亮色画；暗色主题在上屏前由 apply_dark 统一转换，面板缓存因此与主题无关。
struct BuiltPanel {
    seq: u64,
    conn: ConnStatus,
    /// 面板像素宽（小面板 320 / 超大卡片 660）
    w: u32,
    ph: u32,
    /// 是否为超大卡片层
    big: bool,
    /// 该面板是按哪个主题画的（切主题要重建）
    dark: bool,
    header_bottom: bool,
    /// 该面板是按哪一份卡片开关画出来的（开关变了要重建，哪怕高度恰好没变）
    cards: CardVis,
    /// 网络卡柱状图几何（无网络卡时为 None）
    bars: Option<BarsGeom>,
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
    /// 顶栏圆角条缓存（320×52）：宽度动画期间整层 blit，
    /// 超出可视宽的部分不会被上屏，省掉每帧抗锯齿 fill_path
    bar: Option<Pixmap>,
    /// 已派发请求的键 (seq, conn, ph, header_bottom, cards, dark, big, w)：动画帧键不变，避免重复派发
    sent_key: Option<(u64, ConnStatus, u32, bool, CardVis, bool, bool, u32)>,
    /// 收起态已推进心电图历史的数据版本（与构建线程 last_seq 各自去重）
    last_adv_seq: u64,
    /// 展开态顶栏右侧的设置齿轮图标（渲染失败为 None，自动隐藏按钮）
    gear: Option<Pixmap>,
    /// 未固定时药丸胶囊用的四个线性图标
    icons: crate::icons::Icons,
    /// 标题行设置齿轮在窗口内的位置（左上角）；供命中测试复用
    gear_pos: (f32, f32),
    /// 网络卡柱状图当前悬停的柱子下标（None = 未悬停）
    hover_bar: Option<usize>,
    /// 任务栏高度（用于垂直居中对齐，0 表示未知）
    taskbar_h: i32,
    /// 鼠标悬停态：圆环描边加亮
    hovered: bool,
    /// 网络环兜底刻度（会话峰值 rx/tx，字节/秒）：网卡协商速率读不到时（WiFi 等）
    /// 用峰值做满环基准，按帧缓慢衰减，避免历史尖峰长期钉住刻度
    net_peak: (u64, u64),
    /// 超大卡片目标几何：(宽, 高, 是否贴屏幕左侧)。由 app 按工作区下发。
    big_geom: (u32, u32, bool),
}

/// 卡片正文。
enum Body {
    /// 双环仪表：名称（可带小字明细）在左，右侧数据环（CPU / 显卡：使用率、温度）。
    /// 读不到的指标不放环（连标注一起省掉），环数可为 1 或 2，右对齐排布。
    DualGauge {
        title: String,
        subtitle: String,
        rings: Vec<RingSpec>,
    },
    /// 单大圆环仪表：名称（可带小字明细）在左，右侧一个大圆环
    BigGauge {
        title: String,
        subtitle: String,
        pct: Option<f32>,
    },
    /// 横向用量条：标题、右上注、每盘一条
    Bars {
        title: String,
        note: String,
        rows: Vec<BarRow>,
    },
    /// 卷树：标题、右上注、卷行（可点开看底层盘）
    Volumes {
        title: String,
        note: String,
        rows: Vec<VolRow>,
    },
    /// 图表卡：标题 + 右上角小字补充 + 曲线区 + 底部两段读数
    Chart {
        title: String,
        note: String,
        pts: Vec<(f32, f32, f32)>,
        detail1: String,
        detail2: String,
        c1: (u8, u8, u8, u8),
        c2: (u8, u8, u8, u8),
    },
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
    fn new(
        center: impl Into<String>,
        ratio: Option<f32>,
        color: (u8, u8, u8, u8),
        cap: impl Into<String>,
    ) -> Self {
        Self {
            center: center.into(),
            ratio,
            color,
            cap: cap.into(),
        }
    }
}

struct Card {
    body: Body,
    h: f32,
}

impl Card {
    /// 双环仪表卡：左侧名称（可带小字明细），右侧数据环；高度按卡片类型传入
    fn dual_gauge(
        title: impl Into<String>,
        subtitle: impl Into<String>,
        rings: Vec<RingSpec>,
        h: f32,
    ) -> Self {
        Self {
            body: Body::DualGauge {
                title: title.into(),
                subtitle: subtitle.into(),
                rings,
            },
            h,
        }
    }
    fn big_gauge(title: impl Into<String>, subtitle: impl Into<String>, pct: Option<f32>) -> Self {
        Self {
            body: Body::BigGauge {
                title: title.into(),
                subtitle: subtitle.into(),
                pct,
            },
            h: DUAL_GAUGE_H,
        }
    }
    fn bars(title: impl Into<String>, note: String, rows: Vec<BarRow>) -> Self {
        let h = BAR_BASE + rows.len() as f32 * BAR_ROW_H;
        Self {
            body: Body::Bars {
                title: title.into(),
                note,
                rows,
            },
            h,
        }
    }
    /// 卷卡高度：标题/留白 + 每卷一行。
    fn volumes(title: impl Into<String>, note: String, rows: Vec<VolRow>) -> Self {
        let h = BAR_BASE + rows.len() as f32 * VOL_ROW_H;
        Self {
            body: Body::Volumes {
                title: title.into(),
                note,
                rows,
            },
            h,
        }
    }
    fn chart(
        title: impl Into<String>,
        note: String,
        pts: Vec<(f32, f32, f32)>,
        detail1: String,
        detail2: String,
        c1: (u8, u8, u8, u8),
        c2: (u8, u8, u8, u8),
    ) -> Self {
        Self {
            body: Body::Chart {
                title: title.into(),
                note,
                pts,
                detail1,
                detail2,
                c1,
                c2,
            },
            h: CHART_H,
        }
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
            hover_bar: None,
            taskbar_h: 0,
            hovered: false,
            net_peak: (0, 0),
            big_geom: (BIG_W, 720, false),
        }
    }

    /// 收起态窗口尺寸：固定到任务栏＝任务栏内三环窗口；竖排＝细长条；否则＝药丸胶囊。
    fn collapsed_size(&self, frame: &Frame) -> (u32, u32) {
        if frame.pinned {
            (RINGS_W, RINGS_H)
        } else if frame.vertical {
            (PILL_V_W, PILL_V_H)
        } else {
            (PILL_W, PILL_H)
        }
    }

    /// 收起态分发：固定到任务栏画透明三圆环，未固定画药丸胶囊（横排药丸 / 竖排三环）。
    fn render_collapsed(&mut self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        if frame.pinned {
            self.render_rings(px, frame, font);
        } else if frame.vertical {
            self.render_rings_v(px, frame, font, 0.0, 0.0);
        } else {
            self.render_pill(px, frame, font);
        }
    }
}

/// 无变换、整体不透明的 Pixmap 合成快路径：源/目的同为预乘 RGBA，
/// 与 Source 模式 draw_pixmap（opacity=1）逐位等价，但只做逐行 memcpy，
/// 省掉 tiny-skia 逐像素 u16 混合——1MB 面板在低频小核上也只需数百微秒。
/// `dx` 为负（展开动画中面板相对窗口左移）时源从 `-dx` 列起取，右缘锚定；
/// 源纵向起点由 `sy` 指定（固定态面板只画动画高度那一段，取面板最下面一段）。
fn blit_rows(dst: &mut Pixmap, src: PixmapRef<'_>, dx: i32, dy: i32, sy: u32, w: u32, h: u32) {
    blit_rect(
        dst,
        src,
        dx,
        dy,
        dx.saturating_neg().max(0) as u32,
        sy,
        w,
        h,
    );
}

/// 显式指定源左上角 (sx, sy) 与目标左上角 (dx, dy) 的矩形拷贝：
/// 竖排面板从竖条一侧裁切生长时需要"源从哪一列起"与"落到哪一列"分别指定。
#[allow(clippy::too_many_arguments)]
fn blit_rect(
    dst: &mut Pixmap,
    src: PixmapRef<'_>,
    dx: i32,
    dy: i32,
    sx: u32,
    sy: u32,
    w: u32,
    h: u32,
) {
    use std::ptr;
    let dx = dx.max(0) as u32;
    let dy = dy.max(0) as u32;
    let w = w
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

/// 容量文本「已用 / 总量」（标准单位 B / KB / MB / GB / TB）；缺数据时给出「容量未知」。
fn capacity_text_unit(used: Option<u64>, total: Option<u64>) -> String {
    match (used, total) {
        (Some(u), Some(t)) if t > 0 => {
            format!("{} / {}", fmt::bytes_unit(u), fmt::bytes_unit(t))
        }
        _ => "容量未知".to_string(),
    }
}

/// 用量百分比（0~100）；总量为 0 或缺数据时返回 None。
fn ratio_of(used: Option<u64>, total: Option<u64>) -> Option<f32> {
    match (used, total) {
        (Some(u), Some(t)) if t > 0 => Some((u as f64 / t as f64 * 100.0) as f32),
        _ => None,
    }
}

/// 卡片 1px 描边（亮色主题下的一条细线；暗色主题由 apply_dark 反相）。
fn stroke_card(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32) {
    let line = rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, 10.0);
    let mut paint = Paint::default();
    paint.set_color_rgba8(CARD_LINE.0, CARD_LINE.1, CARD_LINE.2, CARD_LINE.3);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: 1.0,
        ..Default::default()
    };
    pm.stroke_path(&line, &paint, &stroke, Transform::identity(), None);
}

/// 暗色主题：把画面里的**中性色**映射到暗色（浅底↔深底、深字↔浅字），彩色（蓝/绿/橙/紫等状态色）
/// 原样保留。这样整套界面只按亮色构建一次，暗色在上屏前一次性转换，面板缓存与主题无关。
///
/// 映射：中性像素按"墨量" ink = a - c 走同一条单调曲线
/// `out = ink + ink²·3/(5a) + a·24/255`（背景 ink≈0，满覆盖 ink≈a）；
/// 抛物线项把文字/图标的中间覆盖率提亮（浅色字配深底需要比物理覆盖率更"实"），
/// 但又**不抹平过渡带**——这是暗色文字不显胖的关键。
///
/// 保留条件必须是「彩色 **且** 不浅」：
/// - 只按饱和度判会漏掉彩色图形与浅底混出来的过渡边——那些像素是"彩色与浅底混出来的浅色",
///   按逐通道映射会算成中间灰（≈90~135），在近黑底上就是一圈白毛边；
/// - 所以彩色像素统一走"反算覆盖率、落到同色暗调"的公式（见下），不再分档。
///
/// 规则汇总：
/// 1. **彩色**像素（状态色/曲线/彩色小字/描边过渡边）→ 由 ink 方向反推本色、按估计覆盖率
///    落到深底的**同色**（满覆盖 = 本色，半覆盖 = 同色暗调，不会压成灰）；
/// 2. **中性**像素（背景/描边/文字/图标）→ 墨量曲线：背景压到暗端保留层次，
///    文字/图标满覆盖落到纯白，中间覆盖率按抛物线过渡。
pub(crate) fn apply_dark(pix: &mut Pixmap) {
    /// 饱和度阈值：极差/最大值 超过它视为彩色
    const SAT_PCT: u32 = 40;
    /// 色度阈值：极差绝对值超过它也算彩色（捞低饱和的过渡边，见下）
    const TINT_MIN: u32 = 26;
    for px4 in pix.data_mut().as_chunks_mut::<4>().0 {
        let a = px4[3] as u32;
        if a == 0 {
            continue;
        }
        let (r, g, b) = (px4[0] as u32, px4[1] as u32, px4[2] as u32);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let chroma = mx - mn;
        // 彩色判定分两档：高饱和（色度占最大值 40%）与"低饱和但明显带色"。
        // 后者专门捞彩色描边与浅底混出来的过渡边——它们饱和度随覆盖率下降（绿 40% 覆盖
        // 时只剩 ~24%），若落进中性分支会被线性反相成一条亮粉边（红通道能到 110），
        // 在近黑底上比纯白边还扎眼。阈值 26 夹在 FG_DIM 次要文字（色度 20）之上，
        // 过渡边（色度 ≥ 30）之下，不误伤中性色。
        if chroma * 100 > mx * SAT_PCT || chroma * 255 > TINT_MIN * a {
            // 彩色像素：亮底彩墨 → 深底**同色**。
            //
            // 亮色像素可写成 p_i = 白 − t·(白 − color_i)，t = 覆盖率（彩色与白底混合时
            // 各通道的落差方向不变，所以 ink_i = a − p_i 的方向就是本色方向，是精确的）。
            // 深底上想要的是 out_i = 背景 + t·(color_i − 背景)，两式消去 color_i：
            //     out_i = 背景 + t·(a − 背景) − ink_i
            // 只有幅度要估：t ≈ max(ink) / 200（200 ≈ 典型饱和色"最亮通道相对白底"的落差），
            // 钳到 a。满覆盖的饱和色估到 t = a，反算回本色，误差 ≤ 3%。
            // 好处：单调、保色相、全覆盖率落到本色、半覆盖率落到同色暗调、永不低于背景。
            //
            // 这替代了原来的"深彩色原样保留 + 其余压成 30~60 暗灰"两档。压灰会把半覆盖的
            // 彩色像素（细折线、彩色小字、描边过渡带）抹成接近背景的灰：
            // 折线断成虚线、彩色图例字发花，都是它干的。
            let bg = (a * 24) >> 8;
            let span = a - bg;
            let mut m = 0u32;
            for ch in &px4[..3] {
                m = m.max(a.saturating_sub(*ch as u32));
            }
            let t = (m * a / 200).min(a);
            let base = bg + t * span / a;
            for ch in &mut px4[..3] {
                let ink = a.saturating_sub(*ch as u32);
                *ch = base.saturating_sub(ink).min(255) as u8;
            }
            continue;
        }
        // 中性像素：亮底的深字/深图标 → 深底的浅色。
        // ink = a - c 是"墨量"信号（0 = 纯背景，≈a = 满覆盖）：
        //   ink 小（背景/卡片/轨道/描边）→ 线性反相，原来的层次差照旧保留；
        //   ink 大（文字/图标）→ 再叠一个抛物线提亮，满覆盖落到纯白。
        // 关键：**不做硬提白**。之前用"亮度 ≤ 190 就置纯白"，会把整条抗锯齿过渡带压平成一个
        // 台阶——每个边缘像素都变实心白，字的墨量比亮色多 8%，常规字重的标题在暗色下看着
        // 就像粗体。换成这条保留过渡带的曲线后，暗色墨量与亮色基本持平（实测 152036 : 153414）。
        let lift = (a * 24) >> 8;
        for c in &mut px4[..3] {
            let ink = a.saturating_sub(*c as u32);
            let boost = ink * ink * 3 / (5 * a.max(1));
            *c = (ink + boost + lift).min(255) as u8;
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

/// 显卡名压缩：卡片左侧留给名称的宽度只有 ~110px（右边两个数据环占掉了大半），
/// 驱动报出的全名（如 `Intel(R) UHD Graphics 630`）放不下。这里去掉商标标记
/// （`(R)`/`(TM)`/`(C)`）与卡片语境下冗余的 `Graphics`，并折叠空白：
/// `Intel(R) UHD Graphics 630` → `Intel UHD 630`，`AMD Radeon(TM) Graphics` → `AMD Radeon`。
/// 仍然过长时由 `fit_text_bold` 兜底截断。
fn compact_gpu_name(name: &str) -> String {
    let mut s = name.to_string();
    for tag in [
        "(R)",
        "(r)",
        "(TM)",
        "(tm)",
        "(C)",
        "(c)",
        "®",
        "™",
        "©",
        " Corporation",
        " Inc.",
        " Ltd.",
    ] {
        if s.contains(tag) {
            s = s.replace(tag, "");
        }
    }
    s = s.replace("Graphics ", "").replace(" Graphics", "");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 显卡名拆成「厂商 + 型号」，卡片左上角显示厂商、下面一行显示型号：
/// `Intel(R) UHD Graphics 630` → `("Intel", "UHD 630")`；
/// `AMD Radeon(TM) Graphics` → `("AMD", "Radeon")`；
/// `NVIDIA GeForce RTX 3060` → `("NVIDIA", "GeForce RTX 3060")`。
/// 只有一个词时（如 `i915`）不拆，整个当厂商、型号留空。
fn split_gpu_name(name: &str) -> (String, String) {
    let s = compact_gpu_name(name);
    let mut it = s.split_whitespace();
    let vendor = it.next().unwrap_or("").to_string();
    let model = it.collect::<Vec<_>>().join(" ");
    (vendor, model)
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
fn draw_ring(
    px: &mut Pixmap,
    cx: f32,
    cy: f32,
    r: f32,
    width: f32,
    pct: Option<f32>,
    c: (u8, u8, u8, u8),
) {
    let mut stroke = |path: &Path, col: (u8, u8, u8, u8), width: f32| {
        let mut paint = Paint::default();
        paint.set_color_rgba8(col.0, col.1, col.2, col.3);
        paint.anti_alias = true;
        let s = Stroke {
            width,
            line_cap: LineCap::Round,
            ..Default::default()
        };
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

/// 上下半环：上半＝通道一（如上行）、下半＝通道二（如下行），各自按百分比填充。
/// pct 为 None 表示未知（只画轨道）；两半都从左侧水平点起、绕向另一侧。
#[allow(clippy::too_many_arguments)]
fn draw_ring_split(
    px: &mut Pixmap,
    cx: f32,
    cy: f32,
    r: f32,
    width: f32,
    upper: Option<f32>,
    upper_c: (u8, u8, u8, u8),
    lower: Option<f32>,
    lower_c: (u8, u8, u8, u8),
) {
    let mut stroke = |path: &Path, col: (u8, u8, u8, u8), width: f32| {
        let mut paint = Paint::default();
        paint.set_color_rgba8(col.0, col.1, col.2, col.3);
        paint.anti_alias = true;
        let s = Stroke {
            width,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        px.stroke_path(path, &paint, &s, Transform::identity(), None);
    };
    // 轨道：整圈
    if let Some(p) = arc_path(cx, cy, r, -PI / 2.0, -PI / 2.0 + TAU) {
        stroke(&p, TRACK, width);
    }
    // 上半环：左侧水平点 → 顶部 → 右侧（角度 PI → 2PI 递增）
    // 速率未知（None）时也画成"淡色的半环"，否则整只环只剩一圈浅灰轨道，
    // 在浅底/深底上都不易分辨，看起来像"没有这个环"。
    match upper {
        Some(v) => {
            let v = (v / 100.0).clamp(0.0, 1.0);
            if v > 0.005 {
                if let Some(p) = arc_path(cx, cy, r, PI, PI + PI * v) {
                    stroke(&p, upper_c, width);
                }
            }
        }
        None => {
            if let Some(p) = arc_path(cx, cy, r, PI, PI + PI) {
                stroke(&p, dim_alpha(upper_c, 110), width);
            }
        }
    }
    // 下半环：左侧水平点 → 底部 → 右侧（角度 PI−PI·v → PI 递增）
    match lower {
        Some(v) => {
            let v = (v / 100.0).clamp(0.0, 1.0);
            if v > 0.005 {
                if let Some(p) = arc_path(cx, cy, r, PI - PI * v, PI) {
                    stroke(&p, lower_c, width);
                }
            }
        }
        None => {
            if let Some(p) = arc_path(cx, cy, r, PI, PI + PI) {
                stroke(&p, dim_alpha(lower_c, 110), width);
            }
        }
    }
}

/// 同色但更淡的版本（用于"无数据"时的淡色提示）：只换 alpha。
fn dim_alpha(c: (u8, u8, u8, u8), a: u8) -> (u8, u8, u8, u8) {
    (c.0, c.1, c.2, a)
}

/// 使用率环：环心百分比 + 分级色；无数据只画轨道。
fn pct_ring(pct: Option<f32>) -> RingSpec {
    RingSpec::new(
        pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round())),
        pct,
        pct.map_or(ACCENT, usage_color),
        "使用率",
    )
}

/// 温度环：30~90°C 映射到整圈，颜色按 temp_color 分级；无数据只画轨道。
fn temp_ring(t: Option<f32>) -> RingSpec {
    match t {
        Some(v) => RingSpec::new(
            format!("{v:.0}°"),
            Some(((v - 30.0) / 60.0).clamp(0.0, 1.0) * 100.0),
            temp_color(v),
            "温度",
        ),
        None => RingSpec::new("--°", None, ACCENT, "温度"),
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

/// 深色任务栏上的环内文字：把分级色提亮一档（保留色相），保证 12.5px 小字在深底可读。
/// 只用于文字；环描边仍用原始分级色，避免整环发白、丢掉分级感。
fn ring_text_color(c: (u8, u8, u8, u8)) -> (u8, u8, u8, u8) {
    match c {
        LEVEL_GREEN => (74, 222, 128, c.3),
        WARN => (251, 146, 60, c.3),
        LEVEL_RED => (248, 113, 113, c.3),
        ACCENT => (96, 165, 250, c.3),
        _ => c,
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
/// 每整圈 180 段：半径 20+ 时弦到弧的偏差 < 0.01px，抗锯齿描边看不出多边折角。
fn arc_path(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Option<Path> {
    let steps = (((a1 - a0) / TAU) * 180.0).ceil().max(2.0) as usize;
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

/// 折线/面积图：基线 + 两条折线（rx 蓝 / tx 橙，或 读绿 / 写蓝），亮色下带半透明面积填充。
/// 暗色主题有两条专用处理，都是因为面板"按亮色构建 → 整体反相映射"这个前提：
/// - **不画面积填充**：填充 alpha 只有 c.3/6≈42，预乘后每通道只剩个位数，
///   映射时色相全丢，在深底上就是一块边界生硬的泥灰板（亮色下它是好看的浅色衬底）。
/// - **基线换色**：映射是单调反相的（源色越亮 → 暗色里越暗），TRACK(226) 映射后≈55，
///   在近黑卡片上比数据线还抢；换成 240 让它落到≈38，比卡片(≈23)略亮，够当地板不抢线。
/// 另外暗色下线宽加一档补回存在感。**不要**去"提亮线色"：apply_dark 判定"彩色本体"的
/// 条件是最暗通道 ≤ 70，绿色/橙色提亮后最暗通道越线，整条线会被当成过渡边压成灰色。
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
    dark: bool,
) {
    let base = if dark { (240, 240, 240, 255) } else { TRACK };
    fill_round(px, cx, cy + ch, cw, 1.0, 0.0, base);
    let (w_main, w_sub) = if dark {
        (1.6f32, 1.4f32)
    } else {
        (1.4f32, 1.0f32)
    };
    for (comp, c, width) in [(1usize, c1, w_main), (2, c2, w_sub)] {
        // 下方面积填充（仅亮色；暗色下低 alpha 彩色会映射成泥灰，见函数头说明）+ 上方折线
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
        // 填充面积（暗色下跳过）
        if !dark {
            if let Some(path) = pb_fill.finish() {
                let mut paint = Paint::default();
                paint.set_color_rgba8(c.0, c.1, c.2, (c.3 / 6).max(8));
                paint.anti_alias = true;
                px.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
        // 描折线
        if let Some(path) = pb_line.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(c.0, c.1, c.2, c.3);
            // 抗锯齿两个主题都开：暗色下彩色分支会按覆盖率把半覆盖像素落成同色暗调，
            // 线是连续且平滑的（早先这里必须关 AA，是因为那时半覆盖像素被压成接近背景的灰，
            // 线会断成虚线）。
            paint.anti_alias = true;
            let stroke = Stroke {
                width,
                ..Default::default()
            };
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

/// 把标题按空格折成 2~`max_lines` 行，每行都塞得进 `avail`（按粗体测量）。
/// 放不下、或某个单词自己就超宽、或行数超限时返回 None，交回调用方的降号/截断逻辑。
/// 用途：显卡名（`Intel UHD 630`）在 ~110px 的窄栏里一行放不下，折两行比缩字号清楚。
fn wrap_title_bold(
    font: &Font,
    size: f32,
    text: &str,
    avail: f32,
    max_lines: usize,
) -> Option<Vec<String>> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in words {
        let cand = if cur.is_empty() {
            w.to_string()
        } else {
            format!("{cur} {w}")
        };
        if font.measure_bold(size, &cand) <= avail {
            cur = cand;
            continue;
        }
        if cur.is_empty() || font.measure_bold(size, w) > avail {
            return None; // 第一个词就超宽，或下一个词自己超宽
        }
        lines.push(std::mem::take(&mut cur));
        cur = w.to_string();
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() < 2 || lines.len() > max_lines {
        return None;
    }
    Some(lines)
}

/// 标题行连接状态：短标签 + 文字颜色 + 状态灯颜色（在线为绿灯）。
fn conn_badge(conn: &ConnStatus, dark: bool) -> (String, Color, (u8, u8, u8, u8)) {
    let (_, text_color) = status_text(conn, dark);
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
        px.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

impl View for CapsuleView {
    fn render(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        vis: (u32, u32),
    ) -> bool {
        // 重建每行不透明度表（默认全不透明），长度只覆盖可视行
        self.row_fade.clear();
        self.row_fade.resize(vis.1 as usize, 255);

        if self.anim > 0.0005 || (frame.pinned && self.open) {
            if frame.big {
                self.render_big(px, frame, font, self.anim, vis.0, vis.1)
            } else {
                self.render_expanded(px, frame, font, self.anim, vis.0)
            }
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
        if !open {
            self.hover_bar = None;
        }
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
        if frame.big {
            return self.big_geom.1.max(1);
        }
        let ph = panel_height(frame.has_data, frame.snap, frame.cards);
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
        // 超大卡片：展开期间窗口一次到位（贴边 660×工作区高−24），动画只让内容滑入
        if frame.big {
            return if self.anim <= 0.0005 && !self.open {
                self.collapsed_size(frame)
            } else {
                (self.big_geom.0.max(1), self.big_geom.1.max(1))
            };
        }
        // 固定到任务栏：窗口尺寸一次到位（展开动画只动内容，不再每帧改窗口几何，
        // 否则 SetWindowPos 的合成开销会让动画抖动）
        if frame.pinned {
            return (PANEL_W, expanded_h(frame));
        }
        let (cw, ch) = self.collapsed_size(frame);
        if frame.vertical {
            // 竖排：窗口同样一次到位（竖条 + 面板并排），展开动画只把面板从侧向"推"出来，
            // 不做窗口尺寸插值——横向每帧改窗口宽度会和固定态一样带来合成抖动。
            return if self.anim <= 0.0005 {
                (cw, ch)
            } else {
                (PILL_V_W + PANEL_W, expanded_h(frame))
            };
        }
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
        if frame.big {
            return if self.anim <= 0.0005 && !self.open {
                self.collapsed_size(frame)
            } else {
                (self.big_geom.0.max(1), self.big_geom.1.max(1))
            };
        }
        // 与 frame_size 的竖排分支保持一致：动画尚未起步（anim==0）时缓冲也用收起态，
        // 否则展开首帧会出现"可视区 178 高、缓冲 52 高"的错配，blit 时切片越界。
        if self.anim <= 0.0005 {
            self.collapsed_size(frame)
        } else {
            // 展开目标态（含展开/收起动画过程）：一次分配到全展开尺寸，
            // 动画期间只改变可视窗口与逐行淡入，不再重建缓冲与 DIB。
            if frame.vertical {
                (PILL_V_W + PANEL_W, expanded_h(frame))
            } else {
                (PANEL_W, expanded_h(frame))
            }
        }
    }

    fn row_fade(&self) -> &[u8] {
        &self.row_fade
    }

    /// 超大卡片目标几何（宽、高、是否贴左侧），由 app 按工作区算好后下发。
    fn set_big_geometry(&mut self, w: u32, h: u32, left: bool) {
        if w > 0 && h > 0 {
            self.big_geom = (w, h, left);
        }
    }

    /// 设置齿轮命中测试。齿轮在面板标题行右侧，首帧就画在最终位置，
    /// 所以不要求展开动画结束——否则动画期间点齿轮会被当成「点面板」。
    fn hit_settings(&self, x: i32, y: i32) -> bool {
        if !self.open {
            return false;
        }
        let (gx, gy) = self.gear_pos;
        let pad = 6.0; // 命中区比图标略大，便于点按
        let xf = x as f32;
        let yf = y as f32;
        xf >= gx - pad && xf <= gx + GEAR_SIZE + pad && yf >= gy - pad && yf <= gy + GEAR_SIZE + pad
    }

    /// 柱状图悬停命中：返回柱子下标（不在柱图区域内为 None）。
    fn hit_bar(&self, x: i32, y: i32) -> Option<usize> {
        let geom = self.current.as_ref()?.bars?;
        let (xf, yf) = (x as f32, y as f32);
        if xf < geom.x || xf >= geom.x + geom.w || yf < geom.y - 2.0 || yf > geom.line_y {
            return None;
        }
        const GAP: f32 = 1.5;
        let bw = ((geom.w - GAP * (geom.count as f32 - 1.0)) / geom.count as f32).max(1.2);
        let i = ((xf - geom.x) / (bw + GAP)).floor() as usize;
        (i < geom.count).then_some(i)
    }

    /// 设置悬停柱子；返回是否有变化（调用方据此决定要不要重绘）。
    fn set_hover_bar(&mut self, v: Option<usize>) -> bool {
        if self.hover_bar == v {
            return false;
        }
        self.hover_bar = v;
        true
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

/// 连接状态文案与颜色。暗色主题下中性色统一取主前景色——apply_dark 只把"深于背景"的
/// 中性像素提白，中间色（如 FG_DIM 128,135,148）的抗锯齿像素会卡在阈值下方被反相成中灰。
fn status_text(conn: &ConnStatus, dark: bool) -> (String, Color) {
    match conn {
        ConnStatus::Online => {
            let c = text_c(dark, FG_DIM);
            ("等待数据…".into(), color(c.0, c.1, c.2, c.3))
        }
        ConnStatus::Connecting => (
            "正在连接 NAS…".into(),
            color(ACCENT.0, ACCENT.1, ACCENT.2, ACCENT.3),
        ),
        ConnStatus::Reconnect(n) => (
            format!("已断开，{n}s 后重连"),
            color(WARN.0, WARN.1, WARN.2, WARN.3),
        ),
        ConnStatus::AuthFailed => (
            "配对令牌错误".into(),
            color(LEVEL_RED.0, LEVEL_RED.1, LEVEL_RED.2, LEVEL_RED.3),
        ),
    }
}

/// 竖排细长条（宽 96）用的短状态文案：长句放不下。
fn status_text_short(conn: &ConnStatus, dark: bool) -> (String, Color) {
    match conn {
        ConnStatus::Online => {
            let c = text_c(dark, FG_DIM);
            ("等待数据".into(), color(c.0, c.1, c.2, c.3))
        }
        ConnStatus::Connecting => (
            "连接中…".into(),
            color(ACCENT.0, ACCENT.1, ACCENT.2, ACCENT.3),
        ),
        ConnStatus::Reconnect(n) => (
            format!("{n}s 后重连"),
            color(WARN.0, WARN.1, WARN.2, WARN.3),
        ),
        ConnStatus::AuthFailed => (
            "令牌错误".into(),
            color(LEVEL_RED.0, LEVEL_RED.1, LEVEL_RED.2, LEVEL_RED.3),
        ),
    }
}

fn disk_io_sum(s: &Snapshot) -> (u64, u64) {
    s.disks
        .as_ref()
        .map(|v| {
            v.iter().fold((0u64, 0u64), |(r, w), d| {
                (
                    r + d.read_bytes_per_sec.unwrap_or(0),
                    w + d.write_bytes_per_sec.unwrap_or(0),
                )
            })
        })
        .unwrap_or((0, 0))
}

pub(crate) fn net_io_sum(s: &Snapshot) -> (u64, u64) {
    s.network
        .as_ref()
        .map(|v| {
            v.iter().fold((0u64, 0u64), |(r, w), n| {
                (
                    r + n.rx_bytes_per_sec.unwrap_or(0),
                    w + n.tx_bytes_per_sec.unwrap_or(0),
                )
            })
        })
        .unwrap_or((0, 0))
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
    pb.finish()
        .unwrap_or_else(|| PathBuilder::from_rect(Rect::from_xywh(x, y, w, h).unwrap()))
}

/// 抗锯齿圆角矩形填充（半透明/磨砂路径用）。
fn fill_round(px: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: (u8, u8, u8, u8)) {
    let path = rounded_rect(x, y, w, h, r);
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.0, c.1, c.2, c.3);
    paint.anti_alias = true;
    px.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
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
    // 圆角抗锯齿像素必须走默认的 SourceOver：Source 会把覆盖率和上层卡片底一起写死成
    // 半透明像素，叠在已画好的面板底上时表现为四个角透出背后窗口（看着像黑角）。
    pa.anti_alias = true;
    let rr = rounded_rect(x, y, w, h, r);
    px.fill_path(
        &rr,
        &pa,
        FillRule::Winding,
        Transform::identity(),
        Some(clip),
    );
}

// 宣传视频的渲染代码已迁到同目录的 video_v2.rs / video_h.rs / video_full_old.rs，产品 UI 只留界面绘制
#[cfg(all(test, feature = "zzz_old_video"))]
#[path = "video_full_old.rs"]
mod video_full_old;

#[cfg(test)]
#[path = "video_v2.rs"]
mod video2;

#[cfg(test)]
#[path = "video_h.rs"]
mod video_h;
