import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const root = path.dirname(fileURLToPath(import.meta.url));
execFileSync(process.execPath, [path.join(root, 'build.mjs')]);
const dist = path.join(root, 'dist');
const downloadHtml = fs.readFileSync(path.join(dist, 'download/index.html'), 'utf8');
const downloadScript = [...downloadHtml.matchAll(/<script>([\s\S]*?)<\/script>/g)][0][1];
const release = { gui: { version: '0.1.8', file: 'qidiwork-setup-0.1.8.zip', url: 'https://qidiwork.qidiai.ltd/', sha256: 'a'.repeat(64), size_bytes: 62707266, published_at: '2026-09-26T08:51:50+08:00' }, kernel: { version: '0.1.220-alpha.8' } };

async function runDownload(data, ok = true) {
  const elements = new Map();
  const context = {
    document: { getElementById(id) {
      if (!elements.has(id)) elements.set(id, { textContent: '', disabled: false, hidden: false, attrs: {}, addEventListener() {}, setAttribute(k,v) { this.attrs[k] = v; }, removeAttribute(k) { delete this.attrs[k]; if (k === 'href') delete this.href; } });
      return elements.get(id);
    } },
    fetch: async () => ({ ok, json: async () => data }),
    AbortController, setTimeout, clearTimeout,
    navigator: { clipboard: { writeText: async () => {} } },
  };
  vm.createContext(context);
  vm.runInContext(downloadScript, context);
  await context.loadRelease();
  return elements;
}

test('构建页面的本地链接、资源与锚点存在', () => {
  const pages = fs.readdirSync(dist, { recursive: true }).filter(file => file.endsWith('.html'));
  for (const file of ['blog/index.html', 'blog/free-quota-cheatsheet/index.html', 'blog/qwen-image-6gb/index.html', 'tech/free-token-resources/index.html']) {
    assert.ok(pages.some(page => page.split(path.sep).join('/') === file), `缺少已发布页面 ${file}`);
  }
  for (const file of pages) {
    const html = fs.readFileSync(path.join(dist, file), 'utf8');
    assert.ok(!html.includes('启狄'), `${file} 品牌名称`);
    assert.ok(!/@(?:PRODUCT|CASE|TECH|DIGEST)_/.test(html), `${file} 模板未替换`);
    for (const match of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
      const url = match[1];
      if (/^(?:https?:|mailto:|data:)/.test(url) || url.startsWith('/releases/')) continue;
      const parsed = new URL(url, `https://local.test/${file}`);
      let target = path.join(dist, decodeURIComponent(parsed.pathname));
      if (parsed.pathname.endsWith('/')) target = path.join(target, 'index.html');
      assert.ok(fs.existsSync(target), `${file} 缺少资源 ${url}`);
      if (parsed.hash) {
        const content = fs.readFileSync(target, 'utf8');
        assert.ok(content.includes(`id="${parsed.hash.slice(1)}"`), `${file} 缺少锚点 ${url}`);
      }
    }
  }
});

