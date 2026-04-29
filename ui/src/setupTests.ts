import '@testing-library/jest-dom/vitest';

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}

class DOMMatrixReadOnlyMock {
  m11 = 1;
  m22 = 1;
  m33 = 1;
  m44 = 1;
  m41 = 0;
  m42 = 0;

  constructor(transform = 'none') {
    if (typeof transform !== 'string' || transform === 'none') {
      return;
    }

    const matrixMatch = transform.match(/matrix\(([^)]+)\)/);
    if (matrixMatch) {
      const [a, , , d, e, f] = matrixMatch[1].split(',').map((value) => Number.parseFloat(value.trim()));
      this.m11 = Number.isFinite(a) ? a : 1;
      this.m22 = Number.isFinite(d) ? d : 1;
      this.m41 = Number.isFinite(e) ? e : 0;
      this.m42 = Number.isFinite(f) ? f : 0;
      return;
    }

    const scaleMatch = transform.match(/scale\(([^)]+)\)/);
    if (scaleMatch) {
      const scale = Number.parseFloat(scaleMatch[1].trim());
      if (Number.isFinite(scale)) {
        this.m11 = scale;
        this.m22 = scale;
      }
    }

    const translateMatch = transform.match(/translate\(([^,]+),([^)]+)\)/);
    if (translateMatch) {
      const x = Number.parseFloat(translateMatch[1].trim());
      const y = Number.parseFloat(translateMatch[2].trim());
      this.m41 = Number.isFinite(x) ? x : 0;
      this.m42 = Number.isFinite(y) ? y : 0;
    }
  }
}

Object.defineProperty(globalThis, 'ResizeObserver', {
  writable: true,
  configurable: true,
  value: ResizeObserverMock
});

Object.defineProperty(globalThis, 'DOMMatrixReadOnly', {
  writable: true,
  configurable: true,
  value: DOMMatrixReadOnlyMock
});

Object.defineProperty(globalThis, 'DOMMatrix', {
  writable: true,
  configurable: true,
  value: DOMMatrixReadOnlyMock
});

Object.defineProperty(HTMLElement.prototype, 'offsetWidth', {
  configurable: true,
  get() {
    return 1200;
  }
});

Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
  configurable: true,
  get() {
    return 800;
  }
});

Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
  configurable: true,
  value() {
    return {
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 1200,
      bottom: 800,
      width: 1200,
      height: 800,
      toJSON() {
        return this;
      }
    };
  }
});

Object.defineProperty(SVGElement.prototype, 'getBBox', {
  configurable: true,
  value() {
    return {
      x: 0,
      y: 0,
      width: 120,
      height: 24
    };
  }
});
