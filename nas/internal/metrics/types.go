package metrics

// Snapshot 全量采样结果，/metrics 与 /ws 推送共用此结构。
// 缺失数据以 null 表示（指针字段），不报错。
type Snapshot struct {
	Timestamp int64   `json:"timestamp"`
	CPU       CPU     `json:"cpu"`
	Memory    Memory  `json:"memory"`
	Disks     []Disk  `json:"disks"`
	GPUs      []GPU   `json:"gpus"`
	Network   []NetIf `json:"network"`
}

type CPU struct {
	Usage float64  `json:"usage"` // 0~100
	Temp  *float64 `json:"temp"`  // 摄氏度，无法读取时为 null
}

type Memory struct {
	Usage float64 `json:"usage"` // 0~100
	Used  uint64  `json:"used"`  // 字节
	Total uint64  `json:"total"` // 字节
}

type Disk struct {
	Device   string `json:"device"` // /dev/sda
	Status   string `json:"status"` // active / standby / unknown
	Temp     *int   `json:"temp"`   // 休眠时为 null（绝不主动唤醒）
	ReadBPS  uint64 `json:"read_bytes_per_sec"`
	WriteBPS uint64 `json:"write_bytes_per_sec"`
}

type GPU struct {
	Index       int    `json:"index"`
	Name        string `json:"name"`
	Temp        *int   `json:"temp"`
	Utilization *int   `json:"utilization"` // 0~100
	MemUsed     uint64 `json:"memory_used"`
	MemTotal    uint64 `json:"memory_total"`
}

type NetIf struct {
	Name  string `json:"interface"`
	RxBPS uint64 `json:"rx_bytes_per_sec"`
	TxBPS uint64 `json:"tx_bytes_per_sec"`
}
