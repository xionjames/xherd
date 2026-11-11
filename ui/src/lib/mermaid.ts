// Diagrams in agent documents: a ```mermaid block (rendered by marked as
// <pre><code class="language-mermaid">) is drawn as SVG. The source is agent
// text, so untrusted (spec §13): Mermaid runs in its `strict` security level
// with plain SVG labels, its output is sanitised again with DOMPurify's SVG
// profile, and any CSS url() other than an in-document `#marker` reference is
// dropped. Mermaid is loaded only when a document has a diagram.
import DOMPurify from 'dompurify';

type Mermaid = typeof import('mermaid').default;
let loading: Promise<Mermaid> | null = null;
let seq = 0;

/** Colours and font from the app's tokens, so diagrams follow the theme. */
function themeVariables(): Record<string, string> {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    fontFamily: v('--sans'),
    fontSize: '14px',
    background: v('--pen'),
    primaryColor: v('--pen-2'),
    primaryTextColor: v('--ink'),
    primaryBorderColor: v('--fence-strong'),
    secondaryColor: v('--accent-wash'),
    tertiaryColor: v('--well'),
    lineColor: v('--ink-3'),
    textColor: v('--ink'),
    noteBkgColor: v('--accent-wash'),
    noteTextColor: v('--ink'),
    noteBorderColor: v('--fence-strong'),
    actorBkg: v('--pen-2'),
    actorBorder: v('--fence-strong'),
    actorTextColor: v('--ink'),
    signalColor: v('--ink-2'),
    signalTextColor: v('--ink'),
    labelBoxBkgColor: v('--pen-2'),
    labelTextColor: v('--ink'),
  };
}

async function load(): Promise<Mermaid> {
  loading ??= import('mermaid').then((m) => m.default);
  const mermaid = await loading;
  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', htmlLabels: false, theme: 'base', themeVariables: themeVariables() });
  return mermaid;
}

function clean(svg: string): string {
  const safe = DOMPurify.sanitize(svg, { USE_PROFILES: { svg: true, svgFilters: true }, ADD_TAGS: ['style'] });
  return safe.replace(/url\((?!\s*['"]?#)[^)]*\)/gi, 'none');
}

/**
 * Draws every Mermaid block inside `root`, inside its own `<pre>` (the node
 * the page rendered, so it goes away with the document). A drawn diagram
 * keeps its source in `data-src` and is drawn again (for a theme change).
 */
export async function drawDiagrams(root: HTMLElement): Promise<void> {
  for (const code of root.querySelectorAll<HTMLElement>('pre > code.language-mermaid')) {
    const pre = code.parentElement!;
    pre.classList.add('mmd');
    pre.dataset.src = code.textContent ?? '';
  }
  const figs = [...root.querySelectorAll<HTMLElement>('pre.mmd')];
  if (!figs.length) return;
  const mermaid = await load();
  for (const fig of figs) {
    const src = fig.dataset.src ?? '';
    try {
      const { svg } = await mermaid.render(`mmd-${++seq}`, src);
      fig.innerHTML = clean(svg);
    } catch {
      fig.classList.add('failed');
      fig.replaceChildren(src, Object.assign(document.createElement('span'), { className: 'hint', textContent: 'This diagram could not be drawn.' }));
    }
  }
}
