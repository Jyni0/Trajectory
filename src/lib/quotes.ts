/**
 * Splitting a reply from the earlier messages it quotes. In a conversation
 * view those are already shown above, so the quote is folded away ("•••").
 */

export interface Split {
  /** The new part of the message. */
  body: string;
  /** Whether a trailing quote was found (and cut from `body`). */
  quoted: boolean;
}

/** "On Tue, Oct 6, 2026, Support wrote:" and its usual translations. */
const ATTRIBUTION = /(wrote|написал[аи]?|пишет|писал[аи]?|schrieb|a écrit|escribió|ha scritto|escreveu|schreef|napisał[a]?)\s*:?\s*$/i;
const HEADER_BLOCK = /^(from|von|de|от|od)\s*:/i;

const MEANINGFUL = (n: Node) =>
  n.nodeType === Node.ELEMENT_NODE ? (n as Element).tagName !== "BR" && ((n as Element).textContent?.trim() || (n as Element).querySelector("img")) : !!n.textContent?.trim();

function lastMeaningful(el: Node): ChildNode | null {
  for (let n = el.lastChild; n; n = n.previousSibling) if (MEANINGFUL(n)) return n;
  return null;
}

function prevMeaningful(n: Node): ChildNode | null {
  for (let p = n.previousSibling; p; p = p.previousSibling) if (MEANINGFUL(p)) return p;
  return null;
}

/** Element right before `n` in reading order (climbing while `n` opens its parent). */
function before(n: Node, root: Node): ChildNode | null {
  for (let cur: Node | null = n; cur && cur !== root; cur = cur.parentNode) {
    const p = prevMeaningful(cur);
    if (p) return p;
  }
  return null;
}

/** Drops the <br>s and empty blocks a cut quote leaves at the end. */
function trimTrailingBlank(el: Node) {
  for (let n = el.lastChild; n; n = el.lastChild) {
    const blank =
      n.nodeType === Node.TEXT_NODE
        ? !n.textContent?.trim()
        : n.nodeType === Node.ELEMENT_NODE && ((n as Element).tagName === "BR" || (!(n as Element).textContent?.trim() && !(n as Element).querySelector("img")));
    if (!blank) {
      if (n.nodeType === Node.ELEMENT_NODE) trimTrailingBlank(n);
      return;
    }
    el.removeChild(n);
  }
}

function hasContent(root: Element): boolean {
  return !!root.textContent?.trim() || !!root.querySelector("img");
}

/**
 * Cuts the quote that ends an HTML reply: Gmail's `.gmail_quote`, Apple Mail's
 * `blockquote[type=cite]`, Outlook's `#divRplyFwdMsg` and, generally, a
 * trailing `<blockquote>` with the "… wrote:" line before it. A message that
 * would be left empty is returned whole.
 */
export function splitHtmlQuote(html: string): Split {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  const root = doc.body;

  // The first match in document order is the outermost one.
  let cut: Element | null = root.querySelector(".gmail_quote, #divRplyFwdMsg, #appendonsend, .yahoo_quoted, .moz-cite-prefix");
  if (cut && !cut.classList.contains("gmail_quote") && !cut.classList.contains("yahoo_quoted")) {
    // Outlook / Thunderbird: everything from the marker on is the old mail.
    const prev = cut.previousElementSibling;
    if (prev?.tagName === "HR") cut = prev;
  } else if (!cut) {
    // Apple Mail's <blockquote type=cite>, or any trailing <blockquote>.
    let node: ChildNode | null = root.querySelector("blockquote[type=cite]");
    if (!node) {
      node = lastMeaningful(root);
      while (node && node.nodeType === Node.ELEMENT_NODE && (node as Element).tagName !== "BLOCKQUOTE") node = lastMeaningful(node);
    }
    if (node && (node as Element).tagName === "BLOCKQUOTE") {
      cut = node as Element;
      const attribution = before(cut, root);
      const text = attribution?.textContent?.trim() ?? "";
      if (attribution && text.length < 400 && (ATTRIBUTION.test(text) || HEADER_BLOCK.test(text))) {
        // The attribution and the quote may sit in different parents: drop both.
        attribution.parentNode?.removeChild(attribution);
      }
    }
  }
  if (!cut) return { body: html, quoted: false };

  // Remove the quote and everything after it, at every level up to the body.
  for (let n: Node | null = cut; n && n !== root; n = n.parentNode) {
    while (n.nextSibling) n.parentNode!.removeChild(n.nextSibling);
  }
  cut.parentNode?.removeChild(cut);
  if (!hasContent(root)) return { body: html, quoted: false };
  trimTrailingBlank(root);
  return { body: root.innerHTML, quoted: true };
}

/** Plain-text counterpart: trailing "> " lines and the attribution above them. */
export function splitTextQuote(text: string): Split {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  let end = lines.length;
  while (end > 0 && !lines[end - 1].trim()) end--;
  let start = end;
  while (start > 0 && (lines[start - 1].startsWith(">") || !lines[start - 1].trim())) start--;
  while (start < end && !lines[start].trim()) start++;
  if (start === end || !lines.slice(start, end).some((l) => l.startsWith(">"))) return { body: text, quoted: false };
  // "On …, X wrote:" may wrap over two lines.
  const line = start > 0 ? lines[start - 1].trim() : "";
  if (ATTRIBUTION.test(line)) {
    const opens = (l: string) => /^(on|am|le|el|il|em|op|w|в|\d)\b/i.test(l.trim());
    start -= !opens(line) && start > 1 && opens(lines[start - 2]) ? 2 : 1;
  }
  const body = lines.slice(0, start).join("\n").trimEnd();
  return body.trim() ? { body, quoted: true } : { body: text, quoted: false };
}
