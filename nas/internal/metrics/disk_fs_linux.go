//go:build linux

package metrics

import "syscall"

// fsUsedTotal 返回挂载点的已用/总容量（字节）。
// 已用量按 total - Bavail 计算（Bavail 为普通用户可用块，与 df 口径一致）。
func fsUsedTotal(path string) (used, total uint64, ok bool) {
	var st syscall.Statfs_t
	if err := syscall.Statfs(path, &st); err != nil {
		return 0, 0, false
	}
	bsize := uint64(st.Bsize)
	total = bsize * st.Blocks
	used = total - bsize*st.Bavail
	return used, total, true
}
