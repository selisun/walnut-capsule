package metrics

import (
	"context"
	"encoding/json"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

// 铁律：绝不主动唤醒休眠硬盘。
// - hdparm -C 仅查询电源状态，不唤醒；
// - standby 盘只报状态，跳过 SMART；
// - SMART 前再加 smartctl -n standby 双保险（期间进入休眠也不唤醒）；
// - SMART 间隔 ≥ smartInterval。

const (
	smartInterval  = 30 * time.Minute
	cmdTimeout     = 15 * time.Second
	smartTimeout   = 30 * time.Second
	sectorSize     = 512
)

// 整盘设备名：sda / nvme0n1 / mmcblk0 / vda 等，排除分区与虚拟设备。
var wholeDiskRe = regexp.MustCompile(`^(sd[a-z]+|hd[a-z]+|vd[a-z]+|xvd[a-z]+|nvme[0-9]+n[0-9]+|mmcblk[0-9]+)$`)

type diskState struct {
	name      string
	status    string // active / standby / unknown
	temp      *int
	readBPS   uint64
	writeBPS  uint64
	prevRead  uint64
	prevWrite uint64
	prevAt    time.Time
	lastSMART time.Time
}

type diskSampler struct {
	proc string

	mu    sync.RWMutex
	disks map[string]*diskState
	order []string // 稳定输出顺序
}

func newDiskSampler(proc string) *diskSampler {
	return &diskSampler{proc: proc, disks: map[string]*diskState{}}
}

func (s *diskSampler) snapshot() []Disk {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]Disk, 0, len(s.order))
	for _, name := range s.order {
		d := s.disks[name]
		out = append(out, Disk{
			Device:   "/dev/" + d.name,
			Status:   d.status,
			Temp:     d.temp,
			ReadBPS:  d.readBPS,
			WriteBPS: d.writeBPS,
		})
	}
	return out
}

// sampleRates 每秒调用：读 /proc/diskstats 差值算速率，同时发现新盘。
// 只读 /proc，对休眠盘无影响。
func (s *diskSampler) sampleRates() {
	b, err := os.ReadFile(filepath.Join(s.proc, "diskstats"))
	if err != nil {
		return
	}
	now := time.Now()
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, line := range strings.Split(string(b), "\n") {
		f := strings.Fields(line)
		if len(f) < 10 || !wholeDiskRe.MatchString(f[2]) {
			continue
		}
		name := f[2]
		rd, err1 := strconv.ParseUint(f[5], 10, 64)  // sectors read
		wr, err2 := strconv.ParseUint(f[9], 10, 64)  // sectors written
		if err1 != nil || err2 != nil {
			continue
		}
		d, ok := s.disks[name]
		if !ok {
			d = &diskState{name: name, status: "unknown"}
			s.disks[name] = d
			s.order = append(s.order, name)
		}
		if !d.prevAt.IsZero() {
			dt := now.Sub(d.prevAt).Seconds()
			if dt > 0 && rd >= d.prevRead && wr >= d.prevWrite {
				d.readBPS = uint64(float64((rd-d.prevRead)*sectorSize) / dt)
				d.writeBPS = uint64(float64((wr-d.prevWrite)*sectorSize) / dt)
			}
		}
		d.prevRead, d.prevWrite, d.prevAt = rd, wr, now
	}
}

// sampleStatus 低频调用（30~60s）：hdparm -C 查询电源状态；
// 仅 active/idle 且距上次 SMART ≥30 分钟才执行 smartctl。
func (s *diskSampler) sampleStatus() {
	s.mu.RLock()
	names := make([]string, len(s.order))
	copy(names, s.order)
	s.mu.RUnlock()

	for _, name := range names {
		status := probePowerState(name)

		s.mu.RLock()
		d := s.disks[name]
		lastSMART := d.lastSMART
		s.mu.RUnlock()

		var temp *int
		runSMART := status != "standby" && time.Since(lastSMART) >= smartInterval
		if status == "standby" {
			// 休眠盘：只报状态，温度置 null，不碰 SMART。
		} else if runSMART {
			temp = readSMARTTemp(name)
			s.mu.Lock()
			s.disks[name].lastSMART = time.Now()
			s.mu.Unlock()
		} else {
			s.mu.RLock()
			temp = s.disks[name].temp // 保留上次温度
			s.mu.RUnlock()
		}

		s.mu.Lock()
		if d, ok := s.disks[name]; ok {
			d.status = status
			d.temp = temp
		}
		s.mu.Unlock()
	}
}

// probePowerState 执行 hdparm -C（只查询，不唤醒盘）。
func probePowerState(name string) string {
	ctx, cancel := context.WithTimeout(context.Background(), cmdTimeout)
	defer cancel()
	out, err := exec.CommandContext(ctx, "hdparm", "-C", "/dev/"+name).CombinedOutput()
	if err != nil {
		// hdparm 不可用或不支持（如部分 NVMe/USB）：状态未知，后续由 smartctl -n standby 兜底。
		return "unknown"
	}
	low := strings.ToLower(string(out))
	switch {
	case strings.Contains(low, "standby"):
		return "standby"
	case strings.Contains(low, "active"):
		return "active"
	default:
		return "unknown"
	}
}

type smartJSON struct {
	Temperature *struct {
		Current *int `json:"current"`
	} `json:"temperature"`
	PowerMode *string `json:"power_mode"`
}

// readSMARTTemp 读取盘温。-n standby：若盘恰好在探测后进入休眠，直接退出不唤醒。
func readSMARTTemp(name string) *int {
	ctx, cancel := context.WithTimeout(context.Background(), smartTimeout)
	defer cancel()
	// smartctl 退出码非零也常带有效 JSON，忽略 err 直接解析 stdout。
	out, _ := exec.CommandContext(ctx, "smartctl", "-n", "standby", "-a", "-j", "/dev/"+name).Output()
	var sj smartJSON
	if err := json.Unmarshal(out, &sj); err != nil {
		log.Printf("smartctl parse %s: %v", name, err)
		return nil
	}
	if sj.PowerMode != nil && strings.Contains(strings.ToUpper(*sj.PowerMode), "STANDBY") {
		return nil // 竞态保护：盘已休眠
	}
	if sj.Temperature != nil && sj.Temperature.Current != nil {
		v := *sj.Temperature.Current
		return &v
	}
	return nil
}
