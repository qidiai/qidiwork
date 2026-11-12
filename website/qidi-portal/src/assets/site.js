document.documentElement.classList.add('js');

if (['www.qidiai.ltd', 'qidiai.ltd'].includes(location.hostname)) {
  const submission = document.createElement('script');
  submission.src = 'https://zz.bdstatic.com/linksubmit/push.js';
  submission.async = true;
  document.head.append(submission);
}

(function navigation() {
  const toggle = document.querySelector('.menu-toggle');
  const nav = document.getElementById('siteNav');
  if (!toggle || !nav) return;
  const close = () => {
    nav.classList.remove('is-open');
    toggle.setAttribute('aria-expanded', 'false');
    toggle.textContent = '菜单';
  };
  toggle.addEventListener('click', () => {
    const open = nav.classList.toggle('is-open');
    toggle.setAttribute('aria-expanded', String(open));
    toggle.textContent = open ? '收起' : '菜单';
  });
  nav.addEventListener('click', event => { if (event.target.closest('a')) close(); });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && nav.classList.contains('is-open')) { close(); toggle.focus(); }
  });
})();

(function world() {
  const root = document.getElementById('playground');
  const canvas = document.getElementById('particleCanvas');
  if (!root || !canvas) return;
  let context;
  try { context = canvas.getContext('2d'); } catch { return; }
  if (!context) return;
  const scenes = {
    together: {
      request: '我想把零散的业务资料，变成一个能用的项目。',
      steps: ['理解需求', '拆解任务', '协作构建', '成果落地'],
      results: ['从你的想法开始，先明确要解决的问题。', 'QidiWork 梳理资料，QIDI Code 拆解开发任务。', '代码、文档与业务流程，逐渐连接起来。', '从资料到代码，再到文档，一个项目逐步成形。'],
    },
    code: {
      request: '上传总是中断，能帮我梳理代码，加入断点续传吗？',
      steps: ['阅读代码', '定位问题', '编写修改', '验证交付'],
      results: ['先理解工程结构与上传流程，不急着修改。', '找出中断处理、分片与进度记录的关键环节。', 'QIDI Code 协助修改代码，补齐验证用例。', '输出修改与验证说明，由你审阅后应用。'],
    },
    work: {
      request: '面对一堆表格和文档，帮我整理重点并形成工作报告。',
      steps: ['读取资料', '整理要点', '分析汇总', '形成报告'],
      results: ['从你选择的工作目录和资料开始。', 'QidiWork 协助提取关键信息、整理文档结构。', '把零散的内容组织成要点、表格和待核实事项。', '形成可继续编辑的报告，重要信息由你复核。'],
    },
  };
  const media = matchMedia('(prefers-reduced-motion: reduce)');
  const pointer = matchMedia('(hover: hover) and (pointer: fine)');
  const sceneButtons = [...root.querySelectorAll('[data-scene]')];
  const shapeButtons = [...root.querySelectorAll('button[data-shape]')];
  const steps = [...root.querySelectorAll('.build-steps li')];
  const pause = document.getElementById('pauseWorld');
  const replay = document.getElementById('replayWorld');
  const auto = document.getElementById('autoWorld');
  const title = document.getElementById('modelTitle');
  const description = document.getElementById('modelDescription');
  const progress = document.getElementById('sceneProgress');
  const status = document.getElementById('morphStatus');
  const request = document.getElementById('sceneRequest');
  const result = document.getElementById('sceneResult');
  const small = (canvas.parentElement.clientWidth || root.clientWidth || window.innerWidth) < 600;
  const count = small ? 850 : 1500;
  const TAU = Math.PI * 2;
  const glyphs = ['0', '1', '0', '1', '0', '1', '<', '>', '{', '}'];
  const colors = ['#73dfc1', '#e8bb79'];
  const weights = { together: .55, code: .84, work: .23 };
  const clamp = (value, min, max) => Math.max(min, Math.min(max, value));
  const mix = (a, b, t) => a + (b - a) * t;
  const ease = t => t * t * (3 - 2 * t);
  function randomSource(seed) {
    return () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 4294967296; };
  }
  const models = [
    { key: 'city', name: '城市', text: '字符从地面网格向上生长，连接成错落有致的立体城市。', yaw: -.36, pitch: .24 },
    { key: 'plane', name: '飞机', text: '代码汇聚为机身、双翼与尾翼，让一个想法准备起飞。', yaw: -.30, pitch: .68 },
    { key: 'car', name: '汽车', text: '车身、车窗与四轮由字符共同构建，推动想法向前。', yaw: -.40, pitch: .23 },
    { key: 'sun', name: '太阳', text: '零与一凝聚为恒星，沿着日冕向外传递创造的能量。', yaw: 0, pitch: .04 },
    { key: 'moon', name: '月亮', text: '字符沿球面明暗边界排列，勾勒出一弯清晰的新月。', yaw: 0, pitch: .02 },
    { key: 'earth', name: '地球', text: '经纬线环绕球面，青绿海洋与金色大陆连接成共同的世界。', yaw: .16, pitch: .10 },
    { key: 'universe', name: '宇宙', text: '螺旋星系与交错轨道展开，让有限的字符连接无限可能。', yaw: .10, pitch: -.12 },
  ];

  function buildModel(model, seed) {
    const random = randomSource(seed);
    const parts = [];
    const point = (x, y, z, bias = 0) => ({ x, y, z, bias });
    const add = (weight, sample) => parts.push({ weight, sample });
    const line = (a, b, weight = 8, bias = 0) => add(weight, t => point(
      mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t), bias));
    function triangle(a, b, c, weight, bias = 0) {
      add(weight, () => {
        const u = Math.sqrt(random()), v = random();
        return point(...a.map((value, axis) => value * (1 - u) + b[axis] * u * (1 - v) + c[axis] * u * v), bias);
      });
    }
    function quad(a, b, c, d, weight, bias = 0) {
      triangle(a, b, c, weight / 2, bias); triangle(a, c, d, weight / 2, bias);
      [a, b, c, d].forEach((p, i, corners) => line(p, corners[(i + 1) % 4], weight / 14, bias));
    }
    function box(x, y, z, width, height, depth, weight) {
      add(weight, () => {
        const coordinates = [random() - .5, random() - .5, random() - .5];
        const areas = [height * depth, width * depth, width * height];
        const face = random() * (areas[0] + areas[1] + areas[2]);
        const axis = face < areas[0] ? 0 : face < areas[0] + areas[1] ? 1 : 2;
        coordinates[axis] = random() < .5 ? -.5 : .5;
        if (random() < .3) coordinates[(axis + 1 + Math.floor(random() * 2)) % 3] = random() < .5 ? -.5 : .5;
        if (model.key === 'city') {
          [width, height, depth].forEach((size, index) => {
            const divisions = Math.max(2, Math.round(size / 16));
            coordinates[index] = Math.round((coordinates[index] + .5) * divisions) / divisions - .5;
          });
        }
        return point(x + coordinates[0] * width, y + coordinates[1] * height, z + coordinates[2] * depth, axis === 0 ? .25 : -.1);
      });
    }
    function sphere(rx, ry, rz, weight, bias = 0) {
      add(weight, (t, i) => {
        const y = 1 - 2 * t, radius = Math.sqrt(Math.max(0, 1 - y * y));
        const angle = i * 2.399963229728653;
        return point(rx * radius * Math.cos(angle), ry * y, rz * radius * Math.sin(angle), bias);
      });
    }
    if (model.key === 'city') {
      for (let x = -250; x <= 250; x += 50) line([x, 150, -145], [x, 150, 145], 8);
      for (let z = -145; z <= 145; z += 48.3) line([-255, 150, z], [255, 150, z], 12);
      const towers = [
        [-183, 80, 55, 160, 60], [-110, -52, 64, 270, 68], [-18, -63, 63, 320, 72],
        [75, -78, 60, 230, 64], [166, -70, 62, 175, 64], [-190, -85, 46, 125, 48],
        [-100, 87, 70, 140, 64], [5, 70, 72, 200, 67], [114, 74, 69, 115, 65], [202, 68, 53, 220, 55],
      ];
      towers.forEach(([x, z, width, height, depth]) => {
        box(x, 150 - height / 2, z, width, height, depth, height * width / 230);
        for (let y = 150 - height; y < 140; y += 38) {
          line([x - width / 2, y, z - depth / 2], [x + width / 2, y, z - depth / 2], 4);
        }
      });
      line([-18, -170, -63], [-18, -195, -63], 7, -.2);
    } else if (model.key === 'plane') {
      add(350, (t, i) => {
        const x = mix(-240, 245, t), angle = i * 2.399963229728653;
        const taper = Math.pow(Math.sin(Math.PI * t), .48);
        return point(x, Math.cos(angle) * 23 * taper, Math.sin(angle) * 28 * taper);
      });
      for (const side of [-1, 1]) {
        quad([55, 3, side * 18], [-92, 7, side * 25], [-153, 0, side * 160], [-91, -5, side * 158], 130);
        quad([-161, -3, side * 13], [-228, -3, side * 14], [-236, -8, side * 81], [-202, -8, side * 79], 48);
        box(-35, 24, side * 66, 83, 24, 24, 28);
        line([182, -18, side * 10], [154, -21, side * 19], 9, -.3);
      }
      quad([-171, -13, 0], [-233, -7, 0], [-223, -91, 0], [-202, -92, 0], 55);
      line([-231, 0, 0], [240, 0, 0], 40);
    } else if (model.key === 'car') {
      box(0, 29, 0, 443, 78, 150, 290);
      box(149, -16, 0, 134, 20, 144, 42);
      box(-182, -15, 0, 74, 22, 144, 32);
      quad([-107, -105, -59], [54, -105, -59], [54, -105, 59], [-107, -105, 59], 55);
      for (const side of [-1, 1]) {
        const z = side * 74, top = side * 59;
        quad([-154, -10, z], [113, -10, z], [54, -105, top], [-107, -105, top], 74, -.32);
        line([-30, -103, top], [-30, -12, z], 18);
        line([-204, 21, z], [204, 21, z], 24);
        for (const x of [-145, 145]) {
          add(74, t => {
            const angle = t * TAU * 3, radius = 35 + random() * 9;
            return point(x + radius * Math.cos(angle), 65 + radius * Math.sin(angle), side * (83 + random() * 13), -.2);
          });
          for (let spoke = 0; spoke < 5; spoke++) {
            const a = spoke * TAU / 5;
            line([x, 65, side * 96], [x + 30 * Math.cos(a), 65 + 30 * Math.sin(a), side * 96], 3);
          }
        }
      }
      quad([113, -10, -74], [113, -10, 74], [54, -105, 59], [54, -105, -59], 50, -.32);
      quad([-154, -10, -74], [-107, -105, -59], [-107, -105, 59], [-154, -10, 74], 30, -.32);
    } else if (model.key === 'sun') {
      sphere(174, 153, 132, 650, -.36);
      for (let ray = 0; ray < 28; ray++) {
        const angle = ray * TAU / 28;
        add(11, t => {
          const a = angle + Math.sin(t * Math.PI) * .035;
          return point(Math.cos(a) * mix(185, 244 + (ray % 3) * 7, t),
            Math.sin(a) * mix(163, 188, t), Math.sin(angle * 3) * 24 * (1 - t), -.5);
        });
      }
    } else if (model.key === 'moon') {
      const radius = 180, inner = 170, shift = 75;
      const crossing = (radius * radius - inner * inner + shift * shift) / (2 * shift);
      add(620, t => {
        const y = mix(-radius, radius, t), edge = Math.sqrt(Math.max(0, radius * radius - y * y));
        const shadow = Math.abs(y) < inner ? Math.min(edge, shift - Math.sqrt(inner * inner - y * y)) : edge;
        const x = mix(-edge, shadow, random());
        const z = Math.sqrt(Math.max(0, 1 - (x * x + y * y) / (radius * radius))) * 112;
        return point(x, y, -z * (.65 + random() * .35), -.16);
      });
      const outerAngle = Math.acos(crossing / radius), innerAngle = Math.acos((crossing - shift) / inner);
      add(90, t => {
        const angle = mix(outerAngle, TAU - outerAngle, t);
        return point(radius * Math.cos(angle), radius * Math.sin(angle), 0, -.32);
      });
      add(90, t => {
        const angle = mix(innerAngle, TAU - innerAngle, t);
        const x = shift + inner * Math.cos(angle), y = inner * Math.sin(angle);
        return point(x, y, -112 * Math.sqrt(Math.max(0, 1 - (x * x + y * y) / (radius * radius))), .2);
      });
    } else if (model.key === 'earth') {
      const continents = [
        [[-168, 65], [-130, 72], [-105, 55], [-60, 52], [-78, 26], [-100, 15], [-118, 32], [-151, 57]],
        [[-81, 12], [-51, 5], [-35, -8], [-51, -29], [-69, -55], [-77, -15]],
        [[-18, 35], [9, 38], [34, 30], [51, 11], [39, -10], [20, -35], [9, -28], [-5, 5], [-17, 15]],
        [[-10, 36], [-9, 58], [24, 71], [65, 70], [110, 74], [177, 62], [145, 42], [121, 20], [109, 2], [95, 7], [79, 28], [72, 8], [54, 25], [40, 42], [26, 39]],
        [[112, -11], [137, -10], [154, -23], [146, -39], [116, -33]],
        [[-53, 60], [-24, 70], [-39, 83], [-62, 79]],
        [[45, -13], [50, -16], [47, -26], [43, -22]],
      ];
      function land(lon, lat) {
        return continents.some(polygon => {
          let inside = false;
          for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
            const [x, y] = polygon[i], [px, py] = polygon[j];
            if ((y > lat) !== (py > lat) && lon < (px - x) * (lat - y) / (py - y) + x) inside = !inside;
          }
          return inside;
        });
      }
      function globe(lon, lat) {
        const a = lon * Math.PI / 180, b = lat * Math.PI / 180;
        return point(180 * Math.cos(b) * Math.sin(a), -180 * Math.sin(b),
          -145 * Math.cos(b) * Math.cos(a), land(lon, lat) ? -.65 : .35);
      }
      add(730, (t, i) => globe(((i * 137.507764) % 360) - 180, Math.asin(1 - 2 * t) * 180 / Math.PI));
      for (let lat = -75; lat <= 75; lat += 15) add(22 * Math.cos(lat * Math.PI / 180), t => globe(t * 360 - 180, lat));
      for (let lon = -180; lon < 180; lon += 20) add(17, t => globe(lon, mix(-88, 88, t)));
    } else {
      for (let arm = 0; arm < 4; arm++) {
        add(160, t => {
          const radius = 22 + 222 * Math.sqrt(t), angle = arm * TAU / 4 + (1 - t) * 6;
          const spread = (random() - .5) * (9 + t * 20);
          return point(Math.cos(angle) * radius + spread, Math.sin(angle) * radius * .57 + spread,
            Math.sin(angle) * radius * .28 + (random() - .5) * 18, arm % 2 ? -.25 : .2);
        });
      }
      sphere(39, 30, 28, 100, -.35);
      for (const tilt of [.5, 1.7, 2.6]) {
        add(74, t => {
          const angle = t * TAU;
          return point(258 * Math.cos(angle), 172 * Math.sin(angle) * Math.cos(tilt),
            144 * Math.sin(angle) * Math.sin(tilt), .12);
        });
      }
    }
    const total = parts.reduce((sum, part) => sum + part.weight, 0);
    const points = [];
    let partIndex = 0, start = 0;
    for (let i = 0; i < count; i++) {
      const distance = (i + .5) / count * total;
      while (distance > start + parts[partIndex].weight && partIndex < parts.length - 1) start += parts[partIndex++].weight;
      points.push(parts[partIndex].sample((distance - start) / parts[partIndex].weight, i));
    }
    // 将视角烘入目标点，切换模型时相机不跳变。
    const cy = Math.cos(model.yaw), sy = Math.sin(model.yaw), cp = Math.cos(model.pitch), sp = Math.sin(model.pitch);
    const min = [Infinity, Infinity, Infinity], max = [-Infinity, -Infinity, -Infinity];
    points.forEach(p => {
      const x = p.x * cy + p.z * sy, z = -p.x * sy + p.z * cy;
      const y = p.y * cp - z * sp;
      p.z = p.y * sp + z * cp; p.x = x; p.y = y;
      [p.x, p.y, p.z].forEach((value, axis) => { min[axis] = Math.min(min[axis], value); max[axis] = Math.max(max[axis], value); });
    });
    const scale = Math.min(520 / (max[0] - min[0]), 360 / (max[1] - min[1]));
    const depth = Math.min(scale, 300 / Math.max(1, max[2] - min[2]));
    points.forEach(p => {
      p.x = (p.x - (min[0] + max[0]) / 2) * scale;
      p.y = (p.y - (min[1] + max[1]) / 2) * scale;
      p.z = (p.z - (min[2] + max[2]) / 2) * depth;
    });
    for (let i = points.length - 1; i > 0; i--) {
      const j = Math.floor(random() * (i + 1));
      [points[i], points[j]] = [points[j], points[i]];
    }
    return points;
  }
  models.forEach((model, index) => { model.points = buildModel(model, index + 41); });
  const random = randomSource(2026);
  const particles = Array.from({ length: count }, (_, index) => ({
    x: (index % 2 ? 1 : -1) * (235 + random() * 65), y: 200 + random() * 55, z: (random() - .5) * 100,
    bias: 0, channel: random(), glyph: glyphs[Math.floor(random() * glyphs.length)],
    dx: (random() - .5) * 150, dy: (random() - .5) * 140, dz: (random() - .5) * 220, delay: random() * .08,
    sx: 0, sy: 0, depth: 0, perspective: 1,
  }));
  const drawOrder = particles.slice();
  const dust = Array.from({ length: small ? 38 : 62 }, () => ({
    x: 30 + random() * 840, y: 30 + random() * 550, phase: random() * TAU,
    alpha: .07 + random() * .11, glyph: glyphs[Math.floor(random() * glyphs.length)],
  }));
  const cursor = { x: 450, y: 310, active: false };
  let scene = 'together', current = 0, stage = -1, paused = false, automatic = true, morphing = false;
  let visible = typeof IntersectionObserver === 'undefined', sized = false;
  let frame = null, lastTime = null, clock = 0, elapsed = 0, hold = 0;
  let tiltX = 0, tiltY = 0, hover = 0;
  let glowCode, glowWork;

  function renderStory() {
    const data = scenes[scene];
    root.dataset.scene = scene;
    root.dataset.stage = String(stage);
    if (request) request.textContent = data.request;
    if (result) result.textContent = data.results[stage];
    sceneButtons.forEach(button => button.setAttribute('aria-pressed', String(button.dataset.scene === scene)));
    steps.forEach((step, index) => {
      step.textContent = data.steps[index];
      step.classList.toggle('done', index <= stage);
      if (index === stage) step.setAttribute('aria-current', 'step'); else step.removeAttribute('aria-current');
    });
  }
  function setStage(value) {
    if (stage === value) return;
    stage = value; renderStory();
  }
  function renderControls() {
    shapeButtons.forEach(button => {
      button.disabled = false;
      button.setAttribute('aria-pressed', String(button.dataset.shape === models[current].key));
    });
    sceneButtons.forEach(button => { button.disabled = false; });
    if (pause) {
      pause.hidden = false; pause.disabled = media.matches;
      pause.textContent = media.matches ? '静态展示（减少动态）' : paused ? '继续动画' : '暂停动画';
      pause.setAttribute('aria-pressed', String(paused));
    }
    if (auto) {
      auto.hidden = false; auto.disabled = media.matches;
      auto.textContent = media.matches ? '自动演化不可用（减少动态）' : automatic ? '自动演化：开' : '自动演化：关';
      auto.setAttribute('aria-pressed', String(automatic && !media.matches));
    }
    if (replay) {
      replay.hidden = false; replay.disabled = false;
      replay.textContent = media.matches ? '重新呈现当前形态' : '重播当前形态';
    }
  }
  function finishMorph() {
    particles.forEach((p, index) => {
      const target = models[current].points[index];
      p.x = target.x; p.y = target.y; p.z = target.z; p.bias = target.bias;
    });
    morphing = false; elapsed = 1800; hold = 0;
    root.dataset.morphing = 'false';
    if (status) status.textContent = '构建完成';
    setStage(3);
  }
  function selectShape(index) {
    current = index; elapsed = 0; hold = 0;
    const model = models[current];
    particles.forEach(p => {
      p.fromX = p.x; p.fromY = p.y; p.fromZ = p.z; p.fromBias = p.bias;
      p.burstX = p.x * 1.13 + p.dx; p.burstY = p.y * 1.1 + p.dy; p.burstZ = p.z + p.dz;
    });
    root.dataset.shape = model.key;
    if (title) title.textContent = model.name;
    if (description) description.textContent = model.text;
    if (progress) progress.textContent = `${String(current + 1).padStart(2, '0')} / 07`;
    canvas.setAttribute('aria-label', `${model.name}：${model.text} 由零、一与代码符号组成的三维字符模型。`);
    morphing = true; root.dataset.morphing = 'true';
    if (status) status.textContent = '正在汇聚';
    if (media.matches || paused) finishMorph(); else setStage(0);
    renderControls();
  }
  function advance(delta) {
    clock += delta;
    const smoothing = 1 - Math.exp(-delta / 190);
    hover = mix(hover, cursor.active ? 1 : 0, smoothing);
    tiltX = mix(tiltX, cursor.active ? (cursor.x - 450) / 450 * .09 : 0, smoothing);
    tiltY = mix(tiltY, cursor.active ? (cursor.y - 310) / 310 * .06 : 0, smoothing);
    if (morphing) {
      elapsed = Math.min(1800, elapsed + delta);
      const fraction = elapsed / 1800;
      particles.forEach((p, index) => {
        const target = models[current].points[index];
        const t = clamp((fraction - p.delay) / (1 - p.delay), 0, 1);
        if (t < .28) {
          const spread = ease(t / .28);
          p.x = mix(p.fromX, p.burstX, spread); p.y = mix(p.fromY, p.burstY, spread); p.z = mix(p.fromZ, p.burstZ, spread);
        } else {
          const gather = ease((t - .28) / .72);
          p.x = mix(p.burstX, target.x, gather); p.y = mix(p.burstY, target.y, gather); p.z = mix(p.burstZ, target.z, gather);
        }
        p.bias = mix(p.fromBias, target.bias, ease(t));
      });
      if (fraction === 1) finishMorph(); else setStage(fraction < .28 ? 0 : fraction < .66 ? 1 : 2);
    } else if (automatic) {
      hold += delta;
      if (hold >= 6000) selectShape((current + 1) % models.length);
    }
  }
  function draw() {
    if (!sized) return;
    context.setTransform(canvas.width / 900, 0, 0, canvas.height / 640, 0, 0);
    context.clearRect(0, 0, 900, 640);
    context.globalAlpha = 1;
    context.fillStyle = glowCode; context.fillRect(0, 0, 900, 640);
    context.fillStyle = glowWork; context.fillRect(0, 0, 900, 640);
    context.textAlign = 'center'; context.textBaseline = 'middle';
    context.font = `${small ? 16 : 12}px ui-monospace, SFMono-Regular, Consolas, monospace`;
    context.fillStyle = colors[0];
    dust.forEach(p => {
      context.globalAlpha = p.alpha;
      context.fillText(p.glyph, p.x, p.y + Math.sin(clock * .0002 + p.phase) * 5);
    });
    const streamCount = small ? 42 : 62;
    const weight = weights[scene];
    for (let side = 0; side < 2; side++) {
      context.fillStyle = colors[side];
      for (let i = 0; i < streamCount; i++) {
        const t = (i / streamCount + clock * .00013) % 1;
        const spread = Math.sin(t * Math.PI);
        const x = mix(side ? 756 : 144, side ? 524 : 376, ease(t)) + Math.sin(i * 2.4 + clock * .0006) * spread * 18;
        const y = 605 - t * 217 + Math.sin(i * 1.7) * spread * 16;
        context.globalAlpha = (.12 + spread * .65) * (.35 + .65 * (side ? 1 - weight : weight));
        context.fillText(glyphs[(i + side * 3) % glyphs.length], x, y);
      }
    }
    const yaw = media.matches ? 0 : Math.sin(clock * .00016) * .045 + tiltX;
    const pitch = media.matches ? 0 : Math.sin(clock * .00012) * .024 + tiltY;
    const cy = Math.cos(yaw), sy = Math.sin(yaw), cp = Math.cos(pitch), sp = Math.sin(pitch);
    particles.forEach(p => {
      const x = p.x * cy + p.z * sy, z = -p.x * sy + p.z * cy;
      const y = p.y * cp - z * sp;
      p.depth = p.y * sp + z * cp;
      p.perspective = 850 / (850 + p.depth);
      p.sx = 450 + x * p.perspective * 1.28; p.sy = 310 + y * p.perspective * 1.28;
      if (hover > .001 && !media.matches) {
        const dx = p.sx - cursor.x, dy = p.sy - cursor.y, distance = Math.hypot(dx, dy);
        if (distance > .01 && distance < 108) {
          const push = (1 - distance / 108) ** 2 * 19 * hover;
          p.sx += dx / distance * push; p.sy += dy / distance * push;
        }
      }
    });
    drawOrder.sort((a, b) => b.depth - a.depth);
    let previousFont = 0, previousColor = -1;
    drawOrder.forEach(p => {
      const font = Math.round(clamp((small ? 18 : 14) * p.perspective, small ? 14 : 11, small ? 23 : 19));
      if (font !== previousFont) {
        context.font = `600 ${font}px ui-monospace, SFMono-Regular, Consolas, monospace`; previousFont = font;
      }
      const color = p.channel < clamp(weight + p.bias, .04, .96) ? 0 : 1;
      if (color !== previousColor) { context.fillStyle = colors[color]; previousColor = color; }
      let alpha = clamp(.73 - p.depth / 490, .22, 1);
      if (models[current].key === 'earth' && !morphing && p.depth > 0) alpha *= .35;
      context.globalAlpha = alpha;
      context.fillText(p.glyph, p.sx, p.sy);
    });
    context.globalAlpha = 1;
  }
  function canRun() { return sized && visible && !document.hidden && !paused && !media.matches; }
  function tick(now) {
    frame = null;
    if (!canRun()) { lastTime = null; return; }
    if (lastTime === null) lastTime = now;
    const delta = now - lastTime;
    if (delta >= 1000 / 30) {
      lastTime = now;
      advance(Math.min(delta, 80)); draw();
    }
    frame = requestAnimationFrame(tick);
  }
  function syncMotion() {
    root.classList.toggle('paused', paused || media.matches);
    root.classList.toggle('offscreen', !visible || document.hidden);
    if (!canRun()) {
      if (frame !== null) cancelAnimationFrame(frame);
      frame = null; lastTime = null;
    } else if (frame === null) {
      lastTime = null; frame = requestAnimationFrame(tick);
    }
  }
  function resize() {
    const width = canvas.getBoundingClientRect().width;
    sized = Number.isFinite(width) && width > 0;
    if (sized) {
      const dpr = clamp(window.devicePixelRatio || 1, 1, 1.75);
      const pixels = Math.max(1, Math.round(width * dpr));
      const height = Math.max(1, Math.round(width * 640 / 900 * dpr));
      if (canvas.width !== pixels || canvas.height !== height || !glowCode) {
        canvas.width = pixels; canvas.height = height;
        context.setTransform(pixels / 900, 0, 0, height / 640, 0, 0);
        glowCode = context.createRadialGradient(365, 290, 10, 365, 290, 290);
        glowCode.addColorStop(0, 'rgba(87,211,176,.055)'); glowCode.addColorStop(1, 'rgba(87,211,176,0)');
        glowWork = context.createRadialGradient(570, 340, 10, 570, 340, 260);
        glowWork.addColorStop(0, 'rgba(224,172,99,.045)'); glowWork.addColorStop(1, 'rgba(224,172,99,0)');
      }
      draw();
    }
    syncMotion();
  }
  function resetPointer() { cursor.active = false; }
  sceneButtons.forEach(button => button.addEventListener('click', () => {
    if (!Object.prototype.hasOwnProperty.call(scenes, button.dataset.scene)) return;
    scene = button.dataset.scene;
    const details = document.getElementById('sceneDetails');
    if (details) details.open = true;
    renderStory(); draw();
  }));
  shapeButtons.forEach(button => button.addEventListener('click', () => {
    const index = models.findIndex(model => model.key === button.dataset.shape);
    if (index < 0) return;
    automatic = false; selectShape(index); draw(); syncMotion();
  }));
  if (pause) pause.addEventListener('click', () => {
    if (media.matches) return;
    paused = !paused; renderControls(); syncMotion();
  });
  if (auto) auto.addEventListener('click', () => {
    if (media.matches) return;
    automatic = !automatic; hold = 0; renderControls(); syncMotion();
  });
  if (replay) replay.addEventListener('click', () => { paused = false; selectShape(current); draw(); syncMotion(); });
  canvas.addEventListener('pointermove', event => {
    if (!pointer.matches || media.matches || !canRun() || event.pointerType === 'touch') return;
    const bounds = canvas.getBoundingClientRect();
    if (bounds.width <= 0 || bounds.height <= 0) return;
    cursor.x = clamp((event.clientX - bounds.left) / bounds.width * 900, 0, 900);
    cursor.y = clamp((event.clientY - bounds.top) / bounds.height * 640, 0, 640);
    cursor.active = true;
  }, { passive: true });
  canvas.addEventListener('pointerleave', resetPointer, { passive: true });
  canvas.addEventListener('pointercancel', resetPointer, { passive: true });
  pointer.addEventListener('change', resetPointer);
  media.addEventListener('change', () => {
    resetPointer(); hover = 0; tiltX = 0; tiltY = 0; hold = 0;
    if (media.matches) finishMorph();
    renderControls(); syncMotion(); draw();
  });
  document.addEventListener('visibilitychange', () => { resetPointer(); syncMotion(); });
  if (typeof IntersectionObserver !== 'undefined') {
    const observer = new IntersectionObserver(entries => {
      visible = entries[0].isIntersecting;
      if (!visible) resetPointer();
      syncMotion();
    }, { threshold: 0 });
    observer.observe(canvas);
  }
  canvas.setAttribute('role', 'img');
  selectShape(0);
  root.classList.add('particle-ready');
  if (typeof ResizeObserver !== 'undefined') {
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
  }
  window.addEventListener('resize', resize, { passive: true });
  resize();
})();

