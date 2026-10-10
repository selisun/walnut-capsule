package main

import (
	"crypto/sha256"
	"encoding/hex"
	"flag"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"hetao-nas/internal/metrics"
	"hetao-nas/internal/server"
)

const appName = "hetaocapsule"

// appVersion 仅作 TRIM_APPVER 缺失时的兜底显示，正常由 fpk 注入。
// 需与 hetaocapsule/manifest 的 version 保持一致。
const appVersion = "0.1.16"

// clientVersion 电脑端安装器版本，需与 capsule/installer/hetao-capsule.iss 的 AppVer 一致。
// 版本策略：客户端与 fpk 各自独立计数——客户端每改一次递增；fpk 有改动也递增。
const clientVersion = "0.1.14"

func main() {
	addr := flag.String("addr", ":8765", "listen address")
	flag.Parse()

	// fnOS 运行期注入的路径：TRIM_PKGVAR 持久化目录、TRIM_APPDEST 安装目录。
	// 非 fpk 环境（本地调试）为空，此时仅跑 TCP API。
	stateDir := os.Getenv("TRIM_PKGVAR")
	appDest := os.Getenv("TRIM_APPDEST")

	// 配对令牌：CAPSULE_TOKEN 指定则用之（启动脚本从 var/token 注入，保证重启不变）；
	// 未指定则生成并落盘到 TRIM_PKGVAR/token。令牌保证非空：server 对无令牌请求一律拒绝。
	var tokenFile string
	if stateDir != "" {
		tokenFile = filepath.Join(stateDir, "token")
	}
	tok := server.NewTokenStore(os.Getenv("CAPSULE_TOKEN"), tokenFile)
	tok.Ensure()
	log.Printf("配对令牌已就绪：%s（完整值请在本应用管理页点「显示」查看/复制）", server.MaskToken(tok.Get()))

	col := metrics.NewCollector()

	// 网卡选择：状态目录里若存有 iface 且未用 NET_IFACE 环境变量强制指定，则应用之。
	if stateDir != "" && os.Getenv("NET_IFACE") == "" {
		if b, err := os.ReadFile(filepath.Join(stateDir, "iface")); err == nil {
			if name := strings.TrimSpace(string(b)); name != "" {
				col.SetNetIface(name)
			}
		}
	}

	col.Run()

	// 飞牛统一网关 unix socket：NAS 端管理页（令牌/网卡/日志）。网关已校验飞牛登录态。
	// 放在独立监听上，局域网 TCP 端口绝不暴露该页面，令牌不会在局域网裸奔。
	if appDest != "" {
		if err := serveAdmin(appDest, stateDir, col, tok); err != nil {
			log.Printf("admin socket 启动失败：%v", err)
		}
	}

	// 局域网 TCP：仅数据 API（/health、/metrics、/ws），令牌鉴权。
	srv := server.New(col, tok)
	httpSrv := &http.Server{
		Addr:              *addr,
		Handler:           srv.Handler(),
		ReadHeaderTimeout: 5 * time.Second,
	}
	log.Printf("hetao-nas listening on %s", *addr)
	log.Fatal(httpSrv.ListenAndServe())
}

// serveAdmin 在 ${TRIM_APPDEST}/app.sock 上提供管理页，
// 网关把 /app/<appname>/* 转发到这里（保留前缀，由 handler 剥离）。
func serveAdmin(appDest, stateDir string, col *metrics.Collector, tok *server.TokenStore) error {
	sock := filepath.Join(appDest, "app.sock")
	_ = os.Remove(sock) // 清理上次遗留
	ln, err := net.Listen("unix", sock)
	if err != nil {
		return err
	}
	// 网关 worker（nobody）需要连接该 socket。
	_ = os.Chmod(sock, 0o666)

	logPath := ""
	if stateDir != "" {
		logPath = filepath.Join(stateDir, "hetao-nas.log")
	}
	// 电脑端安装器随 fpk 下发到 target/ui/download/，管理页据此提供下载；
	// 启动时算一次大小与 SHA256，供页面展示核对。
	setupPath := filepath.Join(appDest, "ui", "download", "hetao-capsule-setup.exe")
	var setupSize int64
	var setupSHA string
	if st, err := os.Stat(setupPath); err == nil {
		setupSize = st.Size()
		if f, err := os.Open(setupPath); err == nil {
			hh := sha256.New()
			if _, err := io.Copy(hh, f); err == nil {
				setupSHA = hex.EncodeToString(hh.Sum(nil))
			}
			_ = f.Close()
		}
	} else {
		log.Printf("电脑端安装器未找到：%s", setupPath)
	}
	h := server.NewAdmin("/app/"+appName, server.AdminDeps{
		Col:           col,
		Tok:           tok,
		Version:       getenvOr("TRIM_APPVER", appVersion),
		Hostname:      col.Hostname(),
		LogPath:       logPath,
		StateDir:      stateDir,
		IconPath:      filepath.Join(appDest, "ui", "images", "icon_64.png"),
		SetupPath:     setupPath,
		ClientVersion: clientVersion,
		ClientSize:    setupSize,
		ClientSHA256:  setupSHA,
		Start:         time.Now(),
	})
	go func() {
		if err := (&http.Server{Handler: h}).Serve(ln); err != nil {
			log.Printf("admin socket serve: %v", err)
		}
	}()
	log.Printf("admin UI on unix:%s（网关路径 /app/%s）", sock, appName)
	return nil
}

func getenvOr(k, d string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return d
}
