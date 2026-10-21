'use strict';

/* =====================================================================
   medialib front end. No framework: a tiny element builder, a hash
   router, and one function per page. Every value from the server is
   inserted with textContent / attributes, never as HTML, because file
   and folder names are untrusted.
   ===================================================================== */

// ------------------------------------------------------------- icons
const ICONS = {
  home: '<path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M9 22V12h6v10"/>',
  film: '<rect x="2" y="2" width="20" height="20" rx="2.2"/><path d="M7 2v20M17 2v20M2 12h20M2 7h5M2 17h5M17 17h5M17 7h5"/>',
  tv: '<rect x="2" y="7" width="20" height="15" rx="2"/><path d="m17 2-5 5-5-5"/>',
  bookmark: '<path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/>',
  history: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  sliders: '<path d="M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3M14 2v4M8 10v4M16 18v4"/>',
  search: '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
  play: '<path d="M6 3.5v17a1 1 0 0 0 1.5.86l14-8.5a1 1 0 0 0 0-1.72l-14-8.5A1 1 0 0 0 6 3.5z"/>',
  layers: '<path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z"/><path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65"/><path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65"/>',
  folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
  sync: '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
  stop: '<rect x="5" y="5" width="14" height="14" rx="2"/>',
  pencil: '<path d="M21.17 6.81a1 1 0 0 0-3.99-3.99L3.84 16.17a2 2 0 0 0-.5.83l-1.32 4.35a.5.5 0 0 0 .62.62l4.35-1.32a2 2 0 0 0 .83-.5z"/><path d="m15 5 4 4"/>',
  trash: '<path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>',
  sparkles: '<path d="M12 3l1.9 5.8a2 2 0 0 0 1.3 1.3L21 12l-5.8 1.9a2 2 0 0 0-1.3 1.3L12 21l-1.9-5.8a2 2 0 0 0-1.3-1.3L3 12l5.8-1.9a2 2 0 0 0 1.3-1.3z"/><path d="M5 3v4M3 5h4M19 17v4M17 19h4"/>',
  x: '<path d="M18 6 6 18M6 6l12 12"/>',
  chevronLeft: '<path d="m15 18-6-6 6-6"/>',
  check: '<path d="M20 6 9 17l-5-5"/>',
  checkCircle: '<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>',
  star: '<path d="M11.5 2.3a.6.6 0 0 1 1 0l2.9 5.9 6.5.9a.6.6 0 0 1 .3 1l-4.7 4.6 1.1 6.5a.6.6 0 0 1-.8.6L12 18.8l-5.8 3a.6.6 0 0 1-.8-.6l1.1-6.5L1.8 10a.6.6 0 0 1 .3-1l6.5-.9z"/>',
  pin: '<path d="M20 10c0 5-5.5 10.2-7.4 11.8a1 1 0 0 1-1.2 0C9.5 20.2 4 15 4 10a8 8 0 0 1 16 0"/><circle cx="12" cy="10" r="3"/>',
  languages: '<path d="m5 8 6 6M4 14l6-6 2-3M2 5h12M7 2h1M22 22l-5-10-5 10M14 18h6"/>',
  send: '<path d="M14.5 21.7a.5.5 0 0 0 .9 0l6.5-19a.5.5 0 0 0-.6-.6l-19 6.5a.5.5 0 0 0 0 .9l7.9 3.2a2 2 0 0 1 1.1 1.1z"/><path d="m21.9 2.1-11 11"/>',
  database: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5M3 12a9 3 0 0 0 18 0"/>',
  monitor: '<rect width="20" height="14" x="2" y="3" rx="2"/><path d="M8 21h8M12 17v4"/>',
  info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
  drive: '<path d="M22 12H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11zM6 16h.01M10 16h.01"/>',
  globe: '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20M2 12h20"/>',
  external: '<path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  eye: '<path d="M2.06 12.35a1 1 0 0 1 0-.7 10.75 10.75 0 0 1 19.88 0 1 1 0 0 1 0 .7 10.75 10.75 0 0 1-19.88 0"/><circle cx="12" cy="12" r="3"/>',
  link: '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>',
  broom: '<path d="M3 6h18M7 6v14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2V6M10 11v6M14 11v6"/>',
};
const FILLED = new Set(['play', 'star']);

function icon(name, cls = '') {
  const t = document.createElement('template');
  const fill = FILLED.has(name) ? ' fill' : '';
  t.innerHTML = `<svg class="i${fill} ${cls}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name] || ''}</svg>`;
  return t.content.firstChild;
}

// ----------------------------------------------------------- helpers
const $ = (sel, root = document) => root.querySelector(sel);
const page = $('#page');

/** Element builder: h('div', {class, text, onclick, ...attrs}, ...children). */
function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  let value;
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v == null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = v;
      else if (k === 'value') value = v;
      else if (k === 'style') el.style.cssText = v;
      else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
      else if (v === true) el.setAttribute(k, '');
      else el.setAttribute(k, v);
    }
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  if (value !== undefined) el.value = value;
  return el;
}

/** replaceChildren that skips null/false, like h() does. */
function fill(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return el;
}

async function api(method, path, body) {
  const opts = { method, headers: {} };
  if (body !== undefined) {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  const r = await fetch(path, opts);
  const d = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(d.error || r.statusText || 'Request failed');
  return d;
}
const GET = (p) => api('GET', p);
const POST = (p, b) => api('POST', p, b ?? {});
const PUT = (p, b) => api('PUT', p, b);
const DEL = (p) => api('DELETE', p);

function qs(obj) {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(obj)) if (v !== undefined && v !== null && v !== '' && v !== false) p.set(k, v);
  return p.toString();
}

function toast(msg, err = false) {
  const t = h('div', { class: 'toast' + (err ? ' err' : ''), text: msg });
  $('#toasts').append(t);
  setTimeout(() => t.remove(), err ? 6000 : 3200);
}

const num = (n) => Number(n || 0).toLocaleString('en-US');
const plural = (n, word) => `${num(n)} ${word}${n === 1 ? '' : 's'}`;
function size(b) {
  if (b == null) return '';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return (i >= 3 ? b.toFixed(1) : Math.round(b)) + ' ' + u[i];
}
function ago(ts) {
  if (!ts) return 'Never synced';
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return 'Synced just now';
  if (s < 3600) return `Synced ${Math.round(s / 60)} min ago`;
  if (s < 86400) return `Synced ${Math.round(s / 3600)} h ago`;
  return `Synced ${Math.round(s / 86400)} d ago`;
}
function fmtDate(iso) {
  if (!iso) return '';
  const d = new Date(iso + 'T00:00:00');
  return isNaN(d) ? iso : d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}
const pad = (n) => String(n).padStart(2, '0');
const epCode = (e) => (e.season != null && e.episode != null) ? `S${pad(e.season)}E${pad(e.episode)}` : (e.episode != null ? `E${pad(e.episode)}` : '');
function years(c) {
  if (!c.year) return '';
  if (c.kind === 'show') {
    if (c.ongoing) return `${c.year}–`;
    if (c.year_end && c.year_end !== c.year) return `${c.year}–${c.year_end}`;
  }
  return String(c.year);
}
function runtime(min) {
  if (!min) return '';
  return min >= 60 ? `${Math.floor(min / 60)}h ${min % 60}m` : `${min} min`;
}
function playableName(name) {
  const ext = (name.split('.').pop() || '').toLowerCase();
  return ['mp4', 'm4v', 'webm', 'mov', 'ogv'].includes(ext);
}
function playerName() {
  const cmd = app.state?.flags?.player || 'player';
  const base = cmd.split(/[\\/]/).pop().replace(/\.exe$/i, '');
  return base.toLowerCase() === 'vlc' ? 'VLC' : base;
}
const go = (hash) => { location.hash = hash; };

// --------------------------------------------------------------- app
const app = {
  state: null,
  wasRunning: false,
  pageCtl: null,
  gridMemo: {},
  detailSeason: {},
  editing: null,
  navCount: 0,
};

// ----------------------------------------------------------- sidebar
function navItem(href, iconName, label, count, extra) {
  return h('a', { class: 'nav-item', href, 'data-href': href },
    icon(iconName), h('span', { class: 'label', text: label }), extra,
    count != null ? h('span', { class: 'count', text: num(count) }) : null);
}

function renderSidebar() {
  const s = app.state;
  const c = s?.counts || {};
  $('#nav-main').replaceChildren(
    navItem('#/', 'home', 'Home'),
    navItem('#/movies', 'film', 'Movies', c.movies ?? 0),
    navItem('#/shows', 'tv', 'TV Shows', c.shows ?? 0),
  );
  $('#nav-library').replaceChildren(
    navItem('#/watchlist', 'bookmark', 'Watchlist', c.watchlist || null),
    navItem('#/recent', 'history', 'Recently played', c.recent || null),
  );
  const sources = s?.sources || [];
  $('#nav-sources').replaceChildren(...(sources.length ? sources.map((src) => {
    const a = navItem('#/source/' + encodeURIComponent(src.name), src.kind === 'shows' ? 'tv' : 'film',
      src.name, src.count, src.syncing ? h('span', { class: 'nav-dot', title: 'Syncing' }) : null);
    a.classList.add('small');
    a.title = src.name;
    return a;
  }) : [h('a', { class: 'nav-item small', href: '#/settings/sources' }, icon('plus'), h('span', { class: 'label', text: 'Add a source' }))]));
  $('#nav-bottom').replaceChildren(navItem('#/settings/sources', 'sliders', 'Settings'));
  markActiveNav();
}

