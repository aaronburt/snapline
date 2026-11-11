interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface Marquee {
  pointerId: number;
  startX: number;
  startY: number;
  x: number;
  y: number;
  originX: number;
  originY: number;
  active: boolean;
  base: ReadonlySet<number>;
  cards: Map<number, Box>;
}

export function startMarquee(
  element: HTMLElement,
  event: PointerEvent,
  base: ReadonlySet<number>,
): Marquee {
  const bounds = element.getBoundingClientRect();
  return {
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    x: event.clientX,
    y: event.clientY,
    originX: event.clientX - bounds.left + element.scrollLeft,
    originY: event.clientY - bounds.top + element.scrollTop,
    active: false,
    base,
    cards: new Map(),
  };
}

export function readMarquee(element: HTMLElement, drag: Marquee) {
  const bounds = element.getBoundingClientRect();
  const width = element.clientWidth;
  const height = element.clientHeight;
  const x = Math.max(0, Math.min(width, drag.x - bounds.left)) + element.scrollLeft;
  const y = Math.max(0, Math.min(height, drag.y - bounds.top)) + element.scrollTop;
  const area: Box = {
    left: Math.min(x, drag.originX),
    top: Math.min(y, drag.originY),
    right: Math.max(x, drag.originX),
    bottom: Math.max(y, drag.originY),
  };

  const offsetX = element.scrollLeft - bounds.left;
  const offsetY = element.scrollTop - bounds.top;
  for (const card of element.querySelectorAll<HTMLElement>('[data-capture-id]')) {
    const rect = card.getBoundingClientRect();
    drag.cards.set(Number(card.dataset.captureId), {
      left: rect.left + offsetX,
      top: rect.top + offsetY,
      right: rect.right + offsetX,
      bottom: rect.bottom + offsetY,
    });
  }

  const ids = new Set(drag.base);
  for (const [id, card] of drag.cards) {
    if (
      card.left < area.right &&
      card.right > area.left &&
      card.top < area.bottom &&
      card.bottom > area.top
    ) {
      ids.add(id);
    }
  }

  const left = Math.max(0, area.left - element.scrollLeft);
  const top = Math.max(0, area.top - element.scrollTop);
  const edge = Math.min(36, height / 4);
  const pointerY = drag.y - bounds.top;
  const scroll =
    pointerY < edge
      ? -Math.min(20, edge - pointerY)
      : pointerY > height - edge
        ? Math.min(20, pointerY - (height - edge))
        : 0;
  return {
    ids,
    left,
    top,
    width: Math.max(0, Math.min(width, area.right - element.scrollLeft) - left),
    height: Math.max(0, Math.min(height, area.bottom - element.scrollTop) - top),
    scroll,
  };
}
