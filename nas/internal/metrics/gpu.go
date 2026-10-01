package metrics

import (
	"context"
	"os/exec"
	"strconv"
	"strings"
	"sync"
	"time"
)

// gpuSampler 通过 nvidia-smi 采集（2~5s 间隔，由外层调度）。
// 无 nvidia-smi 时自动禁用，GPUs 返回空数组。
type gpuSampler struct {
	path string

	mu   sync.RWMutex
	gpus []GPU
}

func newGPUSampler() *gpuSampler {
	path, _ := exec.LookPath("nvidia-smi")
	return &gpuSampler{path: path}
}

func (s *gpuSampler) enabled() bool { return s.path != "" }

func (s *gpuSampler) snapshot() []GPU {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]GPU, len(s.gpus))
	copy(out, s.gpus)
	return out
}

func (s *gpuSampler) sample() {
	if !s.enabled() {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, s.path,
		"--query-gpu=index,name,temperature.gpu,utilization.gpu,memory.used,memory.total",
		"--format=csv,noheader,nounits").Output()
	if err != nil {
		return
	}
	var gpus []GPU
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		f := strings.Split(line, ",")
		if len(f) < 6 {
			continue
		}
		for i := range f {
			f[i] = strings.TrimSpace(f[i])
		}
		idx, err := strconv.Atoi(f[0])
		if err != nil {
			continue
		}
		g := GPU{Index: idx, Name: f[1]}
		g.Temp = parseIntOrNil(f[2])
		g.Utilization = parseIntOrNil(f[3])
		g.MemUsed = parseMiB(f[4])
		g.MemTotal = parseMiB(f[5])
		gpus = append(gpus, g)
	}
	s.mu.Lock()
	s.gpus = gpus
	s.mu.Unlock()
}

func parseIntOrNil(s string) *int {
	v, err := strconv.Atoi(s)
	if err != nil {
		return nil
	}
	return &v
}

func parseMiB(s string) uint64 {
	v, err := strconv.ParseUint(s, 10, 64)
	if err != nil {
		return 0
	}
	return v * 1024 * 1024
}
