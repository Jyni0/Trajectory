import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
  localStorage.clear();
});

// jsdom has no layout engine: give elements a size so the virtualized list renders rows.
Object.defineProperties(HTMLElement.prototype, {
  offsetHeight: { configurable: true, get: () => 900 },
  offsetWidth: { configurable: true, get: () => 360 },
});
Element.prototype.getBoundingClientRect = function () {
  return { x: 0, y: 0, top: 0, left: 0, bottom: 900, right: 360, width: 360, height: 900, toJSON: () => ({}) } as DOMRect;
};
Element.prototype.scrollTo = function () {} as typeof Element.prototype.scrollTo;

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;

window.matchMedia ??= vi.fn().mockImplementation((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: vi.fn(),
  removeListener: vi.fn(),
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  dispatchEvent: vi.fn(),
}));