test('品牌 SVG 无脚本或外部引用，门户与下载副本和源文件一致', () => {
  const source = fs.readFileSync(path.join(root, 'src/assets/qidi-logo.svg'));
  const svg = source.toString('utf8');
  assert.match(svg, /<svg\b[^>]*\bxmlns="http:\/\/www\.w3\.org\/2000\/svg"/);
  assert.match(svg, /\bviewBox="0 0 64 64"/);
  assert.doesNotMatch(svg, /<\s*(?:[\w-]+:)?(?:script|foreignObject)\b|\bon[\w-]+\s*=|<!\s*(?:DOCTYPE|ENTITY)\b|<\?xml-stylesheet/i);
  for (const [, value] of svg.matchAll(/\b(?:xlink:)?href\s*=\s*["']([^"']*)["']/gi)) {
    assert.match(value, /^#[\w.-]+$/, 'SVG href 只能引用内部片段');
  }
  for (const [, value] of svg.matchAll(/url\(\s*([^)]*?)\s*\)/gi)) {
    assert.match(value, /^(?:#[\w.-]+|"#[\w.-]+"|'#[\w.-]+')$/, 'SVG url() 只能引用内部片段');
  }
  for (const file of ['assets/qidi-logo.svg', 'download/qidi-logo.svg']) {
    const target = path.join(dist, file);
    assert.ok(fs.existsSync(target), `缺少本地图标 ${file}`);
    assert.deepEqual(fs.readFileSync(target), source, `${file} 必须与源 SVG 完全一致`);
  }
});

test('所有生成 HTML 使用同源品牌图标与 favicon，下载页保持相对路径', () => {
  const version = createHash('sha256').update(fs.readFileSync(path.join(root, 'src/assets/qidi-logo.svg'))).digest('hex').slice(0, 12);
  const pages = fs.readdirSync(dist, { recursive: true }).filter(file => file.endsWith('.html'));
  for (const file of pages) {
    const html = fs.readFileSync(path.join(dist, file), 'utf8');
    const normalized = file.split(path.sep).join('/');
    const expected = `${normalized === 'download/index.html' ? '' : '/assets/'}qidi-logo.svg?v=${version}`;
    const header = html.match(/<header\b[^>]*>[\s\S]*?<\/header>/)?.[0] || '';
    const images = [...html.matchAll(/<img\b[^>]*\bsrc="[^"]*qidi-logo\.svg[^>]*>/g)].map(match => match[0]);
    assert.ok(images.length > 0, `${file} 缺少品牌图标`);
    assert.ok(header.includes(`src="${expected}"`), `${file} 页头未使用品牌图标`);
    for (const image of images) {
      assert.ok(image.includes(`src="${expected}"`), `${file} 品牌图标路径或哈希错误`);
      assert.match(image, /\balt=""/, `${file} 品牌图标应为装饰性图片`);
    }
    const icons = [...html.matchAll(/<link\b[^>]*\brel="(?:shortcut )?icon"[^>]*>/g)].map(match => match[0]);
    assert.ok(icons.length > 0, `${file} 缺少 favicon`);
    for (const icon of icons) assert.ok(icon.includes(`href="${expected}"`), `${file} favicon 路径或哈希错误`);
    if (normalized === 'admin/index.html' || normalized === 'download/index.html') {
      assert.match(header, /<a\b[^>]*\baria-label="[^"]+"[^>]*>\s*<img\b/, `${file} 保留品牌链接无障碍名称`);
      for (const image of images) {
        const width = Number(image.match(/\bwidth="(\d+)"/)?.[1]);
        const height = Number(image.match(/\bheight="(\d+)"/)?.[1]);
        assert.ok(width > 0 && width <= 44 && height === width, `${file} 图标须等宽高且不超过 44px`);
      }
    }
  }
});

test('CSS、JS、SVG 版本哈希对应正确源文件，下载统计脚本仍为相对资源', () => {
  const pages = fs.readdirSync(dist, { recursive: true }).filter(file => file.endsWith('.html'));
  for (const file of pages) {
    const html = fs.readFileSync(path.join(dist, file), 'utf8');
    for (const [, url, query] of html.matchAll(/(?:src|href)="(\/assets\/[^"?]+\.(?:css|js|svg)|analytics\.js|qidi-logo\.svg)([^"\s]*)"/g)) {
      const source = path.join(root, 'src/assets', url.startsWith('/assets/') ? url.slice('/assets/'.length) : url);
      const expected = createHash('sha256').update(fs.readFileSync(source)).digest('hex').slice(0, 12);
      assert.equal(query, `?v=${expected}`, `${file} ${url} 版本哈希错误`);
    }
  }
  assert.match(downloadHtml, /<script src="analytics\.js\?v=[a-f0-9]{12}" data-page="\/download\/" defer><\/script>/);
  assert.deepEqual(fs.readFileSync(path.join(dist, 'download/analytics.js')), fs.readFileSync(path.join(root, 'src/assets/analytics.js')));
});

test('已发布文章完整保留正文、日期、结构化数据与分享图片', () => {
  for (const [section, slug] of [['tech', 'free-token-resources'], ['blog', 'free-quota-cheatsheet'], ['blog', 'qwen-image-6gb']]) {
    const source = fs.readFileSync(path.join(root, `src/content/published/${slug}.html`), 'utf8');
    const html = fs.readFileSync(path.join(dist, `${section}/${slug}/index.html`), 'utf8');
    const article = /<article\b[^>]*>[\s\S]*?<\/article>/;
    const jsonLd = /<script type="application\/ld\+json">[\s\S]*?<\/script>/;
    assert.equal(html.match(article)[0], source.match(article)[0]);
    assert.equal(html.match(jsonLd)[0], source.match(jsonLd)[0]);
    assert.equal(html.match(/<link rel="canonical"[^>]*>/)[0], source.match(/<link rel="canonical"[^>]*>/)[0]);
    const title = source.match(/<title>(.*?)<\/title>/)[1].replace(/\s*·\s*启迪技术分享(?: QIDI AI)?$/, '');
    const description = source.match(/<meta name="description" content="([^"]*)"/)[1];
    for (const name of ['og', 'twitter']) {
      assert.ok(html.includes(`${name}:title" content="${title} · 启迪技术分享"`));
      assert.ok(html.includes(`${name}:description" content="${description}"`));
    }
    assert.match(html, /property="og:type" content="article"/);
    assert.equal(html.match(/<meta property="og:image"[^>]*>/)[0], source.match(/<meta property="og:image"[^>]*>/)[0]);
    assert.match(html, /href="\/assets\/site.css\?v=[a-f0-9]{12}"/);
    assert.match(html, /src="\/assets\/site.js\?v=[a-f0-9]{12}"/);
    assert.match(html, /src="\/assets\/analytics.js\?v=[a-f0-9]{12}"/);
    assert.ok(!html.includes('linksubmit/push.js'));
  }
  for (const name of ['qwen-image-1660ti-before-after.webp', 'qwen-image-1660ti-finished-road.webp']) {
    assert.deepEqual(fs.readFileSync(path.join(dist, 'assets/img', name)), fs.readFileSync(path.join(root, 'src/assets/img', name)));
  }
  const qwen = fs.readFileSync(path.join(dist, 'blog/qwen-image-6gb/index.html'), 'utf8');
  const image = new URL(qwen.match(/property="og:image" content="([^"]+)"/)[1]);
  assert.ok(fs.existsSync(path.join(dist, image.pathname)));
});

