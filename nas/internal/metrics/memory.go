package metrics

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
)

// memSampler 读取 /proc/meminfo（MemTotal / MemFree / Buffers / Cached）。
// 展示口径与 Windows 任务管理器一致：已用 + 缓存 + 可用 = 总计，
// 其中「已用」不含可回收的缓存，「可用」是真正的空闲内存。
type memSampler struct {
	proc string

	mu    sync.RWMutex
	usage float64
	used  uint64
	total uint64
	cache uint64 // Buffers + Cached
	free  uint64 // MemFree
}

func newMemSampler(proc string) *memSampler {
	return &memSampler{proc: proc}
}

func (s *memSampler) snapshot() (float64, uint64, uint64, uint64, uint64) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.usage, s.used, s.total, s.cache, s.free
}

func (s *memSampler) sample() {
	b, err := os.ReadFile(filepath.Join(s.proc, "meminfo"))
	if err != nil {
		return
	}
	var total, free, buffers, cached uint64
	for _, line := range strings.Split(string(b), "\n") {
		f := strings.Fields(line)
		if len(f) < 2 {
			continue
		}
		v, err := strconv.ParseUint(f[1], 10, 64)
		if err != nil {
			continue
		}
		switch f[0] {
		case "MemTotal:":
			total = v * 1024
		case "MemFree:":
			free = v * 1024
		case "Buffers:":
			buffers = v * 1024
		case "Cached:":
			cached = v * 1024
		}
	}
	if total == 0 {
		return
	}
	cache := buffers + cached
	used := uint64(0)
	if total > free+cache {
		used = total - free - cache
	}
	s.mu.Lock()
	s.usage = float64(used) / float64(total) * 100
	s.used, s.total, s.cache, s.free = used, total, cache, free
	s.mu.Unlock()
}
