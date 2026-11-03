//! # screen-protocol
//!
//! 3.5 英寸旧系统组合级真机基线的纯函数：候选帧头、RGB565LE、命令白名单、分块。
//! **不依赖任何 IO，也不代表官方协议标准。**

pub mod chunk;
pub mod command;
pub mod error;
pub mod header;
pub mod pixels;

pub use chunk::{chunk_ranges, LEGACY_CHUNK_SIZE};
pub use command::{brightness_zero, command, CMD_BRIGHTNESS, CMD_SCREEN_ON};
pub use error::ProtocolError;
pub use header::{frame_header, END_MARKER, HEADER_LEN, LEGACY_480X320_HEADER};
pub use pixels::{rgba_to_rgb565le, BYTES_PER_PIXEL};
