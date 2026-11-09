package metrics

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// nvidiaEnv 构造「驱动已加载 + 设备节点存在」的假环境，返回模块目录与设备节点路径。
func nvidiaEnv(t *testing.T) (modDir, nodePath string) {
	t.Helper()
	dir := t.TempDir()
	modDir = filepath.Join(dir, "nvidia")
	nodePath = filepath.Join(dir, "nvidiactl")
	if err := os.MkdirAll(modDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(nodePath, []byte{}, 0o644); err != nil {
		t.Fatal(err)
	}
	return modDir, nodePath
}

// ---------------- AMD：伪造 /sys/class/drm ----------------

// amdgpu 独显全指标：使用率、显存、温度都应解析成功，状态为 ok。
func TestAMDSimulatedDGPUFullMetrics(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/boot_vga", "0\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "63\n")
	writeFake(t, sys, "class/drm/card0/device/mem_info_vram_used", "2147483648\n")   // 2 GiB
	writeFake(t, sys, "class/drm/card0/device/mem_info_vram_total", "17179869184\n") // 16 GiB
	writeFake(t, sys, "class/drm/card0/device/hwmon/hwmon2/temp1_input", "52000\n")

	s := newTestSampler(sys)
	s.sample()
	gpus := s.snapshot()
	if len(gpus) != 1 {
		t.Fatalf("应采集 1 块 AMD 独显，实际 %d：%+v", len(gpus), gpus)
	}
	g := gpus[0]
	if g.Vendor != "amd" || g.Type != gpuTypeDGPU || g.Name != "AMD 独显" {
		t.Fatalf("AMD 独显标识异常：%+v", g)
	}
	if g.Utilization == nil || *g.Utilization != 63 {
		t.Fatalf("使用率异常：%+v", g.Utilization)
	}
	if g.MemUsed == nil || *g.MemUsed != 2*1024*1024*1024 {
		t.Fatalf("显存已用异常：%+v", g.MemUsed)
	}
	if g.MemTotal == nil || *g.MemTotal != 16*1024*1024*1024 {
		t.Fatalf("显存总量异常：%+v", g.MemTotal)
	}
	if g.Temp == nil || *g.Temp != 52 {
		t.Fatalf("温度异常：%+v", g.Temp)
	}
	if g.Status != gpuItemOK {
		t.Fatalf("状态应为 ok，实际 %q", g.Status)
	}
	if s.Status() != gpuStatusOK {
		t.Fatalf("总体状态应为「正常」，实际 %q", s.Status())
	}
}

// AMD 核显没有独立显存统计：显存必须留空（N/A），不能拿共享内存冒充。
func TestAMDSimulatedIGPUHasNoVRAM(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/boot_vga", "1\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "12\n")

	s := newTestSampler(sys)
	s.sample()
	gpus := s.snapshot()
	if len(gpus) != 1 {
		t.Fatalf("应采集 1 块 AMD 核显，实际 %d", len(gpus))
	}
	g := gpus[0]
	if g.Type != gpuTypeIGPU || g.Name != "AMD 核显" {
		t.Fatalf("AMD 核显标识异常：%+v", g)
	}
	if g.MemUsed != nil || g.MemTotal != nil {
		t.Fatalf("核显不应有独立显存：used=%v total=%v", g.MemUsed, g.MemTotal)
	}
	if g.Status != gpuItemOK {
		t.Fatalf("有使用率时应为 ok，实际 %q", g.Status)
	}
}

// gpu_busy_percent 给越界值（amdgpu 偶发 101 / 负值）时必须丢弃，不伪造使用率。
func TestAMDBusyPercentOutOfRangeIsRejected(t *testing.T) {
	cases := []string{"101", "-1", "n/a", ""}
	for _, raw := range cases {
		sys := t.TempDir()
		writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
		writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", raw+"\n")
		writeFake(t, sys, "class/drm/card0/device/hwmon/hwmon0/temp1_input", "48000\n")

		s := newTestSampler(sys)
		s.sample()
		gpus := s.snapshot()
		if len(gpus) != 1 {
			t.Fatalf("[%q] 应识别 1 块卡，实际 %d", raw, len(gpus))
		}
		if gpus[0].Utilization != nil {
			t.Fatalf("[%q] 越界使用率应丢弃，实际 %v", raw, *gpus[0].Utilization)
		}
		if gpus[0].Status != gpuItemPartial {
			t.Fatalf("[%q] 只有温度时应为 partial，实际 %q", raw, gpus[0].Status)
		}
	}
}

