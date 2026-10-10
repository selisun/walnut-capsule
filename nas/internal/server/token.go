package server

import (
	"crypto/rand"
	"encoding/base64"
	"os"
	"strings"
	"sync"
)

// TokenStore 保存配对令牌，支持运行时重置并持久化到文件（重启/升级不变）。
type TokenStore struct {
	mu   sync.RWMutex
	v    string
	file string // 持久化路径；为空则仅内存
}

// NewTokenStore 构造令牌存储：initial 非空则优先（如 NET_IFACE 式的显式注入）；
// 否则从 file 读取已持久化的令牌，保证重启/升级后令牌不变。
func NewTokenStore(initial, file string) *TokenStore {
	t := &TokenStore{v: initial, file: file}
	if t.v == "" && file != "" {
		if b, err := os.ReadFile(file); err == nil {
			t.v = strings.TrimSpace(string(b))
		}
	}
	return t
}

func (t *TokenStore) Get() string {
	t.mu.RLock()
	defer t.mu.RUnlock()
	return t.v
}

// Ensure 保证令牌非空：为空则生成一枚。
func (t *TokenStore) Ensure() {
	if t.Get() != "" {
		return
	}
	_, _ = t.Reset()
}

// Reset 生成新令牌并落盘，返回新值。旧令牌立即失效，客户端需重新配对。
func (t *TokenStore) Reset() (string, error) {
	var b [24]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	v := base64.RawURLEncoding.EncodeToString(b[:])
	t.mu.Lock()
	t.v = v
	file := t.file
	t.mu.Unlock()
	if file != "" {
		if err := os.WriteFile(file, []byte(v), 0600); err != nil {
			return v, err
		}
	}
	return v, nil
}
