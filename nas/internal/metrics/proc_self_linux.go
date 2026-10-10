//go:build linux

package metrics

import (
	"os"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"
)

var (
	selfMu        sync.Mutex
	selfLastTicks uint64
	selfLastAt    time.Time
)

// SelfUsage 返回本进程的 CPU 占用（相对整机容量，0~100）与常驻内存字节数。
// CPU 为两次采样间的增量，首次采样前返回 0。
func SelfUsage() (float64, uint64) {
	ticks, ok := selfCPUTicks()
	rss := selfRSS()
	now := time.Now()

	selfMu.Lock()
	defer selfMu.Unlock()

	var pct float64
	if ok && !selfLastAt.IsZero() {
		if dt := now.Sub(selfLastAt).Seconds(); dt > 0 && ticks >= selfLastTicks {
			// 1 tick = 10ms，先折算成占单核的百分比，再按核数归一化到整机
			pct = float64(ticks-selfLastTicks) / 100.0 / dt * 100.0 / float64(runtime.NumCPU())
		}
	}
	if ok {
		selfLastTicks, selfLastAt = ticks, now
	}
	return pct, rss
}

// selfCPUTicks 读取 /proc/self/stat 的 utime+stime（单位 tick）。
func selfCPUTicks() (uint64, bool) {
	b, err := os.ReadFile("/proc/self/stat")
	if err != nil {
		return 0, false
	}
	s := string(b)
	// comm（第 2 字段）可能含空格与括号，用最后一个 ')' 定位其后的字段序列
	i := strings.LastIndexByte(s, ')')
	if i < 0 || i+2 >= len(s) {
		return 0, false
	}
	f := strings.Fields(s[i+2:])
	// f[0] 为第 3 字段 state，故 utime(第 14) = f[11]，stime(第 15) = f[12]
	if len(f) < 13 {
		return 0, false
	}
	u, err1 := strconv.ParseUint(f[11], 10, 64)
	st, err2 := strconv.ParseUint(f[12], 10, 64)
	if err1 != nil || err2 != nil {
		return 0, false
	}
	return u + st, true
}

// selfRSS 读取 /proc/self/statm 的常驻页数并换算为字节。
func selfRSS() uint64 {
	b, err := os.ReadFile("/proc/self/statm")
	if err != nil {
		return 0
	}
	f := strings.Fields(string(b))
	if len(f) < 2 {
		return 0
	}
	p, err := strconv.ParseUint(f[1], 10, 64)
	if err != nil {
		return 0
	}
	return p * uint64(os.Getpagesize())
}

// 进程启动即建立基线，避免首次请求因无历史采样而恒为 0。
func init() { _, _ = SelfUsage() }
