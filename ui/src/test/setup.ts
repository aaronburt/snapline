class ResizeObserverStub implements ResizeObserver {
  observe(): void {}

  unobserve(): void {}

  disconnect(): void {}
}

if (!('ResizeObserver' in globalThis)) {
  globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
}
