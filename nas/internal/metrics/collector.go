package metrics

import (
	"log"
	"os"
	"strings"
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
	cpu      *cpuSampler
	mem      *memSampler
	disk     *diskSampler
	gpu      *gpuSampler
	net      *netSampler
	hostname string
}

func NewCollector() *Collector {
	proc := getenvOr("HOST_PROC", "/proc")
	sys := getenvOr("HOST_SYS", "/sys")
	root := getenvOr("HOST_ROOT", "") // 宿主机根在容器内的挂载点（/host/root），用于还原宿主路径
	return &Collector{
		cpu:      newCPUSampler(proc, sys),
		mem:      newMemSampler(proc),
		disk:     newDiskSampler(proc, sys, root),
		gpu:      newGPUSampler(),
		net:      newNetSampler(proc),
		hostname: readHostname(proc),
	}
}

// readHostname 取宿主机名：优先读挂载进来的宿主机 procfs
// （容器内 /host/proc/sys/kernel/hostname），回退当前 UTS 名
// （compose 配置 uts: host 后，UTS 名就是宿主机名）。
// 主机名运行期不变，启动时读一次即可。
func readHostname(proc string) string {
	if b, err := os.ReadFile(proc + "/sys/kernel/hostname"); err == nil {
		if h := strings.TrimSpace(string(b)); h != "" {
			return h
		}
	} else {
		log.Printf("read %s/sys/kernel/hostname: %v（改用 UTS 主机名）", proc, err)
	}
	if h, err := os.Hostname(); err == nil {
		h = strings.TrimSpace(h)
		if h != "" {
			return h
		}
	}
	return ""
}

// Run 启动后台采样循环（不阻塞）。
func (c *Collector) Run() {
	log.Printf("host: %q", c.hostname)
	// 启动即采一轮，避免接口返回空数据。
	c.cpu.sample()
	c.mem.sample()
	c.disk.sampleRates()
	c.net.sample()
	c.gpu.sample()
	go c.disk.sampleStatus() // 启动时也按铁律探测一轮
	c.disk.sampleCapacity()  // 启动即解析挂载表采一轮容量

	go loop(fastInterval, func() {
		c.cpu.sample()
		c.mem.sample()
		c.disk.sampleRates()
		c.net.sample()
	})
	go loop(gpuInterval, c.gpu.sample)
	go loop(diskStatusInterval, func() {
		c.disk.sampleStatus()
		c.disk.sampleCapacity()
	})
}

// Snapshot 聚合当前各采样器的最新值。
func (c *Collector) Snapshot() Snapshot {
	usage, temp := c.cpu.snapshot()
	memUsage, memUsed, memTotal := c.mem.snapshot()
	return Snapshot{
		Timestamp: time.Now().UnixMilli(),
		Hostname:  c.hostname,
		CPU:       CPU{Usage: usage, Temp: temp},
		Memory:    Memory{Usage: memUsage, Used: memUsed, Total: memTotal},
		Disks:     c.disk.snapshot(),
		Volumes:   c.disk.snapshotVolumes(),
		GPUs:      c.gpu.snapshot(),
		Network:   c.net.snapshot(),
	}
}

// Hostname 返回宿主机名（启动时读取一次）。
func (c *Collector) Hostname() string { return c.hostname }

// SetNetIface 运行时切换手动指定网卡（空串恢复自动选择）。
func (c *Collector) SetNetIface(name string) { c.net.setTarget(name) }

// NetIfaces 返回可用网卡名列表（供管理页下拉）。
func (c *Collector) NetIfaces() []string { return c.net.availableIfaces() }

// NetIface 返回当前手动指定的网卡（空串为自动）。
func (c *Collector) NetIface() string { return c.net.currentTarget() }

// NetIfaceEffective 返回当前实际生效的网卡（含自动选择结果）。
func (c *Collector) NetIfaceEffective() string { return c.net.effectiveTarget() }

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
