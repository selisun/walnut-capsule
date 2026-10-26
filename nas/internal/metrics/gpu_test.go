package metrics

import (
	"os"
	"testing"
	"time"
)

// 驱动未加载（没有 /sys/module/nvidia、/dev/nvidiactl）时，独显采集必须直接跳过，
// 不能去执行 nvidia-smi——那会连带触发 nvidia-modprobe → modprobe，
// 在驱动被拒绝的机器上造成持续内核态 CPU 占用。
func TestSampleNvidiaSkipsWhenDriverMissing(t *testing.T) {
	if _, err := os.Stat(nvidiaModDir); err == nil {
		t.Skipf("本机存在 %s（NVIDIA 驱动已加载），跳过", nvidiaModDir)
	}
	s := &gpuSampler{
		path:     "/bin/true", // 若被调用会成功返回，便于暴露误调用
		igPrev:   map[string]uint64{},
		igPrevAt: map[string]time.Time{},
	}
	if got := s.sampleNvidia(); got != nil {
		t.Fatalf("驱动未加载时应返回 nil，实际 %v", got)
	}
	if got := s.Status(); got != gpuStatusNoDrv {
		t.Fatalf("状态应为 %q，实际 %q", gpuStatusNoDrv, got)
	}
	if !s.nvNextTry.IsZero() {
		t.Fatalf("未执行 nvidia-smi 不应进入退避，实际 %v", s.nvNextTry)
	}
}