// 显存接口边界：0 已用合法；总量为 0 / 超 16 TiB 视为不可用。
func TestAMDVRAMBoundedValues(t *testing.T) {
	sys := t.TempDir()
	card := filepath.Join(sys, "card0")

	// 已用 0 + 合法总量：0 是真实值，必须保留
	writeFake(t, sys, "card0/device/mem_info_vram_used", "0\n")
	writeFake(t, sys, "card0/device/mem_info_vram_total", "8589934592\n")
	used, total := amdVRAM(card)
	if used == nil || *used != 0 {
		t.Fatalf("已用 0 应保留，实际 %v", used)
	}
	if total == nil || *total != 8589934592 {
		t.Fatalf("合法总量应保留，实际 %v", total)
	}

	// 总量为 0：丢弃
	writeFake(t, sys, "card0/device/mem_info_vram_total", "0\n")
	if _, total := amdVRAM(card); total != nil {
		t.Fatalf("总量 0 应丢弃，实际 %v", *total)
	}

	// 总量超过 16 TiB：丢弃
	writeFake(t, sys, "card0/device/mem_info_vram_total", strconv.FormatUint(maxVRAMBytes+1, 10)+"\n")
	if _, total := amdVRAM(card); total != nil {
		t.Fatalf("超上限总量应丢弃，实际 %v", *total)
	}

	// 文件缺失：两项都为 nil
	if used, total := amdVRAM(filepath.Join(sys, "card-missing")); used != nil || total != nil {
		t.Fatalf("接口缺失时应为 nil，实际 used=%v total=%v", used, total)
	}
}

// 温度越界（0℃ / 200℃）必须丢弃，但使用率等其它指标继续采集。
func TestAMDTempOutOfRangeIsRejected(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "20\n")
	writeFake(t, sys, "class/drm/card0/device/hwmon/hwmon0/temp1_input", "0\n")
	writeFake(t, sys, "class/drm/card0/device/hwmon/hwmon1/temp1_input", "200000\n")

	s := newTestSampler(sys)
	s.sample()
	gpus := s.snapshot()
	if len(gpus) != 1 {
		t.Fatalf("应识别 1 块卡，实际 %d", len(gpus))
	}
	if gpus[0].Temp != nil {
		t.Fatalf("越界温度应丢弃，实际 %v", *gpus[0].Temp)
	}
	if gpus[0].Utilization == nil || *gpus[0].Utilization != 20 {
		t.Fatalf("温度越界不应影响使用率：%+v", gpus[0].Utilization)
	}
}

// 多卡隔离：一块 AMD 全指标正常、另一块完全不可读，互不影响。
func TestAMDMultiCardIsolation(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/boot_vga", "0\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "44\n")
	writeFake(t, sys, "class/drm/card1/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card1/device/boot_vga", "0\n")

	s := newTestSampler(sys)
	s.sample()
	gpus := s.snapshot()
	if len(gpus) != 2 {
		t.Fatalf("应采集 2 块卡，实际 %d：%+v", len(gpus), gpus)
	}
	if gpus[0].Status != gpuItemOK || gpus[1].Status != gpuItemError {
		t.Fatalf("单卡状态隔离失败：%+v", gpus)
	}
	if s.Status() != gpuStatusPartial {
		t.Fatalf("一块可用一块失败时应为「部分可用」，实际 %q", s.Status())
	}
}

// 设备发现缓存 30s：缓存期内新增的卡不出现，失效后立刻出现。
func TestDRMDiscoveryCacheExpires(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "10\n")

	s := newTestSampler(sys)
	s.sample()
	if len(s.snapshot()) != 1 {
		t.Fatalf("初始应 1 块卡：%+v", s.snapshot())
	}

	writeFake(t, sys, "class/drm/card1/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card1/device/gpu_busy_percent", "20\n")
	s.sample()
	if len(s.snapshot()) != 1 {
		t.Fatalf("缓存期内不应重新发现：%+v", s.snapshot())
	}

	s.devAt = time.Time{} // 模拟缓存到期
	s.sample()
	if len(s.snapshot()) != 2 {
		t.Fatalf("缓存失效后应发现 2 块卡：%+v", s.snapshot())
	}
}

