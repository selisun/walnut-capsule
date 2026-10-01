package metrics

import (
	"os"
	"time"
)

const (
	fastInterval       = 1 * time.Second  // cpu/内存/盘速率/网络
	gpuInterval        = 3 * time.Second  // nvidia-smi（规格 2~5s）
	diskStatusInterval = 45 * time.Second // hdparm -C（规格 30~60s）
)

// Collector 调度各采样器并聚合快照。
// 路径经 HOST_PROC / HOST_SYS 环境变量注入，容器内挂宿主机 /proc、/sys。
type Collector struct {
	cpu  *cpuSampler
	mem  *memSampler
	disk *diskSampler
	gpu  *gpuSampler
	net  *netSampler
}

func NewCollector() *Collector {
	proc := getenvOr("HOST_PROC", "/proc")
	sys := getenvOr("HOST_SYS", "/sys")
	return &Collector{
		cpu:  newCPUSampler(proc, sys),
		mem:  newMemSampler(proc),
		disk: newDiskSampler(proc),
		gpu:  newGPUSampler(),
		net:  newNetSampler(proc),
	}
}

// Run 启动后台采样循环（不阻塞）。
func (c *Collector) Run() {
	// 启动即采一轮，避免接口返回空数据。
	c.cpu.sample()
	c.mem.sample()
	c.disk.sampleRates()
	c.net.sample()
	c.gpu.sample()
	go c.disk.sampleStatus() // 启动时也按铁律探测一轮

	go loop(fastInterval, func() {
		c.cpu.sample()
		c.mem.sample()
		c.disk.sampleRates()
		c.net.sample()
	})
	go loop(gpuInterval, c.gpu.sample)
	go loop(diskStatusInterval, c.disk.sampleStatus)
}

// Snapshot 聚合当前各采样器的最新值。
func (c *Collector) Snapshot() Snapshot {
	usage, temp := c.cpu.snapshot()
	memUsage, memUsed, memTotal := c.mem.snapshot()
	return Snapshot{
		Timestamp: time.Now().UnixMilli(),
		CPU:       CPU{Usage: usage, Temp: temp},
		Memory:    Memory{Usage: memUsage, Used: memUsed, Total: memTotal},
		Disks:     c.disk.snapshot(),
		GPUs:      c.gpu.snapshot(),
		Network:   c.net.snapshot(),
	}
}

func loop(d time.Duration, f func()) {
	t := time.NewTicker(d)
	defer t.Stop()
	for range t.C {
		f()
	}
}

func getenvOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