function markActiveNav() {
  const hash = location.hash || '#/';
  for (const a of document.querySelectorAll('#sidebar .nav-item')) {
    const href = a.getAttribute('data-href') || '';
    const on = href === '#/' ? (hash === '#/' || hash === '#' || hash === '')
      : href.startsWith('#/settings') ? hash.startsWith('#/settings')
        : hash === href;
    a.classList.toggle('on', on);
  }
}

// ------------------------------------------------- state & job pill
let pollTimer = 0;
async function refreshState() {
  clearTimeout(pollTimer);
  try {
    app.state = await GET('/api/state');
  } catch {
    pollTimer = setTimeout(refreshState, 3000);
    return;
  }
  renderSidebar();
  renderJob();
  const job = app.state.job;
  const busy = job.running || job.queued_roots.length > 0 || job.lookup_queued;
  if (app.wasRunning && !busy) onJobsFinished(job);
  app.wasRunning = busy;
  app.pageCtl?.onState?.(app.state);
  pollTimer = setTimeout(refreshState, busy ? 1200 : 8000);
}

function renderJob() {
  const job = app.state.job;
  const busy = job.running || job.queued_roots.length > 0 || job.lookup_queued;
  $('#job').hidden = !busy;
  const progress = $('#progress');
  if (!busy) {
    progress.classList.remove('indeterminate');
    $('#progress-bar').style.width = '0';
    return;
  }
  let text = job.label || 'Waiting…';
  if (job.detail) text += ' · ' + job.detail;
  const waiting = job.queued_roots.length + (job.lookup_queued ? 1 : 0);
  if (waiting) text += ` (+${waiting} queued)`;
  $('#job-text').textContent = text;
  if (job.total) {
    progress.classList.remove('indeterminate');
    $('#progress-bar').style.width = `${Math.round((100 * (job.done || 0)) / job.total)}%`;
  } else {
    progress.classList.add('indeterminate');
    $('#progress-bar').style.width = '';
  }
}

function onJobsFinished(job) {
  if (job.message) toast(job.message);
  // New titles: refresh list pages, but don't yank the user's scroll.
  if (app.pageCtl?.refresh && page.scrollTop < 300) app.pageCtl.refresh();
}

