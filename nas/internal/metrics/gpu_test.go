package metrics

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// 驱动未加载（没有 /sys/module/nvidia）时，独显采集必须直接跳过，
// 不能去执行 nvidia-smi——那会连带触发 nvidia-modprobe → modprobe，
// 在驱动被拒绝的机器上造成持续内核态 CPU 占用。
func TestSampleNvidiaSkipsWhenDriverMissing(t *testing.T) {
	called := false
	s := &gpuSampler{
		nvPath:    "/bin/true", // 若被调用会成功返回，便于暴露误调用
		nvModDir:  filepath.Join(t.TempDir(), "missing-nvidia"),
		nvDevNode: filepath.Join(t.TempDir(), "missing-nvidiactl"),
		igPrev:    map[string]rc6Sample{},
		nvRun: func(context.Context, string) ([]byte, error) {
			called = true
			return []byte("0, fake, 40, 10, 100, 1000\n"), nil
		},
	}
	if got := s.sampleNvidia(time.Now()); got != nil {
		t.Fatalf("驱动未加载时应返回 nil，实际 %v", got)
	}
	if called {
		t.Fatal("驱动未加载时绝不允许执行 nvidia-smi")
	}
	if s.nvState != gpuStatusNoDrv {
		t.Fatalf("状态应为 %q，实际 %q", gpuStatusNoDrv, s.nvState)
	}
	if !s.nvNextTry.IsZero() {
		t.Fatalf("未执行 nvidia-smi 不应进入退避，实际 %v", s.nvNextTry)
	}
}

// 模块已加载但设备节点缺失：同样只提示、不执行。
func TestSampleNvidiaSkipsWhenDeviceNodeMissing(t *testing.T) {
	modDir := filepath.Join(t.TempDir(), "nvidia")
	if err := os.MkdirAll(modDir, 0o755); err != nil {
		t.Fatal(err)
	}
	called := false
	s := &gpuSampler{
		nvPath:    "/bin/true",
		nvModDir:  modDir,
		nvDevNode: filepath.Join(t.TempDir(), "nvidiactl"),
		igPrev:    map[string]rc6Sample{},
		nvRun: func(context.Context, string) ([]byte, error) {
			called = true
			return nil, nil
		},
	}
	if got := s.sampleNvidia(time.Now()); got != nil {
		t.Fatalf("设备节点缺失时应返回 nil，实际 %v", got)
	}
	if called {
		t.Fatal("设备节点缺失时绝不允许执行 nvidia-smi")
	}
	if s.nvState != gpuStatusNoNode {
		t.Fatalf("状态应为 %q，实际 %q", gpuStatusNoNode, s.nvState)
	}
}

// 没有 nvidia-smi 时只标记「无驱动工具」，不进入退避。
func TestSampleNvidiaWithoutTool(t *testing.T) {
	s := &gpuSampler{igPrev: map[string]rc6Sample{}}
	if got := s.sampleNvidia(time.Now()); got != nil {
		t.Fatalf("没有 nvidia-smi 时应返回 nil，实际 %v", got)
	}
	if s.nvState != gpuStatusNoTool {
		t.Fatalf("状态应为 %q，实际 %q", gpuStatusNoTool, s.nvState)
	}
}

