//go:build !linux

package metrics

// 非 Linux 平台不采集真实容量（部署目标仅 linux/amd64、linux/arm64）。
func fsUsedTotal(path string) (used, total uint64, ok bool) {
	return 0, 0, false
}
