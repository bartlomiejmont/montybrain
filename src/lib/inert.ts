// Overlay dialogs can overlap (details + agent); releasing one must not unlock
// the other's background or leave it permanently inert when both close.
const locks = new WeakMap<HTMLElement, { count: number; previous: boolean }>();
export function lockInert(elements: HTMLElement[]): () => void {
  for (const el of elements) {
    const state = locks.get(el) ?? { count: 0, previous: el.inert };
    state.count += 1;
    locks.set(el, state);
    el.inert = true;
  }
  return () => {
    for (const el of elements) {
      const state = locks.get(el);
      if (state && --state.count === 0) {
        el.inert = state.previous;
        locks.delete(el);
      }
    }
  };
}