// 执行失败后必须退避：退避期内不得再次启动进程。
func TestSampleNvidiaHonorsBackoff(t *testing.T) {
	dir := t.TempDir()
	modDir := filepath.Join(dir, "nvidia")
	nodePath := filepath.Join(dir, "nvidiactl")
	if err := os.MkdirAll(modDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(nodePath, []byte{}, 0o644); err != nil {
		t.Fatal(err)
	}
	calls := 0
	s := &gpuSampler{
		nvPath:    "/bin/false",
		nvModDir:  modDir,
		nvDevNode: nodePath,
		igPrev:    map[string]rc6Sample{},
		nvRun: func(context.Context, string) ([]byte, error) {
			calls++
			return nil, errors.New("boom")
		},
	}
	now := time.Now()
	if got := s.sampleNvidia(now); len(got) != 0 {
		t.Fatalf("失败时应返回空，实际 %v", got)
	}
	if calls != 1 {
		t.Fatalf("首次失败应只调用一次，实际 %d", calls)
	}
	if s.nvState != gpuStatusFailed {
		t.Fatalf("状态应为 %q，实际 %q", gpuStatusFailed, s.nvState)
	}
	// 退避期内（60s 内）再采两轮，都不应执行命令
	s.sampleNvidia(now.Add(10 * time.Second))
	s.sampleNvidia(now.Add(59 * time.Second))
	if calls != 1 {
		t.Fatalf("退避期内不得重复执行，实际调用 %d 次", calls)
	}
	// 退避到期后允许重试
	s.sampleNvidia(now.Add(61 * time.Second))
	if calls != 2 {
		t.Fatalf("退避到期后应重试一次，实际调用 %d 次", calls)
	}
}

func TestNVBackoffSchedule(t *testing.T) {
	cases := []struct {
		fails int
		want  time.Duration
	}{
		{1, 60 * time.Second},
		{2, 300 * time.Second},
		{3, 600 * time.Second},
		{4, 1200 * time.Second},
		{5, nvBackoffMax},
		{20, nvBackoffMax},
	}
	for _, c := range cases {
		if got := nvBackoff(c.fails); got != c.want {
			t.Fatalf("nvBackoff(%d)=%v，期望 %v", c.fails, got, c.want)
		}
	}
}

func TestParseNvidiaCSV(t *testing.T) {
	raw := strings.Join([]string{
		"0, NVIDIA GeForce RTX 3060, 55, 37, 2048, 12288",
		"1, Tesla T4, 40, 0, 0, 15360",
		"",                            // 空行
		"x, bad, 1, 2, 3, 4",          // index 非法
		"2, Bad Temp, -40, 150, 1, 2", // 温度/使用率越界 → 该两项为 nil
		"3, Huge VRAM, 50, 10, 999999999999, 999999999999", // 超上限 → 显存为 nil
	}, "\n")
	gpus := parseNvidiaCSV([]byte(raw))
	if len(gpus) != 4 {
		t.Fatalf("应解析出 4 块卡，实际 %d：%+v", len(gpus), gpus)
	}
	if gpus[0].Name != "NVIDIA GeForce RTX 3060" || gpus[0].Vendor != "nvidia" ||
		gpus[0].Type != gpuTypeDGPU || gpus[0].Dev != "nvidia:0" {
		t.Fatalf("第一块卡字段异常：%+v", gpus[0])
	}
	if gpus[0].Utilization == nil || *gpus[0].Utilization != 37 {
		t.Fatalf("使用率解析异常：%+v", gpus[0].Utilization)
	}
	if gpus[0].MemTotal == nil || *gpus[0].MemTotal != 12288*1024*1024 {
		t.Fatalf("显存换算异常：%+v", gpus[0].MemTotal)
	}
	// 越界值必须为 nil，不能伪造成正常数据
	if gpus[2].Temp != nil || gpus[2].Utilization != nil {
		t.Fatalf("越界温度/使用率应丢弃：%+v", gpus[2])
	}
	if gpus[2].Status != gpuItemError && gpus[2].Status != gpuItemPartial {
		t.Fatalf("越界数据的状态异常：%q", gpus[2].Status)
	}
	if gpus[3].MemUsed != nil || gpus[3].MemTotal != nil {
		t.Fatalf("超大显存应丢弃：%+v", gpus[3])
	}
}

func TestParseNvidiaCSVRejectsUsedGreaterThanTotal(t *testing.T) {
	gpus := parseNvidiaCSV([]byte("0, X, 40, 10, 999, 1\n"))
	if len(gpus) != 1 {
		t.Fatalf("应解析出 1 块卡，实际 %d", len(gpus))
	}
	if gpus[0].MemTotal != nil {
		t.Fatalf("已用 > 总量时应丢弃总量，实际 %v", *gpus[0].MemTotal)
	}
}

func TestLimitedBufferCapsOutput(t *testing.T) {
	var b limitedBuffer
	b.max = 8
	n, err := b.Write([]byte("0123456789abcdef"))
	if err != nil || n != 16 {
		t.Fatalf("Write 应报告完整长度，n=%d err=%v", n, err)
	}
	if got := b.String(); got != "01234567" {
		t.Fatalf("输出应截断到 8 字节，实际 %q", got)
	}
}

func TestGPUNameSanitize(t *testing.T) {
	if got := sanitizeGPUName("  Intel\tUHD\x00 630 \n", "GPU"); got != "Intel UHD 630" {
		t.Fatalf("名称清理异常：%q", got)
	}
	if got := sanitizeGPUName("   ", "NVIDIA GPU"); got != "NVIDIA GPU" {
		t.Fatalf("空名称应回退：%q", got)
	}
	if got := sanitizeGPUName(strings.Repeat("a", 200), "GPU"); len([]rune(got)) > 65 {
		t.Fatalf("过长名称应截断：%d", len(got))
	}
}