// ------------------------------------------------------------ router
const routes = [
  [/^#?\/?$/, () => homePage()],
  [/^#\/movies$/, () => gridPage({ kind: 'movie', title: 'Movies', noun: 'movie' })],
  [/^#\/shows$/, () => gridPage({ kind: 'show', title: 'TV Shows', noun: 'show' })],
  [/^#\/source\/(.+)$/, (m) => sourcePage(decodeURIComponent(m[1]))],
  [/^#\/watchlist$/, () => gridPage({ watchlist: true, title: 'Watchlist', noun: 'title', mixed: true, sort: 'watchlist', empty: ['bookmark', 'Nothing saved yet', 'Use the bookmark on any poster to save it for later.'] })],
  [/^#\/recent$/, () => recentPage()],
  [/^#\/search\/(.*)$/, (m) => searchPage(decodeURIComponent(m[1]))],
  [/^#\/title\/(\d+)$/, (m) => detailPage(Number(m[1]))],
  [/^#\/settings(?:\/(\w+))?$/, (m) => settingsPage(m[1] || 'sources')],
];

async function route() {
  app.pageCtl?.cleanup?.();
  app.pageCtl = null;
  app.navCount++;
  markActiveNav();
  const hash = location.hash || '#/';
  if (!hash.startsWith('#/search/') && document.activeElement !== $('#search')) $('#search').value = '';
  page.replaceChildren();
  page.scrollTop = 0;
  for (const [re, fn] of routes) {
    const m = hash.match(re);
    if (m) {
      try {
        app.pageCtl = (await fn(m)) || null;
      } catch (e) {
        page.replaceChildren(emptyState('info', 'Something went wrong', e.message));
      }
      return;
    }
  }
  go('#/');
}

function emptyState(iconName, title, text, ...actions) {
  return h('div', { class: 'empty' },
    h('div', { class: 'big-icon' }, icon(iconName)),
    h('h2', { text: title }),
    h('div', { text }),
    actions.length ? h('div', { class: 'actions' }, actions) : null);
}

// ------------------------------------------------------------- cards
function placeholder(c) {
  return h('div', { class: 'placeholder', text: c.name });
}

function badge(kind) {
  return h('span', { class: 'badge ' + kind }, icon('languages'), kind === 'dual' ? 'Dual audio' : 'Multi audio');
}

function cardMeta(c) {
  if (c.kind === 'show') {
    const count = c.season_count > 1 ? plural(c.season_count, 'season') : `${num(c.file_count)} eps`;
    return [years(c), count].filter(Boolean).join(' · ');
  }
  return [c.year, c.quality].filter(Boolean).join(' · ');
}

function card(c) {
  const img = c.poster
    ? h('img', { src: c.poster, loading: 'lazy', alt: '', onerror() { this.replaceWith(placeholder(c)); } })
    : placeholder(c);
  const badges = h('div', { class: 'badges' },
    c.audio === 'dual' && badge('dual'),
    c.audio === 'multi' && badge('multi'),
    c.kind === 'show' && c.ongoing && h('span', { class: 'badge ongoing', text: 'Ongoing' }));
  const mark = h('button', {
    class: 'mark' + (c.in_watchlist ? ' on' : ''), title: c.in_watchlist ? 'Remove from watchlist' : 'Add to watchlist',
    async onclick(e) {
      e.stopPropagation();
      c.in_watchlist = !c.in_watchlist;
      mark.classList.toggle('on', c.in_watchlist);
      mark.title = c.in_watchlist ? 'Remove from watchlist' : 'Add to watchlist';
      try { await POST('/api/watchlist', { title_id: c.id, on: c.in_watchlist }); refreshState(); }
      catch (err) { toast(err.message, true); }
    },
  }, icon('bookmark'));
  const play = h('button', { class: 'round-play', title: 'Play', onclick(e) { e.stopPropagation(); quickPlay(c.id); } }, icon('play'));
  return h('div', { class: 'card', onclick: () => go('#/title/' + c.id) },
    h('div', { class: 'poster' }, img, badges, h('div', { class: 'hover-actions' }, play, mark)),
    h('div', { class: 'card-title', text: c.name, title: c.name }),
    h('div', { class: 'card-meta', text: cardMeta(c) }));
}

// -------------------------------------------------------------- play
function findFile(d, fileId) {
  for (const f of d.files || []) if (f.file_id === fileId) return f;
  for (const s of d.seasons || []) {
    for (const e of s.episodes) {
      if (e.file_id === fileId) return e;
      for (const v of e.versions || []) if (v.file_id === fileId) return { ...e, ...v };
    }
  }
  return null;
}

async function quickPlay(titleId) {
  try {
    const d = await GET('/api/titles/' + titleId);
    if (!d.next_up) return toast('No video files for this title', true);
    await playFile(d, d.next_up);
  } catch (e) { toast(e.message, true); }
}

/** Plays a file in the app (browser-friendly files, if preferred) or in
 *  the external player. */
async function playFile(d, fileId, { builtin } = {}) {
  const f = findFile(d, fileId) || { name: '', file_id: fileId };
  const label = [d.name, epCode(f), f.title].filter(Boolean).join(' · ');
  const useBuiltin = builtin ?? (app.state?.flags?.prefer_builtin && playableName(f.name));
  if (useBuiltin) {
    const src = f.url && !f.url.startsWith('file:') ? f.url : `/media/${fileId}`;
    openBuiltin(src, label, d, fileId);
    POST('/api/play', { file_id: fileId, record_only: true }).catch(() => {});
    return;
  }
  try {
    await POST('/api/play', { file_id: fileId });
    toast(`Opening ${label} in ${playerName()}`);
  } catch (e) {
    toast(e.message, true);
  }
}

function openBuiltin(src, label, d, fileId) {
  const root = $('#overlay-root');
  const video = h('video', { controls: true, autoplay: true, playsinline: true });
  const close = () => { video.pause(); video.removeAttribute('src'); video.load(); root.replaceChildren(); document.removeEventListener('keydown', onKey); };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  document.addEventListener('keydown', onKey);
  const overlay = h('div', { class: 'player-overlay' },
    h('div', { class: 'player-bar' },
      h('div', { class: 't', text: label }),
      h('button', { class: 'btn small', onclick() { close(); playFile(d, fileId, { builtin: false }); } }, icon('external'), `Open in ${playerName()}`),
      h('button', { class: 'icon-btn', title: 'Close (Esc)', onclick: close }, icon('x'))),
    video);
  // The extension only hints at the codec; if the browser can't decode
  // it, say so and offer the external player.
  video.onerror = () => {
    overlay.append(h('div', { class: 'player-error' }, h('div', {},
      h('p', { text: "This video can't play inside the app — usually because it's HEVC/x265 or an MKV file." }),
      h('div', { class: 'actions', style: 'justify-content:center' },
        h('button', { class: 'btn primary', onclick() { close(); playFile(d, fileId, { builtin: false }); } }, icon('play'), `Open in ${playerName()}`),
        h('button', { class: 'btn', onclick: close, text: 'Close' })))));
  };
  video.src = src;
  root.replaceChildren(overlay);
}

// ------------------------------------------------------------- grids
const MOVIE_FILTERS = [['year', 'Any year'], ['genre', 'Any genre'], ['quality', 'Any quality'], ['platform', 'Any platform'], ['folder', 'All folders'], ['audio', 'Any audio']];
const SHOW_FILTERS = [['year', 'Any year'], ['genre', 'Any genre'], ['status', 'Any status'], ['audio', 'Any audio']];
const MIXED_FILTERS = [['kind', 'All types'], ['year', 'Any year'], ['genre', 'Any genre'], ['audio', 'Any audio']];
const SORTS = {
  movie: [['added', 'Recently added'], ['title', 'Title A–Z'], ['year', 'Newest first'], ['rating', 'Top rated']],
  show: [['updated', 'Recently updated'], ['added', 'Recently added'], ['title', 'Title A–Z'], ['year', 'Newest first'], ['rating', 'Top rated']],
  mixed: [['added', 'Recently added'], ['title', 'Title A–Z'], ['year', 'Newest first'], ['rating', 'Top rated']],
};

function selectBox(options, value, onchange, placeholderLabel) {
  const sel = h('select', { onchange() { onchange(sel.value); sel.classList.toggle('active', !!sel.value); } },
    placeholderLabel != null ? h('option', { value: '', text: placeholderLabel }) : null,
    options.map(([v, label]) => h('option', { value: v, text: label })));
  sel.value = value ?? '';
  if (placeholderLabel != null && sel.value) sel.classList.add('active');
  return h('div', { class: 'select' }, sel);
}

/** A poster grid with filters, sort and infinite scroll. */
async function gridPage(o) {
  const key = location.hash;
  const sortKey = o.mixed ? 'mixed' : o.kind;
  const memo = app.gridMemo[key] || (app.gridMemo[key] = {
    f: { sort: o.sort || (o.kind === 'show' ? 'updated' : 'added') }, loaded: 0, top: 0,
  });
  const sorts = o.sort === 'watchlist' ? [['watchlist', 'Recently saved'], ...SORTS.mixed] : SORTS[sortKey];
  const defs = o.mixed ? MIXED_FILTERS : (o.kind === 'show' ? SHOW_FILTERS : MOVIE_FILTERS);

  const sub = h('div', { class: 'subtitle', text: ' ' });
  const filters = h('div', { class: 'filters' });
  const grid = h('div', { class: 'grid' });
  const more = h('div', { class: 'loading-more' });
  const sentinel = h('div', { class: 'sentinel' });
  const head = h('div', { class: 'actions', style: 'justify-content:space-between;align-items:flex-end' },
    h('div', {}, h('h1', { class: 'page-title', text: o.title }), sub), o.headActions || null);
  page.append(h('div', { class: 'wrap' }, head, filters, grid, more, sentinel));

  const base = () => ({ kind: o.kind, root: o.root, watchlist: o.watchlist, q: o.q, ...memo.f });
  let offset = 0, done = false, loading = false, gen = 0;

  async function loadFacets() {
    let f;
    try { f = await GET('/api/facets?' + qs(base())); } catch { return; }
    const opt = (list, fmt) => list.map((x) => [x.value, fmt ? fmt(x) : `${x.label}`]);
    const lists = {
      year: opt(f.years),
      genre: opt(f.genres),
      quality: opt(f.qualities),
      platform: opt(f.platforms),
      folder: opt(f.folders),
      audio: [...opt(f.audios), ['single', 'Single audio']],
      status: [['ongoing', 'Ongoing'], ['ended', 'Ended']],
      kind: [['movie', 'Movies'], ['show', 'TV shows']],
    };
    const boxes = defs.map(([name, label]) => {
      const box = selectBox(lists[name], memo.f[name], (v) => { memo.f[name] = v || undefined; reset(); }, label);
      if (!lists[name].length && !memo.f[name]) {
        box.firstChild.disabled = true;
        box.title = name === 'genre' ? 'Add a TMDB key in Settings → Movie info to filter by genre' : 'Nothing to filter by yet';
      }
      return box;
    });
    const active = defs.some(([n]) => memo.f[n]);
    fill(filters, ...boxes,
      active ? h('button', { class: 'clear-filters', text: 'Clear', onclick() { for (const [n] of defs) delete memo.f[n]; reset(); } }) : null,
      h('div', { class: 'spacer' }),
      h('span', { class: 'sort-label', text: 'Sort' }),
      selectBox(sorts, memo.f.sort, (v) => { memo.f.sort = v; reset(); }));
  }

  async function loadMore(n = 60) {
    if (loading || done) return;
    loading = true;
    const my = gen;
    more.textContent = offset ? 'Loading…' : '';
    let p;
    try {
      p = await GET('/api/titles?' + qs({ ...base(), offset, limit: n }));
    } catch (e) {
      loading = false;
      more.textContent = e.message;
      return;
    }
    if (my !== gen) return;
    sub.textContent = o.subtitle ? o.subtitle(p.total) : plural(p.total, o.noun);
    if (!p.total) {
      const filtered = defs.some(([n]) => memo.f[n]);
      grid.replaceChildren(filtered
        ? emptyState('search', 'No matches', 'Nothing fits these filters.', h('button', { class: 'btn', text: 'Clear filters', onclick() { for (const [n] of defs) delete memo.f[n]; reset(); } }))
        : o.empty ? emptyState(...o.empty) : libraryEmpty());
      grid.style.display = 'block';
    } else {
      grid.style.display = '';
      grid.append(...p.items.map(card));
    }
    offset += p.items.length;
    memo.loaded = offset;
    done = offset >= p.total;
    more.textContent = '';
    loading = false;
  }

  function reset() {
    gen++;
    offset = 0; done = false; loading = false;
    grid.replaceChildren();
    loadFacets();
    loadMore();
  }

  const obs = new IntersectionObserver((entries) => {
    if (entries.some((e) => e.isIntersecting)) loadMore();
  }, { root: page, rootMargin: '900px' });
  obs.observe(sentinel);

  loadFacets();
  await loadMore(Math.min(500, Math.max(60, memo.loaded)));
  page.scrollTop = memo.top;

  return {
    cleanup() { obs.disconnect(); memo.top = page.scrollTop; },
    refresh() { memo.loaded = 0; reset(); },
  };
}

function libraryEmpty() {
  if (!app.state?.sources?.length) {
    return emptyState('folder', 'Your library is empty', 'Add a link to your media server, or a folder on this PC, to get started.',
      h('a', { class: 'btn primary', href: '#/settings/sources' }, icon('plus'), 'Add a source'));
  }
  const busy = app.state.job.running;
  return emptyState(busy ? 'sync' : 'film', busy ? 'Syncing your sources…' : 'Nothing here yet',
    busy ? 'Titles appear as soon as each source finishes syncing.' : 'Sync your sources to fill the library.',
    busy ? null : h('button', { class: 'btn primary', async onclick() { await POST('/api/sync').catch((e) => toast(e.message, true)); refreshState(); } }, icon('sync'), 'Sync now'));
}

function sourcePage(name) {
  const src = app.state?.sources?.find((s) => s.name === name);
  if (!src) return gridPage({ root: name, title: name, noun: 'title', mixed: true });
  const kind = src.kind === 'shows' ? 'show' : 'movie';
  const head = h('div', { class: 'actions' },
    h('button', { class: 'btn', async onclick() { await POST(`/api/sources/${encodeURIComponent(name)}/sync`).catch((e) => toast(e.message, true)); refreshState(); } }, icon('sync'), 'Sync'),
    h('a', { class: 'btn ghost', href: '#/settings/sources' }, icon('sliders'), 'Manage'));
  return gridPage({ root: name, kind, title: name, noun: kind, headActions: head });
}

function searchPage(q) {
  const input = $('#search');
  if (document.activeElement !== input) input.value = q;
  return gridPage({ q, title: q ? `Results for “${q}”` : 'Search', noun: 'result', mixed: true,
    empty: ['search', 'No matches', 'Try fewer words, or another spelling.'] });
}

async function recentPage() {
  const wrap = h('div', { class: 'wrap' });
  page.append(wrap);
  const d = await GET('/api/recent');
  wrap.append(h('h1', { class: 'page-title', text: 'Recently played' }),
    h('div', { class: 'subtitle', text: plural(d.total, 'title') }));
  if (!d.items.length) {
    wrap.append(emptyState('history', 'Nothing played yet', 'Titles you play show up here, newest first.'));
  } else {
    wrap.append(h('div', { class: 'grid' }, d.items.map(card)));
  }
}

// -------------------------------------------------------------- home
async function homePage() {
  const wrap = h('div', { class: 'wrap' });
  page.append(wrap);
  if (!app.state) await refreshState();
  if (!app.state?.sources?.length) {
    wrap.append(welcome());
    return;
  }
  const { rows } = await GET('/api/home');
  if (!rows.length) {
    wrap.append(libraryEmpty());
    return { refresh: () => route() };
  }
  // A featured title with artwork, when movie info is available.
  const pool = [...(rows.find((r) => r.title === 'Top rated movies')?.items || []), ...(rows.find((r) => r.title === 'Recently added movies')?.items || [])];
  const featured = pool.slice(0, 12)[Math.floor(Math.random() * Math.min(12, pool.length))];
  const heroSlot = h('div');
  wrap.append(heroSlot);
  if (featured && app.state.flags.tmdb) {
    GET('/api/titles/' + featured.id).then((d) => {
      if (!d.backdrop) return;
      heroSlot.replaceWith(h('div', { class: 'hero-home' },
        h('div', { class: 'bg', style: `background-image:url("${encodeURI(d.backdrop)}")` }),
        h('div', { class: 'content' },
          h('div', { class: 'eyebrow', text: 'Featured from your library' }),
          h('h2', { text: d.name }),
          h('div', { class: 'meta-row', style: 'margin-bottom:12px' },
            d.year && h('span', { text: years(d) }),
            d.rating && h('span', { class: 'star' }, icon('star'), d.rating.toFixed(1)),
            d.genres.slice(0, 3).join(' · ')),
          d.overview && h('p', { text: d.overview }),
          h('div', { class: 'actions' },
            h('button', { class: 'btn primary big', onclick: () => playFile(d, d.next_up) }, icon('play'), 'Play'),
            h('a', { class: 'btn big', href: '#/title/' + d.id }, icon('info'), 'More info')))));
    }).catch(() => {});
  }
  for (const r of rows) {
    wrap.append(
      h('div', { class: 'row-head' }, h('h2', { text: r.title }), r.link && h('a', { href: r.link, text: 'See all' })),
      h('div', { class: 'strip' }, r.items.map(card)));
  }
  return { refresh: () => route() };
}

function welcome() {
  return h('div', { class: 'empty' },
    h('div', { class: 'big-icon' }, icon('film')),
    h('h2', { text: 'Welcome to medialib' }),
    h('div', { text: 'Add a link to your media server (or a folder on this PC). Everything inside is indexed into a searchable poster library.' }),
    h('div', { class: 'actions' }, h('a', { class: 'btn primary big', href: '#/settings/sources' }, icon('plus'), 'Add your first source')));
}

// ------------------------------------------------------------ detail
async function detailPage(id) {
  const d = await GET('/api/titles/' + id);
  const isShow = d.kind === 'show';
  const listHash = isShow ? '#/shows' : '#/movies';

  const backdrop = d.backdrop
    ? h('div', { class: 'backdrop', style: `background-image:url("${encodeURI(d.backdrop)}")` })
    : d.poster ? h('div', { class: 'backdrop blur', style: `background-image:url("${encodeURI(d.poster)}")` }) : null;

  const episodesCount = (d.seasons || []).reduce((n, s) => n + s.episodes.length, 0);
  const next = findFile(d, d.next_up);
  const meta = h('div', { class: 'meta-row' },
    years(d) && h('span', { text: years(d) }),
    d.rating && h('span', { class: 'star' }, icon('star'), d.rating.toFixed(1)),
    isShow && d.ongoing && h('span', { class: 'chip green', text: 'Ongoing' }),
    d.quality && h('span', { class: 'chip outline', text: d.quality }),
    d.audio && h('span', { class: 'chip teal', text: d.audio === 'multi' ? 'Multi Audio' : 'Dual Audio' }),
    d.platform && h('span', { class: 'chip outline', text: d.platform }),
    isShow ? h('span', { style: 'color:var(--muted)', text: `${plural(d.seasons.length, 'season')} · ${plural(episodesCount, 'episode')}` })
      : h('span', { style: 'color:var(--muted)', text: [runtime(d.runtime), d.total_size && size(d.total_size)].filter(Boolean).join(' · ') }));

  const genres = (d.genres.length || d.country) ? h('div', { class: 'genres' },
    d.genres.map((g) => h('span', { class: 'pill', text: g })),
    d.country && h('span', { class: 'place' }, icon('pin'), d.country)) : null;

  const mark = h('button', {
    class: 'btn big square' + (d.in_watchlist ? ' on' : ''), title: d.in_watchlist ? 'Remove from watchlist' : 'Add to watchlist',
    async onclick() {
      d.in_watchlist = !d.in_watchlist;
      mark.classList.toggle('on', d.in_watchlist);
      mark.title = d.in_watchlist ? 'Remove from watchlist' : 'Add to watchlist';
      try { await POST('/api/watchlist', { title_id: d.id, on: d.in_watchlist }); refreshState(); toast(d.in_watchlist ? 'Added to watchlist' : 'Removed from watchlist'); }
      catch (e) { toast(e.message, true); }
    },
  }, icon('bookmark'));

  const playLabel = isShow ? `Play ${epCode(next || {}) || ''}`.trim() : 'Play';
  const nextPlayable = next && playableName(next.name);
  const actions = h('div', { class: 'actions detail-actions' },
    d.next_up && h('button', { class: 'btn primary big', onclick: () => playFile(d, d.next_up).then(() => setTimeout(() => refreshDetail(d.id), 600)) }, icon('play'), playLabel),
    isShow && h('button', {
      class: 'btn big', async onclick() {
        try {
          const r = await POST('/api/playlist', { title_id: d.id, from_file_id: d.next_up });
          toast(`Playlist of ${plural(r.count, 'episode')} sent to ${playerName()}`);
          setTimeout(() => refreshDetail(d.id), 600);
        } catch (e) { toast(e.message, true); }
      },
    }, icon('layers'), 'Play as playlist'),
    !isShow && nextPlayable && h('button', {
      class: 'btn big', onclick: () => playFile(d, d.next_up, { builtin: !app.state.flags.prefer_builtin }),
    }, icon(app.state.flags.prefer_builtin ? 'external' : 'eye'), app.state.flags.prefer_builtin ? `Open in ${playerName()}` : 'Watch here'),
    mark,
    h('button', {
      class: 'btn big square', title: 'Open folder',
      async onclick() { try { await POST('/api/open-folder', { title_id: d.id }); } catch (e) { toast(e.message, true); } },
    }, icon('folder')));

  const fix = h('div', { class: 'fix-match' },
    app.state?.flags?.tmdb
      ? h('button', { class: 'link muted', text: d.tmdb_status === 'ok' ? 'Wrong info? Fix the match' : 'No info found — search TMDB', onclick: () => fixMatch(d) })
      : h('a', { class: 'link muted', href: '#/settings/info', text: 'Add posters, ratings and plots: set up Movie info' }));

  const hero = h('div', { class: 'detail-hero' }, backdrop,
    h('div', { class: 'detail-inner' },
      h('a', { class: 'back', onclick() { app.navCount > 1 ? history.back() : go(listHash); } }, icon('chevronLeft'), isShow ? 'TV Shows' : 'Movies'),
      h('div', { class: 'detail-top' },
        h('div', { class: 'detail-poster' }, d.poster ? h('img', { src: d.poster, alt: '', onerror() { this.replaceWith(placeholder(d)); } }) : placeholder(d)),
        h('div', { class: 'detail-info' },
          h('h1', { text: d.name }),
          meta, genres,
          d.tagline && h('p', { class: 'tagline', text: d.tagline }),
          d.overview && h('p', { class: 'overview', text: d.overview }),
          d.credits && h('div', { class: 'credits' }, d.credits_label, h('b', { text: d.credits })),
          actions, fix))));

  const container = h('div');
  container.append(hero);
  if (isShow) container.append(...seasonsView(d));
  else container.append(filesView(d));
  page.append(container);
  return { cleanup() {} };
}

async function refreshDetail(id) {
  if (location.hash !== '#/title/' + id) return;
  const top = page.scrollTop;
  page.replaceChildren();
  app.pageCtl = (await detailPage(id)) || null;
  page.scrollTop = top;
}

function seasonsView(d) {
  if (!d.seasons.length) return [h('div', { class: 'files' }, h('div', { class: 'subtitle', text: 'No episodes found.' }))];
  const nextSeason = d.seasons.find((s) => s.episodes.some((e) => e.file_id === d.next_up || (e.versions || []).some((v) => v.file_id === d.next_up)))?.season;
  let current = app.detailSeason[d.id] ?? nextSeason ?? d.seasons[0].season;
  if (!d.seasons.some((s) => s.season === current)) current = d.seasons[0].season;

  const bar = h('div', { class: 'seasons-bar' });
  const list = h('div', { class: 'episodes' });
  function render() {
    const s = d.seasons.find((x) => x.season === current);
    bar.replaceChildren(
      ...d.seasons.map((x) => h('button', {
        class: 'season-pill' + (x.season === current ? ' on' : ''),
        onclick() { current = x.season; app.detailSeason[d.id] = current; render(); },
      }, x.season === 0 ? 'Specials' : `Season ${x.season}`, h('small', { text: x.episodes.length }))),
      h('span', { class: 'total', text: `${plural(s.episodes.length, 'episode')}${s.size ? ' · ' + size(s.size) : ''}` }));
    list.replaceChildren(...s.episodes.map((e) => episodeRow(d, e)));
  }
  render();
  return [bar, list];
}

function episodeRow(d, e) {
  const isNext = e.file_id === d.next_up || (e.versions || []).some((v) => v.file_id === d.next_up);
  const title = e.title || (e.episode != null ? `Episode ${e.episode}` : e.name);
  const still = h('div', { class: 'still' },
    e.still ? h('img', { src: e.still, loading: 'lazy', alt: '' }) : h('div', { class: 'ep-num', text: e.episode != null ? pad(e.episode) : '▶' }),
    h('div', { class: 'play-hint' }, h('span', { class: 'round-play' }, icon('play'))));
  const check = h('button', {
    class: 'ep-check' + (e.watched ? ' on' : ''), title: e.watched ? 'Mark as unwatched' : 'Mark as watched',
    async onclick(ev) {
      ev.stopPropagation();
      e.watched = !e.watched;
      check.classList.toggle('on', e.watched);
      check.title = e.watched ? 'Mark as unwatched' : 'Mark as watched';
      const ids = [e.file_id, ...(e.versions || []).map((v) => v.file_id)];
      try { for (const fid of new Set(ids)) await POST('/api/watched', { file_id: fid, on: e.watched }); }
      catch (err) { toast(err.message, true); }
    },
  }, icon(e.watched ? 'checkCircle' : 'check'));
  const versions = (e.versions || []).length > 1 ? h('div', { class: 'versions' },
    e.versions.map((v) => h('button', {
      class: 'version', title: v.name,
      text: [v.quality, v.audio === 'dual' ? 'Dual audio' : v.audio === 'multi' ? 'Multi audio' : v.audio === 'dubbed' ? 'Dubbed' : 'Original', v.size && size(v.size)].filter(Boolean).join(' · '),
      onclick(ev) { ev.stopPropagation(); playFile(d, v.file_id).then(() => setTimeout(() => refreshDetail(d.id), 600)); },
    }))) : null;
  return h('div', {
    class: 'episode' + (isNext ? ' next' : ''), title: e.name,
    onclick: () => playFile(d, e.file_id).then(() => setTimeout(() => refreshDetail(d.id), 600)),
  },
  still,
  h('div', { class: 'ep-body' },
    h('div', { class: 'ep-code' }, epCode(e) || 'Episode', isNext && h('span', { class: 'up-next', text: 'Up next' })),
    h('div', { class: 'ep-title', text: title }),
    h('div', { class: 'ep-meta', text: [fmtDate(e.air_date), e.runtime && `${e.runtime} min`, e.size && size(e.size)].filter(Boolean).join(' · ') }),
    e.overview && h('div', { class: 'ep-overview', text: e.overview }),
    versions),
  check);
}

function filesView(d) {
  const box = h('div', { class: 'files' }, h('h3', { text: d.files.length > 1 ? `${d.files.length} versions` : 'File' }));
  for (const f of d.files) {
    const check = h('button', {
      class: 'btn small ghost' + (f.watched ? ' on' : ''), title: 'Watched',
      async onclick() {
        f.watched = !f.watched;
        check.classList.toggle('on', f.watched);
        await POST('/api/watched', { file_id: f.file_id, on: f.watched }).catch((e) => toast(e.message, true));
      },
    }, icon('check'));
    box.append(h('div', { class: 'file-row' },
      h('div', { class: 'name' }, f.name, h('div', { class: 'sub', text: [f.quality, f.size && size(f.size), f.played_at && 'played'].filter(Boolean).join(' · ') })),
      playableName(f.name) && h('button', { class: 'btn small', onclick: () => playFile(d, f.file_id, { builtin: true }) }, icon('eye'), 'Watch here'),
      h('button', { class: 'btn small primary', onclick: () => playFile(d, f.file_id, { builtin: false }) }, icon('play'), playerName()),
      check));
  }
  return box;
}

function fixMatch(d) {
  const input = h('input', { class: 'input', value: d.name, placeholder: 'Title, TMDB id or themoviedb.org link' });
  const year = h('input', { class: 'input', value: d.year || '', placeholder: 'Year', style: 'width:90px' });
  const msg = h('div', { class: 'result-line' });
  const go_ = h('button', {
    class: 'btn primary', text: 'Search', async onclick() {
      go_.disabled = true;
      msg.className = 'result-line'; msg.textContent = 'Searching…';
      try {
        const r = await POST(`/api/titles/${d.id}/match`, { query: input.value, year: Number(year.value) || null });
        close();
        toast(`Matched: ${r.title}`);
        refreshDetail(d.id);
      } catch (e) {
        msg.className = 'result-line err'; msg.textContent = e.message;
        go_.disabled = false;
      }
    },
  });
  const close = modal(h('div', {},
    h('h2', { text: 'Fix the match' }),
    h('p', { text: 'Search TMDB again with a cleaner title, or paste the TMDB page link for this ' + (d.kind === 'show' ? 'show.' : 'movie.') }),
    h('div', { class: 'with-btn' }, input, year),
    msg,
    h('div', { class: 'actions' }, h('button', { class: 'btn', text: 'Cancel', onclick: () => close() }), go_)));
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') go_.click(); });
  input.focus();
  input.select();
}

function modal(content) {
  const root = $('#overlay-root');
  const back = h('div', { class: 'modal-back', onclick(e) { if (e.target === back) close(); } }, h('div', { class: 'modal' }, content));
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  function close() { back.remove(); document.removeEventListener('keydown', onKey); }
  document.addEventListener('keydown', onKey);
  root.append(back);
  return close;
}

function confirmBox(title, text, okLabel, danger = false) {
  return new Promise((resolve) => {
    const close = modal(h('div', {},
      h('h2', { text: title }), h('p', { text }),
      h('div', { class: 'actions' },
        h('button', { class: 'btn', text: 'Cancel', onclick() { close(); resolve(false); } }),
        h('button', { class: 'btn ' + (danger ? 'danger-solid' : 'primary'), text: okLabel, onclick() { close(); resolve(true); } }))));
  });
}

// ---------------------------------------------------------- settings
const TABS = [
  ['sources', 'database', 'Sources'],
  ['player', 'monitor', 'Player'],
  ['info', 'info', 'Movie info'],
  ['assistant', 'sparkles', 'AI assistant'],
  ['library', 'drive', 'Library & storage'],
];

async function settingsPage(tab) {
  if (!TABS.some((t) => t[0] === tab)) tab = 'sources';
  const wrap = h('div', { class: 'wrap' },
    h('h1', { class: 'page-title', text: 'Settings' }),
    h('div', { class: 'tabs' }, TABS.map(([id, ic, label]) =>
      h('a', { class: 'tab' + (id === tab ? ' on' : ''), href: '#/settings/' + id }, icon(ic), label))));
  page.append(wrap);
  const body = h('div');
  wrap.append(body);
  const settings = await GET('/api/settings');
  switch (tab) {
    case 'player': return playerTab(body, settings);
    case 'info': return infoTab(body, settings);
    case 'assistant': return assistantTab(body, settings);
    case 'library': return libraryTab(body, settings);
    default: return sourcesTab(body);
  }
}

function field(label, control, hint) {
  return h('div', { class: 'field' }, h('label', { text: label }), control, hint && h('div', { class: 'hint', text: hint }));
}

function seg(options, value, onchange) {
  const box = h('div', { class: 'seg' });
  function render() {
    box.replaceChildren(...options.map(([v, label, ic]) => h('button', {
      class: v === value ? 'on' : '', type: 'button', onclick() { value = v; render(); onchange(v); },
    }, ic && icon(ic), label)));
  }
  render();
  return box;
}

async function pickPath(kind) {
  try { return (await POST('/api/pick', { kind })).path; } catch (e) { toast(e.message, true); return null; }
}

function sourcesTab(body) {
  // ---- add a link
  let kind = 'movies';
  const loc = h('input', { class: 'input mono', placeholder: 'http://172.16.50.14/DHAKA-FLIX-14/English Movies (1080p)/', spellcheck: 'false' });
  const name = h('input', { class: 'input', placeholder: 'English Movies 1080p' });
  const kindHint = h('div', { class: 'hint', text: 'Folder → Movie folders' });
  const kindSeg = seg([['movies', 'Movies', 'film'], ['shows', 'TV shows', 'tv']], kind, (v) => {
    kind = v;
    kindHint.textContent = v === 'shows' ? 'Folder → Show folders → Seasons' : 'Folder → Movie folders';
  });
  const result = h('div', { class: 'result-line' });
  const browse = h('button', { class: 'btn', title: 'Choose a folder on this PC', async onclick() { const p = await pickPath('folder'); if (p) loc.value = p; } }, icon('folder'));
  const check = h('button', {
    class: 'btn', text: 'Check link', async onclick() {
      if (!loc.value.trim()) return loc.focus();
      check.disabled = true;
      result.className = 'result-line'; result.textContent = 'Checking…';
      try {
        const r = await POST('/api/sources/check', { location: loc.value });
        result.className = 'result-line ok';
        fill(result, `✓ Works: ${plural(r.folders, 'folder')}, ${plural(r.videos, 'video')} at the top level. Looks like ${r.kind === 'shows' ? 'TV shows' : 'movies'}.`,
          r.sample.length ? h('span', { class: 'sample', text: 'e.g. ' + r.sample.join(' · ') }) : null);
        if (r.kind !== kind) kindSeg.querySelectorAll('button')[r.kind === 'shows' ? 1 : 0].click();
        if (!name.value.trim()) name.value = r.name;
      } catch (e) {
        result.className = 'result-line err'; result.textContent = e.message;
      }
      check.disabled = false;
    },
  });
  const add = h('button', {
    class: 'btn primary', async onclick() {
      if (!loc.value.trim()) return loc.focus();
      add.disabled = true;
      try {
        const r = await POST('/api/sources', { location: loc.value, name: name.value, kind, sync: true });
        result.className = 'result-line ok'; result.textContent = `Added “${r.name}”. Syncing now — titles appear as soon as it finishes.`;
        loc.value = ''; name.value = '';
        await refreshState();
      } catch (e) {
        result.className = 'result-line err'; result.textContent = e.message;
      }
      add.disabled = false;
    },
  }, icon('plus'), 'Add and sync');
  [loc, name].forEach((el) => el.addEventListener('keydown', (e) => { if (e.key === 'Enter') add.click(); }));

  const addPanel = h('div', { class: 'panel' },
    h('h2', { text: 'Add a link' }),
    h('p', { class: 'lead', text: 'A movie folder (category, year or collection) or a TV series folder such as KOREAN TV & WEB Series. A folder on this PC works too.' }),
    h('div', { class: 'form-row' },
      h('div', { class: 'field', style: 'flex:1 1 280px' }, h('label', { text: 'Folder link' }), h('div', { class: 'with-btn' }, loc, browse), h('div', { class: 'hint', text: 'A category or year folder. Everything inside it is scanned.' })),
      h('div', { class: 'field' }, h('label', { text: 'Type' }), kindSeg, kindHint),
      h('div', { class: 'field', style: 'flex:0 1 180px' }, h('label', { text: 'Name' }), name, h('div', { class: 'hint', text: 'Shown in the sidebar.' })),
      h('div', { class: 'field', style: 'padding-top:22px' }, h('div', { class: 'actions', style: 'flex-wrap:nowrap' }, check, add))),
    result);

  // ---- your sources
  const list = h('div', { class: 'sources-list' });
  const countLine = h('div', { class: 'subtitle' });
  const syncAll = h('button', { class: 'btn' });
  const full = h('button', {
    class: 'btn', text: 'Full rescan', async onclick() {
      if (!await confirmBox('Full rescan', 'Re-walk every source from scratch. Files that are gone from the server are removed from the library. This can take a while for large sources.', 'Rescan everything')) return;
      try { await POST('/api/sync', { full: true }); refreshState(); } catch (e) { toast(e.message, true); }
    },
  });
  const listPanel = h('div', { class: 'panel' },
    h('div', { class: 'panel-head' }, h('div', {}, h('h2', { text: 'Your sources' }), countLine), h('div', { class: 'actions' }, full, syncAll)),
    list);

  const logBox = h('pre', { class: 'log' });
  const logPanel = h('div', { class: 'panel' }, h('h2', { text: 'Activity' }), h('p', { class: 'lead', text: 'What the last syncs and lookups did.' }), logBox);

  body.append(addPanel, listPanel, logPanel);

  function render(state) {
    const sources = state.sources;
    const busy = state.job.running || state.job.queued_roots.length > 0;
    countLine.textContent = plural(sources.length, 'link');
    syncAll.className = 'btn' + (busy ? ' busy' : ' primary');
    syncAll.replaceChildren(busy ? h('span', { class: 'spinner' }) : icon('sync'), busy ? 'Syncing…' : 'Sync all');
    syncAll.onclick = async () => {
      if (busy) { await POST('/api/jobs/stop'); toast('Stopping…'); }
      else { try { await POST('/api/sync'); } catch (e) { toast(e.message, true); } }
      refreshState();
    };
    full.disabled = !sources.length;
    logBox.textContent = state.job.log.length ? state.job.log.join('\n') : 'Nothing yet.';
    logBox.scrollTop = logBox.scrollHeight;
    if (app.editing) return;
    if (!sources.length) {
      list.replaceChildren(h('div', { class: 'subtitle', style: 'padding:14px 0', text: 'No sources yet. Add a link above.' }));
      return;
    }
    list.replaceChildren(...sources.map((s) => sourceRow(s, state)));
  }

  function sourceRow(s, state) {
    const noun = s.kind === 'shows' ? 'show' : 'movie';
    const dotClass = s.syncing ? 'syncing' : s.status === 'ok' ? 'ok' : s.status === 'error' ? 'error' : '';
    let when = h('div', { class: 'when', text: ago(s.last_sync) });
    if (s.syncing) when = h('div', { class: 'when busy', text: `Syncing${state.job.detail ? ' · ' + state.job.detail : '…'}` });
    else if (s.queued) when = h('div', { class: 'when busy', text: 'Queued' });
    else if (s.status === 'error') when = h('div', { class: 'when err', text: `${ago(s.last_sync)} · ${s.error || 'error'}` });
    else if (s.status === 'stopped') when = h('div', { class: 'when', text: `${ago(s.last_sync)} · stopped before finishing` });
    const enc = encodeURIComponent(s.name);
    const row = h('div', { class: 'source-row' },
      h('div', { class: 'body' },
        h('div', { class: 'name-line' },
          h('span', { class: 'dot ' + dotClass }),
          h('a', { class: 'name', href: '#/source/' + enc, text: s.name }),
          h('span', { class: 'tag', text: s.kind === 'shows' ? 'TV shows' : 'Movies' }),
          h('span', { class: 'tag', text: plural(s.count, noun) })),
        h('div', { class: 'loc', text: s.location, title: s.location }),
        when),
      h('div', { class: 'row-actions' },
        h('button', { class: 'btn small', disabled: s.syncing || s.queued, async onclick() { await POST(`/api/sources/${enc}/sync`).catch((e) => toast(e.message, true)); refreshState(); } }, icon('sync'), 'Sync'),
        h('button', { class: 'icon-btn', title: 'Stop', disabled: !(s.syncing || s.queued), async onclick() { await POST(`/api/sources/${enc}/stop`); refreshState(); } }, icon('stop')),
        h('button', { class: 'icon-btn', title: 'Edit', onclick() { editRow(row, s); } }, icon('pencil')),
        h('button', {
          class: 'icon-btn danger', title: 'Delete', async onclick() {
            if (!await confirmBox(`Delete “${s.name}”?`, `Its ${plural(s.count, noun)} leave your library (watchlist and history for them too). Nothing on the server or disk is touched.`, 'Delete', true)) return;
            try { await DEL(`/api/sources/${enc}`); toast(`Deleted “${s.name}”`); } catch (e) { toast(e.message, true); }
            refreshState();
          },
        }, icon('trash'))));
    return row;
  }

  function editRow(row, s) {
    app.editing = s.name;
    let kind_ = s.kind;
    const n = h('input', { class: 'input', value: s.name });
    const l = h('input', { class: 'input mono', value: s.location });
    const msg = h('div', { class: 'result-line' });
    const box = h('div', { class: 'edit-box' },
      h('div', { class: 'form-row' },
        h('div', { class: 'field', style: 'flex:0 1 220px' }, h('label', { text: 'Name' }), n),
        h('div', { class: 'field', style: 'flex:1 1 320px' }, h('label', { text: 'Folder link' }), l),
        h('div', { class: 'field' }, h('label', { text: 'Type' }), seg([['movies', 'Movies', 'film'], ['shows', 'TV shows', 'tv']], kind_, (v) => { kind_ = v; }))),
      msg,
      h('div', { class: 'actions', style: 'margin-top:12px' },
        h('button', {
          class: 'btn primary', text: 'Save', async onclick() {
            try {
              await PUT(`/api/sources/${encodeURIComponent(s.name)}`, { name: n.value, location: l.value, kind: kind_ });
              app.editing = null;
              toast('Saved');
              await refreshState();
              render(app.state);
            } catch (e) { msg.className = 'result-line err'; msg.textContent = e.message; }
          },
        }),
        h('button', { class: 'btn', text: 'Cancel', onclick() { app.editing = null; render(app.state); } })));
    row.replaceWith(h('div', { class: 'source-row', style: 'display:block' }, box));
  }

  render(app.state);
  return { onState: render, cleanup() { app.editing = null; } };
}

function playerTab(body, s) {
  const p = s.player;
  const cmd = h('input', { class: 'input mono', value: p.command });
  const args = h('input', { class: 'input mono', value: p.args.join(' ') });
  const builtin = h('input', { type: 'checkbox' });
  builtin.checked = p.prefer_builtin;
  const msg = h('div', { class: 'result-line' });
  body.append(h('div', { class: 'panel' },
    h('h2', { text: 'Video player' }),
    h('p', { class: 'lead', text: 'Play buttons hand the video to this program. VLC plays everything, including MKV and HEVC/x265, which browsers can’t.' }),
    h('div', { class: 'form-row' },
      h('div', { class: 'field', style: 'flex:1 1 420px' }, h('label', { text: 'Program' }),
        h('div', { class: 'with-btn' }, cmd, h('button', { class: 'btn', async onclick() { const x = await pickPath('program'); if (x) cmd.value = x; } }, icon('folder'), 'Browse…')),
        h('div', { class: 'hint', text: 'Full path to vlc.exe, mpv.exe, or another player.' })),
      h('div', { class: 'field', style: 'flex:1 1 260px' }, h('label', { text: 'Arguments' }), args, h('div', { class: 'hint', text: '{url} becomes the video link or file path.' }))),
    h('label', { class: 'check-row', style: 'margin-top:16px' }, builtin, 'Play MP4 and WebM files inside the app instead (MKV always opens in the player)'),
    h('div', { class: 'actions', style: 'margin-top:18px' },
      h('button', {
        class: 'btn primary', text: 'Save', async onclick() {
          try {
            await PUT('/api/settings/player', { command: cmd.value, args: args.value.split(/\s+/).filter(Boolean), prefer_builtin: builtin.checked });
            msg.className = 'result-line ok'; msg.textContent = 'Saved.';
            refreshState();
          } catch (e) { msg.className = 'result-line err'; msg.textContent = e.message; }
        },
      }),
      h('button', { class: 'btn ghost', onclick: () => POST('/api/open-url', { url: 'https://www.videolan.org/vlc/' }).catch((e) => toast(e.message, true)) }, icon('external'), 'Get VLC')),
    msg));
}

async function infoTab(body, s) {
  const t = s.tmdb;
  const key = h('input', { class: 'input mono', type: 'password', value: t.api_key, placeholder: 'TMDB API key or read access token' });
  const show = h('button', { class: 'btn', title: 'Show', onclick() { key.type = key.type === 'password' ? 'text' : 'password'; } }, icon('eye'));
  const lang = selectBox([['en-US', 'English'], ['bn-BD', 'Bangla'], ['hi-IN', 'Hindi'], ['ko-KR', 'Korean'], ['ja-JP', 'Japanese'], ['ta-IN', 'Tamil'], ['te-IN', 'Telugu'], ['es-ES', 'Spanish'], ['fr-FR', 'French'], ['de-DE', 'German'], ['zh-CN', 'Chinese'], ['ar-SA', 'Arabic']], t.language, () => {});
  const auto = h('input', { type: 'checkbox' });
  auto.checked = t.auto_lookup;
  const msg = h('div', { class: 'result-line' });
  const stats = h('div', { class: 'stats' });
  body.append(
    h('div', { class: 'panel' },
      h('h2', { text: 'Movie info from TMDB' }),
      h('p', { class: 'lead', text: 'Posters, backdrops, genres, ratings, plots and episode names come from The Movie Database. It needs a free API key.' }),
      h('div', { class: 'form-row' },
        h('div', { class: 'field', style: 'flex:1 1 420px' }, h('label', { text: 'API key' }), h('div', { class: 'with-btn' }, key, show),
          h('div', { class: 'hint' }, 'Either the “API Key” or the “API Read Access Token” from ',
            h('button', { class: 'link', text: 'themoviedb.org/settings/api', onclick: () => POST('/api/open-url', { url: 'https://www.themoviedb.org/settings/api' }).catch((e) => toast(e.message, true)) }), ' works.')),
        h('div', { class: 'field' }, h('label', { text: 'Language' }), lang)),
      h('label', { class: 'check-row', style: 'margin-top:16px' }, auto, 'Look up new titles automatically after each sync'),
      h('div', { class: 'actions', style: 'margin-top:18px' },
        h('button', {
          class: 'btn', text: 'Test key', async onclick() {
            msg.className = 'result-line'; msg.textContent = 'Testing…';
            try { await POST('/api/tmdb/test', { api_key: key.value }); msg.className = 'result-line ok'; msg.textContent = '✓ The key works.'; }
            catch (e) { msg.className = 'result-line err'; msg.textContent = e.message; }
          },
        }),
        h('button', {
          class: 'btn primary', text: 'Save', async onclick() {
            try {
              await PUT('/api/settings/tmdb', { api_key: key.value.trim(), language: lang.firstChild.value, auto_lookup: auto.checked });
              msg.className = 'result-line ok'; msg.textContent = 'Saved.' + (key.value.trim() && auto.checked ? ' Looking up titles in the background.' : '');
              refreshState(); loadStats();
            } catch (e) { msg.className = 'result-line err'; msg.textContent = e.message; }
          },
        })),
      msg),
    h('div', { class: 'panel' },
      h('h2', { text: 'Lookup status' }),
      stats,
      h('div', { class: 'actions', style: 'margin-top:16px' },
        h('button', { class: 'btn', async onclick() { try { await POST('/api/tmdb/lookup', { retry: false }); toast('Looking up missing info…'); refreshState(); } catch (e) { toast(e.message, true); } } }, icon('search'), 'Look up missing now'),
        h('button', { class: 'btn', async onclick() { try { await POST('/api/tmdb/lookup', { retry: true }); toast('Retrying titles without a match…'); refreshState(); } catch (e) { toast(e.message, true); } } }, icon('sync'), 'Retry not found'))));
  async function loadStats() {
    const st = await GET('/api/storage');
    const x = st.stats;
    stats.replaceChildren(
      h('div', { class: 'stat' }, h('b', { text: num(x.matched) }), h('span', { text: 'matched' })),
      h('div', { class: 'stat' }, h('b', { text: num(x.unmatched) }), h('span', { text: 'not found' })),
      h('div', { class: 'stat' }, h('b', { text: num(x.pending) }), h('span', { text: 'waiting' })),
      h('div', { class: 'stat' }, h('b', { text: num(x.posters) }), h('span', { text: 'with posters' })));
  }
  await loadStats();
  return { onState: (st) => { if (st.job.running && st.job.total) loadStats(); } };
}

function assistantTab(body, s) {
  const a = s.assistant;
  const key = h('input', { class: 'input mono', type: 'password', value: a.api_key, placeholder: 'Gemini API key' });
  const show = h('button', { class: 'btn', title: 'Show', onclick() { key.type = key.type === 'password' ? 'text' : 'password'; } }, icon('eye'));
  const model = h('input', { class: 'input mono', value: a.model, list: 'model-list' });
  const datalist = h('datalist', { id: 'model-list' });
  const msg = h('div', { class: 'result-line' });
  body.append(h('div', { class: 'panel' },
    h('h2', { text: 'Watch assistant' }),
    h('p', { class: 'lead', text: 'Ask for “a slow-burn Korean thriller” or “something like Interstellar”, in any language. It uses Google Gemini and only ever recommends titles that are in your library.' }),
    h('div', { class: 'form-row' },
      h('div', { class: 'field', style: 'flex:1 1 380px' }, h('label', { text: 'Gemini API key' }), h('div', { class: 'with-btn' }, key, show),
        h('div', { class: 'hint' }, 'Free from ', h('button', { class: 'link', text: 'aistudio.google.com/apikey', onclick: () => POST('/api/open-url', { url: 'https://aistudio.google.com/apikey' }).catch((e) => toast(e.message, true)) }))),
      h('div', { class: 'field', style: 'flex:1 1 260px' }, h('label', { text: 'Model' }),
        h('div', { class: 'with-btn' }, model, h('button', {
          class: 'btn', text: 'Load list', async onclick() {
            msg.className = 'result-line'; msg.textContent = 'Loading models…';
            try {
              const r = await GET('/api/assistant/models?' + qs({ key: key.value.trim() }));
              datalist.replaceChildren(...r.models.map((m) => h('option', { value: m })));
              msg.className = 'result-line ok'; msg.textContent = `✓ Key works. ${r.models.length} models available — pick one in the Model box.`;
            } catch (e) { msg.className = 'result-line err'; msg.textContent = e.message; }
          },
        })), datalist)),
    h('div', { class: 'actions', style: 'margin-top:18px' },
      h('button', {
        class: 'btn primary', text: 'Save', async onclick() {
          try {
            await PUT('/api/settings/assistant', { api_key: key.value.trim(), model: model.value.trim() });
            msg.className = 'result-line ok'; msg.textContent = 'Saved.';
            refreshState();
          } catch (e) { msg.className = 'result-line err'; msg.textContent = e.message; }
        },
      }),
      h('button', {
        class: 'btn danger', async onclick() {
          if (!await confirmBox('Clear chat history?', 'The assistant forgets the conversation so far.', 'Clear', true)) return;
          await DEL('/api/assistant'); assistant.messages = []; renderAssistant(); toast('Chat cleared');
        },
      }, icon('trash'), 'Clear chat history')),
    msg));
}

async function libraryTab(body, s) {
  const lib = s.library;
  const skip = h('input', { class: 'input', value: lib.skip_dirs.join(', ') });
  const ext = h('input', { class: 'input', value: lib.media_extensions.join(', ') });
  const conc = h('input', { class: 'input', type: 'number', min: 1, max: 64, value: lib.concurrency });
  const delay = h('input', { class: 'input', type: 'number', min: 0, value: lib.delay_ms });
  const timeout = h('input', { class: 'input', type: 'number', min: 1, value: lib.timeout_secs });
  const depth = h('input', { class: 'input', type: 'number', min: 1, value: lib.max_depth });
  const msg = h('div', { class: 'result-line' });
  const stats = h('div', { class: 'stats' });
  const kv = h('div', { class: 'kv' });
  const list = (v) => v.split(',').map((x) => x.trim()).filter(Boolean);
  const n = (el) => Math.max(0, parseInt(el.value, 10) || 0);

  async function action(name, title, text, ok, danger) {
    if (!await confirmBox(title, text, ok, danger)) return;
    try { await POST('/api/storage/' + name); toast('Done'); refreshState(); load(); app.gridMemo = {}; }
    catch (e) { toast(e.message, true); }
  }

  body.append(
    h('div', { class: 'panel' }, h('h2', { text: 'Library' }), stats, kv,
      h('div', { class: 'actions', style: 'margin-top:16px' },
        h('button', { class: 'btn', onclick: () => POST('/api/storage/open').catch((e) => toast(e.message, true)) }, icon('folder'), 'Open data folder'))),
    h('div', { class: 'panel' },
      h('h2', { text: 'Scanning' }),
      h('p', { class: 'lead', text: 'Applies to the next sync.' }),
      h('div', { class: 'form-row' },
        h('div', { class: 'field', style: 'flex:1 1 300px' }, h('label', { text: 'Skip folders whose name contains' }), skip, h('div', { class: 'hint', text: 'Comma-separated, not case-sensitive.' })),
        h('div', { class: 'field', style: 'flex:1 1 300px' }, h('label', { text: 'Video file types' }), ext)),
      h('div', { class: 'form-row', style: 'margin-top:14px' },
        field('Parallel requests', conc), field('Pause between levels (ms)', delay), field('Timeout (s)', timeout), field('Max folder depth', depth)),
      h('div', { class: 'actions', style: 'margin-top:18px' },
        h('button', {
          class: 'btn primary', text: 'Save', async onclick() {
            try {
              await PUT('/api/settings/library', { skip_dirs: list(skip.value), media_extensions: list(ext.value), concurrency: n(conc), delay_ms: n(delay), timeout_secs: n(timeout), max_depth: n(depth) });
              msg.className = 'result-line ok'; msg.textContent = 'Saved.';
            } catch (e) { msg.className = 'result-line err'; msg.textContent = e.message; }
          },
        })),
      msg),
    h('div', { class: 'panel' },
      h('h2', { text: 'Maintenance' }),
      h('p', { class: 'lead', text: 'None of these touch your media files.' }),
      h('div', { class: 'actions' },
        h('button', { class: 'btn', onclick: () => action('rebuild', 'Rebuild library', 'Regroup all files into movies and shows again. Watchlist and info are kept.', 'Rebuild') }, icon('sync'), 'Rebuild library'),
        h('button', { class: 'btn', onclick: () => action('clear-history', 'Clear watch history', 'Forget what you played and every watched mark.', 'Clear', true) }, icon('history'), 'Clear watch history'),
        h('button', { class: 'btn', onclick: () => action('clear-info', 'Clear movie info', 'Forget all TMDB info and look everything up again.', 'Clear', true) }, icon('info'), 'Clear movie info'),
        h('button', { class: 'btn danger-solid', onclick: () => action('reset', 'Reset library', 'Empty the whole library: titles, history, watchlist and chat. Your sources stay; sync them to rebuild.', 'Reset everything', true) }, icon('trash'), 'Reset library'))));

  async function load() {
    const st = await GET('/api/storage');
    const x = st.stats;
    stats.replaceChildren(
      ...[[x.movies, 'movies'], [x.shows, 'shows'], [x.files, 'video files'], [x.posters, 'with posters'], [x.watchlist, 'in watchlist'], [x.plays, 'played']]
        .map(([v, l]) => h('div', { class: 'stat' }, h('b', { text: num(v) }), h('span', { text: l }))),
      h('div', { class: 'stat' }, h('b', { text: size(x.bytes) || '—' }), h('span', { text: 'of video (where listed)' })),
      h('div', { class: 'stat' }, h('b', { text: size(st.db_bytes) }), h('span', { text: 'database' })));
    kv.replaceChildren(
      h('span', { text: 'Data folder' }), h('code', { text: st.data_dir }),
      h('span', { text: 'Settings file' }), h('code', { text: st.config_path }),
      h('span', { text: 'Database' }), h('code', { text: st.db_path }));
  }
  await load();
}

// --------------------------------------------------------- assistant
const assistant = { open: false, messages: null, busy: false };
const SUGGESTIONS = [
  'কোরিয়ান রোমান্টিক কমেডি ড্রামা সাজেস্ট করো',
  'Hindi dubbed action movies from the last few years',
  'Something like Interstellar',
  'আজ রাতে পরিবারের সাথে দেখার মতো মজার মুভি',
];

$('#fab').append(icon('sparkles'));
$('#fab').onclick = () => toggleAssistant();

async function toggleAssistant(force) {
  assistant.open = force ?? !assistant.open;
  $('#assistant').hidden = !assistant.open;
  const fab = $('#fab');
  fab.classList.toggle('open', assistant.open);
  fab.replaceChildren(icon(assistant.open ? 'x' : 'sparkles'));
  if (!assistant.open) return;
  if (assistant.messages === null) {
    assistant.messages = [];
    renderAssistant();
    try { assistant.messages = (await GET('/api/assistant')).messages; } catch { /* empty chat */ }
  }
  renderAssistant();
  $('#assistant input')?.focus();
}

function renderAssistant() {
  const box = $('#assistant');
  const flags = app.state?.flags || {};
  const counts = app.state?.counts || {};
  const bodyEl = h('div', { class: 'as-body' });
  const input = h('input', { placeholder: 'e.g. a slow-burn Korean thriller…', disabled: assistant.busy || !flags.assistant });
  const send = h('button', { class: 'send', title: 'Send', disabled: assistant.busy || !flags.assistant, onclick: () => ask(input.value) }, icon('send'));
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') ask(input.value); });

  if (!flags.assistant) {
    bodyEl.append(h('div', { class: 'as-intro' },
      h('h3', { text: 'Set up the assistant' }),
      h('p', { text: 'Add a free Google Gemini API key and the assistant will suggest titles from your own library.' }),
      h('a', { class: 'btn primary', href: '#/settings/assistant', onclick: () => toggleAssistant(false) }, icon('sliders'), 'Open AI assistant settings')));
  } else if (!assistant.messages?.length) {
    bodyEl.append(h('div', { class: 'as-intro' },
      h('h3', { text: 'What do you feel like watching?' }),
      h('p', { text: `Describe a mood, genre, language or a title you liked. I'll pick from your ${num(counts.movies)} movies and ${num(counts.shows)} shows.` }),
      SUGGESTIONS.map((s) => h('button', { class: 'suggestion', text: s, onclick: () => ask(s) }))));
  } else {
    for (const m of assistant.messages) {
      if (m.role === 'user') bodyEl.append(h('div', { class: 'msg-user', text: m.text }));
      else if (m.role === 'error') bodyEl.append(h('div', { class: 'msg-err', text: m.text }));
      else {
        bodyEl.append(h('div', { class: 'msg-bot' }, m.text,
          m.picks?.length ? h('div', { class: 'picks' }, m.picks.map((c) => h('div', { class: 'pick', onclick: () => go('#/title/' + c.id) },
            c.poster ? h('img', { src: c.poster, alt: '', loading: 'lazy' }) : h('div', { class: 'ph' }),
            h('div', {}, h('b', { text: c.name }), h('span', { text: [years(c), c.kind === 'show' ? 'TV show' : 'Movie', c.rating && `★ ${c.rating.toFixed(1)}`].filter(Boolean).join(' · ') }))))) : null));
      }
    }
    if (assistant.busy) bodyEl.append(h('div', { class: 'typing' }, h('i'), h('i'), h('i')));
  }

  box.replaceChildren(
    h('div', { class: 'as-head' },
      h('div', { class: 'as-icon' }, icon('sparkles')),
      h('div', { class: 'grow' }, h('b', { text: 'Watch assistant' }), h('small', { text: `Finds picks in your library · ${flags.model || 'Gemini'} · chat is saved` })),
      h('button', { class: 'icon-btn', title: 'Close', onclick: () => toggleAssistant(false) }, icon('x'))),
    bodyEl,
    h('div', { class: 'as-foot' }, input, send));
  bodyEl.scrollTop = bodyEl.scrollHeight;
  if (!assistant.busy && flags.assistant) input.focus();
}

async function ask(text) {
  text = (text || '').trim();
  if (!text || assistant.busy) return;
  assistant.messages.push({ role: 'user', text });
  assistant.busy = true;
  renderAssistant();
  try {
    const r = await POST('/api/assistant', { message: text });
    assistant.messages.push({ role: 'assistant', text: r.reply, picks: r.picks });
  } catch (e) {
    assistant.messages.push({ role: 'error', text: e.message });
  }
  assistant.busy = false;
  renderAssistant();
}

// ------------------------------------------------------- search & keys
(function setupTopbar() {
  $('.search-icon').append(icon('search'));
  $('#top-settings').append(icon('sliders'));
  $('#job-stop').append(icon('stop'));
  $('#add-source').append(icon('plus'));
  $('#add-source').onclick = () => go('#/settings/sources');
  $('#job-stop').onclick = async () => { await POST('/api/jobs/stop').catch(() => {}); toast('Stopping…'); refreshState(); };

  const input = $('#search');
  let timer = 0;
  input.addEventListener('input', () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const q = input.value.trim();
      const target = '#/search/' + encodeURIComponent(q);
      if (!q) {
        if (location.hash.startsWith('#/search/')) history.back();
        return;
      }
      delete app.gridMemo[target];
      if (location.hash.startsWith('#/search/')) {
        history.replaceState(null, '', target);
        route();
      } else {
        location.hash = target;
      }
    }, 220);
  });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { input.value = ''; input.blur(); if (location.hash.startsWith('#/search/')) history.back(); }
  });
  document.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      input.focus();
      input.select();
    } else if (e.key === '/' && !/INPUT|TEXTAREA|SELECT/.test(document.activeElement.tagName)) {
      e.preventDefault();
      input.focus();
    }
  });
})();

window.addEventListener('hashchange', route);
refreshState().then(route);