// Dev 标识必须稳定且唯一：假 sysfs 里 device 是普通目录时也要回退到卡名，
// 不能把所有卡都记成 "device"（否则多卡 RC6 基线串台、对外 Dev 重复）。
func TestDRMDevIDsAreUnique(t *testing.T) {
	sys := t.TempDir()
	for i := 0; i < 2; i++ {
		card := "class/drm/card" + strconv.Itoa(i)
		writeFake(t, sys, card+"/device/vendor", "0x8086\n")
		writeFake(t, sys, card+"/device/boot_vga", "1\n")
		writeFake(t, sys, card+"/gt/gt0/rc6_enable", "1\n")
		writeFake(t, sys, card+"/gt/gt0/rc6_residency_ms", "1000\n")
	}
	s := newTestSampler(sys)
	devs := s.discover(time.Now())
	if len(devs) != 2 {
		t.Fatalf("应发现 2 块卡，实际 %d", len(devs))
	}
	ids := map[string]bool{}
	for _, d := range devs {
		if d.id == "" || ids[d.id] {
			t.Fatalf("Dev 标识重复或为空：%q（全部 %+v）", d.id, devs)
		}
		ids[d.id] = true
	}

	// 两块卡的 RC6 基线必须各自独立：卡 1 增量 500ms/2s → 75%，卡 2 增量 3000ms/2s → 0%
	now := time.Now()
	if got := s.intelRC6Util(devs[0], now); got != nil {
		t.Fatalf("首次采样只建基线，实际 %v", *got)
	}
	if got := s.intelRC6Util(devs[1], now); got != nil {
		t.Fatalf("首次采样只建基线，实际 %v", *got)
	}
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_residency_ms", "1500\n")
	writeFake(t, sys, "class/drm/card1/gt/gt0/rc6_residency_ms", "4000\n")
	got0 := s.intelRC6Util(devs[0], now.Add(2*time.Second))
	got1 := s.intelRC6Util(devs[1], now.Add(2*time.Second))
	if got0 == nil || *got0 != 75 {
		t.Fatalf("卡 0 使用率应为 75，实际 %v", got0)
	}
	if got1 == nil || *got1 != 0 {
		t.Fatalf("卡 1 使用率应为 0（RC6 无中断），实际 %v", got1)
	}
}

// ---------------- NVIDIA：注入 nvidia-smi 输出 ----------------

