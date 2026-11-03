//! 6 字节命令帧。只允许旧系统已验证存在的 0x6D（开屏）与 0x6E（亮度）。

use crate::error::ProtocolError;

pub const CMD_SCREEN_ON: u8 = 0x6D;
pub const CMD_BRIGHTNESS: u8 = 0x6E;

/// 与旧实现一致：`[0,0,0,0,0,code]`。
pub fn command(code: u8) -> Result<[u8; 6], ProtocolError> {
    match code {
        CMD_SCREEN_ON | CMD_BRIGHTNESS => Ok([0, 0, 0, 0, 0, code]),
        other => Err(ProtocolError::UnknownCommand(other)),
    }
}

/// 亮度等级编码未验证（`Unknown`）：只允许复现 0 值。
pub fn brightness_zero() -> [u8; 6] {
    [0, 0, 0, 0, 0, CMD_BRIGHTNESS]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_commands_are_allowed() {
        assert_eq!(command(CMD_SCREEN_ON).unwrap(), [0, 0, 0, 0, 0, 0x6D]);
        assert_eq!(command(CMD_BRIGHTNESS).unwrap(), brightness_zero());
    }

    #[test]
    fn unknown_or_forbidden_commands_are_rejected() {
        assert_eq!(command(0x79), Err(ProtocolError::UnknownCommand(0x79)));
        assert!(command(0x00).is_err());
        assert!(command(0x6F).is_err());
    }
}
