// Herd view "Tiles": one tile per agent, grouped by repository, finished
// agents left out. A small example of a view plugin, plain JavaScript with no
// build step.
//
// The kit script (loaded before this file) defines the global `xherdView`:
//   xherdView.onData(cb)  cb({agents, flows, theme}) on every change
//   xherdView.open(id)    open that agent's page in xherd
//
// Agent fields come from the Herd page (names, branches, tickets) and are
// drawn with `textContent` only, never as HTML.
(function () {
  'use strict';

  var root = document.getElementById('tiles');

  // Status words, as the xherd UI shows them.
  var STATUS_LABEL = {
    queued: 'queued', running: 'running', 'waiting-input': 'your answer',
    blocked: 'blocked', idle: 'idle', parked: 'parked', paused: 'paused',
    watching: 'watching PR', done: 'done', cancelled: 'cancelled', error: 'error'
  };

  // Not working and not waiting on you: drawn as a hollow dot.
  var HOLLOW = ['queued', 'paused', 'watching', 'parked', 'cancelled'];

  // "now", "5m", "3h", "2d".
  function ago(ms) {
    var s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 60) return 'now';
    var m = Math.floor(s / 60);
    if (m < 60) return m + 'm';
    var h = Math.floor(m / 60);
    if (h < 48) return h + 'h';
    return Math.floor(h / 24) + 'd';
  }

  // Shorthand: an element with a class and, optionally, its text.
  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  // The agent's colour dot (filled, or hollow when it is not working).
  function dot(a) {
    var d = el('span', 'dot');
    if (HOLLOW.indexOf(a.status) >= 0) d.style.border = '1.5px solid ' + a.hue;
    else d.style.background = a.hue;
    return d;
  }

  // The status pill. "Needs you" is ember (--gate) and always has a glyph
  // and words, never colour alone.
  function pill(a) {
    if (a.needsYou) {
      var wait = a.status === 'blocked' ? 'blocked'
        : a.gate && a.gate !== 'question' ? 'needs you' : 'your answer';
      return el('span', 'pill gate', '✋ ' + wait);
    }
    var label = STATUS_LABEL[a.status] || a.status;
    if (a.status === 'running') {
      var p = el('span', 'pill live');
      var d = el('span', 'dot');
      d.style.cssText = 'background:var(--live);width:6px;height:6px';
      p.appendChild(d);
      p.appendChild(document.createTextNode(label));
      return p;
    }
    if (a.status === 'error') return el('span', 'pill halt', '✕ ' + label);
    return el('span', a.status === 'watching' ? 'pill done' : 'pill', label);
  }

  function tile(a) {
    var t = el('button', 'card tile' + (a.needsYou ? ' need' : ''));
    t.type = 'button';
    t.addEventListener('click', function () { xherdView.open(a.id); });
    var head = el('div', 'row');
    head.appendChild(dot(a));
    head.appendChild(el('span', 'mono name', a.name));
    t.appendChild(head);
    t.appendChild(pill(a));
    var stage = a.stageName || a.stage || 'no stage';
    t.appendChild(el('div', 'hint', stage + ' · ' + ago(a.lastActivity)));
    return t;
  }

  function draw(data) {
    // Group the active agents by repository, repositories in name order.
    var groups = {};
    data.agents.forEach(function (a) {
      if (a.status === 'done' || a.status === 'cancelled') return;
      (groups[a.repo] = groups[a.repo] || []).push(a);
    });
    var repos = Object.keys(groups).sort(function (x, y) { return x.localeCompare(y); });

    root.replaceChildren();
    if (!repos.length) {
      root.appendChild(el('p', 'empty', 'No active agents.'));
      return;
    }
    repos.forEach(function (repo) {
      var list = groups[repo];
      var head = el('div', 'sect');
      head.appendChild(el('h2', '', repo));
      head.appendChild(el('span', 'hint', String(list.length)));
      root.appendChild(head);
      var grid = el('div', 'tiles');
      list.forEach(function (a) { grid.appendChild(tile(a)); });
      root.appendChild(grid);
    });
  }

  xherdView.onData(draw);
})();
