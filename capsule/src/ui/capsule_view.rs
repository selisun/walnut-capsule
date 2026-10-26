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

    /// 未固定时的药丸胶囊：276×32 圆角底，两行 × 三列
    /// （行1 CPU/硬盘读/网络上行，行2 内存/硬盘写/网络下行）。
    fn render_pill(&self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        let Some(font) = font else { return };
        fill_round(px, 0.0, 0.0, PILL_W as f32, PILL_H as f32, PILL_R, BG);

        if !frame.has_data {
            let (text, c) = status_text(frame.conn, frame.dark);
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
        draw_pill_cell(
            px,
            font,
            self.icons.cpu.as_ref(),
            x0,
            cy0,
            None,
            &pill_pct(cpu),
            FG,
        );
        draw_pill_cell(
            px,
            font,
            self.icons.ram.as_ref(),
            x0,
            cy1,
            None,
            &pill_pct(mem),
            FG,
        );
        // 列2：硬盘读取（行1）/ 写入（行2），R 蓝、W 绿
        draw_pill_cell(
            px,
            font,
            self.icons.disk.as_ref(),
            x1,
            cy0,
            Some(("R", READ_BLUE)),
            &fmt::rate(dr),
            FG,
        );
        draw_pill_cell(
            px,
            font,
            self.icons.disk.as_ref(),
            x1,
            cy1,
            Some(("W", LEVEL_GREEN)),
            &fmt::rate(dw),
            FG,
        );
        // 列3：网络上行（行1）/ 下行（行2），UL 橙、DL 紫
        draw_pill_cell(
            px,
            font,
            self.icons.net.as_ref(),
            x2,
            cy0,
            Some(("UL", TX_LINE)),
            &fmt::rate(nw),
            FG,
        );
        draw_pill_cell(
            px,
            font,
            self.icons.net.as_ref(),
            x2,
            cy1,
            Some(("DL", DL_PURPLE)),
            &fmt::rate(nr),
            FG,
        );
    }

    /// 竖排收起态：三环纵向堆叠（CPU / 内存 / 网络），与任务栏三环同一套画法，
    /// 只是排列方向与尺寸不同。(ox, oy) 为左上角偏移——展开态要画在窗口侧边。
    fn render_rings_v(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        ox: f32,
        oy: f32,
    ) {
        // 协商速率读不到时（WiFi 网卡没有 sysfs speed）网络环改用会话峰值做满环基准
        let (rx, tx) = net_io_sum(frame.snap);
        let decay = |p: u64, v: u64| (p * 99 / 100).max(v);
        self.net_peak = (decay(self.net_peak.0, rx), decay(self.net_peak.1, tx));
        fill_round(px, ox, oy, PILL_V_W as f32, PILL_V_H as f32, PILL_R, BG);
        draw_rings_v(px, frame, font, ox, oy, self.net_peak);
    }

    fn render_rings(&mut self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>) {
        // 协商速率读不到时（WiFi 网卡没有 sysfs speed）网络环改用会话峰值做满环基准
        let (rx, tx) = net_io_sum(frame.snap);
        let decay = |p: u64, v: u64| (p * 99 / 100).max(v);
        self.net_peak = (decay(self.net_peak.0, rx), decay(self.net_peak.1, tx));
        draw_rings(px, frame, font, self.net_peak);
    }

    fn render_expanded(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        p: f32,
        vis_w: u32,
    ) -> bool {
        let ph = panel_height(frame.has_data, frame.snap, frame.cards);
        // 固定到任务栏且任务栏在下方时，标题行放面板底部（贴任务栏一侧）
        let header_bottom = frame.pinned && frame.expand_up;
        // 数据版本/连接状态/面板高度/标题行位置/卡片开关/主题变化时，把整层构建派发到后台线程；
        // 动画帧键不变，不重复派发。面板光栅化（2~4ms）完全离开 UI 关键路径。
        // 卡片开关必须进键：开关变了但总高度恰好相同时（如关 CPU 同时开显卡）也要重建。
        // 主题也必须进键：暗色下文字取白色，切主题要重画文字。
        let key = (
            frame.seq,
            frame.conn.clone(),
            ph,
            header_bottom,
            frame.cards,
            frame.dark,
            false,
            PANEL_W,
        );
        let up_to_date = self.current.as_ref().is_some_and(|c| {
            c.seq == frame.seq
                && c.conn == *frame.conn
                && c.ph == ph
                && !c.big
                && c.w == PANEL_W
                && c.header_bottom == header_bottom
                && c.cards == frame.cards
                && c.dark == frame.dark
        });
        if !up_to_date && self.sent_key.as_ref() != Some(&key) {
            let _ = self.req_tx.send(PanelJob::Build(BuildReq {
                seq: frame.seq,
                conn: frame.conn.clone(),
                ph,
                big: false,
                w: PANEL_W,
                has_data: frame.has_data,
                dark: frame.dark,
                header_bottom,
                cards: frame.cards,
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
        // 竖排时竖条与面板并排（面板顶边对齐竖条顶边，面板从竖条一侧推出）；
        // 其余未固定情况：顶栏贴窗口上沿、面板在其下方。
        let up = frame.expand_up;
        let pinned = frame.pinned;
        // 固定到任务栏时窗口本身就是面板，竖排设置不参与（收起态由任务栏三环承担）
        let vertical = frame.vertical && !frame.pinned;
        let vis_h = self.row_fade.len() as u32;
        let bar_y = 0.0f32;
        let y0 = if pinned || vertical {
            0i32
        } else {
            (CAPSULE_H + GAP) as i32
        };
        // 面板 blit 锚点：
        // - 固定态窗口已是全尺寸，面板只画“当前动画高度”那一段（向上展开时贴窗口底、
        //   源取面板最下面一段），其余留透明，靠内容生长成动画；
        // - 竖排窗口同样一次到位，面板靠“横向已推出宽度”裁切生长；
        // - 未固定横排：面板整幅贴在顶栏下方。
        let (panel_dx, panel_dy, panel_sx, panel_sy, panel_sw, panel_h) = if vertical {
            let grown = if p >= 0.999 {
                PANEL_W
            } else {
                (ease_motion(p) * PANEL_W as f32)
                    .round()
                    .clamp(0.0, PANEL_W as f32) as u32
            };
            if frame.panel_left {
                // 胶囊贴屏幕右缘：面板在竖条左侧，从贴竖条的一侧（面板右缘）向左推出
                let dx = PANEL_W - grown;
                (dx as i32, 0, dx, 0, grown, ph)
            } else {
                // 胶囊贴屏幕左缘：面板在竖条右侧，从面板左缘向右推出
                (PILL_V_W as i32, 0, 0, 0, grown, ph)
            }
        } else if pinned {
            let h = if p >= 0.999 {
                ph
            } else {
                let r = ease_motion(p);
                ((RINGS_H as f32) + (ph as f32 - RINGS_H as f32) * r)
                    .round()
                    .max(1.0) as u32
            };
            let h = h.min(ph);
            let (dy, sy) = if up {
                (vis_h as i32 - h as i32, ph - h)
            } else {
                (0, 0)
            };
            (0, dy, 0, sy, vis_w, h)
        } else {
            // 未固定横排：窗口向左加宽（右缘不动），面板内容右缘锚定、整体左移宽度差
            (
                vis_w as i32 - PANEL_W as i32,
                y0,
                PANEL_W - vis_w,
                0,
                vis_w,
                ph,
            )
        };

        // 面板区域随展开进度淡入。固定态/竖排面板与竖条并排，只对面板整块做淡入；
        // 未固定横排时顶栏保持不透明，只淡入其下方的面板区域。
        if p < 0.999 {
            // 内容在动画前段就基本不透明（约 45% 处到位），后段靠位移收尾，
            // 避免整段都在半透明里"发灰"。
            let a = ease_out_quad((p / 0.45).clamp(0.0, 1.0));
            let av = ((a * 255.0).round() as u8).max(if p > 0.001 { 1 } else { 0 });
            if pinned || up {
                for v in self.row_fade.iter_mut().take(ph as usize) {
                    *v = av;
                }
            } else if !vertical {
                for v in self.row_fade.iter_mut().skip(CAPSULE_H as usize) {
                    *v = av;
                }
            }
        }

        // 顶栏（横排，展开时与面板同宽）/ 竖条（竖排，贴在面板旁边）。
        // 圆角条按亮色调色板缓存为 320×52 像素层，动画中整层 blit
        // （超出可视宽的右段不会被 StretchDIBits 上屏）。
        // 固定在任务栏时顶栏不画底（透明融入任务栏）；竖排时窗口本身就是竖条，不画顶栏。
        let bar_w = vis_w;
        if vertical {
            let bx = if frame.panel_left {
                PANEL_W as f32
            } else {
                0.0
            };
            self.render_rings_v(px, frame, font, bx, bar_y);
        } else if !frame.pinned {
            if self.bar.is_none() {
                let mut bpm = Pixmap::new(PANEL_W, CAPSULE_H).unwrap();
                fill_round(
                    &mut bpm,
                    0.0,
                    0.0,
                    PANEL_W as f32,
                    CAPSULE_H as f32,
                    14.0,
                    BG,
                );
                self.bar = Some(bpm);
            }
            blit_rows(
                px,
                self.bar.as_ref().unwrap().as_ref(),
                0,
                bar_y as i32,
                0,
                vis_w,
                CAPSULE_H,
            );
        }

        if let Some(font) = font {
            // 固定到深色任务栏时顶栏文字改用浅色；暗色主题下次要文字也统一提白
            let light_on_dark = frame.pinned && !frame.taskbar_light;
            let (fg, dim) = if light_on_dark {
                (
                    color(PIN_FG.0, PIN_FG.1, PIN_FG.2, PIN_FG.3),
                    color(PIN_FG_DIM.0, PIN_FG_DIM.1, PIN_FG_DIM.2, PIN_FG_DIM.3),
                )
            } else {
                let d = text_c(frame.dark, FG_DIM);
                (color(FG.0, FG.1, FG.2, FG.3), color(d.0, d.1, d.2, d.3))
            };

            // 未固定横排：名称/IP 在顶栏（顶栏在面板区域之外，不会被面板 blit 覆盖）；
            // 固定到任务栏与竖排时改画在面板标题行（见帧尾），状态灯/状态/齿轮两者都在帧尾画
            if !frame.pinned && !vertical {
                let name = frame
                    .snap
                    .hostname
                    .as_deref()
                    .map(str::trim)
                    .filter(|h| !h.is_empty());
                let hbaseline =
                    bar_y + CAPSULE_H as f32 / 2.0 + font.ascent(FONT_SIZE_HEADER) * 0.36;
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
            blit_rect(
                px,
                c.pm.as_ref(),
                panel_dx,
                panel_dy,
                panel_sx,
                panel_sy,
                panel_sw,
                panel_h,
            );
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
                blit_rect(
                    px,
                    c.pm.as_ref(),
                    panel_dx,
                    panel_dy,
                    panel_sx,
                    panel_sy,
                    panel_sw,
                    panel_h,
                );
            }
        }

        // 标题行：状态灯 + 连接状态 + 设置齿轮（固定到任务栏/竖排时左侧还有 NAS 名称/IP）。
        // 固定态标题行贴任务栏一侧（任务栏在下方时贴面板底）；竖排与未固定横排都在面板顶部。
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
            // 竖排时面板在窗口里的横向偏移（竖条占另一侧），标题行随之平移
            let px0 = if vertical && !frame.panel_left {
                PILL_V_W as f32
            } else {
                0.0
            };
            let cy = baseline - font.ascent(FONT_SIZE_SMALL) * 0.34;
            // 齿轮贴面板右内边距；状态文字右对齐到齿轮左侧
            let gx = px0 + PANEL_W as f32 - PANEL_PAD - GEAR_SIZE;
            let gy = cy - GEAR_SIZE / 2.0;
            self.gear_pos = (gx, gy);
            let (text, sc, dc) = conn_badge(frame.conn, frame.dark);
            // 暗色下文字统一为白（状态灯仍用状态色）
            let tc = if frame.dark {
                color(FG.0, FG.1, FG.2, FG.3)
            } else {
                sc
            };
            let tw = font.measure(FONT_SIZE_SMALL, &text);
            let status_right = gx - 8.0;
            font.draw(px, status_right - tw, baseline, FONT_SIZE_SMALL, &text, tc);
            // 状态灯：连接状态色实心圆点，排在状态文字之前
            let dot_r = 3.0;
            let dot_cx = status_right - tw - 9.0 - dot_r;
            fill_dot(px, dot_cx, cy, dot_r, dc);
            if frame.pinned || vertical {
                // 固定到任务栏/竖排：名称/IP 排在状态灯之前（面板浅色底，用主题深色文字）
                let name = frame
                    .snap
                    .hostname
                    .as_deref()
                    .map(str::trim)
                    .filter(|h| !h.is_empty());
                let avail = (dot_cx - dot_r - 10.0 - px0 - PANEL_PAD).max(60.0);
                let d = text_c(frame.dark, FG_DIM);
                draw_title(
                    font,
                    px,
                    px0 + PANEL_PAD,
                    baseline,
                    avail,
                    (FONT_SIZE_HEADER, FONT_SIZE_SMALL),
                    (color(FG.0, FG.1, FG.2, FG.3), color(d.0, d.1, d.2, d.3)),
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

    /// 超大卡片：窗口几何一次到位，内容从贴边一侧滑入（只改 blit 源/目标偏移）。
    /// 标题行固定在大卡顶部，不随滑入平移。
    fn render_big(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        p: f32,
        vis_w: u32,
        vis_h: u32,
    ) -> bool {
        let (bw, bh, left) = self.big_geom;
        let bw = bw.max(PANEL_W);
        let bh = bh.max(320);

        let key = (
            frame.seq,
            frame.conn.clone(),
            bh,
            false,
            frame.cards,
            frame.dark,
            true,
            bw,
        );
        let up_to_date = self.current.as_ref().is_some_and(|c| {
            c.seq == frame.seq
                && c.conn == *frame.conn
                && c.ph == bh
                && c.big
                && c.w == bw
                && c.cards == frame.cards
                && c.dark == frame.dark
        });
        if !up_to_date && self.sent_key.as_ref() != Some(&key) {
            let _ = self.req_tx.send(PanelJob::Build(BuildReq {
                seq: frame.seq,
                conn: frame.conn.clone(),
                ph: bh,
                big: true,
                w: bw,
                has_data: frame.has_data,
                dark: frame.dark,
                header_bottom: false,
                cards: frame.cards,
                snap: frame.arc.clone(),
            }));
            self.sent_key = Some(key);
            // 稳态且动画已结束：等后台构建完成（WM_APP_PANEL）再整帧上屏
            if p >= 0.999 {
                return false;
            }
        }

        // 滑入：贴右侧时内容从右缘向左推入，贴左侧时从左缘向右推入
        let ease = if p >= 0.999 { 1.0 } else { ease_motion(p) };
        let shift = ((1.0 - ease) * bw as f32).round() as u32;
        let (dx, sx, sw) = if left {
            (0, shift, bw - shift)
        } else {
            (shift as i32, 0, bw - shift)
        };
        let bh = bh.min(vis_h.max(1));
        let sw = sw.min(vis_w.max(1));
        if sw > 0 {
            if let Some(c) = &self.current {
                blit_rect(px, c.pm.as_ref(), dx, 0, sx, 0, sw, bh);
            }
        }
        let mut swapped = false;
        while let Ok(panel) = self.panel_rx.try_recv() {
            if let Some(old) = self.current.replace(panel) {
                let _ = self.req_tx.send(PanelJob::Recycle(old.pm));
            }
            swapped = true;
        }
        if swapped && sw > 0 {
            if let Some(c) = &self.current {
                blit_rect(px, c.pm.as_ref(), dx, 0, sx, 0, sw, bh);
            }
        }

        if let Some(font) = font {
            let row_cy = BIG_PAD + BIG_HEADER_H / 2.0;
            let baseline = row_cy + font.ascent(FONT_SIZE_SMALL) * 0.36;
            let gx = bw as f32 - BIG_PAD - GEAR_SIZE;
            let gy = row_cy - GEAR_SIZE / 2.0;
            self.gear_pos = (gx, gy);

            let (text, sc, dc) = conn_badge(frame.conn, frame.dark);
            let tc = if frame.dark {
                color(FG.0, FG.1, FG.2, FG.3)
            } else {
                sc
            };
            let tw = font.measure(FONT_SIZE_SMALL, &text);
            let status_right = gx - 12.0;
            font.draw(px, status_right - tw, baseline, FONT_SIZE_SMALL, &text, tc);
            let dot_r = 3.0;
            let dot_cx = status_right - tw - 9.0 - dot_r;
            fill_dot(px, dot_cx, row_cy, dot_r, dc);

            let name = frame
                .snap
                .hostname
                .as_deref()
                .map(str::trim)
                .filter(|h| !h.is_empty());
            let avail = (dot_cx - dot_r - 12.0 - BIG_PAD).max(80.0);
            let d = text_c(frame.dark, FG_DIM);
            draw_title(
                font,
                px,
                BIG_PAD,
                baseline,
                avail,
                (FONT_SIZE_HEADER, FONT_SIZE_SMALL),
                (color(FG.0, FG.1, FG.2, FG.3), color(d.0, d.1, d.2, d.3)),
                name,
                frame.nas_host.trim(),
            );

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

            // 网络卡柱图信息栏：未悬停显示「今日 / 30 日」合计，悬停某根柱子显示该日数据
            if let Some(geom) = self.current.as_ref().and_then(|c| c.bars) {
                if let Some(tf) = frame.snap.traffic.as_ref() {
                    let idx = self.hover_bar.filter(|i| *i < geom.count);
                    let d = text_c(frame.dark, FG_DIM);
                    let dc = color(d.0, d.1, d.2, d.3);
                    if let Some(i) = idx {
                        // 悬停柱高亮：整列铺一层浅色底（不遮相邻柱），再描一圈强调色边框
                        const GAP: f32 = 1.5;
                        let bw = ((geom.w - GAP * (geom.count as f32 - 1.0)) / geom.count as f32)
                            .max(1.2);
                        let bx = geom.x + i as f32 * (bw + GAP);
                        let mut wash = Paint::default();
                        wash.set_color_rgba8(ACCENT.0, ACCENT.1, ACCENT.2, 26);
                        if let Some(r) =
                            Rect::from_xywh(bx - 1.0, geom.y - 2.0, bw + 2.0, geom.h + 2.0)
                        {
                            px.fill_rect(r, &wash, Transform::identity(), None);
                        }
                        let outline =
                            rounded_rect(bx - 1.5, geom.y - 2.0, bw + 3.0, geom.h + 3.0, 1.5);
                        let mut paint = Paint::default();
                        paint.set_color_rgba8(ACCENT.0, ACCENT.1, ACCENT.2, 255);
                        paint.anti_alias = true;
                        let stroke = Stroke {
                            width: 1.5,
                            ..Default::default()
                        };
                        px.stroke_path(&outline, &paint, &stroke, Transform::identity(), None);
                    }
                    let text = match idx {
                        Some(i) => {
                            let date = tf
                                .dates
                                .get(i)
                                .map(|s| fmt_date_cn(s))
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| format!("第 {} 天", i + 1));
                            let rx = tf.days_rx.get(i).copied().unwrap_or(0);
                            let tx = tf.days_tx.get(i).copied().unwrap_or(0);
                            format!(
                                "{date}  下行 {} · 上行 {}",
                                fmt::bytes_unit(rx),
                                fmt::bytes_unit(tx)
                            )
                        }
                        None => format!(
                            "今日 {} / {} · 30 日 {} / {}",
                            fmt::bytes_unit(tf.rx_today),
                            fmt::bytes_unit(tf.tx_today),
                            fmt::bytes_unit(tf.rx_days30),
                            fmt::bytes_unit(tf.tx_days30)
                        ),
                    };
                    font.draw(px, geom.x, geom.line_y, 9.5, &text, dc);
                }
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

/// 组装设备卡片：CPU/内存/GPU 仪表、网卡心电图、硬盘读写、存储状态。
/// 心电图历史由后台构建线程持有（UI 线程不再触碰）。
/// `cards` 为用户的显示开关；与 card_heights 用同一份开关，保证高度与内容对齐。
fn build_cards(
    s: &Snapshot,
    net_hist: &HashMap<String, VecDeque<(u64, u64)>>,
    disk_hist: &VecDeque<(u64, u64)>,
    cards: CardVis,
) -> Vec<Card> {
    let mut cards_out = Vec::new();
    let disks = s.disks.as_deref().unwrap_or(&[]);
    let volumes = s.volumes.as_deref().unwrap_or(&[]);
    let has_storage = !disks.is_empty() || !volumes.is_empty();

    // 顺序跟随设置里的勾选先后（最近勾选的排最后）
    for idx in cards.ordered() {
        match idx {
            // CPU：使用率 / 温度各自可读才画环；两项都读不到时整张卡不显示
            0 => {
                if let Some(c) = &s.cpu {
                    let usage = c.usage.map(|v| v as f32);
                    let temp = c.temp.map(|v| v as f32);
                    let mut rings = Vec::new();
                    if usage.is_some() {
                        rings.push(pct_ring(usage));
                    }
                    if temp.is_some() {
                        rings.push(temp_ring(temp));
                    }
                    if !rings.is_empty() {
                        cards_out.push(Card::dual_gauge("CPU", "", rings, CPU_CARD_H));
                    }
                }
            }
            // 内存
            1 => {
                if let Some(m) = &s.memory {
                    let usage = m.usage.map(|v| v as f32);
                    let subtitle = match (m.used, m.total) {
                        (Some(u), Some(t)) => format!("{} / {}", fmt::bytes(u), fmt::bytes(t)),
                        _ => String::new(),
                    };
                    if usage.is_some() || !subtitle.is_empty() {
                        cards_out.push(Card::big_gauge("内存", subtitle, usage));
                    }
                }
            }
            // 显卡：使用率环 + 温度环（有独显看独显，没有则看核显）。
            // 卡片左侧分两行：左上角厂商（Intel / AMD），下面一行具体型号（UHD 630）。
            2 => {
                let gpus = s.gpus.as_deref().unwrap_or(&[]);
                // 单卡时不带 `GPU0 · ` 前缀：左侧栏只有 ~110px 宽，前缀会吃掉近一半
                let multi = gpus.len() > 1;
                for g in gpus {
                    let (vendor, model) = split_gpu_name(&g.name);
                    let vram = match (g.memory_used, g.memory_total) {
                        (Some(mu), Some(mt)) => format!("显存 {}/{}", fmt::bytes(mu), fmt::bytes(mt)),
                        _ => String::new(),
                    };
                    // 型号放第二行；有独显显存就并到同一行，型号在前——真放不下时截断的是显存，不丢型号
                    let detail = match (model.is_empty(), vram.is_empty()) {
                        (false, false) => format!("{model} · {vram}"),
                        (false, true) => model,
                        (true, _) => vram,
                    };
                    let title = if multi {
                        format!("GPU{} · {}", g.index, vendor)
                    } else {
                        vendor
                    };
                    let util = g.utilization.map(|v| v as f32);
                    let temp = g.temp.map(|v| v as f32);
                    // 使用率 / 温度读不到的环不画（标注一起省掉）；名称本身可读，卡片照常显示
                    let mut rings = Vec::new();
                    if util.is_some() {
                        rings.push(pct_ring(util));
                    }
                    if temp.is_some() {
                        rings.push(temp_ring(temp));
                    }
                    cards_out.push(Card::dual_gauge(title, detail, rings, DUAL_GAUGE_H));
                }
            }
            // 网卡心电图：每个接口一张图（蓝收/橙发），标题带上网卡协商速率（读不到则只显示"网络"）
            3 => {
                for n in s.network.as_deref().unwrap_or(&[]) {
                    let pts = net_hist.get(&n.name).map_or_else(Vec::new, hist_points);
                    // 读不到的收发速度整段不显示（连"收 / 发"字样一起省掉）
                    let d1 = n
                        .rx_bytes_per_sec
                        .map_or_else(String::new, |v| format!("收 {}", fmt::rate(v)));
                    let d2 = n
                        .tx_bytes_per_sec
                        .map_or_else(String::new, |v| format!("发 {}", fmt::rate(v)));
                    let speed = fmt::link_speed(n.speed_mbps.unwrap_or(0));
                    // 标题只留"网络"，链路协商速率作为右上角小字（避免标题串过长过重）
                    cards_out.push(Card::chart(
                        "网络".to_string(),
                        speed,
                        pts,
                        d1,
                        d2,
                        ACCENT,
                        TX_LINE,
                    ));
                }
            }
            // 硬盘读写：所有盘汇总的读（蓝）/写（绿）双曲线心电图
            4 => {
                if has_storage {
                    let (dr, dw) = disk_io_sum(s);
                    let pts = hist_points(disk_hist);
                    let d1 = format!("读 {}", fmt::rate(dr));
                    let d2 = format!("写 {}", fmt::rate(dw));
                    cards_out.push(Card::chart(
                        "硬盘读写".to_string(),
                        String::new(),
                        pts,
                        d1,
                        d2,
                        READ_BLUE,
                        LEVEL_GREEN,
                    ));
                }
            }
            // 存储状态：容量读不到的行整行不显示（连卷名 / 盘名一起省掉）
            5 => {
                let vols: Vec<&crate::data::Volume> =
                    volumes.iter().filter(|v| vol_row_ok(v)).collect();
                let ds: Vec<&Disk> = disks.iter().filter(|d| disk_row_ok(d)).collect();
                if !vols.is_empty() {
                    // 方案一：卷为主，默认只显示卷行；点开的卷列出底层盘（standby 灰显）
                    let rows = vols
                        .iter()
                        .map(|vol| VolRow {
                            name: vol.name.clone(),
                            cap: format!(
                                "{} / {}",
                                fmt::bytes(vol.used_bytes.unwrap_or(0)),
                                fmt::bytes(vol.total_bytes.unwrap_or(0))
                            ),
                            ratio: ratio_of(vol.used_bytes, vol.total_bytes),
                        })
                        .collect();
                    cards_out.push(Card::volumes(
                        "存储状态",
                        format!("{} 卷", vols.len()),
                        rows,
                    ));
                } else if !ds.is_empty() {
                    // 兜底：旧服务端/无卷数据时，盘直接平铺，保证物理盘可见
                    let rows = ds
                        .iter()
                        .map(|d| {
                            let name = d.device.trim_start_matches("/dev/").to_string();
                            let (ratio, cap) = disk_capacity_text(d);
                            BarRow { name, ratio, cap }
                        })
                        .collect();
                    cards_out.push(Card::bars("存储状态", format!("{} 块盘", ds.len()), rows));
                }
            }
            _ => {}
        }
    }

    cards_out
}

/// CPU 卡可显示：使用率 / 温度至少一项读得到（都读不到时连卡名一起不显示）。
fn cpu_card_ok(s: &Snapshot) -> bool {
    s.cpu
        .as_ref()
        .is_some_and(|c| c.usage.is_some() || c.temp.is_some())
}

/// 内存卡可显示：使用率或「已用 / 总量」读得到。
fn mem_card_ok(s: &Snapshot) -> bool {
    s.memory
        .as_ref()
        .is_some_and(|m| m.usage.is_some() || (m.used.is_some() && m.total.is_some()))
}

/// 卷行可显示：已用与总量都读得到（读不到的整行不显示，连卷名一起省掉）。
fn vol_row_ok(v: &crate::data::Volume) -> bool {
    v.total_bytes.is_some_and(|t| t > 0) && v.used_bytes.is_some()
}

/// 盘行可显示：同卷行。
fn disk_row_ok(d: &Disk) -> bool {
    d.total_bytes.is_some_and(|t| t > 0) && d.used_bytes.is_some()
}

/// 存储卡可显示的行数与行高：卷行 42 / 盘行 48（读不到容量的行整行不显示）。
fn storage_rows_geom(s: &Snapshot) -> (usize, f32) {
    let vols = s.volumes.as_deref().unwrap_or(&[]);
    let nv = vols.iter().filter(|v| vol_row_ok(v)).count();
    if nv > 0 {
        return (nv, VOL_ROW_H);
    }
    let nd = s
        .disks
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|d| disk_row_ok(d))
        .count();
    (nd, BAR_ROW_H)
}

/// 追加网卡样本并清理已消失的接口（后台线程持有心电图历史）。
fn update_net_hist(hist: &mut HashMap<String, VecDeque<(u64, u64)>>, s: &Snapshot) {
    let Some(nets) = &s.network else { return };
    for n in nets {
        let e = hist.entry(n.name.clone()).or_default();
        e.push_back((
            n.rx_bytes_per_sec.unwrap_or(0),
            n.tx_bytes_per_sec.unwrap_or(0),
        ));
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

/// 追加 CPU（使用率、温度）、显卡（使用率、温度）与内存使用率样本，供大卡全宽曲线。
/// 缺数据的指标记 -1，画图时跳过该点。
fn update_gauge_hist(
    cpu: &mut VecDeque<(f32, f32)>,
    gpu: &mut VecDeque<(f32, f32)>,
    mem: &mut VecDeque<f32>,
    s: &Snapshot,
) {
    let usage = s
        .cpu
        .as_ref()
        .and_then(|c| c.usage)
        .map_or(-1.0, |v| v as f32);
    let temp = s
        .cpu
        .as_ref()
        .and_then(|c| c.temp)
        .map_or(-1.0, |v| v as f32);
    cpu.push_back((usage, temp));
    while cpu.len() > NET_HIST {
        cpu.pop_front();
    }
    // 显卡只取第一张（与网络/硬盘卡同样只显示首项），温度缺失同样记 -1
    let g = s.gpus.as_deref().and_then(|g| g.first());
    let gu = g.and_then(|g| g.utilization).map_or(-1.0, |v| v as f32);
    let gt = g.and_then(|g| g.temp).map_or(-1.0, |v| v as f32);
    gpu.push_back((gu, gt));
    while gpu.len() > NET_HIST {
        gpu.pop_front();
    }
    let mu = s
        .memory
        .as_ref()
        .and_then(|m| m.usage)
        .map_or(-1.0, |v| v as f32);
    mem.push_back(mu);
    while mem.len() > NET_HIST {
        mem.pop_front();
    }
}

/// 展开面板总高度（仅由设备数量与卡片开关决定）。
fn panel_height(has_data: bool, snap: &Snapshot, cards: CardVis) -> u32 {
    if !has_data {
        return (PANEL_PAD * 2.0 + HEADER_H) as u32;
    }
    let hs = card_heights(snap, cards);
    let sum: f32 = hs.iter().sum();
    if hs.is_empty() {
        // 卡片被全部关掉（理论上设置界面会拦，这里兜底）：只留标题行高度
        return (PANEL_PAD * 2.0 + HEADER_H) as u32;
    }
    (PANEL_PAD * 2.0 + HEADER_H + sum + CARD_GAP * (hs.len() as f32 - 1.0)) as u32
}

/// 展开面板高度：固定到任务栏时窗口本身就是面板（底边贴任务栏，无顶栏）；
/// 竖排时与竖条并排、面板顶边对齐竖条顶边（同样无顶栏）；未固定横排 = 顶栏 + 间隙 + 面板。
fn expanded_h(frame: &Frame) -> u32 {
    let ph = panel_height(frame.has_data, frame.snap, frame.cards);
    if frame.pinned || frame.vertical {
        ph
    } else {
        CAPSULE_H + GAP + ph
    }
}

/// 卡片区起始 y：标题行在底部时卡片从顶部内边距开始，否则让出顶部标题行。
fn card_top_y(header_bottom: bool) -> f32 {
    if header_bottom {
        PANEL_PAD
    } else {
        PANEL_PAD + HEADER_H
    }
}

/// 后台面板构建线程：把每秒一次的整层光栅化（2~4ms）移出 UI 帧关键路径。
/// 持有独立的字体实例、心电图历史与可复用 clip mask；
/// 请求通道关闭（UI 退出）时 recv 出错，线程自然结束。
struct PanelBuilder {
    font: Option<Font>,
    net_hist: HashMap<String, VecDeque<(u64, u64)>>,
    disk_hist: VecDeque<(u64, u64)>,
    /// CPU 历史：(使用率%, 温度°C)，温度缺省记 -1；大卡曲线用
    cpu_hist: VecDeque<(f32, f32)>,
    /// 显卡历史：(使用率%, 温度°C)，缺省记 -1
    gpu_hist: VecDeque<(f32, f32)>,
    /// 内存使用率历史（%）
    mem_hist: VecDeque<f32>,
    /// 本次构建得到的柱状图几何（随面板一起交给 UI 线程）
    bars_pending: Option<BarsGeom>,
    last_seq: u64,
    clip: Option<Mask>,
    clip_w: u32,
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
                    cpu_hist: VecDeque::new(),
                    gpu_hist: VecDeque::new(),
                    mem_hist: VecDeque::new(),
                    bars_pending: None,
                    last_seq: 0,
                    clip: None,
                    clip_w: 0,
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
            PanelJob::Advance {
                seq,
                has_data,
                snap,
            } => {
                if has_data && seq != self.last_seq {
                    self.last_seq = seq;
                    update_net_hist(&mut self.net_hist, &snap);
                    update_disk_hist(&mut self.disk_hist, &snap);
                    update_gauge_hist(
                        &mut self.cpu_hist,
                        &mut self.gpu_hist,
                        &mut self.mem_hist,
                        &snap,
                    );
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
            update_gauge_hist(
                &mut self.cpu_hist,
                &mut self.gpu_hist,
                &mut self.mem_hist,
                &req.snap,
            );
        }

        let (w, ph) = (req.w, req.ph);
        self.bars_pending = None;
        // 优先复用 UI 归还的同尺寸缓冲；复用前整幅清零（圆角外域必须保持透明）
        let mut pm = match self
            .spare
            .iter()
            .position(|p| p.width() == w && p.height() == ph)
        {
            Some(i) => {
                let mut pm = self.spare.swap_remove(i);
                pm.data_mut().fill(0);
                pm
            }
            None => Pixmap::new(w, ph).unwrap(),
        };
        if self.clip.as_ref().is_none() || self.clip_h != ph || self.clip_w != w {
            self.clip = Mask::new(w, ph);
            self.clip_w = w;
            self.clip_h = ph;
        }

        if req.big {
            self.paint_big(&mut pm, req);
        } else {
            let heights = card_heights(&req.snap, req.cards);
            // 背景与卡片底/描边：clip mask 借用限制在本块内
            {
                let clip = self.clip.as_mut().unwrap();
                fill_round_opaque(&mut pm, clip, 0.0, 0.0, w as f32, ph as f32, 14.0, BG);

                let mut cy = card_top_y(req.header_bottom);
                for &h in &heights {
                    fill_round_opaque(&mut pm, clip, PANEL_PAD, cy, CARD_W, h, 10.0, CARD_BG);
                    stroke_card(&mut pm, PANEL_PAD, cy, CARD_W, h);
                    cy += h + CARD_GAP;
                }
            }

            if let Some(font) = &self.font {
                // 标题行的连接状态灯/文字与设置齿轮由 render_expanded 在面板 blit 之后绘制
                if req.has_data {
                    let cards = build_cards(&req.snap, &self.net_hist, &self.disk_hist, req.cards);
                    let mut cy = card_top_y(req.header_bottom);
                    for card in &cards {
                        draw_card(&mut pm, font, PANEL_PAD, cy, CARD_W, card, req.dark);
                        cy += card.h + CARD_GAP;
                    }
                }
            }
        }

        BuiltPanel {
            seq: req.seq,
            conn: req.conn.clone(),
            w,
            ph,
            big: req.big,
            dark: req.dark,
            header_bottom: req.header_bottom,
            cards: req.cards,
            bars: self.bars_pending,
            pm,
        }
    }

    /// 超大卡片整层：不透明圆角底 + 两列卡片（复用 292 宽的小面板卡片）。
    /// 标题行由 UI 线程在 blit 之后绘制（齿轮图标在视图侧）。
    fn paint_big(&mut self, pm: &mut Pixmap, req: &BuildReq) {
        let (w, h) = (req.w as f32, req.ph as f32);
        {
            let clip = self.clip.as_mut().unwrap();
            fill_round_opaque(pm, clip, 0.0, 0.0, w, h, BIG_R, BIG_BG);
        }
        if !req.has_data {
            return;
        }
        let Some(font) = &self.font else { return };
        let s = &req.snap;
        let dark = req.dark;
        let d = text_c(dark, FG_DIM);
        let dim = color(d.0, d.1, d.2, d.3);
        let fg = color(FG.0, FG.1, FG.2, FG.3);
        let x0 = BIG_PAD;
        let w = BIG_INNER_W;
        // 每块图表各自一张卡片：内容再内缩一层内边距
        let ix = x0 + BIG_CARD_PAD;
        let iw = w - BIG_CARD_PAD * 2.0;
        // 左右两栏：左栏实时曲线 + 实时读数，右栏统计（列表 / 柱状图）
        const LEFT_W: f32 = 344.0;
        let cx = ix + 34.0;
        let cw = LEFT_W - 34.0;
        // 通栏卡片（显卡 / 硬盘读写）的曲线宽度：占满整卡，不再留右栏
        let cw_full = iw - 34.0;
        let div_x = ix + LEFT_W + 6.0;
        let rxo = div_x + 10.0;
        let rw = ix + iw - rxo;
        let cy_off = 36.0;
        // 显示哪些块完全跟随设置窗的「显示卡片」开关，顺序与设置窗一致
        let cards = req.cards;
        let cpu = s.cpu.as_ref();
        let gpu = s.gpus.as_deref().and_then(|g| g.first());
        let net = s.network.as_deref().and_then(|n| n.first());
        let vols = s.volumes.as_deref().unwrap_or(&[]);
        let disks = s.disks.as_deref().unwrap_or(&[]);
        // 显示哪些块：开关打开 **且** 该项读得到（读不到的块整块不显示，连标题一起省掉）
        let show_cpu = cards.cpu && cpu_card_ok(s);
        let show_mem = cards.memory && mem_card_ok(s);
        let show_gpu = cards.gpu && gpu.is_some();
        let show_net = cards.network && net.is_some();
        let show_disk = cards.disk_io && !(disks.is_empty() && vols.is_empty());
        // 存储块与小面板同源：优先卷行，无卷时退回物理盘；容量读不到的行不计入
        let store_rows = storage_rows_geom(s).0;
        let show_store = cards.storage && store_rows > 0;

        // 面板与卡片高度都固定：始终按 6 个卡位算，勾选少时留白、不缩面板；
        // 只有工作区太矮、6 个 152 的卡位放不下时才等比压缩。
        const STORE_HEAD: f32 = 42.0;
        const STORE_ROW: f32 = 28.0;
        let slots = crate::config::CARD_COUNT as f32;
        let avail_h = h - BIG_PAD * 2.0 - BIG_HEADER_H;
        let fixed_h = BIG_CARD_H * slots + BIG_BLOCK_GAP * (slots - 1.0);
        let bh = if fixed_h <= avail_h {
            BIG_CARD_H
        } else {
            ((avail_h - BIG_BLOCK_GAP * (slots - 1.0)) / slots).max(110.0)
        };
        // 存储卡最多 4 行卷，行数随卡片高度自适应
        let shown_rows = (((bh - STORE_HEAD) / STORE_ROW).floor() as usize)
            .clamp(1, 4)
            .min(store_rows);
        let ch = (bh - cy_off - 58.0).max(24.0);
        // 块顺序跟随设置里的勾选先后：先按顺序给每块算好 y（块之间不重叠，
        // 绘制先后不影响结果），未显示的块不占位。
        let visible = [show_cpu, show_mem, show_gpu, show_net, show_disk, show_store];
        let mut ypos = [0.0f32; 6];
        let mut rank = 0usize;
        for idx in cards.ordered() {
            if visible[idx] {
                ypos[idx] = BIG_PAD + BIG_HEADER_H + rank as f32 * (bh + BIG_BLOCK_GAP);
                rank += 1;
            }
        }

        // ---- CPU：使用率面积 + 温度折线 ----
        if show_cpu {
            let y = ypos[0];
            let usage = cpu.and_then(|c| c.usage).map(|v| v as f32);
            let temp = cpu.and_then(|c| c.temp).map(|v| v as f32);
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            // 右上说明只列真的画得出的指标
            let cpu_note = if temp.is_some() {
                "使用率 · 温度"
            } else {
                "使用率"
            };
            draw_big_title(pm, font, ix, iw, y, "CPU", cpu_note, fg, dim);
            draw_big_grid(
                pm,
                font,
                cx,
                y + cy_off,
                cw,
                ch,
                &percent_grid_labels(),
                dim,
            );
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw,
                ch,
                &hist_ratio_pts(&self.cpu_hist, false),
                ACCENT,
                true,
            );
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw,
                ch,
                &hist_ratio_pts(&self.cpu_hist, true),
                WARN,
                false,
            );
            let sy = y + bh - 48.0;
            // 读数行：读不到的指标整段不画（连"利用率 / 温度"标签一起省掉），依次左排
            let mut sx = ix;
            if let Some(v) = usage {
                let (txt, col) = (format!("{v:.0}%"), usage_color(v));
                draw_big_stat(pm, font, sx, sy, col, "利用率", &txt, 22.0, dark, dim);
                sx += 174.0;
            }
            if let Some(v) = temp {
                let (txt, col) = (format!("{v:.0}°C"), temp_color(v));
                draw_big_stat(pm, font, sx, sy, col, "温度", &txt, 22.0, dark, dim);
            }
            // 右栏：平均负载（/proc/loadavg 三个时间窗口）；读不到时整块不显示
            let load = match (
                cpu.and_then(|c| c.load1),
                cpu.and_then(|c| c.load5),
                cpu.and_then(|c| c.load15),
            ) {
                (Some(a), Some(b), Some(c)) => {
                    let mut rows = vec![
                        ("1 分钟".to_string(), format!("{a:.2}")),
                        ("5 分钟".to_string(), format!("{b:.2}")),
                        ("15 分钟".to_string(), format!("{c:.2}")),
                    ];
                    // 30 分钟档是服务端自算的（对 load1 采样取均值），覆盖满 30 分钟才有值
                    if let Some(d) = cpu.and_then(|c| c.load30) {
                        rows.push(("30 分钟".to_string(), format!("{d:.2}")));
                    }
                    Some(rows)
                }
                _ => None,
            };
            if let Some(load_rows) = load {
                draw_big_divider(pm, div_x, y + 32.0, bh - 46.0);
                draw_big_kv_grid(
                    pm,
                    font,
                    rxo,
                    y + 30.0,
                    rw,
                    bh - 40.0,
                    &load_rows,
                    fg,
                    dim,
                );
            }
        }

        // ---- 内存：使用率面积 ----
        if show_mem {
            let y = ypos[1];
            let mem = s.memory.as_ref();
            let mem_usage = mem.and_then(|m| m.usage).map(|v| v as f32);
            let mem_note = match (mem.and_then(|m| m.used), mem.and_then(|m| m.total)) {
                (Some(u), Some(t)) => {
                    format!("{} / {}", fmt::bytes_unit(u), fmt::bytes_unit(t))
                }
                _ => String::new(),
            };
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "内存", &mem_note, fg, dim);
            draw_big_grid(
                pm,
                font,
                cx,
                y + cy_off,
                cw,
                ch,
                &percent_grid_labels(),
                dim,
            );
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw,
                ch,
                &mem_ratio_pts(&self.mem_hist),
                ACCENT,
                true,
            );
            let sy = y + bh - 48.0;
            // 一位小数：整数百分比在空闲机器上会长时间停在同一格，看起来像"卡住了"
            if let Some(v) = mem_usage {
                let (txt, col) = (format!("{v:.1}%"), usage_color(v));
                draw_big_stat(pm, font, ix, sy, col, "使用率", &txt, 22.0, dark, dim);
            }
            // 右栏：内存明细小卡片（总计 / 已用 / 缓存 / 可用，三者相加等于总计）；
            // 读不到的项整行不显示，一行都读不到时整块不显示
            let mem_rows: Vec<(String, String)> = match mem {
                Some(m) => [
                    ("总计", m.total),
                    ("已用", m.used),
                    ("缓存", m.cache),
                    ("可用", m.free),
                ]
                .into_iter()
                .filter_map(|(k, v)| v.map(|b| (k.to_string(), fmt::bytes_unit(b))))
                .collect(),
                None => Vec::new(),
            };
            if !mem_rows.is_empty() {
                draw_big_divider(pm, div_x, y + 32.0, bh - 46.0);
                draw_big_kv_grid(
                    pm,
                    font,
                    rxo,
                    y + 30.0,
                    rw,
                    bh - 40.0,
                    &mem_rows,
                    fg,
                    dim,
                );
            }
        }

        // ---- 显卡：使用率面积 + 温度折线（与小面板一样是双指标）----
        if show_gpu {
            if let Some(g) = gpu {
                let y = ypos[2];
                let gu = g.utilization.map(|v| v as f32);
                let gt = g.temp.map(|v| v as f32);
                draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
                draw_big_title(pm, font, ix, iw, y, "显卡", &g.name, fg, dim);
                draw_big_grid(
                    pm,
                    font,
                    cx,
                    y + cy_off,
                    cw_full,
                    ch,
                    &percent_grid_labels(),
                    dim,
                );
                draw_big_series(
                    pm,
                    cx,
                    y + cy_off,
                    cw_full,
                    ch,
                    &hist_ratio_pts(&self.gpu_hist, false),
                    ACCENT,
                    true,
                );
                draw_big_series(
                    pm,
                    cx,
                    y + cy_off,
                    cw_full,
                    ch,
                    &hist_ratio_pts(&self.gpu_hist, true),
                    WARN,
                    false,
                );
                let sy = y + bh - 48.0;
                // 读数行：读不到的指标整段不画（标签一起省掉），依次左排
                let mut sx = ix;
                if let Some(v) = gu {
                    let (txt, col) = (format!("{v:.0}%"), usage_color(v));
                    draw_big_stat(pm, font, sx, sy, col, "利用率", &txt, 22.0, dark, dim);
                    sx += 174.0;
                }
                if let Some(v) = gt {
                    let (txt, col) = (format!("{v:.0}°C"), temp_color(v));
                    draw_big_stat(pm, font, sx, sy, col, "温度", &txt, 22.0, dark, dim);
                    sx += 174.0;
                }
                // 通栏：第三列显示显存（已用 / 总量 + 占用百分比）；读不到时整段不显示
                if let (Some(mu), Some(mt)) = (g.memory_used, g.memory_total) {
                    if mt > 0 {
                        let note = format!("显存 {:.0}%", mu as f64 / mt as f64 * 100.0);
                        let txt = format!("{} / {}", fmt::bytes_unit(mu), fmt::bytes_unit(mt));
                        draw_big_stat(pm, font, sx, sy, FG_DIM, &note, &txt, 16.0, dark, dim);
                    }
                }
            }
        }

        // ---- 网络：接收 / 发送双线 ----
        if show_net {
            let y = ypos[3];
            let iface = net.map_or("--", |n| n.name.as_str());
            let speed = net.map_or(String::new(), |n| {
                fmt::link_speed(n.speed_mbps.unwrap_or(0))
            });
            let net_note = if speed.is_empty() {
                iface.to_string()
            } else {
                format!("{iface} · {speed}")
            };
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "网络", &net_note, fg, dim);
            let q = self.net_hist.get(iface);
            let (net_max, net_pts) =
                q.map_or((0.0f64, Vec::new()), |q| (hist_max(q), hist_points(q)));
            let net_labels = rate_grid_labels(net_max);
            draw_big_grid(pm, font, cx, y + cy_off, cw, ch, &net_labels, dim);
            let rx_pts: Vec<(f32, f32)> = net_pts.iter().map(|p| (p.0, p.1)).collect();
            let tx_pts: Vec<(f32, f32)> = net_pts.iter().map(|p| (p.0, p.2)).collect();
            draw_big_series(pm, cx, y + cy_off, cw, ch, &rx_pts, ACCENT, true);
            draw_big_series(pm, cx, y + cy_off, cw, ch, &tx_pts, TX_LINE, false);
            let sy = y + bh - 48.0;
            // 读数行：读不到的方向整段不画（标签一起省掉），依次左排
            let mut sx = ix;
            if let Some(v) = net.and_then(|n| n.rx_bytes_per_sec) {
                draw_big_stat(
                    pm,
                    font,
                    sx,
                    sy,
                    ACCENT,
                    "接收速度",
                    &fmt::rate_unit(v),
                    22.0,
                    dark,
                    dim,
                );
                sx += 174.0;
            }
            if let Some(v) = net.and_then(|n| n.tx_bytes_per_sec) {
                draw_big_stat(
                    pm,
                    font,
                    sx,
                    sy,
                    TX_LINE,
                    "发送速度",
                    &fmt::rate_unit(v),
                    22.0,
                    dark,
                    dim,
                );
            }
            // 右栏：近 30 日堆叠柱状图（下蓝＝下行 / 上橙＝上行）
            // 下面的信息栏由 UI 线程叠加绘制（悬停时换成单日数据），这里只画图并记几何。
            // 服务端没给流量统计时整块不显示，连分隔线一起省掉。
            if let Some(tf) = s.traffic.as_ref() {
                draw_big_divider(pm, div_x, y + 32.0, bh - 46.0);
                font.draw(pm, rxo, y + 30.0, 9.5, "近 30 日流量（下蓝 / 上橙）", dim);
                let bars_y = y + 40.0;
                let line_y = y + bh - 12.0;
                let bars_h = (line_y - 18.0) - bars_y;
                let count = tf.days_rx.len().min(tf.days_tx.len()).min(30);
                if bars_h >= 22.0 && count > 0 {
                    draw_big_bars(pm, rxo, bars_y, rw, bars_h, &tf.days_rx, &tf.days_tx);
                    self.bars_pending = Some(BarsGeom {
                        x: rxo,
                        y: bars_y,
                        w: rw,
                        h: bars_h,
                        line_y,
                        count,
                    });
                }
            }
        }

        // ---- 硬盘读写：读 / 写双线 ----
        if show_disk {
            let y = ypos[4];
            let (dr, dw) = disk_io_sum(s);
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "硬盘读写", "读 / 写", fg, dim);
            let disk_max = hist_max(&self.disk_hist);
            let disk_labels = rate_grid_labels(disk_max);
            draw_big_grid(pm, font, cx, y + cy_off, cw_full, ch, &disk_labels, dim);
            let disk_pts = hist_points(&self.disk_hist);
            let read_pts: Vec<(f32, f32)> = disk_pts.iter().map(|p| (p.0, p.1)).collect();
            let write_pts: Vec<(f32, f32)> = disk_pts.iter().map(|p| (p.0, p.2)).collect();
            draw_big_series(pm, cx, y + cy_off, cw_full, ch, &read_pts, READ_BLUE, true);
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw_full,
                ch,
                &write_pts,
                LEVEL_GREEN,
                false,
            );
            let sy = y + bh - 48.0;
            draw_big_stat(
                pm,
                font,
                ix,
                sy,
                READ_BLUE,
                "读取速度",
                &fmt::rate_unit(dr),
                22.0,
                dark,
                dim,
            );
            draw_big_stat(
                pm,
                font,
                ix + 174.0,
                sy,
                LEVEL_GREEN,
                "写入速度",
                &fmt::rate_unit(dw),
                22.0,
                dark,
                dim,
            );
            // 通栏：第三列显示磁盘总容量（单盘明细在存储卡里）；容量读不到时整段不显示
            let (du, dt): (u64, u64) = {
                let u: u64 = disks.iter().filter_map(|d| d.used_bytes).sum();
                let t: u64 = disks.iter().filter_map(|d| d.total_bytes).sum();
                (u, t)
            };
            if dt > 0 {
                let cap_txt = format!("{} / {}", fmt::bytes_unit(du), fmt::bytes_unit(dt));
                draw_big_stat(
                    pm,
                    font,
                    ix + 348.0,
                    sy,
                    FG_DIM,
                    "总容量",
                    &cap_txt,
                    16.0,
                    dark,
                    dim,
                );
            }
        }

        // ---- 存储状态：卷用量进度条（通栏，无卷时退回物理盘）----
        if show_store {
            let y = ypos[5];
            // 容量读不到的行整行不显示（连卷名 / 盘名一起省掉）
            let (rows, used, total): (Vec<(String, String, Option<f32>)>, u64, u64) = {
                let vrows: Vec<&crate::data::Volume> =
                    vols.iter().filter(|v| vol_row_ok(v)).collect();
                if !vrows.is_empty() {
                    let rows = vrows
                        .iter()
                        .map(|v| {
                            (
                                v.name.clone(),
                                capacity_text_unit(v.used_bytes, v.total_bytes),
                                ratio_of(v.used_bytes, v.total_bytes),
                            )
                        })
                        .collect();
                    let u = vrows.iter().filter_map(|v| v.used_bytes).sum();
                    let t = vrows.iter().filter_map(|v| v.total_bytes).sum();
                    (rows, u, t)
                } else {
                    let drows: Vec<&Disk> = disks.iter().filter(|d| disk_row_ok(d)).collect();
                    let rows = drows
                        .iter()
                        .map(|d| {
                            let name = d
                                .device
                                .rsplit('/')
                                .next()
                                .unwrap_or(d.device.as_str())
                                .to_string();
                            (
                                name,
                                capacity_text_unit(d.used_bytes, d.total_bytes),
                                ratio_of(d.used_bytes, d.total_bytes),
                            )
                        })
                        .collect();
                    let u = drows.iter().filter_map(|d| d.used_bytes).sum();
                    let t = drows.iter().filter_map(|d| d.total_bytes).sum();
                    (rows, u, t)
                }
            };
            // 通栏：已用 / 可用 / 使用率直接放在标题行右侧
            let note = if total > 0 {
                format!(
                    "已用 {} · 可用 {} · {:.1}%",
                    fmt::bytes_unit(used),
                    fmt::bytes_unit(total.saturating_sub(used)),
                    used as f64 / total as f64 * 100.0
                )
            } else {
                String::new()
            };
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "存储", &note, fg, dim);
            // 卷用量条通栏
            let bar_w = iw;
            for (i, (name, cap, ratio)) in rows.iter().take(shown_rows).enumerate() {
                let ry = y + 36.0 + i as f32 * STORE_ROW;
                let base = ry + 13.0;
                let right = ix + bar_w;
                let cap_w = font.measure(10.5, cap);
                font.draw(pm, right - cap_w, base, 10.5, cap, dim);
                let nm = fit_text(font, 10.5, name, (right - cap_w - 12.0 - ix).max(40.0));
                font.draw(pm, ix, base, 10.5, &nm, fg);
                const PB: f32 = 5.0;
                fill_round(pm, ix, ry + 18.0, bar_w, PB, PB / 2.0, TRACK);
                if let Some(p) = ratio {
                    let pc = usage_color(*p);
                    let fw = (bar_w * (p / 100.0).clamp(0.0, 1.0)).max(PB);
                    fill_round(pm, ix, ry + 18.0, fw, PB, PB / 2.0, pc);
                }
            }
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

/// 大卡左右两栏的竖分隔线（从标题行下方到卡片底部）。
fn draw_big_divider(pm: &mut Pixmap, x: f32, y: f32, h: f32) {
    fill_round(pm, x, y, 1.0, h, 0.5, CARD_LINE);
}

/// 大卡右栏的 2×2 小卡片网格：每张小卡一项「竖排标签 + 大号数值」
/// （CPU：平均负载 1 / 5 / 15 / 30 分钟，内存：总计 / 已用 / 缓存 / 可用）。
/// 标签竖排只占一条窄列，横向空间留给数值，数值因此能放大到 17px。
/// 读不到的项不会传进来，只剩几项就排几项，空位不留灰底。
fn draw_big_kv_grid(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    y: f32,
    w: f32,
    max_h: f32,
    items: &[(String, String)],
    fg: Color,
    dim: Color,
) {
    const GAP: f32 = 12.0;
    const COLS: usize = 2;
    const PAD: f32 = 10.0;
    // 竖排标签：一字一行（空格丢弃），列宽 12、行距 12
    const LBL_FS: f32 = 10.0;
    const LBL_LH: f32 = 12.0;
    const LBL_COL: f32 = 12.0;
    // 数值：标签列右侧，字号放大
    const VAL_FS: f32 = 17.0;
    if items.is_empty() {
        return;
    }
    let rows = (items.len() + COLS - 1) / COLS;
    let cw = (w - GAP * (COLS as f32 - 1.0)) / COLS as f32;
    let ch = ((max_h - GAP * (rows as f32 - 1.0)) / rows as f32).min(58.0);
    if ch < 34.0 || cw < 60.0 {
        return;
    }
    for (i, (label, value)) in items.iter().enumerate() {
        let cx = x + (i % COLS) as f32 * (cw + GAP);
        let cy = y + (i / COLS) as f32 * (ch + GAP);
        fill_round(pm, cx, cy, cw, ch, 10.0, BIG_SUB_BG);
        // 竖排标签：整块在卡片内垂直居中
        let chars: Vec<char> = label.chars().filter(|c| !c.is_whitespace()).collect();
        let block_top = cy + (ch - chars.len() as f32 * LBL_LH) / 2.0;
        for (k, c) in chars.iter().enumerate() {
            let s = c.to_string();
            let tw = font.measure(LBL_FS, &s);
            font.draw(
                pm,
                cx + PAD + (LBL_COL - tw) / 2.0,
                block_top + k as f32 * LBL_LH + LBL_LH / 2.0 + font.ascent(LBL_FS) / 2.0,
                LBL_FS,
                &s,
                dim,
            );
        }
        // 数值：标签列右侧、垂直居中
        let vx = cx + PAD + LBL_COL + 4.0;
        let vw = (cx + cw - PAD - vx).max(20.0);
        let v = fit_text_bold(font, VAL_FS, value, vw);
        font.draw_bold(
            pm,
            vx,
            cy + ch / 2.0 + font.ascent(VAL_FS) / 2.0,
            VAL_FS,
            &v,
            fg,
        );
    }
}

/// 大卡右栏的 30 日堆叠柱状图：下半蓝色为下行、上半橙色为上行，按当前最大值归一。
fn draw_big_bars(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, rx: &[u64], tx: &[u64]) {
    let n = rx.len().min(tx.len()).min(30);
    if n < 2 || h < 18.0 {
        return;
    }
    let mut max = 0u64;
    for i in 0..n {
        max = max.max(rx[i].saturating_add(tx[i]));
    }
    if max == 0 {
        return;
    }
    const GAP: f32 = 1.5;
    let bw = ((w - GAP * (n as f32 - 1.0)) / n as f32).max(1.2);
    let base = y + h;
    for i in 0..n {
        let total = rx[i].saturating_add(tx[i]) as f32;
        let filled = h * (total / max as f32);
        let tx_ratio = if total > 0.0 {
            tx[i] as f32 / total
        } else {
            0.0
        };
        let h_tx = filled * tx_ratio;
        let h_rx = filled - h_tx;
        let bx = x + i as f32 * (bw + GAP);
        if h_rx > 0.4 {
            // 下行：蓝
            fill_round(pm, bx, base - h_rx, bw, h_rx, 0.5, ACCENT);
        }
        if h_tx > 0.4 {
            // 上行：橙
            fill_round(pm, bx, base - h_rx - h_tx, bw, h_tx, 0.5, TX_LINE);
        }
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

/// 超大卡片面板总高：**恒定按 6 个卡位算**（与实际勾选几张、读得到几张无关），
/// 面板大小因此不会随内张小卡片增减而变化，用不满的卡位就留白。
pub(crate) fn big_panel_height() -> u32 {
    let slots = crate::config::CARD_COUNT as f32;
    let h = BIG_PAD * 2.0 + BIG_HEADER_H + slots * BIG_CARD_H + (slots - 1.0) * BIG_BLOCK_GAP;
    h.ceil() as u32
}

/// 大卡内一块图表的卡片底 + 1px 描边。
fn draw_big_card(pm: &mut Pixmap, clip: &mut Mask, x: f32, y: f32, w: f32, h: f32) {
    fill_round_opaque(pm, clip, x, y, w, h, 10.0, BIG_CARD_BG);
    stroke_card(pm, x, y, w, h);
}

/// 大卡区块标题：左上标题（粗体）、右上补充说明。
fn draw_big_title(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    w: f32,
    y: f32,
    title: &str,
    note: &str,
    fg: Color,
    dim: Color,
) {
    font.draw_bold(pm, x, y + 15.0, FONT_SIZE_TITLE + 0.5, title, fg);
    if !note.is_empty() {
        let nw = font.measure(FONT_SIZE_SMALL, note);
        font.draw(pm, x + w - nw, y + 15.0, FONT_SIZE_SMALL, note, dim);
    }
}

/// 大卡图表网格：5 条横线 + 左侧 3 个刻度（上 / 中 / 下）。
fn draw_big_grid(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    labels: &[String; 3],
    dim: Color,
) {
    for i in 0..=4 {
        let gy = y + h * i as f32 / 4.0;
        fill_round(pm, x, gy, w, 1.0, 0.0, (231, 235, 242, 255));
    }
    for (i, lab) in labels.iter().enumerate() {
        if lab.is_empty() {
            continue;
        }
        let ly = y + h * [0.0f32, 0.5, 1.0][i];
        let lw = font.measure(9.0, lab);
        let bl = match i {
            0 => ly + 8.0,
            1 => ly + 3.0,
            _ => ly - 2.0,
        };
        font.draw(pm, x - 6.0 - lw, bl, 9.0, lab, dim);
    }
}

/// 大卡曲线：面积填充（可选）+ 折线。`pts` 为 (x 0..1, y 0..1，1=底)。
fn draw_big_series(
    pm: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    pts: &[(f32, f32)],
    c: (u8, u8, u8, u8),
    fill: bool,
) {
    if pts.len() < 2 {
        return;
    }
    let px_at = |p: &(f32, f32)| (x + p.0.clamp(0.0, 1.0) * w, y + p.1.clamp(0.0, 1.0) * h);
    if fill {
        let mut pb = PathBuilder::new();
        let (fx, fy) = px_at(&pts[0]);
        pb.move_to(fx, y + h);
        pb.line_to(fx, fy);
        for p in &pts[1..] {
            let (vx, vy) = px_at(p);
            pb.line_to(vx, vy);
        }
        let (lx, _) = px_at(&pts[pts.len() - 1]);
        pb.line_to(lx, y + h);
        pb.close();
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(c.0, c.1, c.2, (c.3 / 7).max(10));
            paint.anti_alias = true;
            pm.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    let mut pb = PathBuilder::new();
    let (fx, fy) = px_at(&pts[0]);
    pb.move_to(fx, fy);
    for p in &pts[1..] {
        let (vx, vy) = px_at(p);
        pb.line_to(vx, vy);
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.0, c.1, c.2, c.3);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.6,
            ..Default::default()
        };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// 大卡底部统计：竖色条 + 小标签 + 大号数值（暗色主题下数值随 apply_dark 提白）。
#[allow(clippy::too_many_arguments)]
fn draw_big_stat(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    y: f32,
    c: (u8, u8, u8, u8),
    label: &str,
    value: &str,
    value_fs: f32,
    dark: bool,
    dim: Color,
) {
    fill_round(pm, x, y + 2.0, 2.5, 34.0, 1.25, c);
    font.draw(pm, x + 10.0, y + 14.0, FONT_SIZE_SMALL - 0.5, label, dim);
    let vc = text_c(dark, c);
    font.draw_bold(
        pm,
        x + 10.0,
        y + 38.0,
        value_fs,
        value,
        color(vc.0, vc.1, vc.2, vc.3),
    );
}

/// CPU 历史 -> 归一化折线（`temp` 选择温度序列；温度缺省点跳过）。
fn hist_ratio_pts(hist: &VecDeque<(f32, f32)>, temp: bool) -> Vec<(f32, f32)> {
    let n = hist.len();
    if n < 2 {
        return Vec::new();
    }
    hist.iter()
        .enumerate()
        .filter_map(|(i, &(u, t))| {
            let v = if temp { t } else { u };
            if v < 0.0 {
                None
            } else {
                Some((i as f32 / (n - 1) as f32, 1.0 - (v / 100.0).clamp(0.0, 1.0)))
            }
        })
        .collect()
}

/// 内存使用率历史 -> 归一化折线。
fn mem_ratio_pts(hist: &VecDeque<f32>) -> Vec<(f32, f32)> {
    let n = hist.len();
    if n < 2 {
        return Vec::new();
    }
    hist.iter()
        .enumerate()
        .filter_map(|(i, &v)| {
            if v < 0.0 {
                None
            } else {
                Some((i as f32 / (n - 1) as f32, 1.0 - (v / 100.0).clamp(0.0, 1.0)))
            }
        })
        .collect()
}

/// 速率历史的最大值（与 hist_points 一致，含 1.2 头部余量）。
fn hist_max(q: &VecDeque<(u64, u64)>) -> f64 {
    q.iter()
        .fold(1.0f64, |m, e| m.max(e.0 as f64).max(e.1 as f64))
        * 1.2
}

/// 百分比图表刻度（0-100）。
fn percent_grid_labels() -> [String; 3] {
    ["100".to_string(), "50".to_string(), "0".to_string()]
}

/// 速率图表的三个刻度：上 / 中 / 0。
fn rate_grid_labels(max: f64) -> [String; 3] {
    [
        fmt::rate(max as u64),
        fmt::rate((max / 2.0) as u64),
        "0".to_string(),
    ]
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

/// 绘制一张卡片正文（背景与描边由 panel_bg 缓存统一提供），按正文类型分发。
/// `dark` 只影响**文字取色**（暗色下所有文字走中性色，随后被 apply_dark 提白），
/// 环/条/曲线等图形仍按亮色的彩色画。
fn draw_card(px: &mut Pixmap, font: &Font, x: f32, y: f32, w: f32, card: &Card, dark: bool) {
    let h = card.h;

    let fg = color(FG.0, FG.1, FG.2, FG.3);
    // 次要文字：亮色用灰，暗色取主前景色（apply_dark 把深中性色提白）。
    // 直接用 FG_DIM 的话，它的抗锯齿中间覆盖率像素会落在提白阈值下方、被反相成中灰，
    // 整排小字看着是"灰的"而不是"白的"。
    let dc = text_c(dark, FG_DIM);
    let dim = color(dc.0, dc.1, dc.2, dc.3);

    match &card.body {
        Body::DualGauge {
            title,
            subtitle,
            rings,
        } => {
            // 右：数据环（数值在环心偏上、标注在环内下方）；环数随可读指标变化，右对齐
            const RING_R: f32 = 33.0;
            const RING_FS: f32 = GAUGE_VALUE_FS;
            const CAP_FS: f32 = GAUGE_CAP_FS;
            const RING_GAP: f32 = 82.0;
            let cy = y + h / 2.0;
            let cx_t = x + w - CARD_PAD - RING_R - 1.0;
            let n_ring = rings.len();
            let cx_u = cx_t - RING_GAP * n_ring.saturating_sub(1) as f32;
            // 名称可用宽度：到最左环边缘为止
            let avail = (cx_u - RING_R - 4.5 - (x + CARD_PAD)).max(60.0);

            // 左：名称（有小字明细时名称在上、明细在下；否则整体垂直居中）
            // 名称可用宽度只有 ~110px：先试一行；放不下按空格折两行（显卡名 `Intel UHD 630`
            // 就是这种情况，折行比缩字号清楚）；两行仍放不下才降一档字号，最后才截断。
            // 卡片标题一律**粗体**（与 NAS 名同级）；宽度按粗体测量，避免截断。
            const TITLE_MIN_FS: f32 = FONT_SIZE;
            let one_line = font.measure_bold(FONT_SIZE_TITLE, title) <= avail;
            let two_lines = if one_line {
                None
            } else {
                wrap_title_bold(font, FONT_SIZE_TITLE, title, avail, 2)
            };
            if let Some(lines) = two_lines {
                // 两行标题（+可选副行）作为一整块在卡片里垂直居中
                const LH: f32 = 17.0;
                const SUB_H: f32 = 15.0;
                let sub_h = if subtitle.is_empty() { 0.0 } else { SUB_H };
                let top = cy - (lines.len() as f32 * LH + sub_h) / 2.0;
                for (i, ln) in lines.iter().enumerate() {
                    let bl = top + LH / 2.0 + i as f32 * LH + font.ascent(FONT_SIZE_TITLE) / 2.0;
                    font.draw_bold(px, x + CARD_PAD, bl, FONT_SIZE_TITLE, ln, fg);
                }
                if !subtitle.is_empty() {
                    let sub = fit_text(font, FONT_SIZE_SMALL, subtitle, avail);
                    let bl = top
                        + lines.len() as f32 * LH
                        + SUB_H / 2.0
                        + font.ascent(FONT_SIZE_SMALL) / 2.0;
                    font.draw(px, x + CARD_PAD, bl, FONT_SIZE_SMALL, &sub, dim);
                }
            } else {
                let (title, tfs) = if one_line {
                    (title.clone(), FONT_SIZE_TITLE)
                } else if font.measure_bold(TITLE_MIN_FS, title) <= avail {
                    (title.clone(), TITLE_MIN_FS)
                } else {
                    (
                        fit_text_bold(font, TITLE_MIN_FS, title, avail),
                        TITLE_MIN_FS,
                    )
                };
                if subtitle.is_empty() {
                    font.draw_bold(
                        px,
                        x + CARD_PAD,
                        cy + font.ascent(tfs) / 2.0,
                        tfs,
                        &title,
                        fg,
                    );
                } else {
                    let sub = fit_text(font, FONT_SIZE_SMALL, subtitle, avail);
                    font.draw_bold(
                        px,
                        x + CARD_PAD,
                        y + 40.0 + font.ascent(tfs),
                        tfs,
                        &title,
                        fg,
                    );
                    font.draw(px, x + CARD_PAD, y + 74.0, FONT_SIZE_SMALL, &sub, dim);
                }
            }

            for (i, spec) in rings.iter().enumerate() {
                let cx = cx_t - RING_GAP * (n_ring - 1 - i) as f32;
                draw_ring(px, cx, cy, RING_R, 9.0, spec.ratio, spec.color);
                let vc = if spec.ratio.is_some() {
                    let c = text_c(dark, spec.color);
                    color(c.0, c.1, c.2, c.3)
                } else {
                    fg
                };
                let tw = font.measure(RING_FS, &spec.center);
                font.draw(px, cx - tw / 2.0, cy - 1.0, RING_FS, &spec.center, vc);
                let cw = font.measure(CAP_FS, &spec.cap);
                font.draw(px, cx - cw / 2.0, cy + 13.0, CAP_FS, &spec.cap, dim);
            }
        }
        Body::BigGauge {
            title,
            subtitle,
            pct,
        } => {
            // 左：名称 + 小字明细，垂直居中（标题粗体，同 DualGauge）
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
                font.draw_bold(
                    px,
                    x + CARD_PAD,
                    y + 40.0 + font.ascent(FONT_SIZE_TITLE),
                    FONT_SIZE_TITLE,
                    &title,
                    fg,
                );
                font.draw(px, x + CARD_PAD, y + 74.0, FONT_SIZE_SMALL, &sub, dim);
            }
            // 右：单个大圆环（使用率读不到时整环不画，连「使用率」一起省掉）
            if let Some(pct) = *pct {
                const RING_R: f32 = 36.0;
                const RING_FS: f32 = GAUGE_VALUE_FS;
                let cy = y + h / 2.0;
                let cx = x + w - CARD_PAD - RING_R - 2.0;
                let rc = usage_color(pct);
                draw_ring(px, cx, cy, RING_R, 8.0, Some(pct), rc);
                let ctext = format!("{:.0}%", pct.round());
                let tw = font.measure(RING_FS, &ctext);
                let c = text_c(dark, rc);
                font.draw(
                    px,
                    cx - tw / 2.0,
                    cy + font.ascent(RING_FS) / 2.0,
                    RING_FS,
                    &ctext,
                    color(c.0, c.1, c.2, c.3),
                );
            }
        }
        Body::Bars { title, note, rows } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            if !note.is_empty() {
                let nw = font.measure(FONT_SIZE_SMALL, note);
                font.draw(
                    px,
                    x + w - CARD_PAD - nw,
                    y + 24.0 - NOTE_LIFT,
                    FONT_SIZE_SMALL,
                    note,
                    dim,
                );
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
                font.draw(
                    px,
                    x + w - CARD_PAD - nw,
                    y + 24.0 - NOTE_LIFT,
                    FONT_SIZE_SMALL,
                    note,
                    dim,
                );
            }
            let mut ry = y + 28.0;
            for row in rows {
                // 卷行：上半 卷名(左) + 容量文本(右)；下半 用量进度条
                let text_baseline = ry + 16.0;
                let right = x + w - CARD_PAD;
                let cap_w = font.measure(FONT_SIZE_SMALL, &row.cap);
                font.draw(
                    px,
                    right - cap_w,
                    text_baseline,
                    FONT_SIZE_SMALL,
                    &row.cap,
                    dim,
                );
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
        Body::Chart {
            title,
            note,
            pts,
            detail1,
            detail2,
            c1,
            c2,
        } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            // 右上角小字补充（如网络卡的链路速率）：与"存储状态 · 1 卷"同一套排版，
            // 不并进标题串，否则标题又长又重，比其它卡片标题"大"一截。
            if !note.is_empty() {
                let nw = font.measure(FONT_SIZE_SMALL, note);
                font.draw(
                    px,
                    x + w - CARD_PAD - nw,
                    y + 24.0 - NOTE_LIFT,
                    FONT_SIZE_SMALL,
                    note,
                    dim,
                );
            }
            if !pts.is_empty() {
                draw_chart(
                    px,
                    x + CARD_PAD,
                    y + 32.0,
                    w - CARD_PAD * 2.0,
                    h - 62.0,
                    pts,
                    *c1,
                    *c2,
                    dark,
                );
            }
            // 底部明细：左段用 c1 色，右段用 c2 色——**两个主题都跟线条同色**。
            // 暗色下不再改成白：彩色像素的反算映射会把半覆盖的笔画落成同色暗调，
            // 小字照样是彩色的（以前统一改白是因为压灰会把彩色小字花掉）。
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

/// 画「NAS 名称（粗体）+ IP」标题：名称超宽按 avail 截断；缺名称只显 IP，都没有则 “NAS” 兜底。
/// 未固定时用在顶栏，固定到任务栏/竖排时用在面板标题行（排在“已连接”之前）；
/// 名称字号两处统一取 FONT_SIZE_HEADER，IP 取 FONT_SIZE_SMALL。
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
        if self.anim <= 0.0005 && !self.open {
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

/// 各卡片高度（与运行时数据无关，仅由设备数量与卡片开关决定）。
/// 顺序与 build_cards 完全一致（跟随设置里的勾选先后）。
/// 与 build_cards 必须用同一份开关，否则窗口高度与卡片内容会错位。
fn card_heights(s: &Snapshot, cards: CardVis) -> Vec<f32> {
    let mut v = Vec::new();
    let ng = s.gpus.as_deref().map_or(0, |g| g.len());
    let nn = s.network.as_deref().map_or(0, |n| n.len());
    let nd = s.disks.as_deref().map_or(0, |d| d.len());
    let nv = s.volumes.as_deref().map_or(0, |v| v.len());
    // 存储卡只按「容量读得到」的行数与行高算
    let (store_rows, store_row_h) = storage_rows_geom(s);

    for idx in cards.ordered() {
        match idx {
            0 => {
                if cpu_card_ok(s) {
                    v.push(CPU_CARD_H);
                }
            }
            1 => {
                if mem_card_ok(s) {
                    v.push(DUAL_GAUGE_H);
                }
            }
            // 显卡：每卡一张
            2 => v.extend(std::iter::repeat_n(DUAL_GAUGE_H, ng)),
            // 网卡心电图：每接口一张
            3 => v.extend(std::iter::repeat_n(CHART_H, nn)),
            4 => {
                if nd + nv > 0 {
                    v.push(CHART_H);
                }
            }
            // 存储卡（卷行优先，无卷时盘平铺）
            5 => {
                if store_rows > 0 {
                    v.push(BAR_BASE + store_rows as f32 * store_row_h);
                }
            }
            _ => {}
        }
    }
    v
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

/// 网络环两个百分比（上行、下行）：优先以网卡**协商速率**为分母；
/// 协商速率读不到时（WiFi 网卡无 sysfs speed、网卡 down）退化为以会话峰值为基准的相对刻度。
fn net_ring_pct(s: &Snapshot, peak: (u64, u64)) -> (Option<f32>, Option<f32>) {
    let (rx, tx) = net_io_sum(s);
    let speed = s
        .network
        .as_deref()
        .and_then(|v| v.iter().find_map(|n| n.speed_mbps.filter(|v| *v > 0)));
    let Some(speed) = speed else {
        // 未知协商速率：相对峰值（峰值恒 ≥ 当前值，结果落在 0~100）
        let rel =
            |v: u64, p: u64| Some(((v as f64 / p.max(1) as f64) * 100.0).clamp(0.0, 100.0) as f32);
        return (rel(tx, peak.1), rel(rx, peak.0));
    };
    // 协商速率是 Mbps（10^6 bit/s），换算成字节/秒作为满环基准
    let cap_bytes = speed as f64 * 1_000_000.0 / 8.0;
    let pct = |v: u64| Some(((v as f64 / cap_bytes) * 100.0).clamp(0.0, 100.0) as f32);
    (pct(tx), pct(rx))
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
        tx = font.draw(
            px,
            tx,
            bl,
            FONT_SIZE_SMALL,
            p,
            color(pc.0, pc.1, pc.2, pc.3),
        );
        // 前缀与数值之间保留一个字符间距
        tx += font.measure(FONT_SIZE_SMALL, "R");
    }
    font.draw(
        px,
        tx,
        bl,
        FONT_SIZE_SMALL,
        value,
        color(vc.0, vc.1, vc.2, vc.3),
    )
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
pub(crate) fn draw_rings(
    px: &mut Pixmap,
    frame: &Frame,
    font: Option<&Font>,
    net_peak: (u64, u64),
) {
    let Some(font) = font else { return };
    // 三环直接浮在任务栏上，文字对比度取决于任务栏底色（不是应用主题）：
    // 深色任务栏用浅色标注 + 提亮后的分级色，亮色任务栏沿用主题配色。
    let dark_taskbar = frame.pinned && !frame.taskbar_light;
    let base = if dark_taskbar { PIN_FG_DIM } else { FG_DIM };
    let d = if dark_taskbar {
        base
    } else {
        text_c(frame.dark, base)
    };
    let dim = color(d.0, d.1, d.2, d.3);
    let tint = |c: (u8, u8, u8, u8)| if dark_taskbar { ring_text_color(c) } else { c };

    if !frame.has_data {
        // 无数据时只画一个居中的状态环（灰轨道）+ 状态文字。
        // 深色任务栏没有 apply_dark 兜底，这里必须直接取浅色文案。
        let (text, c) = status_text(frame.conn, frame.dark && !dark_taskbar);
        let cx = RINGS_W as f32 / 2.0;
        draw_ring(px, cx, RING_CY, RING_R, RING_STROKE, None, TRACK);
        draw_ring_inner(
            px,
            font,
            cx,
            RING_CY,
            &text,
            c,
            "",
            dim,
            RING_VALUE_FS,
            RING_LABEL_FS,
            false,
        );
        return;
    }

    let s = frame.snap;
    let cpu_pct = s.cpu.as_ref().and_then(|c| c.usage).map(|v| v as f32);
    let mem_pct = s.memory.as_ref().and_then(|m| m.usage).map(|v| v as f32);
    let (nr, nw) = net_io_sum(s);

    // CPU 环：数值（分色）+ 环内小字标注
    let cpu_color = cpu_pct.map_or(ACCENT, usage_color);
    let cpu_text_color = tint(cpu_color);
    draw_ring(
        px,
        RING_CX[0],
        RING_CY,
        RING_R,
        RING_STROKE,
        cpu_pct,
        cpu_color,
    );
    let cpu_text = cpu_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    draw_ring_inner(
        px,
        font,
        RING_CX[0],
        RING_CY,
        &cpu_text,
        color(
            cpu_text_color.0,
            cpu_text_color.1,
            cpu_text_color.2,
            cpu_text_color.3,
        ),
        "CPU",
        dim,
        RING_VALUE_FS,
        RING_LABEL_FS,
        false,
    );

    // 内存环
    let mem_color = mem_pct.map_or(ACCENT, usage_color);
    let mem_text_color = tint(mem_color);
    draw_ring(
        px,
        RING_CX[1],
        RING_CY,
        RING_R,
        RING_STROKE,
        mem_pct,
        mem_color,
    );
    let mem_text = mem_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    draw_ring_inner(
        px,
        font,
        RING_CX[1],
        RING_CY,
        &mem_text,
        color(
            mem_text_color.0,
            mem_text_color.1,
            mem_text_color.2,
            mem_text_color.3,
        ),
        "内存",
        dim,
        RING_VALUE_FS,
        RING_LABEL_FS,
        false,
    );

    // 网络环：上半＝上行（橙）、下半＝下行（紫）。分母优先取网卡协商速率，
    // 读不到时退化为会话峰值相对刻度。环内两行数值不再带 UL/DL 前缀——
    // 方向由颜色（上橙下紫）与所在半环表达，腾出的宽度用于放大字号
    let (ul_pct, dl_pct) = net_ring_pct(s, net_peak);
    draw_ring_split(
        px,
        RING_CX[2],
        RING_CY,
        RING_R,
        RING_STROKE,
        ul_pct,
        TX_LINE,
        dl_pct,
        DL_PURPLE,
    );
    draw_net_inner(
        px,
        font,
        RING_CX[2],
        RING_CY,
        &fmt::rate_compact(nw),
        &fmt::rate_compact(nr),
        RING_NET_FS,
        false,
        false,
    );
}

/// 竖排收起态：三环纵向堆叠（CPU / 内存 / 网络），环更大、字号相应放大。
/// 与任务栏三环共用 draw_ring / draw_ring_split / 环内文字的画法。
pub(crate) fn draw_rings_v(
    px: &mut Pixmap,
    frame: &Frame,
    font: Option<&Font>,
    ox: f32,
    oy: f32,
    net_peak: (u64, u64),
) {
    let Some(font) = font else { return };
    // 暗色主题下环内标注统一提白（同 draw_rings）
    let d = text_c(frame.dark, FG_DIM);
    let dim = color(d.0, d.1, d.2, d.3);
    let cx = ox + RING_V_CX;

    if !frame.has_data {
        // 无数据：中环画灰轨道，环内放短状态文案（竖条窄，长句放不下）
        let (text, c) = status_text_short(frame.conn, frame.dark);
        let cy = oy + RING_V_CY[1];
        draw_ring(px, cx, cy, RING_V_R, RING_V_STROKE, None, TRACK);
        let tw = font.measure_bold(RING_V_NET_FS, &text);
        font.draw_bold(
            px,
            cx - tw / 2.0,
            cy + RING_V_NET_FS * 0.36,
            RING_V_NET_FS,
            &text,
            c,
        );
        return;
    }

    let s = frame.snap;
    let cpu_pct = s.cpu.as_ref().and_then(|c| c.usage).map(|v| v as f32);
    let mem_pct = s.memory.as_ref().and_then(|m| m.usage).map(|v| v as f32);
    let (nr, nw) = net_io_sum(s);

    // CPU 环
    let cpu_color = cpu_pct.map_or(ACCENT, usage_color);
    let cy0 = oy + RING_V_CY[0];
    draw_ring(px, cx, cy0, RING_V_R, RING_V_STROKE, cpu_pct, cpu_color);
    let cpu_text = cpu_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    let c0 = text_c(frame.dark, cpu_color);
    draw_ring_inner(
        px,
        font,
        cx,
        cy0,
        &cpu_text,
        color(c0.0, c0.1, c0.2, c0.3),
        "CPU",
        dim,
        RING_V_VALUE_FS,
        RING_V_LABEL_FS,
        true,
    );

    // 内存环
    let mem_color = mem_pct.map_or(ACCENT, usage_color);
    let cy1 = oy + RING_V_CY[1];
    draw_ring(px, cx, cy1, RING_V_R, RING_V_STROKE, mem_pct, mem_color);
    let mem_text = mem_pct.map_or_else(|| "--%".into(), |v| format!("{:.0}%", v.round()));
    let m1 = text_c(frame.dark, mem_color);
    draw_ring_inner(
        px,
        font,
        cx,
        cy1,
        &mem_text,
        color(m1.0, m1.1, m1.2, m1.3),
        "内存",
        dim,
        RING_V_VALUE_FS,
        RING_V_LABEL_FS,
        true,
    );

    // 网络环：上半橙＝上行、下半紫＝下行
    let (ul_pct, dl_pct) = net_ring_pct(s, net_peak);
    let cy2 = oy + RING_V_CY[2];
    draw_ring_split(
        px,
        cx,
        cy2,
        RING_V_R,
        RING_V_STROKE,
        ul_pct,
        TX_LINE,
        dl_pct,
        DL_PURPLE,
    );
    draw_net_inner(
        px,
        font,
        cx,
        cy2,
        &fmt::rate_compact(nw),
        &fmt::rate_compact(nr),
        RING_V_NET_FS,
        true,
        frame.dark,
    );
}

/// 环内文字：上行数值（分色）+ 下行灰色小字标注，两行都收在环内。
/// 字号由调用方给定（横排三环小、竖排三环大），行位按字号比例排布，大小环都居中；
/// `bold` 供竖排三环用粗体（环小、字要顶得住）。
#[allow(clippy::too_many_arguments)]
fn draw_ring_inner(
    px: &mut Pixmap,
    font: &Font,
    cx: f32,
    cy: f32,
    value: &str,
    vc: Color,
    label: &str,
    dim: Color,
    value_fs: f32,
    label_fs: f32,
    bold: bool,
) {
    let measure = |size: f32, t: &str| {
        if bold {
            font.measure_bold(size, t)
        } else {
            font.measure(size, t)
        }
    };
    let put = |px: &mut Pixmap, x: f32, baseline: f32, size: f32, t: &str, c: Color| {
        if bold {
            font.draw_bold(px, x, baseline, size, t, c);
        } else {
            font.draw(px, x, baseline, size, t, c);
        }
    };
    let vw = measure(value_fs, value);
    put(px, cx - vw / 2.0, cy - value_fs * 0.17, value_fs, value, vc);
    if !label.is_empty() {
        let lw = measure(label_fs, label);
        put(
            px,
            cx - lw / 2.0,
            cy + label_fs * 1.45,
            label_fs,
            label,
            dim,
        );
    }
}

/// 网络环内文字：上行（橙）/ 下行（紫）两行速率数值，行位置与半环颜色一一对应。
/// `dark`（仅竖排环用）：暗色下文字统一走中性色，随后被 apply_dark 提白。
fn draw_net_inner(
    px: &mut Pixmap,
    font: &Font,
    cx: f32,
    cy: f32,
    ul: &str,
    dl: &str,
    fs: f32,
    bold: bool,
    dark: bool,
) {
    let cu = text_c(dark, TX_LINE);
    let cd = text_c(dark, DL_PURPLE);
    let measure = |t: &str| {
        if bold {
            font.measure_bold(fs, t)
        } else {
            font.measure(fs, t)
        }
    };
    let put = |px: &mut Pixmap, x: f32, baseline: f32, t: &str, c: Color| {
        if bold {
            font.draw_bold(px, x, baseline, fs, t, c);
        } else {
            font.draw(px, x, baseline, fs, t, c);
        }
    };
    let uw = measure(ul);
    let dw = measure(dl);
    put(
        px,
        cx - uw / 2.0,
        cy - fs * 0.17,
        ul,
        color(cu.0, cu.1, cu.2, cu.3),
    );
    put(
        px,
        cx - dw / 2.0,
        cy + fs * 0.87,
        dl,
        color(cd.0, cd.1, cd.2, cd.3),
    );
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

#[cfg(all(test, feature = "zzz_old_video"))]
mod video_full_old {
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
                    read_bytes_per_sec: Some(
                        (100_000_000.0 + (t * 3.0).sin() * 50_000_000.0) as u64,
                    ),
                    write_bytes_per_sec: Some(
                        (80_000_000.0 + (t * 2.5).cos() * 40_000_000.0) as u64,
                    ),
                    used_bytes: Some(800 * 1024 * 1024 * 1024),
                    total_bytes: Some(1000 * 1024 * 1024 * 1024),
                },
                Disk {
                    device: "sdb".into(),
                    read_bytes_per_sec: Some(
                        (50_000_000.0 + (t * 2.0).cos() * 20_000_000.0) as u64,
                    ),
                    write_bytes_per_sec: Some(
                        (120_000_000.0 + (t * 3.5).sin() * 60_000_000.0) as u64,
                    ),
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
            let mem_pct =
                (snap.memory.as_ref().and_then(|m| m.usage).unwrap_or(0.0) / 100.0) as f32;
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
}

// ============================================================================
// 全功能宣传视频（深色主题版）：忠实复刻产品 UI，含镜头推进/特写/交互动画
// ============================================================================
#[cfg(test)]
mod video2 {
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
                    read_bytes_per_sec: Some(
                        (100_000_000.0 + (t * 3.0).sin() * 50_000_000.0) as u64,
                    ),
                    write_bytes_per_sec: Some(
                        (80_000_000.0 + (t * 2.5).cos() * 40_000_000.0) as u64,
                    ),
                    used_bytes: Some(800 * 1024 * 1024 * 1024),
                    total_bytes: Some(1000 * 1024 * 1024 * 1024),
                },
                Disk {
                    device: "sdb".into(),
                    read_bytes_per_sec: Some(
                        (50_000_000.0 + (t * 2.0).cos() * 20_000_000.0) as u64,
                    ),
                    write_bytes_per_sec: Some(
                        (120_000_000.0 + (t * 3.5).sin() * 60_000_000.0) as u64,
                    ),
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
    fn ring(
        px: &mut Pixmap,
        cx: f32,
        cy: f32,
        r: f32,
        wd: f32,
        pct: Option<f32>,
        c: C4,
        track_a: u8,
    ) {
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
    fn ring_split(
        px: &mut Pixmap,
        cx: f32,
        cy: f32,
        r: f32,
        wd: f32,
        up: f32,
        down: f32,
        track_a: u8,
    ) {
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
}

// ============================================================================
// 横屏版 1920x1080（30fps / 30s / 同 6 场景时长，复用 v8 音轨）
#[cfg(test)]
mod video_h {
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
                    read_bytes_per_sec: Some(
                        (100_000_000.0 + (t * 3.0).sin() * 50_000_000.0) as u64,
                    ),
                    write_bytes_per_sec: Some(
                        (80_000_000.0 + (t * 2.5).cos() * 40_000_000.0) as u64,
                    ),
                    used_bytes: Some(800 * 1024 * 1024 * 1024),
                    total_bytes: Some(1000 * 1024 * 1024 * 1024),
                },
                Disk {
                    device: "sdb".into(),
                    read_bytes_per_sec: Some(
                        (50_000_000.0 + (t * 2.0).cos() * 20_000_000.0) as u64,
                    ),
                    write_bytes_per_sec: Some(
                        (120_000_000.0 + (t * 3.5).sin() * 60_000_000.0) as u64,
                    ),
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

    fn ring(
        px: &mut Pixmap,
        cx: f32,
        cy: f32,
        r: f32,
        wd: f32,
        pct: Option<f32>,
        c: C4,
        track_a: u8,
    ) {
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

    fn ring_split(
        px: &mut Pixmap,
        cx: f32,
        cy: f32,
        r: f32,
        wd: f32,
        up: f32,
        down: f32,
        track_a: u8,
    ) {
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
}
