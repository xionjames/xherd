// xherd view kit, protocol v1 (served at /api/v1/view-kit/xherd-view.js).
//
// A Herd view plugin loads this classic script before its own. It defines the
// global `xherdView`:
//
//   xherdView.onData(cb)   cb({agents, savedAgents, flows, theme}) for every `data` message
//                          from the Herd page; a callback added late gets the
//                          last one at once.
//   Saved agents carry `name` (the full name, what openAgent takes) and `label`
//   (the name without a flow plugin's prefix: show this one). Jobs carry
//   `agent` (full name) and, when they have one, `agentLabel` (to show).
//   xherdView.open(id)     ask the Herd page to open that job's page (`agents` holds jobs).
//   xherdView.openAgent(name)  ask it to open that saved agent's page.
//
// Before each callback it sets `data-theme` on <html> to the page's visible
// theme, so the kit stylesheet (xherd.css) follows it. It accepts only
// messages from the parent window that carry `xherdView: 1`; everything else
// is ignored. The view's origin is opaque (sandboxed), so messages go to '*':
// the parent is the only window it talks to.
(function () {
  'use strict';

  var VERSION = 1;
  var callbacks = [];
  var last = null;

  function post(msg) {
    msg.xherdView = VERSION;
    window.parent.postMessage(msg, '*');
  }

  window.addEventListener('message', function (e) {
    if (e.source !== window.parent) return;
    var m = e.data;
    if (!m || typeof m !== 'object' || m.xherdView !== VERSION) return;
    if (m.kind !== 'data') return;
    last = {
      agents: Array.isArray(m.agents) ? m.agents : [],
      savedAgents: Array.isArray(m.savedAgents) ? m.savedAgents : [],
      flows: Array.isArray(m.flows) ? m.flows : [],
      theme: m.theme === 'dark' ? 'dark' : 'light'
    };
    document.documentElement.setAttribute('data-theme', last.theme);
    for (var i = 0; i < callbacks.length; i++) callbacks[i](last);
  });

  window.xherdView = Object.freeze({
    version: VERSION,
    onData: function (cb) {
      if (typeof cb !== 'function') return;
      callbacks.push(cb);
      if (last) cb(last);
    },
    open: function (agentId) {
      if (typeof agentId !== 'string' || !agentId) return;
      post({ kind: 'open', agentId: agentId });
    },
    openAgent: function (name) {
      if (typeof name !== 'string' || !name) return;
      post({ kind: 'open-agent', name: name });
    }
  });

  post({ kind: 'ready' });
})();