// 两块 NVIDIA 与 Intel 核显并列，按 Index 排序且 Dev 唯一。
func TestNvidiaSimulatedMultiCardWithIntel(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x8086\n")
	writeFake(t, sys, "class/drm/card0/device/boot_vga", "1\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "5\n")
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	modDir, nodePath := nvidiaEnv(t)
	s := newTestSampler(sys)
	s.nvPath = "/bin/true"
	s.nvModDir = modDir
	s.nvDevNode = nodePath
	s.nvRun = func(context.Context, string) ([]byte, error) {
		return []byte("1, NVIDIA GeForce RTX 4090, 61, 88, 4096, 24576\n" +
			"0, Tesla T4, 45, 3, 512, 15360\n"), nil
	}
	s.sample()

	gpus := s.snapshot()
	if len(gpus) != 3 {
		t.Fatalf("应采集 3 块 GPU，实际 %d：%+v", len(gpus), gpus)
	}
	// 排序：Index 升序；同 Index 时按厂商字典序（intel < nvidia）
	want := []struct {
		vendor string
		dev    string
	}{
		{"intel", ""},
		{"nvidia", "nvidia:0"},
		{"nvidia", "nvidia:1"},
	}
	for i, w := range want {
		if gpus[i].Vendor != w.vendor {
			t.Fatalf("第 %d 块厂商应为 %q，实际 %+v", i, w.vendor, gpus[i])
		}
		if w.dev != "" && gpus[i].Dev != w.dev {
			t.Fatalf("第 %d 块 Dev 应为 %q，实际 %q", i, w.dev, gpus[i].Dev)
		}
	}
	if gpus[1].Name != "Tesla T4" || gpus[1].Utilization == nil || *gpus[1].Utilization != 3 {
		t.Fatalf("Tesla T4 解析异常：%+v", gpus[1])
	}
	if gpus[2].MemTotal == nil || *gpus[2].MemTotal != 24576*1024*1024 {
		t.Fatalf("RTX 4090 显存异常：%+v", gpus[2].MemTotal)
	}
	if s.Status() != gpuStatusOK {
		t.Fatalf("全部正常时总体应为「正常」，实际 %q", s.Status())
	}
}

// nvidia-smi 对新卡/受限字段会输出 [N/A]、[Not Supported]，必须丢弃而不是解析失败。
func TestParseNvidiaCSVNAValues(t *testing.T) {
	raw := []byte("0, NVIDIA H100, [N/A], [Not Supported], N/A, [N/A]\n")
	gpus := parseNvidiaCSV(raw)
	if len(gpus) != 1 {
		t.Fatalf("应解析出 1 块卡，实际 %d", len(gpus))
	}
	g := gpus[0]
	if g.Temp != nil || g.Utilization != nil || g.MemUsed != nil || g.MemTotal != nil {
		t.Fatalf("N/A 字段必须为 nil：%+v", g)
	}
	if g.Status != gpuItemError {
		t.Fatalf("无任何可用指标时应为 error，实际 %q", g.Status)
	}
	if g.Name != "NVIDIA H100" {
		t.Fatalf("名称应保留，实际 %q", g.Name)
	}
}

// 空输出、纯空白、列数不足、负 index 一律跳过，不产生假卡。
func TestParseNvidiaCSVGarbage(t *testing.T) {
	cases := [][]byte{
		nil,
		[]byte(""),
		[]byte(" \n\t\n"),
		[]byte("garbage\n1,2,3\n"),
		[]byte("-1, Bad Index, 40, 10, 1, 2\n"),
	}
	for _, raw := range cases {
		if got := parseNvidiaCSV(raw); len(got) != 0 {
			t.Fatalf("[%q] 应解析出 0 块卡，实际 %+v", raw, got)
		}
	}
}

// 执行超时：状态为「采集异常」、原因写明超时，并立即进入退避，退避期内不再启动进程。
func TestNvidiaTimeoutEntersBackoff(t *testing.T) {
	modDir, nodePath := nvidiaEnv(t)
	calls := 0
	s := &gpuSampler{
		nvPath:    "/bin/true",
		nvModDir:  modDir,
		nvDevNode: nodePath,
		igPrev:    map[string]rc6Sample{},
		nvRun: func(context.Context, string) ([]byte, error) {
			calls++
			return nil, errNvTimeout
		},
	}
	now := time.Now()
	if got := s.sampleNvidia(now); len(got) != 0 {
		t.Fatalf("超时应返回空，实际 %+v", got)
	}
	if s.nvState != gpuStatusFailed {
		t.Fatalf("状态应为 %q，实际 %q", gpuStatusFailed, s.nvState)
	}
	if !strings.Contains(s.nvReason, "超时") {
		t.Fatalf("失败原因应说明超时，实际 %q", s.nvReason)
	}
	if s.nvNextTry.IsZero() || s.nvNextTry.Before(now) {
		t.Fatalf("超时后应立即进入退避，实际 %v", s.nvNextTry)
	}
	s.sampleNvidia(now.Add(30 * time.Second))
	if calls != 1 {
		t.Fatalf("退避期内不得重复执行，实际调用 %d 次", calls)
	}
}

// 权限不足要能被识别出来（日志里要能指导用户加权限，而不是笼统「执行失败」）。
func TestNvidiaPermissionErrorReason(t *testing.T) {
	modDir, nodePath := nvidiaEnv(t)
	s := &gpuSampler{
		nvPath:    "/bin/true",
		nvModDir:  modDir,
		nvDevNode: nodePath,
		igPrev:    map[string]rc6Sample{},
		nvRun: func(context.Context, string) ([]byte, error) {
			return nil, fmt.Errorf("exec: %w", os.ErrPermission)
		},
	}
	s.sampleNvidia(time.Now())
	if !strings.Contains(s.nvReason, "权限") {
		t.Fatalf("失败原因应说明权限不足，实际 %q", s.nvReason)
	}
}

// 驱动恢复（模块 + 节点重新出现）后必须立刻重试，不能被之前的退避卡住。
func TestNvidiaDriverRestoredResetsBackoff(t *testing.T) {
	modDir, nodePath := nvidiaEnv(t)
	calls := 0
	s := &gpuSampler{
		nvPath:    "/bin/true",
		nvModDir:  modDir,
		nvDevNode: nodePath,
		igPrev:    map[string]rc6Sample{},
		nvRun: func(context.Context, string) ([]byte, error) {
			calls++
			return nil, errors.New("boom")
		},
	}
	t0 := time.Now()
	s.sampleNvidia(t0)
	s.sampleNvidia(t0.Add(61 * time.Second)) // 退避到期后第二次失败 → 退避拉长
	if s.nvFails != 2 || calls != 2 {
		t.Fatalf("应累计 2 次失败，实际 fails=%d calls=%d", s.nvFails, calls)
	}

	if err := os.Remove(modDir); err != nil {
		t.Fatal(err)
	}
	s.sampleNvidia(t0.Add(62 * time.Second))
	if calls != 2 || s.nvState != gpuStatusNoDrv {
		t.Fatalf("驱动消失时不应执行，实际 calls=%d state=%q", calls, s.nvState)
	}

	if err := os.MkdirAll(modDir, 0o755); err != nil {
		t.Fatal(err)
	}
	s.nvRun = func(context.Context, string) ([]byte, error) {
		return []byte("0, RTX 3060, 50, 20, 1000, 12288\n"), nil
	}
	got := s.sampleNvidia(t0.Add(62 * time.Second))
	if len(got) != 1 {
		t.Fatalf("驱动恢复后应立即采集到 1 块卡，实际 %+v（calls=%d）", got, calls)
	}
	if s.nvFails != 0 || !s.nvNextTry.IsZero() {
		t.Fatalf("恢复后应清空退避，实际 fails=%d next=%v", s.nvFails, s.nvNextTry)
	}
	if s.nvState != gpuStatusOK {
		t.Fatalf("恢复后状态应为 %q，实际 %q", gpuStatusOK, s.nvState)
	}
}

// 失败后成功：退避清零，下一轮立即重新执行（不残留旧退避）。
func TestNvidiaSuccessAfterFailureResetsBackoff(t *testing.T) {
	modDir, nodePath := nvidiaEnv(t)
	fail := true
	calls := 0
	s := &gpuSampler{
		nvPath:    "/bin/true",
		nvModDir:  modDir,
		nvDevNode: nodePath,
		igPrev:    map[string]rc6Sample{},
		nvRun: func(context.Context, string) ([]byte, error) {
			calls++
			if fail {
				return nil, errors.New("boom")
			}
			return []byte("0, RTX 3060, 50, 20, 1000, 12288\n"), nil
		},
	}
	t0 := time.Now()
	s.sampleNvidia(t0)
	if !s.nvNextTry.Equal(t0.Add(nvBackoffMin)) {
		t.Fatalf("首次失败应退避 %v，实际 %v", nvBackoffMin, s.nvNextTry)
	}

	fail = false
	got := s.sampleNvidia(t0.Add(nvBackoffMin + time.Second))
	if len(got) != 1 {
		t.Fatalf("退避到期后应采集成功，实际 %+v", got)
	}
	if s.nvFails != 0 || !s.nvNextTry.IsZero() {
		t.Fatalf("成功后应清零退避，实际 fails=%d next=%v", s.nvFails, s.nvNextTry)
	}
	// 清零后下一轮必须立即执行，而不是继续等下一档退避
	s.sampleNvidia(t0.Add(nvBackoffMin + 2*time.Second))
	if calls != 3 {
		t.Fatalf("退避清零后应立即执行，实际调用 %d 次", calls)
	}
}

// 只有 NVIDIA 实体卡且采集失败：总体必须是「采集异常」，不能报「未检测到」。
func TestNvidiaOnlyCardFailureAggregatesFailed(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")
	modDir, nodePath := nvidiaEnv(t)

	s := newTestSampler(sys)
	s.nvPath = "/bin/true"
	s.nvModDir = modDir
	s.nvDevNode = nodePath
	s.nvRun = func(context.Context, string) ([]byte, error) {
		return nil, errors.New("boom")
	}
	s.sample()
	if got := s.Status(); got != gpuStatusFailed {
		t.Fatalf("总体状态应为 %q，实际 %q", gpuStatusFailed, got)
	}
	gpus := s.snapshot()
	// 采集失败也要保留品牌占位卡（只有厂商，没有数值），供小屏选底图
	if len(gpus) != 1 || gpus[0].Vendor != "nvidia" || gpus[0].Status != gpuItemUnavailable {
		t.Fatalf("应保留一条 NVIDIA 品牌占位卡：%+v", gpus)
	}
	if gpus[0].Utilization != nil || gpus[0].Temp != nil || gpus[0].MemTotal != nil {
		t.Fatalf("占位卡不得伪造指标：%+v", gpus[0])
	}
}

// 有 NVIDIA 实体卡但驱动未加载：报「驱动未加载」，不能报「未检测到」。
func TestNvidiaPresentDriverMissingAggregatesNoDrv(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	s := newTestSampler(sys)
	s.nvPath = "/bin/true"
	s.nvModDir = filepath.Join(t.TempDir(), "missing-nvidia")
	s.nvDevNode = filepath.Join(t.TempDir(), "missing-nvidiactl")
	s.sample()
	if got := s.Status(); got != gpuStatusNoDrv {
		t.Fatalf("总体状态应为 %q，实际 %q", gpuStatusNoDrv, got)
	}
	if !strings.Contains(s.reason, "驱动未加载") {
		t.Fatalf("原因应说明驱动未加载，实际 %q", s.reason)
	}
}

// 有 NVIDIA 实体卡但没装 nvidia-smi：必须明确「无驱动工具」，不能报「未检测到」，
// 同时仍要报出厂商（只有品牌、没有数值），供小屏选 nvidia 品牌底图。
func TestNvidiaPresentWithoutToolAggregatesNoTool(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	s := newTestSampler(sys)
	s.nvPath = "" // 模拟 PATH 里没有 nvidia-smi
	s.sample()
	if got := s.Status(); got != gpuStatusNoTool {
		t.Fatalf("总体状态应为 %q，实际 %q", gpuStatusNoTool, got)
	}
	gpus := s.snapshot()
	if len(gpus) != 1 {
		t.Fatalf("应保留 1 条 NVIDIA 品牌占位卡，实际 %+v", gpus)
	}
	g := gpus[0]
	if g.Vendor != "nvidia" || g.Type != gpuTypeDGPU || g.Name != "NVIDIA 显卡" {
		t.Fatalf("占位卡品牌信息异常：%+v", g)
	}
	if g.Status != gpuItemUnavailable {
		t.Fatalf("占位卡状态应为 %q，实际 %q", gpuItemUnavailable, g.Status)
	}
	if g.Utilization != nil || g.Temp != nil || g.MemUsed != nil || g.MemTotal != nil {
		t.Fatalf("占位卡不得伪造任何指标：%+v", g)
	}
}

// 驱动可用后，占位卡必须被真实数据替换，不能重复出现两张 NVIDIA。
func TestNvidiaBrandCardReplacedByRealMetrics(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	s := newTestSampler(sys)
	s.nvPath = "" // 第一轮：没工具 → 占位卡
	s.sample()
	if gpus := s.snapshot(); len(gpus) != 1 || gpus[0].Status != gpuItemUnavailable {
		t.Fatalf("无工具时应只有占位卡：%+v", gpus)
	}

	modDir, nodePath := nvidiaEnv(t)
	s.nvPath = "/bin/true"
	s.nvModDir = modDir
	s.nvDevNode = nodePath
	s.nvRun = func(context.Context, string) ([]byte, error) {
		return []byte("0, GeForce GT 730, 47, 12, 512, 2048\n"), nil
	}
	s.sample()

	gpus := s.snapshot()
	if len(gpus) != 1 {
		t.Fatalf("应只有一张真实 NVIDIA 卡，实际 %+v", gpus)
	}
	g := gpus[0]
	if g.Status != gpuItemOK {
		t.Fatalf("驱动可用后状态应为 ok，实际 %q", g.Status)
	}
	if g.Name != "GeForce GT 730" || g.Utilization == nil || *g.Utilization != 12 {
		t.Fatalf("应替换为真实指标：%+v", g)
	}
	if s.Status() != gpuStatusOK {
		t.Fatalf("总体状态应为 %q，实际 %q", gpuStatusOK, s.Status())
	}
}

// 核显正常但 NVIDIA 缺工具：总体应为「部分可用」，不能隐藏成「正常」。
func TestNvidiaNoToolWithIntelIsPartial(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x8086\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "8\n")
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	s := newTestSampler(sys)
	s.nvPath = ""
	s.sample()
	if got := s.Status(); got != gpuStatusPartial {
		t.Fatalf("总体状态应为 %q，实际 %q", gpuStatusPartial, got)
	}
	gpus := s.snapshot()
	if len(gpus) != 2 || gpus[0].Vendor != "intel" {
		t.Fatalf("核显应照常采集：%+v", gpus)
	}
	if gpus[1].Vendor != "nvidia" || gpus[1].Status != gpuItemUnavailable {
		t.Fatalf("NVIDIA 应保留品牌占位卡：%+v", gpus[1])
	}
}
