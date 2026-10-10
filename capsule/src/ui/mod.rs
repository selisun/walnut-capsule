//! 视图层。View trait 当前实现 CapsuleView，其余形态预留。

pub mod capsule_view;

// 预留形态（留口，暂未实现）：
// pub mod bar_view;
// pub mod ring_view;

use crate::data::Frame;
use crate::font::Font;
use tiny_skia::Pixmap;

pub trait View {
    /// 渲染一帧到像素缓冲（premultiplied RGBA）。
    /// `vis` 为本帧可视区域尺寸（可能小于像素缓冲的实际尺寸）。
    /// 传入的 Frame 是锁外快照，渲染全程不与数据源线程争锁。
    /// 返回 false 表示本帧画面无变化、调用方应跳过本次上屏提交。
    fn render(&mut self, px: &mut Pixmap, frame: &Frame, font: Option<&Font>, vis: (u32, u32)) -> bool;
    /// 是否处于展开视觉态（含展开动画过程中）。
    fn expanded(&self) -> bool;
    /// 立即跳到目标展开态（不做尺寸动画）。固定在任务栏时面板使用（三环常驻任务栏，面板不做形变）。
    fn jump_anim(&mut self) {}
    /// 设置展开目标态。
    fn set_expanded(&mut self, expanded: bool);
    /// 推进展开动画；返回 true 表示动画仍在进行。
    fn tick(&mut self) -> bool;
    /// 当前帧可视窗口尺寸（随展开动画变化）。
    fn frame_size(&self, frame: &Frame) -> (u32, u32);
    /// 展开动画过程中的“可见面板高度”，供固定态的磨砂背景层跟随。
    fn visible_panel_h(&self, frame: &Frame) -> u32;
    /// 像素缓冲/DIB 的分配尺寸：展开动画期间一次性按全展开尺寸分配，
    /// 避免动画每帧重建像素缓冲与 GDI DIB；收起后回到小尺寸。
    fn buffer_size(&self, frame: &Frame) -> (u32, u32);
    /// 每像素行附加不透明度（0..255）；空切片表示全不透明。
    fn row_fade(&self) -> &[u8];
    /// 展开态顶栏设置齿轮的命中测试。默认形态无此按钮。
    fn hit_settings(&self, _x: i32, _y: i32) -> bool {
        false
    }
    /// 固定在任务栏时，按任务栏高度与形态调整收起态外观（药丸底垂直内缩以贴合任务栏，
    /// `mini` 为空间不足时的迷你形态）。返回 true 表示外观已变化、需要重排窗口。默认形态无此行为。
    fn set_taskbar_style(&mut self, _taskbar_h: i32, _mini: bool) -> bool {
        false
    }
    /// 固定在任务栏时的鼠标悬停态（药丸底悬停高亮）。返回 true 表示状态已变化。默认形态无此行为。
    fn set_hovered(&mut self, _on: bool) -> bool {
        false
    }
}
