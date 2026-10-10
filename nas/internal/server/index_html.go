package server

// indexHTML 是 NAS 端管理页（挂在飞牛统一网关的 unix socket 上，网关已校验登录态）。
// 功能：令牌展示/复制、网卡选择、令牌重置、运行日志。
// 纯静态内联资源，不引外部依赖，离线可用。
const indexHTML = `<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>核桃胶囊</title>
<style>
*{box-sizing:border-box}
body{margin:0;font:14px/1.6 -apple-system,"Segoe UI","Microsoft YaHei",sans-serif;color:#1f2937;background:linear-gradient(180deg,#f6f8fb 0,#eef1f6 100%);min-height:100vh}
.wrap{max-width:840px;margin:0 auto;padding:28px 20px 56px}
/* 顶栏：品牌 + 状态徽章 */
header{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:18px;flex-wrap:wrap}
.brand{display:flex;align-items:center;gap:10px}
.brand .logo{font-size:20px;font-weight:700;letter-spacing:.5px;color:#111827}
#brandIcon{width:28px;height:28px;border-radius:8px;flex:none}
.badge{display:inline-flex;align-items:center;gap:6px;font-size:12px;font-weight:600;padding:5px 12px;border-radius:999px;background:#e7f6ec;color:#15803d}
.badge::before{content:'';width:7px;height:7px;border-radius:50%;background:#22c55e}
.badge.off{background:#fee2e2;color:#b91c1c}
.badge.off::before{background:#ef4444}
.head-right{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
/* 进程用量胶囊（徽章左侧） */
.chip{display:inline-flex;align-items:center;gap:6px;font-size:12px;font-weight:600;padding:5px 12px;border-radius:999px;background:#eef2f7;color:#475569;font-variant-numeric:tabular-nums}
.chip .dot{color:#cbd5e1}
/* 概览信息条 */
.stats{display:grid;grid-template-columns:repeat(4,1fr);gap:10px;margin-bottom:16px}
.stat{background:#fff;border:1px solid #e5e7eb;border-radius:12px;padding:12px 14px;box-shadow:0 1px 2px rgba(0,0,0,.03)}
.stat .k{font-size:12px;color:#9ca3af;margin-bottom:2px}
.stat .v{font-size:15px;font-weight:600;color:#111827;word-break:break-all}
/* 卡片网格 */
.grid{display:grid;grid-template-columns:1fr 1fr;gap:16px}
.card{background:#fff;border:1px solid #e5e7eb;border-radius:14px;padding:18px 20px;box-shadow:0 1px 3px rgba(0,0,0,.04)}
.card.wide{grid-column:1/-1}
.card h2{display:flex;align-items:center;gap:8px;margin:0 0 6px;font-size:15px;font-weight:700;color:#111827}
.card h2 .ic{width:18px;height:18px;color:#1e90ff;flex:none}
.hint{margin:0 0 12px;color:#6b7280;font-size:13px}
.hint.small{font-size:12px;margin-top:10px;color:#9ca3af}
/* 令牌卡：令牌文本 + 复制 */
.token-row{display:flex;gap:10px;align-items:center}
code#token{flex:1;min-width:0;font:15px/1.4 ui-monospace,Consolas,monospace;background:#f6f7f9;border:1px solid #e5e7eb;border-radius:8px;padding:10px 12px;word-break:break-all;user-select:all}
button{border:0;border-radius:8px;padding:9px 16px;font-size:14px;background:#2563eb;color:#fff;cursor:pointer;transition:background .15s}
button:hover{background:#1d4ed8}
button.danger{background:#dc2626}
button.danger:hover{background:#b91c1c}
button.ghost{background:#eef2ff;color:#3730a3}
button.ghost:hover{background:#e0e7ff}
button.small{padding:6px 12px;font-size:13px}
.btn{display:inline-block;border-radius:8px;padding:9px 16px;font-size:14px;background:#2563eb;color:#fff;cursor:pointer;text-decoration:none;transition:background .15s;white-space:nowrap}
.btn:hover{background:#1d4ed8}
.row{display:flex;gap:10px;align-items:center;flex-wrap:wrap}
select{flex:1;min-width:0;padding:9px 12px;border:1px solid #d1d5db;border-radius:8px;font-size:14px;background:#fff}
pre#logs{margin:12px 0 0;padding:12px;background:#0b1020;color:#d1e0ff;border-radius:8px;max-height:280px;overflow:auto;font:12px/1.5 ui-monospace,Consolas,monospace;white-space:pre-wrap}
.toast{position:fixed;left:50%;bottom:28px;transform:translateX(-50%);background:#111827;color:#fff;padding:9px 16px;border-radius:8px;font-size:13px;opacity:0;transition:opacity .2s;pointer-events:none;box-shadow:0 4px 12px rgba(0,0,0,.2);z-index:10}
.toast.show{opacity:1}
@media (max-width:640px){
  .stats{grid-template-columns:repeat(2,1fr)}
  .grid{grid-template-columns:1fr}
  .token-row{flex-wrap:wrap}
  code#token{min-width:160px}
}
</style>
</head>
<body>
<div class="wrap">
<header>
  <div class="brand"><img id="brandIcon" src="api/icon" alt="核桃胶囊" onerror="this.style.display='none'"><span class="logo">核桃胶囊</span></div>
  <div class="head-right">
    <span class="chip" id="usage">CPU — <span class="dot">·</span> 内存 —</span>
    <span class="badge" id="badge">连接中</span>
  </div>
</header>

<div class="stats" id="stats">
  <div class="stat"><div class="k">主机名</div><div class="v" id="stHost">—</div></div>
  <div class="stat"><div class="k">版本</div><div class="v" id="stVer">—</div></div>
  <div class="stat"><div class="k">已运行</div><div class="v" id="stUp">—</div></div>
  <div class="stat"><div class="k">服务端口</div><div class="v" id="stPort">—</div></div>
</div>

<div class="grid">
<section class="card">
<h2><svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>配对令牌</h2>
<p class="hint">在电脑客户端的设置中填入本令牌，即可连接查看监控数据。</p>
<div class="token-row">
  <code id="token">—</code>
  <button id="copyBtn">复制</button>
</div>
<p class="hint small">复制后粘贴到电脑客户端“设置 NAS 连接”的配对令牌一栏。</p>
</section>

<section class="card">
<h2><svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>电脑客户端</h2>
<p class="hint">在 Windows 电脑上安装“核桃胶囊”客户端，填入本页的配对令牌即可实时查看 NAS 状态。</p>
<div class="row" style="justify-content:space-between">
  <div>
    <div style="font-weight:600;color:#111827">Windows 版 <span id="cliVer" style="color:#6b7280;font-weight:400"></span></div>
    <div class="hint small" id="cliSize" style="margin-top:2px"></div>
  </div>
  <a id="dlBtn" class="btn" href="api/download">下载安装包</a>
</div>
<p class="hint small" id="cliSha" style="word-break:break-all"></p>
</section>

<section class="card">
<h2><svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="1" y="5" width="22" height="14" rx="2"/><path d="M7 9h2m4 0h4M7 13h2m4 0h4"/></svg>网络接口</h2>
<p class="hint">选择用于统计速率的物理网卡；选“自动”则由系统按默认路由挑选。</p>
<div class="row">
  <select id="ifaceSel"></select>
  <button id="saveIface">保存</button>
</div>
<p class="hint small" id="ifaceEff"></p>
</section>

<section class="card">
<h2><svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2 3 6v6c0 5 3.8 8.7 9 10 5.2-1.3 9-5 9-10V6z"/><path d="M12 8v4m0 3h.01"/></svg>安全</h2>
<p class="hint">重置后旧令牌立即失效，所有客户端需用新令牌重新配对。</p>
<button id="resetBtn" class="danger">重置令牌</button>
</section>

<section class="card wide">
<h2><svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12h4l3-8 4 16 3-8h4"/></svg>运行日志<button id="logBtn" class="ghost small" style="margin-left:auto">刷新日志</button></h2>
<pre id="logs">点击“刷新日志”加载…</pre>
</section>
</div>
</div>
<div class="toast" id="toast"></div>
<script>
var BASE = location.pathname.replace(/\/+$/, '');
function api(p){ return BASE + p; }
function toast(msg){
  var t = document.getElementById('toast');
  t.textContent = msg; t.className = 'toast show';
  setTimeout(function(){ t.className = 'toast'; }, 1600);
}
function fmtDur(s){
  var d = Math.floor(s/86400), h = Math.floor(s%86400/3600), m = Math.floor(s%3600/60);
  if (d > 0) return d + '天' + h + '小时';
  if (h > 0) return h + '小时' + m + '分';
  return m + '分';
}
function fmtBytes(n){
  if (!n) return '';
  if (n >= 1048576) return (n/1048576).toFixed(1) + ' MB';
  if (n >= 1024) return (n/1024).toFixed(0) + ' KB';
  return n + ' B';
}
function setToken(tok){
  document.getElementById('token').textContent = tok;
}
function loadState(){
  fetch(api('/api/state')).then(function(r){ return r.json(); }).then(function(s){
    document.getElementById('stHost').textContent = s.hostname || '—';
    document.getElementById('stVer').textContent = s.version || '—';
    document.getElementById('stUp').textContent = fmtDur(s.uptime_sec || 0);
    document.getElementById('stPort').textContent = s.service_port || '—';
    setToken(s.token);
    var sel = document.getElementById('ifaceSel');
    sel.innerHTML = '';
    var optAuto = document.createElement('option');
    optAuto.value = ''; optAuto.textContent = '自动（按默认路由）';
    sel.appendChild(optAuto);
    (s.ifaces || []).forEach(function(n){
      var o = document.createElement('option');
      o.value = n; o.textContent = n;
      sel.appendChild(o);
    });
    sel.value = s.iface || '';
    document.getElementById('ifaceEff').textContent = '当前生效网卡：' + (s.iface_effective || '未知');
    // 电脑客户端安装包信息
    document.getElementById('cliVer').textContent = s.client_version ? ('v' + s.client_version) : '';
    if (s.client_available) {
      document.getElementById('cliSize').textContent = fmtBytes(s.client_size);
      document.getElementById('cliSha').textContent = 'SHA256：' + (s.client_sha256 || '');
    } else {
      document.getElementById('cliSize').textContent = '安装包未随本应用提供';
      document.getElementById('dlBtn').style.display = 'none';
    }
  }).catch(function(){ toast('加载状态失败'); });
}
function setBadge(ok){
  var b = document.getElementById('badge');
  b.textContent = ok ? '运行中' : '连接异常';
  b.className = ok ? 'badge' : 'badge off';
}
// 本应用进程的 CPU / 内存用量（徽章左侧）
function loadUsage(){
  fetch(api('/api/usage')).then(function(r){ return r.json(); }).then(function(u){
    var cpu = (typeof u.cpu_percent === 'number') ? (u.cpu_percent.toFixed(1) + '%') : '—';
    var mem = u.mem_bytes ? fmtBytes(u.mem_bytes) : '—';
    document.getElementById('usage').innerHTML = 'CPU ' + cpu + ' <span class="dot">·</span> 内存 ' + mem;
    setBadge(true);
  }).catch(function(){ setBadge(false); });
}
document.getElementById('copyBtn').onclick = function(){
  var tok = document.getElementById('token').textContent;
  if (navigator.clipboard) {
    navigator.clipboard.writeText(tok).then(function(){ toast('已复制'); }, function(){ toast('复制失败，请手动选择'); });
  } else {
    var r = document.createRange(); r.selectNodeContents(document.getElementById('token'));
    var sel = getSelection(); sel.removeAllRanges(); sel.addRange(r);
    try { document.execCommand('copy'); toast('已复制'); } catch(e){ toast('请手动复制'); }
  }
};
document.getElementById('saveIface').onclick = function(){
  var name = document.getElementById('ifaceSel').value;
  fetch(api('/api/iface'), {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify({name:name})})
    .then(function(r){ if(!r.ok) throw 0; return r.json(); })
    .then(function(){ toast('已保存'); loadState(); })
    .catch(function(){ toast('保存失败'); });
};
document.getElementById('resetBtn').onclick = function(){
  if (!confirm('确定重置配对令牌？所有客户端需用新令牌重新配对。')) return;
  fetch(api('/api/token/reset'), {method:'POST'})
    .then(function(r){ if(!r.ok) throw 0; return r.json(); })
    .then(function(s){ setToken(s.token); toast('已重置，请更新客户端'); })
    .catch(function(){ toast('重置失败'); });
};
document.getElementById('logBtn').onclick = function(){
  fetch(api('/api/logs?lines=300')).then(function(r){ return r.text(); }).then(function(t){
    document.getElementById('logs').textContent = t;
  }).catch(function(){ toast('读取日志失败'); });
};
loadState();
loadUsage();
setInterval(loadUsage, 5000);
</script>
</body>
</html>
`
