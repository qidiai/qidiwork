(() => {
  "use strict";

  // 不引入官网跟踪脚本，不使用 Cookie、浏览器存储或 URL 保存凭据。
  const byId = (id) => document.getElementById(id);
  const loginPanel = byId("login-panel");
  const dashboard = byId("dashboard");
  const passwordInput = byId("admin-password");
  const loginButton = byId("login-button");
  const period = byId("period");
  const refreshButton = byId("refresh-button");
  const status = byId("status");
  const number = new Intl.NumberFormat("zh-CN");
  const clickNames = {
    download_click: "下载点击",
    source_click: "源码点击",
    product_click: "产品点击",
    contact_click: "联系点击",
  };
  const deviceNames = { desktop: "桌面设备", mobile: "移动设备", tablet: "平板设备", other: "其他设备" };
  const metricIds = ["today-pv", "today-uv", "today-clicks", "period-pv", "period-visitor-days", "period-clicks"];
  const containerIds = ["trend-chart", "trend-table", "click-breakdown", "pages-table", "referrers-list", "devices-list"];
  let password = "";
  let controller = null;
  let session = 0;
  let busy = false;
  let loadedDays = null;

  function node(tag, text, className) {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = String(text);
    if (className) element.className = className;
    return element;
  }

  function showStatus(message, isError = false) {
    status.textContent = message;
    status.classList.toggle("error", isError);
  }

  function setBusy(value) {
    busy = value;
    loginButton.disabled = value;
    refreshButton.disabled = value;
    period.disabled = value;
    passwordInput.disabled = value;
    dashboard.setAttribute("aria-busy", String(value));
    loginButton.textContent = value && dashboard.hidden ? "正在验证…" : "登录并查看报表 →";
    refreshButton.textContent = value ? "正在加载…" : "刷新报表";
  }

  function selectView(table) {
    byId("trend-chart").hidden = table;
    byId("trend-table-wrap").hidden = !table;
    byId("chart-toggle").setAttribute("aria-pressed", String(!table));
    byId("table-toggle").setAttribute("aria-pressed", String(table));
  }

  function clearReport() {
    for (const id of metricIds) byId(id).textContent = "—";
    for (const id of containerIds) byId(id).replaceChildren();
    for (const id of ["empty-state", "pages-empty", "referrers-empty", "devices-empty"]) byId(id).hidden = true;
    byId("report-range").textContent = "等待加载报表";
    byId("updated-at").textContent = "尚未加载";
    loadedDays = null;
    period.value = "7";
    selectView(false);
  }

  function logout(message = "已退出登录，密码与报表已清空。", focus = true) {
    session += 1;
    if (controller) controller.abort();
    controller = null;
    password = "";
    passwordInput.value = "";
    dashboard.hidden = true;
    loginPanel.hidden = false;
    clearReport();
    setBusy(false);
    showStatus(message, message.includes("失效") || message.includes("不正确"));
    if (focus) passwordInput.focus();
  }

  function validCount(value) {
    return Number.isSafeInteger(value) && value >= 0;
  }

  function validMetrics(value, daily) {
    if (!value || !validCount(value.pv) || !validCount(value.clicks)) return false;
    if (!validCount(daily ? value.uv : value.visitor_days)) return false;
    return value.clicks_by_event && Object.keys(clickNames).every((key) => validCount(value.clicks_by_event[key]));
  }

  function validReport(data, days) {
    const validDate = (value) => typeof value === "string" && /^\d{4}-\d{2}-\d{2}$/.test(value);
    return data && data.days === days && data.timezone === "Asia/Shanghai"
      && validDate(data.start_date) && validDate(data.end_date)
      && typeof data.generated_at === "string" && data.generated_at.length < 64
      && Number.isFinite(Date.parse(data.generated_at))
      && validMetrics(data.today, true) && validMetrics(data.totals, false)
      && Array.isArray(data.daily) && data.daily.length === days
      && data.daily.every((row) => row && validDate(row.date) && validMetrics(row, true))
      && Array.isArray(data.pages) && data.pages.length <= 20
      && data.pages.every((row) => row && typeof row.path === "string" && row.path.length <= 128 && validCount(row.pv) && validCount(row.clicks))
      && Array.isArray(data.referrers) && data.referrers.length <= 20
      && data.referrers.every((row) => row && typeof row.hostname === "string" && row.hostname.length <= 253 && validCount(row.pv))
      && Array.isArray(data.devices) && data.devices.length <= 4
      && data.devices.every((row) => row && Object.hasOwn(deviceNames, row.device) && validCount(row.pv));
  }

  function makeProgress(value, maximum, label, className) {
    const progress = node("progress", undefined, className);
    progress.max = Math.max(1, maximum);
    progress.value = value;
    progress.setAttribute("aria-label", label);
    progress.textContent = number.format(value);
    return progress;
  }

  function renderTrend(daily) {
    const chart = document.createDocumentFragment();
    const table = document.createDocumentFragment();
    const maximum = Math.max(1, ...daily.map((row) => Math.max(row.pv, row.uv)));
    for (const row of daily) {
      const item = node("div", undefined, "trend-row");
      const date = node("time", row.date.slice(5), "trend-date");
      date.dateTime = row.date;
      date.title = row.date;
      const bars = node("div", undefined, "trend-bars");
      for (const [key, label] of [["pv", "浏览次数"], ["uv", "估算访客"]]) {
        const bar = node("div", undefined, "trend-bar");
        bar.append(makeProgress(row[key], maximum, `${row.date} ${label} ${row[key]}`, key));
        bar.append(node("span", number.format(row[key]), "bar-value"));
        bars.append(bar);
      }
      item.append(date, bars);
      chart.append(item);
      const tr = node("tr");
      const th = node("th", row.date);
      th.scope = "row";
      tr.append(th);
      for (const value of [row.pv, row.uv, row.clicks, ...Object.keys(clickNames).map((key) => row.clicks_by_event[key])]) {
        tr.append(node("td", number.format(value)));
      }
      table.append(tr);
    }
    byId("trend-chart").replaceChildren(chart);
    byId("trend-table").replaceChildren(table);
  }

  function renderBreakdowns(data) {
    const clicks = document.createDocumentFragment();
    for (const [key, label] of Object.entries(clickNames)) {
      const item = node("div", undefined, "click-item");
      item.append(node("span", label), node("strong", number.format(data.totals.clicks_by_event[key])));
      clicks.append(item);
    }
    byId("click-breakdown").replaceChildren(clicks);

    const pages = document.createDocumentFragment();
    for (const row of data.pages) {
      const tr = node("tr");
      const path = node("th", row.path, "page-path");
      path.scope = "row";
      tr.append(path, node("td", number.format(row.pv)), node("td", number.format(row.clicks)));
      pages.append(tr);
    }
    byId("pages-table").replaceChildren(pages);
    byId("pages-empty").hidden = data.pages.length > 0;

    const referrers = document.createDocumentFragment();
    for (const [index, row] of data.referrers.entries()) {
      const item = node("li");
      item.append(node("span", String(index + 1).padStart(2, "0"), "rank"), node("span", row.hostname, "hostname"), node("strong", number.format(row.pv)));
      referrers.append(item);
    }
    byId("referrers-list").replaceChildren(referrers);
    byId("referrers-empty").hidden = data.referrers.length > 0;

    const devices = document.createDocumentFragment();
    const total = data.devices.reduce((sum, row) => sum + row.pv, 0);
    for (const row of data.devices) {
      const label = deviceNames[row.device];
      const item = node("li");
      const heading = node("div", undefined, "device-heading");
      const percent = total ? (row.pv / total * 100).toFixed(1) : "0.0";
      heading.append(node("span", label), node("strong", `${number.format(row.pv)} · ${percent}%`));
      item.append(heading, makeProgress(row.pv, total, `${label} ${row.pv} 次浏览，占比 ${percent}%`, "pv"));
      devices.append(item);
    }
    byId("devices-list").replaceChildren(devices);
    byId("devices-empty").hidden = data.devices.length > 0;
  }

  function render(data) {
    const values = [data.today.pv, data.today.uv, data.today.clicks, data.totals.pv, data.totals.visitor_days, data.totals.clicks];
    metricIds.forEach((id, index) => { byId(id).textContent = number.format(values[index]); });
    byId("report-range").textContent = `${data.start_date} — ${data.end_date} · 含今天，共 ${data.days} 天`;
    byId("updated-at").textContent = "更新于 " + new Intl.DateTimeFormat("zh-CN", {
      timeZone: "Asia/Shanghai", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false,
    }).format(new Date(data.generated_at));
    byId("empty-state").hidden = data.totals.pv > 0 || data.totals.clicks > 0;
    renderTrend(data.daily);
    renderBreakdowns(data);
    loadedDays = data.days;
    period.value = String(data.days);
  }

  async function loadReport() {
    if (!password || busy) return;
    const currentSession = session;
    const signingIn = dashboard.hidden;
    const days = Number(period.value);
    const request = new AbortController();
    controller = request;
    let timedOut = false;
    const timeout = window.setTimeout(() => { timedOut = true; request.abort(); }, 10000);
    setBusy(true);
    showStatus(signingIn ? "正在验证并加载报表…" : "正在刷新报表…");
    try {
      const response = await fetch(`/api/analytics/report?days=${days}`, {
        method: "GET",
        headers: { Authorization: `Bearer ${password}`, Accept: "application/json" },
        cache: "no-store",
        credentials: "omit",
        redirect: "error",
        referrerPolicy: "no-referrer",
        signal: request.signal,
      });
      if (currentSession !== session) return;
      if (response.status === 401) {
        logout("管理密码不正确或会话已失效，请重新登录。");
        return;
      }
      if (!response.ok) {
        const messages = {
          403: "当前访问来源未获允许，请检查服务的 QIDI_ALLOWED_ORIGINS 配置。",
          404: "未找到统计接口，请确认本地统计服务或 API 转发已启用。",
          429: "请求过于频繁，请至少等待一分钟后重试。",
          500: "统计服务暂时不可用，请稍后重试。",
          502: "统计服务连接失败，请检查后端服务后重试。",
          503: "统计服务暂时不可用，请稍后重试。",
        };
        throw new Error(messages[response.status] || "报表加载失败，请检查服务后重试。");
      }
      let data;
      try {
        data = await response.json();
      } catch {
        throw new Error("报表格式不正确，请检查统计接口后重试。");
      }
      if (currentSession !== session) return;
      if (!validReport(data, days)) throw new Error("报表格式不正确，请检查统计接口后重试。");
      render(data);
      passwordInput.value = "";
      loginPanel.hidden = true;
      dashboard.hidden = false;
      showStatus("报表已更新。UV 为每日估算访客，点击不代表完成下载或后续转化。");
      if (signingIn) byId("dashboard-title").focus();
    } catch (error) {
      if (currentSession !== session) return;
      const message = timedOut ? "请求超时，请确认服务可用后重试。" : error instanceof TypeError
        ? "网络连接失败，请检查网络与统计服务后重试。" : error.message || "报表加载失败，请稍后重试。";
      showStatus(message + (loadedDays !== null ? " 当前仍显示上一次成功加载的报表。" : ""), true);
      if (loadedDays !== null) period.value = String(loadedDays);
      if (signingIn) {
        password = "";
        passwordInput.value = "";
      }
    } finally {
      window.clearTimeout(timeout);
      if (currentSession === session) {
        controller = null;
        setBusy(false);
      }
    }
  }

  byId("login-form").addEventListener("submit", (event) => {
    event.preventDefault();
    if (busy || !passwordInput.reportValidity()) return;
    password = passwordInput.value;
    passwordInput.value = "";
    void loadReport();
  });
  refreshButton.addEventListener("click", () => { void loadReport(); });
  period.addEventListener("change", () => { void loadReport(); });
  byId("logout-button").addEventListener("click", () => logout());
  byId("chart-toggle").addEventListener("click", () => selectView(false));
  byId("table-toggle").addEventListener("click", () => selectView(true));
  // 防止浏览器前进/后退缓存恢复已登录的报表与凭据。
  window.addEventListener("pagehide", () => logout("", false));
  window.addEventListener("pageshow", (event) => { if (event.persisted) logout("页面已恢复，请重新登录。", false); });
})();
