//! # screen-render
//!
//! tiny-skia CPU 渲染骨架：画布、矩形绘制、旋转/镜像/反色、帧缓冲输出。
//!
//! 本阶段不连接设备、不做 RGB565 转换（属 `screen-protocol`），不绘制任何假数据。

pub mod canvas;
pub mod error;
pub mod font;
pub mod image;
pub mod legacy;
pub mod metrics;
pub mod pipeline;
pub mod text;
pub mod transform;

pub use canvas::Canvas;
pub use error::RenderError;
pub use image::BackgroundImage;
pub use legacy::{render_legacy, LegacyData};
pub use metrics::{bind, bind_with_clock, local_clock, ClockParts, MetricView};
pub use pipeline::{
    render, render_notice, render_notice_bg, render_with_bg, render_with_metrics,
    render_with_metrics_bg,
};
pub use text::{create_text_renderer, FontSpec, TextRenderer, UnavailableTextRenderer};
pub use transform::{invert, mirror_horizontal, orient, rotate180, rotate270, rotate90};
