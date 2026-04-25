import { QueryClient } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useOverviewLiveUpdates } from './useOverviewLiveUpdates';

class MockEventSource {
  static instances: MockEventSource[] = [];

  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  readonly listeners = new Map<string, Array<(event: Event) => void>>();
  closed = false;

  constructor(readonly url: string) {
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject) {
    const callback =
      typeof listener === 'function'
        ? listener
        : (event: Event) => {
            listener.handleEvent(event);
          };
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), callback]);
  }

  close() {
    this.closed = true;
  }

  emitOpen() {
    this.onopen?.(new Event('open'));
  }

  emitError() {
    this.onerror?.(new Event('error'));
  }

  emit(type: string) {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(new Event(type));
    }
  }

  static reset() {
    MockEventSource.instances = [];
  }
}

describe('useOverviewLiveUpdates', () => {
  beforeEach(() => {
    MockEventSource.reset();
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('keeps one EventSource instance alive across transient errors', async () => {
    const onToast = vi.fn();
    const client = new QueryClient();
    const { result } = renderHook(() => useOverviewLiveUpdates({ client, onToast }));

    expect(MockEventSource.instances).toHaveLength(1);
    const firstStream = MockEventSource.instances[0];

    await waitFor(() => {
      expect(firstStream.onopen).toBeTypeOf('function');
      expect(firstStream.onerror).toBeTypeOf('function');
    });

    act(() => {
      firstStream.emitOpen();
    });
    await waitFor(() => {
      expect(result.current.liveUpdates).toBe('live');
    });

    act(() => {
      firstStream.emitError();
    });
    await waitFor(() => {
      expect(result.current.liveUpdates).toBe('reconnecting');
    });
    expect(MockEventSource.instances).toHaveLength(1);
    expect(firstStream.closed).toBe(false);

    act(() => {
      firstStream.emitOpen();
    });
    await waitFor(() => {
      expect(result.current.liveUpdates).toBe('live');
    });
    expect(onToast).toHaveBeenCalledWith('Reconnected to live updates.', 'success');
  });

  it('pauses after repeated errors and opens a fresh stream on retry', async () => {
    const onToast = vi.fn();
    const client = new QueryClient();
    const { result } = renderHook(() => useOverviewLiveUpdates({ client, onToast }));

    const firstStream = MockEventSource.instances[0];
    await waitFor(() => {
      expect(firstStream.onerror).toBeTypeOf('function');
    });

    act(() => {
      firstStream.emitError();
      firstStream.emitError();
      firstStream.emitError();
    });

    await waitFor(() => {
      expect(result.current.liveUpdates).toBe('paused');
    });
    expect(firstStream.closed).toBe(true);

    await act(async () => {
      result.current.retryLiveUpdates();
    });

    expect(MockEventSource.instances).toHaveLength(2);
    expect(MockEventSource.instances[1].url).toBe('/events');
  });
});
