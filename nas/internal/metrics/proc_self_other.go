//go:build !linux

package metrics

// SelfUsage 非 Linux 平台不采集进程自身用量（部署目标仅 linux/amd64、linux/arm64）。
func SelfUsage() (float64, uint64) {
	return 0, 0
}
