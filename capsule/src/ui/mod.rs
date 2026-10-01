//! 视图层。View trait 当前实现 CapsuleView，其余形态预留。

pub mod capsule_view;

// 预留形态（留口，暂未实现）：
// pub mod bar_view;
// pub mod ring_view;

use crate::data::SharedState;
use crate::font::Font;
use tiny_skia::Pixmap;

pub trait View {
    /// 渲染一帧到像素缓冲（premultiplied RGBA）。
    fn render(&mut self, px: &mut Pixmap, state: &SharedState, font: Option<&Font>);
    /// 是否展开。
    fn expanded(&self) -> bool;
    fn set_expanded(&mut self, expanded: bool);
}
