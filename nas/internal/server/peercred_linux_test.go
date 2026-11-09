//go:build linux

package server

import (
	"context"
	"testing"
)

// 未配置白名单时默认只放行 root 与网关 worker(nobody)，其它 UID 一律拒绝。
func TestPeerAllowedDefaultsToRootAndNobody(t *testing.T) {
	t.Setenv("ADMIN_SOCKET_ALLOW_UIDS", "")
	t.Setenv("ADMIN_SOCKET_ALLOW_ALL", "")

	ctxFor := func(uid uint32) context.Context {
		return context.WithValue(context.Background(), peerUIDKey{}, uid)
	}

	if !peerAllowed(ctxFor(0)) {
		t.Fatal("root 应允许")
	}
	if !peerAllowed(ctxFor(65534)) {
		t.Fatal("nobody 应允许")
	}
	if peerAllowed(ctxFor(1234)) {
		t.Fatal("普通用户 UID 应被拒绝")
	}
}

// 拿不到对端 UID（非 unix socket / 读取失败）时不阻断，保持原有行为。
func TestPeerAllowedWithoutUIDDoesNotBlock(t *testing.T) {
	t.Setenv("ADMIN_SOCKET_ALLOW_UIDS", "")
	t.Setenv("ADMIN_SOCKET_ALLOW_ALL", "")
	if !peerAllowed(context.Background()) {
		t.Fatal("无 UID 信息时不应阻断")
	}
}