(function news() {
  const feed = document.getElementById('newsFeed');
  if (!feed) return;
  const text = (tag, value, className) => {
    const element = document.createElement(tag);
    element.textContent = value;
    if (className) element.className = className;
    return element;
  };
  async function load() {
    feed.setAttribute('aria-busy', 'true');
    feed.replaceChildren(text('p', '正在获取最新资讯…', 'hint'));
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 8000);
    try {
      const response = await fetch('/data/news.json', { signal: controller.signal });
      if (!response.ok) throw new Error('加载失败');
      const data = await response.json();
      if (!Array.isArray(data.items)) throw new Error('格式错误');
      const items = data.items.filter(item => {
        if (!item || typeof item.title !== 'string' || typeof item.link !== 'string') return false;
        try { return ['http:', 'https:'].includes(new URL(item.link).protocol); } catch { return false; }
      }).slice(0, 12);
      feed.replaceChildren(text('p', items.length ? `公开资讯索引 · 更新于 ${data.updated || '暂未提供更新时间'}` : '暂无新资讯，可以先阅读左侧精选内容。', 'hint'));
      items.forEach(item => {
        const card = text('article', '', 'news-item');
        const meta = text('div', '', 'news-meta');
        meta.append(text('span', item.source || '公开来源', 'src'), text('time', item.date || ''));
        const link = text('a', item.title, 't');
        link.href = new URL(item.link).href;
        link.target = '_blank'; link.rel = 'noopener noreferrer nofollow';
        card.append(meta, link);
        if (typeof item.summary === 'string' && item.summary) card.append(text('p', item.summary, 'sum'));
        feed.append(card);
      });
    } catch {
      const button = text('button', '重新加载', 'btn btn-ghost news-retry');
      button.type = 'button'; button.addEventListener('click', load);
      feed.replaceChildren(text('p', '资讯暂时无法加载，其他页面功能不受影响。', 'hint'), button);
    } finally {
      clearTimeout(timeout);
      feed.setAttribute('aria-busy', 'false');
    }
  }
  load();
})();

(function contact() {
  const button = document.getElementById('copyEmail');
  if (!button) return;
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText('289700172@qq.com');
      button.textContent = '邮箱已复制';
    } catch { button.textContent = '请长按或选中上方邮箱复制'; }
  });
})();

(function reveal() {
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const observer = new IntersectionObserver(entries => entries.forEach(entry => {
    if (entry.isIntersecting) { entry.target.classList.add('reveal'); observer.unobserve(entry.target); }
  }), { threshold: .08 });
  document.querySelectorAll('.outcome-strip > div, .repo, .card, .case, .techcard, .about').forEach(element => observer.observe(element));
})();
