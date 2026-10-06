import { useEffect, useMemo, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { api } from "@/lib/api";
import { useUiStore } from "@/store/uiStore";

/** The email body's base look, in the colours of the current app theme. */
function baseStyle(): string {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  const scheme = document.documentElement.style.colorScheme || "dark";
  return `
  :root { color-scheme: ${scheme}; }
  html, body { margin: 0; padding: 0; background: transparent; }
  /* flow-root keeps the first/last child's margins inside, so the measured height is exact. */
  body { display: flow-root; }
  body { color: ${v("--text-main", "#e4e4e7")}; font: 14px/1.6 Inter, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; word-wrap: break-word; overflow-wrap: anywhere; }
  a { color: ${v("--accent", "#388bfd")}; }
  img { max-width: 100%; height: auto; }
  table { max-width: 100%; }
  blockquote { margin: 8px 0; padding-left: 12px; border-left: 2px solid ${v("--border", "rgba(255,255,255,.14)")}; color: ${v("--text-muted", "#a1a1aa")}; }
  pre { white-space: pre-wrap; }
`;
}

/**
 * Renders untrusted email HTML. Defense in depth:
 *  1. sanitized in Rust (ammonia) and again here (DOMPurify); the mail's own
 *     <style> is kept and applies after the app's base look;
 *  2. sandboxed iframe without `allow-scripts`;
 *  3. CSP that forbids scripts, frames, forms and — unless allowed — remote
 *     images (tracking pixels).
 * `allow-same-origin` (without scripts) only lets the parent measure height
 * and intercept link clicks.
 */
export function HtmlViewer({ html, allowRemote }: { html: string; allowRemote: boolean }) {
  const ref = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(24);
  // Re-render the body when the theme changes (its colours come from it).
  const theme = useUiStore((s) => s.prefs.theme);

  const srcDoc = useMemo(() => {
    const clean = DOMPurify.sanitize(html, {
      // <style> stays (layout, hidden preheaders): no scripts run here and the
      // CSP below blocks any remote load it could trigger.
      FORBID_TAGS: ["script", "iframe", "form", "input", "button", "object", "embed", "link", "meta", "base", "title", "noscript"],
      FORBID_ATTR: ["srcset"],
      ALLOW_DATA_ATTR: false,
      // Without it DOMPurify drops a <style> that opens the fragment.
      FORCE_BODY: true,
    });
    const img = allowRemote ? "data: blob: https: http:" : "data: blob:";
    const csp = `default-src 'none'; img-src ${img}; style-src 'unsafe-inline'; font-src data:;`;
    return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><style>${baseStyle()}</style></head><body>${clean}</body></html>`;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [html, allowRemote, theme]);

  useEffect(() => {
    const frame = ref.current;
    if (!frame) return;
    let observer: ResizeObserver | undefined;
    const onLoad = () => {
      const doc = frame.contentDocument;
      if (!doc) return;
      // The body, not the document: the document is never shorter than the frame,
      // so measuring it would never let the frame shrink to short content.
      const measure = () => setHeight(Math.max(20, Math.ceil(doc.body.getBoundingClientRect().height)));
      measure();
      if (typeof ResizeObserver !== "undefined") {
        observer = new ResizeObserver(measure);
        observer.observe(doc.body);
      }
      doc.addEventListener("click", (e) => {
        const a = (e.target as HTMLElement).closest("a");
        if (!a) return;
        e.preventDefault();
        const href = a.getAttribute("href");
        if (href && /^(https?:|mailto:)/i.test(href)) void api.openExternal(href);
      });
    };
    frame.addEventListener("load", onLoad);
    return () => {
      frame.removeEventListener("load", onLoad);
      observer?.disconnect();
    };
  }, [srcDoc]);

  return (
    <iframe
      ref={ref}
      title="Email content"
      data-testid="email-body"
      sandbox="allow-same-origin allow-popups"
      srcDoc={srcDoc}
      style={{ height }}
      className="w-full border-0 bg-transparent"
    />
  );
}
