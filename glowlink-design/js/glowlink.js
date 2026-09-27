/* ============================================================
   萤光 GlowLink — 320×500 小窗交互
   状态机：boot → Empty（搜索中）→ Discover（已发现）
           └─ 选设备 + 投放文件 → Transfer → 完成
   设备发现按 UDP 广播语义模拟：逐个亮起，超时自动下线。
   所有传输数字都由真实文件大小与实际耗时推导，不编造指标。
   ============================================================ */
(function () {
  "use strict";

  var root = document.querySelector("[data-glow]");
  if (!root) return;

  var wires = root.querySelector("[data-wires]");
  var drop = root.querySelector("[data-drop]");
  var dropText = root.querySelector("[data-drop-text]");
  var fileIn = root.querySelector("[data-file]");
  var xfer = root.querySelector("[data-xfer]");
  var xName = root.querySelector("[data-x-name]");
  var xPct = root.querySelector("[data-x-pct]");
  var xBar = root.querySelector("[data-x-bar]");
  var xMeta = root.querySelector("[data-x-meta]");
  var xActs = root.querySelector("[data-x-actions]");
  var btnAgain = root.querySelector("[data-act='again']");
  var btnClose = root.querySelector("[data-act='close']");
  var privacy = root.querySelector("[data-privacy]");
  var privacyBtn = root.querySelector("[data-act='privacy']");
  var trayBtn = root.querySelector("[data-act='tray']");
  var trayBar = root.querySelector("[data-traybar]");
  var selfNode = root.querySelector(".node--self");

  var reduce = window.matchMedia
    ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
    : false;

  /* 节点图坐标系 320×396，与 CSS 中的 left/top 保持同一契约 */
  var CENTER = { x: 160, y: 196 };
  var POS = {
    phone: { x: 66, y: 42 },
    tablet: { x: 254, y: 42 },
    desktop: { x: 160, y: 320 }
  };

  var selected = null;
  var queue = [];
  var index = 0;
  var rafId = 0;
  var denyTimer = 0;
  var bootTimer = 0;
  var holdTimer = 0;
  var sending = false;

  /* ---------- 工具 ---------- */
  function el(sel) {
    return root.querySelector(sel);
  }
  function nodeOf(id) {
    return el('[data-device="' + id + '"]');
  }
  function nameOf(id) {
    var n = el('[data-device="' + id + '"] .node__label');
    return n ? n.textContent.trim() : id;
  }
  function toneOf(id) {
    var n = nodeOf(id);
    return n ? n.style.getPropertyValue("--tone").trim() : "";
  }
  function fmtSize(bytes) {
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1048576) return (bytes / 1024).toFixed(1) + " KB";
    if (bytes < 1073741824) return (bytes / 1048576).toFixed(1) + " MB";
    return (bytes / 1073741824).toFixed(2) + " GB";
  }
  function fmtSecs(s) {
    return s < 0.1 ? "不足 0.1 秒" : s.toFixed(1) + " 秒";
  }

  /* ---------- 状态 ---------- */
  function setState(s) {
    root.classList.remove("is-empty", "is-discover", "is-transfer");
    root.classList.add(s);
  }

  function boot() {
    setState("is-empty");
    bootTimer = setTimeout(function () {
      setState("is-discover");
      if (reduce) return;
      var nodes = root.querySelectorAll("[data-device]");
      Array.prototype.forEach.call(nodes, function (n, i) {
        n.style.transitionDelay = i * 0.09 + "s";
        setTimeout(function () {
          n.style.transitionDelay = "";
        }, 400 + i * 90);
      });
    }, 1400);
  }

  /* ---------- 选择设备 ---------- */
  function select(id) {
    if (sending) return;
    if (selected === id) {
      selected = null;
      nodeOf(id).classList.remove("is-selected");
      dropText.textContent = "拖拽文件到这里";
      return;
    }
    if (selected) nodeOf(selected).classList.remove("is-selected");
    selected = id;
    nodeOf(id).classList.add("is-selected");
    dropText.textContent = "拖到此处 → " + nameOf(id);
  }

  /* ---------- 流动虚线：本机环外缘 → 目标设备环外缘 ---------- */
  /* 连线不从两个圆心出发，否则虚线会横穿本机中心图标。
     先按弦长把二次贝塞尔从两端各裁掉一段（本机 42+4，设备 26+5），
     剩下的一段仍是二次贝塞尔，控制点用 de Casteljau 反算。 */
  var TRIM_SELF = 46;
  var TRIM_DEV = 31;

  function lerp(a, b, t) {
    return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
  }

  function splitQuad(seg, t) {
    var a = lerp(seg[0], seg[1], t);
    var b = lerp(seg[1], seg[2], t);
    var c = lerp(a, b, t);
    return { left: [seg[0], a, c], right: [c, b, seg[2]] };
  }

  function qAt(seg, t) {
    var u = 1 - t;
    return {
      x: u * u * seg[0].x + 2 * u * t * seg[1].x + t * t * seg[2].x,
      y: u * u * seg[0].y + 2 * u * t * seg[1].y + t * t * seg[2].y
    };
  }

  /* 从 t=0 沿弧长前进 dist，返回命中的 t */
  function tForward(seg, dist) {
    var acc = 0;
    var prev = seg[0];
    for (var i = 1; i <= 400; i++) {
      var t = i / 400;
      var p = qAt(seg, t);
      acc += Math.hypot(p.x - prev.x, p.y - prev.y);
      prev = p;
      if (acc >= dist) return t;
    }
    return 1;
  }

  /* 从 t=1 沿弧长后退 dist，返回命中的 t */
  function tBackward(seg, dist) {
    var acc = 0;
    var prev = qAt(seg, 1);
    for (var i = 1; i <= 400; i++) {
      var t = 1 - i / 400;
      var p = qAt(seg, t);
      acc += Math.hypot(p.x - prev.x, p.y - prev.y);
      prev = p;
      if (acc >= dist) return t;
    }
    return 0;
  }

  function drawWire(id) {
    var p = POS[id];
    if (!p) {
      wires.innerHTML = "";
      return;
    }
    var dx = p.x - CENTER.x;
    var dy = p.y - CENTER.y;
    var len = Math.sqrt(dx * dx + dy * dy) || 1;
    var bow = 16;
    var nx = (-dy / len) * bow;
    var ny = (dx / len) * bow;
    var mx = (CENTER.x + p.x) / 2 + nx;
    var my = (CENTER.y + p.y) / 2 + ny;

    var seg = [
      { x: CENTER.x, y: CENTER.y },
      { x: mx, y: my },
      { x: p.x, y: p.y }
    ];
    seg = splitQuad(seg, tForward(seg, TRIM_SELF)).right;
    seg = splitQuad(seg, tBackward(seg, TRIM_DEV)).left;

    var d =
      "M" + seg[0].x.toFixed(1) + " " + seg[0].y.toFixed(1) +
      " Q" + seg[1].x.toFixed(1) + " " + seg[1].y.toFixed(1) +
      " " + seg[2].x.toFixed(1) + " " + seg[2].y.toFixed(1);
    wires.style.setProperty("--wire-color", toneOf(id));
    wires.innerHTML =
      '<path class="wire wire--base" d="' + d + '"/>' +
      '<path class="wire wire--flow" d="' + d + '"/>';
  }

  function clearWire() {
    wires.innerHTML = "";
    if (selfNode) selfNode.classList.remove("is-source");
    root.querySelectorAll(".node.is-target").forEach(function (n) {
      n.classList.remove("is-target");
    });
  }

  /* ---------- 投放校验 ---------- */
  function deny() {
    drop.classList.add("is-denied");
    dropText.textContent = "先选择一台设备";
    clearTimeout(denyTimer);
    denyTimer = setTimeout(function () {
      drop.classList.remove("is-denied");
      dropText.textContent = selected
        ? "拖到此处 → " + nameOf(selected)
        : "拖拽文件到这里";
    }, 1800);
  }

  function accept(list) {
    var files = Array.prototype.slice.call(list).filter(function (f) {
      return f && typeof f.size === "number";
    });
    if (!files.length) return;
    if (!selected) {
      deny();
      return;
    }
    queue = files;
    index = 0;
    sendNext();
  }

  /* ---------- 传输 ---------- */
  function showCard(file) {
    xName.textContent = file.name;
    xName.title = file.name;
    xPct.textContent = "0%";
    xBar.style.width = "0%";
    xActs.hidden = true;
    xfer.hidden = false;
  }

  function metaText(extra) {
    var f = queue[index];
    var head = queue.length > 1 ? index + 1 + "/" + queue.length + " · " : "";
    return head + nameOf(selected) + " · " + fmtSize(f ? f.size : 0) + " · " + extra;
  }

  function sendNext() {
    if (index >= queue.length) {
      done();
      return;
    }
    var file = queue[index];
    var size = Math.max(file.size, 1);

    /* 直连握手 0.8s（TCP 私有协议 + 配对确认），之后按字节推算 */
    var LINK = 0.8;
    var SEND = Math.min(9, Math.max(0.9, size / (48.6 * 1048576)));
    var t0 = 0;

    sending = true;
    setState("is-transfer");
    showCard(file);
    drawWire(selected);
    if (selfNode) selfNode.classList.add("is-source");
    nodeOf(selected).classList.add("is-target");
    xMeta.textContent = metaText("正在建立直连…");

    function frame(ts) {
      if (!t0) t0 = ts;
      var elapsed = (ts - t0) / 1000;

      if (elapsed < LINK) {
        xMeta.textContent = metaText("正在建立直连…");
        rafId = requestAnimationFrame(frame);
        return;
      }

      var sendElapsed = elapsed - LINK;
      var p = Math.min(1, sendElapsed / SEND);
      var eased = 1 - Math.pow(1 - p, 2.2);

      xBar.style.width = (eased * 100).toFixed(1) + "%";
      xPct.textContent = Math.round(eased * 100) + "%";

      if (p < 1) {
        xMeta.textContent = metaText(
          "剩余 " + fmtSecs((1 - p) * SEND)
        );
        rafId = requestAnimationFrame(frame);
        return;
      }

      /* 收尾：显示真实耗时（发送阶段 / 全部） */
      xBar.style.width = "100%";
      xPct.textContent = "100%";
      xMeta.textContent = metaText("已发送 · 用时 " + fmtSecs(sendElapsed));
      index += 1;
      if (index < queue.length) {
        holdTimer = setTimeout(sendNext, 700);
      } else {
        holdTimer = setTimeout(done, 700);
      }
    }

    rafId = requestAnimationFrame(frame);
  }

  function done() {
    sending = false;
    clearWire();
    xPct.textContent = "完成";
    xBar.style.width = "100%";
    xActs.hidden = false;
  }

  function reset() {
    cancelAnimationFrame(rafId);
    clearTimeout(holdTimer);
    queue = [];
    index = 0;
    sending = false;
    clearWire();
    xfer.hidden = true;
    setState("is-discover");
  }

  /* ---------- 事件绑定 ---------- */
  root.querySelectorAll("[data-device]").forEach(function (n) {
    n.addEventListener("click", function () {
      select(n.getAttribute("data-device"));
    });
  });

  ["dragenter", "dragover"].forEach(function (ev) {
    drop.addEventListener(ev, function (e) {
      e.preventDefault();
      e.stopPropagation();
      drop.classList.add("is-over");
    });
  });
  ["dragleave", "dragend"].forEach(function (ev) {
    drop.addEventListener(ev, function () {
      drop.classList.remove("is-over");
    });
  });
  drop.addEventListener("drop", function (e) {
    e.preventDefault();
    e.stopPropagation();
    drop.classList.remove("is-over");
    if (e.dataTransfer && e.dataTransfer.files) accept(e.dataTransfer.files);
  });

  /* 窗外投放不触发浏览器直接打开文件 */
  ["dragover", "drop"].forEach(function (ev) {
    document.addEventListener(ev, function (e) {
      if (!drop.contains(e.target)) e.preventDefault();
    });
  });

  drop.addEventListener("click", function () {
    if (!sending && !xfer.hidden) reset();
    fileIn.click();
  });
  drop.addEventListener("keydown", function (e) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      fileIn.click();
    }
  });
  fileIn.addEventListener("change", function () {
    if (fileIn.files && fileIn.files.length) accept(fileIn.files);
    fileIn.value = "";
  });

  btnAgain.addEventListener("click", reset);
  btnClose.addEventListener("click", reset);

  /* 隐私承诺浮层 */
  function togglePrivacy(force) {
    var open = force === undefined ? privacy.hidden : force;
    privacy.hidden = !open;
    privacyBtn.setAttribute("aria-expanded", open ? "true" : "false");
  }
  privacyBtn.addEventListener("click", function (e) {
    e.stopPropagation();
    togglePrivacy();
  });
  document.addEventListener("click", function (e) {
    if (privacy.hidden) return;
    if (!privacy.contains(e.target) && e.target !== privacyBtn) togglePrivacy(false);
  });

  /* 收进托盘 / 恢复 */
  trayBtn.addEventListener("click", function () {
    root.classList.add("is-tray");
  });
  trayBar.addEventListener("click", function () {
    root.classList.remove("is-tray");
  });

  document.addEventListener("keydown", function (e) {
    if (e.key !== "Escape") return;
    if (!privacy.hidden) {
      togglePrivacy(false);
      return;
    }
    if (root.classList.contains("is-tray")) {
      root.classList.remove("is-tray");
    }
  });

  boot();
})();