test('博客分组保留五篇原 URL，技术列表保留三篇', () => {
  const blog = fs.readFileSync(path.join(dist, 'blog/index.html'), 'utf8');
  const tech = fs.readFileSync(path.join(dist, 'tech/index.html'), 'utf8');
  const links = html => [...html.matchAll(/class="go" href="([^"]+)"/g)].map(match => match[1]).sort();
  const technical = ['/blog/qwen-image-6gb/', '/tech/multitenant-credential-layers/', '/tech/rust-provider-abstraction/'];
  const resources = ['/blog/free-quota-cheatsheet/', '/tech/free-token-resources/'];
  const groups = blog.split(/<h2[^>]*>(?:技术文章|免费资源分享)<\/h2>/);
  assert.equal(groups.length, 3);
  assert.deepEqual(links(groups[1]), technical.sort());
  assert.deepEqual(links(groups[2]), resources.sort());
  assert.deepEqual(links(tech), ['/tech/free-token-resources/', ...technical.filter(url => url.startsWith('/tech/'))].sort());
});

test('robots 复制原规则并禁止各爬虫抓取管理页，sitemap 保留线上日期', () => {
  const robots = fs.readFileSync(path.join(dist, 'robots.txt'), 'utf8');
  assert.equal(robots, fs.readFileSync(path.join(root, 'src/robots.txt'), 'utf8'));
  for (const group of robots.split('User-agent: ').slice(1)) {
    assert.match(group, /Allow: \/\r?\n/);
    assert.match(group, /Disallow: \/admin\//);
  }
  assert.match(robots, /Sitemap: https:\/\/www.qidiai.ltd\/sitemap.xml/);
  const sitemap = fs.readFileSync(path.join(dist, 'sitemap.xml'), 'utf8');
  const entries = [...sitemap.matchAll(/<url>\s*<loc>(.*?)<\/loc>\s*<lastmod>(.*?)<\/lastmod>\s*<\/url>/g)];
  assert.equal(entries.length, 9);
  assert.deepEqual(Object.fromEntries(entries.map(([, url, date]) => [new URL(url).pathname, date])), {
    '/': '2026-10-24', '/tech/': '2026-10-20',
    '/tech/rust-provider-abstraction/': '2026-09-21', '/tech/multitenant-credential-layers/': '2026-09-21',
    '/tech/free-token-resources/': '2026-10-21', '/blog/': '2026-10-21',
    '/blog/free-quota-cheatsheet/': '2026-10-21', '/blog/qwen-image-6gb/': '2026-10-21',
    '/privacy/': '2026-11-22',
  });
});

test('有效发布清单按文件名下载，不误用首页 URL', async () => {
  const elements = await runDownload(release);
  assert.equal(elements.get('dl').href, '/releases/qidiwork-setup-0.1.8.zip');
  assert.equal(elements.get('shaCopy').disabled, false);
  assert.match(elements.get('archiveHint').textContent, /先解压/);
});

test('加载失败明确禁用下载，不回退到旧版本', async () => {
  const elements = await runDownload(null, false);
  assert.equal(elements.get('dl').href, undefined);
  assert.equal(elements.get('dl').attrs['aria-disabled'], 'true');
  assert.equal(elements.get('retryDownload').hidden, false);
  assert.equal(elements.get('shaCopy').disabled, true);
});

test('缺失清单、越界路径与非法校验值不生成下载链接', async () => {
  for (const data of [{}, { ...release, gui: { ...release.gui, file: '../../private.exe' } }, { ...release, gui: { ...release.gui, sha256: '<script>' } }, { ...release, gui: { ...release.gui, size_bytes: -1 } }]) {
    const elements = await runDownload(data);
    assert.equal(elements.get('dl').href, undefined);
    assert.equal(elements.get('retryDownload').hidden, false);
  }
});

test('演示和隐私控件有可访问的初始状态', () => {
  const html = fs.readFileSync(path.join(dist, 'index.html'), 'utf8');
  assert.equal((html.match(/<button\b[^>]*data-scene=/g) || []).length, 3);
  assert.match(html, /id="pauseWorld"/);
  assert.match(html, /非实时 AI 执行/);
  assert.match(html, /id="analyticsPreference"/);
  assert.match(html, /aria-controls="siteNav"/);
  const admin = fs.readFileSync(path.join(dist, 'admin/index.html'), 'utf8');
  assert.match(admin, /noindex/);
  assert.ok(!admin.includes('src="/assets/analytics.js"'));
});

test('统计偏好跨标签同步，存储失败时仍尊重当前页关闭选择', () => {
  const handlers = {}, windowHandlers = {}, preferenceHandlers = {};
  const storage = new Map();
  const requests = [];
  const preference = { addEventListener: (event, handler) => { preferenceHandlers[event] = handler; } };
  const localStorage = { getItem: key => storage.get(key) || null, setItem: (key, value) => storage.set(key, value) };
  const context = {
    navigator: {}, location: { pathname: '/', href: 'https://www.qidiai.ltd/' }, URL,
    localStorage, window: { addEventListener: (event, handler) => { windowHandlers[event] = handler; } },
    document: {
      getElementById: () => preference, currentScript: { dataset: {} }, referrer: '', visibilityState: 'visible',
      addEventListener: (event, handler) => { handlers[event] = handler; },
    },
    fetch: (...args) => { requests.push(args); return Promise.resolve({ok:true}); },
  };
  vm.runInNewContext(fs.readFileSync(path.join(root, 'src/assets/analytics.js'), 'utf8'), context);
  const click = () => handlers.click({ target: { closest: () => ({href:'https://qidiwork.qidiai.ltd/',hasAttribute:()=>false}) } });
  assert.equal(requests.length, 1);
  storage.set('qidi-analytics-disabled', '1');
  click();
  assert.equal(requests.length, 1);
  windowHandlers.storage({key:'qidi-analytics-disabled'});
  assert.equal(preference.textContent, '开启访问统计');
  storage.set('qidi-analytics-disabled', '0');
  windowHandlers.storage({key:'qidi-analytics-disabled'});
  click();
  assert.equal(requests.length, 2);
  localStorage.setItem = () => { throw new Error('storage unavailable'); };
  preferenceHandlers.click();
  click();
  assert.equal(requests.length, 2);
});

// world 单元回归：只运行原 IIFE；模拟时间不代表浏览器帧率或主线程性能。
const worldHtml = fs.readFileSync(path.join(root, 'src/pages/index.html'), 'utf8');
const worldSource = fs.readFileSync(path.join(root, 'src/assets/site.js'), 'utf8');
const worldShapes = [
  ['city', '城市'], ['plane', '飞机'], ['car', '汽车'], ['sun', '太阳'],
  ['moon', '月亮'], ['earth', '地球'], ['universe', '宇宙'],
];

function worldEvents(target = {}) {
  const listeners = new Map();
  target.addEventListener = (type, listener) => {
    if (!listeners.has(type)) listeners.set(type, []);
    listeners.get(type).push(listener);
  };
  target.emit = (type, details = {}) => {
    assert.ok(listeners.get(type)?.length, `必须注册 ${type} 监听器`);
    for (const listener of listeners.get(type)) listener({ target, ...details });
  };
  return target;
}

// 初始属性来自真实 HTML，而不是在 mock 中预置新引擎应有的状态。
function worldNode(tag, attributes, content = '') {
  const attrs = Object.fromEntries([...attributes.matchAll(/([^\s=<>/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'))?/g)]
    .map(([, key, double, single]) => [key, double ?? single ?? '']));
  const node = worldEvents({
    tagName: tag.toUpperCase(), attrs, style: {}, innerHTML: content,
    textContent: content.replace(/<[^>]*>/g, '').trim(),
    getAttribute: key => attrs[key] ?? null,
    setAttribute: (key, value) => { attrs[key] = String(value); },
    removeAttribute: key => { delete attrs[key]; },
    hasAttribute: key => Object.hasOwn(attrs, key),
  });
  const dataKey = key => `data-${key.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`)}`;
  node.dataset = new Proxy({}, {
    get: (_, key) => attrs[dataKey(key)],
    set: (_, key, value) => { attrs[dataKey(key)] = String(value); return true; },
  });
  const classes = () => new Set((attrs.class || '').split(/\s+/).filter(Boolean));
  node.classList = {
    contains: value => classes().has(value),
    toggle(value, force) {
      const list = classes(), present = force ?? !list.has(value);
      if (present) list.add(value); else list.delete(value);
      attrs.class = [...list].join(' ');
      return present;
    },
    add(...values) { values.forEach(value => this.toggle(value, true)); },
    remove(...values) { values.forEach(value => this.toggle(value, false)); },
  };
  for (const key of ['hidden', 'disabled']) Object.defineProperty(node, key, {
    get: () => Object.hasOwn(attrs, key),
    set: value => { if (value) attrs[key] = ''; else delete attrs[key]; },
  });
  return node;
}

function worldFixture() {
  const ids = ['playground', 'particleCanvas', 'worldFallback', 'worldTitle', 'worldDesc',
    'pauseWorld', 'autoWorld', 'replayWorld', 'modelTitle', 'modelDescription',
    'sceneProgress', 'morphStatus', 'sceneRequest', 'sceneResult'];
  const elements = new Map(ids.map(id => {
    const match = worldHtml.match(new RegExp(`<([a-z][\\w-]*)\\b([^>]*\\bid="${id}"[^>]*)>([\\s\\S]*?)<\\/\\1>`));
    assert.ok(match, `首页必须包含 ${id}`);
    return [id, worldNode(match[1], match[2], match[3])];
  }));
  const buttons = [...worldHtml.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)]
    .map(([, attrs, content]) => worldNode('button', attrs, content));
  const shapes = buttons.filter(button => button.hasAttribute('data-shape'));
  const scenes = buttons.filter(button => button.hasAttribute('data-scene'));
  const list = worldHtml.match(/<ol\b[^>]*class="build-steps"[^>]*>([\s\S]*?)<\/ol>/);
  assert.ok(list, '首页必须包含步骤列表');
  const steps = [...list[1].matchAll(/<li\b([^>]*)>([\s\S]*?)<\/li>/g)]
    .map(([, attrs, content]) => worldNode('li', attrs, content));
  const get = id => elements.get(id);
  get('playground').querySelectorAll = selector => {
    const matches = { '[data-scene]': scenes, 'button[data-shape]': shapes, '.build-steps li': steps };
    assert.ok(Object.hasOwn(matches, selector), `未实现的 world 选择器：${selector}`);
    return matches[selector];
  };
  const snapshot = () => [...elements.values(), ...shapes, ...scenes, ...steps].map(node => ({
    attrs: { ...node.attrs }, text: node.textContent, html: node.innerHTML, style: { ...node.style },
  }));
  return { get, elements, shapes, scenes, steps, snapshot };
}

function runWorld({ width = 900, reduced = false, visible = true, contextMode = '2d', dpr = 1 } = {}) {
  const fixture = worldFixture();
  const { get } = fixture;
  const playground = get('playground'), canvas = get('particleCanvas');
  const initial = fixture.snapshot();
  const pending = new Map(), cancelled = [], contextRequests = [];
  let now = 0, nextId = 0, maximumPending = 0, paints = 0, calls = [];
  const context = {
    setTransform() {},
    clearRect() { paints++; calls = []; },
    fillRect() {},
    createRadialGradient: () => ({ addColorStop() {} }),
    fillText(text, x, y) {
      calls.push({ text, x, y, font: this.font, alpha: this.globalAlpha, color: this.fillStyle });
    },
  };
  canvas.width = Number(canvas.getAttribute('width'));
  canvas.height = Number(canvas.getAttribute('height'));
  canvas.parentElement = { clientWidth: width };
  canvas.getBoundingClientRect = () => ({ width, height: width * 640 / 900, left: 0, top: 0 });
  canvas.getContext = type => {
    contextRequests.push(type);
    if (contextMode === 'throw') throw new Error('Canvas unavailable');
    return contextMode === 'null' ? null : context;
  };
  playground.clientWidth = width;
  const media = worldEvents({ matches: reduced });
  const pointer = worldEvents({ matches: true });
  const document = worldEvents({ hidden: false, visibilityState: 'visible', getElementById: id => get(id) || null });
  const observers = [];
  const sandbox = {
    document, window: worldEvents({ innerWidth: width, devicePixelRatio: dpr }),
    matchMedia(query) {
      if (query === '(prefers-reduced-motion: reduce)') return media;
      assert.equal(query, '(hover: hover) and (pointer: fine)');
      return pointer;
    },
    requestAnimationFrame(callback) {
      pending.set(++nextId, callback);
      maximumPending = Math.max(maximumPending, pending.size);
      return nextId;
    },
    cancelAnimationFrame(id) { cancelled.push(id); pending.delete(id); },
    IntersectionObserver: class {
      constructor(callback) { this.callback = callback; observers.push(this); }
      observe(target) { this.target = target; }
    },
  };
  const script = worldSource.match(/\(function world\(\)\s*\{[\s\S]*?^\}\)\(\);/m)?.[0];
  assert.ok(script, '必须能够提取原 world IIFE');
  vm.runInNewContext(script, sandbox, { filename: 'site.js:world', timeout: 5000 });
  const harness = {
    ...fixture, playground, canvas, initial, pending, cancelled, contextRequests, observers,
    get paints() { return paints; },
    get calls() { return calls; },
    get scheduled() { return nextId; },
    get maximumPending() { return maximumPending; },
    click(nodeOrId) {
      const node = typeof nodeOrId === 'string' ? get(nodeOrId) : nodeOrId;
      assert.ok(node, '点击目标必须存在');
      assert.equal(node.disabled, false, '点击目标必须可用');
      assert.equal(node.hidden, false, '点击目标必须显示');
      node.emit('click');
    },
    select(key) {
      const button = fixture.shapes.find(node => node.dataset.shape === key);
      assert.ok(button, `必须存在 ${key} 形态按钮`);
      this.click(button);
    },
    intersect(isIntersecting) {
      assert.equal(observers.length, 1, 'world 必须监听画布可见性');
      assert.equal(observers[0].target, canvas);
      observers[0].callback([{ target: canvas, isIntersecting }]);
    },
    hide(hidden) {
      document.hidden = hidden;
      document.visibilityState = hidden ? 'hidden' : 'visible';
      document.emit('visibilitychange');
    },
    reduce(matches) { media.matches = matches; media.emit('change', { matches }); },
    // 每批只执行已有回调，新 RAF 留到下一帧；取消的回调绝不再执行。
    step(delta = 40) {
      now += delta;
      for (const [id, callback] of [...pending]) {
        if (!pending.delete(id)) continue;
        callback(now);
      }
    },
    advance(milliseconds) {
      for (let left = milliseconds; left > 0; left -= 40) this.step(Math.min(left, 40));
    },
    until(predicate, message, milliseconds = 10000) {
      for (let elapsed = 0; !predicate() && elapsed < milliseconds; elapsed += 40) {
        assert.ok(pending.size > 0, `${message}：不能在无 RAF 时推进动画`);
        this.step();
      }
      assert.ok(predicate(), message);
    },
  };
  if (visible && observers.length) harness.intersect(true);
  return harness;
}

function assertWorldSelection(harness, index, complete = false) {
  const [key, name] = worldShapes[index];
  assert.equal(harness.playground.dataset.shape, key);
  assert.equal(harness.get('modelTitle').textContent, name);
  const description = harness.get('modelDescription').textContent;
  assert.ok(description.length > 10, `${key} 必须有模型说明`);
  assert.ok(harness.canvas.getAttribute('aria-label').includes(`${name}：${description}`));
  assert.equal(harness.get('sceneProgress').textContent, `${String(index + 1).padStart(2, '0')} / 07`);
  assert.deepEqual(harness.shapes.map(button => button.getAttribute('aria-pressed')),
    worldShapes.map(([shape]) => String(shape === key)));
  assert.ok(harness.shapes.every(button => !button.disabled));
  if (complete) {
    assert.equal(harness.playground.dataset.morphing, 'false');
    assert.equal(harness.playground.dataset.stage, '3');
    assert.equal(harness.get('morphStatus').textContent, '构建完成');
    assert.deepEqual(harness.steps.map(step => step.getAttribute('aria-current')), [null, null, null, 'step']);
    assert.ok(harness.steps.every(step => step.classList.contains('done')));
  }
}

function worldDrawSignature(harness, expectedParticles) {
  assert.ok(harness.paints > 0, '必须真正调用 Canvas 绘制');
  assert.ok(harness.calls.length > expectedParticles, '除模型外还应绘制字符流');
  for (const call of harness.calls) {
    assert.match(call.text, /^[01<>{}/;]$/, 'draw 必须输出非空的二进制或代码字符');
    assert.ok(Number.isFinite(call.x) && Number.isFinite(call.y), '绘制坐标必须有限');
    assert.ok(call.alpha > 0 && call.alpha <= 1, '字符不能全透明');
  }
  // 模型粒子使用粗体，排除背景尘埃和输入字符流，避免空模型仍通过。
  const particles = harness.calls.filter(call => /^600 /.test(call.font));
  assert.equal(particles.length, expectedParticles, '真实绘制的模型点数必须符合宽/窄屏预算');
  const positions = particles.map(({ x, y }) => `${x.toFixed(3)},${y.toFixed(3)}`).sort();
  // 建筑表面与轮廓可共用角点，不要求粒子投影全部唯一；仍拒绝重复填充的空壳模型。
  const distinct = new Set(positions).size;
  assert.ok(distinct > expectedParticles / 2, `模型必须有足够分散的真实坐标：${distinct}/${expectedParticles}`);
  for (const [axis, minimum] of [['x', 200], ['y', 100]]) {
    const coordinates = particles.map(point => point[axis]);
    assert.ok(Math.max(...coordinates) - Math.min(...coordinates) > minimum, `模型 ${axis} 方向必须展开`);
  }
  assert.deepEqual([...new Set(particles.map(point => point.color))].sort(), ['#73dfc1', '#e8bb79']);
  return createHash('sha256').update(positions.join('|')).digest('hex');
}

function assertWorldStopped(harness) {
  assert.equal(harness.pending.size, 0);
  const before = { paints: harness.paints, scheduled: harness.scheduled, state: harness.snapshot() };
  harness.advance(12000);
  assert.equal(harness.pending.size, 0);
  assert.equal(harness.paints, before.paints, '停止时不能继续绘制');
  assert.equal(harness.scheduled, before.scheduled, '停止时不能偷偷请求 RAF');
  assert.deepEqual(harness.snapshot(), before.state, '停止时形态和步骤不能前进');
}

test('world 源码结构：Canvas、七形态、ARIA 和完整静态 fallback', () => {
  const fixture = worldFixture(), canvas = fixture.get('particleCanvas'), fallback = fixture.get('worldFallback');
  assert.equal(canvas.tagName, 'CANVAS');
  assert.equal(canvas.getAttribute('width'), '900');
  assert.equal(canvas.getAttribute('height'), '640');
  assert.equal(canvas.getAttribute('role'), 'img');
  assert.ok(canvas.getAttribute('aria-label')?.length > 10);
  for (const [, name] of worldShapes) assert.ok(canvas.textContent.includes(name), `Canvas 替代文本缺少${name}`);
  assert.deepEqual(fixture.shapes.map(button => [button.dataset.shape, button.textContent]), worldShapes);
  for (const [index, button] of fixture.shapes.entries()) {
    assert.equal(button.getAttribute('type'), 'button');
    assert.equal(button.getAttribute('aria-pressed'), String(index === 0));
    assert.equal(button.disabled, true, '无 Canvas 时不显示虚假的可用形态控件');
  }
  assert.match(worldHtml, /<div\b[^>]*class="shape-switch"[^>]*role="group"[^>]*aria-label="[^"]+"/);
  assert.equal(fallback.tagName, 'SVG');
  assert.equal(fallback.getAttribute('role'), 'img');
  assert.equal(fallback.getAttribute('aria-labelledby'), 'worldTitle worldDesc');
  for (const id of ['worldTitle', 'worldDesc']) assert.ok(fixture.get(id).textContent.length > 10);
  assert.match(fallback.innerHTML, /<path\b/);
  assert.equal(fallback.hidden, false);
  assert.notEqual(fallback.getAttribute('aria-hidden'), 'true');
  assert.equal(fixture.get('playground').classList.contains('particle-ready'), false);
  assert.equal(fixture.get('playground').dataset.stage, '3');
  for (const id of ['pauseWorld', 'autoWorld', 'replayWorld']) assert.equal(fixture.get(id).hidden, true);
  assert.equal(fixture.get('pauseWorld').getAttribute('aria-pressed'), 'false');
  assert.equal(fixture.get('autoWorld').getAttribute('aria-pressed'), 'true');
});

test('world 初始化真实绘制并按步骤汇聚，进入视口后仅启动一条 RAF', () => {
  const h = runWorld({ visible: false, dpr: 2 });
  assert.deepEqual(h.contextRequests, ['2d']);
  assert.equal(h.playground.classList.contains('particle-ready'), true);
  assert.equal(h.canvas.width, 1575, 'DPR 应封顶，不能无上限扩大画布');
  assert.equal(h.canvas.height, 1120);
  assert.equal(h.canvas.getAttribute('role'), 'img');
  assertWorldSelection(h, 0);
  assert.equal(h.playground.dataset.scene, 'together');
  assert.deepEqual(h.scenes.map(button => button.getAttribute('aria-pressed')), ['true', 'false', 'false']);
  assert.equal(h.playground.dataset.morphing, 'true');
  assert.equal(h.playground.dataset.stage, '0');
  assert.ok(h.calls.some(call => /^[01]$/.test(call.text)), '初始化不能只有清屏而无字符');
  for (const id of ['pauseWorld', 'autoWorld', 'replayWorld']) {
    assert.equal(h.get(id).hidden, false);
    assert.equal(h.get(id).disabled, false);
  }
  assertWorldStopped(h);
  h.intersect(true);
  assert.equal(h.pending.size, 1);
  for (const stage of ['1', '2', '3']) {
    h.until(() => h.playground.dataset.stage === stage, `必须实际经过阶段 ${stage}`, 2400);
  }
  assertWorldSelection(h, 0, true);
  worldDrawSignature(h, 1500);
  assert.equal(h.maximumPending, 1);
});

for (const [width, count] of [[480, 850], [900, 1500]]) {
  test(`world reduced-motion ${width}px：七形态数据集立即完成，真实字符几何各异且无 RAF`, () => {
    const h = runWorld({ width, reduced: true });
    assertWorldSelection(h, 0, true);
    assert.equal(h.get('pauseWorld').disabled, true);
    assert.equal(h.get('autoWorld').disabled, true);
    assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'false');
    assert.equal(h.get('replayWorld').disabled, false);
    const signatures = [];
    for (const [index, [key]] of worldShapes.entries()) {
      h.select(key);
      assertWorldSelection(h, index, true);
      signatures.push(worldDrawSignature(h, count));
      assert.equal(h.scheduled, 0, '减少动态时连首次 RAF 也不应请求');
    }
    assert.equal(new Set(signatures).size, worldShapes.length, '不能只换标题，七个模型必须画出不同的点集');
    h.select('city');
    assert.equal(worldDrawSignature(h, count), signatures[0], '返回城市必须恢复同一数据集');
    h.click('replayWorld');
    assertWorldSelection(h, 0, true);
    assert.equal(worldDrawSignature(h, count), signatures[0]);
    assertWorldStopped(h);
    assert.equal(h.scheduled, 0);
  });
}

test('world 暂停取消待执行 RAF，恢复不补算暂停时间且继续完成形态', () => {
  const h = runWorld();
  h.advance(400);
  const id = [...h.pending.keys()][0];
  assert.ok(id);
  h.click('pauseWorld');
  assert.ok(h.cancelled.includes(id), '暂停必须取消已排队回调，而不是只让回调空转');
  assert.equal(h.get('pauseWorld').getAttribute('aria-pressed'), 'true');
  assert.equal(h.get('pauseWorld').textContent, '继续动画');
  assert.equal(h.playground.classList.contains('paused'), true);
  assertWorldStopped(h);
  const paints = h.paints, stage = h.playground.dataset.stage;
  h.click('pauseWorld');
  assert.equal(h.get('pauseWorld').getAttribute('aria-pressed'), 'false');
  assert.equal(h.playground.classList.contains('paused'), false);
  assert.equal(h.pending.size, 1);
  h.step();
  assert.equal(h.paints, paints, '恢复首帧只重置计时，不能补算隐藏时间');
  assert.equal(h.playground.dataset.stage, stage);
  h.until(() => h.playground.dataset.morphing === 'false', '恢复后必须完成汇聚', 2400);
  assertWorldSelection(h, 0, true);
  worldDrawSignature(h, 1500);
  assert.equal(h.maximumPending, 1);
});

test('world 离屏与 document.hidden 分别取消 RAF，双重门禁全部解除才恢复', () => {
  const h = runWorld();
  h.advance(400);
  let id = [...h.pending.keys()][0];
  h.intersect(false);
  assert.ok(h.cancelled.includes(id));
  assert.equal(h.playground.classList.contains('offscreen'), true);
  assertWorldStopped(h);
  h.hide(true);
  h.intersect(true);
  assertWorldStopped(h);
  h.hide(false);
  assert.equal(h.pending.size, 1);
  assert.equal(h.playground.classList.contains('offscreen'), false);
  h.advance(120);
  id = [...h.pending.keys()][0];
  h.hide(true);
  assert.ok(h.cancelled.includes(id), '页面隐藏也必须主动取消 RAF');
  assertWorldStopped(h);
  h.intersect(false);
  h.hide(false);
  assertWorldStopped(h);
  h.intersect(true);
  assert.equal(h.pending.size, 1);
  h.until(() => h.playground.dataset.morphing === 'false', '重新可见后继续完成形态', 2400);
  assertWorldSelection(h, 0, true);
  assert.equal(h.maximumPending, 1);
});

test('world 运行时切入 reduced-motion 立即完成并取消 RAF，切出后可恢复', () => {
  const h = runWorld();
  h.advance(280);
  const id = [...h.pending.keys()][0];
  h.reduce(true);
  assert.ok(h.cancelled.includes(id));
  assertWorldSelection(h, 0, true);
  assert.equal(h.get('pauseWorld').disabled, true);
  assertWorldStopped(h);
  h.select('earth');
  assertWorldSelection(h, 5, true);
  worldDrawSignature(h, 1500);
  assertWorldStopped(h);
  h.reduce(false);
  assert.equal(h.get('pauseWorld').disabled, false);
  assert.equal(h.get('autoWorld').disabled, false);
  assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'false', '减少动态期间的手动选择也要关闭 auto');
  assert.equal(h.pending.size, 1);
  const paints = h.paints;
  h.advance(120);
  assert.ok(h.paints > paints);
  assert.equal(h.maximumPending, 1);
});

test('world auto 按七形态顺序完成并循环回城市，不跳过停留阶段', () => {
  const h = runWorld({ width: 480 });
  const visited = [];
  for (const index of [0, 1, 2, 3, 4, 5, 6, 0]) {
    const key = worldShapes[index][0];
    h.until(() => h.playground.dataset.shape === key, `自动演化必须到达 ${key}`, 8000);
    h.until(() => h.playground.dataset.morphing === 'false', `${key} 必须汇聚完成`, 2400);
    assertWorldSelection(h, index, true);
    worldDrawSignature(h, 850);
    visited.push(h.playground.dataset.shape);
    assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'true');
    h.advance(1000);
    assert.equal(h.playground.dataset.shape, key, '完成后必须停留展示而不是立即切换');
  }
  assert.deepEqual(visited, [...worldShapes.map(([key]) => key), 'city']);
  assert.equal(h.pending.size, 1);
  assert.equal(h.maximumPending, 1);
});

test('world 手动选择关闭 auto，显式开启后继续轮转，关闭后保持当前形态', () => {
  const h = runWorld({ width: 480 });
  h.select('universe');
  assertWorldSelection(h, 6);
  assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'false');
  h.until(() => h.playground.dataset.morphing === 'false', '手动形态也必须完成', 2400);
  h.advance(14000);
  assertWorldSelection(h, 6, true);
  h.click('autoWorld');
  assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'true');
  h.until(() => h.playground.dataset.shape === 'city', '显式开启 auto 后继续循环', 8000);
  h.until(() => h.playground.dataset.morphing === 'false', '循环后的城市完成', 2400);
  h.click('autoWorld');
  assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'false');
  h.advance(14000);
  assertWorldSelection(h, 0, true);
  h.click('replayWorld');
  assert.equal(h.playground.dataset.morphing, 'true');
  assert.equal(h.playground.dataset.stage, '0');
  assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'false');
  h.until(() => h.playground.dataset.morphing === 'false', '重播也必须完成', 2400);
  assertWorldSelection(h, 0, true);
  assert.equal(h.maximumPending, 1);
});

test('world 快速手动切换只保留一条 RAF，最终选中模型真实绘制', () => {
  const h = runWorld();
  for (let index = 0; index < 35; index++) {
    h.select(worldShapes[index % worldShapes.length][0]);
    assertWorldSelection(h, index % worldShapes.length);
    assert.equal(h.pending.size, 1, `第 ${index + 1} 次切换不能累积 RAF`);
    if (index % 7 === 0) h.step();
  }
  h.select('moon');
  h.until(() => h.playground.dataset.morphing === 'false', '快速切换后必须收敛至最后一次选择', 2400);
  assertWorldSelection(h, 4, true);
  worldDrawSignature(h, 1500);
  assert.equal(h.get('autoWorld').getAttribute('aria-pressed'), 'false');
  assert.equal(h.maximumPending, 1, '不能短暂创建多条 RAF 再取消来掩盖泄漏');
  h.click('pauseWorld');
  assertWorldStopped(h);
});

for (const contextMode of ['null', 'throw']) {
  test(`world Canvas context ${contextMode}：静态 fallback 与原始控件保持不变`, () => {
    const h = runWorld({ contextMode });
    assert.deepEqual(h.contextRequests, ['2d']);
    assert.deepEqual(h.snapshot(), h.initial, 'Canvas 失败不能更改静态内容、可访问性或启用伪控件');
    assert.equal(h.playground.classList.contains('particle-ready'), false, '不能用就绪样式掩盖 SVG fallback');
    assert.equal(h.playground.dataset.stage, '3');
    assert.equal(h.get('worldFallback').hidden, false);
    assert.notEqual(h.get('worldFallback').getAttribute('aria-hidden'), 'true');
    assert.equal(h.paints, 0);
    assert.equal(h.scheduled, 0);
    assertWorldStopped(h);
  });
}
