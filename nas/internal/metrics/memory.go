package metrics

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
)

// memSampler 读取 /proc/meminfo（MemTotal / MemAvailable）。
type memSampler struct {
	proc string

	mu    sync.RWMutex
	usage float64
	used  uint64
	total uint64
}

func newMemSampler(proc string) *memSampler {
	return &memSampler{proc: proc}
}

func (s *memSampler) snapshot() (float64, uint64, uint64) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.usage, s.used, s.total
}

func (s *memSampler) sample() {
	b, err := os.ReadFile(filepath.Join(s.proc, "meminfo"))
	if err != nil {
		return
	}
	var total, avail uint64
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
		case "MemAvailable:":
			avail = v * 1024
		}
	}
	if total == 0 {
		return
	}
	used := total - avail
	s.mu.Lock()
	s.usage = float64(used) / float64(total) * 100
	s.used, s.total = used, total
	s.mu.Unlock()
}
