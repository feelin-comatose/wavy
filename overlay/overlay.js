// Overlay: subscribes to /events (SSE) of the local Wavy server.
(function () {
  const qs = new URLSearchParams(location.search);
  const $ = (id) => document.getElementById(id);
  const ov = $('ov');
  const base = qs.get('server') || window.__WAVY_SERVER__ || ''; // the Tauri window loads the page locally and talks to the Wavy server
  let st = null;
  let lastId = null;
  let hideTimer = null;

  const fmt = (s) => {
    if (!isFinite(s) || s <= 0) return '0:00';
    s = Math.floor(s);
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
    return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(x).padStart(2, '0');
  };

  function applyTheme(cfg) {
    const theme = qs.get('theme') || cfg.theme || 'card';
    document.body.className = 't-' + theme;
  }

  function marquee() {
    const box = document.querySelector('.title');
    const span = $('title');
    box.classList.remove('marquee');
    requestAnimationFrame(() => {
      const over = span.scrollWidth - box.clientWidth;
      if (over > 4) {
        box.style.setProperty('--x', `-${span.scrollWidth + 60 - box.clientWidth}px`);
        box.style.setProperty('--d', Math.max(8, span.scrollWidth / 25) + 's');
        box.classList.add('marquee');
      }
    });
  }

  function show() {
    ov.classList.remove('hidden');
    clearTimeout(hideTimer);
    const sec = Number(qs.get('hide') ?? st?.config?.autoHideSec ?? 0);
    if (sec > 0) hideTimer = setTimeout(() => ov.classList.add('hidden'), sec * 1000);
  }

  const flag = (k, cfg) => qs.has(k) ? qs.get(k) !== '0' : !!cfg;
  // language follows Wavy (the lang field); can be set in the link: ?lang=en
  const tr = (ru, en) => ((qs.get('lang') || st?.lang) === 'en' ? en : ru);
  let lyricIdx = -1;

  // "Next": up to three tracks; chat requests are labeled with the nickname
  function renderQueue(data) {
    const box = $('queue');
    const list = flag('queue', data.config?.showQueue) ? (data.queue || []) : [];
    box.replaceChildren(...list.map((q, i) => {
      const row = document.createElement('div');
      row.className = 'q';
      row.textContent = (i === 0 ? tr('Дальше: ', 'Next: ') : '') + (q.artist ? q.artist + ' — ' : '') + q.title + (q.by ? tr('  · от ', '  · from ') + q.by : '');
      return row;
    }));
    box.style.display = list.length ? 'block' : 'none';
  }

  function render(data) {
    const prev = st;
    st = data;
    applyTheme(data.config || {});
    renderQueue(data);
    lyricIdx = -1;
    const t = data.track;
    if (!t) { ov.classList.add('hidden'); lastId = null; return; }
    ov.classList.toggle('paused', !data.playing);
    $('label').textContent = data.playing ? tr('Сейчас играет', 'Now playing') : tr('Пауза', 'Paused');
    if (t.id !== lastId) {
      $('title').textContent = t.title;
      $('artist').textContent = t.artist || '';
      $('cover').style.backgroundImage = t.cover ? `url("${t.cover.startsWith('/') ? base + t.cover : t.cover}")` : '';
      ov.classList.remove('swap'); void ov.offsetWidth; ov.classList.add('swap');
      lastId = t.id;
      marquee();
      show();
    } else if (!prev || prev.playing !== data.playing) {
      show();
    }
  }

  function tick() {
    if (st && st.track) {
      const pos = Math.min(st.duration || Infinity, st.position + (st.playing ? (Date.now() - st.at) / 1000 : 0));
      $('fill').style.width = st.duration ? (pos / st.duration) * 100 + '%' : '0%';
      $('cur').textContent = fmt(pos);
      $('dur').textContent = fmt(st.duration);
      // current lyrics line
      const ly = flag('lyrics', st.config?.showLyrics) ? st.lyrics : null;
      let idx = -1;
      if (ly && ly.length) { for (let i = 0; i < ly.length; i++) { if (ly[i][0] <= pos + 0.2) idx = i; else break; } }
      if (idx !== lyricIdx) {
        lyricIdx = idx;
        const el = $('lyric');
        el.textContent = idx >= 0 ? ly[idx][1] : '';
        el.style.display = ly && ly.length ? 'block' : 'none';
        el.classList.remove('in'); void el.offsetWidth; el.classList.add('in');
      }
    }
    requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);

  function connect() {
    const es = new EventSource(base + '/events');
    es.onmessage = (e) => render(JSON.parse(e.data));
    es.onerror = () => { es.close(); setTimeout(connect, 2000); };
  }
  connect();

  // Tauri: moving and resizing the window in edit mode
  const T = window.__TAURI__;
  if (T && T.window) {
    const w = T.window.getCurrentWindow();
    document.addEventListener('mousedown', (e) => {
      if (!document.documentElement.classList.contains('edit') || e.button !== 0) return;
      if (e.target.classList.contains('grip')) w.startResizeDragging('SouthEast');
      else w.startDragging();
    });
  }
})();
