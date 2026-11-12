(() => {
  if (location.pathname.startsWith('/admin')) return;
  const preference = document.getElementById('analyticsPreference');
  const browserOptOut = navigator.doNotTrack === '1' || navigator.globalPrivacyControl === true;
  let disabled = browserOptOut;
  let localPreferenceOnly = false;
  function readPreference() {
    if (localPreferenceOnly) return;
    try { disabled = browserOptOut || localStorage.getItem('qidi-analytics-disabled') === '1'; } catch {}
  }
  readPreference();
  window.addEventListener('storage', event => {
    if (event.key === 'qidi-analytics-disabled' || event.key === null) { readPreference(); renderPreference(); }
  });
  function renderPreference() {
    if (!preference) return;
    preference.textContent = browserOptOut ? '已尊重浏览器的拒绝跟踪设置' : disabled ? '开启访问统计' : '关闭访问统计';
    preference.disabled = browserOptOut;
  }
  preference?.addEventListener('click', () => {
    disabled = !disabled;
    try { localStorage.setItem('qidi-analytics-disabled', disabled ? '1' : '0'); } catch { localPreferenceOnly = true; }
    renderPreference();
  });
  renderPreference();
  const script = document.currentScript;
  const path = script?.dataset.page || location.pathname;
  let referrer = '';
  try { referrer = document.referrer ? new URL(document.referrer).origin : ''; } catch {}
  const send = event => {
    readPreference();
    if (disabled || document.visibilityState === 'prerender') return;
    fetch('/api/analytics/event', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      credentials: 'omit', keepalive: true,
      body: JSON.stringify({ event, path, referrer }),
    }).catch(() => {});
  };
  send('pageview');
  document.addEventListener('click', event => {
    const anchor = event.target.closest('a[href]');
    if (!anchor) return;
    const url = new URL(anchor.href, location.href);
    if (anchor.hasAttribute('download') || url.hostname === 'qidiwork.qidiai.ltd' || url.pathname === '/download/') send('download_click');
    else if (['github.com', 'gitee.com'].includes(url.hostname)) send('source_click');
    else if (url.protocol === 'mailto:') send('contact_click');
    else if (['biaoshu.qidiai.ltd', 'geo.qidiai.ltd'].includes(url.hostname) || (url.hostname === 'qidiai.ltd' && url.pathname.startsWith('/quant/'))) send('product_click');
  });
})();
