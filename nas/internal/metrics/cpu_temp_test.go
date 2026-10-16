package metrics

import (
	"bytes"
	"log"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// 在伪 sysfs 根下写文件（自动建父目录）
func writeSys(t *testing.T, root, rel, content string) {
	t.Helper()
	p := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

// hwChan 一个 hwmon 温度通道：num 为 tempN 的 N，val 为毫摄氏度，label 可空
type hwChan struct {
	num   string
	val   string
	label string
}

// addHwmon 在伪 sysfs 下造一个 /sys/class/hwmon/hwmonN
func addHwmon(t *testing.T, root, n, name string, chans []hwChan) {
	t.Helper()
	dir := "class/hwmon/hwmon" + n
	writeSys(t, root, dir+"/name", name)
	for _, c := range chans {
		writeSys(t, root, dir+"/temp"+c.num+"_input", c.val)
		if c.label != "" {
			writeSys(t, root, dir+"/temp"+c.num+"_label", c.label)
		}
	}
}

func TestCPUTempFromHwmon(t *testing.T) {
	cases := []struct {
		name  string
		build func(t *testing.T, root string)
		want  float64
		ok    bool
	}{
		{
			name: "AMD k10temp 取 Tctl（优先于 Tccd1、忽略 amdgpu）",
			build: func(t *testing.T, root string) {
				addHwmon(t, root, "3", "amdgpu", []hwChan{{"1", "37000", "edge"}})
				addHwmon(t, root, "4", "k10temp", []hwChan{
					{"1", "45200", "Tctl"},
					{"3", "47100", "Tccd1"},
				})
			},
			want: 45.2, ok: true,
		},
		{
			name: "Intel coretemp 取 Package id 0",
			build: func(t *testing.T, root string) {
				addHwmon(t, root, "2", "coretemp", []hwChan{
					{"1", "39000", "Package id 0"},
					{"2", "38000", "Core 0"},
				})
			},
			want: 39.0, ok: true,
		},
		{
			name: "无 label 时退到通道号最小的可读通道（temp1 为 0 被过滤）",
			build: func(t *testing.T, root string) {
				addHwmon(t, root, "5", "zenpower", []hwChan{
					{"1", "0", ""},
					{"2", "42000", ""},
				})
			},
			want: 42.0, ok: true,
		},
		{
			name: "只有非 CPU 传感器时不应命中",
			build: func(t *testing.T, root string) {
				addHwmon(t, root, "0", "amdgpu", []hwChan{{"1", "36000", "edge"}})
			},
			ok: false,
		},
		{
			name:  "无任何 hwmon",
			build: func(t *testing.T, root string) {},
			ok:    false,
		},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			root := t.TempDir()
			c.build(t, root)
			s := newCPUSampler("/nonexistent/proc", root)
			got, ok := s.tempFromHwmon()
			if ok != c.ok {
				t.Fatalf("tempFromHwmon ok=%v, want %v", ok, c.ok)
			}
			if ok && math.Abs(got-c.want) > 0.001 {
				t.Fatalf("tempFromHwmon=%v, want %v", got, c.want)
			}
		})
	}
}

// 老机型（只有 x86_pkg_temp 热区、无 hwmon）仍应从热区读到温度
func TestCPUTempFallbackToThermalZone(t *testing.T) {
	root := t.TempDir()
	writeSys(t, root, "class/thermal/thermal_zone0/type", "x86_pkg_temp")
	writeSys(t, root, "class/thermal/thermal_zone0/temp", "41000")

	s := newCPUSampler("/nonexistent/proc", root)
	s.sampleTemp()
	_, temp := s.snapshot()
	if temp == nil || math.Abs(*temp-41.0) > 0.001 {
		t.Fatalf("热区兜底失败，temp=%v", temp)
	}
}

// AMD 机型：hwmon 的 k10temp 优先于 acpitz 热区（acpitz 常是主板温度）
func TestCPUTempHwmonWinsOverZone(t *testing.T) {
	root := t.TempDir()
	addHwmon(t, root, "1", "k10temp", []hwChan{{"1", "52000", "Tctl"}})
	writeSys(t, root, "class/thermal/thermal_zone0/type", "acpitz")
	writeSys(t, root, "class/thermal/thermal_zone0/temp", "16800")

	s := newCPUSampler("/nonexistent/proc", root)
	s.sampleTemp()
	_, temp := s.snapshot()
	if temp == nil || math.Abs(*temp-52.0) > 0.001 {
		t.Fatalf("应优先取 hwmon 的 Tctl，temp=%v", temp)
	}
}

// 复现用户机型：AMD 无 thermal_zone、CPU 温度只在 hwmon → 修复后必须能读到
func TestCPUTempAMDNoThermalZone(t *testing.T) {
	root := t.TempDir()
	addHwmon(t, root, "2", "k10temp", []hwChan{
		{"1", "43800", "Tctl"},
		{"3", "44100", "Tccd1"},
	})

	s := newCPUSampler("/nonexistent/proc", root)
	s.sampleTemp()
	_, temp := s.snapshot()
	if temp == nil || math.Abs(*temp-43.8) > 0.001 {
		t.Fatalf("AMD 无热区机型仍读不到 CPU 温度，temp=%v", temp)
	}
}

// 启动探测日志要能说清「这台机器有什么传感器、最终取到多少」，供远程排查
func TestLogTempSources(t *testing.T) {
	root := t.TempDir()
	addHwmon(t, root, "2", "k10temp", []hwChan{{"1", "43800", "Tctl"}})
	writeSys(t, root, "class/thermal/thermal_zone0/type", "acpitz")
	writeSys(t, root, "class/thermal/thermal_zone0/temp", "16800")

	var buf bytes.Buffer
	prev := log.Writer()
	log.SetOutput(&buf)
	defer log.SetOutput(prev)

	s := newCPUSampler("/nonexistent/proc", root)
	s.logTempSources()

	out := buf.String()
	for _, want := range []string{"k10temp", "Tctl=43.8", "acpitz=16.8", "结果=43.8°C"} {
		if !strings.Contains(out, want) {
			t.Fatalf("日志缺少 %q：%s", want, out)
		}
	}
}

// 完全没有温度源时，日志要明确说明「未找到」，而不是静默
func TestLogTempSourcesNothing(t *testing.T) {
	root := t.TempDir()

	var buf bytes.Buffer
	prev := log.Writer()
	log.SetOutput(&buf)
	defer log.SetOutput(prev)

	s := newCPUSampler("/nonexistent/proc", root)
	s.logTempSources()

	if out := buf.String(); !strings.Contains(out, "未找到") {
		t.Fatalf("日志应说明未找到温度源：%s", out)
	}
}
