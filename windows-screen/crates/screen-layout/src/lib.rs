//! # screen-layout
//!
//! 按设备逻辑尺寸生成布局：模板选择、网格计算、字体/间距缩放、边界与重叠校验。
//!
//! 不判断具体型号、不读取 VID/PID；320×480 / 480×320 只是测试基准之一。

pub mod engine;
pub mod error;
pub mod template;

pub use engine::build;
pub use error::LayoutError;
pub use template::TemplateKind;
